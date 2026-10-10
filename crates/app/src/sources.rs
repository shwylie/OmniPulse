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

/// A search on every platform (every one answers before anything shows), best match first (rank)
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
    // MusicBrainz: its songs play from YouTube Music, so only while that can play (yt-dlp here)
    let mb = async {
        if !MUSICBRAINZ.load(std::sync::atomic::Ordering::Relaxed) || core::tools::ytdlp_path().is_none() {
            return Vec::new();
        }
        core::musicbrainz::search(q).await.map(|l| l.iter().filter_map(Track::musicbrainz).collect()).unwrap_or_default()
    };
    // Deezer's and Apple's catalogues (no account): their songs play from YouTube Music too
    let cat = async {
        if !CATALOG.load(std::sync::atomic::Ordering::Relaxed) || core::tools::ytdlp_path().is_none() {
            return Vec::new();
        }
        core::catalog::search(q).await.map(|l| l.iter().filter_map(Track::catalog).collect()).unwrap_or_default()
    };
    // each running plugin that can search and play (crate::plugrun)
    let plugs = futures_util::future::join_all(crate::plugrun::players().into_iter().map(|p| {
        let q = q.to_string();
        async move { p.call("search", vec![serde_json::json!(q)]).await.map(|v| v.as_array().cloned().unwrap_or_default().iter().take(200).filter_map(|x| Track::plugin(&p.id, x)).collect::<Vec<_>>()).unwrap_or_default() }
    }));
    let (a, b, c, d, f, e) = tokio::join!(sc, sp, yt, mb, cat, plugs);
    let mut lists = vec![a, b, c, d, f];
    lists.extend(e);
    once_each(rank(q, interleave(lists)))
}

/// The same song found twice (the same title and first artist, about as long) shows once, where it
/// first came (the web app merges them into one row too)
pub fn once_each(list: Vec<Track>) -> Vec<Track> {
    let key = |t: &Track| format!("{}|{}", words(&t.title).join(" "), words(t.artist.split([',', '&']).next().unwrap_or("")).join(" "));
    let mut seen: Vec<(String, u64)> = Vec::new();
    list.into_iter().filter(|t| {
        let k = key(t);
        if seen.iter().any(|(sk, ms)| *sk == k && (t.ms == 0 || *ms == 0 || t.ms.abs_diff(*ms) <= 3000)) { return false; }
        seen.push((k, t.ms));
        true
    }).collect()
}

/// Something a search found besides tracks (Search's shelves, as the web app's): an artist (where they
/// are on each platform), an album or a playlist
#[derive(Clone, Debug)]
pub enum Found {
    Artist { name: String, img: String, sc: Option<u64>, sp: Option<String>, yt: Option<String> },
    Album { plat: &'static str, id: String, title: String, by: String, img: String },
    List { plat: &'static str, id: String, title: String, by: String, img: String },
}

impl Found {
    /// (title, the line under it, its picture, the platform's icon)
    pub fn tile(&self) -> (String, String, String, &'static str) {
        match self {
            Found::Artist { name, img, sc, sp, .. } => (name.clone(), "Artist".into(), img.clone(), if sp.is_some() { "spotify" } else if sc.is_some() { "soundcloud" } else { "youtube" }),
            Found::Album { plat, title, by, img, .. } => (title.clone(), by.clone(), img.clone(), plat),
            Found::List { plat, title, by, img, .. } => (title.clone(), by.clone(), img.clone(), plat),
        }
    }
}

/// Artists, albums and playlists for a search, from every platform (Spotify gives an app 10 at a time)
pub async fn search_found(q: &str) -> (Vec<Found>, Vec<Found>, Vec<Found>) {
    let enc = urlencode(q);
    let sc_on = core::soundcloud::signed_in();
    let sp_on = core::spotify::spotify_signed_in();
    let sp = |kind: &'static str| {
        let enc = enc.clone();
        async move {
            if !sp_on { return Value::Null; }
            core::spotify::spotify_api("GET".into(), format!("/search?type={kind}&limit=10&market=from_token&q={enc}"), None).await.unwrap_or_default()
        }
    };
    let sc = |path: String| async move { if sc_on { core::soundcloud::api_get(sc_path(path)).await.unwrap_or_default() } else { Value::Null } };
    let (sp_art, sp_alb, sp_pl, sc_users, sc_pl, yt_art, yt_pl) = tokio::join!(
        sp("artist"), sp("album"), sp("playlist"),
        sc(format!("/users?q={enc}&limit=6")), sc(format!("/playlists?q={enc}&limit=8&linked_partitioning=true")),
        async { core::youtube::search_artists(q).await.unwrap_or_default() }, async { core::youtube::search_playlists(q).await.unwrap_or_default() },
    );
    let img_of = |v: &Value| v["images"].as_array().and_then(|i| i.get(1).or(i.first())).and_then(|i| i["url"].as_str()).unwrap_or("").to_string();
    // artists: the same name on several platforms is one
    let mut artists: Vec<Found> = Vec::new();
    let mut put = |name: &str, img: String, sc: Option<u64>, sp: Option<String>, yt: Option<String>| {
        let k = name_key(name);
        if k.is_empty() { return; }
        match artists.iter_mut().find(|a| matches!(a, Found::Artist { name, .. } if name_key(name) == k)) {
            Some(Found::Artist { img: i, sc: a, sp: b, yt: c, .. }) => {
                if i.is_empty() { *i = img; }
                if a.is_none() { *a = sc; }
                if b.is_none() { *b = sp; }
                if c.is_none() { *c = yt; }
            }
            _ => artists.push(Found::Artist { name: name.to_string(), img, sc, sp, yt }),
        }
    };
    for a in sp_art["artists"]["items"].as_array().cloned().unwrap_or_default().iter().take(6) {
        put(a["name"].as_str().unwrap_or(""), img_of(a), None, a["id"].as_str().map(String::from), None);
    }
    for u in items(&sc_users).iter().take(6) {
        put(u["username"].as_str().unwrap_or(""), u["avatar_url"].as_str().unwrap_or("").replace("-large", "-t200x200"), u["id"].as_u64(), None, None);
    }
    for a in yt_art.iter().take(6) {
        put(&a.name, a.thumb.clone(), None, None, Some(a.id.clone()));
    }
    let albums: Vec<Found> = sp_alb["albums"]["items"].as_array().cloned().unwrap_or_default().iter().filter_map(|a| Some(Found::Album {
        plat: "spotify", id: a["id"].as_str()?.to_string(), title: a["name"].as_str().unwrap_or("").to_string(),
        by: a["artists"].as_array().map(|x| x.iter().filter_map(|y| y["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(), img: img_of(a),
    })).collect();
    let mut lists: Vec<Found> = Vec::new();
    for p in items(&sc_pl).iter().take(8) {
        if let Some(id) = p["id"].as_u64() {
            lists.push(Found::List { plat: "soundcloud", id: id.to_string(), title: p["title"].as_str().unwrap_or("").to_string(), by: p["user"]["username"].as_str().unwrap_or("").to_string(), img: p["artwork_url"].as_str().unwrap_or("").replace("-large", "-t200x200") });
        }
    }
    for p in sp_pl["playlists"]["items"].as_array().cloned().unwrap_or_default().iter().filter(|p| !p.is_null()).take(8) {
        if let Some(id) = p["id"].as_str() {
            lists.push(Found::List { plat: "spotify", id: id.to_string(), title: p["name"].as_str().unwrap_or("").to_string(), by: p["owner"]["display_name"].as_str().unwrap_or("").to_string(), img: img_of(p) });
        }
    }
    for p in yt_pl.iter().take(8) {
        lists.push(Found::List { plat: "youtube", id: p.id.clone(), title: p.title.clone(), by: p.author.clone(), img: p.thumb.clone() });
    }
    (artists, albums, lists)
}

/// Your saved albums: Spotify's saved albums and the albums you've liked on SoundCloud (as the web
/// app's Albums chip), newest first, taking turns
pub async fn saved_albums() -> Vec<Found> {
    let sp = async {
        if !core::spotify::spotify_signed_in() { return Vec::new(); }
        let mut out = Vec::new();
        for offset in [0, 50, 100, 150] {
            let Ok(v) = core::spotify::spotify_api("GET".into(), format!("/me/albums?limit=50&offset={offset}&market=from_token"), None).await else { break };
            let page = v["items"].as_array().cloned().unwrap_or_default();
            out.extend(page.iter().filter_map(|x| {
                let a = &x["album"];
                Some(Found::Album {
                    plat: "spotify", id: a["id"].as_str()?.to_string(), title: a["name"].as_str().unwrap_or("").to_string(),
                    by: a["artists"].as_array().map(|x| x.iter().filter_map(|y| y["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                    img: a["images"].as_array().and_then(|i| i.get(1).or(i.first())).and_then(|i| i["url"].as_str()).unwrap_or("").to_string(),
                })
            }));
            if page.len() < 50 { break; }
        }
        out
    };
    let sc = async {
        if !core::soundcloud::signed_in() { return Vec::new(); }
        let Ok(v) = core::soundcloud::api_get(sc_path("/me/likes/playlists?limit=50&linked_partitioning=true".into())).await else { return Vec::new() };
        items(&v).iter().filter_map(|x| {
            let p = if x.get("playlist").is_some() { &x["playlist"] } else { x };
            let kind = format!("{} {}", p["playlist_type"].as_str().unwrap_or(""), p["set_type"].as_str().unwrap_or("")).to_lowercase();
            let album = ["album", "ep", "compilation"].iter().any(|k| kind.contains(k)) || p["is_album"].as_bool() == Some(true);
            if !album { return None; }
            Some(Found::Album {
                plat: "soundcloud", id: p["id"].as_u64()?.to_string(), title: p["title"].as_str().unwrap_or("").to_string(),
                by: p["user"]["username"].as_str().unwrap_or("").to_string(), img: p["artwork_url"].as_str().unwrap_or("").replace("-large", "-t200x200"),
            })
        }).collect()
    };
    let (sp, sc) = tokio::join!(sp, sc);
    let mut out = Vec::new();
    let (mut a, mut b) = (sp.into_iter(), sc.into_iter());
    loop {
        let (x, y) = (a.next(), b.next());
        if x.is_none() && y.is_none() { return out; }
        out.extend(x);
        out.extend(y);
    }
}

/// Albums among your music files: an album name shared by more than one of them
pub fn local_albums(files: &[Track]) -> Vec<Found> {
    let mut by: std::collections::BTreeMap<String, (Found, usize)> = Default::default();
    for t in files.iter().filter(|t| t.plat() == "local" && !t.album.trim().is_empty()) {
        let e = by.entry(t.album.trim().to_lowercase()).or_insert_with(|| (Found::Album { plat: "local", id: t.album.trim().to_string(), title: t.album.trim().to_string(), by: t.artist.clone(), img: t.art.clone() }, 0));
        e.1 += 1;
    }
    by.into_values().filter(|(_, n)| *n > 1).map(|(f, _)| f).collect()
}

/// What's in something a search found: an artist's songs, an album's or a playlist's tracks
pub async fn found_tracks(f: &Found) -> Vec<Track> {
    match f {
        Found::Artist { name, sc, sp, yt, img } => {
            let a = Artist { name: name.clone(), img: img.clone(), sc: *sc, sp: sp.clone() };
            let yt = async {
                match yt { Some(id) => core::youtube::artist(id).await.map(|p| p.songs.iter().map(Track::youtube).collect()).unwrap_or_default(), None => Vec::new() }
            };
            let (mine, yt) = tokio::join!(artist_tracks(&a), yt);
            interleave(vec![mine, yt])
        }
        Found::Album { plat: "soundcloud", id, title, .. } => playlist_tracks(&Playlist { plat: "soundcloud", id: id.clone(), title: title.clone(), tracks: None, own: false }).await,
        // (your files' albums are picked out of the Library by the app: main.rs)
        Found::Album { plat: "local", .. } => Vec::new(),
        Found::Album { id, img, .. } => {
            let Ok(v) = core::spotify::spotify_api("GET".into(), format!("/albums/{id}/tracks?limit=50&market=from_token"), None).await else { return Vec::new() };
            // (an album's tracks come without the album: its cover is the album's)
            v["items"].as_array().cloned().unwrap_or_default().iter().filter_map(Track::spotify).map(|mut t| { if t.art.is_empty() { t.art = img.clone(); } t }).collect()
        }
        Found::List { plat: "youtube", id, .. } => core::youtube::playlist(id).await.map(|l| l.iter().map(Track::youtube).collect()).unwrap_or_default(),
        Found::List { plat, id, title, .. } => playlist_tracks(&Playlist { plat, id: id.clone(), title: title.clone(), tracks: None, own: false }).await,
    }
}

/// Whether the catalogues are searched too (Settings: the web app's prefs.catOn, on unless switched off)
pub static CATALOG: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// Whether MusicBrainz is searched too (Settings: the web app's prefs.mbOn, on unless switched off)
pub static MUSICBRAINZ: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The words of a search or a title, as the web app's reqWords: lower case, accents off, Korean and
/// Japanese and Chinese kept, single letters dropped (but not single digits)
pub fn words(s: &str) -> Vec<String> {
    core::ranking::words(s)
}

/// Best match first, as the web app's searchRanker (core::ranking)
pub fn rank(q: &str, list: Vec<Track>) -> Vec<Track> {
    core::ranking::rank_by(q, list, |t| (t.title.as_str(), t.artist.as_str(), t.plays))
}

pub fn interleave(lists: Vec<Vec<Track>>) -> Vec<Track> {
    core::ranking::interleave(lists)
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

/// "%2F" and "+" back to what they stand for (a link's query values)
pub fn urldecode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'+' => out.push(b' '),
            b'%' if i + 2 < b.len() => {
                let hex = |c: u8| (c as char).to_digit(16);
                match (hex(b[i + 1]), hex(b[i + 2])) {
                    (Some(h), Some(l)) => { out.push((h * 16 + l) as u8); i += 2; }
                    _ => out.push(b'%'),
                }
            }
            c => out.push(c),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

pub fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod rank_tests {
    use super::*;
    fn t(title: &str, artist: &str, plays: u64) -> Track { Track { title: title.into(), artist: artist.into(), plays, ..Default::default() } }

    #[test]
    fn best_match_first_as_the_web_app() {
        let got = rank("hello", vec![t("허각 - Hello", "huh gak", 3_840_000), t("Hello Hello", "Wajid", 3_400_000), t("Hello", "colby!", 291_000), t("Hello", "Adele", 95_000_000)]);
        let names: Vec<&str> = got.iter().map(|x| x.artist.as_str()).collect();
        assert_eq!(names, ["Adele", "colby!", "huh gak", "Wajid"]);
        let got = rank("ginsu ghosts n stuff", vec![t("Ghosts 'n' Stuff", "deadmau5", 90_000_000), t("Ginsu × Ghosts 'n' Stuff × Goodbye", "Tritonal", 200_000)]);
        assert_eq!(got[0].artist, "Tritonal");
        // an artist's name: their most played, not a little-known song named after them
        let got = rank("deadmau5", vec![t("deadmau5", "Someone", 0), t("Strobe", "deadmau5", 50_000_000)]);
        assert_eq!(got[0].title, "Strobe");
        // twice the same song shows once; a remix of it isn't the same
        let mut a = t("My Pet Coelacanth", "deadmau5", 1); a.ms = 496_000;
        let mut b = t("My Pet Coelacanth", "deadmau5", 1); b.ms = 497_000;
        let mut c = t("My Pet Coelacanth (Remix)", "deadmau5", 1); c.ms = 300_000;
        assert_eq!(once_each(vec![a, b, c]).len(), 2);
        assert_eq!(words("Café n Stuff 2"), ["cafe", "stuff", "2"]);
    }
}
