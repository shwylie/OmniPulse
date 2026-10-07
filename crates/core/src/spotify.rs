// Spotify Web API: sign-in (PKCE), tokens and API calls. Playback is in spotify_engine.

use crate::{bind_addr, data_dir, load_config, now_secs, Config};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize, Clone)]
pub struct SpTokens {
    access_token: String,
    refresh_token: Option<String>,
    expires_at: u64,
}

pub fn load_sp_tokens() -> Option<SpTokens> {
    let text = fs::read_to_string(data_dir().ok()?.join("spotify_tokens.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_sp_tokens(t: &SpTokens) -> Result<(), String> {
    let text = serde_json::to_string(t).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("spotify_tokens.json"), text).map_err(|e| e.to_string())
}

pub async fn sp_token_request(params: &[(&str, &str)]) -> Result<(String, Option<String>, u64), String> {
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

pub async fn sp_refresh(cfg: &Config, old: &SpTokens) -> Result<SpTokens, String> {
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
pub async fn wait_for_callback(bind: String, url: String) -> Result<(String, String), String> {
    let listener = TcpListener::bind(&bind)
        .map_err(|e| format!("Could not listen for the sign-in callback (is the port busy?): {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    opener::open(&url).map_err(|e| e.to_string())?;

    let result = tokio::task::spawn_blocking(move || -> Result<(String, String), String> {
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

pub async fn spotify_login() -> Result<(), String> {
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

pub fn spotify_signed_in() -> bool {
    load_sp_tokens().is_some()
}

pub fn spotify_logout() -> Result<(), String> {
    let path = data_dir()?.join("spotify_tokens.json");
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// The Spotify player in the page asks for a token whenever it needs one
pub async fn spotify_token() -> Result<String, String> {
    let cfg = load_config()?;
    let mut t = load_sp_tokens().ok_or("Not connected to Spotify")?;
    if t.expires_at < now_secs() + 120 {
        t = sp_refresh(&cfg, &t).await?;
    }
    Ok(t.access_token)
}

pub async fn spotify_api(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
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
