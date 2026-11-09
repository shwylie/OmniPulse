// An addon asked directly, with no script: a server with a manifest.json at its address (the BeatBoss /
// Eclipse Music shape), asked for songs by search (GET /search?q=) and for how to play one (its streamURL,
// else GET /stream/:id). In the apps with the plugin runtime OmniPulse's adapter script (addon.js) does this;
// a build without the runtime uses this, the same adapter in Rust. Every request goes through
// plugins::http_for, so an addon reaches only its own site, as its manifest says.

use crate::plugins::{http_for, Manifest};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A song as an addon gives it, in OmniPulse's words
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Song {
    pub id: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub ms: u64,
    pub artwork: String,
    /// its stream's address, when the addon gives it with the song (else asked for by /stream/:id)
    pub stream: String,
    /// its format ("mp3", "flac", "m4a"...), lower-case; "" when the addon doesn't say
    pub format: String,
}

async fn get(m: &Manifest, path: &str) -> Result<Value, String> {
    let r = http_for(m.hosts.clone(), &format!("{}{}", m.addon.trim_end_matches('/'), path), "GET", Vec::new(), None).await?;
    let status = r["status"].as_u64().unwrap_or(0);
    if !(200..300).contains(&status) {
        return Err(format!("The addon answered {status}."));
    }
    serde_json::from_str(r["body"].as_str().unwrap_or("")).map_err(|_| "The addon's answer wasn't JSON.".to_string())
}

/// An artist as a name: "Name", { name }, or several of either
fn artist_of(a: &Value) -> String {
    match a {
        Value::Array(l) => l.iter().map(artist_of).filter(|s| !s.is_empty()).collect::<Vec<_>>().join(", "),
        Value::Object(o) => o.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string(),
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn album_of(a: &Value) -> String {
    match a {
        Value::Object(o) => o.get("name").or_else(|| o.get("title")).and_then(|n| n.as_str()).unwrap_or("").to_string(),
        Value::String(s) => s.clone(),
        _ => String::new(),
    }
}

/// (seconds, as the spec says; a number far too big for seconds was milliseconds)
fn ms_of(d: &Value) -> u64 {
    let n = d.as_f64().or_else(|| d.as_str().and_then(|s| s.parse().ok())).unwrap_or(0.0).max(0.0);
    if n > 20000.0 { n.round() as u64 } else { (n * 1000.0).round() as u64 }
}

fn text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn first<'a>(t: &'a Value, keys: &[&str]) -> &'a Value {
    keys.iter().map(|k| &t[*k]).find(|v| !v.is_null()).unwrap_or(&Value::Null)
}

/// One of the addon's songs, in OmniPulse's words
pub fn song(t: &Value) -> Option<Song> {
    if t["id"].is_null() {
        return None;
    }
    let title = text(first(t, &["title", "name"]));
    let artist = artist_of(first(t, &["artist", "artists"]));
    Some(Song {
        id: text(&t["id"]),
        title: if title.is_empty() { "Untitled".into() } else { title },
        artist: if artist.is_empty() { "Unknown artist".into() } else { artist },
        album: album_of(&t["album"]),
        ms: ms_of(&t["duration"]),
        artwork: text(first(t, &["artworkURL", "image", "cover", "albumCover"])),
        stream: text(first(t, &["streamURL", "stream_url"])),
        format: text(&t["format"]).to_lowercase(),
    })
}

/// The addon's songs for a search
pub async fn search(m: &Manifest, query: &str) -> Result<Vec<Song>, String> {
    let q = query.trim();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let d = get(m, &format!("/search?q={}&limit=50", urlencoding::encode(q))).await?;
    let list = match &d {
        Value::Array(l) => l.clone(),
        other => other["tracks"].as_array().cloned().unwrap_or_default(),
    };
    Ok(list.iter().filter_map(song).collect())
}

/// How to play one of its songs: (the stream's address, its format or "")
pub async fn resolve(m: &Manifest, id: &str, stream: &str, format: &str) -> Result<(String, String), String> {
    if !stream.is_empty() {
        return Ok((stream.to_string(), format.to_lowercase()));
    }
    let s = get(m, &format!("/stream/{}", urlencoding::encode(id))).await?;
    let url = text(first(&s, &["url", "streamURL", "stream_url"]));
    if url.is_empty() {
        return Err("The addon gave no stream for it.".into());
    }
    let ext = text(&s["format"]);
    Ok((url, if ext.is_empty() { format.to_lowercase() } else { ext.to_lowercase() }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn songs_read_as_the_script_does() {
        let s = song(&json!({ "id": 7, "name": "Strobe", "artists": [{ "name": "deadmau5" }, "Someone"], "album": { "title": "For Lack" }, "duration": 634.2, "cover": "https://x/c.jpg", "streamURL": "https://x/7.mp3", "format": "MP3" })).unwrap();
        assert_eq!(s, Song { id: "7".into(), title: "Strobe".into(), artist: "deadmau5, Someone".into(), album: "For Lack".into(), ms: 634200, artwork: "https://x/c.jpg".into(), stream: "https://x/7.mp3".into(), format: "mp3".into() });
        // (milliseconds given as seconds; nothing known: the stand-ins)
        let s = song(&json!({ "id": "a", "duration": 634200 })).unwrap();
        assert_eq!((s.title.as_str(), s.artist.as_str(), s.ms), ("Untitled", "Unknown artist", 634200));
        assert!(song(&json!({ "title": "no id" })).is_none());
    }
}
