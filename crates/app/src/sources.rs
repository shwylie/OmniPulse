// What the pages show, from each platform: likes, playlists, search, sets, related tracks.

use crate::track::{Src, Track};
use omnipulse_core as core;
use serde_json::Value;

#[derive(Clone, Debug)]
pub struct Playlist {
    pub plat: &'static str,
    pub id: String,
    pub title: String,
    /// the web app's own playlists come with their tracks
    pub tracks: Option<Vec<Track>>,
}

fn items(v: &Value) -> Vec<Value> {
    v["collection"].as_array().cloned().or_else(|| v.as_array().cloned()).unwrap_or_default()
}

fn next_path(v: &Value) -> Option<String> {
    v["next_href"].as_str().and_then(|h| h.strip_prefix("https://api.soundcloud.com")).map(String::from)
}

/// A SoundCloud list, page after page (up to `max`)
async fn sc_pages(first: String, max: usize) -> Vec<Track> {
    let mut out = Vec::new();
    let mut path = first;
    while out.len() < max {
        let Ok(v) = core::soundcloud::api_get(path.clone()).await else { break };
        let list = items(&v);
        out.extend(list.iter().filter_map(Track::soundcloud));
        match next_path(&v) {
            Some(n) if !list.is_empty() => path = n,
            _ => break,
        }
    }
    out
}

pub async fn soundcloud_likes() -> Vec<Track> {
    if !core::soundcloud::signed_in() {
        return Vec::new();
    }
    sc_pages("/me/likes/tracks?limit=200&linked_partitioning=true&access=playable".into(), 10_000).await
}

async fn sp_pages(first: String, max: usize, pick: fn(&Value) -> Option<Track>) -> Vec<Track> {
    let mut out = Vec::new();
    let mut offset = 0;
    while out.len() < max {
        let sep = if first.contains('?') { '&' } else { '?' };
        let Ok(v) = core::spotify::spotify_api("GET".into(), format!("{first}{sep}limit=50&offset={offset}&market=from_token"), None).await else { break };
        let list = v["items"].as_array().cloned().unwrap_or_default();
        out.extend(list.iter().filter_map(pick));
        if list.len() < 50 {
            break;
        }
        offset += 50;
    }
    out
}

pub async fn spotify_likes(max: usize) -> Vec<Track> {
    if !core::spotify::spotify_signed_in() {
        return Vec::new();
    }
    sp_pages("/me/tracks".into(), max, |it| Track::spotify(&it["track"])).await
}

pub fn local_files() -> Vec<Track> {
    core::local::local_load_index();
    core::local::LOCAL
        .lock()
        .map(|g| {
            g.iter()
                .map(|e| Track {
                    src: Src::Local(e.path.clone()),
                    key: e.id.clone(),
                    title: if e.title.is_empty() { std::path::Path::new(&e.path).file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled").to_string() } else { e.title.clone() },
                    artist: e.artist.clone(),
                    album: e.album.clone(),
                    ms: e.duration,
                    art: String::new(),
                    genre: String::new(),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Your playlists on SoundCloud and Spotify (the web app's own come from its saved data)
pub async fn playlists() -> Vec<Playlist> {
    let mut out = Vec::new();
    if core::soundcloud::signed_in() {
        if let Ok(v) = core::soundcloud::api_get("/me/playlists?limit=200&linked_partitioning=true&show_tracks=false".into()).await {
            for p in items(&v) {
                if let (Some(id), Some(title)) = (p["id"].as_u64(), p["title"].as_str()) {
                    out.push(Playlist { plat: "soundcloud", id: id.to_string(), title: title.to_string(), tracks: None });
                }
            }
        }
    }
    if core::spotify::spotify_signed_in() {
        if let Ok(v) = core::spotify::spotify_api("GET".into(), "/me/playlists?limit=50".into(), None).await {
            for p in v["items"].as_array().cloned().unwrap_or_default() {
                if let (Some(id), Some(title)) = (p["id"].as_str(), p["name"].as_str()) {
                    out.push(Playlist { plat: "spotify", id: id.to_string(), title: title.to_string(), tracks: None });
                }
            }
        }
    }
    out
}

pub async fn playlist_tracks(p: &Playlist) -> Vec<Track> {
    if let Some(t) = &p.tracks {
        return t.clone();
    }
    match p.plat {
        "soundcloud" => sc_pages(format!("/playlists/{}/tracks?access=playable&limit=200&linked_partitioning=true", p.id), 2000).await,
        "spotify" => sp_pages(format!("/playlists/{}/items", p.id), 1000, |it| Track::spotify(if it.get("item").is_some() { &it["item"] } else { &it["track"] })).await,
        _ => Vec::new(),
    }
}

/// A search on every platform, taking turns so each gets a look in
pub async fn search(q: &str) -> Vec<Track> {
    let enc = urlencode(q);
    let sc = async {
        if !core::soundcloud::signed_in() {
            return Vec::new();
        }
        core::soundcloud::api_get(format!("/tracks?q={enc}&limit=50&access=playable&linked_partitioning=true")).await.map(|v| items(&v).iter().filter_map(Track::soundcloud).collect()).unwrap_or_default()
    };
    let sp = async {
        if !core::spotify::spotify_signed_in() {
            return Vec::new();
        }
        core::spotify::spotify_api("GET".into(), format!("/search?type=track&limit=10&market=from_token&q={enc}"), None)
            .await
            .map(|v| v["tracks"]["items"].as_array().cloned().unwrap_or_default().iter().filter_map(Track::spotify).collect())
            .unwrap_or_default()
    };
    let yt = async { core::youtube::search(q).await.map(|l| l.iter().map(Track::youtube).collect()).unwrap_or_default() };
    let (a, b, c): (Vec<Track>, Vec<Track>, Vec<Track>) = tokio::join!(sc, sp, yt);
    interleave(vec![a, b, c])
}

pub fn interleave(lists: Vec<Vec<Track>>) -> Vec<Track> {
    let mut its: Vec<_> = lists.into_iter().map(|l| l.into_iter()).collect();
    let mut out = Vec::new();
    loop {
        let mut any = false;
        for it in its.iter_mut() {
            if let Some(t) = it.next() {
                out.push(t);
                any = true;
            }
        }
        if !any {
            return out;
        }
    }
}

/// New from the artists you follow on SoundCloud
pub async fn following_tracks(max: usize) -> Vec<Track> {
    if !core::soundcloud::signed_in() {
        return Vec::new();
    }
    sc_pages("/me/followings/tracks?access=playable&limit=100&linked_partitioning=true".into(), max).await
}

/// Tracks like this one: SoundCloud's related tracks, or YouTube Music's radio
pub async fn related(t: &Track) -> Vec<Track> {
    match &t.src {
        Src::SoundCloud(id) => core::soundcloud::api_get(format!("/tracks/{id}/related?access=playable&limit=30")).await.map(|v| items(&v).iter().filter_map(Track::soundcloud).collect()).unwrap_or_default(),
        Src::YouTube(id) => core::youtube::radio(id).await.map(|l| l.iter().map(Track::youtube).skip(1).collect()).unwrap_or_default(),
        _ => {
            // elsewhere: YouTube Music's radio for the same song
            let q = format!("{} {}", t.artist, t.title);
            match core::youtube::search(&q).await.ok().and_then(|l| l.into_iter().next()) {
                Some(first) => core::youtube::radio(&first.id).await.map(|l| l.iter().map(Track::youtube).skip(1).collect()).unwrap_or_default(),
                None => Vec::new(),
            }
        }
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
