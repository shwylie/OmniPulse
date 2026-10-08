// Casting to Google Cast devices (Chromecast, Nest speakers and displays, TVs and speakers with Cast
// built in) on the same network.
//
// The app stays the player: its sound, after the EQ, loudness match and limiter, is sent here as PCM
// and served to the device as one live WAV stream, so every source (Spotify too), trims and the queue
// work as they do on the PC. The device is found by mDNS, and controlled over the Cast protocol
// (CASTV2: protobuf messages over TLS on the device's port).
//
// The stream is served only while casting, on the one network address that reaches the device, under
// a random path; nothing else is served there.

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::{HashMap, VecDeque};
use std::io::{Read, Write};
use std::net::{IpAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Google's Default Media Receiver: plays a stream and shows its title, artist and cover
pub const DEFAULT_APP: &str = "CC1AD845";
const NS_CONN: &str = "urn:x-cast:com.google.cast.tp.connection";
const NS_HEART: &str = "urn:x-cast:com.google.cast.tp.heartbeat";
const NS_RECV: &str = "urn:x-cast:com.google.cast.receiver";
const NS_MEDIA: &str = "urn:x-cast:com.google.cast.media";
const SENDER: &str = "sender-omnipulse";

#[derive(Clone, Serialize, Debug)]
pub struct Device {
    pub id: String,
    pub name: String,
    pub model: String,
    pub host: String,
    pub port: u16,
    /// has a screen (a Nest Hub, a TV)
    pub display: bool,
    /// a speaker group
    pub group: bool,
}

// ---------- finding devices ----------
static DEVICES: Mutex<Option<HashMap<String, Device>>> = Mutex::new(None);
static BROWSING: AtomicBool = AtomicBool::new(false);

/// Starts looking for devices (once; it keeps listening for devices coming and going)
pub fn discover_start() -> Result<(), String> {
    if BROWSING.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let daemon = mdns_sd::ServiceDaemon::new().map_err(|e| {
        BROWSING.store(false, Ordering::SeqCst);
        format!("Couldn't look for Cast devices: {e}")
    })?;
    let rx = daemon.browse("_googlecast._tcp.local.").map_err(|e| {
        BROWSING.store(false, Ordering::SeqCst);
        format!("Couldn't look for Cast devices: {e}")
    })?;
    std::thread::spawn(move || {
        let _keep = daemon;
        while let Ok(ev) = rx.recv() {
            match ev {
                mdns_sd::ServiceEvent::ServiceResolved(info) => {
                    let addr = info.get_addresses().iter().find(|a| a.is_ipv4()).or(info.get_addresses().iter().next()).copied();
                    let Some(addr) = addr else { continue };
                    let prop = |k: &str| info.get_property_val_str(k).unwrap_or("").to_string();
                    let ca: u64 = prop("ca").parse().unwrap_or(0);
                    let id = if prop("id").is_empty() { info.get_fullname().to_string() } else { prop("id") };
                    let d = Device {
                        id: id.clone(),
                        name: if prop("fn").is_empty() { info.get_hostname().trim_end_matches('.').to_string() } else { prop("fn") },
                        model: prop("md"),
                        host: addr.to_string(),
                        port: info.get_port(),
                        display: ca & 1 == 1,
                        group: ca & 32 == 32 || prop("md") == "Google Cast Group",
                    };
                    DEVICES.lock().unwrap().get_or_insert_with(HashMap::new).insert(id, d);
                }
                mdns_sd::ServiceEvent::ServiceRemoved(_, full) => {
                    if let Some(map) = DEVICES.lock().unwrap().as_mut() {
                        map.retain(|_, d| !full.contains(&d.id));
                    }
                }
                _ => {}
            }
        }
        BROWSING.store(false, Ordering::SeqCst);
    });
    Ok(())
}

pub fn devices() -> Vec<Device> {
    let mut list: Vec<Device> = DEVICES.lock().unwrap().as_ref().map(|m| m.values().cloned().collect()).unwrap_or_default();
    list.sort_by_key(|d| d.name.to_lowercase());
    list
}

// ---------- the Cast protocol's messages ----------
// CastMessage { protocol_version=1 (0), source_id=2, destination_id=3, namespace=4, payload_type=5 (0: text), payload_utf8=6 }
fn varint(out: &mut Vec<u8>, mut v: u64) {
    loop {
        let b = (v & 0x7f) as u8;
        v >>= 7;
        if v == 0 {
            out.push(b);
            return;
        }
        out.push(b | 0x80);
    }
}

pub(crate) fn encode(src: &str, dst: &str, ns: &str, payload: &str) -> Vec<u8> {
    let mut m = vec![0x08, 0x00];
    for (tag, s) in [(0x12u8, src), (0x1a, dst), (0x22, ns)] {
        m.push(tag);
        varint(&mut m, s.len() as u64);
        m.extend_from_slice(s.as_bytes());
    }
    m.extend_from_slice(&[0x28, 0x00, 0x32]);
    varint(&mut m, payload.len() as u64);
    m.extend_from_slice(payload.as_bytes());
    let mut framed = (m.len() as u32).to_be_bytes().to_vec();
    framed.extend(m);
    framed
}

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Msg {
    pub src: String,
    pub dst: String,
    pub ns: String,
    pub payload: String,
}

pub(crate) fn decode(b: &[u8]) -> Option<Msg> {
    let mut m = Msg::default();
    let mut i = 0;
    let read_var = |i: &mut usize| -> Option<u64> {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let byte = *b.get(*i)?;
            *i += 1;
            v |= ((byte & 0x7f) as u64) << shift;
            if byte & 0x80 == 0 {
                return Some(v);
            }
            shift += 7;
            if shift > 63 {
                return None;
            }
        }
    };
    while i < b.len() {
        let key = read_var(&mut i)?;
        let (field, wire) = (key >> 3, key & 7);
        match wire {
            0 => {
                read_var(&mut i)?;
            }
            2 => {
                let len = read_var(&mut i)? as usize;
                let s = b.get(i..i + len)?;
                i += len;
                let text = String::from_utf8_lossy(s).into_owned();
                match field {
                    2 => m.src = text,
                    3 => m.dst = text,
                    4 => m.ns = text,
                    6 => m.payload = text,
                    _ => {}
                }
            }
            _ => return None,
        }
    }
    Some(m)
}

// ---------- the live stream ----------
// The last few seconds of sound, so a device that (re)connects starts where the track started
// instead of losing what played while it was buffering
const RING_SECS: usize = 8;

struct Ring {
    data: VecDeque<u8>,
    start: u64,
    mark: u64,
    rate: u32,
}

struct Live {
    ring: Mutex<Ring>,
    more: Condvar,
}

static LIVE: Live = Live { ring: Mutex::new(Ring { data: VecDeque::new(), start: 0, mark: 0, rate: 48000 }), more: Condvar::new() };

/// 16-bit stereo PCM from the page, at its audio context's rate
pub fn push_pcm(bytes: &[u8], rate: u32) {
    let mut r = LIVE.ring.lock().unwrap();
    if rate != r.rate && rate > 0 {
        r.rate = rate;
        r.start += r.data.len() as u64;
        r.data.clear();
    }
    let n = bytes.len() & !3;
    r.data.extend(&bytes[..n]);
    let cap = RING_SECS * r.rate as usize * 4;
    if r.data.len() > cap {
        let drop = (r.data.len() - cap) & !3;
        r.data.drain(..drop);
        r.start += drop as u64;
    }
    drop(r);
    LIVE.more.notify_all();
}

/// A new track starts here: a device connecting from now on starts from this point
pub fn mark() {
    let mut r = LIVE.ring.lock().unwrap();
    r.mark = r.start + r.data.len() as u64;
}

fn wav_header(rate: u32) -> Vec<u8> {
    let mut h = Vec::with_capacity(44);
    h.extend_from_slice(b"RIFF");
    h.extend_from_slice(&u32::MAX.to_le_bytes());
    h.extend_from_slice(b"WAVEfmt ");
    h.extend_from_slice(&16u32.to_le_bytes());
    h.extend_from_slice(&1u16.to_le_bytes());
    h.extend_from_slice(&2u16.to_le_bytes());
    h.extend_from_slice(&rate.to_le_bytes());
    h.extend_from_slice(&(rate * 4).to_le_bytes());
    h.extend_from_slice(&4u16.to_le_bytes());
    h.extend_from_slice(&16u16.to_le_bytes());
    h.extend_from_slice(b"data");
    h.extend_from_slice(&(u32::MAX - 36).to_le_bytes());
    h
}

fn serve_listener(listener: TcpListener, path: String, alive: Arc<AtomicBool>) {
    let _ = listener.set_nonblocking(true);
    while alive.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((conn, _)) => {
                let (path, alive) = (path.clone(), alive.clone());
                std::thread::spawn(move || {
                    let _ = serve_conn(conn, &path, &alive);
                });
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
}

fn serve_conn(mut conn: TcpStream, path: &str, alive: &AtomicBool) -> std::io::Result<()> {
    conn.set_nonblocking(false)?;
    conn.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut head = Vec::new();
    let mut buf = [0u8; 1024];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < 8192 {
        let n = conn.read(&mut buf)?;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
    }
    let text = String::from_utf8_lossy(&head);
    let mut first = text.lines().next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("");
    let target = first.next().unwrap_or("");
    if target != path || !(method == "GET" || method == "HEAD") {
        conn.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")?;
        return Ok(());
    }
    conn.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: audio/wav\r\nCache-Control: no-cache, no-store\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\n\r\n")?;
    if method == "HEAD" {
        return Ok(());
    }
    let (mut pos, rate) = {
        let r = LIVE.ring.lock().unwrap();
        (r.mark.max(r.start), r.rate)
    };
    conn.write_all(&wav_header(rate))?;
    conn.set_write_timeout(Some(Duration::from_secs(10)))?;
    let mut chunk = Vec::with_capacity(65536);
    while alive.load(Ordering::SeqCst) {
        chunk.clear();
        {
            let mut r = LIVE.ring.lock().unwrap();
            if pos < r.start {
                pos = r.start; // fell behind: carry on from the oldest sound kept
            }
            let end = r.start + r.data.len() as u64;
            if pos >= end {
                r = LIVE.more.wait_timeout(r, Duration::from_secs(1)).unwrap().0;
                let end = r.start + r.data.len() as u64;
                if pos >= end || pos < r.start {
                    continue;
                }
            }
            let from = (pos - r.start) as usize;
            let to = (from + 65536).min(r.data.len());
            let (a, b) = r.data.as_slices();
            if from < a.len() {
                chunk.extend_from_slice(&a[from..to.min(a.len())]);
                if to > a.len() {
                    chunk.extend_from_slice(&b[..to - a.len()]);
                }
            } else {
                chunk.extend_from_slice(&b[from - a.len()..to - a.len()]);
            }
        }
        conn.write_all(&chunk)?;
        pos += chunk.len() as u64;
    }
    Ok(())
}

// ---------- a cast session ----------
type Events = Arc<dyn Fn(Value) + Send + Sync>;

struct Session {
    out: mpsc::Sender<Vec<u8>>,
    alive: Arc<AtomicBool>,
    transport: Mutex<String>,
    session_id: Mutex<String>,
    media: Mutex<Option<i64>>,
    req: AtomicI64,
    app: String,
    url: String,
    device: Device,
    meta: Mutex<Value>,
}

static SESSION: Mutex<Option<Arc<Session>>> = Mutex::new(None);

impl Session {
    fn send(&self, dst: &str, ns: &str, payload: Value) {
        let _ = self.out.send(encode(SENDER, dst, ns, &payload.to_string()));
    }
    fn next_req(&self) -> i64 {
        self.req.fetch_add(1, Ordering::SeqCst)
    }
    fn load(&self) {
        let meta = self.meta.lock().unwrap().clone();
        let mut m = json!({ "metadataType": 3 });
        for (k, to) in [("title", "title"), ("artist", "artist"), ("album", "albumName")] {
            if let Some(v) = meta.get(k).and_then(|v| v.as_str()) {
                m[to] = json!(v);
            }
        }
        if let Some(img) = meta.get("art").and_then(|v| v.as_str()).filter(|s| s.starts_with("http")) {
            m["images"] = json!([{ "url": img }]);
        }
        let t = self.transport.lock().unwrap().clone();
        let sid = self.session_id.lock().unwrap().clone();
        self.send(&t, NS_MEDIA, json!({
            "type": "LOAD", "requestId": self.next_req(), "sessionId": sid, "autoplay": true, "currentTime": 0,
            "media": { "contentId": self.url, "contentUrl": self.url, "contentType": "audio/wav", "streamType": "LIVE", "metadata": m },
        }));
    }
}

fn local_ip_for(host: &str, port: u16) -> Result<IpAddr, String> {
    let s = UdpSocket::bind("0.0.0.0:0").map_err(|e| e.to_string())?;
    s.connect((host, port)).map_err(|e| format!("Can't reach {host}: {e}"))?;
    Ok(s.local_addr().map_err(|e| e.to_string())?.ip())
}

/// Casts to a device: opens Google's player there and plays the live stream. `meta`: { title, artist,
/// album, art }, which its screen (if it has one) shows.
pub fn start(device_id: &str, meta: Value, events: Events) -> Result<Device, String> {
    stop();
    let device = devices().into_iter().find(|d| d.id == device_id).ok_or("That device isn't on the network any more.")?;
    let app = DEFAULT_APP.to_string();

    // the stream, on the address that reaches the device
    let ip = local_ip_for(&device.host, device.port)?;
    let listener = TcpListener::bind((ip, 0)).map_err(|e| format!("Couldn't open the stream for the device: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    let secret: String = (0..24).map(|_| format!("{:x}", rand::random::<u8>() & 15)).collect();
    let path = format!("/live/{secret}.wav");
    let url = format!("http://{ip}:{port}{path}");
    let alive = Arc::new(AtomicBool::new(true));
    mark();
    {
        let alive = alive.clone();
        std::thread::spawn(move || serve_listener(listener, path, alive));
    }

    // the device
    let tcp = TcpStream::connect_timeout(&(device.host.parse::<IpAddr>().map_err(|e| e.to_string())?, device.port).into(), Duration::from_secs(5))
        .map_err(|e| { alive.store(false, Ordering::SeqCst); format!("Couldn't connect to {}: {e}", device.name) })?;
    let _ = tcp.set_nodelay(true);
    // Cast devices present their own self-made certificates, so there's no name or authority to check
    let tls = native_tls::TlsConnector::builder()
        .danger_accept_invalid_certs(true)
        .danger_accept_invalid_hostnames(true)
        .build()
        .map_err(|e| e.to_string())?;
    let mut stream = tls.connect(&device.host, tcp).map_err(|e| { alive.store(false, Ordering::SeqCst); format!("Couldn't connect to {}: {e}", device.name) })?;
    let _ = stream.get_ref().set_read_timeout(Some(Duration::from_millis(100)));

    let (out, out_rx) = mpsc::channel::<Vec<u8>>();
    let sess = Arc::new(Session {
        out, alive: alive.clone(), transport: Mutex::new(String::new()), session_id: Mutex::new(String::new()), media: Mutex::new(None),
        req: AtomicI64::new(1), app: app.clone(), url, device: device.clone(), meta: Mutex::new(meta),
    });
    sess.send("receiver-0", NS_CONN, json!({ "type": "CONNECT" }));
    sess.send("receiver-0", NS_RECV, json!({ "type": "LAUNCH", "appId": app, "requestId": sess.next_req() }));
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
    {
        let sess = sess.clone();
        std::thread::spawn(move || {
            let mut ready = Some(ready_tx);
            let mut buf: Vec<u8> = Vec::new();
            let mut tmp = [0u8; 16384];
            let mut last_ping = Instant::now();
            let mut last_heard = Instant::now();
            let started = Instant::now();
            let mut why = String::new();
            while sess.alive.load(Ordering::SeqCst) {
                while let Ok(m) = out_rx.try_recv() {
                    if stream.write_all(&m).is_err() {
                        why = "The connection to the device was lost.".into();
                        sess.alive.store(false, Ordering::SeqCst);
                    }
                }
                if last_ping.elapsed() > Duration::from_secs(5) {
                    let _ = stream.write_all(&encode(SENDER, "receiver-0", NS_HEART, r#"{"type":"PING"}"#));
                    last_ping = Instant::now();
                }
                if last_heard.elapsed() > Duration::from_secs(20) {
                    why = "The device stopped answering.".into();
                    break;
                }
                if let Some(tx) = &ready {
                    if started.elapsed() > Duration::from_secs(20) {
                        let _ = tx.send(Err(format!("{} didn't open the player.", sess.device.name)));
                        ready = None;
                        break;
                    }
                }
                match stream.read(&mut tmp) {
                    Ok(0) => { why = "The device closed the connection.".into(); break; }
                    Ok(n) => buf.extend_from_slice(&tmp[..n]),
                    Err(e) if matches!(e.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut) => {}
                    Err(e) => { why = format!("The connection to the device was lost ({e})."); break; }
                }
                while buf.len() >= 4 {
                    let len = u32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
                    if buf.len() < 4 + len {
                        break;
                    }
                    let frame: Vec<u8> = buf.drain(..4 + len).skip(4).collect();
                    let Some(m) = decode(&frame) else { continue };
                    last_heard = Instant::now();
                    let p: Value = serde_json::from_str(&m.payload).unwrap_or(Value::Null);
                    let kind = p.get("type").and_then(|v| v.as_str()).unwrap_or("");
                    match (m.ns.as_str(), kind) {
                        (NS_HEART, "PING") => { let _ = stream.write_all(&encode(SENDER, &m.src, NS_HEART, r#"{"type":"PONG"}"#)); }
                        (NS_RECV, "RECEIVER_STATUS") => {
                            let st = &p["status"];
                            let ours = st["applications"].as_array().and_then(|l| l.iter().find(|a| a["appId"].as_str() == Some(sess.app.as_str()))).cloned();
                            if let Some(v) = st["volume"]["level"].as_f64() {
                                events(json!({ "type": "volume", "level": v, "muted": st["volume"]["muted"].as_bool().unwrap_or(false) }));
                            }
                            match ours {
                                Some(a) => {
                                    let t = a["transportId"].as_str().unwrap_or("").to_string();
                                    let first = sess.transport.lock().unwrap().is_empty();
                                    if first && !t.is_empty() {
                                        *sess.transport.lock().unwrap() = t.clone();
                                        *sess.session_id.lock().unwrap() = a["sessionId"].as_str().unwrap_or("").to_string();
                                        let _ = stream.write_all(&encode(SENDER, &t, NS_CONN, r#"{"type":"CONNECT"}"#));
                                        sess.load();
                                        if let Some(tx) = ready.take() { let _ = tx.send(Ok(())); }
                                    }
                                }
                                None if !sess.transport.lock().unwrap().is_empty() => {
                                    why = format!("Something else is playing on {} now.", sess.device.name);
                                    break;
                                }
                                None => {}
                            }
                        }
                        (NS_RECV, "LAUNCH_ERROR") => {
                            if let Some(tx) = ready.take() {
                                let _ = tx.send(Err(format!("{} couldn't open the player ({}).", sess.device.name, p["reason"].as_str().unwrap_or("launch error"))));
                            }
                            break;
                        }
                        (NS_MEDIA, "MEDIA_STATUS") => {
                            if let Some(s) = p["status"].as_array().and_then(|l| l.first()) {
                                if let Some(id) = s["mediaSessionId"].as_i64() {
                                    *sess.media.lock().unwrap() = Some(id);
                                }
                                events(json!({ "type": "media", "state": s["playerState"], "idleReason": s["idleReason"] }));
                            }
                        }
                        (NS_MEDIA, "LOAD_FAILED") | (NS_MEDIA, "LOAD_CANCELLED") | (NS_MEDIA, "INVALID_REQUEST") => {
                            events(json!({ "type": "error", "message": format!("{} couldn't play the stream ({kind}).", sess.device.name) }));
                        }
                        (NS_CONN, "CLOSE") => { why = format!("{} ended the cast.", sess.device.name); break; }
                        _ => {}
                    }
                }
            }
            sess.alive.store(false, Ordering::SeqCst);
            if let Some(tx) = ready.take() {
                let _ = tx.send(Err(if why.is_empty() { "The cast was stopped.".into() } else { why.clone() }));
            }
            let mut cur = SESSION.lock().unwrap();
            if cur.as_ref().is_some_and(|c| Arc::ptr_eq(c, &sess)) {
                *cur = None;
                drop(cur);
                events(json!({ "type": "ended", "message": why }));
            }
        });
    }
    *SESSION.lock().unwrap() = Some(sess.clone());
    match ready_rx.recv_timeout(Duration::from_secs(25)) {
        Ok(Ok(())) => Ok(device),
        Ok(Err(e)) => { stop(); Err(e) }
        Err(_) => { stop(); Err(format!("{} didn't answer.", device.name)) }
    }
}

fn current() -> Option<Arc<Session>> {
    SESSION.lock().unwrap().clone()
}

/// A new track: the stream is loaded again with the new title (from where the track started, so
/// nothing is missed)
pub fn track(meta: Value) {
    mark();
    let Some(s) = current() else { return };
    *s.meta.lock().unwrap() = meta;
    s.load();
}

/// The stream again from now (after the device was paused, its buffer is old)
pub fn reload() {
    mark();
    if let Some(s) = current() {
        s.load();
    }
}

pub fn set_volume(level: f64) {
    if let Some(s) = current() {
        s.send("receiver-0", NS_RECV, json!({ "type": "SET_VOLUME", "requestId": s.next_req(), "volume": { "level": level.clamp(0.0, 1.0) } }));
    }
}

pub fn active() -> Option<Device> {
    current().map(|s| s.device.clone())
}

pub fn stop() {
    let Some(s) = SESSION.lock().unwrap().take() else { return };
    let sid = s.session_id.lock().unwrap().clone();
    if !sid.is_empty() {
        s.send("receiver-0", NS_RECV, json!({ "type": "STOP", "sessionId": sid, "requestId": s.next_req() }));
    }
    // give the message a moment to go before the connection ends
    let alive = s.alive.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(400));
        alive.store(false, Ordering::SeqCst);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn messages_round_trip() {
        let payload = json!({ "type": "LAUNCH", "appId": DEFAULT_APP, "requestId": 1 }).to_string();
        let framed = encode("sender-0", "receiver-0", NS_RECV, &payload);
        let len = u32::from_be_bytes([framed[0], framed[1], framed[2], framed[3]]) as usize;
        assert_eq!(len, framed.len() - 4);
        let m = decode(&framed[4..]).unwrap();
        assert_eq!(m, Msg { src: "sender-0".into(), dst: "receiver-0".into(), ns: NS_RECV.into(), payload });
        // long payloads need multi-byte lengths
        let big = "x".repeat(300);
        let m = decode(&encode("a", "b", NS_MEDIA, &big)[4..]).unwrap();
        assert_eq!(m.payload.len(), 300);
    }

    #[test]
    fn serves_only_the_stream() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let alive = Arc::new(AtomicBool::new(true));
        {
            let alive = alive.clone();
            std::thread::spawn(move || serve_listener(listener, "/live/abc.wav".into(), alive));
        }
        let get = |path: &str| {
            let mut c = TcpStream::connect(("127.0.0.1", port)).unwrap();
            c.set_read_timeout(Some(Duration::from_millis(1500))).unwrap();
            write!(c, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").unwrap();
            let mut got = Vec::new();
            let mut b = [0u8; 4096];
            while let Ok(n) = c.read(&mut b) {
                if n == 0 || got.len() > 8000 { break; }
                got.extend_from_slice(&b[..n]);
            }
            got
        };
        let r = get("/other");
        assert!(String::from_utf8_lossy(&r).starts_with("HTTP/1.1 404"), "got {:?}", String::from_utf8_lossy(&r));
        mark();
        let pusher = std::thread::spawn(|| for _ in 0..20 { push_pcm(&[7u8; 800], 48000); std::thread::sleep(Duration::from_millis(20)); });
        let got = get("/live/abc.wav");
        pusher.join().unwrap();
        alive.store(false, Ordering::SeqCst);
        let text = String::from_utf8_lossy(&got);
        assert!(text.starts_with("HTTP/1.1 200 OK") && text.contains("audio/wav"));
        let body = got.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
        assert_eq!(&got[body..body + 4], b"RIFF");
        assert!(got.len() > body + 44 + 800, "sound followed the header");
    }

    #[test]
    fn stream_starts_at_the_track() {
        push_pcm(&[1u8; 4000], 48000);
        mark();
        push_pcm(&[2u8; 400], 48000);
        let r = LIVE.ring.lock().unwrap();
        let from = (r.mark - r.start) as usize;
        assert!(r.data.iter().skip(from).all(|&b| b == 2));
        assert_eq!(r.data.len() - from, 400);
        assert_eq!(wav_header(48000).len(), 44);
    }
}
