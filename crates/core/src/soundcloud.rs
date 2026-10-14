// SoundCloud: sign-in (OAuth 2.1 with PKCE), API calls, streams, likes, follows and playlists.

use crate::{bind_addr, data_dir, load_config, load_tokens, save_tokens, Config, Tokens};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    time::{Duration, Instant},
};

// ---------- OAuth 2.1 with PKCE ----------

pub async fn token_request(params: &[(&str, &str)]) -> Result<Tokens, String> {
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

pub async fn refresh(cfg: &Config, old: &Tokens) -> Result<Tokens, String> {
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

/// A fresh key after a 401, one refresh at a time: SoundCloud's refresh token works once, so many
/// requests turned down together (a library loading, with its key just expired) all refreshing with it
/// left all but the first failing, and those parts (playlists, search results) empty. Waiting its
/// turn, a request first takes a key someone else got meanwhile (this app or the other, sharing the
/// data folder), and refreshes only if there isn't one
pub async fn refreshed_after_401(cfg: &Config, stale: &Tokens) -> Result<Tokens, String> {
    static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _turn = ONE_AT_A_TIME.lock().await;
    let disk = load_tokens().ok_or("Not signed in")?;
    if disk.access_token != stale.access_token { return Ok(disk); }
    match refresh(cfg, &disk).await {
        Ok(t) => Ok(t),
        // (refused: another app may have just used it; its new key, if it saved one)
        Err(e) => match load_tokens() { Some(t) if t.access_token != stale.access_token => Ok(t), _ => Err(e) },
    }
}

pub async fn login() -> Result<(), String> {
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
    crate::open_browser(&url)?;

    let (code, got_state) = tokio::task::spawn_blocking(
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

pub fn signed_in() -> bool {
    load_tokens().is_some()
}

pub fn logout() -> Result<(), String> {
    let path = data_dir()?.join("tokens.json");
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ---------- API calls ----------

pub async fn authed_request(method: reqwest::Method, path: &str) -> Result<reqwest::Response, String> {
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
        tokens = refreshed_after_401(&cfg, &tokens).await?;
        res = send(tokens.access_token.clone())
            .await
            .map_err(|e| e.to_string())?;
    }
    Ok(res)
}

pub async fn authed_get(path: &str) -> Result<reqwest::Response, String> {
    authed_request(reqwest::Method::GET, path).await
}

pub async fn api_get(path: String) -> Result<Value, String> {
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
pub fn collect_streams(v: &Value) -> Vec<(String, String)> {
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
pub async fn resolve_api_url(url: &str) -> Option<String> {
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

pub async fn stream_url(id: String) -> Result<Value, String> {
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

pub async fn stream_resolve(url: String) -> Result<Value, String> {
    let final_url = resolve_api_url(&url).await.unwrap_or_else(|| url.clone());
    let auth = if final_url.starts_with("https://api.soundcloud.com") {
        load_tokens().map(|t| format!("OAuth {}", t.access_token))
    } else {
        None
    };
    Ok(serde_json::json!({ "url": final_url, "auth": auth }))
}

// Like (POST) or unlike (DELETE) a track.
pub async fn set_like(id: String, like: bool) -> Result<(), String> {
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
pub async fn set_follow(id: String, follow: bool) -> Result<(), String> {
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
/// A playlist write's tracks, { "id": n } each, as { "urn": "soundcloud:tracks:n" }
fn as_urns(mut body: Value) -> Value {
    if let Some(list) = body["playlist"]["tracks"].as_array_mut() {
        for t in list.iter_mut() {
            let id = t["id"].as_u64().map(|n| n.to_string()).or_else(|| t["id"].as_str().filter(|x| x.chars().all(|c| c.is_ascii_digit()) && !x.is_empty()).map(String::from));
            if let Some(id) = id { *t = serde_json::json!({ "urn": format!("soundcloud:tracks:{id}") }); }
        }
    }
    body
}

pub async fn playlist_write(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
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
    // (a playlist's tracks as URNs: SoundCloud turns down an id past 2,147,483,647 given as a number
    // ("Could not parse JSON request body"), and its newer tracks' ids are)
    let body = body.map(as_urns);
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
        tokens = refreshed_after_401(&cfg, &tokens).await?;
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

#[cfg(test)]
mod hls_probe {
    // which container SoundCloud's HLS segments use (read-only, one liked track):
    // cargo test -p omnipulse-core sc_hls_probe -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn sc_hls_probe() {
        let likes = super::api_get("/me/likes/tracks?limit=3&access=playable".into()).await.expect("likes");
        let list = likes.as_array().cloned().or_else(|| likes["collection"].as_array().cloned()).unwrap_or_default();
        let id = list.iter().find_map(|t| t["id"].as_u64().or(t["track"]["id"].as_u64())).expect("a liked track");
        let v = super::stream_url(id.to_string()).await.expect("streams");
        println!("stream kinds: {:?}", v["candidates"].as_array().map(|a| a.iter().map(|c| c["name"].as_str().unwrap_or("")).collect::<Vec<_>>()));
        for c in v["candidates"].as_array().cloned().unwrap_or_default() {
            let name = c["name"].as_str().unwrap_or("").to_string();
            if !name.starts_with("hls") { continue; }
            let url = c["url"].as_str().unwrap_or("").to_string();
            let real = if url.starts_with("https://api.soundcloud.com") { super::resolve_api_url(&url).await.unwrap_or(url) } else { url };
            let m3u8 = reqwest::get(&real).await.unwrap().text().await.unwrap();
            let lines: Vec<&str> = m3u8.lines().collect();
            let tags: Vec<&&str> = lines.iter().filter(|l| l.starts_with("#EXT-X-MAP") || l.starts_with("#EXT-X-KEY") || l.starts_with("#EXT-X-VERSION")).collect();
            let seg = lines.iter().find(|l| !l.starts_with('#') && !l.is_empty()).map(|s| s.to_string()).unwrap_or_default();
            let seg_url = if seg.starts_with("http") { seg } else { real.rsplit_once('/').map(|(b, _)| format!("{b}/{seg}")).unwrap_or(seg) };
            let bytes = reqwest::get(&seg_url).await.unwrap().bytes().await.unwrap();
            let head: Vec<u8> = bytes.iter().take(12).copied().collect();
            let kind = if head.first() == Some(&0x47) { "MPEG-TS" } else if head.len() > 8 && (&head[4..8] == b"moof" || &head[4..8] == b"ftyp" || &head[4..8] == b"styp") { "fragmented MP4" } else { "something else" };
            println!("{name} playlist: {real}");
            println!("{name}: {} segments, tags {:?}, first segment {} bytes, starts {:02x?} = {kind}", lines.iter().filter(|l| l.starts_with("#EXTINF")).count(), tags, bytes.len(), head);
        }
    }
}


#[cfg(test)]
mod urn_tests {
    #[test]
    fn a_playlists_tracks_go_as_urns() {
        let b = super::as_urns(serde_json::json!({ "playlist": { "title": "x", "tracks": [{ "id": 2348177720u64 }, { "id": "12" }, { "urn": "soundcloud:tracks:5" }] } }));
        assert_eq!(b["playlist"]["tracks"], serde_json::json!([{ "urn": "soundcloud:tracks:2348177720" }, { "urn": "soundcloud:tracks:12" }, { "urn": "soundcloud:tracks:5" }]));
        assert_eq!(super::as_urns(serde_json::json!({ "playlist": { "title": "y" } }))["playlist"]["title"], "y");
    }
}
