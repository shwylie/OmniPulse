// Now playing widget for OBS: a tiny web page on this PC (127.0.0.1 only). OBS loads it as a
// Browser Source; the page asks /now.json every second for what's playing. It never plays the
// music. It can also show stream alerts over the card (follows, subs, bits...): they come from
// Streamer.bot's WebSocket (the page connects to it itself) or from a web request to /alert (Mix It
// Up's Web Request action, Streamer.bot's Fetch URL, anything), and each kind can have a sound,
// served from /sound/<kind> and played by the page, so OBS can route it. A musical alert's sound is
// played by the app instead (it pauses the music for it): the page says when it shows one (/shown).

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

/// Name That Tune: while a round's on, what's playing isn't shown (the Stream Widget says "Name That Tune")
static TUNE_MASK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub fn tune_mask(on: bool) { TUNE_MASK.store(on, std::sync::atomic::Ordering::Relaxed); }
pub static WIDGET_GEN: AtomicU64 = AtomicU64::new(0);
pub const WIDGET_HTML: &str = include_str!("../widget/widget.html");
/// The lyrics widget (/lyrics): the playing song's timed lyrics, for karaoke
pub const LYRICS_HTML: &str = include_str!("../widget/lyrics.html");
/// The Social Plug (/socials): your handles on other platforms, in turn
const SOCIALS_HTML: &str = include_str!("../widget/socials.html");
/// The Stream Suite's widgets (/goals, /timer, /latest-follow, /latest-sub, /raid): one page, each as its address says
const SUITE_HTML: &str = include_str!("../widget/suite.html");
/// A webcam overlay made in the Suite's Creators (/cam/<id>): its frame and parts, live ones included, as one source
const CAM_HTML: &str = include_str!("../widget/cam.html");
/// Your channel panels (/panels), each drawn at Twitch's size, with Save as PNG
const PANELS_HTML: &str = include_str!("../widget/panels.html");
/// Frames round a game's HUD (/hud), as marked on a screenshot of it
const HUD_HTML: &str = include_str!("../widget/hud.html");
/// The GPU visualiser scenes, for OBS (/vis/<scene>), run in WebGL2
const VIS_HTML: &str = include_str!("../widget/vis.html");

/// The kinds of alert, as /alert?type= names them
/// (custom ones, set up in the app, come on top: see custom_kinds)
pub const ALERT_KINDS: [&str; 13] = ["follow", "sub", "gift", "bits", "kicks", "donation", "raid", "hypetrain", "poll", "ban", "watchstreak", "liked", "game"];
/// The last few alerts, for the page to pick up (it shows the ones newer than it has seen)
static ALERTS: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
static ALERT_ID: AtomicU64 = AtomicU64::new(0);
/// With bots on other devices allowed (widget_start's remote), the key their /alert requests need.
/// Programs on this PC never need it; other devices can only reach /alert, never the widget itself.
static ALERT_KEY: Mutex<String> = Mutex::new(String::new());
const ALERTS_KEPT: usize = 20;

/// Told when the page shows an alert (its kind), for the app to play a musical one's sound itself
pub type ShownHandler = std::sync::Arc<dyn Fn(&str) + Send + Sync>;
static ON_SHOWN: Mutex<Option<ShownHandler>> = Mutex::new(None);
pub fn set_shown_handler(f: ShownHandler) {
    if let Ok(mut h) = ON_SHOWN.lock() {
        *h = Some(f);
    }
}

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
/// The type is one of ALERT_KINDS or a custom alert's trigger name (set up in the app); unknown
/// ones are refused, so a typo shows up in the reply.
/// Something shown on stream (the chatbot's Show on Stream): its text, picture, video, where and how long,
/// sent to the widget with the alerts (its type "show": the widget shows it over them, not as one)
pub fn overlay_push(fields: &serde_json::Map<String, Value>) -> Result<Value, String> {
    let get = |k: &str| fields.get(k).map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let media = |k: &str| Some(get(k)).filter(|u| (u.starts_with("https://") || u.starts_with("http://") || u.starts_with("/overlay/")) && u.len() <= 500).unwrap_or_default();
    let (text, image, video) = (get("text").chars().take(4000).collect::<String>(), media("image"), media("video"));
    if text.trim().is_empty() && image.is_empty() && video.is_empty() { return Err("nothing to show: give it some text, a picture or a video".into()); }
    let place = match get("place").as_str() { "top" => "top", "bottom" => "bottom", "roll" => "roll", _ => "middle" };
    let a = json!({
        "id": ALERT_ID.fetch_add(1, Ordering::SeqCst) + 1,
        "ts": crate::now_secs() * 1000,
        "type": "show",
        "text": text, "image": image, "video": video, "place": place,
        "seconds": fields.get("seconds").and_then(|v| v.as_f64()).unwrap_or(6.0).clamp(1.0, 120.0),
    });
    let mut q = ALERTS.lock().unwrap();
    q.push_back(a.clone());
    while q.len() > ALERTS_KEPT { q.pop_front(); }
    Ok(a)
}

/// A picture or video of yours, for something shown on stream: copied into the widget's own folder (by what's
/// in it, so it's copied once), and its address on the widget's server
pub fn overlay_media(path: &str, video: bool) -> Result<String, String> {
    let p = std::path::Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let ok: &[&str] = if video { &["mp4", "webm"] } else { &["png", "jpg", "jpeg", "gif", "webp"] };
    if !ok.contains(&ext.as_str()) { return Err(format!("that isn't a {} ({})", if video { "video" } else { "picture" }, ok.join(", "))); }
    let bytes = std::fs::read(p).map_err(|e| format!("couldn't read {path}: {e}"))?;
    if bytes.len() > 200 * 1024 * 1024 { return Err("that file's too big (200 MB at most)".into()); }
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    let name = format!("{:016x}.{ext}", h.finish());
    let dir = crate::data_dir()?.join("overlay-media");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let to = dir.join(&name);
    if !to.exists() { std::fs::write(&to, &bytes).map_err(|e| e.to_string())?; }
    Ok(format!("/overlay/{name}"))
}

pub fn alert_push(fields: &serde_json::Map<String, Value>) -> Result<Value, String> {
    let get = |k: &str| fields.get(k).map(|v| v.as_str().map(String::from).unwrap_or_else(|| v.to_string())).unwrap_or_default();
    let kind = get("type").trim().to_lowercase();
    let kind = match kind.as_str() {
        "follower" | "follows" => "follow".to_string(),
        "subscription" | "subscriber" | "resub" => "sub".to_string(),
        "giftsub" | "gifted" | "giftbomb" | "gifts" => "gift".to_string(),
        "cheer" | "bit" => "bits".to_string(),
        "kick" | "kicksgifted" | "kicks_gifted" => "kicks".to_string(),
        "tip" | "donate" => "donation".to_string(),
        "hype" | "hype_train" | "hypetrainstart" | "hypetrainlevelup" | "hypetrainend" => "hypetrain".to_string(),
        "host" | "hosts" | "raids" => "raid".to_string(),
        "banned" => "ban".to_string(),
        "watch_streak" | "streak" => "watchstreak".to_string(),
        "like" | "likedrequest" | "liked_request" => "liked".to_string(),
        "category" | "streamupdate" | "game_change" | "gamechange" => "game".to_string(),
        other => other.to_string(),
    };
    let custom = custom_kinds();
    if !ALERT_KINDS.contains(&kind.as_str()) && !custom.contains(&kind) {
        let mut all: Vec<String> = ALERT_KINDS.iter().map(|k| k.to_string()).collect();
        all.extend(custom);
        return Err(format!("Unknown alert type \"{kind}\". Use one of: {}.", all.join(", ")));
    }
    let a = json!({
        "id": ALERT_ID.fetch_add(1, Ordering::SeqCst) + 1,
        "ts": crate::now_secs() * 1000,
        "type": kind,
        "user": get("user").chars().take(60).collect::<String>(),
        "amount": get("amount").chars().take(30).collect::<String>(),
        "message": get("message").chars().take(200).collect::<String>(),
        "test": fields.get("test").and_then(|v| v.as_bool()).unwrap_or(false),
        // (its own picture, a game's cover say: a web address only; and the platform it came from)
        "image": Some(get("image")).filter(|u| (u.starts_with("https://") || u.starts_with("http://")) && u.len() <= 500).unwrap_or_default(),
        "platform": get("platform").chars().filter(|c| c.is_ascii_alphanumeric()).take(16).collect::<String>().to_lowercase(),
    });
    // (the activity feed's too, to replay: unless it's a test, a replay, or the chatbot's, which it keeps itself)
    if a["test"] != true && fields.get("feed").and_then(|v| v.as_bool()) != Some(false) {
        crate::docks::activity_push(json!({ "type": a["type"], "platform": a["platform"], "user": a["user"], "amount": a["amount"], "message": a["message"], "alert": a }));
    }
    let mut q = ALERTS.lock().unwrap();
    q.push_back(a.clone());
    while q.len() > ALERTS_KEPT {
        q.pop_front();
    }
    Ok(a)
}

/// The custom alerts' trigger names, from the settings the app last sent
fn custom_kinds() -> Vec<String> {
    WIDGET_STATE
        .lock()
        .ok()
        .and_then(|g| g.as_ref().and_then(|s| s["alerts_cfg"]["custom"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())))
        .unwrap_or_default()
}

/// A custom alert's trigger name: lower-case letters, digits, - and _ (it goes in a web address and a file name)
pub fn kind_ok(kind: &str) -> bool {
    !kind.is_empty() && kind.len() <= 32 && kind.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

// ---------- an alert's sound and picture: one file each, in <data>/alert-sounds and alert-images ----------
// (and its collage's pictures, in alert-collage, named <kind>-<n>)
fn media_dir(slot: &str) -> Result<std::path::PathBuf, String> {
    let d = crate::data_dir()?.join(match slot {
        "image" => "alert-images",
        "collage" => "alert-collage",
        _ => "alert-sounds",
    });
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    Ok(d)
}

fn media_file(kind: &str, slot: &str) -> Option<std::path::PathBuf> {
    let d = media_dir(slot).ok()?;
    std::fs::read_dir(d).ok()?.flatten().map(|e| e.path()).find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(kind))
}

const SOUND_EXTS: [&str; 6] = ["mp3", "wav", "ogg", "m4a", "aac", "flac"];
pub const IMAGE_EXTS: [&str; 5] = ["gif", "png", "webp", "jpg", "jpeg"];
/// (an alert's picture can be a video too: played muted, looping)
pub const VIDEO_EXTS: [&str; 2] = ["webm", "mp4"];

/// Copies a sound or a picture (a GIF, say) in for one alert, replacing its old one; returns its file name
pub fn alert_media_set(kind: &str, slot: &str, path: &str) -> Result<String, String> {
    if !kind_ok(kind) {
        return Err("unknown alert type".into());
    }
    let pic = slot == "image" || slot == "collage";
    let exts: &[&str] = if pic { &IMAGE_EXTS } else { &SOUND_EXTS };
    let src = std::path::Path::new(path);
    let ext = src.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).unwrap_or_default();
    if !exts.contains(&ext.as_str()) && !(slot == "image" && VIDEO_EXTS.contains(&ext.as_str())) {
        return Err(format!("Pick {} file ({}).", if pic { "an image or GIF" } else { "a sound" }, exts.join(", ")));
    }
    let size = std::fs::metadata(src).map_err(|e| e.to_string())?.len();
    if size > 20 * 1024 * 1024 {
        return Err("That file is over 20 MB; alerts want something smaller.".into());
    }
    alert_media_clear(kind, slot);
    std::fs::copy(src, media_dir(slot)?.join(format!("{kind}.{ext}"))).map_err(|e| e.to_string())?;
    Ok(src.file_name().and_then(|n| n.to_str()).unwrap_or("file").to_string())
}

pub fn alert_media_clear(kind: &str, slot: &str) {
    if !kind_ok(kind) {
        return;
    }
    if let Some(p) = media_file(kind, slot) {
        let _ = std::fs::remove_file(p);
    }
}

/// An alert's sound or picture as it's kept, for the app to measure (a sound's loudness) or show
pub fn alert_media_bytes(kind: &str, slot: &str) -> Result<Vec<u8>, String> {
    let f = kind_ok(kind).then(|| media_file(kind, slot)).flatten().ok_or("That alert has no file there.")?;
    std::fs::read(f).map_err(|e| e.to_string())
}

fn media_type(p: &std::path::Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).map(|e| e.to_lowercase()).as_deref() {
        Some("wav") => "audio/wav",
        Some("ogg") => "audio/ogg",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("flac") => "audio/flac",
        Some("gif") => "image/gif",
        Some("png") => "image/png",
        Some("webp") => "image/webp",
        Some("jpg") | Some("jpeg") => "image/jpeg",
        Some("mp4") => "video/mp4",
        Some("webm") => "video/webm",
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

/// A reply for this PC's own pages only (the docks): no other site may read it
fn reply_own(stream: &mut std::net::TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!("HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
}

pub fn widget_serve(listener: TcpListener, gen: u64) {
    let _ = listener.set_nonblocking(true);
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    loop {
        if WIDGET_GEN.load(Ordering::SeqCst) != gen {
            break; // switched off or restarted
        }
        match listener.accept() {
            Ok((mut stream, peer)) => {
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut buf = vec![0u8; 8192];
                let n = stream.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let mut first = req.split_whitespace();
                let method = first.next().unwrap_or("GET").to_string();
                let target = first.next().unwrap_or("/").to_string();
                let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
                // another device (a bot elsewhere, through your router): only /alert, and only with the key
                if !peer.ip().is_loopback() {
                    let key = ALERT_KEY.lock().map(|k| k.clone()).unwrap_or_default();
                    let given = query_map(query).get("key").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                    if path != "/alert" || key.is_empty() || given != key {
                        reply(&mut stream, "403 Forbidden", "application/json", json!({ "ok": false, "error": "Wrong or missing key." }).to_string().as_bytes());
                        continue;
                    }
                }
                match path {
                    "/now.json" => {
                        let preview = query.split('&').any(|q| q == "preview=1");
                        let mut v = WIDGET_STATE.lock().ok().and_then(|g| g.clone()).unwrap_or_else(|| json!({ "playing": false }));
                        // (the preview: the settings being edited over it)
                        if preview {
                            if let (Some(o), Some(Value::Object(p))) = (v.as_object_mut(), PREVIEW.lock().ok().and_then(|g| g.clone())) { o.extend(p); }
                        }
                        if let Some(o) = v.as_object_mut() {
                            // (Name That Tune: what's playing kept from the stream while chat guesses it)
                            if !preview && TUNE_MASK.load(std::sync::atomic::Ordering::Relaxed) {
                                o.insert("title".into(), json!("Name That Tune"));
                                o.insert("artist".into(), json!("Guess the song in chat!"));
                                o.insert("artwork".into(), json!(""));
                                o.insert("requested_by".into(), json!(""));
                                // (the lyrics would give it away too)
                                if let Some(Value::Object(l)) = o.get_mut("lyrics") { l.insert("text".into(), json!([])); l.insert("key".into(), json!("tune")); }
                            }
                            o.insert("v".to_string(), Value::String(widget_version()));
                            o.insert("alerts".to_string(), Value::Array(ALERTS.lock().unwrap().iter().cloned().collect()));
                        }
                        reply(&mut stream, "200 OK", "application/json", v.to_string().as_bytes());
                    }
                    // (something shown on stream: a picture or video copied for it, by its name: never a place outside)
                    p if p.starts_with("/overlay/") => {
                        let name = &p[9..];
                        let safe = !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '.') && !name.starts_with('.');
                        let file = crate::data_dir().ok().map(|d| d.join("overlay-media").join(name)).filter(|f| safe && f.is_file());
                        match file.and_then(|f| std::fs::read(&f).ok().map(|b| (b, f))) {
                            Some((b, f)) => reply(&mut stream, "200 OK", media_type(&f), &b),
                            None => reply(&mut stream, "404 Not Found", "text/plain", b"Not found."),
                        }
                    }
                    // (OBS's docks: the chat, with its moderation; the activity feed, its alerts replayed)
                    "/dock/chat" | "/dock/mod" => reply_own(&mut stream, "200 OK", "text/html; charset=utf-8", crate::docks::CHAT_HTML.as_bytes()),
                    p if p == "/chat" || p.strip_prefix("/chat/").is_some_and(|n| !n.is_empty() && n.len() < 4 && n.bytes().all(|b| b.is_ascii_digit())) => reply_own(&mut stream, "200 OK", "text/html; charset=utf-8", crate::docks::MULTICHAT_HTML.as_bytes()),
                    "/dock/activity" => reply_own(&mut stream, "200 OK", "text/html; charset=utf-8", crate::docks::ACTIVITY_HTML.as_bytes()),
                    "/dock/chat.json" | "/dock/activity.json" => {
                        let after = query_map(query).get("after").and_then(|v| v.as_str().and_then(|a| a.parse().ok())).unwrap_or(0);
                        let v = if path == "/dock/chat.json" { crate::docks::chat_json(after) } else { crate::docks::activity_json(after) };
                        reply_own(&mut stream, "200 OK", "application/json", v.to_string().as_bytes());
                    }
                    "/dock/emotes.json" => reply_own(&mut stream, "200 OK", "application/json", crate::emotes::json().to_string().as_bytes()),
                    "/dock/settings.json" => {
                        let page = query_map(query).get("page").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                        reply_own(&mut stream, "200 OK", "application/json", crate::docks::page_settings(&page).to_string().as_bytes());
                    }
                    "/dock/do" => {
                        let body = req.split_once("\r\n\r\n").map(|x| x.1).unwrap_or("");
                        let answer = if method != "POST" || !crate::docks::trusted(&req, port) { Err("only OmniPulse's own docks can".to_string()) } else {
                            serde_json::from_str::<Value>(body).map_err(|_| "that isn't JSON".to_string()).and_then(crate::docks::dock_do)
                        };
                        let (status, v) = match answer { Ok(m) => ("200 OK", json!({ "ok": true, "message": m })), Err(e) => ("400 Bad Request", json!({ "ok": false, "error": e })) };
                        reply_own(&mut stream, status, "application/json", v.to_string().as_bytes());
                    }
                    "/hud" => reply(&mut stream, "200 OK", "text/html; charset=utf-8", HUD_HTML.as_bytes()),
                    "/panels" => reply(&mut stream, "200 OK", "text/html; charset=utf-8", PANELS_HTML.as_bytes()),
                    p if p == "/cam" || p.starts_with("/cam/") => reply(&mut stream, "200 OK", "text/html; charset=utf-8", CAM_HTML.as_bytes()),
                    "/socials" | "/socials.html" | "/sponsors" => reply(&mut stream, "200 OK", "text/html; charset=utf-8", SOCIALS_HTML.as_bytes()),
                    p if p.starts_with("/goals/") && p.len() < 24 => reply(&mut stream, "200 OK", "text/html; charset=utf-8", SUITE_HTML.as_bytes()),
                    "/goals" | "/timer" | "/latest-follow" | "/latest-sub" | "/raid" | "/hype" | "/voteskip" | "/wheel" | "/leaderboard" | "/emotes" | "/tune" | "/credits" => reply(&mut stream, "200 OK", "text/html; charset=utf-8", SUITE_HTML.as_bytes()),
                    // (the GPU visualiser for OBS: its page, its scenes, the sound's latest frame (asking for it
                    // keeps the app reading the sound), and the cover)
                    p if p == "/vis" || p.strip_prefix("/vis/").is_some_and(|n| !n.is_empty() && n.len() < 20 && n.bytes().all(|b| b.is_ascii_lowercase())) => {
                        reply(&mut stream, "200 OK", "text/html; charset=utf-8", VIS_HTML.as_bytes());
                    }
                    "/vis/scenes.json" => {
                        let s = VIS.lock().map(|v| v.scenes.clone()).unwrap_or_default();
                        reply(&mut stream, "200 OK", "application/json", if s.is_empty() { b"{\"scenes\":{}}".to_vec() } else { s.into_bytes() }.as_slice());
                    }
                    "/vis/frame.json" => {
                        let body = VIS.lock().map(|mut v| {
                            v.asked = Some(std::time::Instant::now());
                            format!(r#"{{"playing":{},"offset":{},"art":{},"frame":{}}}"#, v.playing, v.offset, v.art_n, if v.frame.is_empty() { "null" } else { &v.frame })
                        }).unwrap_or_else(|_| "{}".into());
                        reply(&mut stream, "200 OK", "application/json", body.as_bytes());
                    }
                    "/vis/art.png" => {
                        let png = VIS.lock().map(|v| v.art.clone()).unwrap_or_default();
                        if png.is_empty() { reply(&mut stream, "404 Not Found", "text/plain", b"No cover yet") } else { reply(&mut stream, "200 OK", "image/png", &png) }
                    }
                    "/levels.json" => {
                        let v = LEVELS.lock().map(|g| g.clone()).unwrap_or_default();
                        reply_own(&mut stream, "200 OK", "application/json", serde_json::to_string(&v).unwrap_or_else(|_| "[]".into()).as_bytes());
                    }
                    p if p == "/lyrics" || p == "/lyrics.html" || p.strip_prefix("/lyrics/").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) => {
                        reply(&mut stream, "200 OK", "text/html; charset=utf-8", LYRICS_HTML.as_bytes());
                    }
                    // (the widget; its other outputs (/2, /3...), each in a look of its own; and the alerts alone: one
                    // page, each as its address says)
                    p if p == "/" || p == "/index.html" || p == "/alerts" || (p.len() > 1 && p.len() < 5 && p[1..].bytes().all(|b| b.is_ascii_digit())) => {
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
                    // the page is showing an alert: the app plays a musical one's sound
                    "/shown" => {
                        let kind = query_map(query).get("type").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                        if kind_ok(&kind) {
                            if let Some(h) = ON_SHOWN.lock().ok().and_then(|h| h.clone()) {
                                h(&kind);
                            }
                        }
                        reply(&mut stream, "204 No Content", "text/plain", b"");
                    }
                    // an alert's sound, picture or collage picture
                    p if p.starts_with("/sound/") || p.starts_with("/image/") || p.starts_with("/collage/") => {
                        let (slot, kind) = p[1..].split_once('/').unwrap_or(("", ""));
                        match kind_ok(kind).then(|| media_file(kind, slot)).flatten().and_then(|f| std::fs::read(&f).ok().map(|b| (f, b))) {
                            Some((f, bytes)) => reply(&mut stream, "200 OK", media_type(&f), &bytes),
                            None => reply(&mut stream, "404 Not Found", "text/plain", b"Nothing there"),
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

/// The settings page's preview (the app's pop-out): what's on the page so far, unsaved, over the widget's
/// state, for a page asking with ?preview=1. None: no page open, the state as it is
static PREVIEW: std::sync::Mutex<Option<Value>> = std::sync::Mutex::new(None);

pub fn preview_set(v: Option<Value>) {
    if let Ok(mut p) = PREVIEW.lock() { *p = v; }
}

pub fn preview_on() -> bool {
    PREVIEW.lock().map(|p| p.is_some()).unwrap_or(false)
}

/// remote: bots on other devices may send alerts too (they need key); otherwise only this PC is heard
pub fn widget_start(port: u16, remote: bool, key: &str) -> Result<String, String> {
    // Stops any earlier server first
    let gen = WIDGET_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    if let Ok(mut k) = ALERT_KEY.lock() {
        *k = if remote { key.to_string() } else { String::new() };
    }
    let host = if remote { "0.0.0.0" } else { "127.0.0.1" };
    let mut last_err = String::new();
    for _ in 0..15 {
        match TcpListener::bind((host, port)) {
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

/// The music's levels, 0 to 1, low to high (the app's, while the Equaliser design is in use): /levels.json
pub static LEVELS: Mutex<Vec<f32>> = Mutex::new(Vec::new());

pub fn set_levels(v: Vec<f32>) {
    if let Ok(mut g) = LEVELS.lock() { *g = v; }
}

/// The GPU visualiser for OBS (/vis): the scenes (from the app, as JSON), the sound's latest frame, whether
/// something's playing, how far behind viewers hear it (the OmniPulse Offset's delay, ms), the cover (PNG, and
/// a count that's new with each), and when the page last asked for a frame
#[derive(Default)]
pub struct Vis { pub scenes: String, pub frame: String, pub playing: bool, pub offset: i64, pub art: Vec<u8>, pub art_n: u64, pub asked: Option<std::time::Instant> }

pub static VIS: Mutex<Vis> = Mutex::new(Vis { scenes: String::new(), frame: String::new(), playing: false, offset: 0, art: Vec::new(), art_n: 0, asked: None });

/// Whether OBS has the visualiser up: a frame asked for in the last few seconds
pub fn vis_wanted() -> bool { VIS.lock().ok().and_then(|v| v.asked).is_some_and(|t| t.elapsed().as_secs() < 4) }

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
        assert_eq!(alert_push(&query_map("type=host&user=x&amount=12")).unwrap()["type"], "raid");
        // a custom alert works once the app has said it exists
        assert!(alert_push(&query_map("type=hydrate")).is_err());
        widget_update(json!({ "alerts_cfg": { "custom": ["hydrate"] } }));
        assert_eq!(alert_push(&query_map("type=Hydrate&user=Me")).unwrap()["type"], "hydrate");
        assert!(kind_ok("hydrate_2") && !kind_ok("../x") && !kind_ok("Ab"));
    }
}

#[cfg(test)]
mod overlay_tests {
    use super::*;
    #[test]
    fn what_is_shown_on_stream_goes_with_the_alerts() {
        let m = |v: Value| v.as_object().unwrap().clone();
        assert!(overlay_push(&m(json!({ "text": " " }))).is_err());
        let a = overlay_push(&m(json!({ "text": "Hi", "image": "file:///C:/x.png", "video": "/overlay/ab.mp4", "place": "top", "seconds": 500 }))).unwrap();
        assert_eq!((a["type"].as_str(), a["image"].as_str(), a["video"].as_str(), a["place"].as_str(), a["seconds"].as_f64()), (Some("show"), Some(""), Some("/overlay/ab.mp4"), Some("top"), Some(120.0)));
        assert!(ALERTS.lock().unwrap().iter().any(|x| x["id"] == a["id"]));
        assert!(overlay_media("C:/Windows/notepad.exe", false).is_err());
    }
}
