// yt-dlp and Deno, built in: YouTube Music needs yt-dlp (which finds a video's audio) and a JavaScript
// runtime for it (YouTube's player is JavaScript; yt-dlp issue #15012). Settings > Integrations > YouTube
// Music > Set up downloads both from their official GitHub releases into <data dir>\bin, each checked
// against the SHA-256 checksum published with the release. yt-dlp then updates itself once a day.
//
// A yt-dlp or Deno already on the PATH is used if there's none in our folder. Their caches are kept in
// <data dir>incache too (yt-dlp's --cache-dir, Deno's DENO_DIR), so Remove (uninstall) leaves nothing.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::Duration;

pub struct Sources {
    pub ytdlp: String,
    pub ytdlp_sums: String,
    pub deno_zip: String,
    pub deno_sum: String,
}

impl Default for Sources {
    fn default() -> Sources {
        Sources {
            ytdlp: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe".into(),
            ytdlp_sums: "https://github.com/yt-dlp/yt-dlp/releases/latest/download/SHA2-256SUMS".into(),
            deno_zip: "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip".into(),
            deno_sum: "https://github.com/denoland/deno/releases/latest/download/deno-x86_64-pc-windows-msvc.zip.sha256sum".into(),
        }
    }
}

const UPDATE_EVERY: u64 = 24 * 3600;

#[derive(Default, Clone)]
struct Progress {
    busy: bool,
    step: String,
    done: u64,
    total: u64,
    error: Option<String>,
}

static PROGRESS: Mutex<Option<Progress>> = Mutex::new(None);

fn set_progress(f: impl FnOnce(&mut Progress)) {
    if let Ok(mut g) = PROGRESS.lock() {
        f(g.get_or_insert_with(Progress::default));
    }
}

/// Where our copies live
pub fn bin_dir() -> Result<PathBuf, String> {
    let d = crate::data_dir()?.join("bin");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn ours(name: &str) -> Option<PathBuf> {
    bin_dir().ok().map(|d| d.join(name)).filter(|p| p.is_file())
}

pub fn ytdlp_path() -> Option<PathBuf> {
    ours("yt-dlp.exe")
}

pub fn deno_path() -> Option<PathBuf> {
    ours("deno.exe")
}

fn cache_dir() -> Option<PathBuf> {
    bin_dir().ok().map(|d| d.join("cache"))
}

/// yt-dlp's cache, in our folder
pub fn cache_args() -> Vec<String> {
    cache_dir().map(|d| vec!["--cache-dir".into(), d.join("yt-dlp").display().to_string()]).unwrap_or_default()
}

/// The environment for yt-dlp, so the Deno it starts caches in our folder
pub fn cache_env() -> Vec<(String, String)> {
    cache_dir().map(|d| vec![("DENO_DIR".into(), d.join("deno").display().to_string())]).unwrap_or_default()
}

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.flatten()
                .map(|e| match e.metadata() {
                    Ok(m) if m.is_dir() => dir_size(&e.path()),
                    Ok(m) => m.len(),
                    Err(_) => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// How much space our yt-dlp, Deno and their caches take
pub fn disk_use() -> u64 {
    bin_dir().map(|d| dir_size(&d)).unwrap_or(0)
}

/// Removes our yt-dlp and Deno, their caches, and what was remembered about YouTube tracks
/// (loudness measurements, stream addresses); returns the bytes freed
pub fn uninstall() -> Result<u64, String> {
    if PROGRESS.lock().ok().and_then(|g| g.as_ref().map(|p| p.busy)).unwrap_or(false) {
        return Err("it's still being set up: try again when that's finished".into());
    }
    let dir = bin_dir()?;
    let freed = dir_size(&dir);
    std::fs::remove_dir_all(&dir).map_err(|e| format!("couldn't remove {}: {e}", dir.display()))?;
    crate::loudness::forget("v1:yt:");
    crate::youtube::forget_streams();
    set_progress(|p| *p = Progress::default());
    Ok(freed)
}

/// The yt-dlp to run: ours, or one on the PATH
pub fn ytdlp_program() -> OsString {
    ytdlp_path().map(|p| p.into_os_string()).unwrap_or_else(|| "yt-dlp".into())
}

/// The SHA-256 a checksum file gives, as lower-case hex: from "hash  file" lines (the one naming
/// `file`, if given) or PowerShell's "Hash : ..." layout
pub fn parse_sum(text: &str, file: Option<&str>) -> Option<String> {
    let hex64 = |w: &str| w.len() == 64 && w.chars().all(|c| c.is_ascii_hexdigit());
    let line = match file {
        Some(f) => text.lines().find(|l| l.split_whitespace().last() == Some(f))?,
        None => text,
    };
    line.split(|c: char| c.is_whitespace() || c == ':').find(|w| hex64(w)).map(|w| w.to_lowercase())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

async fn get_text(client: &reqwest::Client, url: &str) -> Result<String, String> {
    let res = client.get(url).send().await.map_err(|e| format!("couldn't reach GitHub: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("GitHub said {} for {url}", res.status().as_u16()));
    }
    res.text().await.map_err(|e| e.to_string())
}

// Downloads into memory, reporting progress as it goes
async fn download(client: &reqwest::Client, url: &str, what: &str) -> Result<Vec<u8>, String> {
    let mut res = client.get(url).send().await.map_err(|e| format!("couldn't reach GitHub: {e}"))?;
    if !res.status().is_success() {
        return Err(format!("GitHub said {} for {url}", res.status().as_u16()));
    }
    let total = res.content_length().unwrap_or(0);
    set_progress(|p| {
        p.step = format!("Downloading {what}");
        p.done = 0;
        p.total = total;
    });
    let mut out = Vec::with_capacity(total as usize);
    while let Some(chunk) = res.chunk().await.map_err(|e| format!("the download of {what} broke off: {e}"))? {
        out.extend_from_slice(&chunk);
        let n = out.len() as u64;
        set_progress(|p| p.done = n);
    }
    Ok(out)
}

// Writes next to the final name, then swaps it in, so a half-written file is never used
fn put(name: &str, bytes: &[u8]) -> Result<(), String> {
    let dir = bin_dir()?;
    let part = dir.join(format!("{name}.part"));
    std::fs::write(&part, bytes).map_err(|e| format!("couldn't save {name}: {e}"))?;
    std::fs::rename(&part, dir.join(name)).map_err(|e| format!("couldn't save {name}: {e}"))
}

fn unzip_one(zip: &[u8], name: &str) -> Result<Vec<u8>, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).map_err(|e| format!("the Deno download isn't a zip: {e}"))?;
    let mut file = archive.by_name(name).map_err(|_| format!("{name} isn't in the Deno download"))?;
    let mut out = Vec::new();
    file.read_to_end(&mut out).map_err(|e| e.to_string())?;
    Ok(out)
}

/// Downloads whatever's missing (blocking the caller until done); progress() says how it's going
pub async fn install(src: &Sources) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .user_agent("OmniPulse")
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())?;
    if ytdlp_path().is_none() {
        set_progress(|p| p.step = "Checking yt-dlp's latest release".into());
        let want = parse_sum(&get_text(&client, &src.ytdlp_sums).await?, Some("yt-dlp.exe"))
            .ok_or("yt-dlp's release has no checksum for yt-dlp.exe")?;
        let exe = download(&client, &src.ytdlp, "yt-dlp").await?;
        if sha256_hex(&exe) != want {
            return Err("the yt-dlp download doesn't match its published checksum, so it wasn't kept".into());
        }
        put("yt-dlp.exe", &exe)?;
    }
    if deno_path().is_none() {
        set_progress(|p| p.step = "Checking Deno's latest release".into());
        let want = parse_sum(&get_text(&client, &src.deno_sum).await?, None).ok_or("Deno's release has no checksum")?;
        let zip = download(&client, &src.deno_zip, "Deno").await?;
        if sha256_hex(&zip) != want {
            return Err("the Deno download doesn't match its published checksum, so it wasn't kept".into());
        }
        set_progress(|p| p.step = "Unpacking Deno".into());
        put("deno.exe", &unzip_one(&zip, "deno.exe")?)?;
    }
    Ok(())
}

/// Starts setting up in the background (if it isn't already)
pub fn start_install() {
    start_install_from(Sources::default());
}

// Callable from anywhere: inside an async runtime it's spawned there, otherwise (a window's thread,
// say) it gets a thread and runtime of its own. tokio::spawn alone panics outside a runtime, and a
// panic in a WebView2 callback can't unwind, so it took the whole app down.
pub fn start_install_from(src: Sources) {
    let already = PROGRESS.lock().ok().and_then(|g| g.as_ref().map(|p| p.busy)).unwrap_or(false);
    if already {
        return;
    }
    set_progress(|p| *p = Progress { busy: true, step: "Starting".into(), ..Default::default() });
    let job = async move {
        let r = install(&src).await;
        set_progress(|p| {
            p.busy = false;
            p.error = r.err();
            p.step = if p.error.is_some() { "Couldn't finish".into() } else { "Done".into() };
        });
    };
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(job);
        }
        Err(_) => {
            std::thread::spawn(move || match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(rt) => rt.block_on(job),
                Err(e) => set_progress(|p| {
                    p.busy = false;
                    p.error = Some(format!("couldn't start the download: {e}"));
                }),
            });
        }
    }
}

pub fn progress() -> Value {
    let p = PROGRESS.lock().ok().and_then(|g| g.clone()).unwrap_or_default();
    json!({ "busy": p.busy, "step": p.step, "done": p.done, "total": p.total, "error": p.error })
}

/// Our yt-dlp updates itself (yt-dlp -U), at most once a day; returns what it said, if it ran
pub async fn update_ytdlp_daily() -> Option<String> {
    let exe = ytdlp_path()?;
    let stamp = bin_dir().ok()?.join("yt-dlp.checked");
    let last = std::fs::read_to_string(&stamp).ok().and_then(|t| t.trim().parse::<u64>().ok()).unwrap_or(0);
    let now = crate::now_secs();
    if now.saturating_sub(last) < UPDATE_EVERY {
        return None;
    }
    let _ = std::fs::write(&stamp, now.to_string());
    let mut cmd = tokio::process::Command::new(exe);
    cmd.arg("-U").args(cache_args()).envs(cache_env()).stdin(std::process::Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // no console window
    let out = tokio::time::timeout(Duration::from_secs(120), cmd.output()).await.ok()?.ok()?;
    Some(String::from_utf8_lossy(&out.stdout).lines().last().unwrap_or("").to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_both_checksum_layouts() {
        let sums = "aaaa  yt-dlp\n66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a  yt-dlp.exe\n1111111111111111111111111111111111111111111111111111111111111111  yt-dlp_x86.exe\n";
        assert_eq!(parse_sum(sums, Some("yt-dlp.exe")).as_deref(), Some("66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a"));
        assert_eq!(parse_sum(sums, Some("nope.exe")), None);
        let ps = "\r\nAlgorithm : SHA256\r\nHash      : A0C3101B4158D1DFB7D6A78A7BF0F3DE80C96BB423C152BEEC8BEB22786F2238\r\nPath      : C:\\a\\deno\\deno\\target\\release\\deno-x86_64-pc-windows-msvc.zip\r\n";
        assert_eq!(parse_sum(ps, None).as_deref(), Some("a0c3101b4158d1dfb7d6a78a7bf0f3de80c96bb423c152beec8beb22786f2238"));
    }

    // A fake GitHub on 127.0.0.1: serves the files given, by path
    fn serve(files: Vec<(&'static str, Vec<u8>)>) -> String {
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                match files.iter().find(|(p, _)| *p == req.url()) {
                    Some((_, body)) => req.respond(tiny_http::Response::from_data(body.clone())).ok(),
                    None => req.respond(tiny_http::Response::empty(404)).ok(),
                };
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn deno_zip(exe: &[u8]) -> Vec<u8> {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        w.start_file("deno.exe", zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(exe).unwrap();
        w.finish().unwrap().into_inner()
    }


    // The crash: setting up from a thread with no async runtime (where Tauri runs plain commands)
    #[test]
    fn starts_from_a_thread_without_a_runtime() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let base = serve(vec![]); // a GitHub with nothing on it: setup fails, but cleanly
        let src = Sources {
            ytdlp: format!("{base}/yt-dlp.exe"),
            ytdlp_sums: format!("{base}/sums"),
            deno_zip: format!("{base}/deno.zip"),
            deno_sum: format!("{base}/deno.zip.sha256sum"),
        };
        std::thread::spawn(move || start_install_from(src)).join().expect("starting setup panicked");
        let t0 = std::time::Instant::now();
        while progress()["busy"] == true && t0.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(20));
        }
        let p = progress();
        assert_eq!(p["busy"], false, "{p}");
        assert!(p["error"].as_str().unwrap_or("").contains("404") || ytdlp_path().is_some(), "{p}");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // only makes the tests take turns; this one runs on one thread
    async fn downloads_checks_and_unpacks() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("omnipulse-tools-{}", std::process::id()));
        std::env::set_var("APPDATA", &tmp);
        let exe = b"MZ fake yt-dlp".to_vec();
        let zip = deno_zip(b"MZ fake deno");
        let good = serve(vec![
            ("/sums", format!("{}  yt-dlp.exe\n", sha256_hex(&exe)).into_bytes()),
            ("/yt-dlp.exe", exe.clone()),
            ("/deno.zip.sha256sum", format!("Algorithm : SHA256\r\nHash : {}\r\n", sha256_hex(&zip).to_uppercase()).into_bytes()),
            ("/deno.zip", zip.clone()),
        ]);
        let src = |base: &str| Sources {
            ytdlp: format!("{base}/yt-dlp.exe"),
            ytdlp_sums: format!("{base}/sums"),
            deno_zip: format!("{base}/deno.zip"),
            deno_sum: format!("{base}/deno.zip.sha256sum"),
        };

        // a download that doesn't match its checksum isn't kept
        let bad = serve(vec![
            ("/sums", format!("{}  yt-dlp.exe\n", "0".repeat(64)).into_bytes()),
            ("/yt-dlp.exe", exe.clone()),
        ]);
        let err = install(&src(&bad)).await.unwrap_err();
        assert!(err.contains("doesn't match"), "{err}");
        assert!(ytdlp_path().is_none());

        install(&src(&good)).await.unwrap();
        assert_eq!(std::fs::read(ytdlp_path().unwrap()).unwrap(), exe);
        assert_eq!(std::fs::read(deno_path().unwrap()).unwrap(), b"MZ fake deno");
        assert!(!bin_dir().unwrap().join("yt-dlp.exe.part").exists());
        assert_eq!(ytdlp_program(), ytdlp_path().unwrap().into_os_string());
        // already there: nothing is fetched again (the bad server would fail it)
        install(&src(&bad)).await.unwrap();
        // Remove takes everything, caches included
        std::fs::create_dir_all(bin_dir().unwrap().join("cache").join("deno")).unwrap();
        std::fs::write(bin_dir().unwrap().join("cache").join("deno").join("x"), b"cached").unwrap();
        assert!(disk_use() > 0);
        assert!(uninstall().unwrap() > 0);
        assert!(ytdlp_path().is_none() && deno_path().is_none());
        assert!(!tmp.join("sc-client").join("bin").join("cache").exists());
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
