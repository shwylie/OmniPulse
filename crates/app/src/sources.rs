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
    /// yours to change (one you only follow, or anyone else's, isn't)
    pub own: bool,
}

/// Go+ tracks in SoundCloud's lists: only while YouTube Music can play them in full (set from main)
pub static GO_PLUS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// A SoundCloud request, asking for Go+ tracks too when they can be played
fn sc_path(path: String) -> String {
    if GO_PLUS.load(std::sync::atomic::Ordering::Relaxed) { path.replace("access=playable", "access=playable,preview") } else { path }
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
    let mut empty = 0;
    while out.len() < max {
        let Ok(v) = core::soundcloud::api_get(sc_path(path.clone())).await else { break };
        let list = items(&v);
        out.extend(list.iter().filter_map(Track::soundcloud));
        // (a page can come back empty, with more after it: a few of those in a row and it stops)
        empty = if list.is_empty() { empty + 1 } else { 0 };
        match next_path(&v) {
            Some(n) if empty < 4 => path = n,
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
                    link: String::new(),
                    go_plus: false, ..Default::default()
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
                    out.push(Playlist { plat: "soundcloud", id: id.to_string(), title: title.to_string(), tracks: None, own: true });
                }
            }
        }
    }
    if core::spotify::spotify_signed_in() {
        let me = core::spotify::spotify_api("GET".into(), "/me".into(), None).await.ok().and_then(|m| m["id"].as_str().map(String::from)).unwrap_or_default();
        if let Ok(v) = core::spotify::spotify_api("GET".into(), "/me/playlists?limit=50".into(), None).await {
            for p in v["items"].as_array().cloned().unwrap_or_default() {
                if let (Some(id), Some(title)) = (p["id"].as_str(), p["name"].as_str()) {
                    out.push(Playlist { plat: "spotify", id: id.to_string(), title: title.to_string(), tracks: None, own: !me.is_empty() && p["owner"]["id"].as_str() == Some(me.as_str()) });
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
        core::soundcloud::api_get(sc_path(format!("/tracks?q={enc}&limit=50&access=playable&linked_partitioning=true"))).await.map(|v| items(&v).iter().filter_map(Track::soundcloud).collect()).unwrap_or_default()
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

// ---------- the artists you follow (the Artists page) ----------
#[derive(Clone, Debug, Default)]
pub struct Artist {
    pub name: String,
    pub img: String,
    /// their SoundCloud user id and Spotify artist id, where you follow them
    pub sc: Option<u64>,
    pub sp: Option<String>,
}

impl Artist {
    pub fn plats(&self) -> Vec<&'static str> {
        [self.sc.is_some().then_some("soundcloud"), self.sp.is_some().then_some("spotify")].into_iter().flatten().collect()
    }
}

fn name_key(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

/// Everyone you follow on SoundCloud and Spotify, as one list (the same name is one artist), A to Z
pub async fn followed_artists() -> Vec<Artist> {
    let mut by: std::collections::BTreeMap<String, Artist> = Default::default();
    let mut add = |name: &str, img: &str, sc: Option<u64>, sp: Option<String>| {
        let k = name_key(name);
        if k.is_empty() {
            return;
        }
        let a = by.entry(k).or_insert_with(|| Artist { name: name.to_string(), ..Default::default() });
        if a.img.is_empty() {
            a.img = img.to_string();
        }
        if sc.is_some() { a.sc = sc; }
        if sp.is_some() { a.sp = sp; }
    };
    if core::soundcloud::signed_in() {
        let mut path = "/me/followings?limit=200&linked_partitioning=true".to_string();
        for _ in 0..10 {
            let Ok(v) = core::soundcloud::api_get(sc_path(path.clone())).await else { break };
            for u in items(&v) {
                let img = u["avatar_url"].as_str().unwrap_or("").replace("-large", "-t200x200");
                add(u["username"].as_str().unwrap_or(""), &img, u["id"].as_u64(), None);
            }
            match next_path(&v) { Some(n) => path = n, None => break }
        }
    }
    if core::spotify::spotify_signed_in() {
        let mut after = String::new();
        for _ in 0..20 {
            let q = format!("/me/following?type=artist&limit=50{}", if after.is_empty() { String::new() } else { format!("&after={after}") });
            let Ok(v) = core::spotify::spotify_api("GET".into(), q, None).await else { break };
            let page = &v["artists"];
            for a in page["items"].as_array().cloned().unwrap_or_default() {
                let img = a["images"].as_array().and_then(|i| i.last()).and_then(|i| i["url"].as_str()).unwrap_or("").to_string();
                add(a["name"].as_str().unwrap_or(""), &img, None, a["id"].as_str().map(String::from));
            }
            after = page["cursors"]["after"].as_str().unwrap_or("").to_string();
            if after.is_empty() || page["next"].is_null() { break; }
        }
    }
    let mut list: Vec<Artist> = by.into_values().collect();
    list.sort_by_key(|a| a.name.to_lowercase());
    list
}

/// An artist's tracks: their SoundCloud uploads and their songs on Spotify, taking turns
pub async fn artist_tracks(a: &Artist) -> Vec<Track> {
    let sc = async {
        match a.sc {
            Some(id) => sc_pages(format!("/users/{id}/tracks?access=playable&limit=50&linked_partitioning=true"), 100).await,
            None => Vec::new(),
        }
    };
    let sp = async {
        if a.sp.is_none() {
            return Vec::new();
        }
        let want = name_key(&a.name);
        let q = urlencode(&format!("artist:\"{}\"", a.name.replace('"', "")));
        // (Spotify gives an app 10 at a time: three pages of them)
        let mut out = Vec::new();
        for offset in [0, 10, 20] {
            let Ok(v) = core::spotify::spotify_api("GET".into(), format!("/search?type=track&limit=10&offset={offset}&market=from_token&q={q}"), None).await else { break };
            let page = v["tracks"]["items"].as_array().cloned().unwrap_or_default();
            out.extend(page.iter().filter_map(Track::spotify).filter(|t| t.artist.split(", ").any(|n| name_key(n) == want)));
            if page.len() < 10 { break; }
        }
        out
    };
    let (sc, sp) = tokio::join!(sc, sp);
    interleave(vec![sc, sp])
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
        Src::SoundCloud(id) => core::soundcloud::api_get(sc_path(format!("/tracks/{id}/related?access=playable&limit=30"))).await.map(|v| items(&v).iter().filter_map(Track::soundcloud).collect()).unwrap_or_default(),
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

// ---------- autoplay: tracks like the one playing, from each platform ----------
pub async fn sc_search(q: &str, limit: usize) -> Vec<Track> {
    core::soundcloud::api_get(sc_path(format!("/tracks?q={}&limit={limit}&access=playable&linked_partitioning=true", urlencode(q)))).await.map(|v| items(&v).iter().filter_map(Track::soundcloud).collect()).unwrap_or_default()
}

/// A SoundCloud search with its Go+ uploads in it (for matching songs from other platforms)
pub async fn sc_search_previews(q: &str, limit: usize) -> Vec<Track> {
    core::soundcloud::api_get(format!("/tracks?q={}&limit={limit}&access=playable,preview&linked_partitioning=true", urlencode(q))).await.map(|v| items(&v).iter().filter_map(Track::soundcloud).collect()).unwrap_or_default()
}

pub async fn sp_search(q: &str) -> Vec<Track> {
    core::spotify::spotify_api("GET".into(), format!("/search?type=track&limit=10&market=from_token&q={}", urlencode(q)), None)
        .await
        .map(|v| v["tracks"]["items"].as_array().cloned().unwrap_or_default().iter().filter_map(Track::spotify).collect())
        .unwrap_or_default()
}

/// Artists like this one, from Last.fm (Spotify no longer offers related artists)
async fn similar_artists(artist: &str, n: usize) -> Vec<String> {
    core::lastfm::lastfm_call("artist.getSimilar".into(), vec![("artist".into(), artist.into()), ("limit".into(), n.to_string())])
        .await
        .ok()
        .and_then(|v| v["similarartists"]["artist"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect()))
        .unwrap_or_default()
}

fn first_artist(t: &Track) -> String {
    t.artist.split([',', '&']).next().unwrap_or("").trim().to_string()
}

pub async fn autoplay_soundcloud(seed: &Track) -> Vec<Track> {
    if !core::soundcloud::signed_in() {
        return Vec::new();
    }
    let mut out = match &seed.src {
        Src::SoundCloud(_) => related(seed).await,
        _ => match sc_search(&format!("{} {}", first_artist(seed), seed.title), 5).await.into_iter().next() {
            Some(found) => related(&found).await,
            None => Vec::new(),
        },
    };
    crate::queue::shuffle_in_place(&mut out);
    out
}

pub async fn autoplay_spotify(seed: &Track) -> Vec<Track> {
    if !core::spotify::spotify_signed_in() {
        return Vec::new();
    }
    let artist = first_artist(seed);
    if artist.is_empty() {
        return Vec::new();
    }
    let mut names = similar_artists(&artist, 10).await;
    crate::queue::shuffle_in_place(&mut names);
    names.truncate(4);
    if names.is_empty() {
        names.push(artist);
    }
    let lists = futures_join_all(names.iter().map(|n| sp_search_owned(format!("artist:\"{}\"", n.replace('"', ""))))).await;
    let mut out: Vec<Track> = lists.into_iter().flatten().collect();
    crate::queue::shuffle_in_place(&mut out);
    out
}

async fn sp_search_owned(q: String) -> Vec<Track> {
    sp_search(&q).await
}

/// A few futures at once, without another dependency
async fn futures_join_all<F: std::future::Future<Output = Vec<Track>> + Send + 'static>(futs: impl Iterator<Item = F>) -> Vec<Vec<Track>> {
    let handles: Vec<_> = futs.map(tokio::spawn).collect();
    let mut out = Vec::new();
    for h in handles {
        out.push(h.await.unwrap_or_default());
    }
    out
}

pub async fn autoplay_youtube(seed: &Track) -> Vec<Track> {
    let mut out = match &seed.src {
        Src::YouTube(_) => related(seed).await,
        _ => {
            let q = format!("{} {}", first_artist(seed), seed.title);
            match core::youtube::search(&q).await.ok().and_then(|l| l.into_iter().next()) {
                Some(first) => core::youtube::radio(&first.id).await.map(|l| l.iter().map(Track::youtube).skip(1).collect()).unwrap_or_default(),
                None => Vec::new(),
            }
        }
    };
    crate::queue::shuffle_in_place(&mut out);
    out
}

/// Your files: the same artist, the same album, then anything
pub fn autoplay_local(seed: &Track, local: &[Track]) -> Vec<Track> {
    let who = first_artist(seed).to_lowercase();
    let mut same: Vec<Track> = local.iter().filter(|t| first_artist(t).to_lowercase() == who).cloned().collect();
    let mut album: Vec<Track> = if seed.album.is_empty() { Vec::new() } else { local.iter().filter(|t| t.album == seed.album).cloned().collect() };
    let mut rest = local.to_vec();
    crate::queue::shuffle_in_place(&mut same);
    crate::queue::shuffle_in_place(&mut album);
    crate::queue::shuffle_in_place(&mut rest);
    same.extend(album);
    same.extend(rest);
    same
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
