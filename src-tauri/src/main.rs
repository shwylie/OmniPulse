#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod spotify_engine;
mod yt_stream;
mod remote;
use spotify_engine::*;
use yt_stream::resolve_yt_stream;
use remote::{remote_result, remote_start, remote_stop};

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_global_shortcut::{Code, Shortcut, ShortcutState};

// ---------- config and token storage ----------
// Config lives in %APPDATA%\sc-client\config.json:
//   { "client_id": "...", "client_secret": "..." }
// The secret stays on this PC and is only read by the Rust side, never the web page.

#[derive(Deserialize)]
struct Config {
    client_id: String,
    client_secret: String,
    #[serde(default = "default_redirect")]
    redirect_uri: String,
    #[serde(default)]
    spotify_client_id: String,
    #[serde(default = "default_spotify_redirect")]
    spotify_redirect_uri: String,
    #[serde(default)]
    discord_client_id: String,
    #[serde(default)]
    lastfm_api_key: String,
    #[serde(default)]
    lastfm_secret: String,
}

fn default_spotify_redirect() -> String {
    "http://127.0.0.1:8081/callback".to_string()
}

fn default_redirect() -> String {
    "http://localhost:8080/callback".to_string()
}

#[derive(Serialize, Deserialize, Clone)]
struct Tokens {
    access_token: String,
    refresh_token: Option<String>,
}

pub(crate) fn data_dir() -> Result<PathBuf, String> {
    let base = std::env::var("APPDATA").map_err(|_| "APPDATA is not set".to_string())?;
    let dir = PathBuf::from(base).join("sc-client");
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

fn load_config() -> Result<Config, String> {
    let path = data_dir()?.join("config.json");
    let text = fs::read_to_string(&path)
        .map_err(|_| format!("Missing config file: {}", path.display()))?;
    serde_json::from_str(&text).map_err(|e| format!("Bad config.json: {e}"))
}

fn load_tokens() -> Option<Tokens> {
    let text = fs::read_to_string(data_dir().ok()?.join("tokens.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_tokens(t: &Tokens) -> Result<(), String> {
    let text = serde_json::to_string(t).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("tokens.json"), text).map_err(|e| e.to_string())
}

// ---------- OAuth 2.1 with PKCE ----------

async fn token_request(params: &[(&str, &str)]) -> Result<Tokens, String> {
    let res = reqwest::Client::new()
        .post("https://secure.soundcloud.com/oauth/token")
        .header("accept", "application/json; charset=utf-8")
        .form(params)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let body: Value = res.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Token error {status}: {body}"));
    }
    Ok(Tokens {
        access_token: body["access_token"]
            .as_str()
            .ok_or("No access_token in response")?
            .to_string(),
        refresh_token: body["refresh_token"].as_str().map(String::from),
    })
}

async fn refresh(cfg: &Config, old: &Tokens) -> Result<Tokens, String> {
    let rt = old
        .refresh_token
        .as_deref()
        .ok_or("No refresh token, please sign in again")?;
    let new = token_request(&[
        ("grant_type", "refresh_token"),
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("refresh_token", rt),
    ])
    .await?;
    let merged = Tokens {
        access_token: new.access_token,
        refresh_token: new.refresh_token.or_else(|| old.refresh_token.clone()),
    };
    save_tokens(&merged)?;
    Ok(merged)
}

fn bind_addr(redirect: &str) -> String {
    let port = redirect
        .split("://")
        .nth(1)
        .and_then(|rest| rest.split('/').next())
        .and_then(|host| host.split(':').nth(1))
        .unwrap_or("8080");
    format!("127.0.0.1:{port}")
}

#[tauri::command]
async fn login() -> Result<(), String> {
    let cfg = load_config()?;

    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let verifier = URL_SAFE_NO_PAD.encode(buf);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    rand::thread_rng().fill_bytes(&mut buf);
    let state = URL_SAFE_NO_PAD.encode(buf);

    let url = format!(
        "https://secure.soundcloud.com/authorize?client_id={}&redirect_uri={}&response_type=code&code_challenge={}&code_challenge_method=S256&state={}",
        urlencoding::encode(&cfg.client_id),
        urlencoding::encode(&cfg.redirect_uri),
        challenge,
        state
    );

    let listener = TcpListener::bind(bind_addr(&cfg.redirect_uri))
        .map_err(|e| format!("Could not listen for the sign-in callback (is the port busy?): {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    opener::open(&url).map_err(|e| e.to_string())?;

    let (code, got_state) = tauri::async_runtime::spawn_blocking(
        move || -> Result<(String, String), String> {
            let deadline = Instant::now() + Duration::from_secs(180);
            loop {
                if Instant::now() > deadline {
                    return Err("Sign-in timed out".to_string());
                }
                let mut stream = match listener.accept() {
                    Ok((s, _)) => s,
                    Err(e) if e.kind() == ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(200));
                        continue;
                    }
                    Err(e) => return Err(e.to_string()),
                };
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0u8; 4096];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req
                    .lines()
                    .next()
                    .unwrap_or("")
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("")
                    .to_string();
                let query = path.split('?').nth(1).unwrap_or("");
                let (mut code, mut st, mut err) = (String::new(), String::new(), String::new());
                for pair in query.split('&') {
                    if let Some((k, v)) = pair.split_once('=') {
                        let v = urlencoding::decode(v)
                            .map(|c| c.into_owned())
                            .unwrap_or_default();
                        match k {
                            "code" => code = v,
                            "state" => st = v,
                            "error" => err = v,
                            _ => {}
                        }
                    }
                }
                let body = "<html><body style=\"font-family:sans-serif;background:#0f1117;color:#e9eaf2;padding:40px\">Signed in. You can close this tab and go back to OmniPulse.</body></html>";
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes());
                if !err.is_empty() {
                    return Err(format!("SoundCloud returned an error: {err}"));
                }
                if !code.is_empty() {
                    return Ok((code, st));
                }
                // otherwise it was a favicon or preconnect request: keep waiting
            }
        },
    )
    .await
    .map_err(|e| e.to_string())??;

    if got_state != state {
        return Err("Sign-in state did not match, please try again".to_string());
    }

    let tokens = token_request(&[
        ("grant_type", "authorization_code"),
        ("client_id", cfg.client_id.as_str()),
        ("client_secret", cfg.client_secret.as_str()),
        ("redirect_uri", cfg.redirect_uri.as_str()),
        ("code_verifier", verifier.as_str()),
        ("code", code.as_str()),
    ])
    .await?;
    save_tokens(&tokens)
}

#[tauri::command]
fn signed_in() -> bool {
    load_tokens().is_some()
}

#[tauri::command]
fn logout() -> Result<(), String> {
    let path = data_dir()?.join("tokens.json");
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- API calls ----------

async fn authed_request(method: reqwest::Method, path: &str) -> Result<reqwest::Response, String> {
    if !path.starts_with('/') {
        return Err("Bad API path".to_string());
    }
    let cfg = load_config()?;
    let mut tokens = load_tokens().ok_or("Not signed in")?;
    let url = format!("https://api.soundcloud.com{path}");
    // Redirects are off so stream_url can read the Location header.
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| e.to_string())?;

    let send = |token: String| {
        client
            .request(method.clone(), &url)
            .header("Authorization", format!("OAuth {token}"))
            .header("accept", "application/json; charset=utf-8")
            .send()
    };

    let mut res = send(tokens.access_token.clone())
        .await
        .map_err(|e| e.to_string())?;
    if res.status().as_u16() == 401 {
        tokens = refresh(&cfg, &tokens).await?;
        res = send(tokens.access_token.clone())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(res)
}

async fn authed_get(path: &str) -> Result<reqwest::Response, String> {
    authed_request(reqwest::Method::GET, path).await
}

#[tauri::command]
async fn api_get(path: String) -> Result<Value, String> {
    let res = authed_get(&path).await?;
    let status = res.status();
    let body = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("API error {status}: {body}"));
    }
    serde_json::from_str(&body).map_err(|e| format!("Bad JSON from API: {e}"))
}

// Full-length audio is listed by /tracks/{id}/streams, usually in several formats.
// The page tries them in order until one plays. The plain /stream address only gives a short preview.
fn collect_streams(v: &Value) -> Vec<(String, String)> {
    let obj = match v
        .get("streams")
        .and_then(|s| s.as_object())
        .or_else(|| v.as_object())
    {
        Some(o) => o,
        None => return Vec::new(),
    };
    let order = [
        "hls_aac_160_url",
        "hls_mp3_128_url",
        "http_mp3_128_url",
        "hls_opus_64_url",
    ];
    let mut out: Vec<(String, String)> = Vec::new();
    for key in order {
        if let Some(u) = obj.get(key).and_then(|x| x.as_str()) {
            out.push((key.to_string(), u.to_string()));
        }
    }
    for (k, val) in obj {
        if k.ends_with("_url")
            && !k.contains("preview")
            && !k.contains("encrypted")
            && !out.iter().any(|(n, _)| n == k)
        {
            if let Some(u) = val.as_str() {
                out.push((k.clone(), u.to_string()));
            }
        }
    }
    out
}

// Stream links on api.soundcloud.com need our token and redirect to the real file.
async fn resolve_api_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("https://api.soundcloud.com")?;
    let res = authed_get(rest).await.ok()?;
    if res.status().is_redirection() {
        return res
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
    }
    if res.status().is_success() {
        let text = res.text().await.ok()?;
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let Some(u) = v.get("url").and_then(|x| x.as_str()) {
                return Some(u.to_string());
            }
        }
    }
    None
}

#[tauri::command]
async fn stream_url(id: String) -> Result<Value, String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("Bad track id".to_string());
    }
    let mut note = String::new();
    let mut cands: Vec<(String, String)> = Vec::new();
    match authed_get(&format!("/tracks/{id}/streams")).await {
        Ok(res) => {
            let status = res.status();
            let body = res.text().await.unwrap_or_default();
            let short: String = body.chars().take(300).collect();
            if status.is_success() {
                match serde_json::from_str::<Value>(&body) {
                    Ok(v) => {
                        cands = collect_streams(&v);
                        if cands.is_empty() {
                            note = format!("No usable stream in the response: {short}");
                        }
                    }
                    Err(_) => note = format!("The streams response was not JSON: {short}"),
                }
            } else {
                note = format!("The streams request failed ({status}): {short}");
            }
        }
        Err(e) => note = e,
    }
    if cands.is_empty() {
        // Last resort: the short preview
        if let Ok(res) = authed_get(&format!("/tracks/{id}/stream")).await {
            if res.status().is_redirection() {
                if let Some(loc) = res.headers().get("location").and_then(|v| v.to_str().ok()) {
                    cands.push(("preview".to_string(), loc.to_string()));
                }
            }
        }
    }
    if cands.is_empty() {
        return Err(format!("No stream is available for this track. {note}"));
    }
    let list: Vec<Value> = cands
        .into_iter()
        .map(|(name, url)| serde_json::json!({ "name": name, "url": url }))
        .collect();
    Ok(serde_json::json!({ "candidates": list, "note": note }))
}

#[tauri::command]
async fn stream_resolve(url: String) -> Result<Value, String> {
    let final_url = resolve_api_url(&url).await.unwrap_or_else(|| url.clone());
    let auth = if final_url.starts_with("https://api.soundcloud.com") {
        load_tokens().map(|t| format!("OAuth {}", t.access_token))
    } else {
        None
    };
    Ok(serde_json::json!({ "url": final_url, "auth": auth }))
}

// Like (POST) or unlike (DELETE) a track.
#[tauri::command]
async fn set_like(id: String, like: bool) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("Bad track id".to_string());
    }
    let method = if like { reqwest::Method::POST } else { reqwest::Method::DELETE };
    let res = authed_request(method, &format!("/likes/tracks/{id}")).await?;
    let status = res.status();
    if status.is_success() {
        return Ok(());
    }
    let body = res.text().await.unwrap_or_default();
    Err(format!("Like request failed ({status}): {body}"))
}

// Follow (PUT) or unfollow (DELETE) an artist.
#[tauri::command]
async fn set_follow(id: String, follow: bool) -> Result<(), String> {
    if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
        return Err("Bad user id".to_string());
    }
    let method = if follow { reqwest::Method::PUT } else { reqwest::Method::DELETE };
    let res = authed_request(method, &format!("/me/followings/{id}")).await?;
    let status = res.status();
    if status.is_success() {
        return Ok(());
    }
    let body = res.text().await.unwrap_or_default();
    Err(format!("Follow request failed ({status}): {body}"))
}

// Create (POST /playlists), change (PUT /playlists/{id}) or delete (DELETE) your own playlists.
#[tauri::command]
async fn playlist_write(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    let ok_path = path == "/playlists"
        || path
            .strip_prefix("/playlists/")
            .map(|r| !r.is_empty() && r.chars().all(|c| c.is_ascii_digit()))
            .unwrap_or(false);
    if !ok_path {
        return Err("That path is not allowed".to_string());
    }
    let m = match method.as_str() {
        "POST" => reqwest::Method::POST,
        "PUT" => reqwest::Method::PUT,
        "DELETE" => reqwest::Method::DELETE,
        _ => return Err("That method is not allowed".to_string()),
    };
    let cfg = load_config()?;
    let mut tokens = load_tokens().ok_or("Not signed in")?;
    let url = format!("https://api.soundcloud.com{path}");
    let client = reqwest::Client::new();
    let send = |token: String| {
        let mut rb = client
            .request(m.clone(), &url)
            .header("Authorization", format!("OAuth {token}"))
            .header("accept", "application/json; charset=utf-8");
        if let Some(b) = &body {
            rb = rb.json(b);
        }
        rb.send()
    };
    let mut res = send(tokens.access_token.clone()).await.map_err(|e| e.to_string())?;
    if res.status().as_u16() == 401 {
        tokens = refresh(&cfg, &tokens).await?;
        res = send(tokens.access_token.clone()).await.map_err(|e| e.to_string())?;
    }
    let status = res.status();
    let text = res.text().await.unwrap_or_default();
    if !status.is_success() {
        let short: String = text.chars().take(300).collect();
        return Err(format!("SoundCloud said {status}: {short}"));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    Ok(serde_json::from_str(&text).unwrap_or(Value::Null))
}

// Opens a SoundCloud page in the default browser (only soundcloud.com links).
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    if !(url.starts_with("https://soundcloud.com/")
        || url.starts_with("https://www.soundcloud.com/")
        || url.starts_with("https://open.spotify.com/")
        || url.starts_with("https://www.spotify.com/")
        || url.starts_with("https://www.last.fm/"))
    {
        return Err("Only SoundCloud, Spotify and Last.fm links can be opened".to_string());
    }
    opener::open(&url).map_err(|e| e.to_string())
}

// ---------- Spotify (kept separate from SoundCloud) ----------

#[derive(Serialize, Deserialize, Clone)]
struct SpTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn load_sp_tokens() -> Option<SpTokens> {
    let text = fs::read_to_string(data_dir().ok()?.join("spotify_tokens.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_sp_tokens(t: &SpTokens) -> Result<(), String> {
    let text = serde_json::to_string(t).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("spotify_tokens.json"), text).map_err(|e| e.to_string())
}

async fn sp_token_request(params: &[(&str, &str)]) -> Result<(String, Option<String>, u64), String> {
    let res = reqwest::Client::new()
        .post("https://accounts.spotify.com/api/token")
        .form(params)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let body: Value = res.json().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Spotify token error {status}: {body}"));
    }
    let access = body["access_token"]
        .as_str()
        .ok_or("No access_token from Spotify")?
        .to_string();
    let refresh = body["refresh_token"].as_str().map(String::from);
    let expires_in = body["expires_in"].as_u64().unwrap_or(3600);
    Ok((access, refresh, now_secs() + expires_in))
}

async fn sp_refresh(cfg: &Config, old: &SpTokens) -> Result<SpTokens, String> {
    let rt = old
        .refresh_token
        .as_deref()
        .ok_or("No Spotify refresh token, please connect again")?;
    let (access, refresh, exp) = sp_token_request(&[
        ("grant_type", "refresh_token"),
        ("refresh_token", rt),
        ("client_id", cfg.spotify_client_id.as_str()),
    ])
    .await?;
    let t = SpTokens {
        access_token: access,
        refresh_token: refresh.or_else(|| old.refresh_token.clone()),
        expires_at: exp,
    };
    save_sp_tokens(&t)?;
    Ok(t)
}

// Opens the sign-in page and waits for the browser to come back to our local address.
async fn wait_for_callback(bind: String, url: String) -> Result<(String, String), String> {
    let listener = TcpListener::bind(&bind)
        .map_err(|e| format!("Could not listen for the sign-in callback (is the port busy?): {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    opener::open(&url).map_err(|e| e.to_string())?;

    let result = tauri::async_runtime::spawn_blocking(move || -> Result<(String, String), String> {
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if Instant::now() > deadline {
                return Err("Sign-in timed out".to_string());
            }
            let mut stream = match listener.accept() {
                Ok((s, _)) => s,
                Err(e) if e.kind() == ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(200));
                    continue;
                }
                Err(e) => return Err(e.to_string()),
            };
            let _ = stream.set_nonblocking(false);
            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req
                .lines()
                .next()
                .unwrap_or("")
                .split_whitespace()
                .nth(1)
                .unwrap_or("")
                .to_string();
            let query = path.split('?').nth(1).unwrap_or("");
            let (mut code, mut st, mut err) = (String::new(), String::new(), String::new());
            for pair in query.split('&') {
                if let Some((k, v)) = pair.split_once('=') {
                    let v = urlencoding::decode(v)
                        .map(|c| c.into_owned())
                        .unwrap_or_default();
                    match k {
                        "code" => code = v,
                        "state" => st = v,
                        "error" => err = v,
                        _ => {}
                    }
                }
            }
            let body = "<html><body style=\"font-family:sans-serif;background:#0f1117;color:#e9eaf2;padding:40px\">Connected. You can close this tab and go back to OmniPulse.</body></html>";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = stream.write_all(resp.as_bytes());
            if !err.is_empty() {
                return Err(format!("Spotify returned an error: {err}"));
            }
            if !code.is_empty() {
                return Ok((code, st));
            }
        }
    })
    .await
    .map_err(|e| e.to_string())?;
    result
}

#[tauri::command]
async fn spotify_login() -> Result<(), String> {
    let cfg = load_config()?;
    if cfg.spotify_client_id.is_empty() {
        return Err("Add \"spotify_client_id\" to config.json first (see the README)".to_string());
    }

    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let verifier = URL_SAFE_NO_PAD.encode(buf);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    rand::thread_rng().fill_bytes(&mut buf);
    let state = URL_SAFE_NO_PAD.encode(buf);

    let scope = "streaming user-read-email user-read-private user-library-read user-library-modify user-read-playback-state user-modify-playback-state playlist-read-private playlist-read-collaborative";
    let url = format!(
        "https://accounts.spotify.com/authorize?response_type=code&client_id={}&scope={}&redirect_uri={}&code_challenge_method=S256&code_challenge={}&state={}",
        urlencoding::encode(&cfg.spotify_client_id),
        urlencoding::encode(scope),
        urlencoding::encode(&cfg.spotify_redirect_uri),
        challenge,
        state
    );

    let (code, got_state) = wait_for_callback(bind_addr(&cfg.spotify_redirect_uri), url).await?;
    if got_state != state {
        return Err("Spotify sign-in state did not match, please try again".to_string());
    }

    let (access, refresh, exp) = sp_token_request(&[
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", cfg.spotify_redirect_uri.as_str()),
        ("client_id", cfg.spotify_client_id.as_str()),
        ("code_verifier", verifier.as_str()),
    ])
    .await?;
    save_sp_tokens(&SpTokens {
        access_token: access,
        refresh_token: refresh,
        expires_at: exp,
    })
}

#[tauri::command]
fn spotify_signed_in() -> bool {
    load_sp_tokens().is_some()
}

#[tauri::command]
fn spotify_logout() -> Result<(), String> {
    let path = data_dir()?.join("spotify_tokens.json");
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// The Spotify player in the page asks for a token whenever it needs one
#[tauri::command]
async fn spotify_token() -> Result<String, String> {
    let cfg = load_config()?;
    let mut t = load_sp_tokens().ok_or("Not connected to Spotify")?;
    if t.expires_at < now_secs() + 120 {
        t = sp_refresh(&cfg, &t).await?;
    }
    Ok(t.access_token)
}

#[tauri::command]
async fn spotify_api(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    let allowed = ["/me", "/search", "/tracks", "/albums", "/artists", "/playlists"];
    if !path.starts_with('/') || !allowed.iter().any(|p| path.starts_with(p)) {
        return Err("That Spotify path is not allowed".to_string());
    }
    let m = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "PUT" => reqwest::Method::PUT,
        "POST" => reqwest::Method::POST,
        _ => return Err("That method is not allowed".to_string()),
    };
    let cfg = load_config()?;
    let mut t = load_sp_tokens().ok_or("Not connected to Spotify")?;
    if t.expires_at < now_secs() + 60 {
        t = sp_refresh(&cfg, &t).await?;
    }

    let url = format!("https://api.spotify.com/v1{path}");
    let client = reqwest::Client::new();
    let send = |token: String| {
        let mut rb = client.request(m.clone(), &url).bearer_auth(token);
        if let Some(b) = &body {
            rb = rb.json(b);
        }
        rb.send()
    };

    let mut res = send(t.access_token.clone()).await.map_err(|e| e.to_string())?;
    if res.status().as_u16() == 401 {
        t = sp_refresh(&cfg, &t).await?;
        res = send(t.access_token.clone()).await.map_err(|e| e.to_string())?;
    }
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Spotify error {status}: {text}"));
    }
    if text.trim().is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_str(&text).map_err(|e| format!("Bad JSON from Spotify: {e}"))
}

// ---------- Extras: lyrics lookup, Discord activity, Last.fm ----------

// Lets the page look up lyrics without running into cross-site rules
#[tauri::command]
async fn lyrics_get(url: String) -> Result<Value, String> {
    if !url.starts_with("https://lrclib.net/api/") {
        return Err("That address is not allowed".to_string());
    }
    let res = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "OmniPulse (personal music player)")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Lyrics lookup failed ({status})"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

// --- Discord activity, spoken directly to the local Discord client ---
// The name shown on your profile ("Listening to ...") is the name of the Discord
// application you create, so name that application whatever you like (for example "Music").

static DISCORD: Mutex<Option<fs::File>> = Mutex::new(None);

fn discord_frame(op: u32, payload: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

fn discord_read(f: &mut fs::File) -> Result<Value, String> {
    let mut head = [0u8; 8];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let mut body = vec![0u8; len.min(1 << 20)];
    f.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(serde_json::from_slice(&body).unwrap_or(Value::Null))
}

fn discord_connect(client_id: &str) -> Result<fs::File, String> {
    for i in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{i}");
        if let Ok(mut f) = fs::OpenOptions::new().read(true).write(true).open(&path) {
            let hello = serde_json::json!({ "v": 1, "client_id": client_id }).to_string();
            f.write_all(&discord_frame(0, &hello)).map_err(|e| e.to_string())?;
            let reply = discord_read(&mut f)?;
            if reply["evt"] == "READY" {
                return Ok(f);
            }
            let msg = reply["message"]
                .as_str()
                .or_else(|| reply["data"]["message"].as_str())
                .unwrap_or("no reason given")
                .to_string();
            return Err(format!("Discord refused the connection ({msg}). Check discord_client_id in config.json."));
        }
    }
    Err("Discord isn't running, or it's running as a different Windows user or as administrator.".to_string())
}

// Runs a Discord exchange on its own thread so a silent Discord can't hang anything
fn with_timeout<T: Send + 'static>(
    secs: u64,
    what: &str,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(r) => r,
        Err(_) => Err(format!("Discord didn't answer within {secs} seconds ({what}).")),
    }
}

fn discord_exchange(f: &mut fs::File, activity: &Option<Value>) -> Result<Value, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .to_string();
    let payload = serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": std::process::id(), "activity": activity },
        "nonce": nonce
    })
    .to_string();
    f.write_all(&discord_frame(1, &payload)).map_err(|e| e.to_string())?;
    discord_read(f)
}

fn discord_send(client_id: &str, activity: Option<Value>) -> Result<(), String> {
    // Take the open connection out, so nothing else waits on it while we talk to Discord
    let existing = DISCORD.lock().map_err(|_| "Discord lock failed".to_string())?.take();
    let id = client_id.to_string();
    let (file, reply) = with_timeout(4, "sending the activity", move || {
        if let Some(mut f) = existing {
            if let Ok(reply) = discord_exchange(&mut f, &activity) {
                return Ok((f, reply));
            }
        }
        let mut f = discord_connect(&id)?;
        let reply = discord_exchange(&mut f, &activity)?;
        Ok((f, reply))
    })?;
    if let Ok(mut g) = DISCORD.lock() {
        *g = Some(file);
    }
    if reply["evt"] == "ERROR" {
        let msg = reply["data"]["message"].as_str().unwrap_or("no reason given");
        return Err(format!("Discord rejected the activity: {msg}"));
    }
    Ok(())
}

fn discord_id() -> String {
    load_config().map(|c| c.discord_client_id).unwrap_or_default()
}

#[derive(Deserialize)]
struct PresenceButton {
    label: String,
    url: String,
}

// Returns a note when Discord only accepted a simpler version of the activity
#[tauri::command]
async fn presence_set(
    details: String,
    state: String,
    start: Option<i64>,
    end: Option<i64>,
    image: Option<String>,
    buttons: Option<Vec<PresenceButton>>,
) -> Result<String, String> {
    let id = discord_id();
    if id.trim().is_empty() {
        return Err("Add discord_client_id to config.json (see the README).".to_string());
    }
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        let fix = |s: String| -> String {
            let t: String = s.chars().take(120).collect();
            if t.trim().is_empty() {
                "Unknown".to_string()
            } else if t.chars().count() < 2 {
                format!("{t}\u{200B}")
            } else {
                t
            }
        };
        // type 2 = "Listening to"
        let mut full = serde_json::json!({ "type": 2, "details": fix(details), "state": fix(state) });
        let mut ts = serde_json::Map::new();
        if let Some(s) = start {
            ts.insert("start".to_string(), s.into());
        }
        if let Some(e) = end {
            ts.insert("end".to_string(), e.into());
        }
        if !ts.is_empty() {
            full["timestamps"] = Value::Object(ts);
        }
        if let Some(img) = image {
            if !img.is_empty() {
                full["assets"] = serde_json::json!({ "large_image": img });
            }
        }
        // Buttons other people can click (Discord allows two; labels up to 32 characters)
        let ok_url = |u: &str| {
            u.len() <= 512
                && (u.starts_with("https://open.spotify.com/") || u.starts_with("https://soundcloud.com/") || u.starts_with("https://www.soundcloud.com/"))
        };
        let list: Vec<Value> = buttons
            .unwrap_or_default()
            .into_iter()
            .filter(|b| ok_url(&b.url) && !b.label.trim().is_empty())
            .take(2)
            .map(|b| serde_json::json!({ "label": b.label.chars().take(32).collect::<String>(), "url": b.url }))
            .collect();
        if !list.is_empty() {
            full["buttons"] = Value::Array(list);
        }

        // Try the full activity, then simpler versions if Discord rejects a part of it
        match discord_send(&id, Some(full.clone())) {
            Ok(()) => return Ok(String::new()),
            Err(e) if !e.starts_with("Discord rejected") => return Err(e),
            Err(_) => {}
        }
        let had_link = full.get("buttons").is_some();
        let mut no_link = full;
        if let Some(o) = no_link.as_object_mut() {
            o.remove("buttons");
        }
        if had_link {
            match discord_send(&id, Some(no_link.clone())) {
                Ok(()) => return Ok("Showing without the buttons (Discord refused them).".to_string()),
                Err(e) if !e.starts_with("Discord rejected") => return Err(e),
                Err(_) => {}
            }
        }
        let mut no_art = no_link;
        if let Some(o) = no_art.as_object_mut() {
            o.remove("assets");
        }
        match discord_send(&id, Some(no_art.clone())) {
            Ok(()) => return Ok("Showing without artwork (Discord refused the cover image).".to_string()),
            Err(e) if !e.starts_with("Discord rejected") => return Err(e),
            Err(_) => {}
        }
        let mut plain = no_art;
        if let Some(o) = plain.as_object_mut() {
            o.remove("type");
            o.remove("timestamps");
        }
        discord_send(&id, Some(plain))
            .map(|_| "Showing as \"Playing\" (Discord refused the \"Listening to\" style).".to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

#[tauri::command]
async fn presence_clear() -> Result<(), String> {
    let id = discord_id();
    if id.trim().is_empty() {
        return Ok(());
    }
    let r = tauri::async_runtime::spawn_blocking(move || discord_send(&id, None))
        .await
        .map_err(|e| e.to_string())?;
    r
}

// Settings > Discord > Test: shows exactly what Discord answers
#[tauri::command]
async fn presence_test() -> Result<String, String> {
    let id = discord_id();
    if id.trim().is_empty() {
        return Err("discord_client_id is missing from config.json.".to_string());
    }
    let r = tauri::async_runtime::spawn_blocking(move || -> Result<String, String> {
        if let Ok(mut g) = DISCORD.lock() {
            *g = None; // start from a fresh connection
        }
        let test = Some(serde_json::json!({
            "type": 2,
            "details": "Connection test",
            "state": "If you can see this, it works"
        }));
        let (file, reply) = with_timeout(5, "connecting and sending a test", move || {
            let mut f = discord_connect(&id)?;
            let reply = discord_exchange(&mut f, &test)?;
            Ok((f, reply))
        })?;
        if let Ok(mut g) = DISCORD.lock() {
            *g = Some(file);
        }
        let text: String = reply.to_string().chars().take(400).collect();
        Ok(format!("Connected. Discord replied: {text}"))
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

// --- Last.fm ---

#[derive(Serialize, Deserialize, Clone)]
struct LfmSession {
    key: String,
    name: String,
}

fn load_lfm() -> Option<LfmSession> {
    let text = fs::read_to_string(data_dir().ok()?.join("lastfm.json")).ok()?;
    serde_json::from_str(&text).ok()
}

fn save_lfm(s: &LfmSession) -> Result<(), String> {
    let text = serde_json::to_string(s).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("lastfm.json"), text).map_err(|e| e.to_string())
}

fn lfm_sign(params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params
        .iter()
        .filter(|(k, _)| k != "format" && k != "callback")
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut s = String::new();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    format!("{:x}", md5::compute(s.as_bytes()))
}

async fn lfm_request(
    cfg: &Config,
    method: &str,
    mut params: Vec<(String, String)>,
    signed: bool,
    post: bool,
) -> Result<Value, String> {
    if cfg.lastfm_api_key.is_empty() {
        return Err("Add lastfm_api_key (and lastfm_secret) to config.json first".to_string());
    }
    params.push(("method".to_string(), method.to_string()));
    params.push(("api_key".to_string(), cfg.lastfm_api_key.clone()));
    if signed {
        if cfg.lastfm_secret.is_empty() {
            return Err("Add lastfm_secret to config.json first".to_string());
        }
        if method != "auth.getToken" && method != "auth.getSession" {
            if let Some(s) = load_lfm() {
                params.push(("sk".to_string(), s.key));
            }
        }
        let sig = lfm_sign(&params, &cfg.lastfm_secret);
        params.push(("api_sig".to_string(), sig));
    }
    params.push(("format".to_string(), "json".to_string()));
    let client = reqwest::Client::new();
    let req = if post {
        client.post("https://ws.audioscrobbler.com/2.0/").form(&params)
    } else {
        client.get("https://ws.audioscrobbler.com/2.0/").query(&params)
    };
    let res = req.send().await.map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| format!("Last.fm replied {status}: {text}"))?;
    if let Some(code) = v.get("error") {
        return Err(format!(
            "Last.fm error {}: {}",
            code,
            v["message"].as_str().unwrap_or("")
        ));
    }
    Ok(v)
}

#[tauri::command]
fn lastfm_status() -> Value {
    match load_lfm() {
        Some(s) => serde_json::json!({ "connected": true, "name": s.name }),
        None => serde_json::json!({ "connected": false, "name": "" }),
    }
}

#[tauri::command]
fn lastfm_disconnect() -> Result<(), String> {
    let path = data_dir()?.join("lastfm.json");
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Opens Last.fm in the browser and waits (up to two minutes) for you to approve
#[tauri::command]
async fn lastfm_connect() -> Result<String, String> {
    let cfg = load_config()?;
    let tok = lfm_request(&cfg, "auth.getToken", Vec::new(), true, false).await?;
    let token = tok["token"].as_str().ok_or("No token from Last.fm")?.to_string();
    let url = format!(
        "https://www.last.fm/api/auth/?api_key={}&token={}",
        urlencoding::encode(&cfg.lastfm_api_key),
        urlencoding::encode(&token)
    );
    opener::open(&url).map_err(|e| e.to_string())?;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let params = vec![("token".to_string(), token.clone())];
        match lfm_request(&cfg, "auth.getSession", params, true, false).await {
            Ok(v) => {
                let key = v["session"]["key"]
                    .as_str()
                    .ok_or("No session key from Last.fm")?
                    .to_string();
                let name = v["session"]["name"].as_str().unwrap_or("").to_string();
                save_lfm(&LfmSession { key, name: name.clone() })?;
                return Ok(name);
            }
            Err(e) => {
                if !e.contains("error 14") {
                    return Err(e);
                }
            }
        }
    }
    Err("Timed out waiting for you to approve in the browser".to_string())
}

#[tauri::command]
async fn lastfm_call(method: String, params: Vec<(String, String)>) -> Result<Value, String> {
    let cfg = load_config()?;
    let (signed, post) = match method.as_str() {
        "track.updateNowPlaying" | "track.scrobble" => {
            if load_lfm().is_none() {
                return Err("Not connected to Last.fm".to_string());
            }
            (true, true)
        }
        "artist.getSimilar" | "artist.getTopTags" => (false, false),
        _ => return Err("That Last.fm method is not allowed".to_string()),
    };
    lfm_request(&cfg, &method, params, signed, post).await
}

// Media keys, the tray menu and the taskbar buttons all go through this:
// it calls window.__mediaAction(...) in the page directly.
fn media_action(app: &AppHandle, action: &str) {
    if let Some(w) = app.get_webview_window("main") {
        let js = format!("window.__mediaAction && window.__mediaAction('{action}')");
        std::thread::spawn(move || {
            let _ = w.eval(&js);
        });
    }
}

// Keeps the taskbar buttons' icons in step with the player
#[tauri::command]
fn taskbar_state(app: AppHandle, playing: bool, shuffle: bool, liked: bool, can_like: bool) {
    #[cfg(windows)]
    {
        let bits = (playing as usize)
            | ((shuffle as usize) << 1)
            | ((liked as usize) << 2)
            | ((can_like as usize) << 3);
        if let Some(w) = app.get_webview_window("main") {
            taskbar_buttons::set_state(&w, bits);
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (app, playing, shuffle, liked, can_like);
    }
}

// ---------- Taskbar thumbnail buttons (Windows) ----------
// Shown when you hover the app on the taskbar, like VLC.
#[cfg(windows)]
mod taskbar_buttons {
    use std::sync::atomic::{AtomicU32, Ordering};
    use tauri::{path::BaseDirectory, Manager, WebviewWindow, Wry};
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows::Win32::UI::Shell::{
        DefSubclassProc, ITaskbarList3, SetWindowSubclass, TaskbarList, THBF_DISABLED, THBF_ENABLED,
        THBN_CLICKED, THB_FLAGS, THB_ICON, THB_TOOLTIP, THUMBBUTTON,
    };
    use windows::Win32::UI::WindowsAndMessaging::{
        LoadImageW, PostMessageW, RegisterWindowMessageW, HICON, IMAGE_ICON, LR_DEFAULTSIZE,
        LR_LOADFROMFILE, WM_COMMAND, WM_USER,
    };

    const SUBCLASS_ID: usize = 7311;
    const BTN_SHUFFLE: u32 = 2001;
    const BTN_PREV: u32 = 2002;
    const BTN_PLAY: u32 = 2003;
    const BTN_NEXT: u32 = 2004;
    const BTN_LIKE: u32 = 2005;
    const WM_UPDATE_STATE: u32 = WM_USER + 21;

    static CREATED_MSG: AtomicU32 = AtomicU32::new(0);

    struct Icons {
        shuffle_off: HICON,
        shuffle_on: HICON,
        prev: HICON,
        play: HICON,
        pause: HICON,
        next: HICON,
        like_off: HICON,
        like_on: HICON,
    }

    struct Ctx {
        window: WebviewWindow<Wry>,
        taskbar: Option<ITaskbarList3>,
        icons: Icons,
        state: usize,
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn set_tip(buf: &mut [u16; 260], s: &str) {
        let mut count = 0;
        for (i, c) in s.encode_utf16().enumerate().take(259) {
            buf[i] = c;
            count = i + 1;
        }
        buf[count] = 0;
    }

    fn load_icon(window: &WebviewWindow<Wry>, name: &str) -> Result<HICON, String> {
        let path = window
            .app_handle()
            .path()
            .resolve(format!("icons/taskbar/{name}.ico"), BaseDirectory::Resource)
            .map_err(|e| e.to_string())?;
        let wpath = wide(&path.to_string_lossy());
        unsafe {
            let h = LoadImageW(
                None,
                PCWSTR(wpath.as_ptr()),
                IMAGE_ICON,
                0,
                0,
                LR_LOADFROMFILE | LR_DEFAULTSIZE,
            )
            .map_err(|e| e.to_string())?;
            Ok(HICON(h.0))
        }
    }

    fn button(id: u32, icon: HICON, tip: &str, enabled: bool) -> THUMBBUTTON {
        let mut b = THUMBBUTTON {
            dwMask: THB_ICON | THB_TOOLTIP | THB_FLAGS,
            iId: id,
            hIcon: icon,
            szTip: [0; 260],
            dwFlags: if enabled { THBF_ENABLED } else { THBF_DISABLED },
            ..Default::default()
        };
        set_tip(&mut b.szTip, tip);
        b
    }

    fn buttons_for(icons: &Icons, bits: usize) -> [THUMBBUTTON; 5] {
        let playing = (bits & 1) != 0;
        let shuffle = (bits & 2) != 0;
        let liked = (bits & 4) != 0;
        let can_like = (bits & 8) != 0;
        [
            button(
                BTN_SHUFFLE,
                if shuffle { icons.shuffle_on } else { icons.shuffle_off },
                if shuffle { "Shuffle: on" } else { "Shuffle: off" },
                true,
            ),
            button(BTN_PREV, icons.prev, "Previous", true),
            button(
                BTN_PLAY,
                if playing { icons.pause } else { icons.play },
                if playing { "Pause" } else { "Play" },
                true,
            ),
            button(BTN_NEXT, icons.next, "Next", true),
            button(
                BTN_LIKE,
                if liked { icons.like_on } else { icons.like_off },
                if liked { "Remove from Likes" } else { "Like" },
                can_like,
            ),
        ]
    }

    unsafe extern "system" fn subclass_proc(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        data: usize,
    ) -> LRESULT {
        let ctx = &mut *(data as *mut Ctx);

        // The taskbar button exists (or Explorer restarted): add the buttons
        let created = CREATED_MSG.load(Ordering::Relaxed);
        if created != 0 && msg == created {
            let tb: windows::core::Result<ITaskbarList3> =
                CoCreateInstance(&TaskbarList, None, CLSCTX_INPROC_SERVER);
            if let Ok(tb) = tb {
                if tb.HrInit().is_ok() {
                    // Fails harmlessly if the buttons were already added
                    let _ = tb.ThumbBarAddButtons(hwnd, &buttons_for(&ctx.icons, ctx.state));
                    ctx.taskbar = Some(tb);
                }
            }
            return DefSubclassProc(hwnd, msg, wparam, lparam);
        }

        if msg == WM_UPDATE_STATE {
            ctx.state = wparam.0;
            if let Some(ref tb) = ctx.taskbar {
                let _ = tb.ThumbBarUpdateButtons(hwnd, &buttons_for(&ctx.icons, ctx.state));
            }
            return LRESULT(0);
        }

        if msg == WM_COMMAND {
            let high = (wparam.0 >> 16) & 0xFFFF;
            let low = (wparam.0 & 0xFFFF) as u32;
            if high as u32 == THBN_CLICKED {
                let action = match low {
                    BTN_SHUFFLE => "shuffle",
                    BTN_PREV => "prev",
                    BTN_PLAY => "play",
                    BTN_NEXT => "next",
                    BTN_LIKE => "like",
                    _ => "",
                };
                if !action.is_empty() {
                    // Hand the click to the page a moment later, outside this window message.
                    // Calling into the page while Windows is still delivering the click gets dropped.
                    let w = ctx.window.clone();
                    let js = format!("window.__mediaAction && window.__mediaAction('{action}')");
                    std::thread::spawn(move || {
                        let _ = w.eval(&js);
                    });
                }
                return LRESULT(0);
            }
        }

        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    pub fn attach(window: &WebviewWindow<Wry>) -> Result<(), String> {
        let hwnd = HWND(window.hwnd().map_err(|e| e.to_string())?.0 as _);
        if CREATED_MSG.load(Ordering::Relaxed) == 0 {
            let name = wide("TaskbarButtonCreated");
            let id = unsafe { RegisterWindowMessageW(PCWSTR(name.as_ptr())) };
            CREATED_MSG.store(id, Ordering::Relaxed);
        }
        let icons = Icons {
            shuffle_off: load_icon(window, "SHUFFLE_OFF")?,
            shuffle_on: load_icon(window, "SHUFFLE_ON")?,
            prev: load_icon(window, "PREV_THUMB")?,
            play: load_icon(window, "PLAY_THUMB")?,
            pause: load_icon(window, "PAUSE_THUMB")?,
            next: load_icon(window, "NEXT_THUMB")?,
            like_off: load_icon(window, "LIKE_OFF")?,
            like_on: load_icon(window, "LIKE_ON")?,
        };
        let ctx = Box::new(Ctx {
            window: window.clone(),
            taskbar: None,
            icons,
            state: 0,
        });
        let ptr = Box::into_raw(ctx);
        unsafe {
            let _ = SetWindowSubclass(hwnd, Some(subclass_proc), SUBCLASS_ID, ptr as usize);
            let id = CREATED_MSG.load(Ordering::Relaxed);
            if id != 0 {
                let _ = PostMessageW(Some(hwnd), id, WPARAM(0), LPARAM(0));
            }
        }
        Ok(())
    }

    pub fn set_state(window: &WebviewWindow<Wry>, bits: usize) {
        if let Ok(h) = window.hwnd() {
            let hwnd = HWND(h.0 as _);
            unsafe {
                let _ = PostMessageW(Some(hwnd), WM_UPDATE_STATE, WPARAM(bits), LPARAM(0));
            }
        }
    }
}

// ---------- Now playing widget for OBS ----------
// A tiny web page on this PC (127.0.0.1 only). OBS loads it as a Browser Source; the page
// asks /now.json every second for what's playing. It never plays audio.

static WIDGET_STATE: Mutex<Option<Value>> = Mutex::new(None);
static WIDGET_GEN: AtomicU64 = AtomicU64::new(0);
const WIDGET_HTML: &str = include_str!("../widget/widget.html");

// Changes whenever widget.html changes; an open widget compares it and reloads itself
fn widget_version() -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in WIDGET_HTML.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:x}")
}

fn widget_serve(listener: TcpListener, gen: u64) {
    let _ = listener.set_nonblocking(true);
    loop {
        if WIDGET_GEN.load(Ordering::SeqCst) != gen {
            break; // switched off or restarted
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = [0u8; 2048];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let path = req
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or("/")
                    .split('?')
                    .next()
                    .unwrap_or("/")
                    .to_string();
                let (status, ctype, body) = match path.as_str() {
                    "/now.json" => {
                        let mut v = WIDGET_STATE
                            .lock()
                            .ok()
                            .and_then(|g| g.clone())
                            .unwrap_or_else(|| serde_json::json!({ "playing": false }));
                        if let Some(o) = v.as_object_mut() {
                            o.insert("v".to_string(), Value::String(widget_version()));
                        }
                        ("200 OK", "application/json", v.to_string())
                    }
                    "/" | "/index.html" => (
                        "200 OK",
                        "text/html; charset=utf-8",
                        WIDGET_HTML.replace("__WIDGET_VERSION__", &widget_version()),
                    ),
                    _ => ("404 Not Found", "text/plain", "Not found".to_string()),
                };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(60)),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

#[tauri::command]
fn widget_start(port: u16) -> Result<String, String> {
    // Stops any earlier server first
    let gen = WIDGET_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let mut last_err = String::new();
    for _ in 0..15 {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                std::thread::spawn(move || widget_serve(listener, gen));
                return Ok(format!("http://127.0.0.1:{port}/"));
            }
            Err(e) => {
                last_err = e.to_string();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(format!("Port {port} is in use or blocked ({last_err}). Try another port."))
}

#[tauri::command]
fn widget_stop() {
    WIDGET_GEN.fetch_add(1, Ordering::SeqCst);
}

#[tauri::command]
fn widget_update(state: Value) {
    if let Ok(mut g) = WIDGET_STATE.lock() {
        *g = Some(state);
    }
}

// ---------- Local music: your own files in folders you choose ----------
// Tags are read once and kept in local_index.json; later scans only re-read files that changed.
// Files are played through a small server on this PC only (127.0.0.1), and only indexed files are served.

#[derive(Serialize, Deserialize, Clone)]
struct LocalEntry {
    id: String,
    path: String,
    size: u64,
    mtime: u64,
    title: String,
    artist: String,
    album: String,
    duration: u64,
}

static LOCAL: Mutex<Vec<LocalEntry>> = Mutex::new(Vec::new());
static LOCAL_SCANNING: AtomicBool = AtomicBool::new(false);
static LOCAL_LOADED: AtomicBool = AtomicBool::new(false);
static LOCAL_SERVER: OnceLock<(u16, String)> = OnceLock::new();

const AUDIO_EXTS: [&str; 10] = ["mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "aif", "aiff"];

fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("ogg") | Some("oga") | Some("opus") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("aif") | Some("aiff") => "audio/aiff",
        _ => "application/octet-stream",
    }
}

fn path_id(p: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in p.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

fn local_folders_load() -> Vec<String> {
    data_dir()
        .ok()
        .and_then(|d| fs::read_to_string(d.join("local.json")).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["folders"].as_array().cloned())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

fn local_index_file() -> Option<PathBuf> {
    data_dir().ok().map(|d| d.join("local_index.json"))
}

fn local_load_index() {
    if LOCAL_LOADED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(p) = local_index_file() {
        if let Ok(text) = fs::read_to_string(p) {
            if let Ok(list) = serde_json::from_str::<Vec<LocalEntry>>(&text) {
                if let Ok(mut g) = LOCAL.lock() {
                    *g = list;
                }
            }
        }
    }
}

fn read_tags(path: &Path) -> (String, String, String, u64) {
    use lofty::prelude::*;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();
    if let Ok(tagged) = lofty::read_from_path(path) {
        let dur = tagged.properties().duration().as_millis() as u64;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            let title = tag
                .title()
                .map(|c| c.to_string())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| stem.clone());
            let artist = tag.artist().map(|c| c.to_string()).unwrap_or_default();
            let album = tag.album().map(|c| c.to_string()).unwrap_or_default();
            return (title, artist, album, dur);
        }
        return (stem, String::new(), String::new(), dur);
    }
    (stem, String::new(), String::new(), 0)
}

fn local_scan_blocking() {
    local_load_index();
    let old: HashMap<String, LocalEntry> = LOCAL
        .lock()
        .map(|g| g.iter().map(|e| (e.path.clone(), e.clone())).collect())
        .unwrap_or_default();
    let mut out: Vec<LocalEntry> = Vec::new();
    for root in local_folders_load() {
        let mut stack = vec![PathBuf::from(&root)];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if !is_audio(&p) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                let size = meta.len();
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let path = p.to_string_lossy().to_string();
                if let Some(o) = old.get(&path) {
                    if o.size == size && o.mtime == mtime {
                        out.push(o.clone());
                        continue;
                    }
                }
                let (title, artist, album, duration) = read_tags(&p);
                out.push(LocalEntry { id: path_id(&path), path, size, mtime, title, artist, album, duration });
            }
        }
    }
    if let Ok(mut g) = LOCAL.lock() {
        *g = out.clone();
    }
    if let Some(f) = local_index_file() {
        if let Ok(text) = serde_json::to_string(&out) {
            let _ = fs::write(f, text);
        }
    }
    LOCAL_SCANNING.store(false, Ordering::SeqCst);
}

fn local_path_for(id: &str) -> Option<PathBuf> {
    LOCAL
        .lock()
        .ok()
        .and_then(|g| g.iter().find(|e| e.id == id).map(|e| PathBuf::from(&e.path)))
}

fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split('?').nth(1)?;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return urlencoding::decode(v).ok().map(|c| c.into_owned());
            }
        }
    }
    None
}

fn add_header<R: Read>(resp: &mut tiny_http::Response<R>, k: &str, v: &str) {
    if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
        resp.add_header(h);
    }
}

fn serve_file(request: tiny_http::Request, path: &Path) {
    use std::io::{Seek, SeekFrom};
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            let _ = request.respond(tiny_http::Response::empty(404));
            return;
        }
    };
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    if total == 0 {
        let _ = request.respond(tiny_http::Response::empty(404));
        return;
    }
    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());
    let last = total - 1;
    let (start, end, partial) = match range.as_deref().and_then(|r| r.strip_prefix("bytes=")) {
        Some(spec) => {
            let mut it = spec.splitn(2, '-');
            let s = it.next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
            let e = it.next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(last);
            (s, e.min(last), true)
        }
        None => (0, last, false),
    };
    if start > end {
        let _ = request.respond(tiny_http::Response::empty(416));
        return;
    }
    let len = end - start + 1;
    if file.seek(SeekFrom::Start(start)).is_err() {
        let _ = request.respond(tiny_http::Response::empty(500));
        return;
    }
    let status: u16 = if partial { 206 } else { 200 };
    let mut resp = tiny_http::Response::new(
        tiny_http::StatusCode(status),
        Vec::new(),
        file.take(len),
        Some(len as usize),
        None,
    );
    add_header(&mut resp, "Content-Type", mime_for(path));
    add_header(&mut resp, "Accept-Ranges", "bytes");
    add_header(&mut resp, "Access-Control-Allow-Origin", "*");
    if partial {
        add_header(&mut resp, "Content-Range", &format!("bytes {start}-{end}/{total}"));
    }
    let _ = request.respond(resp);
}

// Cover art: the picture inside the file, or a cover/folder image beside it
fn serve_cover(request: tiny_http::Request, path: &Path) {
    use lofty::prelude::*;
    let mut data: Option<(Vec<u8>, String)> = None;
    if let Ok(tagged) = lofty::read_from_path(path) {
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(pic) = tag.pictures().first() {
                let mime = pic
                    .mime_type()
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| "image/jpeg".to_string());
                data = Some((pic.data().to_vec(), mime));
            }
        }
    }
    if data.is_none() {
        if let Some(dir) = path.parent() {
            for name in ["cover.jpg", "folder.jpg", "front.jpg", "cover.png", "folder.png"] {
                if let Ok(bytes) = fs::read(dir.join(name)) {
                    let mime = if name.ends_with("png") { "image/png" } else { "image/jpeg" };
                    data = Some((bytes, mime.to_string()));
                    break;
                }
            }
        }
    }
    match data {
        Some((bytes, mime)) => {
            let mut resp = tiny_http::Response::from_data(bytes);
            add_header(&mut resp, "Content-Type", &mime);
            add_header(&mut resp, "Cache-Control", "max-age=86400");
            add_header(&mut resp, "Access-Control-Allow-Origin", "*");
            let _ = request.respond(resp);
        }
        None => {
            let _ = request.respond(tiny_http::Response::empty(404));
        }
    }
}

fn handle_local_request(request: tiny_http::Request, token: &str) {
    let url = request.url().to_string();
    let route = url.split('?').next().unwrap_or("").to_string();
    if query_param(&url, "k").as_deref() != Some(token) {
        let _ = request.respond(tiny_http::Response::empty(403));
        return;
    }
    let id = query_param(&url, "id").unwrap_or_default();
    let Some(path) = local_path_for(&id) else {
        let _ = request.respond(tiny_http::Response::empty(404));
        return;
    };
    match route.as_str() {
        "/file" => serve_file(request, &path),
        "/cover" => serve_cover(request, &path),
        _ => {
            let _ = request.respond(tiny_http::Response::empty(404));
        }
    }
}

fn start_local_server() -> (u16, String) {
    let mut tb = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut tb);
    let token = URL_SAFE_NO_PAD.encode(tb);
    let server = tiny_http::Server::http("127.0.0.1:0").expect("could not start the local music server");
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let tok = token.clone();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let t = tok.clone();
            std::thread::spawn(move || handle_local_request(request, &t));
        }
    });
    (port, token)
}

#[tauri::command]
fn local_folders() -> Vec<String> {
    local_folders_load()
}

#[tauri::command]
fn local_set_folders(folders: Vec<String>) -> Result<Vec<String>, String> {
    let mut clean: Vec<String> = Vec::new();
    for f in folders {
        let p = f.trim().trim_matches('"').to_string();
        if p.is_empty() || clean.contains(&p) {
            continue;
        }
        match fs::metadata(&p) {
            Ok(m) if m.is_dir() => clean.push(p),
            _ => return Err(format!("Cannot open that folder: {p}")),
        }
    }
    let file = data_dir()?.join("local.json");
    fs::write(file, serde_json::json!({ "folders": clean }).to_string()).map_err(|e| e.to_string())?;
    Ok(clean)
}

#[tauri::command]
fn local_scan() {
    if LOCAL_SCANNING.swap(true, Ordering::SeqCst) {
        return; // already scanning
    }
    std::thread::spawn(local_scan_blocking);
}

#[tauri::command]
fn local_status() -> Value {
    let count = LOCAL.lock().map(|g| g.len()).unwrap_or(0);
    serde_json::json!({ "scanning": LOCAL_SCANNING.load(Ordering::SeqCst), "count": count })
}

#[tauri::command]
fn local_tracks() -> Value {
    local_load_index();
    let folders = local_folders_load();
    let list: Vec<Value> = LOCAL
        .lock()
        .map(|g| {
            g.iter()
                .filter(|e| folders.iter().any(|f| e.path.starts_with(f.as_str())))
                .map(|e| {
                    serde_json::json!({
                        "id": e.id, "title": e.title, "artist": e.artist,
                        "album": e.album, "duration": e.duration
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Value::Array(list)
}

#[tauri::command]
fn local_server() -> Value {
    let info = LOCAL_SERVER.get_or_init(start_local_server);
    serde_json::json!({ "port": info.0, "token": info.1 })
}

// ---------- Song requests for chat bots (Streamer.bot, Mix It Up, ...) ----------
// GET /request?key=...&user=...&q=...  adds a SoundCloud track to the end of the queue.
// GET /now                             says what's playing.
// Replies are plain text, ready to post in chat (add &format=json for JSON).
// Only listens on 127.0.0.1, and requests need the secret key from Settings.

pub(crate) static APP: OnceLock<AppHandle> = OnceLock::new();
static REQ_GEN: AtomicU64 = AtomicU64::new(0);
static REQ_NEXT: AtomicU64 = AtomicU64::new(1);
static REQ_KEY: Mutex<String> = Mutex::new(String::new());
static REQ_WAIT: OnceLock<Mutex<HashMap<u64, std::sync::mpsc::Sender<Value>>>> = OnceLock::new();

fn req_wait() -> &'static Mutex<HashMap<u64, std::sync::mpsc::Sender<Value>>> {
    REQ_WAIT.get_or_init(|| Mutex::new(HashMap::new()))
}

// Like query_param, but also treats "+" as a space (how some bots encode text)
fn req_param(url: &str, key: &str) -> Option<String> {
    let q = url.split('?').nth(1)?;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return urlencoding::decode(&v.replace('+', "%20")).ok().map(|c| c.into_owned());
            }
        }
    }
    None
}

// Hands the request to the page (which searches and queues), then waits for its answer
fn ask_page(q: String, user: String) -> Value {
    let Some(app) = APP.get() else {
        return serde_json::json!({ "ok": false, "message": "The music app isn't ready yet." });
    };
    let Some(w) = app.get_webview_window("main") else {
        return serde_json::json!({ "ok": false, "message": "The music app isn't ready yet." });
    };
    let id = REQ_NEXT.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    if let Ok(mut m) = req_wait().lock() {
        m.insert(id, tx);
    }
    let js = format!(
        "window.__songRequest && window.__songRequest({id}, {}, {})",
        serde_json::to_string(&q).unwrap_or_else(|_| "\"\"".to_string()),
        serde_json::to_string(&user).unwrap_or_else(|_| "\"\"".to_string())
    );
    let _ = w.eval(&js);
    let answer = rx.recv_timeout(Duration::from_secs(12));
    if let Ok(mut m) = req_wait().lock() {
        m.remove(&id);
    }
    answer.unwrap_or_else(|_| serde_json::json!({ "ok": false, "message": "The music app didn't answer in time." }))
}

fn handle_req(mut stream: TcpStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 4096];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let url = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let route = url.split('?').next().unwrap_or("/").to_string();
    let as_json = req_param(&url, "format").as_deref() == Some("json");

    let (status, answer) = match route.as_str() {
        "/now" => {
            let s = WIDGET_STATE.lock().ok().and_then(|g| g.clone()).unwrap_or(Value::Null);
            let title = s["title"].as_str().unwrap_or("").to_string();
            if title.is_empty() {
                ("200 OK", serde_json::json!({ "ok": true, "playing": false, "message": "Nothing is playing right now." }))
            } else {
                let artist = s["artist"].as_str().unwrap_or("").to_string();
                let paused = !s["playing"].as_bool().unwrap_or(false);
                let mut msg = format!("Now playing: {title}");
                if !artist.is_empty() {
                    msg.push_str(&format!(" by {artist}"));
                }
                if paused {
                    msg.push_str(" (paused)");
                }
                ("200 OK", serde_json::json!({
                    "ok": true, "playing": !paused, "title": title, "artist": artist,
                    "url": s["url"].as_str().unwrap_or(""), "message": msg
                }))
            }
        }
        "/request" => {
            let expected = REQ_KEY.lock().map(|k| k.clone()).unwrap_or_default();
            let key = req_param(&url, "key").unwrap_or_default();
            if expected.is_empty() || key != expected {
                ("403 Forbidden", serde_json::json!({ "ok": false, "message": "Wrong or missing key." }))
            } else {
                let q = req_param(&url, "q").unwrap_or_default();
                let user = req_param(&url, "user").unwrap_or_default();
                ("200 OK", ask_page(q, user))
            }
        }
        _ => ("404 Not Found", serde_json::json!({ "ok": false, "message": "Not found." })),
    };
    let (ctype, body) = if as_json {
        ("application/json", answer.to_string())
    } else {
        ("text/plain; charset=utf-8", answer["message"].as_str().unwrap_or("").to_string())
    };
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn req_serve(listener: TcpListener, gen: u64) {
    let _ = listener.set_nonblocking(true);
    loop {
        if REQ_GEN.load(Ordering::SeqCst) != gen {
            break;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                std::thread::spawn(move || handle_req(stream));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(60)),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

#[tauri::command]
fn requests_start(port: u16, key: String) -> Result<String, String> {
    if let Ok(mut k) = REQ_KEY.lock() {
        *k = key;
    }
    let gen = REQ_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let mut last_err = String::new();
    for _ in 0..15 {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                std::thread::spawn(move || req_serve(listener, gen));
                return Ok(format!("http://127.0.0.1:{port}/"));
            }
            Err(e) => {
                last_err = e.to_string();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(format!("Port {port} is in use or blocked ({last_err}). Try another port."))
}

#[tauri::command]
fn requests_stop() {
    REQ_GEN.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut k) = REQ_KEY.lock() {
        k.clear();
    }
}

// The page's answer to a request
#[tauri::command]
fn request_result(id: u64, result: Value) {
    if let Ok(m) = req_wait().lock() {
        if let Some(tx) = m.get(&id) {
            let _ = tx.send(result);
        }
    }
}

// ---------- app settings (read at startup) ----------

#[derive(Serialize, Deserialize, Clone)]
struct Settings {
    #[serde(default = "default_true")]
    gpu: bool,
}

fn default_true() -> bool {
    true
}

fn load_settings() -> Settings {
    data_dir()
        .ok()
        .and_then(|d| fs::read_to_string(d.join("settings.json")).ok())
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(Settings { gpu: true })
}

#[tauri::command]
fn get_settings() -> Settings {
    load_settings()
}

#[tauri::command]
fn set_gpu(enabled: bool) -> Result<(), String> {
    let text = serde_json::to_string(&Settings { gpu: enabled }).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("settings.json"), text).map_err(|e| e.to_string())
}

#[tauri::command]
fn restart_app(app: AppHandle) {
    app.restart();
}

// Passing our own browser arguments replaces Tauri's defaults, so the default
// feature list is repeated here.
#[allow(dead_code)]
fn browser_args(gpu: bool) -> String {
    let mut args = String::from(
        "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection --autoplay-policy=no-user-gesture-required --use-fake-ui-for-media-stream",
    );
    if !gpu {
        args.push_str(" --disable-gpu");
    }
    args
}

// ---------- app ----------

fn main() {
    let play = Shortcut::new(None, Code::MediaPlayPause);
    let next = Shortcut::new(None, Code::MediaTrackNext);
    let prev = Shortcut::new(None, Code::MediaTrackPrevious);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_shortcuts([play, next, prev])
                .expect("could not register media keys")
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let action = if shortcut == &play {
                        "play"
                    } else if shortcut == &next {
                        "next"
                    } else if shortcut == &prev {
                        "prev"
                    } else {
                        return;
                    };
                    media_action(app, action);
                })
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let gpu = load_settings().gpu;
            #[allow(unused_mut)]
            let mut builder =
                WebviewWindowBuilder::new(app, "main", WebviewUrl::App("index.html".into()))
                    .title("OmniPulse")
                    .inner_size(1180.0, 760.0)
                    .min_inner_size(820.0, 520.0)
                    .decorations(false)
                    .disable_drag_drop_handler();
            #[cfg(windows)]
            {
                builder = builder.additional_browser_args(&browser_args(gpu));
            }
            #[cfg(not(windows))]
            {
                let _ = gpu;
            }
            let main_window = builder.build()?;
            let _ = APP.set(app.handle().clone());
            // Shuffle / previous / play-pause / next / like buttons on the taskbar thumbnail
            #[cfg(windows)]
            {
                let _ = taskbar_buttons::attach(&main_window);
            }
            #[cfg(not(windows))]
            {
                let _ = &main_window;
            }

            // Tray icon: show the window, control playback, or quit
            let show = MenuItem::with_id(app, "show", "Show", true, None::<&str>)?;
            let play = MenuItem::with_id(app, "play", "Play / Pause", true, None::<&str>)?;
            let next = MenuItem::with_id(app, "next", "Next", true, None::<&str>)?;
            let prev = MenuItem::with_id(app, "prev", "Previous", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &play, &next, &prev, &quit])?;
            let mut tray = TrayIconBuilder::new().menu(&menu).tooltip("Music");
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.on_menu_event(|app, event| match event.id.as_ref() {
                "show" => {
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
                "play" => {
                    media_action(app, "play");
                }
                "next" => {
                    media_action(app, "next");
                }
                "prev" => {
                    media_action(app, "prev");
                }
                "quit" => {
                    app.exit(0);
                }
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let app = tray.app_handle();
                    if let Some(w) = app.get_webview_window("main") {
                        let _ = w.unminimize();
                        let _ = w.show();
                        let _ = w.set_focus();
                    }
                }
            })
            .build(app)?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            login, signed_in, logout, api_get, stream_url, get_settings, set_gpu, restart_app, open_url, set_like, set_follow, playlist_write,
            spotify_login, spotify_signed_in, spotify_logout, spotify_token, spotify_api,
            lyrics_get, presence_set, presence_clear,
            lastfm_status, lastfm_connect, lastfm_disconnect, lastfm_call,
            stream_resolve, presence_test, taskbar_state,
            widget_start, widget_stop, widget_update,
            local_folders, local_set_folders, local_scan, local_status, local_tracks, local_server,
            requests_start, requests_stop, request_result,
            spe_signed_in, spe_login, spe_logout, spe_attach, spe_play, spe_pause, spe_resume, spe_seek, spe_stop, spe_played, spe_playlist, spe_web_get,
            resolve_yt_stream, remote_start, remote_stop, remote_result
        ])
        .run(tauri::generate_context!())
        .expect("error while running app");
}
