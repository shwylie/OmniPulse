// Turning a track into sound for the engine: SoundCloud's streams, Spotify through librespot,
// YouTube Music through yt-dlp, your files. Also the cover and the lyrics for what's playing.

use crate::track::{Src, Track};
use omnipulse_audio::{Decoded, HlsSource, PushFeed, PushSource, Source};
use omnipulse_core as core;
use serde_json::Value;
use slint::{Rgba8Pixel, SharedPixelBuffer};
use std::sync::{Arc, Mutex};

// ---------- Spotify: librespot's sound into the engine ----------
static SP_FEED: Mutex<Option<(PushFeed, u32)>> = Mutex::new(None);

struct FeedOut;
impl core::spotify_engine::PcmOut for FeedOut {
    fn send(&self, gen: u32, samples: &[f32]) -> bool {
        SP_FEED.lock().unwrap().as_ref().is_some_and(|(f, _)| f.send(gen, samples))
    }
}

fn spotify_attach() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        core::spotify_engine::spe_attach(Arc::new(FeedOut), Arc::new(|ev: Value| {
            if ev["type"] == "end" {
                if let Some((f, gen)) = SP_FEED.lock().unwrap().as_ref() {
                    f.end(*gen);
                }
            }
        }));
    });
}

/// Why an age-restricted YouTube song with no other upload doesn't play (main.rs skips it, saying so)
pub const AGE_BLOCKED: &str = "YouTube only plays it to signed-in adults, and there's no other upload of it";

/// YouTube's answer for an age-restricted video (yt-dlp can't sign in as an adult)
fn age_blocked(e: &str) -> bool {
    let e = e.to_lowercase();
    e.contains("confirm your age") || e.contains("age-restrict") || e.contains("age restrict") || e.contains("inappropriate for some users")
}

/// The audio of another upload of the same song (up to three close ones tried), as the web app's
/// ytPlayAnyUpload
async fn other_upload(t: &Track, id: &str) -> Option<(String, String)> {
    let found = core::youtube::search(&format!("{} {}", t.artist, t.title)).await.ok()?;
    for c in found.iter().filter(|c| c.id != id && crate::versions::score(t, c) >= 0.8).take(3) {
        match core::youtube::stream_url(&c.id, false).await {
            Ok(url) => return Some((c.id.clone(), url)),
            Err(e) if age_blocked(&e) => continue,
            Err(_) => return None,
        }
    }
    None
}

pub async fn open_source(t: &Track, rt: tokio::runtime::Handle) -> Result<Box<dyn Source>, String> {
    match &t.src {
        // (played as its YouTube Music version: crate::versions, in main.rs)
        Src::Catalog(_) => Err("it plays from YouTube Music".into()),
        Src::Plugin(id, x) => {
            let p = crate::plugrun::get(id).ok_or("its plugin isn't running (Settings > Integrations)")?;
            let r = p.call("resolve", vec![serde_json::from_str(x).unwrap_or_default()]).await?;
            let url = r["url"].as_str().unwrap_or("").to_string();
            if !(url.starts_with("https://") || url.starts_with("http://")) {
                return Err(format!("{} didn't give a playable address", p.name));
            }
            if !core::plugins::host_allowed(&p.hosts, &url) {
                return Err(format!("{} may only play from {}", p.name, p.hosts.join(", ")));
            }
            let ext = url.split(['?', '#']).next().unwrap_or("").rsplit('.').next().unwrap_or("mp3").to_lowercase();
            let ext = if ["mp3", "flac", "m4a", "ogg", "wav", "aac", "opus"].contains(&ext.as_str()) { ext } else { "mp3".to_string() };
            tokio::task::spawn_blocking(move || omnipulse_audio::open_url(&url, &ext).map(|d| Box::new(d) as Box<dyn Source>)).await.map_err(|e| e.to_string())?
        }
        Src::Local(path) => {
            let p = path.clone();
            tokio::task::spawn_blocking(move || Decoded::open_file(std::path::Path::new(&p)).map(|d| Box::new(d) as Box<dyn Source>)).await.map_err(|e| e.to_string())?
        }
        // not signed in to SoundCloud (a friend in a Jam, say): the track's own audio through yt-dlp
        Src::SoundCloud(_) if !core::soundcloud::signed_in() && core::tools::ytdlp_path().is_some() && t.link.starts_with("https://soundcloud.com/") => {
            let url = core::yt_stream::resolve_soundcloud(t.link.split(['?', '#']).next().unwrap_or("")).await?;
            let hls = url.contains(".m3u8");
            tokio::task::spawn_blocking(move || -> Result<Box<dyn Source>, String> {
                if hls { Ok(Box::new(HlsSource::open(&url)?)) } else { Ok(Box::new(omnipulse_audio::open_url(&url, "mp3")?)) }
            })
            .await
            .map_err(|e| e.to_string())?
        }
        Src::SoundCloud(id) => {
            let v = core::soundcloud::stream_url(id.to_string()).await?;
            let mut why = String::from("SoundCloud sent no stream for it");
            for c in v["candidates"].as_array().cloned().unwrap_or_default() {
                let name = c["name"].as_str().unwrap_or("").to_string();
                if name.contains("preview") || name.contains("opus") {
                    continue;
                }
                let mut url = c["url"].as_str().unwrap_or("").to_string();
                if url.starts_with("https://api.soundcloud.com") {
                    url = core::soundcloud::resolve_api_url(&url).await.unwrap_or(url);
                }
                let hls = name.starts_with("hls");
                let r = tokio::task::spawn_blocking(move || -> Result<Box<dyn Source>, String> {
                    if hls { Ok(Box::new(HlsSource::open(&url)?)) } else { Ok(Box::new(omnipulse_audio::open_url(&url, "mp3")?)) }
                })
                .await
                .map_err(|e| e.to_string())?;
                match r {
                    Ok(s) => return Ok(s),
                    Err(e) => why = e,
                }
            }
            Err(why)
        }
        Src::YouTube(id) => {
            let (vid, url) = match core::youtube::stream_url(id, false).await {
                Ok(url) => (id.clone(), url),
                // age-restricted: another upload of the same song often isn't (the closest few)
                Err(e) if age_blocked(&e) => other_upload(t, id).await.ok_or_else(|| AGE_BLOCKED.to_string())?,
                Err(e) => return Err(e),
            };
            // (an address YouTube stops taking partway, a cached one or one cut off: a fresh one, on from there)
            let refresh: omnipulse_audio::http::Refresh = Box::new(move || rt.block_on(core::youtube::stream_url(&vid, true)));
            tokio::task::spawn_blocking(move || omnipulse_audio::open_url_refreshing(&url, "m4a", refresh).map(|d| Box::new(d) as Box<dyn Source>)).await.map_err(|e| e.to_string())?
        }
        Src::Spotify(uri) => {
            spotify_attach();
            let gen = core::spotify_engine::spe_play(uri.clone(), 0).await?;
            let (src, feed) = PushSource::new(
                44100,
                Some(t.ms),
                Box::new(move |ms| {
                    let g = rt.block_on(core::spotify_engine::spe_seek(ms as u32))?;
                    if let Some(x) = SP_FEED.lock().unwrap().as_mut() {
                        x.1 = g;
                    }
                    Ok(g)
                }),
                Box::new(core::spotify_engine::spe_played),
            );
            *SP_FEED.lock().unwrap() = Some((feed, gen));
            Ok(Box::new(src))
        }
    }
}

/// A cover, decoded off the UI thread, at most `size` pixels across
// ---------- covers: one connection pool for all of them, and kept on disk (as a browser's cache) ----------
/// The covers' folder (data folder\cache\art) holds at most this; past it, the longest unused go
const ART_DISK_MAX: u64 = 300 * 1024 * 1024;
/// a cover bigger than this isn't kept
const ART_FILE_MAX: usize = 2 * 1024 * 1024;

/// One client for every cover: its connections kept open and reused (HTTP/2 where the host has it),
/// not a new secure connection for each picture
fn art_client() -> &'static reqwest::Client {
    static C: std::sync::OnceLock<reqwest::Client> = std::sync::OnceLock::new();
    C.get_or_init(|| reqwest::Client::builder().pool_max_idle_per_host(16).timeout(std::time::Duration::from_secs(20)).build().unwrap_or_default())
}

fn art_dir() -> Option<std::path::PathBuf> {
    core::data_dir().ok().map(|d| d.join("cache").join("art"))
}

fn art_file(url: &str) -> Option<std::path::PathBuf> {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut h);
    art_dir().map(|d| d.join(format!("{:016x}", h.finish())))
}

/// The covers' folder kept to its size: the longest unused first (once a run, in the background)
fn art_trim() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        std::thread::spawn(|| {
            let Some(dir) = art_dir() else { return };
            let Ok(rd) = std::fs::read_dir(&dir) else { return };
            let mut files: Vec<(std::time::SystemTime, u64, std::path::PathBuf)> = rd.flatten().filter_map(|e| {
                let m = e.metadata().ok()?;
                Some((m.accessed().or_else(|_| m.modified()).ok()?, m.len(), e.path()))
            }).collect();
            let mut total: u64 = files.iter().map(|f| f.1).sum();
            if total <= ART_DISK_MAX { return; }
            files.sort_by_key(|f| f.0);
            for (_, len, p) in files {
                if total <= ART_DISK_MAX * 2 / 3 { break; }
                if std::fs::remove_file(&p).is_ok() { total = total.saturating_sub(len); }
            }
        });
    });
}

/// A cover's bytes: from the disk if it's been fetched before, else fetched (and kept)
async fn art_bytes(url: &str) -> Option<Vec<u8>> {
    art_trim();
    let file = art_file(url);
    if let Some(f) = &file {
        if let Ok(b) = tokio::fs::read(f).await {
            if !b.is_empty() { return Some(b); }
        }
    }
    let bytes = art_client().get(url).send().await.ok()?.error_for_status().ok()?.bytes().await.ok()?.to_vec();
    if let Some(f) = file {
        if bytes.len() <= ART_FILE_MAX {
            if let Some(d) = f.parent() { let _ = tokio::fs::create_dir_all(d).await; }
            let _ = tokio::fs::write(&f, &bytes).await;
        }
    }
    Some(bytes)
}

pub async fn fetch_art(url: String, size: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    if url.is_empty() || std::env::var_os("OMNIPULSE_MEM_NO_ART").is_some() {
        return None;
    }
    let bytes = art_bytes(&url).await?;
    // (decoded off the async threads: they're for waiting on the network)
    tokio::task::spawn_blocking(move || {
        let img = image::load_from_memory(&bytes).ok()?.thumbnail(size, size).to_rgba8();
        Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
    }).await.ok().flatten()
}

/// A picture as a circle (an artist's): cropped square from its middle, the corners clear, the edge
/// smoothed. (The renderer can't clip to a round shape itself.)
pub async fn fetch_round(url: String, size: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    if url.is_empty() {
        return None;
    }
    let bytes = art_bytes(&url).await?;
    tokio::task::spawn_blocking(move || round_of(&bytes, size)).await.ok().flatten()
}

fn round_of(bytes: &[u8], size: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::load_from_memory(bytes).ok()?;
    let side = img.width().min(img.height());
    let sq = img.crop_imm((img.width() - side) / 2, (img.height() - side) / 2, side, side).thumbnail(size, size).to_rgba8();
    let n = sq.width();
    let mut px = sq.into_raw();
    let r = n as f32 / 2.0;
    for y in 0..n {
        for x in 0..n {
            let d = ((x as f32 + 0.5 - r).powi(2) + (y as f32 + 0.5 - r).powi(2)).sqrt();
            let i = ((y * n + x) * 4 + 3) as usize;
            px[i] = (px[i] as f32 * (r - d + 0.5).clamp(0.0, 1.0)) as u8;
        }
    }
    Some(SharedPixelBuffer::clone_from_slice(&px, n, n))
}

// ---------- lyrics, from lrclib.net ----------
#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub ms: u64,
    pub text: String,
}

/// Synced lines ("[mm:ss.xx] words"), or the plain text as unsynced lines (ms 0)
pub fn parse_lyrics(v: &Value) -> Vec<Line> {
    if let Some(synced) = v["syncedLyrics"].as_str().filter(|s| !s.trim().is_empty()) {
        let mut out: Vec<Line> = synced
            .lines()
            .filter_map(|l| {
                let rest = l.strip_prefix('[')?;
                let (stamp, text) = rest.split_once(']')?;
                let (m, s) = stamp.split_once(':')?;
                let ms = m.trim().parse::<u64>().ok()? * 60_000 + (s.trim().parse::<f64>().ok()? * 1000.0) as u64;
                Some(Line { ms, text: text.trim().to_string() })
            })
            .collect();
        out.sort_by_key(|l| l.ms);
        return out;
    }
    v["plainLyrics"].as_str().map(|p| p.lines().map(|l| Line { ms: 0, text: l.trim().to_string() }).collect()).unwrap_or_default()
}

/// The lyrics places, as the web app's LYRICS_PROVIDERS, in your order (prefs.lyricsOrder: [{ id, on }])
const PROVIDERS: [&str; 4] = ["lrclib", "netease", "kugou", "youtube"];

/// Every lyrics place, in your order, and whether it's on
pub fn lyrics_all(prefs: &Value) -> Vec<(String, bool)> {
    let mut out: Vec<(String, bool)> = prefs["lyricsOrder"].as_array().into_iter().flatten()
        .filter_map(|x| Some((x["id"].as_str()?.to_string(), x["on"].as_bool() != Some(false))))
        .filter(|(id, _)| PROVIDERS.contains(&id.as_str()))
        .collect();
    for p in PROVIDERS { if !out.iter().any(|(id, _)| id == p) { out.push((p.to_string(), true)); } }
    out
}

pub fn lyrics_order(prefs: &Value) -> Vec<String> {
    let mut out: Vec<(String, bool)> = prefs["lyricsOrder"].as_array().into_iter().flatten()
        .filter_map(|x| Some((x["id"].as_str()?.to_string(), x["on"].as_bool() != Some(false))))
        .filter(|(id, _)| PROVIDERS.contains(&id.as_str()))
        .collect();
    for p in PROVIDERS { if !out.iter().any(|(id, _)| id == p) { out.push((p.to_string(), true)); } }
    out.into_iter().filter(|(_, on)| *on).map(|(id, _)| id).collect()
}

/// A lyrics place's name, as shown ("Lyrics from KuGou")
pub fn lyrics_name(id: &str) -> &'static str {
    match id { "lrclib" => "LRCLIB", "netease" => "NetEase", "kugou" => "KuGou", "youtube" => "YouTube Music", _ => "" }
}

// The place you picked for a song's lyrics (Now Playing's other sources), kept by its key in
// state/native-lyrics-picks.json, as the web app keeps its own
fn picks_file() -> Option<std::path::PathBuf> {
    core::data_dir().ok().map(|d| d.join("state").join("native-lyrics-picks.json"))
}

fn lyrics_pick(key: &str) -> Option<String> {
    let v: Value = picks_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok())?;
    v[key].as_str().map(String::from)
}

pub fn lyrics_pick_set(key: &str, id: &str) {
    let Some(p) = picks_file() else { return };
    let mut v: Value = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_else(|| serde_json::json!({}));
    if !v.is_object() { v = serde_json::json!({}); }
    v[key] = serde_json::json!(id);
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    let _ = std::fs::write(p, v.to_string());
}

// What the places are asked with: the artist, the title, its length, the YouTube video it plays from
// (itself, or its version there: YouTube Music's lyrics)
async fn lyrics_query(t: &Track) -> (String, String, u64, String) {
    let (artist, title) = crate::versions::guess_names(t);
    let artist = if artist.is_empty() { t.artist.split(',').next().unwrap_or("").trim().to_string() } else { artist };
    let video = match &t.src {
        Src::YouTube(id) => id.clone(),
        _ if t.go_plus || matches!(t.src, Src::Catalog(_)) => crate::versions::find(t).await.map(|v| v.yt.id).unwrap_or_default(),
        _ => String::new(),
    };
    (artist, title, t.ms / 1000, video)
}

/// A remix's tag ("Someone Flip", "VIP", "Extended Edit"): what its title says beyond the song's own name, in
/// brackets or after " - ", when it says it's a version of the song
pub fn remix_tag(title: &str) -> Option<String> {
    static TAG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let rx = TAG.get_or_init(|| regex::Regex::new(r"(?i)[(\[]([^)\]]*\b(?:remix|rmx|flip|bootleg|rework|refix|vip|edit|mashup|remake|redo)\b[^)\]]*)[)\]]|\s[-\x{2013}\x{2014}]\s([^-\x{2013}\x{2014}]*\b(?:remix|rmx|flip|bootleg|rework|refix|vip|edit|mashup|remake|redo)\b[^-\x{2013}\x{2014}]*)$").unwrap());
    let c = rx.captures(title)?;
    let tag = c.get(1).or(c.get(2))?.as_str().trim().to_string();
    // ("Radio Edit" and "Original Mix" are the song itself, its lyrics the same)
    let l = tag.to_lowercase();
    if l == "radio edit" || l == "edit" || l.contains("original mix") || l.contains("clean edit") || l.contains("explicit edit") { return None; }
    Some(tag)
}

/// A track's lyrics (as the web app's findLyrics): the first timed ones, asking each place in your
/// order (LRCLIB, NetEase Cloud Music, KuGou, YouTube Music; the one you picked for the song first),
/// else the first plain ones; and the place they're from ("" for none). A remix: its own lyrics if a
/// place has them; if not, the original song's, untimed (their timing is the original's, not the
/// remix's), and the place's name with "+original" after it
pub async fn lyrics(t: &Track, prefs: &Value) -> (Vec<Line>, String) {
    let (artist, title, secs, video) = lyrics_query(t).await;
    let mut order = lyrics_order(prefs);
    let picked = lyrics_pick(&t.key).filter(|p| PROVIDERS.contains(&p.as_str()));
    if let Some(p) = picked.clone() {
        order.retain(|x| *x != p);
        order.insert(0, p);
    }
    if let Some(tag) = remix_tag(&t.title).filter(|_| picked.is_none()) {
        let named = format!("{title} {tag}");
        for p in order.iter().filter(|p| p.as_str() != "youtube") {
            let Ok(v) = core::lyrics::from(p, &artist, &named, secs, "").await else { continue };
            if let Some(l) = timed(&v) { return (l, p.clone()); }
        }
        let (lines, from) = original_lyrics(&order, &artist, &title, secs, &video).await;
        if lines.is_empty() { return (lines, from); }
        return (lines.into_iter().map(|l| Line { ms: 0, text: l.text }).filter(|l| !l.text.is_empty()).collect(), format!("{from}+original"));
    }
    original_lyrics(&order, &artist, &title, secs, &video).await
}

async fn original_lyrics(order: &[String], artist: &str, title: &str, secs: u64, video: &str) -> (Vec<Line>, String) {
    let order = order.to_vec();
    let mut plain: Option<(Value, String)> = None;
    for p in order {
        let Ok(v) = core::lyrics::from(&p, artist, title, secs, video).await else { continue };
        if v.is_null() { continue; }
        // (timed ones only if they are: a place can answer with an empty or untimed "synced")
        if let Some(l) = timed(&v) { return (l, p); }
        if plain.is_none() && v["plain"].as_str().is_some_and(|x| !x.trim().is_empty()) { plain = Some((serde_json::json!({ "plainLyrics": v["plain"] }), p)); }
    }
    plain.map(|(v, p)| (parse_lyrics(&v), p)).unwrap_or_default()
}

/// A track's lyrics from one place (Now Playing's other sources): timed if it has them, else plain
pub async fn lyrics_from(t: &Track, provider: &str) -> Vec<Line> {
    let (artist, title, secs, video) = lyrics_query(t).await;
    let Ok(v) = core::lyrics::from(provider, &artist, &title, secs, &video).await else { return Vec::new() };
    if let Some(l) = timed(&v) { return l; }
    if v["plain"].as_str().is_some_and(|x| !x.trim().is_empty()) { return parse_lyrics(&serde_json::json!({ "plainLyrics": v["plain"] })); }
    Vec::new()
}

/// A track's timed lyrics for Seamless Mix's voices' handoff: LRCLIB's, for a recording `secs` long (what
/// plays, as measured: a remix or another version of another length has none, its timing not the same)
pub async fn mix_lyrics(t: &Track, secs: u64) -> Vec<(u64, String)> {
    let (artist, title, _, _) = lyrics_query(t).await;
    let Ok(v) = core::lyrics::from("lrclib", &artist, &title, secs, "").await else { return Vec::new() };
    timed(&v).map(|l| l.into_iter().filter(|x| x.ms > 0 && !x.text.trim().is_empty()).map(|x| (x.ms, x.text)).collect()).unwrap_or_default()
}

/// A place's timed lyrics, if they are timed (not empty, not untimed text in the timed field)
fn timed(v: &Value) -> Option<Vec<Line>> {
    let l = parse_lyrics(&serde_json::json!({ "syncedLyrics": v["synced"].as_str()? }));
    l.iter().any(|x| x.ms > 0).then_some(l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_places_in_your_order() {
        let p = serde_json::json!({ "lyricsOrder": [{ "id": "kugou", "on": true }, { "id": "lrclib", "on": false }, { "id": "nope" }] });
        assert_eq!(lyrics_order(&p), ["kugou", "netease", "youtube"]);
        assert_eq!(lyrics_order(&Value::Null), ["lrclib", "netease", "kugou", "youtube"]);
    }

    #[test]
    fn synced_lyrics_are_read_in_order() {
        let v = serde_json::json!({ "syncedLyrics": "[00:12.50] Second\n[00:01.00] First\n[01:02.00] Third" });
        let l = parse_lyrics(&v);
        assert_eq!(l, vec![Line { ms: 1000, text: "First".into() }, Line { ms: 12500, text: "Second".into() }, Line { ms: 62000, text: "Third".into() }]);
        let p = parse_lyrics(&serde_json::json!({ "plainLyrics": "a\nb" }));
        assert_eq!(p.len(), 2);
        assert_eq!(p[0].ms, 0);
    }
}

#[cfg(test)]
mod lyrics_live {
    /// cargo test -p omnipulse --bin OmniPulse-native lyrics_sources_live -- --ignored --nocapture (the internet)
    #[test]
    #[ignore]
    fn lyrics_sources_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let t = crate::track::Track { src: crate::track::Src::YouTube("dQw4w9WgXcQ".into()), key: "yt:dQw4w9WgXcQ".into(), title: "Never Gonna Give You Up".into(), artist: "Rick Astley".into(), ms: 213_000, ..Default::default() };
        let (first, from) = rt.block_on(super::lyrics(&t, &serde_json::Value::Null));
        println!("in order: {} lines from {from}", first.len());
        for p in ["lrclib", "netease", "kugou", "youtube"] {
            let l = rt.block_on(super::lyrics_from(&t, p));
            println!("{}: {} lines, timed {}", super::lyrics_name(p), l.len(), l.iter().any(|x| x.ms > 0));
        }
        assert!(!first.is_empty() && !from.is_empty());
    }
}

#[cfg(test)]
mod remix_tests {
    use super::remix_tag;
    #[test]
    fn a_flip_or_a_remix_is_one_a_radio_edit_isnt() {
        assert_eq!(remix_tag("Drops of Jupiter (Someone Flip)").as_deref(), Some("Someone Flip"));
        assert_eq!(remix_tag("Song [VIP]").as_deref(), Some("VIP"));
        assert_eq!(remix_tag("Song - Artist Remix").as_deref(), Some("Artist Remix"));
        assert_eq!(remix_tag("Song (Radio Edit)"), None);
        assert_eq!(remix_tag("Song (Original Mix)"), None);
        assert_eq!(remix_tag("Song (feat. Someone)"), None);
        assert_eq!(remix_tag("Flipside"), None);
    }
}
