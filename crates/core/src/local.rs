// Local music: your own files in folders you choose.
// Tags are read once and kept in local_index.json; later scans only re-read files that changed.
// Files are played through a small server on this PC only (127.0.0.1), and only indexed files are served.

use crate::data_dir;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Serialize, Deserialize, Clone)]
pub struct LocalEntry {
    pub id: String,
    pub path: String,
    pub size: u64,
    pub mtime: u64,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub duration: u64,
}

pub static LOCAL: Mutex<Vec<LocalEntry>> = Mutex::new(Vec::new());
pub static LOCAL_SCANNING: AtomicBool = AtomicBool::new(false);
pub static LOCAL_LOADED: AtomicBool = AtomicBool::new(false);
pub static LOCAL_SERVER: OnceLock<(u16, String)> = OnceLock::new();

pub const AUDIO_EXTS: [&str; 10] = ["mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav", "aif", "aiff"];

pub fn is_audio(p: &Path) -> bool {
    p.extension()
        .and_then(|e| e.to_str())
        .map(|e| AUDIO_EXTS.contains(&e.to_lowercase().as_str()))
        .unwrap_or(false)
}

pub fn mime_for(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .as_deref()
    {
        Some("mp3") => "audio/mpeg",
        Some("flac") => "audio/flac",
        Some("m4a") | Some("aac") => "audio/mp4",
        Some("ogg") | Some("oga") | Some("opus") => "audio/ogg",
        Some("wav") => "audio/wav",
        Some("aif") | Some("aiff") => "audio/aiff",
        _ => "application/octet-stream",
    }
}

pub fn path_id(p: &str) -> String {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in p.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    format!("{h:016x}")
}

pub fn local_folders_load() -> Vec<String> {
    data_dir()
        .ok()
        .and_then(|d| fs::read_to_string(d.join("local.json")).ok())
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| v["folders"].as_array().cloned())
        .map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect())
        .unwrap_or_default()
}

pub fn local_index_file() -> Option<PathBuf> {
    data_dir().ok().map(|d| d.join("local_index.json"))
}

pub fn local_load_index() {
    if LOCAL_LOADED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Some(p) = local_index_file() {
        if let Ok(text) = fs::read_to_string(p) {
            if let Ok(list) = serde_json::from_str::<Vec<LocalEntry>>(&text) {
                if let Ok(mut g) = LOCAL.lock() {
                    *g = list;
                }
            }
        }
    }
}

pub fn read_tags(path: &Path) -> (String, String, String, u64) {
    use lofty::prelude::*;
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Untitled")
        .to_string();
    if let Ok(tagged) = lofty::read_from_path(path) {
        let dur = tagged.properties().duration().as_millis() as u64;
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            let title = tag
                .title()
                .map(|c| c.to_string())
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| stem.clone());
            let artist = tag.artist().map(|c| c.to_string()).unwrap_or_default();
            let album = tag.album().map(|c| c.to_string()).unwrap_or_default();
            return (title, artist, album, dur);
        }
        return (stem, String::new(), String::new(), dur);
    }
    (stem, String::new(), String::new(), 0)
}

pub fn local_scan_blocking() {
    local_load_index();
    let old: HashMap<String, LocalEntry> = LOCAL
        .lock()
        .map(|g| g.iter().map(|e| (e.path.clone(), e.clone())).collect())
        .unwrap_or_default();
    let mut out: Vec<LocalEntry> = Vec::new();
    for root in local_folders_load() {
        let mut stack = vec![PathBuf::from(&root)];
        while let Some(dir) = stack.pop() {
            let Ok(rd) = fs::read_dir(&dir) else { continue };
            for entry in rd.flatten() {
                let p = entry.path();
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                if p.is_dir() {
                    stack.push(p);
                    continue;
                }
                if !is_audio(&p) {
                    continue;
                }
                let Ok(meta) = entry.metadata() else { continue };
                let size = meta.len();
                let mtime = meta
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map(|d| d.as_secs())
                    .unwrap_or(0);
                let path = p.to_string_lossy().to_string();
                if let Some(o) = old.get(&path) {
                    if o.size == size && o.mtime == mtime {
                        out.push(o.clone());
                        continue;
                    }
                }
                let (title, artist, album, duration) = read_tags(&p);
                out.push(LocalEntry { id: path_id(&path), path, size, mtime, title, artist, album, duration });
            }
        }
    }
    if let Ok(mut g) = LOCAL.lock() {
        *g = out.clone();
    }
    if let Some(f) = local_index_file() {
        if let Ok(text) = serde_json::to_string(&out) {
            let _ = fs::write(f, text);
        }
    }
    LOCAL_SCANNING.store(false, Ordering::SeqCst);
}

// The indexed file with this id (its path, size and change time)
pub fn local_entry(id: &str) -> Option<LocalEntry> {
    local_load_index();
    LOCAL.lock().ok()?.iter().find(|e| e.id == id).cloned()
}

pub fn local_path_for(id: &str) -> Option<PathBuf> {
    LOCAL
        .lock()
        .ok()
        .and_then(|g| g.iter().find(|e| e.id == id).map(|e| PathBuf::from(&e.path)))
}

pub fn query_param(url: &str, key: &str) -> Option<String> {
    let q = url.split('?').nth(1)?;
    for pair in q.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return urlencoding::decode(v).ok().map(|c| c.into_owned());
            }
        }
    }
    None
}

pub fn add_header<R: Read>(resp: &mut tiny_http::Response<R>, k: &str, v: &str) {
    if let Ok(h) = tiny_http::Header::from_bytes(k.as_bytes(), v.as_bytes()) {
        resp.add_header(h);
    }
}

pub fn serve_file(request: tiny_http::Request, path: &Path) {
    use std::io::{Seek, SeekFrom};
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => {
            let _ = request.respond(tiny_http::Response::empty(404));
            return;
        }
    };
    let total = file.metadata().map(|m| m.len()).unwrap_or(0);
    if total == 0 {
        let _ = request.respond(tiny_http::Response::empty(404));
        return;
    }
    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .map(|h| h.value.as_str().to_string());
    let last = total - 1;
    let (start, end, partial) = match range.as_deref().and_then(|r| r.strip_prefix("bytes=")) {
        Some(spec) => {
            let mut it = spec.splitn(2, '-');
            let s = it.next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(0);
            let e = it.next().and_then(|x| x.parse::<u64>().ok()).unwrap_or(last);
            (s, e.min(last), true)
        }
        None => (0, last, false),
    };
    if start > end {
        let _ = request.respond(tiny_http::Response::empty(416));
        return;
    }
    let len = end - start + 1;
    if file.seek(SeekFrom::Start(start)).is_err() {
        let _ = request.respond(tiny_http::Response::empty(500));
        return;
    }
    let status: u16 = if partial { 206 } else { 200 };
    let mut resp = tiny_http::Response::new(
        tiny_http::StatusCode(status),
        Vec::new(),
        file.take(len),
        Some(len as usize),
        None,
    );
    add_header(&mut resp, "Content-Type", mime_for(path));
    add_header(&mut resp, "Accept-Ranges", "bytes");
    add_header(&mut resp, "Access-Control-Allow-Origin", "*");
    if partial {
        add_header(&mut resp, "Content-Range", &format!("bytes {start}-{end}/{total}"));
    }
    let _ = request.respond(resp);
}

// Cover art: the picture inside the file, or a cover/folder image beside it
pub fn serve_cover(request: tiny_http::Request, path: &Path) {
    use lofty::prelude::*;
    let mut data: Option<(Vec<u8>, String)> = None;
    if let Ok(tagged) = lofty::read_from_path(path) {
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            if let Some(pic) = tag.pictures().first() {
                let mime = pic
                    .mime_type()
                    .map(|m| m.as_str().to_string())
                    .unwrap_or_else(|| "image/jpeg".to_string());
                data = Some((pic.data().to_vec(), mime));
            }
        }
    }
    if data.is_none() {
        if let Some(dir) = path.parent() {
            for name in ["cover.jpg", "folder.jpg", "front.jpg", "cover.png", "folder.png"] {
                if let Ok(bytes) = fs::read(dir.join(name)) {
                    let mime = if name.ends_with("png") { "image/png" } else { "image/jpeg" };
                    data = Some((bytes, mime.to_string()));
                    break;
                }
            }
        }
    }
    match data {
        Some((bytes, mime)) => {
            let mut resp = tiny_http::Response::from_data(bytes);
            add_header(&mut resp, "Content-Type", &mime);
            add_header(&mut resp, "Cache-Control", "max-age=86400");
            add_header(&mut resp, "Access-Control-Allow-Origin", "*");
            let _ = request.respond(resp);
        }
        None => {
            let _ = request.respond(tiny_http::Response::empty(404));
        }
    }
}

pub fn handle_local_request(request: tiny_http::Request, token: &str) {
    let url = request.url().to_string();
    let route = url.split('?').next().unwrap_or("").to_string();
    if query_param(&url, "k").as_deref() != Some(token) {
        let _ = request.respond(tiny_http::Response::empty(403));
        return;
    }
    let id = query_param(&url, "id").unwrap_or_default();
    if route == "/yt" {
        serve_youtube(request, &id);
        return;
    }
    let Some(path) = local_path_for(&id) else {
        let _ = request.respond(tiny_http::Response::empty(404));
        return;
    };
    match route.as_str() {
        "/file" => serve_file(request, &path),
        "/cover" => serve_cover(request, &path),
        _ => {
            let _ = request.respond(tiny_http::Response::empty(404));
        }
    }
}

pub fn start_local_server() -> (u16, String) {
    let mut tb = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut tb);
    let token = URL_SAFE_NO_PAD.encode(tb);
    let server = tiny_http::Server::http("127.0.0.1:0").expect("could not start the local music server");
    let port = server.server_addr().to_ip().map(|a| a.port()).unwrap_or(0);
    let tok = token.clone();
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            let t = tok.clone();
            std::thread::spawn(move || handle_local_request(request, &t));
        }
    });
    (port, token)
}

pub fn local_folders() -> Vec<String> {
    local_folders_load()
}

pub fn local_set_folders(folders: Vec<String>) -> Result<Vec<String>, String> {
    let mut clean: Vec<String> = Vec::new();
    for f in folders {
        let p = f.trim().trim_matches('"').to_string();
        if p.is_empty() || clean.contains(&p) {
            continue;
        }
        match fs::metadata(&p) {
            Ok(m) if m.is_dir() => clean.push(p),
            _ => return Err(format!("Cannot open that folder: {p}")),
        }
    }
    let file = data_dir()?.join("local.json");
    fs::write(file, serde_json::json!({ "folders": clean }).to_string()).map_err(|e| e.to_string())?;
    Ok(clean)
}

pub fn local_scan() {
    if LOCAL_SCANNING.swap(true, Ordering::SeqCst) {
        return; // already scanning
    }
    std::thread::spawn(local_scan_blocking);
}

pub fn local_status() -> Value {
    let count = LOCAL.lock().map(|g| g.len()).unwrap_or(0);
    serde_json::json!({ "scanning": LOCAL_SCANNING.load(Ordering::SeqCst), "count": count })
}

pub fn local_tracks() -> Value {
    local_load_index();
    let folders = local_folders_load();
    let list: Vec<Value> = LOCAL
        .lock()
        .map(|g| {
            g.iter()
                .filter(|e| folders.iter().any(|f| e.path.starts_with(f.as_str())))
                .map(|e| {
                    serde_json::json!({
                        "id": e.id, "title": e.title, "artist": e.artist,
                        "album": e.album, "duration": e.duration
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    Value::Array(list)
}

pub fn local_server() -> Value {
    let info = LOCAL_SERVER.get_or_init(start_local_server);
    serde_json::json!({ "port": info.0, "token": info.1 })
}

// ---------- YouTube audio, passed through ----------
// The player can't load YouTube's stream URLs itself (no CORS headers, and an open-ended request
// gets throttled), so they're fetched here in pieces of YT_CHUNK bytes, the way YouTube's own
// player asks for them. A URL that has expired is replaced once with a fresh one from yt-dlp.
const YT_CHUNK: u64 = 4 * 1024 * 1024;

fn serve_youtube(request: tiny_http::Request, video_id: &str) {
    let range = request
        .headers()
        .iter()
        .find(|h| h.field.equiv("Range"))
        .and_then(|h| h.value.as_str().strip_prefix("bytes=").map(String::from));
    let (start, want_end) = match range.as_deref().map(|r| r.splitn(2, '-').collect::<Vec<_>>()) {
        Some(p) => (p[0].parse::<u64>().unwrap_or(0), p.get(1).and_then(|e| e.parse::<u64>().ok())),
        None => (0, None),
    };
    let end = want_end.unwrap_or(u64::MAX).min(start + YT_CHUNK - 1);
    let Ok(rt) = tokio::runtime::Builder::new_current_thread().enable_all().build() else {
        let _ = request.respond(tiny_http::Response::empty(500));
        return;
    };
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .unwrap_or_default();
    for fresh in [false, true] {
        let url = match rt.block_on(crate::youtube::stream_url(video_id, fresh)) {
            Ok(u) => u,
            Err(e) => {
                let _ = request.respond(tiny_http::Response::from_string(e).with_status_code(502));
                return;
            }
        };
        let res = match client.get(&url).header("Range", format!("bytes={start}-{end}")).send() {
            Ok(r) => r,
            Err(e) => {
                let _ = request.respond(tiny_http::Response::from_string(e.to_string()).with_status_code(502));
                return;
            }
        };
        let status = res.status().as_u16();
        if (status == 403 || status == 410) && !fresh {
            continue; // the URL expired or was refused: once more with a new one
        }
        if status == 416 {
            let _ = request.respond(tiny_http::Response::empty(416));
            return;
        }
        if status != 200 && status != 206 {
            let _ = request.respond(tiny_http::Response::empty(502));
            return;
        }
        let header = |k: &str| res.headers().get(k).and_then(|v| v.to_str().ok()).map(String::from);
        let ctype = header("content-type").unwrap_or_else(|| "audio/mp4".into());
        let len = header("content-length").and_then(|l| l.parse::<usize>().ok());
        let total = header("content-range").and_then(|r| r.rsplit('/').next().map(String::from));
        let got_end = len.map(|l| start + l as u64 - 1).unwrap_or(end);
        let mut resp = tiny_http::Response::new(tiny_http::StatusCode(206), Vec::new(), res, len, None);
        add_header(&mut resp, "Content-Type", &ctype);
        add_header(&mut resp, "Accept-Ranges", "bytes");
        add_header(&mut resp, "Access-Control-Allow-Origin", "*");
        add_header(&mut resp, "Content-Range", &format!("bytes {start}-{got_end}/{}", total.as_deref().unwrap_or("*")));
        let _ = request.respond(resp);
        return;
    }
    let _ = request.respond(tiny_http::Response::empty(502));
}
