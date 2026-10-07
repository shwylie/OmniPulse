// Now playing widget for OBS: a tiny web page on this PC (127.0.0.1 only). OBS loads it as a
// Browser Source; the page asks /now.json every second for what's playing. It never plays audio.

use serde_json::Value;
use std::{
    io::{ErrorKind, Read, Write},
    net::TcpListener,
    sync::atomic::{AtomicU64, Ordering},
    sync::Mutex,
    time::Duration,
};

pub static WIDGET_STATE: Mutex<Option<Value>> = Mutex::new(None);
pub static WIDGET_GEN: AtomicU64 = AtomicU64::new(0);
pub const WIDGET_HTML: &str = include_str!("../widget/widget.html");

// Changes whenever widget.html changes; an open widget compares it and reloads itself
pub fn widget_version() -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in WIDGET_HTML.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:x}")
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
