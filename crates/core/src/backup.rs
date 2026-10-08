// Back Up and Restore (Settings > App): your settings and what you've added, in one .zip.
//
// In it: the app's saved data from the page (favourites, stats, settings, playlists on this PC,
// history: web.json), your keys (config.json), the alerts' sounds, pictures and collage pictures,
// plugins, loudness measurements, and the native app's state. Not in it: sign-ins (they're tied to
// this PC, and you'd sign in again anyway), and what's downloaded or cached (yt-dlp, Spotify's
// cache), which comes back by itself.
//
// Restoring only writes those same files back into the data folder: anything else in the .zip
// (or any path that tries to leave the folder) is left alone.

use serde_json::{json, Value};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

const MARKER: &str = "omnipulse-backup.json";
const WEB: &str = "web.json";
/// What goes in, under the data folder: files, and folders (everything in them)
const FILES: [&str; 3] = ["config.json", "settings.json", "loudness.json"];
const DIRS: [&str; 5] = ["alert-sounds", "alert-images", "alert-collage", "plugins", "state"];
const MAX_FILE: u64 = 64 * 1024 * 1024;

fn wanted(rel: &str) -> bool {
    let rel = rel.replace('\\', "/");
    if rel.split('/').any(|p| p == ".." || p.is_empty()) {
        return false;
    }
    FILES.contains(&rel.as_str()) || DIRS.iter().any(|d| rel.starts_with(&format!("{d}/")))
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(list) = std::fs::read_dir(dir) else { return };
    for e in list.flatten() {
        let p = e.path();
        if p.is_dir() {
            walk(&p, base, out);
        } else if let Ok(rel) = p.strip_prefix(base) {
            out.push((rel.to_string_lossy().replace('\\', "/"), p));
        }
    }
}

/// Writes a backup to dest (a .zip); web is the page's saved data, as JSON. Returns how many files.
pub fn create(dest: &str, web: &str) -> Result<usize, String> {
    serde_json::from_str::<Value>(web).map_err(|e| format!("the app's data isn't readable: {e}"))?;
    let base = crate::data_dir()?;
    let mut files: Vec<(String, PathBuf)> = FILES.iter().map(|f| (f.to_string(), base.join(f))).filter(|(_, p)| p.is_file()).collect();
    for d in DIRS {
        walk(&base.join(d), &base, &mut files);
    }
    files.retain(|(rel, p)| wanted(rel) && std::fs::metadata(p).map(|m| m.len() <= MAX_FILE).unwrap_or(false));
    let tmp = PathBuf::from(format!("{dest}.part"));
    let out = std::fs::File::create(&tmp).map_err(|e| format!("Couldn't write the backup there: {e}"))?;
    let mut zip = zip::ZipWriter::new(out);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let marker = json!({ "app": "OmniPulse", "version": 1, "made_at": crate::now_secs(), "files": files.len() });
    let mut put = |name: &str, bytes: &[u8]| -> Result<(), String> {
        zip.start_file(name, opts).map_err(|e| e.to_string())?;
        zip.write_all(bytes).map_err(|e| e.to_string())
    };
    put(MARKER, marker.to_string().as_bytes())?;
    put(WEB, web.as_bytes())?;
    for (rel, p) in &files {
        let bytes = std::fs::read(p).map_err(|e| format!("{rel}: {e}"))?;
        put(&format!("data/{rel}"), &bytes)?;
    }
    zip.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dest).or_else(|_| std::fs::copy(&tmp, dest).map(|_| ()).and_then(|_| std::fs::remove_file(&tmp))).map_err(|e| e.to_string())?;
    Ok(files.len())
}

/// Puts a backup's files back in the data folder; returns the page's saved data from it (JSON)
pub fn restore(src: &str) -> Result<String, String> {
    let file = std::fs::File::open(src).map_err(|e| format!("Couldn't open it: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|_| "That isn't an OmniPulse backup.".to_string())?;
    let marker: Value = {
        let mut s = String::new();
        zip.by_name(MARKER).map_err(|_| "That isn't an OmniPulse backup.".to_string())?.read_to_string(&mut s).map_err(|e| e.to_string())?;
        serde_json::from_str(&s).map_err(|_| "That backup is damaged.".to_string())?
    };
    if marker["app"] != "OmniPulse" || marker["version"].as_u64().unwrap_or(0) > 1 {
        return Err("That backup is from a newer OmniPulse, or isn't one.".into());
    }
    let mut web = String::new();
    zip.by_name(WEB).map_err(|_| "That backup is missing the app's data.".to_string())?.read_to_string(&mut web).map_err(|e| e.to_string())?;
    serde_json::from_str::<Value>(&web).map_err(|_| "That backup's data is damaged.".to_string())?;
    let base = crate::data_dir()?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.name().strip_prefix("data/").map(String::from) else { continue };
        if f.is_dir() || !wanted(&rel) || f.enclosed_name().is_none() || f.size() > MAX_FILE {
            continue;
        }
        let dest = base.join(&rel);
        if let Some(parent) = dest.parent() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut bytes = Vec::new();
        f.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
        std::fs::write(&dest, bytes).map_err(|e| format!("{rel}: {e}"))?;
    }
    Ok(web)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backs_up_and_restores_only_what_it_should() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("omnipulse-backup-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(tmp.join("OmniPulse").join("alert-sounds")).unwrap();
        std::env::set_var("APPDATA", &tmp);
        let base = crate::data_dir().unwrap();
        std::fs::write(base.join("config.json"), r#"{"lastfm_api_key":"k"}"#).unwrap();
        std::fs::write(base.join("alert-sounds").join("follow.wav"), b"RIFF").unwrap();
        std::fs::write(base.join("tokens.json"), r#"{"access_token":"secret"}"#).unwrap();
        let zip_path = tmp.join("b.zip");
        assert_eq!(create(zip_path.to_str().unwrap(), r#"{"sc.prefs":"{}"}"#).unwrap(), 2);

        // gone, then back from the backup; the sign-in was never in it
        std::fs::remove_file(base.join("config.json")).unwrap();
        std::fs::remove_dir_all(base.join("alert-sounds")).unwrap();
        std::fs::remove_file(base.join("tokens.json")).unwrap();
        let web = restore(zip_path.to_str().unwrap()).unwrap();
        assert!(web.contains("sc.prefs"));
        assert!(base.join("config.json").exists() && base.join("alert-sounds").join("follow.wav").exists());
        assert!(!base.join("tokens.json").exists());
        assert!(!wanted("../evil.json") && !wanted("alert-sounds/../../x") && !wanted("tokens.json"));
        assert!(restore(base.join("config.json").to_str().unwrap()).is_err());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
