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

// ---------- your Spotify likes: all of them, kept on this PC ----------
// As the web app keeps them: the whole library read once (paced), then only what's new asked for
// (newest first, until a like that's known: usually one request), and all of it again once a day for
// unlikes made in Spotify's own app; liking and unliking here change the copy at once. So whether a
// song is liked is known without asking Spotify (a rate limit can't make a liked one look unliked).
// Kept in state/native-spotify-likes.json, as tracks' snapshots.
const SP_LIKES_FULL_EVERY: u64 = 86_400;

fn sp_likes_file() -> Option<std::path::PathBuf> {
    core::data_dir().ok().map(|d| d.join("state").join("native-spotify-likes.json"))
}

fn sp_likes_load() -> (Vec<Track>, bool, u64) {
    let v: Value = sp_likes_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let list = v["tracks"].as_array().map(|a| a.iter().filter_map(Track::from_snapshot).collect()).unwrap_or_default();
    (list, v["complete"].as_bool().unwrap_or(false), v["full_at"].as_u64().unwrap_or(0))
}

fn sp_likes_save(list: &[Track], complete: bool, full_at: u64) {
    if let Some(p) = sp_likes_file() {
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        let v = serde_json::json!({ "tracks": list.iter().map(|t| t.to_snapshot()).collect::<Vec<_>>(), "complete": complete, "full_at": full_at });
        let _ = std::fs::write(p, v.to_string());
    }
}

/// Your likes a page at a time from the newest, until one in `known` (or the end); and whether it
/// got there (false: cut short, by a rate limit say: what it read is still given)
async fn sp_likes_read(known: Option<&std::collections::HashSet<String>>) -> (Vec<Track>, bool) {
    let mut out = Vec::new();
    let mut offset = 0;
    loop {
        let Ok(v) = core::spotify::spotify_api("GET".into(), format!("/me/tracks?limit=50&offset={offset}&market=from_token"), None).await else { return (out, false) };
        let page: Vec<Track> = v["items"].as_array().cloned().unwrap_or_default().iter().filter_map(|it| Track::spotify(&it["track"])).collect();
        let n = v["items"].as_array().map(|a| a.len()).unwrap_or(0);
        if let Some(k) = known {
            if let Some(i) = page.iter().position(|t| k.contains(&t.key)) {
                out.extend(page.into_iter().take(i));
                return (out, true);
            }
        }
        out.extend(page);
        if n < 50 { return (out, true); }
        offset += 50;
    }
}

/// Your likes from Spotify's own library service, through the playback sign-in (a free account's too):
/// every one, newest first, only the songs not kept yet read; the Web API's allowance (shared with
/// everyone signed in that way, often used up) untouched. None: not signed in that way, or it failed.
async fn sp_likes_library(kept: &[Track]) -> Option<Vec<Track>> {
    if !core::spotify_engine::spe_signed_in() { return None; }
    let known: Vec<String> = kept.iter().filter_map(|t| t.key.strip_prefix("sp:").map(String::from)).collect();
    let v = match core::spotify_library::spe_liked(known).await {
        Ok(v) => v,
        Err(e) => { core::diag::note(&format!("likes: Spotify's library failed: {e}: the Web API instead")); return None; }
    };
    let mut have: std::collections::HashMap<String, Track> = kept.iter().map(|t| (t.key.clone(), t.clone())).collect();
    for x in v["tracks"].as_array().cloned().unwrap_or_default() {
        if let Some(t) = Track::spotify(&x) { have.insert(t.key.clone(), t); }
    }
    Some(v["ids"].as_array().cloned().unwrap_or_default().iter().filter_map(|id| id.as_str()).filter_map(|id| have.remove(&format!("sp:{id}"))).collect())
}

pub async fn spotify_likes(_max: usize) -> Vec<Track> {
    if !core::spotify::spotify_signed_in() {
        return Vec::new();
    }
    let (mut list, mut complete, mut full_at) = sp_likes_load();
    let now = core::now_secs();
    if let Some(got) = sp_likes_library(&list).await {
        sp_likes_save(&got, true, now);
        return got;
    }
    if !complete || now.saturating_sub(full_at) > SP_LIKES_FULL_EVERY {
        let (got, whole) = sp_likes_read(None).await;
        if whole {
            list = got;
            complete = true;
            full_at = now;
        } else {
            // (cut short: what came back on top of what was kept)
            let keys: std::collections::HashSet<String> = got.iter().map(|t| t.key.clone()).collect();
            list.retain(|t| !keys.contains(&t.key));
            list.splice(0..0, got);
        }
    } else {
        let known: std::collections::HashSet<String> = list.iter().map(|t| t.key.clone()).collect();
        let (new, _) = sp_likes_read(Some(&known)).await;
        list.splice(0..0, new);
    }
    sp_likes_save(&list, complete, full_at);
    list
}

/// A like or unlike made here: in the copy at once
pub fn spotify_liked_note(t: &Track, on: bool) {
    let (mut list, complete, full_at) = sp_likes_load();
    list.retain(|x| x.key != t.key);
    if on { list.insert(0, t.clone()); }
    sp_likes_save(&list, complete, full_at);
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
    // (Spotify's own playlist service first, when signed in that way: the Web API's allowance untouched)
    let mut sp_done = false;
    if core::spotify::spotify_signed_in() && core::spotify_engine::spe_signed_in() {
        match core::spotify_library::spe_playlists().await {
            Ok(v) => {
                for p in v.as_array().cloned().unwrap_or_default() {
                    if let (Some(id), Some(title)) = (p["id"].as_str(), p["name"].as_str()) {
                        out.push(Playlist { plat: "spotify", id: id.to_string(), title: title.to_string(), tracks: None, own: p["own"].as_bool().unwrap_or(false) });
                    }
                }
                sp_done = true;
            }
            Err(e) => core::diag::note(&format!("playlists: Spotify's own service failed: {e}: the Web API instead")),
        }
    }
    if !sp_done && core::spotify::spotify_signed_in() {
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
        "spotify" if core::spotify_engine::spe_signed_in() => match core::spotify_engine::spe_playlist(format!("spotify:playlist:{}", p.id)).await {
            Ok(v) => v.as_array().cloned().unwrap_or_default().iter().filter_map(Track::spotify).collect(),
            Err(e) => {
                core::diag::note(&format!("playlist: Spotify's own service failed: {e}: the Web API instead"));
                sp_pages(format!("/playlists/{}/items", p.id), 1000, |it| Track::spotify(if it.get("item").is_some() { &it["item"] } else { &it["track"] })).await
            }
        },
        "spotify" => sp_pages(format!("/playlists/{}/items", p.id), 1000, |it| Track::spotify(if it.get("item").is_some() { &it["item"] } else { &it["track"] })).await,
        _ => Vec::new(),
    }
}

/// A search on every platform but the plugins (every one answers before anything shows), best match
/// first (rank); the plugins' results join after (search_plugins), as the web app's late feeds
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
    let (a, b, c, d, f) = tokio::join!(sc, sp, yt, mb, cat);
    once_each(rank(q, interleave(vec![a, b, c, d, f])))
}

/// Each running plugin that can search and play (crate::plugrun): slower (the Internet Archive), so
/// their results join the others once they're in
pub async fn search_plugins(q: &str) -> Vec<Track> {
    let plugs = futures_util::future::join_all(crate::plugrun::players().into_iter().map(|p| {
        let q = q.to_string();
        async move { p.call("search", vec![serde_json::json!(q)]).await.map(|v| v.as_array().cloned().unwrap_or_default().iter().take(200).filter_map(|x| Track::plugin(&p.id, x)).collect::<Vec<_>>()).unwrap_or_default() }
    })).await;
    interleave(plugs)
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
    Artist { name: String, img: String, sc: Option<u64>, sp: Option<String>, yt: Option<String>, followers: u64 },
    Album { plat: &'static str, id: String, title: String, by: String, img: String },
    List { plat: &'static str, id: String, title: String, by: String, img: String },
}

impl Found {
    /// (title, the line under it, its picture, the platform's icon)
    pub fn tile(&self) -> (String, String, String, &'static str) {
        match self {
            Found::Artist { name, img, sc, sp, followers, .. } => (name.clone(), if *followers > 0 { format!("Artist · {} followers", short_count(*followers)) } else { "Artist".into() }, img.clone(), if sp.is_some() { "spotify" } else if sc.is_some() { "soundcloud" } else { "youtube" }),
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
    let dz_on = CATALOG.load(std::sync::atomic::Ordering::Relaxed) && core::tools::ytdlp_path().is_some();
    let dz_path = format!("/search/artist?limit=10&q={enc}");
    let (sp_art, sp_alb, sp_pl, sc_users, sc_pl, yt_art, yt_pl, dz_art) = tokio::join!(
        sp("artist"), sp("album"), sp("playlist"),
        sc(format!("/users?q={enc}&limit=10")), sc(format!("/playlists?q={enc}&limit=8&linked_partitioning=true")),
        async { core::youtube::search_artists(q).await.unwrap_or_default() }, async { core::youtube::search_playlists(q).await.unwrap_or_default() },
        async { if dz_on { core::catalog::deezer(&dz_path).await.unwrap_or_default() } else { Value::Null } },
    );
    let img_of = |v: &Value| v["images"].as_array().and_then(|i| i.get(1).or(i.first())).and_then(|i| i["url"].as_str()).unwrap_or("").to_string();
    // artists: one per artist however their name is written, whichever platforms they're on (as the
    // web app's searchArtists): written as their most followed profile writes it, a real picture first
    struct A { name: String, img: String, sc: Option<u64>, sp: Option<String>, yt: Option<String>, followers: u64, best: i64, order: usize }
    let mut by: Vec<(String, A)> = Vec::new();
    let mut put = |name: &str, img: String, followers: u64, sc: Option<u64>, sp: Option<String>, yt: Option<String>| {
        let k = core::ranking::artist_key(name);
        if k.is_empty() { return; }
        let order = by.len();
        let at = match by.iter().position(|(key, _)| *key == k) {
            Some(i) => i,
            None => { by.push((k, A { name: name.to_string(), img: String::new(), sc: None, sp: None, yt: None, followers: 0, best: -1, order })); by.len() - 1 }
        };
        let a = &mut by[at].1;
        a.followers += followers;
        if followers as i64 > a.best { a.best = followers as i64; a.name = name.to_string(); }
        if !img.is_empty() && (a.img.is_empty() || (!real_image(&a.img) && real_image(&img))) { a.img = img; }
        if a.sc.is_none() { a.sc = sc; }
        if a.sp.is_none() { a.sp = sp; }
        if a.yt.is_none() { a.yt = yt; }
    };
    let sp_list = sp_art["artists"]["items"].as_array().cloned().unwrap_or_default();
    let sc_list = items(&sc_users);
    let dz_list = dz_art["data"].as_array().cloned().unwrap_or_default();
    for i in 0..10 {
        if let Some(a) = sp_list.get(i) { put(a["name"].as_str().unwrap_or(""), img_of(a), a["followers"]["total"].as_u64().unwrap_or(0), None, a["id"].as_str().map(String::from), None); }
        if let Some(u) = sc_list.get(i) { put(u["username"].as_str().unwrap_or(""), u["avatar_url"].as_str().unwrap_or("").replace("-large", "-t200x200"), u["followers_count"].as_u64().unwrap_or(0), u["id"].as_u64(), None, None); }
        if let Some(a) = yt_art.get(i) { put(&a.name, a.thumb.clone(), count_of(&a.subscribers), None, None, Some(a.id.clone())); }
        if let Some(a) = dz_list.get(i) { put(a["name"].as_str().unwrap_or(""), a["picture_big"].as_str().unwrap_or("").to_string(), a["nb_fan"].as_u64().unwrap_or(0), None, None, None); }
    }
    // the one you meant first: the name itself, then one with all the words; then a real picture, then
    // how many follow them everywhere together, then how many platforms they're on
    let want = words(q);
    let asked = want.join(" ");
    let rank = |a: &A| {
        let have = words(&a.name);
        let exact = if have.join(" ") == asked { 2 } else if want.iter().all(|w| have.contains(w)) { 1 } else { 0 };
        let plats = [a.sc.is_some(), a.sp.is_some(), a.yt.is_some()].iter().filter(|x| **x).count() as f64;
        (exact, real_image(&a.img) as u8, (1.0 + a.followers as f64).log10() + plats * 0.5)
    };
    let mut ranked: Vec<A> = by.into_iter().map(|(_, a)| a).collect();
    ranked.sort_by(|x, y| { let (a, b) = (rank(x), rank(y)); b.0.cmp(&a.0).then(b.1.cmp(&a.1)).then(b.2.total_cmp(&a.2)).then(x.order.cmp(&y.order)) });
    // (only those on a platform this app plays from: a Deezer-only artist opens nothing here)
    let artists: Vec<Found> = ranked.into_iter().filter(|a| a.sc.is_some() || a.sp.is_some() || a.yt.is_some()).take(10)
        .map(|a| Found::Artist { name: a.name, img: a.img, sc: a.sc, sp: a.sp, yt: a.yt, followers: a.followers }).collect();
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
        Found::Artist { name, sc, sp, yt, img, .. } => {
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

// ---------- following (the web app's js/artists.js) ----------
/// An artist of this name on a platform (its id), the best known of those so named
pub async fn find_artist_on(plat: &str, name: &str) -> Option<String> {
    let k = name_key(name);
    if k.is_empty() { return None; }
    match plat {
        "soundcloud" => {
            let v = core::soundcloud::api_get(format!("/search/users?q={}&limit=10", urlencode(name))).await.ok()?;
            items(&v).iter().filter(|u| name_key(u["username"].as_str().unwrap_or("")) == k)
                .max_by_key(|u| u["followers_count"].as_u64().unwrap_or(0))
                .and_then(|u| u["id"].as_u64()).map(|id| id.to_string())
        }
        "spotify" => {
            let v = core::spotify::spotify_api("GET".into(), format!("/search?type=artist&limit=10&q={}", urlencode(name)), None).await.ok()?;
            v["artists"]["items"].as_array()?.iter().find(|a| name_key(a["name"].as_str().unwrap_or("")) == k)
                .and_then(|a| a["id"].as_str()).map(String::from)
        }
        _ => None,
    }
}

/// Follows (or unfollows) an artist on a platform
pub async fn set_follow(plat: &str, id: &str, on: bool) -> Result<(), String> {
    match plat {
        "soundcloud" => core::soundcloud::set_follow(id.to_string(), on).await,
        // (since Spotify's February 2026 changes: /me/library, with the artist's uri)
        "spotify" => core::spotify::spotify_api(if on { "PUT" } else { "DELETE" }.into(), format!("/me/library?uris={}", urlencode(&format!("spotify:artist:{id}"))), None).await.map(|_| ()),
        _ => Err("not a platform you can follow on".into()),
    }
}

/// Follow Everywhere: artists newly followed on one platform (since the last look at it) followed on
/// the other too. A platform seen for the first time only has its follows noted (so a whole list never
/// counts as new); more than ten new at once are left alone. Kept in state/native-follow-seen.json.
/// What it followed, by name.
pub async fn follow_new_everywhere(list: &mut [Artist]) -> Vec<String> {
    let path = core::data_dir().ok().map(|d| d.join("state").join("native-follow-seen.json"));
    let seen: Value = path.as_ref().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    let plats: Vec<&str> = [core::soundcloud::signed_in().then_some("soundcloud"), core::spotify::spotify_signed_in().then_some("spotify")].into_iter().flatten().collect();
    let on = |a: &Artist, p: &str| if p == "soundcloud" { a.sc.is_some() } else { a.sp.is_some() };
    let was = |p: &str| seen[p].as_array().map(|l| l.iter().filter_map(|x| x.as_str().map(String::from)).collect::<std::collections::HashSet<String>>());
    let fresh: Vec<usize> = (0..list.len()).filter(|&i| plats.iter().any(|p| was(p).is_some_and(|w| on(&list[i], p) && !w.contains(&name_key(&list[i].name))))).collect();
    let mut done = Vec::new();
    if fresh.len() <= 10 {
        for i in fresh {
            for p in &plats {
                if on(&list[i], p) { continue; }
                let Some(id) = find_artist_on(p, &list[i].name).await else { continue };
                if set_follow(p, &id, true).await.is_ok() {
                    if *p == "soundcloud" { list[i].sc = id.parse().ok(); } else { list[i].sp = Some(id); }
                    done.push(list[i].name.clone());
                }
            }
        }
    }
    let mut now = if seen.is_object() { seen } else { serde_json::json!({}) };
    for p in &plats { now[*p] = serde_json::json!(list.iter().filter(|a| on(a, p)).map(|a| name_key(&a.name)).collect::<Vec<_>>()); }
    if let Some(p) = path { if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); } let _ = std::fs::write(p, now.to_string()); }
    done.sort();
    done.dedup();
    done
}

/// A picture of them, not a platform's stand-in for one (SoundCloud's default avatar)
fn real_image(url: &str) -> bool {
    !url.is_empty() && !url.contains("default_avatar") && !url.contains("avatars-default")
}

/// 1234567 as "1.2M"
pub fn short_count(n: u64) -> String {
    let f = n as f64;
    if f >= 1e9 { format!("{:.1}B", f / 1e9) } else if f >= 1e6 { format!("{:.1}M", f / 1e6) } else if f >= 1e3 { format!("{:.0}K", f / 1e3) } else { n.to_string() }
}

/// "1.2M subscribers" as a number
fn count_of(s: &str) -> u64 {
    let t = s.split_whitespace().next().unwrap_or("").replace(',', "");
    let (num, mul) = match t.chars().last() {
        Some('K') | Some('k') => (&t[..t.len() - 1], 1e3),
        Some('M') | Some('m') => (&t[..t.len() - 1], 1e6),
        Some('B') | Some('b') => (&t[..t.len() - 1], 1e9),
        _ => (t.as_str(), 1.0),
    };
    num.parse::<f64>().map(|n| (n * mul) as u64).unwrap_or(0)
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

#[cfg(test)]
mod spotify_likes_tests {
    use super::*;

    #[test]
    fn the_copy_is_kept_and_changed_by_likes_here() {
        let dir = std::env::temp_dir().join(format!("omnipulse-splikes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        core::set_data_dir(dir.clone());
        let sp = |n: u32| Track::spotify(&serde_json::json!({ "uri": format!("spotify:track:s{n}"), "id": format!("s{n}"), "name": format!("Song {n}"), "artists": [{ "name": "A" }], "album": { "name": "", "images": [] }, "duration_ms": 1000 })).unwrap();
        sp_likes_save(&[sp(1), sp(2)], true, 123);
        let (list, complete, full_at) = sp_likes_load();
        assert_eq!((list.iter().map(|t| t.key.clone()).collect::<Vec<_>>(), complete, full_at), (vec!["sp:s1".to_string(), "sp:s2".into()], true, 123));
        // liked here: newest first; unliked: gone; the rest as it was
        spotify_liked_note(&sp(3), true);
        spotify_liked_note(&sp(1), false);
        let (list, complete, _) = sp_likes_load();
        assert_eq!(list.iter().map(|t| t.key.clone()).collect::<Vec<_>>(), vec!["sp:s3".to_string(), "sp:s2".into()]);
        assert!(complete);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
