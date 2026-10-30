// The backend shared by the Tauri app (src-tauri) and the native app (crates/app): accounts,
// SoundCloud / Spotify / Last.fm / Discord / lyrics, local music, and the local servers.
// Nothing here knows about a window; the apps pass in callbacks where the page used to be called.

pub mod apple;
pub mod diag;
pub mod docks;
pub mod emotes;
pub mod badges;
#[cfg(feature = "desktop")]
pub mod backup;
pub mod bundled;
pub mod bio;
pub mod catalog;
#[cfg(feature = "desktop")]
pub mod discord;
pub mod lastfm;
#[cfg(feature = "desktop")]
pub mod local;
#[cfg(feature = "desktop")]
pub mod loudness;
pub mod meter;
pub mod lyrics;
pub mod ranking;
pub mod keys;
pub mod keyfinder;
pub mod rhythm;
pub mod songbpm;
pub mod sponsorblock;
pub mod musicbrainz;
pub mod playlistfile;
#[cfg(feature = "desktop")]
pub mod plugins;
#[cfg(feature = "desktop")]
pub mod cast;
#[cfg(feature = "desktop")]
pub mod omni_input;
#[cfg(feature = "desktop")]
pub mod main_out;
#[cfg(feature = "desktop")]
mod capture_diag;
#[cfg(feature = "desktop")]
pub mod remote;
#[cfg(feature = "desktop")]
pub mod requests;
pub mod soundcloud;
pub mod spotify;
#[cfg(feature = "desktop")]
pub mod spotify_engine;
#[cfg(feature = "spotify")]
pub mod spotify_library;
#[cfg(all(feature = "spotify", not(feature = "desktop")))]
pub mod spotify_session;
#[cfg(feature = "desktop")]
pub mod tools;
#[cfg(feature = "desktop")]
pub mod widget;
pub mod youtube;
pub mod yt_account;
#[cfg(feature = "desktop")]
pub mod yt_stream;

use serde::{Deserialize, Serialize};

/// Tests that point APPDATA at a folder of their own take turns
#[cfg(test)]
pub(crate) static TEST_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::{fs, path::PathBuf};

// ---------- config and token storage ----------
// Your own keys live in %APPDATA%\OmniPulse\config.json (Spotify, Last.fm, a YouTube account...).
// The app's own (SoundCloud's, Discord's) are built in (bundled.rs) and never written there; any you
// put in config.json win over them. Secrets are only read by the Rust side, never the web page.

#[derive(Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
    #[serde(default = "default_redirect")]
    pub redirect_uri: String,
    #[serde(default)]
    pub spotify_client_id: String,
    #[serde(default = "default_spotify_redirect")]
    pub spotify_redirect_uri: String,
    #[serde(default)]
    pub discord_client_id: String,
    #[serde(default)]
    pub lastfm_api_key: String,
    #[serde(default)]
    pub lastfm_secret: String,
    /// a Google Cloud OAuth client (Desktop app), for the optional YouTube account (yt_account.rs)
    #[serde(default)]
    pub youtube_client_id: String,
    #[serde(default)]
    pub youtube_client_secret: String,
    /// OmniPulse's own SoundCloud app: no secret here, so its sign-ins are finished by this relay
    #[serde(skip)]
    pub sc_relay: String,
}

pub fn default_spotify_redirect() -> String {
    "http://127.0.0.1:8081/callback".to_string()
}

pub fn default_redirect() -> String {
    "http://localhost:8080/callback".to_string()
}

#[derive(Serialize, Deserialize, Clone)]
pub struct Tokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
}

/// %APPDATA%\OmniPulse. The first time, the old %APPDATA%\sc-client is moved there (if it can't be
/// moved yet, say a file in it is in use, the old one is used until it can).
/// Where the app keeps its things, if not %APPDATA%\OmniPulse (the Android app's own folder)
static DATA_DIR: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);
pub fn set_data_dir(dir: PathBuf) {
    let _ = fs::create_dir_all(&dir);
    *DATA_DIR.lock().unwrap() = Some(dir);
}

/// Opens a sign-in page in the browser: the desktop's default browser, or (set by the Android app)
/// its own way of showing one
type Opener = Box<dyn Fn(&str) -> Result<(), String> + Send + Sync>;
static OPENER: std::sync::Mutex<Option<Opener>> = std::sync::Mutex::new(None);
pub fn set_opener(f: Opener) {
    *OPENER.lock().unwrap() = Some(f);
}
pub fn open_browser(url: &str) -> Result<(), String> {
    if let Some(f) = OPENER.lock().unwrap().as_ref() {
        return f(url);
    }
    #[cfg(feature = "desktop")]
    return opener::open(url).map_err(|e| e.to_string());
    #[cfg(not(feature = "desktop"))]
    Err("No way to open the browser here.".into())
}

/// Anything that goes wrong inside the app (a panic: in it or a library it uses, on any thread) is
/// written to crash.log in the data folder, with where and how it got there, and kept to the last
/// 200 KB: so a crash can be looked into (it can't be seen otherwise: the app has no console).
/// The panic still goes on as before after (that thread or task ends).
pub fn crash_log_install() {
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let thread = std::thread::current().name().unwrap_or("unnamed").to_string();
        let what = info.payload().downcast_ref::<&str>().map(|s| s.to_string()).or_else(|| info.payload().downcast_ref::<String>().cloned()).unwrap_or_else(|| "(no message)".into());
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        let trace = std::backtrace::Backtrace::force_capture();
        let entry = format!("---- {at} (unix secs), version {}, thread {thread}\n{what}\nat {place}\n{trace}\n", env!("CARGO_PKG_VERSION"));
        diag::note(&format!("panic in {thread}: {what} at {place}"));
        if let Ok(dir) = data_dir() {
            let f = dir.join("crash.log");
            let old = std::fs::read_to_string(&f).unwrap_or_default();
            let keep = if old.len() > 200_000 { old.get(old.len() - 150_000..).unwrap_or("").to_string() } else { old };
            let _ = std::fs::write(&f, keep + &entry);
        }
        before(info);
    }));
}

pub fn data_dir() -> Result<PathBuf, String> {
    if let Some(d) = DATA_DIR.lock().unwrap().clone() {
        return Ok(d);
    }
    let base = PathBuf::from(std::env::var("APPDATA").map_err(|_| "APPDATA is not set".to_string())?);
    let dir = base.join("OmniPulse");
    let old = base.join("sc-client");
    // one move at a time: several parts of the app ask for the folder at once on the first start, and
    // one that lost the race mustn't go back to the old folder (and make it again)
    static MOVING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    if !dir.exists() && old.is_dir() {
        let _one = MOVING.lock().unwrap_or_else(|e| e.into_inner());
        if !dir.exists() && old.is_dir() {
            match fs::rename(&old, &dir) {
                Ok(()) => tidy_config(&dir),
                // in use: the old one, until it can be moved
                Err(_) if !dir.exists() => return Ok(old),
                Err(_) => {}
            }
        }
    }
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// The app's own keys out of config.json: they're built in now. Yours stay.
fn tidy_config(dir: &std::path::Path) {
    let path = dir.join("config.json");
    let Ok(text) = fs::read_to_string(&path) else { return };
    let Ok(serde_json::Value::Object(mut o)) = serde_json::from_str::<serde_json::Value>(&text) else { return };
    let mut gone = false;
    let mut drop_if = |k: &str, v: &str| {
        if !v.is_empty() && o.get(k).and_then(|x| x.as_str()) == Some(v) {
            o.remove(k);
            gone = true;
        }
    };
    if let Some((id, redirect)) = bundled::soundcloud() {
        drop_if("client_id", &id);
        drop_if("redirect_uri", &redirect);
    }
    drop_if("discord_client_id", bundled::DISCORD_CLIENT_ID);
    if gone {
        let _ = fs::write(&path, serde_json::to_string_pretty(&serde_json::Value::Object(o)).unwrap_or_default());
    }
}

/// Your keys from config.json (it's fine for it to be missing), with the app's own filling the gaps
pub fn load_config() -> Result<Config, String> {
    let path = data_dir()?.join("config.json");
    let mut cfg: Config = match fs::read_to_string(&path) {
        Ok(text) if !text.trim().is_empty() => serde_json::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("Bad config.json: {e}"))?,
        _ => serde_json::from_str("{}").map_err(|e| e.to_string())?,
    };
    if cfg.client_id.trim().is_empty() || cfg.client_secret.trim().is_empty() {
        if let Some((id, redirect)) = bundled::soundcloud() {
            cfg.client_id = id;
            cfg.client_secret = String::new();
            cfg.sc_relay = bundled::RELAY_URL.to_string();
            if !fs::read_to_string(&path).unwrap_or_default().contains("\"redirect_uri\"") {
                cfg.redirect_uri = redirect;
            }
        }
    }
    if cfg.discord_client_id.trim().is_empty() {
        cfg.discord_client_id = bundled::DISCORD_CLIENT_ID.to_string();
    }
    Ok(cfg)
}

/// Saves keys you set up in the app (Settings > Integrations > Set up) into config.json, keeping the rest
pub fn config_set(fields: &serde_json::Map<String, serde_json::Value>) -> Result<(), String> {
    const ALLOWED: [&str; 6] = ["spotify_client_id", "lastfm_api_key", "lastfm_secret", "youtube_client_id", "youtube_client_secret", "discord_client_id"];
    let path = data_dir()?.join("config.json");
    let mut o = match fs::read_to_string(&path).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(t.trim_start_matches('\u{feff}')).ok()) {
        Some(serde_json::Value::Object(o)) => o,
        _ => serde_json::Map::new(),
    };
    for (k, v) in fields {
        if !ALLOWED.contains(&k.as_str()) {
            return Err(format!("{k} can't be set from the app"));
        }
        let v = v.as_str().unwrap_or("").trim();
        if v.is_empty() { o.remove(k); } else { o.insert(k.clone(), serde_json::Value::String(v.to_string())); }
    }
    fs::write(&path, serde_json::to_string_pretty(&serde_json::Value::Object(o)).map_err(|e| e.to_string())?).map_err(|e| e.to_string())
}

/// Which of your own keys are set (not the keys), for Settings
pub fn config_status() -> serde_json::Value {
    let c = load_config().unwrap_or_default();
    serde_json::json!({
        "spotify": !c.spotify_client_id.trim().is_empty(),
        "lastfm": !c.lastfm_api_key.trim().is_empty() && !c.lastfm_secret.trim().is_empty(),
        "youtube": !c.youtube_client_id.trim().is_empty() && !c.youtube_client_secret.trim().is_empty(),
        "soundcloud": !c.client_id.trim().is_empty(),
        "spotify_redirect": c.spotify_redirect_uri,
    })
}

pub fn load_tokens() -> Option<Tokens> {
    let text = fs::read_to_string(data_dir().ok()?.join("tokens.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_tokens(t: &Tokens) -> Result<(), String> {
    let text = serde_json::to_string(t).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("tokens.json"), text).map_err(|e| e.to_string())
}

pub fn bind_addr(redirect: &str) -> String {
    let port = redirect
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .and_then(|host| host.split(':').nth(1))
        .unwrap_or("8080");
    format!("127.0.0.1:{port}")
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// Opens a SoundCloud page in the default browser (only soundcloud.com links).
/// This PC's address on your network (where a bot on another device reaches it)
pub fn lan_ip() -> Option<String> {
    let s = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect(("192.0.2.1", 9)).ok()?;   // a documentation address: nothing is sent, it only picks the route
    Some(s.local_addr().ok()?.ip().to_string())
}

/// A platform's developer pages, in your browser, for setting up your own keys (only those)
pub fn open_setup_page(url: &str) -> Result<(), String> {
    const OK: [&str; 3] = ["https://developer.spotify.com/", "https://www.last.fm/api/", "https://console.cloud.google.com/"];
    if !OK.iter().any(|p| url.starts_with(p)) {
        return Err("Only Spotify's, Last.fm's and Google Cloud's developer pages open from here".into());
    }
    open_browser(url)
}

pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://soundcloud.com/")
        || url.starts_with("https://www.soundcloud.com/")
        || url.starts_with("https://open.spotify.com/")
        || url.starts_with("https://www.spotify.com/")
        || url.starts_with("https://www.last.fm/")
        || url.starts_with("https://music.youtube.com/")
        || url.starts_with("https://www.youtube.com/")
        || url.starts_with("https://musicbrainz.org/")
        || url.starts_with("https://www.deezer.com/")
        || url.starts_with("https://music.apple.com/")
        || url.starts_with("https://en.wikipedia.org/")
        || url.starts_with("https://sponsor.ajay.app/")
        // (where your own keys are made: the key setup's pages)
        || url.starts_with("https://developer.spotify.com/")
        || url.starts_with("https://console.cloud.google.com/")
        // (the chatbot's: Twitch's and Kick's sign-ins and developer pages, and the donation services' settings)
        || url.starts_with("https://www.twitch.tv/")
        || url.starts_with("https://id.twitch.tv/")
        || url.starts_with("https://dev.twitch.tv/")
        || url.starts_with("https://kick.com/")
        || url.starts_with("https://id.kick.com/")
        || url.starts_with("https://streamlabs.com/")
        || url.starts_with("https://streamelements.com/")
        // (OmniPulse's own pages, from its widget server on this PC: the Panels' page, to save them as pictures)
        || url.starts_with("http://127.0.0.1:")
        || url.starts_with("http://localhost:")
        || url.starts_with("https://ko-fi.com/")
        || url.starts_with("https://shwyl.ie/")
        || url.starts_with("https://help.throne.com/")
        || url.starts_with("https://throne.com/")
        || url.starts_with("https://docs.fourthwall.com/")
        || url.starts_with("https://fourthwall.com/"))
    {
        return Err("Only links to the services OmniPulse works with can be opened from it".to_string());
    }
    open_browser(&url)
}

// ---------- app settings (read at startup) ----------

#[derive(Serialize, Deserialize, Clone)]
pub struct Settings {
    #[serde(default = "default_true")]
    pub gpu: bool,
}

pub fn default_true() -> bool {
    true
}

pub fn load_settings() -> Settings {
    data_dir()
        .ok()
        .and_then(|d| fs::read_to_string(d.join("settings.json")).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Settings { gpu: true })
}

pub fn get_settings() -> Settings {
    load_settings()
}

pub fn set_gpu(enabled: bool) -> Result<(), String> {
    let text = serde_json::to_string(&Settings { gpu: enabled }).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("settings.json"), text).map_err(|e| e.to_string())
}

// ---------- the web app's saved data, for the native app ----------
// The web app keeps its favourites, stats, settings and playlists on this PC in the WebView's own
// storage, which nothing else can read. It writes a copy here as it goes, and the native app reads it.
const WEB_EXPORT: &str = "web-export.json";
// ---------- playlist files (M3U and CSV, imported and exported from the app) ----------
const PLAYLIST_FILE_MAX: u64 = 16 * 1024 * 1024;

fn playlist_file_ok(path: &str) -> Result<(), String> {
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if ["m3u", "m3u8", "csv", "txt"].contains(&ext.as_str()) { Ok(()) } else { Err("Only .m3u, .m3u8, .csv and .txt playlist files".into()) }
}

/// A playlist file's text (a BOM and Windows' own code page aside: it's read as UTF-8, else Latin-1)
pub fn playlist_file_read(path: &str) -> Result<String, String> {
    playlist_file_ok(path)?;
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > PLAYLIST_FILE_MAX {
        return Err("That file is too big for a playlist".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let text = String::from_utf8(bytes.clone()).unwrap_or_else(|_| bytes.iter().map(|&b| b as char).collect());
    Ok(text.trim_start_matches('\u{feff}').to_string())
}

pub fn playlist_file_write(path: &str, text: &str) -> Result<(), String> {
    playlist_file_ok(path)?;
    fs::write(path, text).map_err(|e| e.to_string())
}

const WEB_EXPORT_MAX: usize = 32 * 1024 * 1024;

pub fn save_web_export(text: &str) -> Result<(), String> {
    if text.len() > WEB_EXPORT_MAX {
        return Err("too big".into());
    }
    serde_json::from_str::<serde_json::Value>(text).map_err(|e| format!("not JSON: {e}"))?;
    let dir = data_dir()?;
    let tmp = dir.join(format!("{WEB_EXPORT}.tmp"));
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, dir.join(WEB_EXPORT)).map_err(|e| e.to_string())
}

/// What the web app last wrote: { "sc.favourites": "...", "sc.stats": "...", ... } (each value as it
/// was stored, a JSON string), with "savedAt"
pub fn load_web_export() -> Option<serde_json::Value> {
    let text = std::fs::read_to_string(data_dir().ok()?.join(WEB_EXPORT)).ok()?;
    serde_json::from_str(&text).ok()
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn the_old_folder_moves_and_only_your_keys_stay() {
        let _turn = TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("omnipulse-cfg-{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);
        fs::create_dir_all(tmp.join("sc-client")).unwrap();
        let theirs = bundled::soundcloud();
        let mut old = serde_json::json!({ "spotify_client_id": "mine", "discord_client_id": bundled::DISCORD_CLIENT_ID });
        if let Some((id, _)) = &theirs {
            old["client_id"] = serde_json::json!(id);
        }
        fs::write(tmp.join("sc-client").join("config.json"), old.to_string()).unwrap();
        fs::write(tmp.join("sc-client").join("tokens.json"), "{}").unwrap();
        // (only now: other tests read the data folder, and would move it half-made)
        std::env::set_var("APPDATA", &tmp);

        let dir = data_dir().unwrap();
        assert_eq!(dir, tmp.join("OmniPulse"));
        assert!(!tmp.join("sc-client").exists() && dir.join("tokens.json").exists());
        let kept: serde_json::Value = serde_json::from_str(&fs::read_to_string(dir.join("config.json")).unwrap()).unwrap();
        assert_eq!(kept["spotify_client_id"], "mine");
        assert!(kept.get("discord_client_id").is_none());
        if theirs.is_some() {
            assert!(kept.get("client_id").is_none());
            // (the app's own: finished by the relay, no secret in it)
            let cfg = load_config().unwrap();
            assert!(cfg.client_secret.is_empty() && cfg.sc_relay == bundled::RELAY_URL);
        }
        // and the app's own still fill in, unwritten
        assert_eq!(load_config().unwrap().discord_client_id, bundled::DISCORD_CLIENT_ID);

        // keys set up in the app: only the allowed ones, and an empty one removes it
        let mut f = serde_json::Map::new();
        f.insert("lastfm_api_key".into(), serde_json::json!("k"));
        f.insert("lastfm_secret".into(), serde_json::json!("s"));
        config_set(&f).unwrap();
        assert_eq!(config_status()["lastfm"], true);
        let mut bad = serde_json::Map::new();
        bad.insert("client_secret".into(), serde_json::json!("x"));
        assert!(config_set(&bad).is_err());
        let mut clear = serde_json::Map::new();
        clear.insert("lastfm_secret".into(), serde_json::json!(""));
        config_set(&clear).unwrap();
        assert_eq!(config_status()["lastfm"], false);
        let _ = fs::remove_dir_all(&tmp);
    }
}

#[cfg(test)]
mod crash_log_tests {
    #[test]
    fn a_panic_in_a_thread_is_written_down_and_the_rest_goes_on() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("omnipulse-crash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        *crate::DATA_DIR.lock().unwrap() = Some(tmp.clone());
        super::crash_log_install();
        let r = std::thread::Builder::new().name("spotify-test".into()).spawn(|| { let v: Vec<u8> = Vec::new(); v[3] }).unwrap().join();
        assert!(r.is_err(), "the thread ended");
        let log = std::fs::read_to_string(tmp.join("crash.log")).unwrap();
        assert!(log.contains("thread spotify-test") && log.contains("index out of bounds") && log.contains("lib.rs"), "{log}");
        *crate::DATA_DIR.lock().unwrap() = None;
        let _ = std::panic::take_hook();
    }
}
