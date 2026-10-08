// OmniPulse, native (the rewrite, Phase 3): one window, one process. Pages for Discover, Rediscover,
// the Library, Favourites, Sets & Podcasts, Stats, Search, your playlists and Now Playing (with
// lyrics), played through omnipulse-audio. Built beside the web app, which keeps working until this
// one does everything; the web app's favourites, stats and playlists come from the copy it writes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod player;
mod sources;
mod track;
mod webstate;

use omnipulse_audio::{Engine, Event};
use slint::{ComponentHandle, Model, ModelRc, VecModel};
use sources::Playlist;
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use track::{fmt_ms, Track};
use webstate::WebState;

slint::include_modules!();

const MONTH_MS: u64 = 30 * 86_400_000;
const FILTER_CHIPS: [&str; 3] = ["All", "Tracks", "Sets & Podcasts"];
const PERIOD_CHIPS: [&str; 4] = ["This Week", "This Month", "This Year", "All Time"];

struct App {
    rt: tokio::runtime::Handle,
    engine: Arc<Engine>,
    ui: slint::Weak<AppWindow>,
    rows: Rc<VecModel<Row>>,
    qrows: Rc<VecModel<Row>>,
    lyric_lines: Rc<VecModel<Line>>,
    library: Vec<Track>,
    playlists: Vec<Playlist>,
    web: WebState,
    page: String,
    /// what the page lists, before its filter
    page_all: Vec<Track>,
    page_tracks: Vec<Track>,
    chip: usize,
    shelves: Vec<Vec<Track>>,
    queue: Vec<Track>,
    qi: Option<usize>,
    token: u64,
    lyrics: Vec<player::Line>,
    /// listening to this track: (its key, seconds heard, counted yet)
    heard: (String, f32, bool),
    last_pos: u64,
}

type Shared = Rc<RefCell<App>>;

fn now_ms() -> u64 {
    omnipulse_core::now_secs() * 1000
}

fn shuffle<T>(v: &mut [T]) {
    // a small xorshift, seeded by the clock: no dependency for something this simple
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

// ---------- pages ----------
fn apply_filter(app: &mut App) {
    let chip = app.chip;
    let filtered: Vec<Track> = match (app.page.as_str(), chip) {
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 1) => app.page_all.iter().filter(|t| !t.is_mix()).cloned().collect(),
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 2) => app.page_all.iter().filter(|t| t.is_mix()).cloned().collect(),
        _ => app.page_all.clone(),
    };
    let playing = app.qi.and_then(|i| app.queue.get(i)).map(|t| t.key.clone()).unwrap_or_default();
    app.rows.set_vec(filtered.iter().map(|t| t.row(t.key == playing)).collect::<Vec<_>>());
    app.page_tracks = filtered;
}

fn show_tracks(shared: &Shared, page: &str, title: &str, sub: String, tracks: Vec<Track>, chips: bool) {
    let mut app = shared.borrow_mut();
    let Some(ui) = app.ui.upgrade() else { return };
    app.page = page.to_string();
    app.page_all = tracks;
    app.chip = 0;
    ui.set_page(page.into());
    ui.set_page_kind("tracks".into());
    ui.set_page_title(title.into());
    ui.set_page_sub(sub.into());
    ui.set_chips(ModelRc::from(Rc::new(VecModel::from(if chips { FILTER_CHIPS.iter().map(|c| (*c).into()).collect() } else { Vec::new() }))));
    ui.set_chip_on(0);
    apply_filter(&mut app);
}

/// Something loaded in the background, for a page: shown if that page is still the one open
fn load_into(shared: &Shared, page: &'static str, title: String, fut: impl std::future::Future<Output = (Vec<Track>, String)> + Send + 'static) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let token = { let mut a = shared.borrow_mut(); a.token += 1; a.token };
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    rt.spawn(async move {
        let (tracks, sub) = fut.await;
        let _ = weak.upgrade_in_event_loop(move |_| {
            SHARED.with(|s| {
                if let Some(shared) = s.borrow().clone() {
                    if shared.borrow().token != token { return; }   // another page was opened meanwhile
                    show_tracks(&shared, page, &title, sub, tracks, page != "sets");
                }
            });
        });
    });
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

fn open_page(shared: &Shared, page: &str) {
    let ui = shared.borrow().ui.upgrade().unwrap();
    ui.set_side_on(-1);
    match page {
        "library" => {
            let lib = shared.borrow().library.clone();
            let sub = format!("Everything you've liked on each platform, plus your music files, in one list. {}.", plural(lib.len(), "track", "tracks"));
            show_tracks(shared, "library", "Library", sub, lib, true);
        }
        "favourites" => {
            let fav = shared.borrow().web.favourites.clone();
            let sub = if fav.is_empty() { "Starred tracks show here (from the web app, until this one can star them).".to_string() } else { format!("Starred tracks from every platform. {}.", plural(fav.len(), "track", "tracks")) };
            show_tracks(shared, "favourites", "Favourites", sub, fav, true);
        }
        "sets" => {
            // long ones from your likes, then from the artists you follow
            let mine: Vec<Track> = shared.borrow().library.iter().filter(|t| t.is_mix()).cloned().collect();
            show_tracks(shared, "sets", "Sets & Podcasts", "Long sets, mixes and podcasts: anything over 15 minutes.".into(), mine.clone(), false);
            load_into(shared, "sets", "Sets & Podcasts".into(), async move {
                let more: Vec<Track> = sources::following_tracks(300).await.into_iter().filter(|t| t.is_mix() && !mine.iter().any(|m| m.key == t.key)).collect();
                let n = mine.len() + more.len();
                let mut all = mine;
                all.extend(more);
                (all, format!("Long sets, mixes and podcasts: anything over 15 minutes. {}.", plural(n, "of them", "of them")))
            });
        }
        "rediscover" => {
            let (mut pool, lp) = { let a = shared.borrow(); (a.library.clone(), a.web.last_played.clone()) };
            let cut = now_ms().saturating_sub(MONTH_MS);
            pool.retain(|t| lp.get(&t.key).copied().unwrap_or(0) < cut);
            shuffle(&mut pool);
            pool.sort_by_key(|t| lp.get(&t.key).copied().unwrap_or(0));   // never played first
            let sub = if pool.is_empty() { "You've played all your likes recently.".to_string() } else { "Likes you haven't heard in a month or more, the longest-forgotten first.".to_string() };
            show_tracks(shared, "rediscover", "Rediscover", sub, pool, true);
        }
        "discover" => open_discover(shared),
        "stats" => open_stats(shared, 1),
        "now" => {
            let mut a = shared.borrow_mut();
            a.page = "now".into();
            ui.set_page("now".into());
            ui.set_page_kind("now".into());
            ui.set_chips(ModelRc::default());
        }
        _ => {}
    }
}

fn open_playlist(shared: &Shared, i: usize) {
    let Some(p) = shared.borrow().playlists.get(i).cloned() else { return };
    let ui = shared.borrow().ui.upgrade().unwrap();
    show_tracks(shared, "playlist", &p.title, "Loading...".into(), Vec::new(), true);
    ui.set_side_on(i as i32);
    let title = p.title.clone();
    let by = match p.plat { "spotify" => "On Spotify", "soundcloud" => "On SoundCloud", _ => "On this PC" };
    load_into(shared, "playlist", title, async move {
        let t = sources::playlist_tracks(&p).await;
        let sub = format!("{by}. {}.", plural(t.len(), "track", "tracks"));
        (t, sub)
    });
}

fn open_search(shared: &Shared, q: String) {
    let q = q.trim().to_string();
    if q.is_empty() {
        return;
    }
    show_tracks(shared, "search", &format!("Results for \"{q}\""), "Searching...".into(), Vec::new(), true);
    load_into(shared, "search", format!("Results for \"{q}\""), async move {
        let t = sources::search(&q).await;
        let sub = if t.is_empty() { "Nothing found.".into() } else { plural(t.len(), "track", "tracks") + " from SoundCloud, Spotify and YouTube Music." };
        (t, sub)
    });
}

// ---------- Discover: shelves ----------
fn track_from_play(p: &webstate::Play) -> Option<Track> {
    let src = match p.src.as_str() {
        "spotify" => track::Src::Spotify(format!("spotify:track:{}", p.id.trim_start_matches("sp:"))),
        "youtube" => track::Src::YouTube(p.id.trim_start_matches("yt:").to_string()),
        "local" => track::Src::Local(omnipulse_core::local::local_path_for(&p.id)?.to_string_lossy().into_owned()),
        _ => track::Src::SoundCloud(p.id.parse().ok()?),
    };
    Some(Track { src, key: p.id.clone(), title: p.title.clone(), artist: p.artist.clone(), album: String::new(), ms: p.ms, art: p.art.clone(), genre: p.genre.clone() })
}

fn top_lately(web: &WebState, library: &[Track], days: u64, n: usize) -> Vec<Track> {
    let cut = now_ms().saturating_sub(days * 86_400_000);
    let mut counts: HashMap<String, (usize, &webstate::Play)> = HashMap::new();
    for p in web.plays.iter().filter(|p| p.ts >= cut) {
        counts.entry(p.id.clone()).or_insert((0, p)).0 += 1;
    }
    let mut v: Vec<_> = counts.into_values().collect();
    v.sort_by_key(|x| std::cmp::Reverse(x.0));
    v.into_iter().filter_map(|(_, p)| library.iter().find(|t| t.key == p.id).cloned().or_else(|| track_from_play(p))).take(n).collect()
}

fn open_discover(shared: &Shared) {
    let ui = shared.borrow().ui.upgrade().unwrap();
    {
        let mut a = shared.borrow_mut();
        a.page = "discover".into();
        a.token += 1;
    }
    ui.set_page("discover".into());
    ui.set_page_kind("discover".into());
    ui.set_page_title("Discover".into());
    ui.set_page_sub("Mixes and stations made from what you like.".into());
    ui.set_chips(ModelRc::default());
    ui.set_shelves(ModelRc::default());
    let (library, top, forgotten, rt, weak, token) = {
        let a = shared.borrow();
        let top = top_lately(&a.web, &a.library, 30, 20);
        let cut = now_ms().saturating_sub(MONTH_MS);
        let mut f: Vec<Track> = a.library.iter().filter(|t| a.web.last_played.get(&t.key).copied().unwrap_or(0) < cut && !t.is_mix()).cloned().collect();
        shuffle(&mut f);
        f.truncate(20);
        (a.library.clone(), top, f, a.rt.clone(), a.ui.clone(), a.token)
    };
    rt.spawn(async move {
        // seeds for "Because you like ...": what you've played lately, or else your likes
        let mut seeds: Vec<Track> = top.iter().take(6).cloned().collect();
        if seeds.len() < 2 {
            let mut l: Vec<Track> = library.iter().filter(|t| !t.is_mix()).cloned().collect();
            shuffle(&mut l);
            seeds = l.into_iter().take(6).collect();
        }
        shuffle(&mut seeds);
        let s1 = seeds.first().cloned();
        let s2 = seeds.iter().find(|t| s1.as_ref().is_none_or(|a| a.artist != t.artist)).cloned();
        let rel = |s: Option<Track>| async move { match s { Some(t) => (format!("Because you like {}", t.artist), sources::related(&t).await), None => (String::new(), Vec::new()) } };
        let (follow, r1, r2) = tokio::join!(sources::following_tracks(60), rel(s1), rel(s2));
        let mut shelves: Vec<(String, Vec<Track>)> = Vec::new();
        let follow: Vec<Track> = follow.into_iter().filter(|t| !t.is_mix()).take(20).collect();
        if !follow.is_empty() { shelves.push(("New from artists you follow".into(), follow)); }
        if !r1.1.is_empty() { shelves.push((r1.0, r1.1.into_iter().take(20).collect())); }
        if !top.is_empty() { shelves.push(("Your top tracks lately".into(), top)); }
        if !r2.1.is_empty() { shelves.push((r2.0, r2.1.into_iter().take(20).collect())); }
        if !forgotten.is_empty() { shelves.push(("Rediscover".into(), forgotten)); }
        let arts: Vec<Vec<String>> = shelves.iter().map(|(_, l)| l.iter().map(|t| t.art.clone()).collect()).collect();
        let weak2 = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                if shared.borrow().token != token { return; }
                let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().map(|(_, l)| Rc::new(VecModel::from(l.iter().map(|t| Tile { title: t.title.clone().into(), sub: t.artist.clone().into(), art: slint::Image::default(), plat: t.plat().into() }).collect::<Vec<_>>()))).collect();
                let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|((title, _), m)| Shelf { title: title.clone().into(), tiles: ModelRc::from(m.clone()) }).collect();
                ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
                shared.borrow_mut().shelves = shelves.into_iter().map(|(_, l)| l).collect();
                SHELF_MODELS.with(|m| *m.borrow_mut() = models);
            });
        });
        // the covers, as they come
        for (s, list) in arts.into_iter().enumerate() {
            for (k, url) in list.into_iter().enumerate() {
                if let Some(px) = player::fetch_art(url, 200).await {
                    let _ = weak2.upgrade_in_event_loop(move |_| {
                        SHELF_MODELS.with(|m| {
                            if let Some(model) = m.borrow().get(s) {
                                if let Some(mut t) = model.row_data(k) { t.art = slint::Image::from_rgba8(px); model.set_row_data(k, t); }
                            }
                        });
                    });
                }
            }
        }
    });
}

// ---------- Stats ----------
fn open_stats(shared: &Shared, period: usize) {
    let ui = shared.borrow().ui.upgrade().unwrap();
    {
        let mut a = shared.borrow_mut();
        a.page = "stats".into();
        a.chip = period;
    }
    ui.set_page("stats".into());
    ui.set_page_kind("stats".into());
    ui.set_page_title("Your Listening".into());
    ui.set_page_sub("".into());
    ui.set_chips(ModelRc::from(Rc::new(VecModel::from(PERIOD_CHIPS.iter().map(|c| (*c).into()).collect::<Vec<slint::SharedString>>()))));
    ui.set_chip_on(period as i32);
    let a = shared.borrow();
    let now = now_ms();
    let since = match period {
        0 => now.saturating_sub(7 * 86_400_000),
        1 => now.saturating_sub(MONTH_MS),
        2 => now.saturating_sub(365 * 86_400_000),
        _ => 0,
    };
    let list: Vec<&webstate::Play> = a.web.plays.iter().filter(|p| p.ts >= since).collect();
    if list.is_empty() {
        ui.set_st_empty(true);
        ui.set_st_big("Nothing yet".into());
        ui.set_st_sub("Play some music and your stats will grow here.".into());
        ui.set_st_songs(ModelRc::default());
        ui.set_st_artists(ModelRc::default());
        return;
    }
    ui.set_st_empty(false);
    let ms: u64 = list.iter().map(|p| p.ms).sum();
    let h = ms as f64 / 3_600_000.0;
    ui.set_st_big((if h >= 10.0 { format!("{:.0} hours", h) } else if h >= 1.0 { format!("{h:.1} hours") } else { format!("{} minutes", ms / 60_000) }).into());
    let first_artist = |p: &webstate::Play| p.artist.split(',').next().unwrap_or("").trim().to_string();
    let mut songs: HashMap<String, (usize, &webstate::Play)> = HashMap::new();
    let mut artists: HashMap<String, (usize, &webstate::Play)> = HashMap::new();
    let mut plats: HashMap<String, usize> = HashMap::new();
    let mut hours = [0f32; 24];
    for p in &list {
        songs.entry(format!("{}|{}", p.title, p.artist)).or_insert((0, p)).0 += 1;
        artists.entry(first_artist(p)).or_insert((0, p)).0 += 1;
        *plats.entry(if p.src.is_empty() { "soundcloud".into() } else { p.src.clone() }).or_insert(0) += 1;
        let local_h = ((p.ts / 3_600_000) as i64 + local_offset_hours()).rem_euclid(24) as usize;
        hours[local_h] += 1.0;
    }
    let mut songs: Vec<_> = songs.into_values().collect();
    songs.sort_by_key(|x| std::cmp::Reverse(x.0));
    let mut artists: Vec<_> = artists.into_iter().collect();
    artists.sort_by_key(|x| std::cmp::Reverse(x.1 .0));
    ui.set_st_sub(format!("{} · {} · {} {}", plural(list.len(), "play", "plays"), plural(artists.len(), "artist", "artists"), plural(songs.len(), "song", "songs"), PERIOD_CHIPS[period].to_lowercase()).into());
    if let Some((name, (n, _))) = artists.first() {
        ui.set_st_top_name(name.clone().into());
        ui.set_st_top_sub(format!("{} · {}% of everything you played", plural(*n, "play", "plays"), n * 100 / list.len()).into());
    }
    let song_items: Vec<Item> = songs.iter().take(5).map(|(n, p)| Item { name: p.title.clone().into(), sub: format!("{} · {}", p.artist, plural(*n, "play", "plays")).into(), art: slint::Image::default(), plat: p.src.clone().into() }).collect();
    let artist_items: Vec<Item> = artists.iter().take(6).map(|(name, (n, p))| Item { name: name.clone().into(), sub: plural(*n, "play", "plays").into(), art: slint::Image::default(), plat: p.src.clone().into() }).collect();
    let song_model = Rc::new(VecModel::from(song_items));
    let artist_model = Rc::new(VecModel::from(artist_items));
    ui.set_st_songs(ModelRc::from(song_model.clone()));
    ui.set_st_artists(ModelRc::from(artist_model.clone()));
    // what kind of listener
    let share = |a: usize, b: usize| hours[a..b].iter().sum::<f32>();
    let total: f32 = hours.iter().sum::<f32>().max(1.0);
    let kinds = [
        ("Night Owl", "Most of your listening happens after 10 pm.", (share(22, 24) + share(0, 4)) / total),
        ("Early Bird", "You get going before 9 am.", share(5, 9) / total),
        ("Daytime Listener", "Music keeps your days going.", share(9, 17) / total),
        ("Evening Unwinder", "Your evenings have a soundtrack.", share(17, 22) / total),
    ];
    let k = kinds.iter().max_by(|a, b| a.2.total_cmp(&b.2)).unwrap();
    ui.set_st_kind(k.0.into());
    ui.set_st_kind_sub(k.1.into());
    let peak = hours.iter().cloned().fold(1.0, f32::max);
    ui.set_st_clock(ModelRc::from(Rc::new(VecModel::from(hours.iter().map(|v| v / peak).collect::<Vec<f32>>()))));
    // streak: days in a row, up to today
    let days: std::collections::HashSet<i64> = a.web.plays.iter().map(|p| ((p.ts / 3_600_000) as i64 + local_offset_hours()).div_euclid(24)).collect();
    let mut d = ((now / 3_600_000) as i64 + local_offset_hours()).div_euclid(24);
    if !days.contains(&d) { d -= 1; }
    let mut streak = 0;
    while days.contains(&d) { streak += 1; d -= 1; }
    ui.set_st_streak(plural(streak, "day", "days").into());
    ui.set_st_streak_sub("in a row".into());
    let mut plat_list: Vec<_> = plats.into_iter().collect();
    plat_list.sort_by_key(|x| std::cmp::Reverse(x.1));
    let pname = |p: &str| match p { "spotify" => "Spotify", "youtube" => "YouTube Music", "local" => "Your files", _ => "SoundCloud" };
    ui.set_st_plats(ModelRc::from(Rc::new(VecModel::from(plat_list.iter().map(|(p, n)| Item { name: pname(p).into(), sub: format!("{}%", n * 100 / list.len()).into(), art: slint::Image::default(), plat: p.clone().into() }).collect::<Vec<_>>()))));
    // a few facts, different each visit
    let mut facts: Vec<String> = Vec::new();
    let once = artists.iter().filter(|a| a.1 .0 == 1).count();
    if once > 2 { facts.push(format!("Explorer: {} got just one play.", plural(once, "artist", "artists"))); }
    let short = list.iter().filter(|p| p.ms > 0 && p.ms < 180_000).count();
    if list.len() > 10 { facts.push(format!("Short and sweet: {}% of your plays were under 3 minutes.", short * 100 / list.len())); }
    if let Some(first) = a.web.plays.first().filter(|p| now.saturating_sub(p.ts) >= 2 * 86_400_000) {
        let days_ago = (now.saturating_sub(first.ts)) / 86_400_000;
        facts.push(format!("Since the start: your listening here goes back {}.", plural(days_ago as usize, "day", "days")));
    }
    shuffle(&mut facts);
    facts.truncate(2);
    ui.set_st_facts(ModelRc::from(Rc::new(VecModel::from(facts.into_iter().map(Into::into).collect::<Vec<slint::SharedString>>()))));
    // covers
    let urls: Vec<String> = songs.iter().take(5).map(|(_, p)| p.art.clone()).collect();
    let aurls: Vec<String> = artists.iter().take(6).map(|(_, (_, p))| p.art.clone()).collect();
    let top_url = aurls.first().cloned().unwrap_or_default();
    let weak = a.ui.clone();
    drop(a);
    STAT_MODELS.with(|m| *m.borrow_mut() = Some((song_model, artist_model)));
    shared.borrow().rt.spawn(async move {
        for (i, u) in urls.into_iter().enumerate() {
            if let Some(px) = player::fetch_art(u, 96).await {
                let _ = weak.upgrade_in_event_loop(move |_| STAT_MODELS.with(|m| if let Some((s, _)) = m.borrow().as_ref() { if let Some(mut r) = s.row_data(i) { r.art = slint::Image::from_rgba8(px); s.set_row_data(i, r); } }));
            }
        }
        for (i, u) in aurls.into_iter().enumerate() {
            if let Some(px) = player::fetch_art(u, 160).await {
                let _ = weak.upgrade_in_event_loop(move |_| STAT_MODELS.with(|m| if let Some((_, s)) = m.borrow().as_ref() { if let Some(mut r) = s.row_data(i) { r.art = slint::Image::from_rgba8(px); s.set_row_data(i, r); } }));
            }
        }
        if let Some(px) = player::fetch_art(top_url, 200).await {
            let _ = weak.upgrade_in_event_loop(move |ui| ui.set_st_top_art(slint::Image::from_rgba8(px)));
        }
    });
}

/// This PC's offset from UTC, in hours (for the listening clock and streaks)
fn local_offset_hours() -> i64 {
    static OFF: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| {
        // Windows' own idea of the time zone, through PowerShell would be slow: read TZ bias from the environment if set,
        // otherwise compare the local and UTC clocks as the C runtime sees them
        std::env::var("OMNIPULSE_UTC_OFFSET").ok().and_then(|v| v.parse().ok()).unwrap_or_else(windows_offset)
    })
}

#[cfg(windows)]
fn windows_offset() -> i64 {
    #[repr(C)]
    struct SystemTime { y: u16, m: u16, dow: u16, d: u16, h: u16, min: u16, s: u16, ms: u16 }
    #[repr(C)]
    struct Tzi { bias: i32, sname: [u16; 32], sdate: SystemTime, sbias: i32, dname: [u16; 32], ddate: SystemTime, dbias: i32 }
    extern "system" { fn GetTimeZoneInformation(tzi: *mut Tzi) -> u32; }
    unsafe {
        let mut t: Tzi = std::mem::zeroed();
        let r = GetTimeZoneInformation(&mut t);
        let bias = t.bias + if r == 2 { t.dbias } else { t.sbias };
        -(bias as i64) / 60
    }
}

#[cfg(not(windows))]
fn windows_offset() -> i64 {
    0
}

// ---------- playing ----------
fn start(shared: &Shared) {
    let (t, token, engine, rt, weak) = {
        let mut a = shared.borrow_mut();
        let Some(i) = a.qi else { return };
        let Some(t) = a.queue.get(i).cloned() else { return };
        a.token += 1;
        a.heard = (t.key.clone(), 0.0, false);
        a.last_pos = 0;
        a.lyrics.clear();
        (t, a.token, a.engine.clone(), a.rt.clone(), a.ui.clone())
    };
    let ui = weak.upgrade().unwrap();
    ui.set_now_title(t.title.clone().into());
    ui.set_now_artist(t.artist.clone().into());
    ui.set_now_album(t.album.clone().into());
    ui.set_now_plat(t.plat().into());
    ui.set_now_art(slint::Image::default());
    ui.set_now_art_big(slint::Image::default());
    ui.set_status(format!("Loading \"{}\"...", t.title).into());
    ui.set_lyrics_note("Looking for lyrics...".into());
    ui.set_lyric_on(-1);
    {
        let a = shared.borrow();
        a.lyric_lines.set_vec(Vec::new());
        a.qrows.set_vec(a.queue.iter().enumerate().map(|(k, x)| x.row(Some(k) == a.qi)).collect::<Vec<_>>());
        for k in 0..a.rows.row_count() {
            if let (Some(mut r), Some(x)) = (a.rows.row_data(k), a.page_tracks.get(k)) {
                let on = x.key == t.key;
                if r.playing != on { r.playing = on; a.rows.set_row_data(k, r); }
            }
        }
    }
    let h = rt.clone();
    let weak2 = weak.clone();
    let t2 = t.clone();
    rt.spawn(async move {
        let src = player::open_source(&t, h.clone()).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                if shared.borrow().token != token { return; }
                match src {
                    Ok(src) => {
                        engine.load(src, 0.0);
                        engine.play();
                        ui.set_playing(true);
                        ui.set_status("".into());
                    }
                    Err(e) => ui.set_status(format!("Couldn't play \"{}\": {e}", t.title).into()),
                }
            });
        });
    });
    rt.spawn(async move {
        let art = player::fetch_art(t2.art.replace("-t200x200", "-t500x500"), 500).await;
        let lyrics = player::lyrics(&t2).await;
        let _ = weak2.upgrade_in_event_loop(move |ui| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                if shared.borrow().token < token { return; }
                if let Some(px) = art {
                    let img = slint::Image::from_rgba8(px);
                    ui.set_now_art(img.clone());
                    ui.set_now_art_big(img);
                }
                ui.set_lyrics_note(if lyrics.is_empty() { "No lyrics found for this one.".into() } else if lyrics.iter().all(|l| l.ms == 0) { "These lyrics aren't timed.".into() } else { "".into() });
                let a = &mut *shared.borrow_mut();
                a.lyric_lines.set_vec(lyrics.iter().map(|l| Line { text: l.text.clone().into() }).collect::<Vec<_>>());
                a.lyrics = lyrics;
            });
        });
    });
}

fn play_list(shared: &Shared, list: Vec<Track>, i: usize) {
    {
        let mut a = shared.borrow_mut();
        a.queue = list.into_iter().skip(i).take(500).collect();
        a.qi = Some(0);
    }
    start(shared);
}

fn step(shared: &Shared, d: i32) {
    let engine = shared.borrow().engine.clone();
    if d < 0 && engine.position_ms() > 3000 {
        engine.seek(0);
        return;
    }
    let next = {
        let a = shared.borrow();
        a.qi.map(|i| i as i32 + d).filter(|&n| n >= 0 && (n as usize) < a.queue.len())
    };
    match next {
        Some(n) => {
            shared.borrow_mut().qi = Some(n as usize);
            start(shared);
        }
        None if d > 0 => {
            engine.stop();
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_playing(false); }
        }
        None => {}
    }
}

// four times a second: the position, the lyric being sung, counting the play, the track's end
fn tick(shared: &Shared, events: &std::sync::mpsc::Receiver<Event>) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let engine = shared.borrow().engine.clone();
    let (pos, dur) = (engine.position_ms(), engine.duration_ms());
    ui.set_pos_text(fmt_ms(pos).into());
    ui.set_dur_text(fmt_ms(dur).into());
    ui.set_pos_frac(if dur > 0 { (pos as f32 / dur as f32).min(1.0) } else { 0.0 });
    {
        let mut a = shared.borrow_mut();
        if let Some(i) = a.lyrics.iter().rposition(|l| l.ms > 0 && l.ms <= pos + 200) {
            if ui.get_lyric_on() != i as i32 { ui.set_lyric_on(i as i32); }
        }
        // a play counts after 30 s, or half of a short track (as in the web app)
        if !engine.is_paused() && pos > a.last_pos && pos - a.last_pos < 2000 {
            a.heard.1 += (pos - a.last_pos) as f32 / 1000.0;
        }
        a.last_pos = pos;
        let need = if dur > 0 { (dur as f32 / 2000.0).min(30.0) } else { 30.0 };
        if !a.heard.2 && a.heard.1 >= need {
            a.heard.2 = true;
            if let Some(t) = a.qi.and_then(|i| a.queue.get(i)).cloned() {
                webstate::count_play(&t);
                a.web.last_played.insert(t.key.clone(), now_ms());
            }
        }
    }
    while let Ok(ev) = events.try_recv() {
        match ev {
            Event::Ended => step(shared, 1),
            Event::Error(e) => ui.set_status(e.into()),
        }
    }
}

fn main() -> Result<(), slint::PlatformError> {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
    // Slint's software renderer to start with: measured at 38 MB and ~3% of a core while playing,
    // against ~100 MB with the GPU one (most of it the graphics driver). SLINT_BACKEND picks another.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        let _ = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select();
    }
    let ui = AppWindow::new()?;
    let engine = Arc::new(Engine::new("").map_err(slint::PlatformError::Other)?);
    let events = engine.events().expect("events");
    engine.set_volume(0.8);
    let rows = Rc::new(VecModel::default());
    let qrows = Rc::new(VecModel::default());
    let lyric_lines = Rc::new(VecModel::default());
    ui.set_rows(ModelRc::from(rows.clone()));
    ui.set_queue_rows(ModelRc::from(qrows.clone()));
    ui.set_lyrics(ModelRc::from(lyric_lines.clone()));
    let shared: Shared = Rc::new(RefCell::new(App {
        rt: rt.handle().clone(), engine: engine.clone(), ui: ui.as_weak(), rows, qrows, lyric_lines,
        library: Vec::new(), playlists: Vec::new(), web: webstate::load(), page: "library".into(),
        page_all: Vec::new(), page_tracks: Vec::new(), chip: 0, shelves: Vec::new(), queue: Vec::new(), qi: None, token: 0,
        lyrics: Vec::new(), heard: (String::new(), 0.0, false), last_pos: 0,
    }));
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    ui.set_page_sub("Loading your Library...".into());

    // the Library and your playlists, in the background
    {
        let weak = ui.as_weak();
        let yt_likes = shared.borrow().web.yt_likes.clone();
        let own = shared.borrow().web.own_playlists.clone();
        rt.spawn(async move {
            let t0 = std::time::Instant::now();
            let (sc, sp, lists) = tokio::join!(sources::soundcloud_likes(), sources::spotify_likes(500), sources::playlists());
            let mut all: Vec<Track> = Vec::new();
            all.extend(sc);
            all.extend(sp);
            all.extend(yt_likes);
            all.extend(sources::local_files());
            let secs = t0.elapsed().as_secs_f32();
            let mut playlists = own;
            playlists.extend(lists);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                SHARED.with(|s| {
                    let Some(shared) = s.borrow().clone() else { return };
                    let sides: Vec<Side> = playlists.iter().map(|p| Side { title: p.title.clone().into(), plat: p.plat.into() }).collect();
                    ui.set_sides(ModelRc::from(Rc::new(VecModel::from(sides))));
                    {
                        let mut a = shared.borrow_mut();
                        a.library = all;
                        a.playlists = playlists;
                    }
                    if shared.borrow().page == "library" {
                        open_page(&shared, "library");
                        ui.set_page_sub(format!("{} Loaded in {secs:.1} s.", ui.get_page_sub()).into());
                    }
                });
            });
        });
    }

    let s = shared.clone();
    ui.on_nav(move |p| open_page(&s, &p));
    let s = shared.clone();
    ui.on_side(move |i| open_playlist(&s, i as usize));
    let s = shared.clone();
    ui.on_search(move |q| open_search(&s, q.into()));
    let s = shared.clone();
    ui.on_chip(move |i| {
        let page = s.borrow().page.clone();
        if page == "stats" {
            open_stats(&s, i as usize);
            return;
        }
        let ui = s.borrow().ui.upgrade().unwrap();
        ui.set_chip_on(i);
        let mut a = s.borrow_mut();
        a.chip = i as usize;
        apply_filter(&mut a);
    });
    let s = shared.clone();
    ui.on_play_row(move |i| {
        let list = s.borrow().page_tracks.clone();
        play_list(&s, list, i as usize);
    });
    let s = shared.clone();
    ui.on_tile(move |sh, k| {
        let list = s.borrow().shelves.get(sh as usize).cloned().unwrap_or_default();
        play_list(&s, list, k as usize);
    });
    let s = shared.clone();
    ui.on_lucky(move || {
        // a station from something you like, straight away
        let (seed, rt, weak) = {
            let a = s.borrow();
            let mut pool: Vec<Track> = a.library.iter().filter(|t| !t.is_mix()).cloned().collect();
            shuffle(&mut pool);
            (pool.into_iter().next(), a.rt.clone(), a.ui.clone())
        };
        let Some(seed) = seed else { return };
        if let Some(ui) = weak.upgrade() { ui.set_status(format!("Feeling lucky: a station from {}...", seed.artist).into()); }
        rt.spawn(async move {
            let mut list = vec![seed.clone()];
            list.extend(sources::related(&seed).await);
            let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| if let Some(sh) = s.borrow().clone() { play_list(&sh, list, 0); }));
        });
    });
    let s = shared.clone();
    ui.on_play_queue(move |i| {
        s.borrow_mut().qi = Some(i as usize);
        start(&s);
    });
    let s = shared.clone();
    ui.on_next(move || step(&s, 1));
    let s = shared.clone();
    ui.on_prev(move || step(&s, -1));
    ui.on_toggle({
        let engine = engine.clone();
        let weak = ui.as_weak();
        move || {
            let ui = weak.upgrade().unwrap();
            if engine.is_paused() { engine.play(); ui.set_playing(true); } else { engine.pause(); ui.set_playing(false); }
        }
    });
    ui.on_seek({ let engine = engine.clone(); move |f| { let d = engine.duration_ms(); if d > 0 { engine.seek((f.clamp(0.0, 1.0) as f64 * d as f64) as u64); } } });
    ui.on_volume_changed({ let engine = engine.clone(); move |v| engine.set_volume(v) });

    // the window: dragged by the title bar, its own buttons
    ui.on_drag({
        let weak = ui.as_weak();
        move || {
            use slint::winit_030::WinitWindowAccessor;
            let ui = weak.upgrade().unwrap();
            ui.window().with_winit_window(|w| { let _ = w.drag_window(); });
        }
    });
    ui.on_minimize({ let weak = ui.as_weak(); move || weak.upgrade().unwrap().window().set_minimized(true) });
    ui.on_maximize({
        let weak = ui.as_weak();
        move || {
            let ui = weak.upgrade().unwrap();
            let m = !ui.window().is_maximized();
            ui.window().set_maximized(m);
            ui.set_is_max(m);
        }
    });
    ui.on_close_window(|| { let _ = slint::quit_event_loop(); });

    let timer = slint::Timer::default();
    let s = shared.clone();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || tick(&s, &events));

    // OMNIPULSE_SELFTEST=<folder>: visits each page, plays the first track silently, saves a picture of
    // each page and quits (for checking the app without anyone at the screen)
    let _selftest = std::env::var("OMNIPULSE_SELFTEST").ok().map(|dir| {
        engine.set_volume(0.0);
        ui.set_volume(0.0);
        let t = slint::Timer::default();
        let s = shared.clone();
        let started = std::time::Instant::now();
        let mut step_no = 0usize;
        let plan: [(u64, &str); 9] = [(9, "play"), (13, "library"), (16, "discover"), (24, "rediscover"), (27, "sets"), (31, "stats"), (35, "now"), (39, "search"), (46, "end")];
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
            let Some(&(at, what)) = plan.get(step_no) else { return };
            if started.elapsed().as_secs() < at { return; }
            let ui = s.borrow().ui.upgrade().unwrap();
            // a picture of what the last step opened
            if step_no > 0 {
                let name = plan[step_no - 1].1;
                if let Ok(shot) = ui.window().take_snapshot() {
                    let _ = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec()).map(|i| i.save(format!("{dir}/{name}.png")));
                }
            }
            step_no += 1;
            match what {
                "play" => { if ui.get_rows().row_count() > 0 { ui.invoke_play_row(0); } }
                "search" => { ui.set_search_text("deadmau5".into()); ui.invoke_search("deadmau5".into()); }
                "end" => {
                    println!("selftest: library {} tracks, playlists {}, playing \"{}\" {} / {}, lyrics {}", s.borrow().library.len(), s.borrow().playlists.len(), ui.get_now_title(), ui.get_pos_text(), ui.get_dur_text(), s.borrow().lyrics.len());
                    let _ = slint::quit_event_loop();
                }
                p => ui.invoke_nav(p.into()),
            }
        });
        t
    });

    ui.run()?;
    drop(rt);
    Ok(())
}

thread_local! {
    static SHARED: RefCell<Option<Shared>> = const { RefCell::new(None) };
    static SHELF_MODELS: RefCell<Vec<Rc<VecModel<Tile>>>> = const { RefCell::new(Vec::new()) };
    #[allow(clippy::type_complexity)]
    static STAT_MODELS: RefCell<Option<(Rc<VecModel<Item>>, Rc<VecModel<Item>>)>> = const { RefCell::new(None) };
}
