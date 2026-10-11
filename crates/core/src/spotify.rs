// Spotify Web API: tokens and API calls. Playback is in spotify_engine.
//
// Which sign-in the calls use: the playback one (Spotify's own app, spotify_engine) by default, so
// signing in once is all it takes. It only takes Premium accounts, so a free account signs in
// through a developer app of your own instead (spotify_client_id in config.json, PKCE below).

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

// ---------- staying signed in, and within Spotify's rate limits ----------
// Spotify answers too many requests with 429 and a Retry-After; that's waited out (and other requests
// hold off meanwhile) rather than treated as an error, and never as being signed out. Refreshing the
// token happens one request at a time: Spotify may hand out a new refresh token each time, so racing
// refreshes could leave a stale one saved.
static REFRESHING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static GATE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(2); // requests in flight at once
static COOL_UNTIL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0); // unix secs
const WAIT_AT_MOST: u64 = 10; // seconds; longer than this and the request says so instead

#[cfg(test)]
static API_BASE: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

fn api_base() -> String {
    #[cfg(test)]
    if let Some(b) = API_BASE.lock().ok().and_then(|g| g.clone()) {
        return b;
    }
    "https://api.spotify.com/v1".to_string()
}

/// Prefix of the error a rate limit gives, so the app can tell it from a real failure
pub const RATE_LIMITED: &str = "Spotify is limiting requests";

// The whole wait Spotify asks for (it can be hours after a burst): knocking again sooner only makes it
// longer. Kept on disk too, so reopening the app doesn't knock either.
fn retry_after(res: &reqwest::Response) -> u64 {
    res.headers().get("retry-after").and_then(|v| v.to_str().ok()).and_then(|s| s.trim().parse().ok()).unwrap_or(5).clamp(1, 86400)
}

// Each sign-in has its own allowance (Spotify counts per app: the playback sign-in's is Spotify's own,
// shared with everyone signed in that way; your developer app's is yours), so each its own wait: one
// limited, the other carries on.
static COOL_LOADED: std::sync::Once = std::sync::Once::new();
static COOL_PLAYBACK: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn cool_slot(via: Via) -> &'static std::sync::atomic::AtomicU64 {
    match via { Via::App => &COOL_UNTIL, Via::Playback => &COOL_PLAYBACK }
}

fn cool_file(via: Via) -> Option<std::path::PathBuf> {
    data_dir().ok().map(|d| d.join(match via { Via::App => "spotify_wait.txt", Via::Playback => "spotify_wait_playback.txt" }))
}

fn cool_until(via: Via) -> u64 {
    COOL_LOADED.call_once(|| {
        for v in [Via::App, Via::Playback] {
            let saved = cool_file(v).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| s.trim().parse::<u64>().ok()).unwrap_or(0);
            cool_slot(v).fetch_max(saved, std::sync::atomic::Ordering::SeqCst);
        }
    });
    cool_slot(via).load(std::sync::atomic::Ordering::SeqCst)
}

fn set_cool_until(via: Via, until: u64) {
    let _ = cool_until(via);
    cool_slot(via).fetch_max(until, std::sync::atomic::Ordering::SeqCst);
    if let Some(p) = cool_file(via) {
        let _ = std::fs::write(p, cool_slot(via).load(std::sync::atomic::Ordering::SeqCst).to_string());
    }
}

/// Seconds left before Spotify takes requests again (0: now) through any sign-in you have
pub fn spotify_wait() -> u64 {
    let now = now_secs();
    let vias: Vec<Via> = [Via::App, Via::Playback].into_iter().filter(|v| via_usable(*v)).collect();
    if vias.is_empty() { return cool_until(Via::App).saturating_sub(now); }
    vias.into_iter().map(|v| cool_until(v).saturating_sub(now)).min().unwrap_or(0)
}

// Requests at least this far apart, so a burst (following many artists, say) doesn't trip the limit
// (Spotify counts over a rolling 30 seconds; a developer app's allowance is modest: about 3 a second
// at most keeps well inside it)
const SPACING: std::time::Duration = std::time::Duration::from_millis(300);

// ---------- asked once, not again and again ----------
// What Spotify gave for a GET is kept a few minutes: the same list asked for from several pages (your
// likes, a playlist, a search) comes from here, not Spotify again. The same GET asked for twice at once
// is asked once (the second waits for the first). Any change (a like, a playlist edited) clears it all,
// so what's shown after a change is never stale.
const KEEP_FOR: std::time::Duration = std::time::Duration::from_secs(300);
type Kept = std::collections::HashMap<String, (std::time::Instant, Value)>;
static KEPT: std::sync::Mutex<Option<Kept>> = std::sync::Mutex::new(None);
static ASKING: std::sync::Mutex<Option<std::collections::HashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>>> = std::sync::Mutex::new(None);

fn kept(path: &str) -> Option<Value> {
    let g = KEPT.lock().ok()?;
    let (at, v) = g.as_ref()?.get(path)?;
    (at.elapsed() < KEEP_FOR).then(|| v.clone())
}

fn keep(path: &str, v: &Value) {
    if let Ok(mut g) = KEPT.lock() {
        let m = g.get_or_insert_with(Default::default);
        m.retain(|_, (at, _)| at.elapsed() < KEEP_FOR);
        m.insert(path.to_string(), (std::time::Instant::now(), v.clone()));
    }
}

fn forget_kept() {
    if let Ok(mut g) = KEPT.lock() { *g = None; }
}
static NEXT_SEND: tokio::sync::Mutex<Option<tokio::time::Instant>> = tokio::sync::Mutex::const_new(None);

async fn pace() {
    let mut next = NEXT_SEND.lock().await;
    let now = tokio::time::Instant::now();
    if let Some(at) = *next {
        if at > now {
            tokio::time::sleep_until(at).await;
        }
    }
    *next = Some(tokio::time::Instant::now().max(now) + SPACING);
}

// Waits out a sign-in's rate limit if it's short; errors if it isn't
async fn cool_down(via: Via) -> Result<(), String> {
    let until = cool_until(via);
    let now = now_secs();
    if until > now {
        let wait = until - now;
        if wait > WAIT_AT_MOST {
            return Err(format!("{RATE_LIMITED} right now: try again in {wait} s."));
        }
        tokio::time::sleep(std::time::Duration::from_secs(wait)).await;
    }
    Ok(())
}

// Where a request's token comes from
#[derive(Clone, Copy, PartialEq)]
enum Via {
    Playback,
    App,
}

#[cfg(feature = "desktop")]
fn playback_signed_in() -> bool {
    crate::spotify_engine::spe_signed_in()
}
#[cfg(not(feature = "desktop"))]
fn playback_signed_in() -> bool {
    false
}

#[cfg(feature = "desktop")]
async fn playback_token() -> Result<String, String> {
    crate::spotify_engine::spe_web_token().await
}
#[cfg(not(feature = "desktop"))]
async fn playback_token() -> Result<String, String> {
    Err("Not connected to Spotify".into())
}

/// Signed in with your own developer app (its tokens kept), and not turned down this run
pub fn spotify_app_signed_in() -> bool {
    via_usable(Via::App)
}

/// Your developer app's keys turned down by Spotify this run (its owner hasn't Premium: Spotify's rule
/// for developer apps since February 2026): left alone for the rest of it, the playback sign-in used
static APP_TURNED_DOWN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn via_usable(via: Via) -> bool {
    match via {
        Via::App => load_sp_tokens().is_some() && !APP_TURNED_DOWN.load(std::sync::atomic::Ordering::Relaxed),
        Via::Playback => playback_signed_in(),
    }
}

// Your own developer app's first (its own allowance), signed in with it; else the playback sign-in's
fn first_via() -> Option<Via> {
    if via_usable(Via::App) {
        Some(Via::App)
    } else if playback_signed_in() {
        Some(Via::Playback)
    } else if load_sp_tokens().is_some() {
        Some(Via::App)
    } else {
        None
    }
}

/// The other sign-in, if you have it and it can take requests now
fn other_via(via: Via) -> Option<Via> {
    let o = if via == Via::App { Via::Playback } else { Via::App };
    (via_usable(o) && cool_until(o) <= now_secs()).then_some(o)
}

/// A token that's good for at least a minute (refreshed if not, by one request at a time); with
/// `rejected`, the token Spotify just refused is replaced, unless someone else replaced it meanwhile
async fn usable_token(cfg: &Config, rejected: Option<&str>) -> Result<SpTokens, String> {
    let _one_at_a_time = REFRESHING.lock().await;
    let t = load_sp_tokens().ok_or("Not connected to Spotify")?;
    let stale = match rejected {
        Some(old) => t.access_token == old,
        None => t.expires_at < now_secs() + 60,
    };
    if stale { sp_refresh(cfg, &t).await } else { Ok(t) }
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
    wait_on(listener, url).await
}

/// The same, on a listener that's already open (Google's sign-in picks a free port first)
pub async fn wait_on(listener: TcpListener, url: String) -> Result<(String, String), String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    crate::open_browser(&url)?;

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
                return Err(format!("The sign-in page returned an error: {err}"));
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

    let scope = "streaming user-read-email user-read-private user-library-read user-library-modify user-read-playback-state user-modify-playback-state playlist-read-private playlist-read-collaborative playlist-modify-private playlist-modify-public user-follow-read user-follow-modify";
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
    first_via().is_some()
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
    if first_via() == Some(Via::Playback) {
        return playback_token().await;
    }
    let cfg = load_config()?;
    Ok(usable_token(&cfg, None).await?.access_token)
}

// A token from this sign-in (refused: the one Spotify just turned down, replaced)
async fn token_via(via: Via, cfg: &Config, refused: Option<&str>) -> Result<String, String> {
    match via {
        Via::Playback => playback_token().await,
        Via::App => Ok(usable_token(cfg, refused).await?.access_token),
    }
}

pub async fn spotify_api(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    if method != "GET" {
        let r = spotify_api_now(method, path, body).await;
        forget_kept();
        return r;
    }
    if let Some(v) = kept(&path) { return Ok(v); }
    // (the same one already being asked: wait for it, then it's kept)
    let lock = ASKING.lock().map_err(|e| e.to_string())?.get_or_insert_with(Default::default).entry(path.clone()).or_default().clone();
    let _turn = lock.lock().await;
    if let Some(v) = kept(&path) { return Ok(v); }
    let r = spotify_api_now(method, path.clone(), body).await;
    if let Ok(v) = &r { keep(&path, v); }
    if let Ok(mut g) = ASKING.lock() { if let Some(m) = g.as_mut() { if std::sync::Arc::strong_count(&lock) <= 2 { m.remove(&path); } } }
    r
}

async fn spotify_api_now(method: String, path: String, body: Option<Value>) -> Result<Value, String> {
    let allowed = ["/me", "/search", "/tracks", "/albums", "/artists", "/playlists"];
    if !path.starts_with('/') || !allowed.iter().any(|p| path.starts_with(p)) {
        return Err("That Spotify path is not allowed".to_string());
    }
    let m = match method.as_str() {
        "GET" => reqwest::Method::GET,
        "PUT" => reqwest::Method::PUT,
        "POST" => reqwest::Method::POST,
        // unliking and unfollowing (DELETE /me/library) since Spotify's February 2026 changes
        "DELETE" => reqwest::Method::DELETE,
        _ => return Err("That method is not allowed".to_string()),
    };
    let cfg = load_config()?;
    let mut via = first_via().ok_or("Not connected to Spotify")?;
    let mut token = token_via(via, &cfg, None).await?;

    let url = format!("{}{path}", api_base());
    let client = reqwest::Client::new();
    let send = |token: String| {
        let mut rb = client.request(m.clone(), &url).bearer_auth(token);
        if let Some(b) = &body {
            rb = rb.json(b);
        }
        rb.send()
    };

    let mut tries = 0;
    let res = loop {
        // this sign-in limited for long: the other, if it's free; else said
        if let Err(e) = cool_down(via).await {
            match other_via(via) {
                Some(o) => { via = o; token = token_via(via, &cfg, None).await?; continue; }
                None => return Err(e),
            }
        }
        let res = {
            let _slot = GATE.acquire().await.map_err(|e| e.to_string())?;
            pace().await;
            send(token.clone()).await.map_err(|e| e.to_string())?
        };
        tries += 1;
        match res.status().as_u16() {
            // the playback sign-in's token turned down (or without the permission): your developer
            // app's, if you've signed in with one too
            401 | 403 if via == Via::Playback && tries < 3 && via_usable(Via::App) => {
                via = Via::App;
                token = token_via(via, &cfg, None).await?;
            }
            401 if via == Via::App && tries < 3 => token = token_via(via, &cfg, Some(&token)).await?,
            // your app's keys turned down (its owner without Premium, say): the playback sign-in's,
            // and (for that reason) not your app's again this run
            403 if via == Via::App && tries < 3 && playback_signed_in() => {
                let text = res.text().await.unwrap_or_default();
                if text.to_lowercase().contains("premium subscription required for the owner") {
                    APP_TURNED_DOWN.store(true, std::sync::atomic::Ordering::Relaxed);
                }
                via = Via::Playback;
                token = token_via(via, &cfg, None).await?;
            }
            429 => {
                let wait = retry_after(&res);
                set_cool_until(via, now_secs() + wait);
                // (the other sign-in has an allowance of its own: on with that)
                if let Some(o) = other_via(via).filter(|_| tries < 3) {
                    via = o;
                    token = token_via(via, &cfg, None).await?;
                    continue;
                }
                if wait > WAIT_AT_MOST || tries >= 3 {
                    return Err(format!("{RATE_LIMITED} right now: try again in {wait} s."));
                }
            }
            _ => break res,
        }
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn kept_and_forgotten() {
        forget_kept();
        assert!(kept("/me/tracks?limit=50&offset=0").is_none());
        keep("/me/tracks?limit=50&offset=0", &serde_json::json!({ "items": [1] }));
        assert_eq!(kept("/me/tracks?limit=50&offset=0"), Some(serde_json::json!({ "items": [1] })));
        assert!(kept("/me/tracks?limit=50&offset=50").is_none());
        forget_kept();
        assert!(kept("/me/tracks?limit=50&offset=0").is_none());
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // only makes the tests take turns
    async fn waits_out_short_rate_limits_and_refuses_long_ones() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let tmp = std::env::temp_dir().join(format!("omnipulse-sp-{}", std::process::id()));
        std::env::set_var("APPDATA", &tmp);
        let dir = data_dir().unwrap();
        std::fs::write(dir.join("config.json"), r#"{"client_id":"a","client_secret":"b","spotify_client_id":"c"}"#).unwrap();
        save_sp_tokens(&SpTokens { access_token: "tok".into(), refresh_token: Some("r".into()), expires_at: now_secs() + 3600 }).unwrap();

        // a fake Spotify: 429 (retry after 1 s), then 200, then 429 (retry after 60 s)
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let seen = hits.clone();
        std::thread::spawn(move || {
            for req in server.incoming_requests() {
                let n = seen.fetch_add(1, Ordering::SeqCst);
                let (code, after, body) = match n { 0 => (429, "1", "{}"), 1 => (200, "", r#"{"id":"me"}"#), _ => (429, "60", "{}") };
                let mut res = tiny_http::Response::from_string(body).with_status_code(code);
                if !after.is_empty() {
                    res.add_header(tiny_http::Header::from_bytes("Retry-After", after).unwrap());
                }
                let _ = req.respond(res);
            }
        });
        *API_BASE.lock().unwrap() = Some(format!("http://127.0.0.1:{port}"));
        COOL_UNTIL.store(0, std::sync::atomic::Ordering::SeqCst);
        COOL_PLAYBACK.store(0, std::sync::atomic::Ordering::SeqCst);

        // a short limit is waited out
        let t0 = std::time::Instant::now();
        let v = spotify_api("GET".into(), "/me".into(), None).await.unwrap();
        assert_eq!(v["id"], "me");
        assert!(t0.elapsed() >= std::time::Duration::from_millis(900));
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        // asked again: what it gave is kept (Spotify isn't asked)
        assert_eq!(spotify_api("GET".into(), "/me".into(), None).await.unwrap()["id"], "me");
        assert_eq!(hits.load(Ordering::SeqCst), 2);
        // a long one is reported as a rate limit (not a sign-out)...
        let e = spotify_api("GET".into(), "/me?again=1".into(), None).await.unwrap_err();
        assert!(e.starts_with(RATE_LIMITED) && e.contains("60 s"), "{e}");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        // ...and nothing more is sent to Spotify while it lasts
        let e = spotify_api("GET".into(), "/me?again=2".into(), None).await.unwrap_err();
        assert!(e.starts_with(RATE_LIMITED), "{e}");
        assert_eq!(hits.load(Ordering::SeqCst), 3);
        // the wait is known (for the app to say when), and kept for the next time the app opens
        assert!(spotify_wait() >= 58, "{}", spotify_wait());
        let saved: u64 = std::fs::read_to_string(dir.join("spotify_wait.txt")).unwrap().trim().parse().unwrap();
        assert!(saved >= now_secs() + 58);
        // the sign-in is untouched
        assert_eq!(load_sp_tokens().unwrap().access_token, "tok");

        COOL_UNTIL.store(0, std::sync::atomic::Ordering::SeqCst);
        *API_BASE.lock().unwrap() = None;
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
