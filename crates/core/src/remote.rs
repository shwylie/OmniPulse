// Remote control for the Stream Deck plugin (or anything else on this PC):
//   GET /control?key=KEY&action=ACTION   presses a button, answers with the state after it
//   GET /state?key=KEY                   just the state
// Actions: play, pause, toggle, next, previous, like, favourite, shuffle, repeat, clear-requests,
// clear-queue, sets. The state: { ok, playing, title, artist, shuffle, repeat, liked, canLike,
// favourite, setsAllowed, requests }.
//
// It only listens on 127.0.0.1 and needs the secret key from Settings. While it runs, the port and
// key are written to <data dir>/remote.json, which is where the Stream Deck plugin finds them.
//
// The app registers the handler (set_handler): it presses the button, then answers with remote_result.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::{ErrorKind, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::time::Duration;

/// Called with (request id, action) for each button press
pub type Handler = Arc<dyn Fn(u64, &str) + Send + Sync>;
static HANDLER: Mutex<Option<Handler>> = Mutex::new(None);

pub fn set_handler(f: Handler) {
    if let Ok(mut h) = HANDLER.lock() {
        *h = Some(f);
    }
}

const ACTIONS: &[&str] = &[
    "play", "pause", "toggle", "next", "previous", "like", "favourite", "shuffle", "repeat",
    "clear-requests", "clear-queue", "sets", "state",
    // Seamless Mix feedback: the last transition, rated (logged with how it was mixed)
    "mix-good", "mix-bad",
];

static GEN: AtomicU64 = AtomicU64::new(0);
static NEXT: AtomicU64 = AtomicU64::new(1);
static KEY: Mutex<String> = Mutex::new(String::new());
static WAIT: OnceLock<Mutex<HashMap<u64, mpsc::Sender<Value>>>> = OnceLock::new();

fn waiting() -> &'static Mutex<HashMap<u64, mpsc::Sender<Value>>> {
    WAIT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn param(url: &str, key: &str) -> Option<String> {
    let q = url.split('?').nth(1)?;
    q.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=')?;
        (k == key).then(|| urlencoding::decode(v).ok().map(|c| c.into_owned())).flatten()
    })
}

// Hands the action to the app, then waits for the state it sends back
fn ask_page(action: &str) -> Value {
    let Some(handler) = HANDLER.lock().ok().and_then(|h| h.clone()) else {
        return json!({ "ok": false, "message": "The music app isn't ready yet." });
    };
    let id = NEXT.fetch_add(1, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    if let Ok(mut m) = waiting().lock() {
        m.insert(id, tx);
    }
    handler(id, action);
    let answer = rx.recv_timeout(Duration::from_secs(5));
    if let Ok(mut m) = waiting().lock() {
        m.remove(&id);
    }
    answer.unwrap_or_else(|_| json!({ "ok": false, "message": "The music app didn't answer in time." }))
}

fn handle(mut stream: TcpStream) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let mut buf = [0u8; 2048];
    let n = stream.read(&mut buf).unwrap_or(0);
    let req = String::from_utf8_lossy(&buf[..n]).to_string();
    let url = req.split_whitespace().nth(1).unwrap_or("/").to_string();
    let route = url.split('?').next().unwrap_or("/");
    let expected = KEY.lock().map(|k| k.clone()).unwrap_or_default();
    let key_ok = !expected.is_empty() && param(&url, "key").as_deref() == Some(expected.as_str());

    let (status, body) = match route {
        "/state" | "/control" if !key_ok => ("403 Forbidden", json!({ "ok": false, "message": "Wrong or missing key." })),
        "/state" => ("200 OK", ask_page("state")),
        "/control" => match param(&url, "action") {
            Some(a) if ACTIONS.contains(&a.as_str()) => ("200 OK", ask_page(&a)),
            _ => ("400 Bad Request", json!({ "ok": false, "message": "Unknown action.", "actions": ACTIONS })),
        },
        _ => ("404 Not Found", json!({ "ok": false, "message": "Not found." })),
    };
    let body = body.to_string();
    let resp = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(resp.as_bytes());
}

fn serve(listener: TcpListener, gen: u64) {
    let _ = listener.set_nonblocking(true);
    while GEN.load(Ordering::SeqCst) == gen {
        match listener.accept() {
            Ok((stream, _)) => {
                std::thread::spawn(move || handle(stream));
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(40)),
            Err(_) => std::thread::sleep(Duration::from_millis(200)),
        }
    }
}

fn info_file() -> Option<std::path::PathBuf> {
    crate::data_dir().ok().map(|d| d.join("remote.json"))
}

pub fn remote_start(port: u16, key: String) -> Result<String, String> {
    if key.len() < 16 {
        return Err("The key is too short.".into());
    }
    if let Ok(mut k) = KEY.lock() {
        *k = key.clone();
    }
    let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let mut last = String::new();
    for _ in 0..15 {
        match TcpListener::bind(("127.0.0.1", port)) {
            Ok(listener) => {
                std::thread::spawn(move || serve(listener, gen));
                if let Some(f) = info_file() {
                    let _ = std::fs::write(f, json!({ "port": port, "key": key }).to_string());
                }
                return Ok(format!("http://127.0.0.1:{port}/"));
            }
            Err(e) => {
                last = e.to_string();
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    Err(format!("Port {port} is in use or blocked ({last}). Try another port."))
}

pub fn remote_stop() {
    GEN.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut k) = KEY.lock() {
        k.clear();
    }
    if let Some(f) = info_file() {
        let _ = std::fs::remove_file(f);
    }
}

// The app's answer: the state after the action
pub fn remote_result(id: u64, state: Value) {
    if let Ok(m) = waiting().lock() {
        if let Some(tx) = m.get(&id) {
            let _ = tx.send(state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_query_parameters() {
        let url = "/control?key=abc%20def&action=clear-queue";
        assert_eq!(param(url, "key").as_deref(), Some("abc def"));
        assert_eq!(param(url, "action").as_deref(), Some("clear-queue"));
        assert_eq!(param(url, "nope"), None);
    }

    #[test]
    fn refuses_wrong_keys_and_unknown_actions() {
        if let Ok(mut k) = KEY.lock() {
            *k = "0123456789abcdef0123".into();
        }
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let gen = GEN.fetch_add(1, Ordering::SeqCst) + 1;
        std::thread::spawn(move || serve(listener, gen));
        let get = |path: &str| {
            let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
            write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).unwrap();
            out
        };
        assert!(get("/state?key=wrong").starts_with("HTTP/1.1 403"));
        assert!(get("/control?key=0123456789abcdef0123&action=format-c").starts_with("HTTP/1.1 400"));
        assert!(get("/elsewhere").starts_with("HTTP/1.1 404"));
        // a valid request reaches the app; with no app running it says so instead of hanging
        let ok = get("/control?key=0123456789abcdef0123&action=next");
        assert!(ok.starts_with("HTTP/1.1 200") && ok.contains("isn't ready"), "{ok}");
        // with an app: it gets the action and its answer comes back as the reply
        set_handler(Arc::new(|id, action| {
            let action = action.to_string();
            std::thread::spawn(move || remote_result(id, json!({ "ok": true, "did": action })));
        }));
        let ok = get("/control?key=0123456789abcdef0123&action=shuffle");
        assert!(ok.starts_with("HTTP/1.1 200") && ok.contains(r#""did":"shuffle""#), "{ok}");
        GEN.fetch_add(1, Ordering::SeqCst);
    }
}
