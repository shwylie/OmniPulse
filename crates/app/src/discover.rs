// Discover, as the web app's (dist/js/feed.js and js/catalog.js): shelves of mixes, stations and
// tracks, in its order. New releases first (Deezer's albums from your artists, new tracks from who you
// follow), then the app's own Made for you mixes and artist stations, New for you, your subgenres,
// Trending by genre, and Deezer's top songs and popular artists last. Before there's anything to go
// on (no likes, favourites, plays or genres picked), the genres come first.
//
// The mixes are seeded by your SoundCloud likes, or (with none) by what you star and play here. Songs
// from Deezer's catalogue play from YouTube Music (Src::Catalog, crate::versions).

use crate::track::{Liked, Track};
use omnipulse_core as core;
use serde_json::Value;
use std::collections::HashMap;

/// Genres for Trending by genre (the web app's GENRES), with Deezer's chart for each
pub const GENRES: [(&str, u64); 12] = [("Electronic", 106), ("House", 113), ("Techno", 106), ("Dubstep", 106), ("Drum & Bass", 106), ("Hip-hop & Rap", 116),
    ("Pop", 132), ("Indie", 85), ("R&B", 165), ("Ambient", 106), ("Lo-fi", 106), ("Rock", 152)];

/// Genres you can pick (the web app's TASTE_GENRES), by Deezer's ids
pub const TASTE_GENRES: [(&str, u64); 21] = [("Pop", 132), ("Hip-hop & Rap", 116), ("Rock", 152), ("Dance", 113), ("Electronic", 106), ("R&B", 165),
    ("Alternative & Indie", 85), ("Soul & Funk", 169), ("Jazz", 129), ("Country", 84), ("Folk", 466), ("Metal", 464),
    ("Latin", 197), ("Reggae", 144), ("Blues", 153), ("Classical", 98), ("K-pop & Asian", 16), ("African", 2),
    ("Brazilian", 75), ("Indian", 81), ("Film & Game", 173)];

/// Each pickable genre's subgenres (the web app's SUBGENRES): made from Deezer's playlists of that name
pub fn subgenres(genre: &str) -> &'static [&'static str] {
    match genre {
        "Pop" => &["Synth-pop", "Dance Pop", "Indie Pop", "Hyperpop", "80s Pop", "Teen Pop", "Electropop"],
        "Hip-hop & Rap" => &["Trap", "Boom Bap", "Drill", "Conscious Hip-hop", "Lo-fi Hip-hop", "Old School Hip-hop", "UK Rap", "Cloud Rap"],
        "Rock" => &["Classic Rock", "Indie Rock", "Punk", "Pop Punk", "Grunge", "Hard Rock", "Prog Rock", "Emo", "Post-rock"],
        "Dance" => &["House", "Deep House", "Tech House", "Nu-disco", "EDM", "Eurodance", "Big Room", "UK Garage"],
        "Electronic" => &["Dubstep", "Electro House", "Drum & Bass", "Techno", "Trance", "Future Bass", "Melodic Bass", "Riddim",
            "Synthwave", "Hardstyle", "Glitch Hop", "Liquid DnB", "Chillstep", "Ambient", "Lo-fi", "IDM", "Breakbeat"],
        "R&B" => &["Neo-soul", "Contemporary R&B", "90s R&B", "Alternative R&B", "Slow Jams"],
        "Alternative & Indie" => &["Shoegaze", "Dream Pop", "Post-punk", "Indie Folk", "Bedroom Pop", "Math Rock", "Garage Rock"],
        "Soul & Funk" => &["Motown", "Funk", "Northern Soul", "Disco", "Neo-soul"],
        "Jazz" => &["Smooth Jazz", "Bebop", "Jazz Fusion", "Vocal Jazz", "Nu Jazz", "Swing", "Acid Jazz"],
        "Country" => &["Classic Country", "Country Pop", "Americana", "Bluegrass", "Outlaw Country"],
        "Folk" => &["Indie Folk", "Celtic", "Singer-songwriter", "Folk Rock"],
        "Metal" => &["Metalcore", "Death Metal", "Black Metal", "Thrash Metal", "Nu Metal", "Power Metal", "Djent", "Doom Metal"],
        "Latin" => &["Reggaeton", "Salsa", "Bachata", "Latin Trap", "Regional Mexican", "Cumbia", "Latin Pop"],
        "Reggae" => &["Dancehall", "Dub", "Roots Reggae", "Lovers Rock"],
        "Blues" => &["Delta Blues", "Chicago Blues", "Blues Rock", "Electric Blues"],
        "Classical" => &["Baroque", "Romantic", "Solo Piano", "Opera", "Contemporary Classical", "Minimalism"],
        "K-pop & Asian" => &["K-pop", "J-pop", "City Pop", "C-pop", "J-rock", "Anime Songs"],
        "African" => &["Afrobeats", "Amapiano", "Highlife", "Afro House"],
        "Brazilian" => &["Bossa Nova", "Samba", "MPB", "Funk Carioca", "Sertanejo"],
        "Indian" => &["Bollywood", "Punjabi", "Carnatic", "Indie Hindi"],
        "Film & Game" => &["Film Scores", "Video Game Music", "Epic Orchestral", "Anime Soundtracks"],
        _ => &[],
    }
}

fn taste_id(g: &str) -> Option<u64> {
    TASTE_GENRES.iter().find(|(n, _)| *n == g).map(|(_, id)| *id)
}

/// The genres you picked (prefs.tasteGenres), of those there are
pub fn taste_genres(prefs: &Value) -> Vec<String> {
    prefs["tasteGenres"].as_array().into_iter().flatten().filter_map(|g| g.as_str()).filter(|g| taste_id(g).is_some()).map(String::from).collect()
}

/// The subgenres you picked (prefs.tasteSubgenres), of the genres still picked
pub fn taste_subgenres(prefs: &Value) -> Vec<String> {
    let ok: Vec<&str> = taste_genres(prefs).iter().flat_map(|g| subgenres(g).iter().copied()).collect();
    prefs["tasteSubgenres"].as_array().into_iter().flatten().filter_map(|g| g.as_str()).filter(|g| ok.contains(g)).map(String::from).collect()
}

// ---------- what a tile is ----------
/// Someone a mix or station is made from: their name, what of theirs you like (on SoundCloud, to find
/// its related tracks) and what of theirs you know otherwise (starred, liked elsewhere, your files)
#[derive(Clone, Debug, Default)]
pub struct Seed {
    pub key: String,
    pub name: String,
    pub sc_likes: Vec<Track>,
    pub known: Vec<Track>,
    pub art: String,
    pub score: f64,
    /// a genre's artist (Artists from the genres you like): that genre
    pub genre: String,
}

#[derive(Clone, Debug)]
pub enum How {
    /// a Your Mix (crew of up to three) or a station (one)
    Crew(Vec<Seed>, bool),
    /// a subgenre: what Deezer's playlists of it agree on
    Sub(String),
    /// a genre of Trending by genre: the same
    Genre(String),
    /// one of Deezer's albums
    Album(u64),
}

#[derive(Clone, Debug)]
pub struct Mix {
    pub title: String,
    pub sub: String,
    pub art: String,
    pub plat: &'static str,
    pub desc: String,
    pub how: How,
}

#[derive(Clone, Debug)]
pub enum Item {
    Track(Track),
    Mix(Mix),
}

impl Item {
    /// (title, the line under it, its picture, the platform's icon)
    pub fn tile(&self) -> (String, String, String, &'static str) {
        match self {
            Item::Track(t) => (t.title.clone(), t.artist.clone(), t.art.clone(), t.plat()),
            Item::Mix(m) => (m.title.clone(), m.sub.clone(), m.art.clone(), m.plat),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Shelf {
    pub key: &'static str,
    pub title: String,
    pub items: Vec<Item>,
}

/// What Discover is made from
pub struct Input {
    /// your likes on every platform (and your files)
    pub library: Vec<Track>,
    pub favourites: Vec<Track>,
    /// what you've played: (artist, its cover, genre)
    pub plays: Vec<(String, String, String)>,
    pub prefs: Value,
    pub catalog: bool,
    pub soundcloud: bool,
}

impl Input {
    fn has_likes(&self) -> bool {
        !self.favourites.is_empty() || self.library.iter().any(|t| t.plat() != "local")
    }
    /// Nothing to go on yet: no likes, no favourites, nothing played, no genres picked
    pub fn cold(&self) -> bool {
        !self.has_likes() && self.plays.is_empty() && taste_genres(&self.prefs).is_empty()
    }
}

const FEED_ORDER: [&str; 11] = ["releases", "mixes", "stations", "taste", "foryou", "subgenres", "genres", "top", "popular", "spotlight", ""];
const COLD_ORDER: [&str; 5] = ["genres", "popular", "spotlight", "top", "foryou"];

fn order(shelves: &mut [Shelf], cold: bool) {
    let rank = |s: &Shelf| {
        let list: &[&str] = if cold { &COLD_ORDER } else { &FEED_ORDER };
        list.iter().position(|k| *k == s.key).unwrap_or(list.len())
    };
    shelves.sort_by_key(rank);   // (stable: those of one kind keep their order)
}

// ---------- Deezer ----------
async fn dz(path: &str) -> Value {
    core::catalog::deezer(path).await.unwrap_or_default()
}

fn dz_tracks(v: &Value, album: Option<&Value>) -> Vec<Track> {
    v["data"].as_array().into_iter().flatten().filter_map(|t| core::catalog::dz_track(t, album)).filter_map(|t| Track::catalog(&t)).collect()
}

pub async fn chart(genre: u64) -> Vec<Track> {
    dz_tracks(&dz(&format!("/chart/{genre}/tracks?limit=50")).await, None)
}

/// Deezer's artist of this name: of several so named, the one whose top songs include songs you have
/// by that name (none of them: left unmatched rather than guessed); with nothing to go on, the best
/// known (a common name's best known can be someone else entirely)
static DZ_ARTISTS: std::sync::Mutex<Option<HashMap<String, Option<(u64, String)>>>> = std::sync::Mutex::new(None);

/// The titles of the songs you have, by artist (artist_key -> title keys): what tells namesakes apart
static YOUR_TITLES: std::sync::Mutex<Option<HashMap<String, std::collections::HashSet<String>>>> = std::sync::Mutex::new(None);

/// A title as compared across platforms: no brackets, no " - Remastered"-style tail
fn title_key(s: &str) -> String {
    let mut out = String::new();
    let mut depth = 0;
    for c in s.chars() {
        match c { '(' | '[' => depth += 1, ')' | ']' => depth = (depth - 1).max(0), _ if depth == 0 => out.push(c), _ => {} }
    }
    let cut = out.find(" - ").map(|i| out[..i].to_string()).unwrap_or(out);
    core::ranking::artist_key(&cut)
}

fn your_titles_set(library: &[Track]) {
    let mut m: HashMap<String, std::collections::HashSet<String>> = HashMap::new();
    for t in library {
        let a = core::ranking::artist_key(t.artist.split(',').next().unwrap_or(""));
        let k = title_key(&t.title);
        if !a.is_empty() && !k.is_empty() { m.entry(a).or_default().insert(k); }
    }
    *YOUR_TITLES.lock().unwrap() = Some(m);
}

pub async fn dz_artist(name: &str) -> Option<(u64, String)> {
    let k = core::ranking::artist_key(name);
    if k.is_empty() { return None; }
    if let Some(hit) = DZ_ARTISTS.lock().unwrap().as_ref().and_then(|m| m.get(&k).cloned()) {
        return hit;
    }
    let found = dz(&format!("/search/artist?limit=10&q={}", crate::sources::urlencode(name))).await;
    let mut same: Vec<Value> = found["data"].as_array().into_iter().flatten()
        .filter(|x| x["name"].as_str().is_some_and(|n| core::ranking::artist_key(n) == k))
        .cloned().collect();
    same.sort_by_key(|x| std::cmp::Reverse(x["nb_fan"].as_u64().unwrap_or(0)));
    let pick = |x: &Value| Some((x["id"].as_u64()?, x["picture_big"].as_str().unwrap_or("").to_string()));
    let yours = YOUR_TITLES.lock().unwrap().as_ref().and_then(|m| m.get(&k).cloned()).unwrap_or_default();
    let best = if same.len() > 1 && !yours.is_empty() {
        let mut top = (0usize, None);
        for x in same.iter().take(6) {
            let Some(id) = x["id"].as_u64() else { continue };
            let hits = dz(&format!("/artist/{id}/top?limit=50")).await["data"].as_array().into_iter().flatten()
                .filter(|t| yours.contains(&title_key(t["title_short"].as_str().or(t["title"].as_str()).unwrap_or("")))).count();
            if hits > top.0 { top = (hits, pick(x)); }
        }
        top.1
    } else {
        same.first().and_then(pick)
    };
    DZ_ARTISTS.lock().unwrap().get_or_insert_with(HashMap::new).insert(k, best.clone());
    best
}

/// The best-known songs of artists like this one (Deezer's related artists), taking turns
pub async fn similar_tracks(name: &str) -> Vec<Track> {
    let Some((id, _)) = dz_artist(name).await else { return Vec::new() };
    let mut rel: Vec<u64> = dz(&format!("/artist/{id}/related?limit=8")).await["data"].as_array().into_iter().flatten().filter_map(|x| x["id"].as_u64()).collect();
    crate::queue::shuffle_in_place(&mut rel);
    let mut lists = Vec::new();
    for r in rel.into_iter().take(5) {
        lists.push(dz_tracks(&dz(&format!("/artist/{r}/top?limit=6")).await, None));
    }
    crate::sources::interleave(lists)
}

async fn artist_top(name: &str, n: usize) -> Vec<Track> {
    let Some((id, _)) = dz_artist(name).await else { return Vec::new() };
    dz_tracks(&dz(&format!("/artist/{id}/top?limit={n}")).await, None)
}

// ---------- a genre's music: what Deezer's playlists of it agree on ----------
// As the web app's (dzGenreMix): a playlist named after a genre can hold anything, but the genre's
// artists turn up in playlist after playlist of it. Up to eight playlists named for it (plainly named
// first: "House Essentials" before "Latin House"), the songs of artists in two or more of them (too
// few: the closest-named playlists' own), the best known first, at most two by one artist till the
// rest have had a turn. (Deezer's charts are a few broad genres: Techno, Dubstep and Drum & Bass were
// all its "Electro" chart.)
const GENRE_FILLER: [&str; 27] = ["essentials", "essential", "best", "of", "the", "hits", "top", "mix", "playlist", "classics", "songs", "music", "anthems", "bangers", "new", "latest", "greatest", "ultimate", "hot", "all", "time", "tracks", "selection", "selected", "official", "100", "50"];

fn genre_words(s: &str) -> Vec<String> {
    s.to_lowercase().replace('&', " and ").split(|c: char| !c.is_ascii_alphanumeric()).map(|w| if w == "n" { "and" } else { w })
        .filter(|w| !w.is_empty() && *w != "and").map(String::from).collect()
}

/// Of Deezer's playlists (title, id), the ones named for this genre, plainly named first (eight at most)
fn genre_playlists(name: &str, found: &[(String, u64)]) -> Vec<u64> {
    let want = genre_words(name);
    let year = |w: &str| w.len() == 4 && (w.starts_with("19") || w.starts_with("20")) && w.chars().all(|c| c.is_ascii_digit());
    let mut named: Vec<(usize, u64)> = found.iter().filter_map(|(title, id)| {
        let w = genre_words(title);
        if !want.iter().all(|v| w.contains(v)) { return None; }
        Some((w.iter().filter(|v| !want.contains(v) && !GENRE_FILLER.contains(&v.as_str()) && !year(v)).count(), *id))
    }).collect();
    named.sort_by_key(|(extra, _)| *extra);   // (stable: Deezer's order among the equally plain)
    named.into_iter().take(8).map(|(_, id)| id).collect()
}

/// The songs the playlists agree on, with how many have their artist; best known and most agreed first
fn genre_pick(lists: Vec<Vec<Track>>) -> Vec<Track> {
    let key = |t: &Track| core::ranking::artist_key(&first_artist(t));
    let mut votes: HashMap<String, usize> = HashMap::new();
    for l in &lists {
        let seen: std::collections::HashSet<String> = l.iter().map(key).collect();
        for a in seen { *votes.entry(a).or_default() += 1; }
    }
    let mut seen = std::collections::HashSet::new();
    let mut pool: Vec<Track> = lists.iter().flatten().filter(|t| votes.get(&key(t)).copied().unwrap_or(0) >= 2).filter(|t| seen.insert(t.key.clone())).cloned().collect();
    if pool.len() < 25 {
        pool.extend(lists.iter().take(2).flatten().filter(|t| seen.insert(t.key.clone())).cloned());
    }
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
    let mut chance = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x % 10_000) as f64 / 10_000.0 };
    let mut scored: Vec<(f64, Track)> = pool.into_iter().map(|t| {
        let v = votes.get(&key(&t)).copied().unwrap_or(1) as f64;
        ((1.0 + 0.5 * v) * (10.0 + t.plays as f64 / 100.0).log10() * (0.8 + chance() * 0.4), t)
    }).collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut per: HashMap<String, usize> = HashMap::new();
    let (mut first, mut later) = (Vec::new(), Vec::new());
    for (_, t) in scored {
        let n = per.entry(key(&t)).or_default();
        *n += 1;
        if *n <= 2 { first.push(t) } else { later.push(t) }
    }
    first.extend(later);
    first.truncate(80);
    first
}

/// A genre's (or subgenre's) music, as above
pub async fn subgenre_tracks(name: &str) -> Vec<Track> {
    let found = dz(&format!("/search/playlist?limit=25&q={}", crate::sources::urlencode(name))).await;
    let found: Vec<(String, u64)> = found["data"].as_array().into_iter().flatten()
        .filter(|p| p["nb_tracks"].as_u64().unwrap_or(0) >= 15)
        .filter_map(|p| Some((p["title"].as_str()?.to_string(), p["id"].as_u64()?))).collect();
    let mut lists = Vec::new();
    for id in genre_playlists(name, &found) {
        lists.push(dz_tracks(&dz(&format!("/playlist/{id}/tracks?limit=100")).await, None));
    }
    genre_pick(lists)
}

/// A mix made again (its page's refresh): a subgenre's or a genre's for its title (new music for it),
/// any other made afresh
pub async fn mix_again(m: &Mix, flags: (bool, bool, bool), liked: &Liked, avoid: &std::collections::HashSet<String>) -> Vec<Track> {
    match &m.how {
        // (drawn again from what the playlists agree on: none of what was showing, if there's enough)
        How::Sub(name) | How::Genre(name) => {
            let all = subgenre_tracks(name).await;
            let fresh: Vec<Track> = all.iter().filter(|t| !avoid.contains(&t.key)).cloned().collect();
            if fresh.len() >= 20 { fresh } else { all }
        }
        _ => mix_tracks(m, flags, liked).await,
    }
}

async fn album_tracks(id: u64) -> Vec<Track> {
    let al = dz(&format!("/album/{id}")).await;
    dz_tracks(&al["tracks"], Some(&al))
}

// ---------- your artists ----------
fn first_artist(t: &Track) -> String {
    t.artist.split([',', '&']).next().unwrap_or("").trim().to_string()
}

/// The artists you like on SoundCloud (they seed the app's mixes, as the web app's feedArtists), most
/// first; with none, the ones you star and play (a star counts as three plays; one play is enough)
pub fn your_artists(input: &Input) -> (Vec<Seed>, bool) {
    let mut by: HashMap<String, Seed> = HashMap::new();
    let mut add = |name: &str, w: f64, t: Option<&Track>, sc: bool, art: &str| {
        let k = core::ranking::artist_key(name);
        if k.is_empty() { return; }
        let s = by.entry(k.clone()).or_insert_with(|| Seed { key: k, name: name.to_string(), ..Default::default() });
        s.score += w;
        if let Some(t) = t {
            if sc { s.sc_likes.push(t.clone()); } else { s.known.push(t.clone()); }
            if s.art.is_empty() { s.art = t.art.clone(); }
        }
        if s.art.is_empty() { s.art = art.to_string(); }
    };
    let sc_seeded = input.library.iter().any(|t| t.plat() == "soundcloud" && !t.is_mix());
    if sc_seeded {
        for t in input.library.iter().filter(|t| !t.is_mix()) {
            add(&first_artist(t), 2.0, Some(t), t.plat() == "soundcloud", "");
        }
        for (a, art, _) in &input.plays { add(a.split(',').next().unwrap_or("").trim(), 1.0, None, false, art); }
        let mut list: Vec<Seed> = by.into_values().filter(|s| !s.sc_likes.is_empty()).collect();
        list.sort_by(|a, b| b.score.total_cmp(&a.score));
        return (list, true);
    }
    for t in &input.favourites { add(&first_artist(t), 3.0, Some(t), false, ""); }
    for t in input.library.iter().filter(|t| t.plat() == "youtube" || t.plat() == "spotify") { add(&first_artist(t), 2.0, Some(t), false, ""); }
    for (a, art, _) in &input.plays { add(a.split(',').next().unwrap_or("").trim(), 1.0, None, false, art); }
    let mut list: Vec<Seed> = by.into_values().collect();
    list.sort_by(|a, b| b.score.total_cmp(&a.score));
    (list, false)
}

/// New songs in the vein of these artists: SoundCloud's related tracks for what you like there,
/// Deezer's related artists (for mixes not seeded by SoundCloud), and YouTube Music's radio
// ---------- your taste: what Discover picks by ----------
// As the web app's (js/taste.js): the platforms only put songs forward; which make a mix, and in what
// order, is decided from what you like and listen to. An artist you like or play: up; one put forward
// by more than one platform: up; a genre you listen to: up; one you keep skipping: down; a little
// chance, so a mix made again differs; at most two by one artist near the top.
struct Taste { artists: HashMap<String, f64>, genres: HashMap<String, f64>, top_genre: f64, skips: HashMap<String, f64> }

static TASTE: std::sync::Mutex<Option<Taste>> = std::sync::Mutex::new(None);
const SKIPS_KEEP: u64 = 90 * 86_400;

fn skips_file() -> Option<std::path::PathBuf> {
    core::data_dir().ok().map(|d| d.join("state").join("native-skips.json"))
}

fn skips_load() -> Vec<(String, u64)> {
    let now = core::now_secs();
    let v: Value = skips_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null);
    v.as_array().into_iter().flatten()
        .filter_map(|x| Some((x["a"].as_str()?.to_string(), x["ts"].as_u64()?)))
        .filter(|(_, ts)| now.saturating_sub(*ts) < SKIPS_KEEP)
        .collect()
}

/// A song skipped past (Next, a media key) after a few seconds but before it counts as a play (30 s,
/// or half of it): its artist noted, for Discover (kept 90 days)
pub fn note_skip(t: &Track, pos_ms: u64, dur_ms: u64) {
    let counted = if dur_ms > 0 { (dur_ms / 2).min(30_000) } else { 30_000 };
    if t.is_mix() || pos_ms < 3000 || pos_ms >= counted { return; }
    let a = core::ranking::artist_key(&first_artist(t));
    if a.is_empty() { return; }
    let mut list = skips_load();
    list.push((a, core::now_secs()));
    let start = list.len().saturating_sub(2000);
    let v: Vec<Value> = list[start..].iter().map(|(a, ts)| serde_json::json!({ "a": a, "ts": ts })).collect();
    if let Some(p) = skips_file() {
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(p, Value::Array(v).to_string());
    }
}

fn taste_set(input: &Input) {
    let mut artists: HashMap<String, f64> = HashMap::new();
    let mut genres: HashMap<String, f64> = HashMap::new();
    let add = |m: &mut HashMap<String, f64>, k: String, w: f64| { if !k.is_empty() { *m.entry(k).or_default() += w; } };
    for t in input.library.iter().chain(&input.favourites).filter(|t| !t.is_mix()) {
        add(&mut artists, core::ranking::artist_key(&first_artist(t)), 2.0);
        add(&mut genres, core::ranking::artist_key(&t.genre), 2.0);
    }
    for (artist, _, genre) in &input.plays {
        add(&mut artists, core::ranking::artist_key(artist.split(',').next().unwrap_or("")), 1.0);
        add(&mut genres, core::ranking::artist_key(genre), 1.0);
    }
    let mut skips: HashMap<String, f64> = HashMap::new();
    for (a, _) in skips_load() { add(&mut skips, a, 1.0); }
    let top_genre = genres.values().cloned().fold(1.0, f64::max);
    *TASTE.lock().unwrap() = Some(Taste { artists, genres, top_genre, skips });
}

/// The platforms' lists (one each) made into one: best for you first, at most two by one artist until
/// the rest have had their turn
fn taste_pick(lists: Vec<Vec<Track>>) -> Vec<Track> {
    let key = |t: &Track| core::ranking::artist_key(&first_artist(t));
    let mut votes: HashMap<String, f64> = HashMap::new();
    for l in &lists {
        let seen: std::collections::HashSet<String> = l.iter().map(key).collect();
        for a in seen { *votes.entry(a).or_default() += 1.0; }
    }
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
    let mut chance = move || { x ^= x << 13; x ^= x >> 7; x ^= x << 17; (x % 10_000) as f64 / 10_000.0 };
    let guard = TASTE.lock().unwrap();
    let mut scored: Vec<(f64, Track)> = lists.into_iter().flatten().map(|t| {
        let a = key(&t);
        let mut s = 1.0 + 0.6 * (votes.get(&a).copied().unwrap_or(1.0) - 1.0).max(0.0);
        if let Some(p) = guard.as_ref() {
            let w = p.artists.get(&a).copied().unwrap_or(0.0);
            if w > 0.0 { s += 1.2 * (1.0 + w).log2(); }
            let g = p.genres.get(&core::ranking::artist_key(&t.genre)).copied().unwrap_or(0.0);
            if g > 0.0 { s += 0.8 * g / p.top_genre; }
            s /= 1.0 + 0.5 * p.skips.get(&a).copied().unwrap_or(0.0);
        }
        (s * (0.85 + chance() * 0.3), t)
    }).collect();
    drop(guard);
    scored.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut per: HashMap<String, usize> = HashMap::new();
    let (mut first, mut later) = (Vec::new(), Vec::new());
    for (_, t) in scored {
        let n = per.entry(key(&t)).or_default();
        *n += 1;
        if *n <= 2 { first.push(t) } else { later.push(t) }
    }
    first.extend(later);
    first
}

pub async fn fresh_for(crew: &[Seed], sc_seeded: bool, input_flags: (bool, bool, bool), liked: &Liked) -> Vec<Track> {
    let (catalog, youtube, soundcloud) = input_flags;
    let sc = async {
        if !soundcloud { return Vec::new(); }
        let mut seeds: Vec<Track> = crew.iter().flat_map(|a| a.sc_likes.iter().cloned()).collect();
        crate::queue::shuffle_in_place(&mut seeds);
        let mut out = Vec::new();
        for t in seeds.into_iter().take(4) { out.extend(crate::sources::related(&t).await); }
        out
    };
    let cat = async {
        if !catalog || sc_seeded { return Vec::new(); }
        let mut lists = Vec::new();
        for a in crew.iter().take(3) { lists.push(similar_tracks(&a.name).await); }
        crate::sources::interleave(lists)
    };
    let yt = async {
        if !youtube { return Vec::new(); }
        match crew.first() {
            Some(a) => match core::youtube::search(&a.name).await.ok().and_then(|l| l.into_iter().next()) {
                Some(first) => core::youtube::radio(&first.id).await.map(|l| l.iter().map(Track::youtube).collect()).unwrap_or_default(),
                None => Vec::new(),
            },
            None => Vec::new(),
        }
    };
    let (a, b, c) = tokio::join!(sc, cat, yt);
    let crew_keys: Vec<&str> = crew.iter().map(|s| s.key.as_str()).collect();
    let keep = |t: &Track| !t.is_mix() && !liked.has(t)
        // (the crew's own songs come in as the familiar ones, not as new)
        && (sc_seeded || !crew_keys.contains(&core::ranking::artist_key(&first_artist(t)).as_str()));
    // (the platforms put songs forward; which make it, and in what order, is yours)
    taste_pick([a, b, c].into_iter().map(|l| l.into_iter().filter(|t| keep(t)).collect()).collect())
}

/// A mix's or a station's tracks: mostly new music, with what you know of theirs woven in (two new
/// for each one known); a station with nothing of theirs known starts with a few of their best known
pub async fn crew_tracks(crew: &[Seed], station: bool, sc_seeded: bool, flags: (bool, bool, bool), liked: &Liked) -> Vec<Track> {
    let mut familiar: Vec<Track> = crew.iter().flat_map(|a| a.sc_likes.iter().chain(&a.known).cloned()).collect();
    crate::queue::shuffle_in_place(&mut familiar);
    familiar.truncate(if station { 4 } else { 12 });
    if station && familiar.is_empty() && flags.0 {
        if let Some(a) = crew.first() { familiar = artist_top(&a.name, 4).await; }
    }
    let fresh = fresh_for(crew, sc_seeded, flags, liked).await;
    let mut out = Vec::new();
    if station { out.extend(familiar.iter().cloned()); }
    let (mut i, mut k) = (0, 0);
    while i < fresh.len() || (!station && k < familiar.len()) {
        if i < fresh.len() { out.push(fresh[i].clone()); i += 1; }
        if i < fresh.len() { out.push(fresh[i].clone()); i += 1; }
        if !station && k < familiar.len() { out.push(familiar[k].clone()); k += 1; }
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|t| seen.insert(t.key.clone()));
    out.truncate(60);
    out
}

/// What a tile opens or plays
pub async fn mix_tracks(m: &Mix, flags: (bool, bool, bool), liked: &Liked) -> Vec<Track> {
    match &m.how {
        How::Crew(crew, station) => {
            let sc_seeded = crew.iter().any(|a| !a.sc_likes.is_empty());
            crew_tracks(crew, *station, sc_seeded, flags, liked).await
        }
        How::Genre(name) => subgenre_tracks(name).await,
        How::Sub(name) => subgenre_tracks(name).await,
        How::Album(id) => album_tracks(*id).await,
    }
}

// ---------- the shelves ----------
const RELEASE_DAYS: u64 = 45;

fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    civil(secs / 86_400)
}

/// A day number (since 1970) as YYYY-MM-DD
fn civil(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

fn days_ago(n: u64) -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    civil((secs / 86_400).saturating_sub(n))
}

/// Albums and singles from the last few weeks by the artists you listen to most (Deezer's), newest first
async fn new_releases(names: Vec<String>) -> Vec<(String, Mix)> {
    let since = days_ago(RELEASE_DAYS);
    let today = today();
    let mut out: Vec<(String, Mix)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for name in names.into_iter().take(30) {
        let Some((id, _)) = dz_artist(&name).await else { continue };
        let v = dz(&format!("/artist/{id}/albums?limit=10")).await;
        for a in v["data"].as_array().into_iter().flatten() {
            let date = a["release_date"].as_str().unwrap_or("");
            let Some(aid) = a["id"].as_u64() else { continue };
            if date < since.as_str() || date > today.as_str() || !seen.insert(aid) { continue; }
            let kind = match a["record_type"].as_str().unwrap_or("album") { "single" => "Single", "ep" => "EP", _ => "Album" };
            out.push((date.to_string(), Mix {
                title: a["title"].as_str().unwrap_or("").to_string(),
                sub: format!("{name} · {}{date}", if kind == "Album" { String::new() } else { format!("{kind} · ") }),
                art: a["cover_big"].as_str().unwrap_or("").to_string(),
                plat: "deezer",
                desc: format!("{kind} by {name}, out {date}."),
                how: How::Album(aid),
            }));
        }
    }
    out.sort_by(|a, b| b.0.cmp(&a.0));
    out
}

/// The artists most often in each picked genre's chart, the genres taking turns (Deezer's own genre
/// artist lists are the overall chart for every genre)
async fn taste_artists(genres: &[String]) -> Vec<Seed> {
    let mut lists = Vec::new();
    for g in genres {
        let Some(id) = taste_id(g) else { continue };
        let v = dz(&format!("/chart/{id}/tracks?limit=50")).await;
        let mut by: Vec<(usize, Seed)> = Vec::new();
        for t in v["data"].as_array().into_iter().flatten() {
            let Some(name) = t["artist"]["name"].as_str() else { continue };
            let k = core::ranking::artist_key(name);
            match by.iter_mut().find(|(_, s)| s.key == k) {
                Some(e) => e.0 += 1,
                None => by.push((1, Seed { key: k, name: name.to_string(), art: t["artist"]["picture_big"].as_str().unwrap_or("").to_string(), genre: g.clone(), ..Default::default() })),
            }
        }
        by.sort_by_key(|(n, _)| std::cmp::Reverse(*n));
        lists.push(by.into_iter().take(8).map(|(_, s)| s).collect::<Vec<_>>());
    }
    let mut out: Vec<Seed> = Vec::new();
    for i in 0..8 {
        for l in &lists {
            if let Some(s) = l.get(i) {
                if !out.iter().any(|o| o.key == s.key) { out.push(s.clone()); }
            }
        }
    }
    out
}

async fn popular_artists() -> Vec<Seed> {
    dz("/chart/0/artists?limit=20").await["data"].as_array().into_iter().flatten().filter_map(|x| {
        let name = x["name"].as_str()?;
        Some(Seed { key: core::ranking::artist_key(name), name: name.to_string(), art: x["picture_big"].as_str().unwrap_or("").to_string(), ..Default::default() })
    }).collect()
}

fn station(s: &Seed, flags_cat: bool, sc_seeded: bool) -> Item {
    let plat = if !s.sc_likes.is_empty() { "soundcloud" } else if flags_cat && !sc_seeded { "deezer" } else { "youtube" };
    Item::Mix(Mix {
        title: s.name.clone(),
        sub: if !s.genre.is_empty() { format!("{} station", s.genre) } else if s.score == 0.0 { "Popular right now".into() } else { "Artist station".into() },
        art: s.art.clone(),
        plat,
        desc: format!("Music like {}.", s.name),
        how: How::Crew(vec![s.clone()], true),
    })
}

/// Discover's shelves, in their order
pub async fn build(input: Input) -> Vec<Shelf> {
    your_titles_set(&input.library);
    taste_set(&input);
    let liked = Liked::of(&input.library);
    let cold = input.cold();
    let (artists, sc_seeded) = your_artists(&input);
    let mut shelves: Vec<Shelf> = Vec::new();

    // Made for you: Your Mix 01-06 (one of your top artists plus two more)
    let mains: Vec<Seed> = artists.iter().take(6).cloned().collect();
    let rest: Vec<Seed> = artists.iter().skip(6).cloned().collect();
    let mixes: Vec<Item> = mains.iter().enumerate().map(|(i, a)| {
        let mut crew = vec![a.clone()];
        crew.extend(rest.iter().skip(i * 2).take(2).cloned());
        let who = format!("{} and more", crew.iter().map(|s| s.name.as_str()).collect::<Vec<_>>().join(", "));
        Item::Mix(Mix {
            title: format!("Your Mix {:02}", i + 1),
            sub: who.clone(),
            art: a.art.clone(),
            plat: if sc_seeded { "soundcloud" } else if input.catalog { "deezer" } else { "youtube" },
            desc: format!("{who}: {}, mixed with new ones.", if sc_seeded { "tracks you like" } else { "songs you play" }),
            how: How::Crew(crew, false),
        })
    }).collect();
    if !mixes.is_empty() { shelves.push(Shelf { key: "mixes", title: "Made for you".into(), items: mixes }); }

    // Artist stations: the next artists you like, a little shuffled each time
    let mut next: Vec<Seed> = artists.iter().skip(6).take(24).cloned().collect();
    crate::queue::shuffle_in_place(&mut next);
    let st: Vec<Item> = next.iter().take(8).map(|s| station(s, input.catalog, sc_seeded)).collect();
    if !st.is_empty() { shelves.push(Shelf { key: "stations", title: "Artist stations".into(), items: st }); }

    let names: Vec<String> = artists.iter().map(|s| s.name.clone()).collect();
    let yours: std::collections::HashSet<String> = artists.iter().map(|s| s.key.clone()).collect();
    let picked = taste_genres(&input.prefs);
    let subs = taste_subgenres(&input.prefs);

    // the rest needs asking: all at once
    let releases = async { if input.catalog && !names.is_empty() { new_releases(names.clone()).await } else { Vec::new() } };
    let followed = async { if input.soundcloud { crate::sources::following_tracks(60).await } else { Vec::new() } };
    let for_you = async {
        if !input.catalog || names.is_empty() { return Vec::new(); }
        let mut seeds = names.iter().take(12).cloned().collect::<Vec<_>>();
        crate::queue::shuffle_in_place(&mut seeds);
        let mut lists = Vec::new();
        for s in seeds.into_iter().take(2) { lists.push(similar_tracks(&s).await); }
        crate::sources::interleave(lists)
    };
    let side = async {
        if !input.catalog || sc_seeded { return (Vec::new(), Vec::new()); }
        if !picked.is_empty() { (taste_artists(&picked).await, Vec::new()) } else { (Vec::new(), popular_artists().await) }
    };
    let top = async { if input.catalog { chart(0).await } else { Vec::new() } };
    let spotlight = async { if input.catalog && cold { chart(132).await } else { Vec::new() } };
    let (releases, followed, for_you, (taste, popular), top, spotlight) = tokio::join!(releases, followed, for_you, side, top, spotlight);

    // New releases: Deezer's albums and singles, and new tracks from who you follow, newest first
    let mut dated: Vec<(String, Item)> = releases.into_iter().map(|(d, m)| (d, Item::Mix(m))).collect();
    for t in followed.into_iter().filter(|t| !t.is_mix() && !liked.has(t)).take(30) { dated.push((t.date.clone(), Item::Track(t))); }
    dated.sort_by(|a, b| b.0.cmp(&a.0));
    if !dated.is_empty() { shelves.push(Shelf { key: "releases", title: "New releases".into(), items: dated.into_iter().map(|(_, i)| i).collect() }); }

    let more: Vec<Item> = for_you.into_iter().filter(|t| !t.is_mix() && !liked.has(t)).take(30).map(Item::Track).collect();
    if !more.is_empty() { shelves.push(Shelf { key: "foryou", title: "New for you".into(), items: more }); }

    let taste: Vec<Item> = taste.into_iter().filter(|s| !yours.contains(&s.key)).take(16).map(|s| station(&s, true, false)).collect();
    if !taste.is_empty() { shelves.push(Shelf { key: "taste", title: "Artists from the genres you like".into(), items: taste }); }
    let popular: Vec<Item> = popular.into_iter().filter(|s| !yours.contains(&s.key)).take(12).map(|s| station(&s, true, false)).collect();
    if !popular.is_empty() { shelves.push(Shelf { key: "popular", title: if cold { "Popular artists to start with".into() } else { "Popular artists".into() }, items: popular }); }

    // Your subgenres, then Trending by genre (the ones you picked first)
    if input.catalog && !subs.is_empty() {
        shelves.push(Shelf { key: "subgenres", title: "Your subgenres".into(), items: subs.iter().map(|g| Item::Mix(Mix {
            title: g.clone(), sub: "Mix".into(), art: String::new(), plat: "deezer", desc: format!("{g}, from what Deezer's playlists of it agree on."), how: How::Sub(g.clone()),
        })).collect() });
    }
    if input.catalog {
        let mut genres: Vec<(String, u64)> = picked.iter().filter(|g| !GENRES.iter().any(|(n, _)| n == g)).filter_map(|g| Some((g.clone(), taste_id(g)?))).collect();
        genres.extend(GENRES.iter().filter(|(n, _)| picked.iter().any(|p| p == n)).map(|(n, id)| (n.to_string(), *id)));
        genres.extend(GENRES.iter().filter(|(n, _)| !picked.iter().any(|p| p == n)).map(|(n, id)| (n.to_string(), *id)));
        shelves.push(Shelf { key: "genres", title: "Trending by genre".into(), items: genres.into_iter().map(|(g, _)| Item::Mix(Mix {
            title: g.clone(), sub: "Trending".into(), art: String::new(), plat: "deezer", desc: format!("Trending {g}, from what Deezer's playlists of it agree on."), how: How::Genre(g.clone()),
        })).collect() });
    }
    let top: Vec<Item> = top.into_iter().filter(|t| !liked.has(t)).take(30).map(Item::Track).collect();
    if !top.is_empty() { shelves.push(Shelf { key: "top", title: "Top songs".into(), items: top }); }
    let spot: Vec<Item> = spotlight.into_iter().filter(|t| !liked.has(t)).take(20).map(Item::Track).collect();
    if !spot.is_empty() { shelves.push(Shelf { key: "spotlight", title: "Pop spotlight".into(), items: spot }); }

    order(&mut shelves, cold);
    shelves
}

#[cfg(test)]
mod genre_tests {
    use super::*;
    #[test]
    fn plainly_named_playlists_first_and_only_ones_named_for_it() {
        let found: Vec<(String, u64)> = [("Latin House", 1), ("House Essentials", 2), ("Chill House 2026", 3), ("Party Hits", 4), ("House", 5)].iter().map(|(t, i)| (t.to_string(), *i)).collect();
        assert_eq!(genre_playlists("House", &found), [2, 5, 1, 3]);
        assert_eq!(genre_playlists("Drum & Bass", &[("Drum and Bass Workout".into(), 7), ("Drum n Bass".into(), 8), ("Bass".into(), 9)]), [8, 7]);
    }
    #[test]
    fn songs_the_playlists_agree_on() {
        let t = |a: &str, n: usize| Track { title: format!("{a}{n}"), artist: a.into(), key: format!("{a}{n}"), ..Default::default() };
        let lists = vec![vec![t("A", 1), t("B", 1), t("Odd", 1)], vec![t("A", 2), t("B", 2)], vec![t("A", 3), t("C", 1)]];
        let mut many: Vec<Vec<Track>> = lists.clone();
        many.extend((0..10).map(|i| vec![t("A", 10 + i), t("B", 10 + i)]));
        let out = genre_pick(many);
        assert!(out.iter().all(|x| x.artist != "Odd" && x.artist != "C"));
    }
}

#[cfg(test)]
mod taste_tests {
    use super::*;
    fn t(artist: &str, n: usize, genre: &str) -> Vec<Track> {
        (0..n).map(|i| Track { title: format!("{artist}{i}"), artist: artist.into(), genre: genre.into(), key: format!("{artist}{i}"), ..Default::default() }).collect()
    }
    #[test]
    fn your_artists_and_genres_first_skipped_last_two_each_at_most() {
        let mut artists = HashMap::new();
        artists.insert("fav".to_string(), 4.0);
        let mut genres = HashMap::new();
        genres.insert("dnb".to_string(), 3.0);
        let mut skips = HashMap::new();
        skips.insert("skipped".to_string(), 4.0);
        *TASTE.lock().unwrap() = Some(Taste { artists, genres, top_genre: 3.0, skips });
        let out = taste_pick(vec![[t("Random", 3, ""), t("Fav", 4, ""), t("Skipped", 2, "")].concat(), [t("Genre", 2, "dnb"), t("Random2", 2, "")].concat()]);
        let names: Vec<&str> = out.iter().map(|x| x.artist.as_str()).collect();
        assert_eq!(&names[..2], ["Fav", "Fav"]);
        assert_eq!(&names[2..4], ["Genre", "Genre"]);
        assert_eq!(names[..10].iter().filter(|a| **a == "Fav").count(), 2);
        assert_eq!(names.iter().position(|a| *a == "Skipped").unwrap(), 8);
        *TASTE.lock().unwrap() = None;
    }
}

#[cfg(test)]
mod title_key_tests {
    #[test]
    fn titles_compare_without_their_brackets_and_tails() {
        assert_eq!(super::title_key("Rain Falling Slowly (feat. X) - Remastered 2011"), super::title_key("Rain Falling Slowly [Live]"));
        assert_ne!(super::title_key("Rain"), super::title_key("Rain Falling Slowly"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_order() {
        assert_eq!(civil(0), "1970-01-01");
        assert_eq!(civil(20_371), "2025-10-10");
        let s = |key: &'static str| Shelf { key, title: key.into(), items: Vec::new() };
        let mut v = vec![s("top"), s("genres"), s("mixes"), s("releases"), s("foryou"), s("stations")];
        order(&mut v, false);
        assert_eq!(v.iter().map(|x| x.key).collect::<Vec<_>>(), ["releases", "mixes", "stations", "foryou", "genres", "top"]);
        order(&mut v, true);
        assert_eq!(v[0].key, "genres");
    }

    #[test]
    fn picked_genres_and_subgenres() {
        let p = serde_json::json!({ "tasteGenres": ["Rock", "Nope"], "tasteSubgenres": ["Grunge", "Dubstep"] });
        assert_eq!(taste_genres(&p), ["Rock"]);
        assert_eq!(taste_subgenres(&p), ["Grunge"]);   // (Dubstep's genre isn't picked)
    }
}

#[cfg(test)]
mod genre_live {
    /// cargo test -p omnipulse --bin OmniPulse-native genre_live -- --ignored --nocapture (the internet)
    #[test]
    #[ignore]
    fn genre_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for g in ["Drum & Bass", "Future Bass", "House"] {
            let l = rt.block_on(super::subgenre_tracks(g));
            let mut who: Vec<String> = Vec::new();
            for t in &l { if !who.contains(&t.artist) { who.push(t.artist.clone()); } }
            println!("{g}: {} songs; {}", l.len(), who.iter().take(14).cloned().collect::<Vec<_>>().join(", "));
            assert!(l.len() >= 20);
        }
    }
}
