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

pub async fn open_source(t: &Track, rt: tokio::runtime::Handle) -> Result<Box<dyn Source>, String> {
    match &t.src {
        Src::Local(path) => {
            let p = path.clone();
            tokio::task::spawn_blocking(move || Decoded::open_file(std::path::Path::new(&p)).map(|d| Box::new(d) as Box<dyn Source>)).await.map_err(|e| e.to_string())?
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
            let url = core::youtube::stream_url(id, false).await?;
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
