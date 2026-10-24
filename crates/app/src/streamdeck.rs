// The Stream Deck plugin, carried by the app (packed by scripts/streamdeck-pack.mjs): given to Stream Deck to
// install (Settings > Streaming > Stream Deck & Remote > Install), and again as the app starts when the one
// installed is older, so its keys arrive with the app's updates. Stream Deck installs it from the file
// (its own prompt). A plugin folder that's a link (one being worked on, linked to its source) is left alone.

const PLUGIN: &[u8] = include_bytes!("../streamdeck/OmniPulse.streamDeckPlugin");
const VERSION: &str = include_str!("../streamdeck/version.txt");

pub fn version() -> &'static str { VERSION.trim() }

fn sd_dir() -> Option<std::path::PathBuf> {
    std::env::var_os("APPDATA").map(|a| std::path::PathBuf::from(a).join("Elgato").join("StreamDeck"))
}

/// Stream Deck's app on this PC
pub fn present() -> bool { sd_dir().is_some_and(|d| d.join("Plugins").is_dir()) }

fn installed_dir() -> Option<std::path::PathBuf> { sd_dir().map(|d| d.join("Plugins").join("com.shwylie.scclient.sdPlugin")) }

/// The plugin installed: its version, if it's there
pub fn installed() -> Option<String> {
    let m = std::fs::read_to_string(installed_dir()?.join("manifest.json")).ok()?;
    serde_json::from_str::<serde_json::Value>(&m).ok()?["Version"].as_str().map(String::from)
}

/// Its folder a link (a symlink or a junction): one being worked on, never written over
fn linked() -> bool {
    let Some(d) = installed_dir() else { return false };
    let Ok(m) = std::fs::symlink_metadata(&d) else { return false };
    if m.file_type().is_symlink() { return true; }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if m.file_attributes() & 0x400 != 0 { return true; }
    }
    false
}

fn parts(v: &str) -> Vec<u64> { v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect() }

pub fn newer(a: &str, b: &str) -> bool { parts(a) > parts(b) }

/// Given to Stream Deck to install: written out, then opened (Stream Deck takes .streamDeckPlugin files)
pub fn install() -> Result<(), String> {
    if !present() { return Err("Stream Deck isn't installed on this PC.".into()); }
    if linked() { return Err("The plugin installed is a link to its source: it's left as it is.".into()); }
    let path = std::env::temp_dir().join("OmniPulse.streamDeckPlugin");
    std::fs::write(&path, PLUGIN).map_err(|e| e.to_string())?;
    std::process::Command::new("explorer.exe").arg(&path).spawn().map(|_| ()).map_err(|e| format!("Couldn't open it for Stream Deck: {e}"))
}

/// As the app starts: the plugin installed but older than the app's, updated (once for each version: if it's
/// turned down, it isn't asked again until there's a newer one). The version offered, if it was
pub fn update_on_start(offered: &str) -> Option<&'static str> {
    let have = installed()?;
    if !newer(version(), &have) || offered == version() || linked() { return None; }
    install().ok().map(|_| version())
}

#[cfg(test)]
mod tests {
    #[test]
    fn versions() {
        assert!(super::newer("1.3.0.0", "1.2.0.0"));
        assert!(!super::newer("1.3.0.0", "1.3.0.0"));
        assert!(super::newer("1.10.0.0", "1.9.0.0"));
        assert!(!super::version().is_empty());
    }
}
