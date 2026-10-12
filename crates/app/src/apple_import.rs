// Apple Music, once (Settings > Integrations > Apple Music > Import), as the web app's js/applemusic.js:
// a read-only import. You sign in to Apple Music in a window of its own (crate::main's apple_sign_in);
// your loved songs and your playlists are read (core::apple), and the sign-in is forgotten straight
// away. Nothing on Apple Music is changed. Then:
//   - each loved song is liked where it's found: Spotify first (by its ISRC, the recording's own code,
//     else by its title and artist), then SoundCloud, then YouTube Music
//   - each playlist becomes a playlist on this PC, each song from wherever it's found
//   - what isn't found anywhere is listed, to copy

use crate::track::{Src, Track};
use omnipulse_core as core;
use serde_json::Value;

/// Where it can be looked for (platforms on, and signed in where that's needed)
#[derive(Clone, Copy)]
pub struct Where {
    pub spotify: bool,
    pub soundcloud: bool,
    pub youtube: bool,
}

/// An Apple song as a track to match with (its own platform is none of ours)
fn as_track(s: &Value) -> Track {
    let id = s["id"].as_str().map(String::from).unwrap_or_else(|| s["id"].to_string());
    Track {
        key: format!("am:{id}"),
        src: Src::Catalog(format!("am:{id}")),
        title: s["title"].as_str().unwrap_or("").to_string(),
        artist: s["artist"].as_str().unwrap_or("").to_string(),
        album: s["album"].as_str().unwrap_or("").to_string(),
        ms: s["ms"].as_u64().unwrap_or(0),
        art: s["art"].as_str().unwrap_or("").to_string(),
        ..Default::default()
    }
}

/// How it's said in the list of what wasn't found
pub fn line(s: &Value) -> String {
    format!("{} - {}", s["artist"].as_str().unwrap_or(""), s["title"].as_str().unwrap_or(""))
}

/// Where an Apple song is: the track as that platform has it, or None
pub async fn find(s: &Value, w: Where) -> Option<Track> {
    let t = as_track(s);
    if w.spotify {
        if let Some(isrc) = s["isrc"].as_str().filter(|x| !x.is_empty()) {
            let found = core::spotify::spotify_api("GET".into(), format!("/search?type=track&limit=1&q=isrc:{isrc}"), None).await.ok();
            if let Some(x) = found.as_ref().and_then(|v| v["tracks"]["items"][0].as_object()).map(|o| Value::Object(o.clone())).and_then(|v| Track::spotify(&v)) { return Some(x); }
        }
        if let Some(uri) = crate::mirror::counterpart(&t, "spotify").await {
            let id = uri.rsplit(':').next().unwrap_or("").to_string();
            if let Some(x) = core::spotify::spotify_api("GET".into(), format!("/tracks/{id}"), None).await.ok().and_then(|v| Track::spotify(&v)) { return Some(x); }
        }
    }
    if w.soundcloud {
        if let Some(id) = crate::mirror::counterpart(&t, "soundcloud").await {
            if let Some(x) = core::soundcloud::api_get(format!("/tracks/{id}")).await.ok().and_then(|v| Track::soundcloud(&v)) { return Some(x); }
        }
    }
    if w.youtube {
        if let Some(v) = crate::versions::find(&t).await { return Some(Track::youtube(&v.yt)); }
    }
    None
}

/// What the import did: the loved songs to like (found), the playlists to make (name, tracks found), and
/// what wasn't found anywhere
pub struct Found {
    pub likes: Vec<Track>,
    pub lists: Vec<(String, Vec<Track>)>,
    pub missing: Vec<String>,
}

/// A few at a time (three, as the web app's), saying how far it's got
async fn each(list: &[Value], w: Where, say: &(dyn Fn(usize, usize) + Sync)) -> Vec<Option<Track>> {
    use futures_util::stream::{self, StreamExt};
    let n = list.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    stream::iter(list.iter().cloned())
        .map(|s| {
            let done = &done;
            async move {
                let r = find(&s, w).await;
                let k = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
                if k % 5 == 0 || k == n { say(k, n); }
                r
            }
        })
        .buffered(3)
        .collect()
        .await
}

/// Everything read from Apple Music, looked for (`say`: what it's doing, as it goes)
pub async fn run(lib: Value, w: Where, say: impl Fn(String) + Sync) -> Found {
    let likes = lib["likes"].as_array().cloned().unwrap_or_default();
    let lists = lib["playlists"].as_array().cloned().unwrap_or_default();
    let mut missing = Vec::new();
    let found = each(&likes, w, &|k, n| say(format!("Importing from Apple Music: loved songs {k} of {n}..."))).await;
    let mut liked = Vec::new();
    for (s, f) in likes.iter().zip(found) { match f { Some(t) => liked.push(t), None => missing.push(line(s)) } }
    let mut made = Vec::new();
    for p in &lists {
        let name = p["name"].as_str().unwrap_or("Playlist").to_string();
        let tracks = p["tracks"].as_array().cloned().unwrap_or_default();
        let found = each(&tracks, w, &|k, n| say(format!("Importing from Apple Music: \u{201c}{name}\u{201d} {k} of {n}..."))).await;
        let mut got = Vec::new();
        for (s, f) in tracks.iter().zip(found) { match f { Some(t) => got.push(t), None => missing.push(format!("{} (in \u{201c}{name}\u{201d})", line(s))) } }
        made.push((name, got));
    }
    Found { likes: liked, lists: made, missing }
}
