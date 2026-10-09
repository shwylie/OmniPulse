// Song requests for chat bots (Streamer.bot, Mix It Up, ...)
// GET /request?key=...&user=...&q=...  adds a SoundCloud track to the end of the queue.
// GET /wrongsong?key=...&user=...&mod=... takes back that viewer's last request (a moderator's:
//                                      the last request anyone made).
// GET /now                             says what's playing.
// Replies are plain text, ready to post in chat (add &format=json for JSON).
// Only listens on 127.0.0.1, and requests need the secret key from Settings.
//
// The app registers the asker (set_asker): it searches and queues, then answers with request_result.

use crate::widget::WIDGET_STATE;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Called with (request id, the request) for each one: { action: "request" | "wrongsong", q, user, mod }
pub type Asker = Arc<dyn Fn(u64, Value) + Send + Sync>;
static ASKER: Mutex<Option<Asker>> = Mutex::new(None);

pub fn set_asker(f: Asker) {
    if let Ok(mut a) = ASKER.lock() {
        *a = Some(f);
    }
}

pub static REQ_GEN: AtomicU64 = AtomicU64::new(0);
pub static REQ_NEXT: AtomicU64 = AtomicU64::new(1);
pub static REQ_KEY: Mutex<String> = Mutex::new(String::new());
pub static REQ_WAIT: OnceLock<Mutex<HashMap<u64, std::sync::mpsc::Sender<Value>>>> = OnceLock::new();

pub fn req_wait() -> &'static Mutex<HashMap<u64, std::sync::mpsc::Sender<Value>>> {
    REQ_WAIT.get_or_init(|| Mutex::new(HashMap::new()))
}

// Like query_param, but also treats "+" as a space (how some bots encode text)
pub fn req_param(url: &str, key: &str) -> Option<String> {
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
pub fn ask_page(ask: Value) -> Value {
    let Some(asker) = ASKER.lock().ok().and_then(|a| a.clone()) else {
        return serde_json::json!({ "ok": false, "message": "The music app isn't ready yet." });
    };
    let id = REQ_NEXT.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = std::sync::mpsc::channel();
    if let Ok(mut m) = req_wait().lock() {
        m.insert(id, tx);
    }
    asker(id, ask);
    let answer = rx.recv_timeout(Duration::from_secs(12));
    if let Ok(mut m) = req_wait().lock() {
        m.remove(&id);
    }
    answer.unwrap_or_else(|_| serde_json::json!({ "ok": false, "message": "The music app didn't answer in time." }))
}

pub fn handle_req(mut stream: TcpStream) {
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
                ("200 OK", ask_page(serde_json::json!({ "action": "request", "q": q, "user": user, "mod": false })))
            }
        }
        "/wrongsong" => {
            let expected = REQ_KEY.lock().map(|k| k.clone()).unwrap_or_default();
            let key = req_param(&url, "key").unwrap_or_default();
            if expected.is_empty() || key != expected {
                ("403 Forbidden", serde_json::json!({ "ok": false, "message": "Wrong or missing key." }))
            } else {
                let user = req_param(&url, "user").unwrap_or_default();
                // a moderator: however the bot says so (True, 1, yes, mod, broadcaster...)
                let m = req_param(&url, "mod").unwrap_or_default().trim().to_lowercase();
                let is_mod = matches!(m.as_str(), "true" | "1" | "yes" | "y" | "mod" | "moderator" | "broadcaster" | "streamer" | "owner");
                ("200 OK", ask_page(serde_json::json!({ "action": "wrongsong", "q": "", "user": user, "mod": is_mod })))
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

pub fn req_serve(listener: TcpListener, gen: u64) {
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

/// remote: bots on other devices may ask too (requests always need the key); otherwise only this PC
pub fn requests_start(port: u16, key: String, remote: bool) -> Result<String, String> {
    if let Ok(mut k) = REQ_KEY.lock() {
        *k = key;
    }
    let gen = REQ_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let mut last_err = String::new();
    for _ in 0..15 {
        match TcpListener::bind((if remote { "0.0.0.0" } else { "127.0.0.1" }, port)) {
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

pub fn requests_stop() {
    REQ_GEN.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut k) = REQ_KEY.lock() {
        k.clear();
    }
}

// The page's answer to a request
pub fn request_result(id: u64, result: Value) {
    if let Ok(m) = req_wait().lock() {
        if let Some(tx) = m.get(&id) {
            let _ = tx.send(result);
        }
    }
}
