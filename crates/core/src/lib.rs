// The backend shared by the Tauri app (src-tauri) and the native app (crates/app): accounts,
// SoundCloud / Spotify / Last.fm / Discord / lyrics, local music, and the local servers.
// Nothing here knows about a window; the apps pass in callbacks where the page used to be called.

pub mod discord;
pub mod lastfm;
pub mod local;
pub mod loudness;
pub mod lyrics;
pub mod plugins;
pub mod remote;
pub mod requests;
pub mod soundcloud;
pub mod spotify;
pub mod spotify_engine;
pub mod tools;
pub mod widget;
pub mod youtube;
pub mod yt_stream;

use serde::{Deserialize, Serialize};

/// Tests that point APPDATA at a folder of their own take turns
#[cfg(test)]
pub(crate) static TEST_ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());
use std::{fs, path::PathBuf};

// ---------- config and token storage ----------
// Config lives in %APPDATA%\sc-client\config.json:
//   { "client_id": "...", "client_secret": "..." }
// The secret stays on this PC and is only read by the Rust side, never the web page.

#[derive(Deserialize)]
pub struct Config {
    pub client_id: String,
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

pub fn data_dir() -> Result<PathBuf, String> {
    let base = std::env::var("APPDATA").map_err(|_| "APPDATA is not set".to_string())?;
    let dir = PathBuf::from(base).join("sc-client");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

pub fn load_config() -> Result<Config, String> {
    let path = data_dir()?.join("config.json");
    let text = fs::read_to_string(&path)
        .map_err(|_| format!("Missing config file: {}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("Bad config.json: {e}"))
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
pub fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://soundcloud.com/")
        || url.starts_with("https://www.soundcloud.com/")
        || url.starts_with("https://open.spotify.com/")
        || url.starts_with("https://www.spotify.com/")
        || url.starts_with("https://www.last.fm/")
        || url.starts_with("https://music.youtube.com/"))
    {
        return Err("Only SoundCloud, Spotify, YouTube Music and Last.fm links can be opened".to_string());
    }
    opener::open(&url).map_err(|e| e.to_string())
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
