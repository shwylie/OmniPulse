// Spotify on the phone: librespot's session for your library (likes, playlists, songs), as the
// desktop's playback sign-in gives it, without playing through it (the phone plays Spotify's songs
// from their YouTube Music versions). Signed in once in the browser, as the desktop is (Spotify's own
// app's client id, PKCE, the reply caught on 127.0.0.1:5588); the reusable credentials are kept in
// <data dir>/librespot, and the session's made again from them.

use librespot_core::{authentication::Credentials, cache::Cache, config::SessionConfig, session::Session};
use rand::Rng;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const REDIRECT: &str = "http://127.0.0.1:5588/login";
const SCOPES: &str = "streaming user-read-private user-read-email user-read-playback-state user-modify-playback-state user-library-read playlist-read-private";
/// How long the browser's sign-in is waited for
const SIGN_IN_WAIT_S: u64 = 300;

static SESSION: tokio::sync::Mutex<Option<Session>> = tokio::sync::Mutex::const_new(None);

fn cache() -> Result<Cache, String> {
    let dir = crate::data_dir()?.join("librespot");
    Cache::new(Some(&dir), None, None, None).map_err(|e| format!("librespot cache: {e}"))
}

pub fn signed_in() -> bool {
    cache().ok().and_then(|c| c.credentials()).is_some()
}

async fn connect(creds: Credentials) -> Result<Session, String> {
    let session = Session::new(SessionConfig::default(), Some(cache()?));
    session.connect(creds, true).await.map_err(|e| format!("Spotify sign-in failed: {e}"))?;
    crate::diag::note(&format!("spotify: signed in as {}", session.username()));
    Ok(session)
}

/// The session (made again from the kept sign-in if it dropped)
pub async fn session() -> Result<Session, String> {
    let mut g = SESSION.lock().await;
    if let Some(s) = g.as_ref().filter(|s| !s.is_invalid()) { return Ok(s.clone()); }
    let creds = cache()?.credentials().ok_or("NOT_SIGNED_IN")?;
    let s = connect(creds).await?;
    *g = Some(s.clone());
    Ok(s)
}

fn base64url(b: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b)
}

/// Signs in in the browser: Spotify's page opened (through the app's opener), its reply caught here
pub async fn login() -> Result<String, String> {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~";
    let (verifier, state): (String, String) = {
        let mut r = rand::thread_rng();
        ((0..64).map(|_| CHARS[r.gen_range(0..CHARS.len())] as char).collect(), (0..16).map(|_| CHARS[r.gen_range(0..62)] as char).collect())
    };
    let challenge = base64url(&Sha256::digest(verifier.as_bytes()));
    let client_id = SessionConfig::default().client_id;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:5588").await.map_err(|e| format!("Spotify sign-in couldn't listen for its reply: {e}"))?;
    let url = format!(
        "https://accounts.spotify.com/authorize?client_id={client_id}&response_type=code&redirect_uri={}&code_challenge_method=S256&code_challenge={challenge}&scope={}&state={state}",
        urlenc(REDIRECT), urlenc(SCOPES)
    );
    crate::open_browser(&url)?;
    // the browser comes back to 127.0.0.1:5588/login?code=...&state=...
    let code = tokio::time::timeout(std::time::Duration::from_secs(SIGN_IN_WAIT_S), async {
        loop {
            let (mut sock, _) = listener.accept().await.map_err(|e| e.to_string())?;
            let mut buf = vec![0u8; 8192];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let req = String::from_utf8_lossy(&buf[..n]).to_string();
            let path = req.lines().next().unwrap_or("").split_whitespace().nth(1).unwrap_or("").to_string();
            if !path.starts_with("/login") { let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n").await; continue; }
            let q = |k: &str| path.split(['?', '&']).find_map(|p| p.strip_prefix(&format!("{k}="))).map(String::from);
            let page = "<html><body style=\"font-family:sans-serif;padding:2em\"><h2>Signed in to Spotify</h2><p>You can go back to OmniPulse.</p></body></html>";
            let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{page}", page.len()).as_bytes()).await;
            if q("state").as_deref() != Some(state.as_str()) { return Err("Spotify's sign-in came back for another request".to_string()); }
            if let Some(e) = q("error") { return Err(format!("Spotify's sign-in said: {e}")); }
            return q("code").ok_or_else(|| "Spotify's sign-in gave no code".to_string());
        }
    }).await.map_err(|_| "Spotify's sign-in wasn't finished in time".to_string())??;
    // the code for a token, then a session with it (its reusable credentials kept)
    let res = reqwest::Client::new()
        .post("https://accounts.spotify.com/api/token")
        .form(&[("grant_type", "authorization_code"), ("code", code.as_str()), ("redirect_uri", REDIRECT), ("client_id", client_id.as_str()), ("code_verifier", verifier.as_str())])
        .send().await.map_err(|e| format!("Couldn't reach Spotify: {e}"))?;
    let v: serde_json::Value = res.json().await.map_err(|e| e.to_string())?;
    let token = v["access_token"].as_str().ok_or_else(|| format!("Spotify gave no token: {}", v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("")))?;
    let s = connect(Credentials::with_access_token(token)).await?;
    let name = s.username();
    *SESSION.lock().await = Some(s);
    Ok(name)
}

pub async fn logout() -> Result<(), String> {
    if let Some(s) = SESSION.lock().await.take() { s.shutdown(); }
    let dir = crate::data_dir()?.join("librespot");
    if dir.exists() { std::fs::remove_dir_all(&dir).map_err(|e| e.to_string())?; }
    Ok(())
}

fn urlenc(s: &str) -> String {
    s.bytes().map(|b| match b {
        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
        _ => format!("%{b:02X}"),
    }).collect()
}
