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
async fn other_upload(t: &Track, id: &str) -> Option<String> {
    let found = core::youtube::search(&format!("{} {}", t.artist, t.title)).await.ok()?;
    for c in found.iter().filter(|c| c.id != id && crate::versions::score(t, c) >= 0.8).take(3) {
        match core::youtube::stream_url(&c.id, false).await {
            Ok(url) => return Some(url),
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
            let url = match core::youtube::stream_url(id, false).await {
                Ok(url) => url,
                // age-restricted: another upload of the same song often isn't (the closest few)
                Err(e) if age_blocked(&e) => other_upload(t, id).await.ok_or_else(|| AGE_BLOCKED.to_string())?,
                Err(e) => return Err(e),
            };
            tokio::task::spawn_blocking(move || omnipulse_audio::open_url(&url, "m4a").map(|d| Box::new(d) as Box<dyn Source>)).await.map_err(|e| e.to_string())?
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
pub async fn fetch_art(url: String, size: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    if url.is_empty() {
        return None;
    }
    let bytes = reqwest::get(&url).await.ok()?.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?.thumbnail(size, size).to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

/// A picture as a circle (an artist's): cropped square from its middle, the corners clear, the edge
/// smoothed. (The renderer can't clip to a round shape itself.)
pub async fn fetch_round(url: String, size: u32) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    if url.is_empty() {
        return None;
    }
    let bytes = reqwest::get(&url).await.ok()?.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?;
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

pub async fn lyrics(t: &Track) -> Vec<Line> {
    let enc = |s: &str| s.bytes().map(|b| if b.is_ascii_alphanumeric() { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>();
    let artist = t.artist.split(',').next().unwrap_or("").trim().to_string();
    let title = t.title.split(" (").next().unwrap_or(&t.title).trim().to_string();
    let dur = if t.ms > 0 { format!("&duration={}", t.ms / 1000) } else { String::new() };
    let got = core::lyrics::lyrics_get(format!("https://lrclib.net/api/get?artist_name={}&track_name={}{dur}", enc(&artist), enc(&title))).await;
    let v = match got {
        Ok(v) => v,
        Err(_) => core::lyrics::lyrics_get(format!("https://lrclib.net/api/search?q={}", enc(&format!("{artist} {title}"))))
            .await
            .ok()
            .and_then(|l| l.as_array().and_then(|a| a.first().cloned()))
            .unwrap_or(Value::Null),
    };
    parse_lyrics(&v)
}

#[cfg(test)]
mod tests {
    use super::*;

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
