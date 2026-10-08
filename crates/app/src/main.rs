// OmniPulse, native (the rewrite, Phase 3): one window, one process. Pages for Discover, Rediscover,
// the Library, Favourites, Sets & Podcasts, Stats, Search, your playlists and Now Playing (with
// lyrics), played through omnipulse-audio. Built beside the web app, which keeps working until this
// one does everything; the web app's favourites, stats and playlists come from the copy it writes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desktop;
mod integrations;
mod look;
mod player;
mod queue;
mod requests;
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
    queue: queue::Queue,
    /// the queue panel's rows: their place in the queue
    panel_map: Vec<usize>,
    /// the sidebar's playlists: their place in playlists (those of platforms switched off are left out)
    side_map: Vec<usize>,
    /// where you are, and where you've been (the header's back button)
    here: Option<Place>,
    history: Vec<Place>,
    /// Favourites only (the track list's pill)
    fav_only: bool,
    /// shuffle (the player bar's): on, and the upcoming tracks' order before it, to undo it
    shuffle_on: bool,
    shuffle_backup: Option<Vec<String>>,
    /// repeat: "off", "all" (the queue) or "one" (this track)
    repeat: String,
    autoplay: bool,
    ap_busy: bool,
    ap_then_play: bool,
    /// the track autoplay last came up empty for, so it doesn't keep asking
    ap_failed: Option<String>,
    /// a restored session: where to start its track when it's played
    resume_ms: Option<u64>,
    /// something has been loaded into the engine (not just restored)
    loaded: bool,
    /// the cover's colours, for the widget
    accent: (String, String),
    /// when this track started, for scrobbling
    started_secs: u64,
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

// ---------- pages ----------
fn apply_filter(app: &mut App) {
    let chip = app.chip;
    let prefs = app.web.prefs.clone();
    let favs: std::collections::HashSet<&str> = app.web.favourites.iter().map(|t| t.key.as_str()).collect();
    let fav_only = app.fav_only;
    let shown = |t: &&Track| plat_on(&prefs, t.plat()) && (!fav_only || favs.contains(t.key.as_str()));
    let filtered: Vec<Track> = match (app.page.as_str(), chip) {
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 1) => app.page_all.iter().filter(shown).filter(|t| !t.is_mix()).cloned().collect(),
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 2) => app.page_all.iter().filter(shown).filter(|t| t.is_mix()).cloned().collect(),
        _ => app.page_all.iter().filter(shown).cloned().collect(),
    };
    let playing = app.queue.current().map(|t| t.key.clone()).unwrap_or_default();
    let liked: std::collections::HashSet<&str> = app.library.iter().map(|t| t.key.as_str()).collect();
    app.rows.set_vec(filtered.iter().map(|t| Row { liked: liked.contains(t.key.as_str()), ..t.row(t.key == playing) }).collect::<Vec<_>>());
    app.page_tracks = filtered;
    // its covers, once this is done with the app
    slint::Timer::single_shot(std::time::Duration::ZERO, || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { rows_art(&sh); } }));
}

fn show_tracks(shared: &Shared, page: &str, title: &str, sub: String, mut tracks: Vec<Track>, chips: bool) {
    let mut app = shared.borrow_mut();
    track::apply_fixes(&app.web.prefs, &mut tracks);
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

// ---------- where you are: pages, playlists and searches, and the way back ----------
#[derive(Clone, PartialEq)]
enum Place {
    Page(String),
    Playlist(usize),
    Search(String),
}

/// Opens a place, remembering the one you were on (so Back returns to it)
fn go(shared: &Shared, place: Place) {
    {
        let mut a = shared.borrow_mut();
        if let Some(here) = a.here.take() {
            if here != place {
                a.history.push(here);
                let n = a.history.len();
                if n > 50 { a.history.drain(..n - 50); }
            }
        }
        a.here = Some(place.clone());
    }
    open_place(shared, &place);
}

fn go_back(shared: &Shared) {
    let Some(place) = shared.borrow_mut().history.pop() else { return };
    shared.borrow_mut().here = Some(place.clone());
    open_place(shared, &place);
}

fn open_place(shared: &Shared, place: &Place) {
    match place {
        Place::Page(p) => open_page(shared, p),
        Place::Playlist(i) => open_playlist(shared, *i),
        Place::Search(q) => open_search(shared, q.clone()),
    }
    let a = shared.borrow();
    if let Some(ui) = a.ui.upgrade() { ui.set_can_back(!a.history.is_empty()); }
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
            queue::shuffle_in_place(&mut pool);
            pool.sort_by_key(|t| lp.get(&t.key).copied().unwrap_or(0));   // never played first
            let sub = if pool.is_empty() { "You've played all your likes recently.".to_string() } else { "Likes you haven't heard in a month or more, the longest-forgotten first.".to_string() };
            show_tracks(shared, "rediscover", "Rediscover", sub, pool, true);
        }
        "discover" => open_discover(shared),
        "stats" => open_stats(shared, 1),
        "settings" => {
            let mut a = shared.borrow_mut();
            a.page = "settings".into();
            ui.set_page("settings".into());
            ui.set_page_kind("settings".into());
            ui.set_page_title("Settings".into());
            ui.set_page_sub("".into());
            ui.set_chips(ModelRc::default());
        }
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

// ---------- the platforms, each on or off (the title bar), as the web app's scOn, spOn, ytOn, localOn ----------
fn plat_key(plat: &str) -> &'static str {
    match plat {
        "spotify" => "spOn",
        "youtube" => "ytOn",
        "local" => "localOn",
        _ => "scOn",
    }
}

fn plat_on(prefs: &serde_json::Value, plat: &str) -> bool {
    integrations::on(prefs, plat_key(plat))
}

/// The platforms you have (signed in, or with files), for the title bar
fn paint_plats(a: &App) {
    let Some(ui) = a.ui.upgrade() else { return };
    let mut list: Vec<Plat> = Vec::new();
    let mut add = |id: &str, name: &str| list.push(Plat { id: id.into(), on: plat_on(&a.web.prefs, id), name: name.into() });
    if omnipulse_core::soundcloud::signed_in() { add("soundcloud", "SoundCloud"); }
    if omnipulse_core::spotify::spotify_signed_in() { add("spotify", "Spotify"); }
    add("youtube", "YouTube Music");
    if a.library.iter().any(|t| t.plat() == "local") { add("local", "Your files"); }
    ui.set_plats(ModelRc::from(Rc::new(VecModel::from(list))));
}

/// The sidebar's playlists: those of platforms switched off are left out
fn paint_sides(a: &mut App) {
    let Some(ui) = a.ui.upgrade() else { return };
    let prefs = a.web.prefs.clone();
    a.side_map = (0..a.playlists.len()).filter(|&i| plat_on(&prefs, a.playlists[i].plat) || a.playlists[i].plat == "local").collect();
    let sides: Vec<Side> = a.side_map.iter().map(|&i| Side { title: a.playlists[i].title.clone().into(), plat: a.playlists[i].plat.into() }).collect();
    ui.set_sides(ModelRc::from(Rc::new(VecModel::from(sides))));
}

fn plat_toggle(shared: &Shared, plat: &str) {
    let key = plat_key(plat);
    let now = {
        let mut a = shared.borrow_mut();
        let now = !plat_on(&a.web.prefs, plat);
        if let Some(o) = a.web.prefs.as_object_mut() {
            o.insert(key.into(), serde_json::json!(now));
        }
        paint_sides(&mut a);
        paint_plats(&a);
        apply_filter(&mut a);
        a.ap_failed = None;
        if let Some(ui) = a.ui.upgrade() {
            let name = match plat { "spotify" => "Spotify", "youtube" => "YouTube Music", "local" => "Your files", _ => "SoundCloud" };
            ui.set_status(format!("{name} is {}.", if now { "back" } else { "hidden everywhere" }).into());
        }
        now
    };
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() {
        webstate::save_pref(key, serde_json::json!(now));
    }
}

fn open_playlist(shared: &Shared, i: usize) {
    let Some(p) = shared.borrow().playlists.get(i).cloned() else { return };
    let ui = shared.borrow().ui.upgrade().unwrap();
    show_tracks(shared, "playlist", &p.title, "Loading...".into(), Vec::new(), true);
    ui.set_side_on(shared.borrow().side_map.iter().position(|&k| k == i).map_or(-1, |k| k as i32));
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
    Some(Track { src, key: p.id.clone(), title: p.title.clone(), artist: p.artist.clone(), album: String::new(), ms: p.ms, art: p.art.clone(), genre: p.genre.clone(), link: String::new() })
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
        queue::shuffle_in_place(&mut f);
        f.truncate(20);
        (a.library.clone(), top, f, a.rt.clone(), a.ui.clone(), a.token)
    };
    rt.spawn(async move {
        // seeds for "Because you like ...": what you've played lately, or else your likes
        let mut seeds: Vec<Track> = top.iter().take(6).cloned().collect();
        if seeds.len() < 2 {
            let mut l: Vec<Track> = library.iter().filter(|t| !t.is_mix()).cloned().collect();
            queue::shuffle_in_place(&mut l);
            seeds = l.into_iter().take(6).collect();
        }
        queue::shuffle_in_place(&mut seeds);
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
                let mut shelves = shelves;
                { let prefs = shared.borrow().web.prefs.clone(); shelves.iter_mut().for_each(|(_, l)| track::apply_fixes(&prefs, l)); }
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
    queue::shuffle_in_place(&mut facts);
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

// ---------- covers in the track list ----------
// Only the rows in view (and a few either side) get theirs, a few at a time, kept in a small cache,
// so a list of thousands stays light. The list says which rows are in view (rows-in-view).
const ART_KEEP: usize = 600;
const ART_AT_ONCE: usize = 8;

#[derive(Default)]
struct ArtCache {
    map: HashMap<String, slint::Image>,
    order: std::collections::VecDeque<String>,
    loading: std::collections::HashSet<String>,
    /// the rows in view, as the list last said
    view: (usize, usize),
}

/// A small cover's address (SoundCloud's are sized in the address)
fn thumb_url(t: &Track) -> String {
    t.art.replace("-t200x200", "-t120x120")
}

fn rows_art(shared: &Shared) {
    let a = shared.borrow();
    let (first, count) = ROW_ART.with(|c| c.borrow().view);
    let from = first.saturating_sub(5);
    let to = (first + count + 15).min(a.page_tracks.len()).min(a.rows.row_count());
    let mut want: Vec<String> = Vec::new();
    ROW_ART.with(|c| {
        let c = c.borrow();
        for k in from..to {
            let url = thumb_url(&a.page_tracks[k]);
            if url.is_empty() {
                continue;
            }
            match c.map.get(&url) {
                Some(img) => {
                    if let Some(mut r) = a.rows.row_data(k) {
                        if r.art.size().width == 0 {
                            r.art = img.clone();
                            a.rows.set_row_data(k, r);
                        }
                    }
                }
                None if !c.loading.contains(&url) && !want.contains(&url) => want.push(url),
                None => {}
            }
        }
    });
    let rt = a.rt.clone();
    let weak = a.ui.clone();
    drop(a);
    for url in want {
        let busy = ROW_ART.with(|c| {
            let mut c = c.borrow_mut();
            if c.loading.len() >= ART_AT_ONCE {
                return true;
            }
            c.loading.insert(url.clone());
            false
        });
        if busy {
            break; // the rest when these are in
        }
        let weak = weak.clone();
        rt.spawn(async move {
            let px = player::fetch_art(url.clone(), 80).await;
            let _ = weak.upgrade_in_event_loop(move |_| {
                ROW_ART.with(|c| {
                    let mut c = c.borrow_mut();
                    c.loading.remove(&url);
                    // a missing cover is remembered as an empty one, so it isn't asked for again
                    let img = px.map(slint::Image::from_rgba8).unwrap_or_default();
                    if c.map.insert(url.clone(), img).is_none() {
                        c.order.push_back(url);
                    }
                    while c.order.len() > ART_KEEP {
                        if let Some(old) = c.order.pop_front() {
                            c.map.remove(&old);
                        }
                    }
                });
                SHARED.with(|s| {
                    if let Some(sh) = s.borrow().clone() {
                        rows_art(&sh);
                    }
                });
            });
        });
    }
}

// ---------- the queue panel ----------
fn paint_queue(a: &mut App) {
    let panel = a.queue.panel();
    let map: Vec<usize> = panel.iter().map(|p| p.0).collect();
    let rows: Vec<Row> = panel
        .iter()
        .map(|(i, q, head)| {
            let mut r = q.t.row(Some(*i) == a.queue.qi);
            r.head = (*head).into();
            r.auto = q.auto;
            r
        })
        .collect();
    a.panel_map = map;
    a.qrows.set_vec(rows);
    if let Some(ui) = a.ui.upgrade() {
        ui.set_autoplay(a.autoplay);
        ui.set_autoplay_note(autoplay_note(a).into());
    }
}

fn autoplay_note(a: &App) -> String {
    if !a.autoplay {
        "Off: playback stops at the end of your queue.".into()
    } else if a.ap_busy {
        "Finding similar tracks...".into()
    } else if a.ap_failed.is_some() && a.ap_failed.as_deref() == a.queue.current().map(|t| t.key.as_str()) {
        "Autoplay couldn't find anything similar to this track.".into()
    } else {
        String::new()
    }
}

fn save_session(a: &App) {
    let pos = a.resume_ms.unwrap_or_else(|| a.engine.position_ms());
    let mut v = a.queue.to_session(pos);
    v["autoplay"] = serde_json::json!(a.autoplay);
    queue::save_session(&v);
}

// ---------- playing ----------
fn start(shared: &Shared) {
    let (t, token, engine, rt, weak, resume) = {
        let mut a = shared.borrow_mut();
        let Some(t) = a.queue.current().cloned() else { return };
        a.token += 1;
        a.heard = (t.key.clone(), 0.0, false);
        a.last_pos = 0;
        a.lyrics.clear();
        a.loaded = true;
        let resume = a.resume_ms.take().filter(|&ms| ms > 3000 && (t.ms == 0 || ms + 3000 < t.ms)).unwrap_or(0);
        paint_queue(&mut a);
        save_session(&a);
        (t, a.token, a.engine.clone(), a.rt.clone(), a.ui.clone(), resume)
    };
    let ui = weak.upgrade().unwrap();
    show_now(&ui, &t);
    ui.set_status(format!("Loading \"{}\"...", t.title).into());
    ui.set_lyrics_note("Looking for lyrics...".into());
    ui.set_lyric_on(-1);
    {
        let a = shared.borrow();
        a.lyric_lines.set_vec(Vec::new());
        for k in 0..a.rows.row_count() {
            if let (Some(mut r), Some(x)) = (a.rows.row_data(k), a.page_tracks.get(k)) {
                let on = x.key == t.key;
                if r.playing != on {
                    r.playing = on;
                    a.rows.set_row_data(k, r);
                }
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
                if shared.borrow().token != token {
                    return;
                }
                match src {
                    Ok(src) => {
                        engine.load(src, 0.0);
                        if resume > 0 {
                            engine.seek(resume);
                        }
                        engine.play();
                        ui.set_playing(true);
                        ui.set_status("".into());
                        {
                            let mut a = shared.borrow_mut();
                            a.started_secs = omnipulse_core::now_secs();
                            integrations::lastfm_now_playing(&a.rt, &a.web.prefs, &t);
                        }
                        push_out(&shared, true);
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
                if shared.borrow().token < token {
                    return;
                }
                if let Some(px) = art {
                    shared.borrow_mut().accent = integrations::accents(&px);
                    apply_look(&shared);
                    push_out(&shared, false);
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

/// The player bar's track, without playing it (a restored session shows it paused)
fn show_now(ui: &AppWindow, t: &Track) {
    ui.set_now_title(t.title.clone().into());
    ui.set_now_artist(t.artist.clone().into());
    ui.set_now_album(t.album.clone().into());
    ui.set_now_plat(t.plat().into());
    ui.set_now_art(slint::Image::default());
    ui.set_now_art_big(slint::Image::default());
}

/// Playing from a page: a list (Library, a playlist...) becomes the queue; one track from Search
/// takes the current one's place and leaves the rest of the queue as it was
fn play_list(shared: &Shared, list: Vec<Track>, i: usize) {
    {
        let mut a = shared.borrow_mut();
        if a.page == "search" {
            let Some(t) = list.get(i).cloned() else { return };
            a.queue.replace_current(t);
        } else {
            a.queue.set_list(list, i);
        }
        a.ap_failed = None;
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
        a.queue.qi.map(|i| i as i32 + d).filter(|&n| n >= 0 && (n as usize) < a.queue.items.len())
    };
    match next {
        Some(n) => {
            shared.borrow_mut().queue.qi = Some(n as usize);
            start(shared);
        }
        // the end of the queue with repeat on: back to the start
        None if d > 0 && shared.borrow().repeat == "all" && !shared.borrow().queue.items.is_empty() => {
            shared.borrow_mut().queue.qi = Some(0);
            start(shared);
        }
        // the end of the queue: autoplay adds some, then carries on
        None if d > 0 && !autoplay_fill(shared, true) => {
            engine.stop();
            shared.borrow_mut().loaded = false;
            if let Some(ui) = shared.borrow().ui.upgrade() {
                ui.set_playing(false);
            }
            push_out(shared, true);
        }
        None => {}
    }
}

// ---------- shuffle (the player bar's) ----------
// On: what's after this track, in a fair order (long-unheard first), keeping what you queued and
// autoplay's picks where they are. Off: back in the order it had, with anything added since after.
fn shuffle_toggle(shared: &Shared) {
    let mut a = shared.borrow_mut();
    a.shuffle_on = !a.shuffle_on;
    let from = a.queue.qi.map_or(0, |i| i + 1);
    if a.shuffle_on {
        a.shuffle_backup = Some(a.queue.items.iter().skip(from).map(|q| q.t.key.clone()).collect());
        let lp = a.web.last_played.clone();
        a.queue.shuffle_rest(&lp);
    } else if let Some(order) = a.shuffle_backup.take() {
        let place = |k: &str| order.iter().position(|o| o == k).unwrap_or(usize::MAX);
        if from <= a.queue.items.len() {
            a.queue.items[from..].sort_by_key(|q| place(&q.t.key));
        }
    }
    if let Some(ui) = a.ui.upgrade() {
        ui.set_shuffle_on(a.shuffle_on);
        ui.set_status((if a.shuffle_on { "Shuffle on." } else { "Shuffle off." }).into());
    }
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("shuffle", serde_json::json!(a.shuffle_on)); }
    paint_queue(&mut a);
    save_session(&a);
}

// ---------- autoplay ----------
fn pref_on(prefs: &serde_json::Value, key: &str) -> bool {
    prefs[key].as_bool() != Some(false)
}

/// Adds a batch of similar tracks after the queue (`then_play`: and moves on to them). False if
/// autoplay won't (off, or it came up empty for this track already).
fn autoplay_fill(shared: &Shared, then_play: bool) -> bool {
    let (seed, rt, weak, local, last_plat, taken, prefs, last_played) = {
        let mut a = shared.borrow_mut();
        let Some(seed) = a.queue.current().cloned() else { return false };
        if !a.autoplay || a.repeat != "off" || a.ap_failed.as_deref() == Some(seed.key.as_str()) {
            return false;
        }
        if a.ap_busy {
            a.ap_then_play |= then_play;
            return true;
        }
        a.ap_busy = true;
        a.ap_then_play = then_play;
        paint_queue(&mut a);
        let local: Vec<Track> = a.library.iter().filter(|t| t.plat() == "local").cloned().collect();
        let last_plat = a.queue.items.last().map(|q| q.t.plat()).unwrap_or("");
        let taken: std::collections::HashSet<String> = a.queue.items.iter().map(|q| q.t.key.clone()).collect();
        (seed, a.rt.clone(), a.ui.clone(), local, last_plat, taken, a.web.prefs.clone(), a.web.last_played.clone())
    };
    rt.spawn(async move {
        let want = |p: &str, key: &str| pref_on(&prefs, key) && (p != "local" || !local.is_empty());
        let (sc, sp, yt) = tokio::join!(
            async { if want("soundcloud", "scOn") { sources::autoplay_soundcloud(&seed).await } else { Vec::new() } },
            async { if want("spotify", "spOn") { sources::autoplay_spotify(&seed).await } else { Vec::new() } },
            async { if want("youtube", "ytOn") { sources::autoplay_youtube(&seed).await } else { Vec::new() } },
        );
        let lc = if want("local", "localOn") { sources::autoplay_local(&seed, &local) } else { Vec::new() };
        let lists = vec![("soundcloud", sc), ("spotify", sp), ("youtube", yt), ("local", lc)];
        // sets & podcasts only if you've asked for them; nothing heard lately (Avoid Recent Plays),
        // or if that leaves nothing, nothing from the last three hours
        let sets = prefs["queueSets"].as_bool().unwrap_or(false);
        let window = match prefs["recentWindow"].as_str() {
            Some("day") => 86_400_000,
            Some("month") => MONTH_MS,
            _ => 7 * 86_400_000,
        };
        let avoid = pref_on(&prefs, "avoidRecent");
        let now = now_ms();
        let ok = |within: u64| {
            let lp = &last_played;
            move |t: &Track| (sets || !t.is_mix()) && !(avoid && lp.get(&t.key).is_some_and(|&ts| now.saturating_sub(ts) < within))
        };
        let mut picks = queue::take_turns(lists.clone(), last_plat, &taken, ok(window));
        if picks.is_empty() && avoid {
            picks = queue::take_turns(lists, last_plat, &taken, ok(3 * 3_600_000));
        }
        let _ = weak.upgrade_in_event_loop(move |_| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                let go = {
                    let mut a = shared.borrow_mut();
                    a.ap_busy = false;
                    let then_play = std::mem::take(&mut a.ap_then_play);
                    // you started something else meanwhile, or switched autoplay off
                    if a.queue.current().map(|t| &t.key) != Some(&seed.key) || !a.autoplay {
                        paint_queue(&mut a);
                        return;
                    }
                    if picks.is_empty() {
                        a.ap_failed = Some(seed.key.clone());
                    }
                    let mut picks = picks;
                    track::apply_fixes(&a.web.prefs, &mut picks);
                    a.queue.items.extend(picks.into_iter().map(queue::QItem::auto));
                    paint_queue(&mut a);
                    save_session(&a);
                    then_play && a.queue.qi.is_some_and(|i| i + 1 < a.queue.items.len())
                };
                if go {
                    step(&shared, 1);
                }
            });
        });
    });
    true
}

// four times a second: the position, the lyric being sung, counting the play, autoplay, the
// session, the track's end
fn tick(shared: &Shared, events: &std::sync::mpsc::Receiver<Event>, n: u64) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let engine = shared.borrow().engine.clone();
    if !shared.borrow().loaded {
        return; // a restored session, not started yet: the bar shows where it was
    }
    let (pos, dur) = (engine.position_ms(), engine.duration_ms());
    ui.set_pos_text(fmt_ms(pos).into());
    ui.set_dur_text(fmt_ms(dur).into());
    ui.set_pos_frac(if dur > 0 { (pos as f32 / dur as f32).min(1.0) } else { 0.0 });
    let near_end = {
        let mut a = shared.borrow_mut();
        if let Some(i) = a.lyrics.iter().rposition(|l| l.ms > 0 && l.ms <= pos + 200) {
            if ui.get_lyric_on() != i as i32 {
                ui.set_lyric_on(i as i32);
            }
        }
        // a play counts after 30 s, or half of a short track (as in the web app)
        if !engine.is_paused() && pos > a.last_pos && pos - a.last_pos < 2000 {
            a.heard.1 += (pos - a.last_pos) as f32 / 1000.0;
        }
        a.last_pos = pos;
        let need = if dur > 0 { (dur as f32 / 2000.0).min(30.0) } else { 30.0 };
        if !a.heard.2 && a.heard.1 >= need {
            a.heard.2 = true;
            if let Some(t) = a.queue.current().cloned() {
                webstate::count_play(&t);
                a.web.last_played.insert(t.key.clone(), now_ms());
                integrations::lastfm_scrobble(&a.rt, &a.web.prefs, &t, a.started_secs);
            }
        }
        if n.is_multiple_of(20) && !engine.is_paused() {
            save_session(&a);
            let s = shared.clone();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || push_out(&s, false));
        }
        let last = a.queue.qi.is_some_and(|i| i + 1 >= a.queue.items.len());
        last && dur > 0 && dur.saturating_sub(pos) <= queue::AUTOPLAY_LEAD_MS && !a.ap_busy
    };
    if near_end {
        autoplay_fill(shared, false);
    }
    while let Ok(ev) = events.try_recv() {
        match ev {
            Event::Ended if shared.borrow().repeat == "one" => start(shared),
            Event::Ended => step(shared, 1),
            Event::Error(e) => ui.set_status(e.into()),
        }
    }
}

/// Where you left off: the queue, and the track paused where it was. Play carries on.
fn restore_session(shared: &Shared) {
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() {
        return; // the self-test starts from nothing
    }
    omnipulse_core::local::local_load_index();
    let Some((q, pos)) = queue::load_session() else { return };
    let autoplay = std::fs::read_to_string(omnipulse_core::data_dir().map(|d| d.join("state").join("native-session.json")).unwrap_or_default())
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|v| v["autoplay"].as_bool())
        .unwrap_or(true);
    let mut a = shared.borrow_mut();
    a.queue = q;
    a.autoplay = autoplay;
    a.resume_ms = Some(pos);
    paint_queue(&mut a);
    let Some(t) = a.queue.current().cloned() else { return };
    let Some(ui) = a.ui.upgrade() else { return };
    show_now(&ui, &t);
    ui.set_pos_text(fmt_ms(pos).into());
    ui.set_dur_text(fmt_ms(t.ms).into());
    ui.set_pos_frac(if t.ms > 0 { (pos as f32 / t.ms as f32).min(1.0) } else { 0.0 });
    let (rt, weak, art) = (a.rt.clone(), a.ui.clone(), t.art.replace("-t200x200", "-t500x500"));
    rt.spawn(async move {
        if let Some(px) = player::fetch_art(art, 500).await {
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let img = slint::Image::from_rgba8(px);
                ui.set_now_art(img.clone());
                ui.set_now_art_big(img);
            });
        }
    });
}

// ---------- the outside world: Discord, Last.fm, the OBS widget, the Stream Deck, chat requests ----------
/// Tells Discord (if `presence`) and the widget what's playing now
fn push_out(shared: &Shared, presence: bool) {
    let a = shared.borrow();
    let (pos, dur, playing) = (a.engine.position_ms(), a.engine.duration_ms(), a.loaded && !a.engine.is_paused());
    let t = a.queue.current();
    let prefs = &a.web.prefs;
    if integrations::set(prefs, "widget") || integrations::set(prefs, "req") {
        integrations::widget_update(prefs, t, pos, dur, playing, (&a.accent.0, &a.accent.1));
    }
    if presence {
        integrations::presence(&a.rt, prefs, t, pos, playing);
    }
    desktop::taskbar::state(playing, false, false, false);
}

/// The Stream Deck plugin's state, as remoteState in js/remote.js
fn remote_state(a: &App) -> serde_json::Value {
    let t = a.queue.current();
    serde_json::json!({
        "ok": true,
        "playing": a.loaded && !a.engine.is_paused(),
        "title": t.map(|t| t.title.clone()).unwrap_or_default(),
        "artist": t.map(|t| t.artist.clone()).unwrap_or_default(),
        "shuffle": false, "repeat": "off", "liked": false, "canLike": false, "favourite": false,
        "setsAllowed": a.web.prefs["queueSets"].as_bool().unwrap_or(false),
        "requests": a.queue.pending_requests().count(),
    })
}

/// A Stream Deck button (through core's remote server); answers with the state once it's settled
fn remote_action(shared: &Shared, id: u64, action: &str) {
    let mut note = None;
    match action {
        "play" => if !shared.borrow().loaded || shared.borrow().engine.is_paused() { toggle(shared) },
        "pause" => if shared.borrow().loaded && !shared.borrow().engine.is_paused() { toggle(shared) },
        "toggle" => toggle(shared),
        "next" => step(shared, 1),
        "previous" => step(shared, -1),
        "shuffle" => {
            let mut a = shared.borrow_mut();
            let lp = a.web.last_played.clone();
            a.queue.shuffle_rest(&lp);
            paint_queue(&mut a);
        }
        "clear-queue" => {
            let mut a = shared.borrow_mut();
            let keep = a.queue.qi.map_or(0, |i| i + 1);
            a.queue.items.truncate(keep);
            paint_queue(&mut a);
            save_session(&a);
        }
        "clear-requests" => {
            let mut a = shared.borrow_mut();
            let cur = a.queue.qi.unwrap_or(0);
            let mut k = 0;
            a.queue.items.retain(|q| { let keep = k <= cur || q.by.is_empty(); k += 1; keep });
            paint_queue(&mut a);
            save_session(&a);
        }
        "sets" => {
            let mut a = shared.borrow_mut();
            let now = !a.web.prefs["queueSets"].as_bool().unwrap_or(false);
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert("queueSets".into(), serde_json::json!(now)); }
            a.ap_failed = None;
        }
        "state" => {}
        _ => note = Some("That button isn't in the native app yet."),
    }
    let s = shared.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(if action == "state" { 0 } else { 150 }), move || {
        let mut state = remote_state(&s.borrow());
        if let Some(n) = note {
            state["ok"] = serde_json::json!(false);
            state["message"] = serde_json::json!(n);
        }
        omnipulse_core::remote::remote_result(id, state);
    });
}

/// A request that can't be added: the reply for it, in your words if you've set them
fn req_fail(prefs: &serde_json::Value, base: &[(&str, String)], key: &str, extra: Vec<(&str, String)>) -> serde_json::Value {
    let vars: Vec<(&str, String)> = base.iter().cloned().chain(extra).collect();
    serde_json::json!({ "ok": false, "message": requests::msg(prefs, key, &vars) })
}

/// A song request from chat (through core's requests server); the reply goes back to the bot
fn song_request(shared: &Shared, id: u64, q: String, user: String) {
    let reply = move |v: serde_json::Value| omnipulse_core::requests::request_result(id, v);
    let a = shared.borrow();
    let prefs = a.web.prefs.clone();
    let q: String = q.trim().chars().take(200).collect();
    let who = if user.trim().is_empty() { "chat".to_string() } else { user.trim().to_string() };
    // the platforms that are on (and signed in), as for search
    let mut plats: Vec<&'static str> = Vec::new();
    if integrations::on(&prefs, "scOn") && omnipulse_core::soundcloud::signed_in() { plats.push("soundcloud"); }
    if integrations::on(&prefs, "spOn") && omnipulse_core::spotify::spotify_signed_in() { plats.push("spotify"); }
    let local: Vec<Track> = a.library.iter().filter(|t| t.plat() == "local").cloned().collect();
    if integrations::on(&prefs, "localOn") && !local.is_empty() { plats.push("local"); }
    if integrations::on(&prefs, "ytOn") { plats.push("youtube"); }
    let names = plats.iter().map(|p| match *p { "soundcloud" => "SoundCloud", "spotify" => "Spotify", "local" => "your files", _ => "YouTube Music" }).collect::<Vec<_>>().join(" or ");
    let base = vec![("user", who.clone()), ("query", q.clone()), ("platform", names)];
    if !integrations::set(&prefs, "req") || plats.is_empty() { reply(req_fail(&prefs, &base, "closed", vec![])); return; }
    if q.is_empty() { reply(req_fail(&prefs, &base, "empty", vec![])); return; }
    let pending: Vec<String> = a.queue.pending_requests().map(|x| x.by.to_lowercase()).collect();
    if pending.len() as u64 >= prefs["reqMax"].as_u64().unwrap_or(10) { reply(req_fail(&prefs, &base, "full", vec![])); return; }
    let mine = pending.iter().filter(|b| **b == who.to_lowercase()).count();
    if !user.trim().is_empty() && mine as u64 >= prefs["reqPerUser"].as_u64().unwrap_or(2) {
        reply(req_fail(&prefs, &base, "perUser", vec![("count", mine.to_string())]));
        return;
    }
    let likes: Vec<Track> = a.library.iter().filter(|t| t.plat() != "local").cloned().collect();
    let prefer_radio = integrations::set(&prefs, "reqRadioEdit");
    let (rt, weak) = (a.rt.clone(), a.ui.clone());
    drop(a);
    rt.spawn(async move {
        let picked = requests::pick(&q, &plats, &likes, &local, prefer_radio).await;
        let _ = weak.upgrade_in_event_loop(move |_| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                let t = match picked {
                    Ok(t) => t,
                    Err(key) => { reply(req_fail(&prefs, &base, key, vec![])); return; }
                };
                let vars: Vec<(&str, String)> = base.iter().cloned().chain([("title", t.title.clone()), ("artist", t.artist.clone())]).collect();
                let (pos, idle) = {
                    let mut a = shared.borrow_mut();
                    let from = a.queue.qi.unwrap_or(0);
                    if a.queue.items.iter().skip(from).any(|x| x.t.key == t.key) {
                        reply(serde_json::json!({ "ok": false, "message": requests::msg(&prefs, "duplicate", &vars) }));
                        return;
                    }
                    let mut t = t.clone();
                    track::apply_fixes(&a.web.prefs, std::slice::from_mut(&mut t));
                    let pos = a.queue.add_request(t.clone(), &who);
                    paint_queue(&mut a);
                    save_session(&a);
                    (pos, !a.loaded)
                };
                if idle { start(&shared); }
                let vars: Vec<(&str, String)> = vars.into_iter().chain([("position", pos.to_string())]).collect();
                reply(serde_json::json!({
                    "ok": true, "title": t.title, "artist": t.artist, "url": t.web_link(), "position": pos,
                    "message": requests::msg(&prefs, if pos > 0 { "added" } else { "playing" }, &vars),
                }));
            });
        });
    });
}

/// The servers the web app's settings have on: the widget, the Stream Deck's remote, chat requests
fn outside_start(shared: &Shared) {
    let prefs = shared.borrow().web.prefs.clone();
    integrations::widget_start(&prefs);
    if integrations::set(&prefs, "remoteOn") {
        if let Some(key) = prefs["remoteKey"].as_str().filter(|k| !k.is_empty()) {
            omnipulse_core::remote::set_handler(std::sync::Arc::new(|id, action: &str| {
                let action = action.to_string();
                let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { remote_action(&sh, id, &action); } }));
            }));
            let _ = omnipulse_core::remote::remote_start(prefs["remotePort"].as_u64().unwrap_or(7880) as u16, key.to_string());
        }
    }
    if integrations::set(&prefs, "req") {
        if let Some(key) = prefs["reqKey"].as_str().filter(|k| !k.is_empty()) {
            omnipulse_core::requests::set_asker(std::sync::Arc::new(|id, q, user| {
                let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { song_request(&sh, id, q, user); } }));
            }));
            let _ = omnipulse_core::requests::requests_start(prefs["reqPort"].as_u64().unwrap_or(7879) as u16, key.to_string(), false);
        }
    }
    push_out(shared, false);
}

/// A media key, the tray's menu or a taskbar button
fn desktop_action(shared: &Shared, action: &str) {
    match action {
        "play" => toggle(shared),
        "next" => step(shared, 1),
        "prev" => step(shared, -1),
        "shuffle" => {
            let mut a = shared.borrow_mut();
            let lp = a.web.last_played.clone();
            a.queue.shuffle_rest(&lp);
            paint_queue(&mut a);
        }
        "show" => {
            if let Some(ui) = shared.borrow().ui.upgrade() {
                use slint::winit_030::WinitWindowAccessor;
                let _ = ui.window().show();
                ui.window().set_minimized(false);
                ui.window().with_winit_window(|w| w.focus_window());
            }
        }
        "quit" => {
            save_session(&shared.borrow());
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
}

/// The window's handle, once it exists (for the taskbar buttons)
fn window_hwnd(ui: &AppWindow) -> Option<isize> {
    use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use slint::winit_030::WinitWindowAccessor;
    ui.window().with_winit_window(|w| match w.window_handle().map(|h| h.as_raw()) {
        Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
        _ => None,
    })?
}

// ---------- the look: the theme, its tint, the accent (or the album art's) ----------
const ACCENTS: [&str; 6] = ["#8e9bff", "#1ed760", "#fa586a", "#ff9f43", "#4cc9f0", "#c77dff"];

fn apply_look(shared: &Shared) {
    let a = shared.borrow();
    let Some(ui) = a.ui.upgrade() else { return };
    let prefs = &a.web.prefs;
    let (step, hue, strength) = look::from_prefs(prefs);
    let p = look::palette(step, hue, strength);
    let t = ui.global::<Theme>();
    t.set_bg(p.bg.slint());
    t.set_panel(p.panel.slint());
    t.set_elevated(p.elevated.slint());
    t.set_line(p.line.slint());
    t.set_text(p.text.slint());
    t.set_muted(p.muted.slint());
    // the accent: the album art's while something plays (if asked for), or the one picked
    let art = integrations::set(prefs, "artAccent") && a.loaded;
    let (c1, c2) = if art {
        let parse = |s: &str| look::Rgb::parse(s).map(|c| look::tune_art(c, p.light));
        (parse(&a.accent.0), parse(&a.accent.1))
    } else {
        let mine = look::Rgb::parse(prefs["accent"].as_str().unwrap_or(ACCENTS[0]));
        (mine, prefs["accent2"].as_str().and_then(look::Rgb::parse).or(mine))
    };
    let c1 = c1.unwrap_or(look::Rgb(0x8e, 0x9b, 0xff));
    t.set_accent(c1.slint());
    t.set_accent_2(c2.unwrap_or(c1).slint());
    t.set_on_accent(look::on_accent(c1).slint());
    ui.set_light_theme(p.light);
    // and Settings > Look shows where things are
    ui.set_theme_step(step as f32);
    ui.set_tint_hue(hue);
    ui.set_tint_strength(strength as f32);
    ui.set_theme_step_name(look::STEPS[step].0.into());
    ui.set_tint_strength_name(look::STRENGTHS[strength].into());
    ui.set_tint_preview(if strength > 0 { look::hsl(hue, 0.7, if p.light { 55.0 } else { 60.0 }).slint() } else { slint::Color::from_argb_u8(0, 0, 0, 0) });
    ui.set_accent_list(ModelRc::from(Rc::new(VecModel::from(ACCENTS.iter().filter_map(|c| look::Rgb::parse(c)).map(|c| c.slint()).collect::<Vec<_>>()))));
    let picked = prefs["accent"].as_str().unwrap_or(ACCENTS[0]).to_lowercase();
    ui.set_accent_on(ACCENTS.iter().position(|c| *c == picked).map_or(-1, |i| i as i32));
    ui.set_art_accent(integrations::set(prefs, "artAccent"));
}

/// A Look setting changed here: kept (over the web app's), and shown at once
fn set_look_pref(shared: &Shared, key: &str, value: serde_json::Value) {
    {
        let mut a = shared.borrow_mut();
        if let Some(o) = a.web.prefs.as_object_mut() {
            o.insert(key.to_string(), value.clone());
        }
    }
    // (the self-test tries a theme without keeping it)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() {
        webstate::save_pref(key, value);
    }
    apply_look(shared);
}

fn toggle(shared: &Shared) {
    // a restored session starts on the first press, where it was
    if !shared.borrow().loaded {
        start(shared);
        return;
    }
    let (engine, weak) = { let a = shared.borrow(); (a.engine.clone(), a.ui.clone()) };
    let Some(ui) = weak.upgrade() else { return };
    if engine.is_paused() { engine.play(); ui.set_playing(true); } else { engine.pause(); ui.set_playing(false); }
    save_session(&shared.borrow());
    push_out(shared, true);
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
        page_all: Vec::new(), page_tracks: Vec::new(), chip: 0, shelves: Vec::new(), queue: queue::Queue::default(), panel_map: Vec::new(), side_map: Vec::new(),
        here: None, history: Vec::new(), fav_only: false, shuffle_on: false, shuffle_backup: None, repeat: "off".into(),
        autoplay: true, ap_busy: false, ap_then_play: false, ap_failed: None, resume_ms: None, loaded: false, accent: ("#8e9bff".into(), "#8e9bff".into()), started_secs: 0, token: 0,
        lyrics: Vec::new(), heard: (String::new(), 0.0, false), last_pos: 0,
    }));
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    ui.set_page_sub("Loading your Library...".into());
    restore_session(&shared);
    apply_look(&shared);
    let s = shared.clone();
    ui.on_look_changed(move |key, v| {
        // the step settings keep to their steps
        let v = if key == "tintHue" { v.round().clamp(0.0, 359.0) } else { v.round() };
        set_look_pref(&s, &key, serde_json::json!(v));
    });
    let s = shared.clone();
    ui.on_accent_picked(move |i| {
        if let Some(c) = ACCENTS.get(i as usize) {
            set_look_pref(&s, "accent", serde_json::json!(c));
        }
    });
    let s = shared.clone();
    ui.on_art_accent_changed(move |on| set_look_pref(&s, "artAccent", serde_json::json!(on)));
    let s = shared.clone();
    ui.on_plat_toggle(move |p| plat_toggle(&s, &p));
    // Streamer Mode (as the web app's): kept with the rest of this app's settings
    ui.set_streamer(integrations::set(&shared.borrow().web.prefs, "streamerMode"));
    let s = shared.clone();
    ui.on_streamer_changed(move |on| {
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("streamerMode".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("streamerMode", serde_json::json!(on)); }
    });
    paint_plats(&shared.borrow());
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() {
        // the self-test's own: the widget and requests on ports of their own, key "selftest" (not the
        // remote: starting it writes remote.json, which would point the Stream Deck at the test)
        let mut a = shared.borrow_mut();
        if !a.web.prefs.is_object() {
            a.web.prefs = serde_json::json!({});
        }
        if let Some(o) = a.web.prefs.as_object_mut() {
            for (k, v) in [("widget", serde_json::json!(true)), ("widgetPort", serde_json::json!(7979)), ("remoteOn", serde_json::json!(false)), ("req", serde_json::json!(true)), ("reqPort", serde_json::json!(7982)), ("reqKey", serde_json::json!("selftest")), ("presence", serde_json::json!(false))] {
                o.insert(k.into(), v);
            }
        }
        drop(a);
    }
    outside_start(&shared);

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
                    {
                        let mut a = shared.borrow_mut();
                        a.library = all;
                        let prefs = a.web.prefs.clone();
                        track::apply_fixes(&prefs, &mut a.library);
                        a.playlists = playlists;
                        paint_sides(&mut a);
                        paint_plats(&a);
                    }
                    if shared.borrow().page == "library" {
                        shared.borrow_mut().here = Some(Place::Page("library".into()));
                        open_page(&shared, "library");
                        ui.set_page_sub(format!("{} Loaded in {secs:.1} s.", ui.get_page_sub()).into());
                    }
                });
            });
        });
    }

    let s = shared.clone();
    ui.on_nav(move |p| go(&s, Place::Page(p.into())));
    let s = shared.clone();
    ui.on_back(move || go_back(&s));
    let s = shared.clone();
    ui.on_side(move |i| {
        let at = s.borrow().side_map.get(i as usize).copied();
        if let Some(at) = at { go(&s, Place::Playlist(at)); }
    });
    let s = shared.clone();
    ui.on_search(move |q| { if !q.trim().is_empty() { go(&s, Place::Search(q.trim().into())); } });
    let s = shared.clone();
    ui.on_fav_only_changed(move |on| {
        let mut a = s.borrow_mut();
        a.fav_only = on;
        apply_filter(&mut a);
    });
    let s = shared.clone();
    ui.on_play_page(move || {
        let list = s.borrow().page_tracks.clone();
        if !list.is_empty() { play_list(&s, list, 0); }
    });
    let s = shared.clone();
    ui.on_shuffle_toggle(move || shuffle_toggle(&s));
    let s = shared.clone();
    ui.on_repeat_cycle(move || {
        let mut a = s.borrow_mut();
        a.repeat = match a.repeat.as_str() { "off" => "all", "all" => "one", _ => "off" }.into();
        if let Some(ui) = a.ui.upgrade() {
            ui.set_repeat_mode(a.repeat.clone().into());
            ui.set_status(match a.repeat.as_str() { "all" => "Repeat the queue.", "one" => "Repeat this track.", _ => "Repeat off." }.into());
        }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("repeat", serde_json::json!(a.repeat)); }
        paint_queue(&mut a);
    });
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
    ui.on_rows_in_view(move |first, count| {
        ROW_ART.with(|c| c.borrow_mut().view = (first.max(0) as usize, count.max(0) as usize));
        rows_art(&s);
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
            queue::shuffle_in_place(&mut pool);
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
        let at = s.borrow().panel_map.get(i as usize).copied();
        if let Some(at) = at {
            s.borrow_mut().queue.qi = Some(at);
            start(&s);
        }
    });
    let s = shared.clone();
    ui.on_row_action(move |i, act| {
        let mut a = s.borrow_mut();
        let Some(t) = a.page_tracks.get(i as usize).cloned() else { return };
        let empty = a.queue.current().is_none();
        let title = t.title.clone();
        if act == "next" { a.queue.play_next(t); } else { a.queue.add(t); }
        paint_queue(&mut a);
        save_session(&a);
        if let Some(ui) = a.ui.upgrade() {
            ui.set_status(format!("{} \"{title}\".", if act == "next" { "Playing next:" } else { "Added to the queue:" }).into());
        }
        drop(a);
        if empty { start(&s); }
    });
    let s = shared.clone();
    ui.on_queue_action(move |i, act| {
        let mut a = s.borrow_mut();
        let Some(at) = a.panel_map.get(i as usize).copied() else { return };
        match act.as_str() {
            "next" => a.queue.move_next(at),
            "remove" => a.queue.remove(at),
            _ => {
                let lp = a.web.last_played.clone();
                a.queue.shuffle_rest(&lp);
            }
        }
        paint_queue(&mut a);
        save_session(&a);
    });
    let s = shared.clone();
    ui.on_shuffle(move || {
        let list = {
            let a = s.borrow();
            queue::fair_shuffle(a.page_tracks.clone(), &a.web.last_played)
        };
        {
            let mut a = s.borrow_mut();
            a.queue.set_list(list, 0);
            a.ap_failed = None;
        }
        start(&s);
    });
    let s = shared.clone();
    ui.on_autoplay_changed(move |on| {
        let mut a = s.borrow_mut();
        a.autoplay = on;
        a.ap_failed = None;
        if !on { a.queue.drop_autoplay(); }
        paint_queue(&mut a);
        save_session(&a);
    });
    let s = shared.clone();
    ui.on_next(move || step(&s, 1));
    let s = shared.clone();
    ui.on_prev(move || step(&s, -1));
    let s = shared.clone();
    ui.on_toggle(move || toggle(&s));
    let s = shared.clone();
    ui.on_seek(move |f| {
        let engine = s.borrow().engine.clone();
        let d = engine.duration_ms();
        if d > 0 {
            engine.seek((f.clamp(0.0, 1.0) as f64 * d as f64) as u64);
            let s2 = s.clone();
            slint::Timer::single_shot(std::time::Duration::from_millis(300), move || push_out(&s2, true));
        }
    });
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
    let s = shared.clone();
    ui.on_close_window(move || {
        save_session(&s.borrow());
        let _ = slint::quit_event_loop();
    });

    // the media keys and the tray (made here, on the window's thread, which has the message loop they need)
    let desk = desktop::Desktop::start();
    let timer = slint::Timer::default();
    let s = shared.clone();
    let mut n = 0u64;
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        n += 1;
        tick(&s, &events, n);
        for a in desk.poll() {
            desktop_action(&s, a);
        }
    });
    // the taskbar buttons, once the window is up
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
        if let Some(h) = weak.upgrade().as_ref().and_then(window_hwnd) {
            desktop::taskbar::attach(h);
        }
    });

    // OMNIPULSE_SELFTEST=<folder>: visits each page, plays the first track silently, saves a picture of
    // each page and quits (for checking the app without anyone at the screen)
    let _selftest = std::env::var("OMNIPULSE_SELFTEST").ok().map(|dir| {
        engine.set_volume(0.0);
        ui.set_volume(0.0);
        let t = slint::Timer::default();
        let s = shared.clone();
        let started = std::time::Instant::now();
        let mut step_no = 0usize;
        let plan: [(u64, &str); 14] = [(9, "play"), (13, "library"), (14, "scoff"), (15, "scon"), (16, "discover"), (24, "rediscover"), (27, "sets"), (31, "stats"), (35, "now"), (39, "search"), (42, "settings"), (44, "light"), (46, "autoplay"), (58, "end")];
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
                // SoundCloud switched off (and Streamer Mode on), then back
                "scoff" => { ui.invoke_plat_toggle("soundcloud".into()); ui.set_streamer(true); println!("selftest: soundcloud off, {} rows", ui.get_rows().row_count()); }
                "scon" => { ui.invoke_plat_toggle("soundcloud".into()); ui.set_streamer(false); println!("selftest: soundcloud on, {} rows", ui.get_rows().row_count()); }
                // a light theme with a tint, for its picture
                "light" => {
                    ui.invoke_look_changed("themeStep".into(), 0.0);
                    ui.invoke_look_changed("tintHue".into(), 340.0);
                    ui.invoke_look_changed("tintStrength".into(), 2.0);
                }
                // the queue down to what's playing, so autoplay has to add some
                "autoplay" => {
                    {
                        let mut a = s.borrow_mut();
                        let keep = a.queue.qi.map_or(0, |i| i + 1);
                        a.queue.items.truncate(keep);
                    }
                    autoplay_fill(&s, false);
                    ui.set_queue_open(true);
                }
                "search" => { ui.set_search_text("deadmau5".into()); ui.invoke_search("deadmau5".into()); }
                "end" => {
                    println!("selftest: remote state {}", remote_state(&s.borrow()));
                    println!("selftest: queue {} ({} autoplay, {} requested), {}", s.borrow().queue.items.len(), s.borrow().queue.items.iter().filter(|q| q.auto).count(), s.borrow().queue.items.iter().filter(|q| !q.by.is_empty()).count(), ui.get_autoplay_note());
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
    static ROW_ART: RefCell<ArtCache> = RefCell::new(ArtCache::default());
    static SHARED: RefCell<Option<Shared>> = const { RefCell::new(None) };
    static SHELF_MODELS: RefCell<Vec<Rc<VecModel<Tile>>>> = const { RefCell::new(Vec::new()) };
    #[allow(clippy::type_complexity)]
    static STAT_MODELS: RefCell<Option<(Rc<VecModel<Item>>, Rc<VecModel<Item>>)>> = const { RefCell::new(None) };
}
