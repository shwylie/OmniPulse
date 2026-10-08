// Your YouTube account (optional): sign in with Google, and liking a track here likes it on YouTube
// and YouTube Music too (unliking takes the like back). Nothing else needs an account: YouTube Music
// search and playback work without one, as before.
//
// Sign-in: Google's OAuth for desktop apps, with PKCE, back to a loopback address on a free port
// (the same small server as Spotify's sign-in). The tokens are kept encrypted for your Windows
// account (DPAPI), in <data>/youtube_account.bin: useless if copied to another PC or user.
//
// Needs "youtube_client_id" and "youtube_client_secret" in config.json: a Google Cloud OAuth client
// of type "Desktop app", with the YouTube Data API v3 switched on (see the README).

use crate::{data_dir, load_config};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::net::TcpListener;
use std::time::{SystemTime, UNIX_EPOCH};

/// The scope that allows rating videos (liking, unliking)
const SCOPE: &str = "https://www.googleapis.com/auth/youtube";
const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const API: &str = "https://www.googleapis.com/youtube/v3";

#[derive(Serialize, Deserialize, Clone, Default)]
struct YtTokens {
    access_token: String,
    refresh_token: String,
    expires_at: u64,
    /// the channel's name, to show in Settings
    #[serde(default)]
    name: String,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn client() -> Result<(String, String), String> {
    let cfg = load_config()?;
    if cfg.youtube_client_id.trim().is_empty() || cfg.youtube_client_secret.trim().is_empty() {
        return Err("Add \"youtube_client_id\" and \"youtube_client_secret\" to config.json first (a Google Cloud OAuth client for a desktop app; see the README).".into());
    }
    Ok((cfg.youtube_client_id, cfg.youtube_client_secret))
}

// ---------- keeping the tokens: encrypted for this Windows account ----------
fn token_file() -> Result<std::path::PathBuf, String> {
    Ok(data_dir()?.join("youtube_account.bin"))
}

#[cfg(windows)]
fn protect(plain: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows::Win32::Foundation::{LocalFree, HLOCAL};
    use windows::Win32::Security::Cryptography::{CryptProtectData, CryptUnprotectData, CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN};
    unsafe {
        let input = CRYPT_INTEGER_BLOB { cbData: plain.len() as u32, pbData: plain.as_ptr() as *mut u8 };
        let mut out = CRYPT_INTEGER_BLOB::default();
        let r = if encrypt {
            CryptProtectData(&input, windows::core::w!("OmniPulse YouTube account"), None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        r.map_err(|e| format!("Windows couldn't {} the sign-in: {e}", if encrypt { "protect" } else { "read" }))?;
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        let _ = LocalFree(Some(HLOCAL(out.pbData as _)));
        Ok(bytes)
    }
}

#[cfg(not(windows))]
fn protect(plain: &[u8], _encrypt: bool) -> Result<Vec<u8>, String> {
    Ok(plain.to_vec())
}

fn load() -> Option<YtTokens> {
    let bytes = std::fs::read(token_file().ok()?).ok()?;
    serde_json::from_slice(&protect(&bytes, false).ok()?).ok()
}

fn save(t: &YtTokens) -> Result<(), String> {
    let plain = serde_json::to_vec(t).map_err(|e| e.to_string())?;
    std::fs::write(token_file()?, protect(&plain, true)?).map_err(|e| e.to_string())
}

// ---------- signing in and out ----------
pub fn status() -> Value {
    match load() {
        Some(t) => json!({ "connected": true, "name": t.name }),
        None => json!({ "connected": false, "configured": client().is_ok() }),
    }
}

/// Opens Google's sign-in in your browser and waits (up to three minutes) for you to come back
pub async fn connect() -> Result<Value, String> {
    let (id, secret) = client()?;
    // a free port of our own: Google allows any loopback port for desktop apps
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|e| format!("Couldn't open a port for the sign-in: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let redirect = format!("http://127.0.0.1:{port}");
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    let verifier = URL_SAFE_NO_PAD.encode(buf);
    let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
    rand::thread_rng().fill_bytes(&mut buf);
    let state = URL_SAFE_NO_PAD.encode(buf);
    let url = format!(
        "https://accounts.google.com/o/oauth2/v2/auth?response_type=code&client_id={}&redirect_uri={}&scope={}&code_challenge={challenge}&code_challenge_method=S256&state={state}&access_type=offline&prompt=consent",
        urlencoding::encode(&id),
        urlencoding::encode(&redirect),
        urlencoding::encode(SCOPE),
    );
    // the small server closes itself once the browser has come back (or after three minutes)
    let (code, got_state) = crate::spotify::wait_on(listener, url).await?;
    if got_state != state {
        return Err("The sign-in didn't match. Please try again.".into());
    }
    let res: Value = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[("code", code.as_str()), ("client_id", id.as_str()), ("client_secret", secret.as_str()), ("redirect_uri", redirect.as_str()), ("grant_type", "authorization_code"), ("code_verifier", verifier.as_str())])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    let access = res["access_token"].as_str().ok_or_else(|| format!("Google didn't sign you in: {}", res["error_description"].as_str().or(res["error"].as_str()).unwrap_or("no token")))?;
    let mut t = YtTokens {
        access_token: access.to_string(),
        refresh_token: res["refresh_token"].as_str().unwrap_or("").to_string(),
        expires_at: now() + res["expires_in"].as_u64().unwrap_or(3600),
        name: String::new(),
    };
    if t.refresh_token.is_empty() {
        return Err("Google didn't give a lasting sign-in. Remove OmniPulse at myaccount.google.com/permissions and connect again.".into());
    }
    // the channel's name, for Settings
    if let Ok(v) = authed(&mut t, reqwest::Method::GET, &format!("{API}/channels?part=snippet&mine=true"), None).await {
        t.name = v["items"][0]["snippet"]["title"].as_str().unwrap_or("").to_string();
    }
    save(&t)?;
    Ok(status())
}

/// Signs out here, and tells Google to forget the sign-in
pub async fn disconnect() -> Result<(), String> {
    if let Some(t) = load() {
        let _ = reqwest::Client::new().post("https://oauth2.googleapis.com/revoke").form(&[("token", t.refresh_token.as_str())]).send().await;
    }
    match std::fs::remove_file(token_file()?) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

async fn refresh(t: &mut YtTokens) -> Result<(), String> {
    let (id, secret) = client()?;
    let res: Value = reqwest::Client::new()
        .post(TOKEN_URL)
        .form(&[("client_id", id.as_str()), ("client_secret", secret.as_str()), ("refresh_token", t.refresh_token.as_str()), ("grant_type", "refresh_token")])
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())?;
    match res["access_token"].as_str() {
        Some(a) => {
            t.access_token = a.to_string();
            t.expires_at = now() + res["expires_in"].as_u64().unwrap_or(3600);
            save(t)
        }
        // the sign-in was taken back (at Google): forget it here too
        None if res["error"] == "invalid_grant" => {
            let _ = std::fs::remove_file(token_file()?);
            Err("Your YouTube sign-in has expired. Connect your YouTube account again in Settings.".into())
        }
        None => Err(format!("Couldn't renew the YouTube sign-in: {}", res["error"].as_str().unwrap_or("no reason given"))),
    }
}

/// A YouTube Data API call with your sign-in; an expired token is renewed and the call made again
async fn authed(t: &mut YtTokens, method: reqwest::Method, url: &str, body_len_zero: Option<()>) -> Result<Value, String> {
    if t.expires_at <= now() + 30 {
        refresh(t).await?;
    }
    for attempt in 0..2 {
        let mut req = reqwest::Client::new().request(method.clone(), url).bearer_auth(&t.access_token);
        if body_len_zero.is_some() {
            req = req.header(reqwest::header::CONTENT_LENGTH, "0");
        }
        let res = req.send().await.map_err(|e| e.to_string())?;
        let code = res.status();
        if code == reqwest::StatusCode::UNAUTHORIZED && attempt == 0 {
            refresh(t).await?;
            continue;
        }
        let text = res.text().await.unwrap_or_default();
        if !code.is_success() {
            let v: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
            let why = v["error"]["errors"][0]["reason"].as_str().or(v["error"]["message"].as_str()).unwrap_or("no reason given").to_string();
            return Err(match why.as_str() {
                "quotaExceeded" => "YouTube's daily allowance for this app is used up; it resets at midnight Pacific time.".into(),
                _ => format!("YouTube said no ({code}): {why}"),
            });
        }
        return Ok(if text.trim().is_empty() { Value::Null } else { serde_json::from_str(&text).unwrap_or(Value::Null) });
    }
    Err("YouTube didn't accept the sign-in.".into())
}

// ---------- finding the song on YouTube ----------
/// The video for a song by name: YouTube Music's own match first (free, and the same song entries
/// YouTube Music uses), then the YouTube Data API search for "Artist - Track (Official Audio)",
/// music videos only (type=video, videoCategoryId=10), which costs from a daily allowance
pub async fn find_video(title: &str, artist: &str) -> Result<String, String> {
    let q = format!("{artist} {title}");
    if let Ok(list) = crate::youtube::search(&q).await {
        let norm = |s: &str| s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect::<String>();
        let (t, a) = (norm(title), norm(artist.split(',').next().unwrap_or(artist)));
        if let Some(hit) = list.iter().find(|x| !t.is_empty() && norm(&x.title).contains(&t) && (a.is_empty() || norm(&x.artists).contains(&a))) {
            return Ok(hit.id.clone());
        }
    }
    let mut tok = load().ok_or("Connect your YouTube account first.")?;
    let query = format!("{artist} - {title} (Official Audio)");
    let url = format!("{API}/search?part=snippet&type=video&videoCategoryId=10&maxResults=5&q={}", urlencoding::encode(&query));
    let v = authed(&mut tok, reqwest::Method::GET, &url, None).await?;
    v["items"][0]["id"]["videoId"].as_str().map(String::from).ok_or_else(|| format!("Couldn't find \"{title}\" on YouTube."))
}

// ---------- liking ----------
/// Likes (or with like = false, unlikes) a video on YouTube: the video's id if known, otherwise
/// found by the song's name. Returns the video id, so an unlike later can use the same one.
pub async fn rate(video_id: Option<String>, title: &str, artist: &str, like: bool) -> Result<String, String> {
    let mut tok = load().ok_or("Connect your YouTube account in Settings first.")?;
    let id = match video_id.filter(|v| !v.trim().is_empty()) {
        Some(v) => v,
        None => find_video(title, artist).await?,
    };
    let url = format!("{API}/videos/rate?id={}&rating={}", urlencoding::encode(&id), if like { "like" } else { "none" });
    authed(&mut tok, reqwest::Method::POST, &url, Some(())).await?;
    Ok(id)
}

// ---------- licences ----------
/// Each video's licence on YouTube, by id: "creativeCommon" (the uploader chose Creative Commons) or
/// "youtube" (the standard licence), 50 videos to a call, each call 1 unit of the daily allowance.
/// For Stream-Safe Music; ids YouTube doesn't know are left out.
pub async fn licences(ids: Vec<String>) -> Result<Value, String> {
    let mut tok = load().ok_or("Connect your YouTube account in Settings first.")?;
    let ids: Vec<String> = ids.into_iter().filter(|i| !i.is_empty() && i.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')).collect();
    let mut out = serde_json::Map::new();
    for chunk in ids.chunks(50) {
        let url = format!("{API}/videos?part=status&maxResults=50&id={}", urlencoding::encode(&chunk.join(",")));
        let v = authed(&mut tok, reqwest::Method::GET, &url, None).await?;
        for it in v["items"].as_array().into_iter().flatten() {
            if let (Some(id), Some(l)) = (it["id"].as_str(), it["status"]["license"].as_str()) {
                out.insert(id.to_string(), Value::from(l));
            }
        }
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sign_in_is_kept_encrypted() {
        let plain = br#"{"access_token":"a","refresh_token":"r","expires_at":1}"#;
        let sealed = protect(plain, true).unwrap();
        // not readable as it's kept, and back to itself when read
        assert!(!sealed.windows(5).any(|w| w == b"token"));
        assert_eq!(protect(&sealed, false).unwrap(), plain.to_vec());
    }
}
