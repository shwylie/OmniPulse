// Now playing widget for OBS: a tiny web page on this PC (127.0.0.1 only). OBS loads it as a
// Browser Source; the page asks /now.json every second for what's playing. It never plays the
// music. It can also show stream alerts over the card (follows, subs, bits...): they come from
// Streamer.bot's WebSocket (the page connects to it itself) or from a web request to /alert (Mix It
// Up's Web Request action, Streamer.bot's Fetch URL, anything), and each kind can have a sound,
// served from /sound/<kind> and played by the page, so OBS can route it.

use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicU64, Ordering},
    sync::Mutex,
    time::Duration,
};

pub static WIDGET_STATE: Mutex<Option<Value>> = Mutex::new(None);
pub static WIDGET_GEN: AtomicU64 = AtomicU64::new(0);
pub const WIDGET_HTML: &str = include_str!("../widget/widget.html");

/// The kinds of alert, as /alert?type= names them
pub const ALERT_KINDS: [&str; 9] = ["follow", "sub", "gift", "bits", "donation", "hypetrain", "poll", "ban", "watchstreak"];
/// The last few alerts, for the page to pick up (it shows the ones newer than it has seen)
static ALERTS: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
static ALERT_ID: AtomicU64 = AtomicU64::new(0);
const ALERTS_KEPT: usize = 20;

// Changes whenever widget.html changes; an open widget compares it and reloads itself
pub fn widget_version() -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in WIDGET_HTML.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:x}")
}

/// "a%20b+c" -> "a b c"
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => match std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                Some(v) => {
                    out.push(v);
                    i += 2;
                }
                None => out.push(b'%'),
            },
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn query_map(q: &str) -> serde_json::Map<String, Value> {
    let mut m = serde_json::Map::new();
    for pair in q.split('&').filter(|p| !p.is_empty()) {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        m.insert(url_decode(k).to_lowercase(), Value::String(url_decode(v)));
    }
    m
}

/// An alert, from a web request's fields or the app's Test button: { type, user, amount, message }.
/// Unknown kinds are refused, so a typo shows up in the reply.
pub fn alert_push(fields: &serde_json::Map<String, Value>) -> Result<Value, String> {
    let get = |k: &str| fields.get(k).map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let kind = get("type").to_lowercase();
    let kind = match kind.as_str() {
        "follower" | "follows" => "follow".to_string(),
        "subscription" | "subscriber" | "resub" => "sub".to_string(),
        "giftsub" | "gifted" | "giftbomb" | "gifts" => "gift".to_string(),
        "cheer" | "bit" => "bits".to_string(),
        "tip" | "donate" => "donation".to_string(),
        "hype" | "hype_train" | "hypetrainstart" | "hypetrainlevelup" | "hypetrainend" => "hypetrain".to_string(),
        "banned" => "ban".to_string(),
        "watch_streak" | "streak" => "watchstreak".to_string(),
        other => other.to_string(),
    };
    if !ALERT_KINDS.contains(&kind.as_str()) {
        return Err(format!("Unknown alert type \"{kind}\". Use one of: {}.", ALERT_KINDS.join(", ")));
    }
    let a = json!({
        "id": ALERT_ID.fetch_add(1, Ordering::SeqCst) + 1,
        "ts": crate::now_secs() * 1000,
        "type": kind,
        "user": get("user").chars().take(60).collect::<String>(),
        "amount": get("amount").chars().take(30).collect::<String>(),
        "message": get("message").chars().take(200).collect::<String>(),
        "test": fields.get("test").and_then(|v| v.as_bool()).unwrap_or(false),
    });
    let mut q = ALERTS.lock().unwrap();
    q.push_back(a.clone());
    while q.len() > ALERTS_KEPT {
        q.pop_front();
    }
    Ok(a)
}

// ---------- alert sounds: one file per kind, in <data>/alert-sounds/<kind>.<ext> ----------
fn sounds_dir() -> Result<std::path::PathBuf, String> {
    let d = crate::data_dir()?.join("alert-sounds");
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn sound_file(kind: &str) -> Option<std::path::PathBuf> {
    let d = sounds_dir().ok()?;
    std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(kind))
}

const SOUND_EXTS: [&str; 6] = ["mp3", "wav", "ogg", "m4a", "aac", "flac"];

/// Copies a sound in for one kind of alert (replacing its old one); returns its file name
pub fn alert_sound_set(kind: &str, path: &str) -> Result<String, String> {
    if !ALERT_KINDS.contains(&kind) {
        return Err("unknown alert type".into());
    }
    let src = std::path::Path::new(path);
    let ext = src.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
    if !SOUND_EXTS.contains(&ext.as_str()) {
        return Err(format!("Pick a sound file ({}).", SOUND_EXTS.join(", ")));
    }
    let size = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    if size > 10 * 1024 * 1024 {
        return Err("That sound is over 10 MB; alerts want something short.".into());
    }
    alert_sound_clear(kind);
    std::fs::copy(src, sounds_dir()?.join(format!("{kind}.{ext}"))).map_err(|e| e.to_string())?;
    Ok(src.file_name().and_then(|n| n.to_str()).unwrap_or("sound").to_string())
}

pub fn alert_sound_clear(kind: &str) {
    if let Some(p) = sound_file(kind) {
        let _ = std::fs::remove_file(p);
    }
}

fn sound_type(p: &std::path::Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("flac") => "audio/flac",
        _ => "audio/mpeg",
    }
}

fn reply(stream: &mut std::net::TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: Content-Type\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

pub fn widget_serve(listener: TcpListener, gen: u64) {
    let _ = listener.set_nonblocking(true);
    loop {
        if WIDGET_GEN.load(Ordering::SeqCst) != gen {
            break; // switched off or restarted
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let mut first = req.split_whitespace();
                let method = first.next().unwrap_or("GET").to_string();
                let target = first.next().unwrap_or("/").to_string();
                let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
                match path {
                    "/now.json" => {
                        let mut v = WIDGET_STATE.lock().ok().and_then(|g| g.clone()).unwrap_or_else(|| json!({ "playing": false }));
                        if let Some(o) = v.as_object_mut() {
                            o.insert("v".to_string(), Value::String(widget_version()));
                            o.insert("alerts".to_string(), Value::Array(ALERTS.lock().unwrap().iter().cloned().collect()));
                        }
                        reply(&mut stream, "200 OK", "application/json", v.to_string().as_bytes());
                    }
                    "/" | "/index.html" => {
                        reply(&mut stream, "200 OK", "text/html; charset=utf-8", WIDGET_HTML.replace("__WIDGET_VERSION__", &widget_version()).as_bytes());
                    }
                    // an alert: GET with ?type=&user=&amount=&message=, or POST with those as JSON
                    // or a form; the reply says what was understood
                    "/alert" => {
                        if method == "OPTIONS" {
                            reply(&mut stream, "204 No Content", "text/plain", b"");
                            continue;
                        }
                        let mut fields = query_map(query);
                        if method == "POST" {
                            let body = req.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("");
                            match serde_json::from_str::<Value>(body) {
                                Ok(Value::Object(o)) => fields.extend(o.into_iter().map(|(k, v)| (k.to_lowercase(), v))),
                                _ => fields.extend(query_map(body.trim())),
                            }
                        }
                        match alert_push(&fields) {
                            Ok(a) => reply(&mut stream, "200 OK", "application/json", json!({ "ok": true, "alert": a }).to_string().as_bytes()),
                            Err(e) => reply(&mut stream, "400 Bad Request", "application/json", json!({ "ok": false, "error": e }).to_string().as_bytes()),
                        }
                    }
                    p if p.starts_with("/sound/") => {
                        let kind = &p["/sound/".len()..];
                        match ALERT_KINDS.contains(&kind).then(|| sound_file(kind)).flatten().and_then(|f| std::fs::read(&f).ok().map(|b| (f, b))) {
                            Some((f, bytes)) => reply(&mut stream, "200 OK", sound_type(&f), &bytes),
                            None => reply(&mut stream, "404 Not Found", "text/plain", b"No sound"),
                        }
                    }
                    _ => reply(&mut stream, "404 Not Found", "text/plain", b"Not found"),
                }
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(60)),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

pub fn widget_start(port: u16) -> Result<String, String> {
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

pub fn widget_stop() {
    WIDGET_GEN.fetch_add(1, Ordering::SeqCst);
}

pub fn widget_update(state: Value) {
    if let Ok(mut g) = WIDGET_STATE.lock() {
        *g = Some(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_request_fields_are_read() {
        assert_eq!(url_decode("a%20b+c%26d"), "a b c&d");
        assert_eq!(url_decode("100%"), "100%");
        let m = query_map("type=Cheer&user=Ninja%20Fan&amount=500&message=gg+wp");
        let a = alert_push(&m).unwrap();
        assert_eq!((a["type"].as_str(), a["user"].as_str(), a["amount"].as_str(), a["message"].as_str()), (Some("bits"), Some("Ninja Fan"), Some("500"), Some("gg wp")));
        assert!(alert_push(&query_map("type=raidd&user=x")).is_err());
    }
}
