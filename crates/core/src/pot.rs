// YouTube's proof-of-origin tokens, for when it asks yt-dlp for its bot check: bgutil's PO token provider
// (github.com/Brainicism/bgutil-ytdlp-pot-provider, GPL-3.0), a yt-dlp plugin and a script it runs on our own
// Deno to make each token (YouTube's BotGuard, in a page of its own). Set up once, in the background, the first
// time YouTube asks: its plugin and its script's source from GitHub, each checked against the checksum it had
// when it was tested here, then the script's packages from npm (Deno's install, without its developer tools).
// Used only after YouTube has asked (yt_stream): its web client, with a token, is slower (about 14 s a lookup,
// the script started each time), so the usual lookup is kept till then.

use sha2::{Digest, Sha256};
use std::io::{Cursor, Read};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// The version set up, and what its two downloads were when tested (2026-11-03)
const VERSION: &str = "2.0.2";
const PLUGIN_SHA256: &str = "a26b980bf6d8e8400d8f21445daa4d8381bf6183d20f0be309391f86b31e27ef";
const SOURCE_SHA256: &str = "df68ee6210e4960757bceae298927fb2ebd4d5cd9d390f9ff3a8287b0d2d1054";
/// Its packages' install: at most this long
const INSTALL_TIMEOUT: Duration = Duration::from_secs(600);

/// (being set up now)
static BUSY: AtomicBool = AtomicBool::new(false);

fn dir() -> Option<PathBuf> {
    crate::tools::bin_dir().ok().map(|d| d.join("pot"))
}

/// The plugin's folder (yt-dlp's --plugin-dirs)
pub fn plugins_dir() -> Option<PathBuf> {
    dir().map(|d| d.join("plugins"))
}

/// The script's folder (the plugin's server_home)
pub fn server_dir() -> Option<PathBuf> {
    dir().map(|d| d.join("server"))
}

fn marker() -> Option<PathBuf> {
    dir().map(|d| d.join(format!("ready-{VERSION}")))
}

/// Set up and ready: this version's plugin, its script, its packages, and Deno to run it
pub fn ready() -> bool {
    marker().is_some_and(|m| m.is_file()) && crate::tools::deno_path().is_some()
}

/// Sets it up in the background, if it isn't and isn't being (once at a time)
pub fn start_setup() {
    if ready() || BUSY.swap(true, Ordering::SeqCst) { return; }
    let job = async {
        let r = setup().await;
        match &r {
            Ok(()) => crate::diag::note(&format!("youtube: PO tokens set up ({VERSION})")),
            Err(e) => crate::diag::note(&format!("youtube: PO tokens couldn't be set up: {e}")),
        }
        BUSY.store(false, Ordering::SeqCst);
    };
    match tokio::runtime::Handle::try_current() {
        Ok(h) => { h.spawn(job); }
        Err(_) => { std::thread::spawn(move || { if let Ok(rt) = tokio::runtime::Runtime::new() { rt.block_on(job); } }); }
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

async fn fetch(client: &reqwest::Client, url: &str, sum: &str, what: &str) -> Result<Vec<u8>, String> {
    let res = client.get(url).send().await.map_err(|e| format!("couldn't download {what}: {e}"))?;
    if !res.status().is_success() { return Err(format!("GitHub said {} for {what}", res.status().as_u16())); }
    let bytes = res.bytes().await.map_err(|e| format!("couldn't download {what}: {e}"))?.to_vec();
    if sha256_hex(&bytes) != sum { return Err(format!("{what} isn't what was tested, so it wasn't kept")); }
    Ok(bytes)
}

/// The script's files, from the source's zip: those under its server folder (its packages left to Deno's install)
fn unpack_server(zip: &[u8], to: &std::path::Path) -> Result<usize, String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(zip)).map_err(|e| format!("the source isn't a zip: {e}"))?;
    let lead = format!("bgutil-ytdlp-pot-provider-{VERSION}/server/");
    let mut n = 0;
    for i in 0..archive.len() {
        let mut f = archive.by_index(i).map_err(|e| e.to_string())?;
        let Some(name) = f.enclosed_name() else { continue };
        let name = name.to_string_lossy().replace('\\', "/");
        let Some(rest) = name.strip_prefix(&lead) else { continue };
        if rest.is_empty() || rest.starts_with("node_modules") { continue; }
        let out = to.join(rest);
        if f.is_dir() { std::fs::create_dir_all(&out).map_err(|e| e.to_string())?; continue; }
        if let Some(p) = out.parent() { std::fs::create_dir_all(p).map_err(|e| e.to_string())?; }
        let mut buf = Vec::new();
        f.read_to_end(&mut buf).map_err(|e| e.to_string())?;
        std::fs::write(&out, buf).map_err(|e| e.to_string())?;
        n += 1;
    }
    Ok(n)
}

pub(crate) async fn setup() -> Result<(), String> {
    let deno = crate::tools::deno_path().ok_or("Deno isn't set up (YouTube Music's setup does that)")?;
    let (Some(home), Some(plugins), Some(server)) = (dir(), plugins_dir(), server_dir()) else { return Err("no folder for it".into()) };
    let client = reqwest::Client::builder().user_agent("OmniPulse").connect_timeout(Duration::from_secs(20)).build().map_err(|e| e.to_string())?;
    let base = "https://github.com/Brainicism/bgutil-ytdlp-pot-provider";
    let plugin = fetch(&client, &format!("{base}/releases/download/{VERSION}/bgutil-ytdlp-pot-provider.zip"), PLUGIN_SHA256, "its yt-dlp plugin").await?;
    let source = fetch(&client, &format!("{base}/archive/refs/tags/{VERSION}.zip"), SOURCE_SHA256, "its script").await?;
    // (afresh: an older version's or a half-done one's files gone)
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&plugins).map_err(|e| e.to_string())?;
    std::fs::write(plugins.join("bgutil-ytdlp-pot-provider.zip"), &plugin).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(&server).map_err(|e| e.to_string())?;
    if unpack_server(&source, &server)? == 0 { return Err("its script wasn't in the source".into()); }
    // its packages, as its lock file has them (canvas's own script let run: it fetches its prebuilt library)
    let mut cmd = tokio::process::Command::new(&deno);
    cmd.args(["install", "--prod", "--frozen", "--allow-scripts=npm:canvas"])
        .current_dir(&server)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);   // CREATE_NO_WINDOW
    let out = tokio::time::timeout(INSTALL_TIMEOUT, cmd.output()).await.map_err(|_| "its packages took too long to install".to_string())?.map_err(|e| e.to_string())?;
    if !out.status.success() {
        let why = String::from_utf8_lossy(&out.stderr);
        return Err(format!("its packages didn't install: {}", why.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("no details").trim()));
    }
    std::fs::write(home.join(format!("ready-{VERSION}")), VERSION).map_err(|e| e.to_string())?;
    Ok(())
}

/// What yt-dlp is given to make tokens with it: its plugin, and where its script is
pub fn ytdlp_args() -> Vec<String> {
    match (ready(), plugins_dir(), server_dir()) {
        (true, Some(p), Some(s)) => vec![
            "--plugin-dirs".into(), p.display().to_string(),
            "--extractor-args".into(), format!("youtubepot-bgutilscript:server_home={}", s.display()),
        ],
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_the_script_is_unpacked() {
        // (a zip like GitHub's: the repository under its version's folder)
        let mut buf = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut buf));
            let o = zip::write::SimpleFileOptions::default();
            for (name, body) in [("bgutil-ytdlp-pot-provider-2.0.2/README.md", "x"), ("bgutil-ytdlp-pot-provider-2.0.2/server/deno.json", "{}"), ("bgutil-ytdlp-pot-provider-2.0.2/server/src/generate_once.ts", "//"), ("bgutil-ytdlp-pot-provider-2.0.2/plugin/x.py", "#")] {
                z.start_file(name, o).unwrap();
                std::io::Write::write_all(&mut z, body.as_bytes()).unwrap();
            }
            z.finish().unwrap();
        }
        let to = std::env::temp_dir().join(format!("op-pot-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&to);
        assert_eq!(super::unpack_server(&buf, &to).unwrap(), 2);
        assert!(to.join("deno.json").is_file() && to.join("src").join("generate_once.ts").is_file());
        assert!(!to.join("README.md").exists() && !to.join("plugin").exists());
        let _ = std::fs::remove_dir_all(&to);
    }
}
