// OmniPulse, native (the rewrite, Phase 3): one window, one process. Pages for Discover, Rediscover,
// the Library, Favourites, Sets & Podcasts, Stats, Search, your playlists and Now Playing (with
// lyrics), played through omnipulse-audio. Built beside the web app, which keeps working until this
// one does everything; the web app's favourites, stats and playlists come from the copy it writes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod keysetup;
mod offset;
mod plugrun;
mod collab;
mod desktop;
mod instance;
mod mix;
mod updater;
mod discover;
mod integrations;
mod look;
mod marks;
mod mirror;
mod titles;
mod together;
mod versions;
mod player;
mod queue;
mod requests;
mod sound;
mod share;
mod sources;
mod streamsafe;
mod track;
mod webstate;
mod xpl;

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
    /// who you follow (loaded once, for the Artists page)
    artists: Option<Vec<sources::Artist>>,
    /// Favourites only (the track list's pill)
    fav_only: bool,
    /// what's typed in the search bar: the page's tracks with every word in their title, artist or album
    page_query: String,
    /// the tracks picked on the page (by key), and the last one picked (its row), for Shift+click
    picked: std::collections::HashSet<String>,
    pick_anchor: Option<usize>,
    /// shuffle (the player bar's): on, and the upcoming tracks' order before it, to undo it
    shuffle_on: bool,
    shuffle_backup: Option<Vec<String>>,
    /// repeat: "off", "all" (the queue) or "one" (this track)
    repeat: String,
    /// a Go+ track playing its closest YouTube Music version: (its key, that version)
    closest: Option<(String, omnipulse_core::youtube::YtTrack)>,
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
    /// your playlists on this PC (crate::xpl), and what the naming dialog is for
    xpl: serde_json::Value,
    naming: Option<Naming>,
    /// tracks' measured loudness ("soundcloud:123" -> LUFS), and the equalizer's bands as shown
    loud: HashMap<String, f64>,
    eq_model: Rc<VecModel<f32>>,
    /// the playlists "Add to Playlist" offers (yours here, then the platforms'), by id
    add_map: Vec<String>,
    /// the queue came from your Library or a playlist: shuffled, its recent plays are spaced out
    queue_spaced: bool,
    /// tracks' measurements (loudness, silences, where the sound ends), by what was measured, and
    /// which one is the track playing's
    analysis: HashMap<String, serde_json::Value>,
    cur_loud: Option<String>,
}

enum Naming {
    Rename(String),
}

type Shared = Rc<RefCell<App>>;

/// A track row's share links (Copy SoundCloud / Spotify / YouTube Music Link in its menu)
pub fn share_flags(t: &Track) -> Row {
    let links = share::links(t);
    let has = |p: &str| links.iter().any(|(k, _)| *k == p);
    Row { sc_link: has("soundcloud"), sp_link: has("spotify"), yt_link: has("youtube"), ..Default::default() }
}

fn copy_track_link(shared: &Shared, t: &Track, plat: &str) {
    let Some((_, url)) = share::links(t).into_iter().find(|(k, _)| *k == plat) else { return };
    let note = match share::copy(&url) { Ok(()) => format!("{} link copied.", share::name(plat)), Err(e) => e };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
}

fn now_ms() -> u64 {
    omnipulse_core::now_secs() * 1000
}

// ---------- pages ----------
fn apply_filter(app: &mut App) {
    let chip = app.chip;
    let prefs = app.web.prefs.clone();
    let favs: std::collections::HashSet<&str> = app.web.favourites.iter().map(|t| t.key.as_str()).collect();
    let fav_only = app.fav_only;
    // (as the web app's js/pagefind.js: word by word, in the title, artist or album)
    let want = sources::words(&app.page_query);
    let found = |t: &Track| want.is_empty() || { let have = sources::words(&format!("{} {} {}", t.title, t.artist, t.album)); want.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str()))) };
    let shown = |t: &&Track| plat_on(&prefs, t.plat()) && (!fav_only || favs.contains(t.key.as_str())) && streamsafe::allowed(&prefs, t) && found(t);
    let mut filtered: Vec<Track> = match (app.page.as_str(), chip) {
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 1) => app.page_all.iter().filter(shown).filter(|t| !t.is_mix()).cloned().collect(),
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 2) => app.page_all.iter().filter(shown).filter(|t| t.is_mix()).cloned().collect(),
        _ => app.page_all.iter().filter(shown).cloned().collect(),
    };
    if app.page == "library" {
        lib_sort(&prefs, &mut filtered);
    }
    let playing = app.queue.current().map(|t| t.key.clone()).unwrap_or_default();
    let liked: std::collections::HashSet<&str> = app.library.iter().map(|t| t.key.as_str()).collect();
    app.rows.set_vec(filtered.iter().map(|t| Row { liked: liked.contains(t.key.as_str()), picked: app.picked.contains(&t.key), ..t.row(t.key == playing) }).collect::<Vec<_>>());
    app.page_tracks = filtered;
    // its covers, once this is done with the app
    slint::Timer::single_shot(std::time::Duration::ZERO, || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { rows_art(&sh); } }));
}

/// The rows' picked marks (and how many, in the status bar)
fn pick_paint(a: &mut App) {
    for k in 0..a.rows.row_count() {
        if let (Some(mut r), Some(t)) = (a.rows.row_data(k), a.page_tracks.get(k)) {
            let on = a.picked.contains(&t.key);
            if r.picked != on { r.picked = on; a.rows.set_row_data(k, r); }
        }
    }
    if let Some(ui) = a.ui.upgrade() {
        let n = a.page_tracks.iter().filter(|t| a.picked.contains(&t.key)).count();
        ui.set_status(if n > 0 { format!("{} picked: right-click one for what to do with them (Escape lets them go).", plural(n, "track", "tracks")) } else { String::new() }.into());
    }
}

// ---------- the Library's order (as the web app's sort chips, kept in the same setting) ----------
fn lib_sort_of(prefs: &serde_json::Value) -> (&str, i64) {
    let s = &prefs["libSort"];
    match s["by"].as_str() {
        Some(by @ ("artist" | "title")) => (by, if s["dir"].as_i64() == Some(-1) { -1 } else { 1 }),
        _ => ("", 1),
    }
}

fn lib_sort(prefs: &serde_json::Value, list: &mut [Track]) {
    let (by, dir) = lib_sort_of(prefs);
    if by.is_empty() {
        return;
    }
    // ignoring case and a leading "The"
    let key = |s: &str| {
        let s = s.trim().to_lowercase();
        s.strip_prefix("the ").map(str::to_string).unwrap_or(s)
    };
    list.sort_by(|x, y| {
        let (a, b) = ((key(&x.artist), key(&x.title)), (key(&y.artist), key(&y.title)));
        let o = if by == "artist" { a.cmp(&b) } else { (a.1, a.0).cmp(&(b.1, b.0)) };
        if dir < 0 { o.reverse() } else { o }
    });
}

fn lib_sort_paint(a: &App) {
    let Some(ui) = a.ui.upgrade() else { return };
    let (by, dir) = lib_sort_of(&a.web.prefs);
    ui.set_sort_by(by.into());
    ui.set_sort_dir(dir as i32);
}

/// A sort chip: A to Z, then Z to A, then off (as you liked them)
fn lib_sort_pick(shared: &Shared, by: &str) {
    let mut a = shared.borrow_mut();
    let (cur, dir) = lib_sort_of(&a.web.prefs);
    let next = if cur != by { serde_json::json!({ "by": by, "dir": 1 }) } else if dir > 0 { serde_json::json!({ "by": by, "dir": -1 }) } else { serde_json::json!({ "by": "added", "dir": 1 }) };
    if let Some(o) = a.web.prefs.as_object_mut() { o.insert("libSort".into(), next.clone()); }
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("libSort", next); }
    lib_sort_paint(&a);
    apply_filter(&mut a);
}

fn show_tracks(shared: &Shared, page: &str, title: &str, sub: String, mut tracks: Vec<Track>, chips: bool) {
    let mut app = shared.borrow_mut();
    track::apply_fixes(&app.web.prefs, &mut tracks);
    let Some(ui) = app.ui.upgrade() else { return };
    if page == "search" && app.page != "search" { ui.set_shelves(ModelRc::default()); }
    app.page = page.to_string();
    app.page_all = tracks;
    app.chip = 0;
    app.page_query.clear();   // (a page opened afresh shows all of it)
    app.picked.clear();
    app.pick_anchor = None;
    // Search: its first tracks, then what else it found (shelves); Show all lists every track
    let kind = if page == "search" { "search" } else { "tracks" };
    ui.set_page(page.into());
    ui.set_page_kind(kind.into());
    ui.set_page_title(title.into());
    ui.set_page_sub(sub.into());
    // (the Library has an Albums chip too: your saved albums, as a grid)
    let mut names: Vec<slint::SharedString> = if chips { FILTER_CHIPS.iter().map(|c| (*c).into()).collect() } else { Vec::new() };
    if chips && page == "library" { names.push("Albums".into()); }
    ui.set_chips(ModelRc::from(Rc::new(VecModel::from(names))));
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
    /// by its id (the list's order changes as you make and delete your own)
    Playlist(String),
    Search(String),
    Artist(usize),
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
        Place::Playlist(id) => {
            let at = shared.borrow().playlists.iter().position(|p| &p.id == id);
            match at { Some(i) => open_playlist(shared, i), None => open_page(shared, "library") }
            // (a collaborative one: everyone's changes, at most every 10 seconds as it's opened)
            let due = COLLAB_PULLED.with(|m| m.borrow().get(id).is_none_or(|t: &std::time::Instant| t.elapsed().as_secs() >= 10));
            if due && collab_of(&shared.borrow().xpl, id).is_some() {
                COLLAB_PULLED.with(|m| m.borrow_mut().insert(id.clone(), std::time::Instant::now()));
                collab_pull(shared, id);
            }
        }
        Place::Search(q) => open_search(shared, q.clone()),
        Place::Artist(i) => open_artist(shared, *i),
    }
    let a = shared.borrow();
    if let Some(ui) = a.ui.upgrade() {
        ui.set_can_back(!a.history.is_empty());
        ui.set_own_page(matches!(place, Place::Playlist(id) if xpl::is_own(id)));
    }
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
            let fav = marks::favourites(&shared.borrow().web.favourites);
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
            let none = { let a = shared.borrow(); a.web.favourites.is_empty() && !a.library.iter().any(|t| t.plat() != "local") };
            // (nothing liked yet: asked for some, as the web app's card)
            let sub = if none { "Get better recommendations: add some music to your likes or favourites (the heart or the star on any song), and Discover and Rediscover will be built around it.".to_string() } else if pool.is_empty() { "You've played all your likes recently.".to_string() } else { "Likes you haven't heard in a month or more, the longest-forgotten first.".to_string() };
            show_tracks(shared, "rediscover", "Rediscover", sub, pool, true);
        }
        // (while Stream-Safe Music is on, the Streamer Mode page stands in for Discover)
        "discover" if streamsafe::on(&shared.borrow().web.prefs) => open_streamer(shared),
        "streamer" => open_streamer(shared),
        "discover" => open_discover(shared),
        "artists" => open_artists(shared),
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

/// The title bar's platforms, as the web app's: each one, in its colours while it's set up (signed in,
/// or yt-dlp here for YouTube Music) and on, grey until it's set up (clicking it opens Settings). Your
/// files have no icon (nothing to sign in to): they switch on and off in Settings.
fn paint_plats(a: &App) {
    let Some(ui) = a.ui.upgrade() else { return };
    let mut list: Vec<Plat> = Vec::new();
    let mut add = |id: &str, name: &str, ready: bool| list.push(Plat { id: id.into(), on: ready && plat_on(&a.web.prefs, id), name: name.into(), ready });
    add("soundcloud", "SoundCloud", omnipulse_core::soundcloud::signed_in());
    add("spotify", "Spotify", omnipulse_core::spotify::spotify_signed_in());
    add("youtube", "YouTube Music", omnipulse_core::tools::ytdlp_path().is_some());
    ui.set_plats(ModelRc::from(Rc::new(VecModel::from(list))));
}

/// The sidebar's playlists: those of platforms switched off are left out
fn paint_sides(a: &mut App) {
    let Some(ui) = a.ui.upgrade() else { return };
    let prefs = a.web.prefs.clone();
    let hidden: std::collections::HashSet<String> = prefs["hiddenPlaylists"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    a.side_map = (0..a.playlists.len())
        .filter(|&i| plat_on(&prefs, a.playlists[i].plat) || a.playlists[i].plat == "local")
        .filter(|&i| !hidden.contains(&mirror::side_id(a.playlists[i].plat, &a.playlists[i].id)))
        .collect();
    let sides: Vec<Side> = a.side_map.iter().map(|&i| Side { title: a.playlists[i].title.clone().into(), plat: a.playlists[i].plat.into(), own: xpl::is_own(&a.playlists[i].id), share: a.playlists[i].plat != "local" || xpl::is_own(&a.playlists[i].id), sc: share::platform_id(&a.playlists[i], &a.xpl, "soundcloud").is_some(), sp: share::platform_id(&a.playlists[i], &a.xpl, "spotify").is_some(), collab: collab_of(&a.xpl, &a.playlists[i].id).is_some(), collab_owner: collab_of(&a.xpl, &a.playlists[i].id).is_some_and(|c| c["owner"].as_bool() == Some(true)) }).collect();
    ui.set_sides(ModelRc::from(Rc::new(VecModel::from(sides))));
    // Add to Playlist: yours here first, then the platforms' (one of those becomes a playlist here)
    let mut order: Vec<usize> = a.side_map.iter().copied().filter(|&i| xpl::is_own(&a.playlists[i].id)).collect();
    order.extend(a.side_map.iter().copied().filter(|&i| !xpl::is_own(&a.playlists[i].id) && a.playlists[i].plat != "local"));
    let names: Vec<slint::SharedString> = order.iter().map(|&i| {
        let p = &a.playlists[i];
        match p.plat { "spotify" => format!("{} (Spotify)", p.title), "soundcloud" if !xpl::is_own(&p.id) => format!("{} (SoundCloud)", p.title), _ => p.title.clone() }.into()
    }).collect();
    a.add_map = order.iter().map(|&i| a.playlists[i].id.clone()).collect();
    ui.set_own_lists(ModelRc::from(Rc::new(VecModel::from(names))));
}

/// Go+ tracks are listed only while YouTube Music can play them in full: switched on, and yt-dlp here
fn go_plus_set(prefs: &serde_json::Value) {
    let on = plat_on(prefs, "youtube") && omnipulse_core::tools::ytdlp_path().is_some();
    sources::GO_PLUS.store(on, std::sync::atomic::Ordering::Relaxed);
    // (MusicBrainz in search: Settings > Integrations, as the web app's prefs.mbOn)
    sources::MUSICBRAINZ.store(integrations::on(prefs, "mbOn"), std::sync::atomic::Ordering::Relaxed);
    // (and Deezer's and Apple's catalogues: prefs.catOn)
    sources::CATALOG.store(integrations::on(prefs, "catOn"), std::sync::atomic::Ordering::Relaxed);
}

fn plat_toggle(shared: &Shared, plat: &str) {
    // not set up yet: Settings, to set it up
    let ready = match plat {
        "soundcloud" => omnipulse_core::soundcloud::signed_in(),
        "spotify" => omnipulse_core::spotify::spotify_signed_in(),
        "youtube" => omnipulse_core::tools::ytdlp_path().is_some(),
        _ => true,
    };
    if !ready {
        open_page(shared, "settings");
        return;
    }
    let key = plat_key(plat);
    let now = {
        let mut a = shared.borrow_mut();
        let now = !plat_on(&a.web.prefs, plat);
        if let Some(o) = a.web.prefs.as_object_mut() {
            o.insert(key.into(), serde_json::json!(now));
        }
        paint_sides(&mut a);
        paint_plats(&a);
        go_plus_set(&a.web.prefs);
        paint_settings(&a);
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
    search_shelves(shared, q.clone());
    // the plugins asked at once too; their results (slower: the Internet Archive) join once they're in,
    // ranked with the rest
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let token = { let mut a = shared.borrow_mut(); a.token += 1; a.token };
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    rt.spawn(async move {
        let q2 = q.clone();
        let late = tokio::spawn(async move { sources::search_plugins(&q2).await });
        let t = sources::search(&q).await;
        let title = format!("Results for \"{q}\"");
        let sub = if t.is_empty() { "Nothing found.".to_string() } else { plural(t.len(), "track", "tracks") + ", best match first." };
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if shared.borrow().token != token { return; }
            show_tracks(&shared, "search", &title, sub, t, true);
        }));
        let more = late.await.unwrap_or_default();
        if more.is_empty() { return; }
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let mut a = shared.borrow_mut();
            if a.token != token || a.page != "search" { return; }
            let mut all = std::mem::take(&mut a.page_all);
            all.extend(more);
            a.page_all = sources::once_each(sources::rank(&q, all));
            ui.set_page_sub((plural(a.page_all.len(), "track", "tracks") + ", best match first.").into());
            apply_filter(&mut a);
        }));
    });
}

thread_local! {
    /// what Search found besides tracks, shelf by shelf (its tiles open them)
    static FOUND: RefCell<Vec<Vec<sources::Found>>> = const { RefCell::new(Vec::new()) };
}

/// Search's shelves: artists, albums and playlists from every platform, with their pictures as they come
fn search_shelves(shared: &Shared, q: String) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    FOUND.with(|f| f.borrow_mut().clear());
    rt.spawn(async move {
        let (artists, albums, lists) = sources::search_found(&q).await;
        let shelves: Vec<(&str, Vec<sources::Found>)> = [("Artists", artists), ("Albums", albums), ("Playlists", lists)].into_iter().filter(|(_, l)| !l.is_empty()).collect();
        let arts: Vec<Vec<String>> = shelves.iter().map(|(_, l)| l.iter().map(|f| f.tile().2).collect()).collect();
        let weak2 = weak.clone();
        let q2 = q.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if shared.borrow().page != "search" || !ui.get_page_title().contains(q2.as_str()) { return; }
            let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().map(|(_, l)| Rc::new(VecModel::from(l.iter().map(|f| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into() } }).collect::<Vec<_>>()))).collect();
            let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|((title, _), m)| Shelf { title: (*title).into(), tiles: ModelRc::from(m.clone()) }).collect();
            ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
            SHELF_MODELS.with(|m| *m.borrow_mut() = models);
            FOUND.with(|f| *f.borrow_mut() = shelves.into_iter().map(|(_, l)| l).collect());
        }));
        for (s, list) in arts.into_iter().enumerate() {
            for (k, url) in list.into_iter().enumerate() {
                if url.is_empty() { continue; }
                if let Some(px) = player::fetch_art(url, 200).await {
                    let _ = weak2.upgrade_in_event_loop(move |_| SHELF_MODELS.with(|m| {
                        if let Some(model) = m.borrow().get(s) {
                            if let Some(mut t) = model.row_data(k) { t.art = slint::Image::from_rgba8(px); model.set_row_data(k, t); }
                        }
                    }));
                }
            }
        }
    });
}

thread_local! {
    /// the Library's Albums chip: its albums, as tiles
    static ALBUMS: RefCell<Vec<sources::Found>> = const { RefCell::new(Vec::new()) };
    static ALBUM_MODEL: RefCell<Option<Rc<VecModel<Tile>>>> = const { RefCell::new(None) };
}

/// The Library's Albums: your saved albums on Spotify and SoundCloud, and your files' albums
fn library_albums(shared: &Shared) {
    let (rt, weak, files) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.library.iter().filter(|t| t.plat() == "local").cloned().collect::<Vec<_>>()) };
    if let Some(ui) = weak.upgrade() { ui.set_page_kind("albumgrid".into()); ui.set_album_tiles(ModelRc::default()); ui.set_page_sub("Your saved albums...".into()); }
    rt.spawn(async move {
        let mut list = sources::saved_albums().await;
        list.extend(sources::local_albums(&files));
        let arts: Vec<String> = list.iter().map(|f| f.tile().2).collect();
        let n = list.len();
        let weak2 = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if ui.get_page_kind() != "albumgrid" { return; }
            let model = Rc::new(VecModel::from(list.iter().map(|f| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into() } }).collect::<Vec<_>>()));
            ui.set_album_tiles(ModelRc::from(model.clone()));
            ui.set_page_sub(if n == 0 { "No saved albums yet: save some on Spotify, like some on SoundCloud, or add music files with their album names.".to_string() } else { format!("{}: saved on Spotify, liked on SoundCloud, and your files'.", plural(n, "album", "albums")) }.into());
            ALBUM_MODEL.with(|m| *m.borrow_mut() = Some(model));
            ALBUMS.with(|a| *a.borrow_mut() = list);
        });
        for (k, url) in arts.into_iter().enumerate() {
            if url.is_empty() { continue; }
            if let Some(px) = player::fetch_art(url, 200).await {
                let _ = weak2.upgrade_in_event_loop(move |_| ALBUM_MODEL.with(|m| {
                    if let Some(model) = m.borrow().as_ref() { if let Some(mut t) = model.row_data(k) { t.art = slint::Image::from_rgba8(px); model.set_row_data(k, t); } }
                }));
            }
        }
    });
}

/// An album on the Albums grid: its tracks, as a page
fn album_open(shared: &Shared, i: usize) {
    let Some(f) = ALBUMS.with(|a| a.borrow().get(i).cloned()) else { return };
    if let sources::Found::Album { plat: "local", id, by, .. } = &f {
        let list: Vec<Track> = shared.borrow().library.iter().filter(|t| t.plat() == "local" && t.album.trim() == id).cloned().collect();
        let sub = format!("{by} \u{b7} {}.", plural(list.len(), "track", "tracks"));
        return show_tracks(shared, "album", id, sub, list, false);
    }
    let (title, by, _, _) = f.tile();
    show_tracks(shared, "album", &title, "Loading...".into(), Vec::new(), false);
    load_into(shared, "album", title, async move {
        let list = sources::found_tracks(&f).await;
        let sub = if list.is_empty() { "Nothing to play here.".to_string() } else { format!("{by} \u{b7} {}.", plural(list.len(), "track", "tracks")) };
        (list, sub)
    });
}

/// A tile on Search's shelves: that artist's, album's or playlist's tracks, as a page
fn open_found(shared: &Shared, sh: usize, k: usize) {
    let Some(f) = FOUND.with(|x| x.borrow().get(sh).and_then(|l| l.get(k).cloned())) else { return };
    let (title, by, _, _) = f.tile();
    show_tracks(shared, "found", &title, "Loading...".into(), Vec::new(), true);
    let cat = catalog_on(&shared.borrow().web.prefs);
    load_into(shared, "found", title.clone(), async move {
        let (list, bio) = match &f {
            sources::Found::Artist { name, .. } => {
                let (l, dz, bio) = tokio::join!(sources::found_tracks(&f), artist_top(name, cat), artist_bio(name));
                (sources::once_each(sources::interleave(vec![l, dz])), bio)
            }
            _ => (sources::found_tracks(&f).await, String::new()),
        };
        let sub = if list.is_empty() { "Nothing to play here.".to_string() } else { format!("{bio}{}{}.", if by == "Artist" { String::new() } else { format!("{by} \u{b7} ") }, plural(list.len(), "track", "tracks")) };
        (list, sub)
    });
}

// ---------- Artists: who you follow, and each one's tracks ----------
fn open_artists(shared: &Shared) {
    let ui = shared.borrow().ui.upgrade().unwrap();
    {
        let mut a = shared.borrow_mut();
        a.page = "artists".into();
        a.token += 1;
    }
    ui.set_page("artists".into());
    ui.set_page_kind("artists".into());
    ui.set_page_title("Artists".into());
    ui.set_chips(ModelRc::default());
    let cached = shared.borrow().artists.clone();
    if let Some(list) = cached {
        paint_artists(shared, &list);
        return;
    }
    ui.set_page_sub("Loading the artists you follow...".into());
    ui.set_artists(ModelRc::default());
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    rt.spawn(async move {
        let list = sources::followed_artists().await;
        let _ = weak.upgrade_in_event_loop(move |_| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                shared.borrow_mut().artists = Some(list.clone());
                if shared.borrow().page == "artists" { paint_artists(&shared, &list); }
            });
        });
    });
}

fn paint_artists(shared: &Shared, list: &[sources::Artist]) {
    let (ui, rt, weak) = { let a = shared.borrow(); (a.ui.upgrade().unwrap(), a.rt.clone(), a.ui.clone()) };
    let name = |p: &str| match p { "spotify" => "Spotify", _ => "SoundCloud" };
    let items: Vec<Item> = list.iter().map(|a| Item {
        name: a.name.clone().into(),
        sub: a.plats().into_iter().map(name).collect::<Vec<_>>().join(" \u{b7} ").into(),
        art: slint::Image::default(),
        plat: a.plats().first().copied().unwrap_or("soundcloud").into(),
    }).collect();
    ui.set_page_sub(if list.is_empty() { "Artists you follow on SoundCloud and Spotify show here.".into() } else { format!("{} you follow on SoundCloud and Spotify.", plural(list.len(), "artist", "artists")).into() });
    let model = Rc::new(VecModel::from(items));
    ui.set_artists(ModelRc::from(model.clone()));
    ARTIST_MODEL.with(|m| *m.borrow_mut() = Some(model));
    // their pictures, as they come
    let urls: Vec<String> = list.iter().map(|a| a.img.clone()).collect();
    rt.spawn(async move {
        for (k, url) in urls.into_iter().enumerate() {
            if url.is_empty() { continue; }
            if let Some(px) = player::fetch_round(url, 170).await {
                let _ = weak.upgrade_in_event_loop(move |_| {
                    ARTIST_MODEL.with(|m| {
                        if let Some(model) = m.borrow().as_ref() {
                            if let Some(mut it) = model.row_data(k) { it.art = slint::Image::from_rgba8(px); model.set_row_data(k, it); }
                        }
                    });
                });
            }
        }
    });
}

fn open_artist(shared: &Shared, i: usize) {
    let Some(a) = shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned()) else { return };
    let name = |p: &str| match p { "spotify" => "Spotify", _ => "SoundCloud" };
    let on = a.plats().into_iter().map(name).collect::<Vec<_>>().join(" and ");
    show_tracks(shared, "artist", &a.name, "Loading...".into(), Vec::new(), true);
    let cat = catalog_on(&shared.borrow().web.prefs);
    load_into(shared, "artist", a.name.clone(), async move {
        let (t, dz, bio) = tokio::join!(sources::artist_tracks(&a), artist_top(&a.name, cat), artist_bio(&a.name));
        let t = sources::once_each(sources::interleave(vec![t, dz]));
        let sub = format!("{bio}You follow them on {on}. {}.", plural(t.len(), "track", "tracks"));
        (t, sub)
    });
}

/// An artist's best known songs on Deezer (with the catalogues on)
async fn artist_top(name: &str, on: bool) -> Vec<Track> {
    if !on { return Vec::new(); }
    let Some((id, _)) = discover::dz_artist(name).await else { return Vec::new() };
    omnipulse_core::catalog::deezer(&format!("/artist/{id}/top?limit=20")).await.ok()
        .map(|v| v["data"].as_array().into_iter().flatten().filter_map(|t| omnipulse_core::catalog::dz_track(t, None)).filter_map(|t| Track::catalog(&t)).collect())
        .unwrap_or_default()
}

/// An artist's bio, as the web app's (Wikipedia's, core::bio): its first sentence or two, then a space
async fn artist_bio(name: &str) -> String {
    let Ok(v) = omnipulse_core::bio::artist(name).await else { return String::new() };
    let text = v["text"].as_str().unwrap_or("").trim().to_string();
    if text.is_empty() { return String::new(); }
    let mut out = String::new();
    for sentence in text.split_inclusive(". ") {
        if !out.is_empty() && out.len() + sentence.len() > 260 { break; }
        out.push_str(sentence);
    }
    format!("{} ", out.trim())
}

// ---------- Discover: shelves ----------
thread_local! {
    /// Discover's shelves as built (crate::discover), for its tiles' clicks
    static DISCOVER: RefCell<Vec<discover::Shelf>> = const { RefCell::new(Vec::new()) };
}

/// Whether the catalogues (Deezer's and Apple's) are used: on unless switched off, while YouTube Music
/// can play their songs
fn catalog_on(prefs: &serde_json::Value) -> bool {
    prefs["catOn"].as_bool() != Some(false) && omnipulse_core::tools::ytdlp_path().is_some()
}

fn discover_flags(prefs: &serde_json::Value) -> (bool, bool, bool) {
    (catalog_on(prefs), omnipulse_core::tools::ytdlp_path().is_some(), omnipulse_core::soundcloud::signed_in())
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
    ui.set_page_sub("Mixes and stations made from what you typically like to listen to.".into());
    ui.set_chips(ModelRc::default());
    ui.set_shelves(ModelRc::default());
    ui.set_status("Finding music for you...".into());
    {
        let a = shared.borrow();
        let likes = !a.web.favourites.is_empty() || a.library.iter().any(|t| t.plat() != "local");
        ui.set_likes_prompt(!likes);
        ui.set_taste_label(if discover::taste_genres(&a.web.prefs).is_empty() { "Pick genres".into() } else { "Your genres".into() });
    }
    let (input, rt, weak, token) = {
        let a = shared.borrow();
        let (catalog, _, soundcloud) = discover_flags(&a.web.prefs);
        let plays = a.web.plays.iter().map(|p| (p.artist.clone(), p.art.clone(), p.genre.clone())).collect();
        (discover::Input { library: a.library.clone(), favourites: a.web.favourites.clone(), plays, prefs: a.web.prefs.clone(), catalog, soundcloud }, a.rt.clone(), a.ui.clone(), a.token)
    };
    rt.spawn(async move {
        let shelves = discover::build(input).await;
        let arts: Vec<Vec<String>> = shelves.iter().map(|sh| sh.items.iter().map(|i| i.tile().2).collect()).collect();
        let weak2 = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                // (left meanwhile: its status doesn't stay)
                if ui.get_status() == "Finding music for you..." { ui.set_status("".into()); }
                if shared.borrow().token != token { return; }
                let mut shelves = shelves;
                {
                    let prefs = shared.borrow().web.prefs.clone();
                    for sh in shelves.iter_mut() {
                        for it in sh.items.iter_mut() {
                            if let discover::Item::Track(t) = it { track::apply_fixes(&prefs, std::slice::from_mut(t)); }
                        }
                    }
                }
                let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().map(|sh| Rc::new(VecModel::from(sh.items.iter().map(|i| {
                    let (title, sub, _, plat) = i.tile();
                    Tile { title: title.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into() }
                }).collect::<Vec<_>>()))).collect();
                let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|(sh, m)| Shelf { title: sh.title.clone().into(), tiles: ModelRc::from(m.clone()) }).collect();
                ui.set_status(if rows.is_empty() { "Play or star a few songs, and Discover will fill up with music for you.".into() } else { "".into() });
                ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
                shared.borrow_mut().shelves = shelves.iter().map(|sh| sh.items.iter().filter_map(|i| if let discover::Item::Track(t) = i { Some(t.clone()) } else { None }).collect()).collect();
                DISCOVER.with(|d| *d.borrow_mut() = shelves);
                SHELF_MODELS.with(|m| *m.borrow_mut() = models);
            });
        });
        // the covers, as they come
        for (s, list) in arts.into_iter().enumerate() {
            for (k, url) in list.into_iter().enumerate() {
                if url.is_empty() { continue; }
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

// ---------- Pick genres and Fine-tune (as the web app's tasteOpen / subOpen) ----------
thread_local! {
    /// the dialog's chips: (their group, their name, picked), and where each row's chips are in it
    static TASTE: RefCell<(u8, Vec<(String, String, bool)>, Vec<Vec<usize>>)> = const { RefCell::new((1, Vec::new(), Vec::new())) };
}

fn pref_set(shared: &Shared, key: &str, v: serde_json::Value) {
    if let Some(o) = shared.borrow_mut().web.prefs.as_object_mut() { o.insert(key.into(), v.clone()); }
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(key, v); }
}

fn taste_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    TASTE.with(|t| {
        let mut t = t.borrow_mut();
        let (step, chips, _) = &*t;
        let step = *step;
        let mut rows: Vec<TasteRow> = Vec::new();
        let mut map: Vec<Vec<usize>> = Vec::new();
        let mut group = String::new();
        let mut line: Vec<usize> = Vec::new();
        let flush = |line: &mut Vec<usize>, rows: &mut Vec<TasteRow>, map: &mut Vec<Vec<usize>>| {
            if line.is_empty() { return; }
            rows.push(TasteRow { head: "".into(), chips: ModelRc::from(Rc::new(VecModel::from(line.iter().map(|&i| TasteChip { label: chips[i].1.clone().into(), on: chips[i].2 }).collect::<Vec<_>>()))) });
            map.push(std::mem::take(line));
        };
        for (i, (g, _, _)) in chips.iter().enumerate() {
            if step == 2 && *g != group {
                flush(&mut line, &mut rows, &mut map);
                group = g.clone();
                rows.push(TasteRow { head: g.clone().into(), chips: ModelRc::default() });
                map.push(Vec::new());
            }
            line.push(i);
            if line.len() == 4 { flush(&mut line, &mut rows, &mut map); }
        }
        flush(&mut line, &mut rows, &mut map);
        t.2 = map;
        ui.set_taste_step(step as i32);
        ui.set_taste_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
    });
}

fn taste_open(shared: &Shared, step: u8) {
    let prefs = shared.borrow().web.prefs.clone();
    let chips: Vec<(String, String, bool)> = if step == 1 {
        let on = discover::taste_genres(&prefs);
        discover::TASTE_GENRES.iter().map(|(g, _)| (String::new(), g.to_string(), on.iter().any(|x| x == g))).collect()
    } else {
        let on: Vec<String> = prefs["tasteSubgenres"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
        discover::taste_genres(&prefs).iter().flat_map(|g| discover::subgenres(g).iter().map(|sg| (g.clone(), sg.to_string(), on.iter().any(|x| x == sg))).collect::<Vec<_>>()).collect()
    };
    TASTE.with(|t| *t.borrow_mut() = (step, chips, Vec::new()));
    taste_paint(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_taste_open(true); }
}

fn taste_toggle(shared: &Shared, row: usize, k: usize) {
    TASTE.with(|t| {
        let mut t = t.borrow_mut();
        let Some(&i) = t.2.get(row).and_then(|r| r.get(k)) else { return };
        let now = !t.1[i].2;
        // (a subgenre under two genres is one choice: both its chips follow)
        let name = t.1[i].1.clone();
        for c in t.1.iter_mut().filter(|c| c.1 == name) { c.2 = now; }
    });
    taste_paint(shared);
}

fn taste_picked() -> Vec<String> {
    TASTE.with(|t| {
        let mut out: Vec<String> = Vec::new();
        for c in t.borrow().1.iter().filter(|c| c.2) { if !out.contains(&c.1) { out.push(c.1.clone()); } }
        out
    })
}

fn taste_save(shared: &Shared) {
    let step = TASTE.with(|t| t.borrow().0);
    let picked = taste_picked();
    if step == 1 {
        pref_set(shared, "tasteGenres", serde_json::json!(picked));
        // then the second step, for the subgenres of the ones picked
        if picked.iter().any(|g| !discover::subgenres(g).is_empty()) { return taste_open(shared, 2); }
    } else {
        pref_set(shared, "tasteSubgenres", serde_json::json!(picked));
    }
    taste_done(shared);
}

fn taste_done(shared: &Shared) {
    let prefs = shared.borrow().web.prefs.clone();
    let (g, sg) = (discover::taste_genres(&prefs), discover::taste_subgenres(&prefs));
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_taste_open(false);
        ui.set_taste_label(if g.is_empty() { "Pick genres".into() } else { "Your genres".into() });
        ui.set_status(if g.is_empty() { "No genres picked.".into() } else { format!("Thanks. Discover starts from {} now.", if sg.is_empty() { g.join(", ") } else { sg.join(", ") }).into() });
    }
    open_discover(shared);
}

/// A tile on Discover: a song plays (with the shelf's other songs after it); a mix or station opens as
/// a page of its tracks
fn discover_tile(shared: &Shared, sh: usize, k: usize) {
    let Some(item) = DISCOVER.with(|d| d.borrow().get(sh).and_then(|s| s.items.get(k).cloned())) else { return };
    match item {
        discover::Item::Track(t) => {
            let list: Vec<Track> = DISCOVER.with(|d| d.borrow()[sh].items.iter().filter_map(|i| if let discover::Item::Track(x) = i { Some(x.clone()) } else { None }).collect());
            let i = list.iter().position(|x| x.key == t.key).unwrap_or(0);
            play_list(shared, list, i);
        }
        discover::Item::Mix(m) => {
            let (liked, flags) = { let a = shared.borrow(); (track::Liked::of(&a.library), discover_flags(&a.web.prefs)) };
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Putting {} together...", m.title).into()); }
            let title = if matches!(m.how, discover::How::Crew(_, true)) { format!("{} station", m.title) } else { m.title.clone() };
            let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
            let token = { let mut a = shared.borrow_mut(); a.token += 1; a.token };
            rt.spawn(async move {
                let list = discover::mix_tracks(&m, flags, &liked).await;
                let sub = if list.is_empty() { "Nothing came back for this one: try again in a moment.".to_string() } else { m.desc.clone() };
                let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                    let Some(shared) = s.borrow().clone() else { return };
                    if shared.borrow().token != token { return; }
                    ui.set_status("".into());
                    show_tracks(&shared, "mix", &title, sub, list, false);
                }));
            });
        }
    }
}

// ---------- casting (as the web app's js/cast.js, through crates/core/src/cast.rs) ----------
// The app stays the player: its sound (before the volume) goes to the device as a live stream, the
// PC's own output is silent meanwhile, and the volume slider is the device's. Track changes give the
// device the new title and cover; pausing and playing on either side is followed.
thread_local! {
    /// the device being cast to (its name)
    static CASTING: RefCell<Option<String>> = const { RefCell::new(None) };
    /// the cast devices last listed in the output menu (after Windows' devices)
    static CAST_LIST: RefCell<Vec<omnipulse_core::cast::Device>> = const { RefCell::new(Vec::new()) };
}

pub fn casting() -> Option<String> {
    CASTING.with(|c| c.borrow().clone())
}

fn cast_meta(a: &App) -> serde_json::Value {
    match a.queue.current() {
        Some(t) => serde_json::json!({ "title": t.title, "artist": t.artist, "album": t.album, "art": t.art.replace("-t200x200", "-t500x500"), "playing": !a.engine.is_paused() }),
        None => serde_json::json!({ "title": "OmniPulse" }),
    }
}

fn cast_start(shared: &Shared, d: omnipulse_core::cast::Device) {
    let (rt, weak, meta) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), cast_meta(&a)) };
    if let Some(ui) = weak.upgrade() { ui.set_status(format!("Connecting to {}...", d.name).into()); }
    let events: std::sync::Arc<dyn Fn(serde_json::Value) + Send + Sync> = std::sync::Arc::new(|m| {
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { cast_event(&sh, m); } }));
    });
    rt.spawn_blocking(move || {
        let got = omnipulse_core::cast::start(&d.id, meta, events);
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            match got {
                Ok(dev) => {
                    CASTING.with(|c| *c.borrow_mut() = Some(dev.name.clone()));
                    let engine = shared.borrow().engine.clone();
                    let rate = engine.output_rate();
                    let mut bytes = Vec::new();
                    engine.set_tap("cast", Some(Box::new(move |x: &[f32]| {
                        offset::pcm(x, &mut bytes);
                        if bytes.len() >= rate as usize / 50 * 4 { omnipulse_core::cast::push_pcm(&bytes, rate); bytes.clear(); }
                    })));
                    engine.set_muted(true);
                    let offset_on = shared.borrow().web.prefs["out2On"].as_bool() == Some(true);
                    offset_apply(&shared);
                    ui.set_status(if offset_on { format!("Casting to {}. OmniPulse Offset is paused until you stop.", dev.name) } else { format!("Casting to {}. It plays there a moment after it does here.", dev.name) }.into());
                }
                Err(e) => ui.set_status(format!("Couldn't cast to {}: {e}", d.name).into()),
            }
        }));
    });
}

/// Casting over (stopped here, or by the device): the PC's own output back, and OmniPulse Offset
fn cast_end(shared: &Shared, note: Option<String>) {
    let Some(name) = CASTING.with(|c| c.borrow_mut().take()) else { return };
    let engine = shared.borrow().engine.clone();
    engine.set_tap("cast", None);
    engine.set_muted(false);
    offset_apply(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.unwrap_or_else(|| format!("Stopped casting to {name}.")).into()); }
}

fn cast_stop(shared: &Shared) {
    omnipulse_core::cast::stop();
    cast_end(shared, None);
}

/// What the device says
fn cast_event(shared: &Shared, m: serde_json::Value) {
    if casting().is_none() { return; }
    match m["type"].as_str() {
        Some("ended") => cast_end(shared, m["message"].as_str().map(|x| format!("Casting stopped: {x}"))),
        Some("error") => { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(m["message"].as_str().unwrap_or("").to_string().into()); } }
        // paused or played with the device's own controls
        Some("media") => {
            let paused = shared.borrow().engine.is_paused();
            match m["state"].as_str() {
                Some("PAUSED") if !paused => toggle(shared),
                Some("PLAYING") if paused => { omnipulse_core::cast::reload(); toggle(shared); }
                _ => {}
            }
        }
        _ => {}
    }
}

// ---------- Track preferences (as the web app's, in js/trims.js and the title fixes) ----------
// Its title and artist as you want them shown (prefs.trackFixes[id] = { title, artist }) and the part
// of it that plays (prefs.trackTrims[id] = { start, end }, ms).
thread_local! {
    /// the track being edited
    static TP_TRACK: RefCell<Option<Track>> = const { RefCell::new(None) };
    /// the YouTube Music versions it could play from (Track preferences' list)
    static TP_VERSIONS: RefCell<Vec<omnipulse_core::youtube::YtTrack>> = const { RefCell::new(Vec::new()) };
}

/// For a track that plays from YouTube Music (a Go+ track, a MusicBrainz song): the closest few
/// versions there, best first, with the one it plays marked
fn tp_versions(shared: &Shared, t: &Track) {
    TP_VERSIONS.with(|v| v.borrow_mut().clear());
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_tp_versions(ModelRc::default()); ui.set_tp_version_on(-1); }
    if !(t.go_plus || matches!(t.src, track::Src::Catalog(_))) { return; }
    let (rt, weak, t) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), t.clone()) };
    rt.spawn(async move {
        let (artist, title) = versions::guess_names(&t);
        let mut found = omnipulse_core::youtube::search(&format!("{artist} {title}")).await.unwrap_or_default();
        found.sort_by(|a, b| versions::score(&t, b).total_cmp(&versions::score(&t, a)));
        found.truncate(6);
        let on = versions::saved_id(&t.key);
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if TP_TRACK.with(|x| x.borrow().as_ref().map(|x| x.key.clone())) != Some(t.key.clone()) { return; }
            let names: Vec<slint::SharedString> = found.iter().map(|c| format!("{} \u{b7} {} \u{b7} {}", c.title, c.artists, fmt_ms(c.duration_ms)).into()).collect();
            ui.set_tp_version_on(on.as_ref().and_then(|id| found.iter().position(|c| &c.id == id)).map_or(-1, |i| i as i32));
            ui.set_tp_versions(ModelRc::from(Rc::new(VecModel::from(names))));
            TP_VERSIONS.with(|v| *v.borrow_mut() = found);
        });
    });
}

/// m:ss (or m:ss.s, or seconds) to ms; empty is None, nonsense an error
fn parse_time(s: &str) -> Result<Option<u64>, ()> {
    let s = s.trim();
    if s.is_empty() { return Ok(None); }
    let mut ms = 0f64;
    for p in s.split(':') {
        let n: f64 = p.trim().parse().map_err(|_| ())?;
        if n < 0.0 { return Err(()); }
        ms = ms * 60.0 + n;
    }
    Ok(Some((ms * 1000.0).round() as u64))
}

fn fmt_time(ms: u64) -> String {
    let s = ms as f64 / 1000.0;
    let m = (s / 60.0).floor();
    let rest = s - m * 60.0;
    let r = format!("{rest:04.1}");
    format!("{m}:{}", r.strip_suffix(".0").unwrap_or(&r))
}

fn tp_open(shared: &Shared, t: &Track) {
    let (start, end) = trim_of(&shared.borrow().web.prefs, &t.key);
    TP_TRACK.with(|x| *x.borrow_mut() = Some(t.clone()));
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_tp_title(t.title.clone().into());
        ui.set_tp_artist(t.artist.clone().into());
        ui.set_tp_start(if start > 0 { fmt_time(start) } else { String::new() }.into());
        ui.set_tp_end(end.map(fmt_time).unwrap_or_default().into());
        ui.set_tp_note(format!("It's {} long.", fmt_ms(t.ms)).into());
        ui.set_tp_open(true);
    }
    tp_versions(shared, t);
}

/// Saves (or, with `reset`, takes off) the track's preferences, and shows them where it's listed
fn tp_save(shared: &Shared, reset: bool) {
    let Some(t) = TP_TRACK.with(|x| x.borrow().clone()) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let (start, end) = if reset { (None, None) } else {
        match (parse_time(&ui.get_tp_start()), parse_time(&ui.get_tp_end())) {
            (Ok(a), Ok(b)) => (a, b),
            _ => { ui.set_tp_note("Times are m:ss, like 1:05 (or empty).".into()); return; }
        }
    };
    if let (Some(a), Some(b)) = (start, end) {
        if b <= a { ui.set_tp_note("The end has to come after the start.".into()); return; }
    }
    let (title, artist) = (ui.get_tp_title().trim().to_string(), ui.get_tp_artist().trim().to_string());
    let (fixes, trims) = {
        let mut a = shared.borrow_mut();
        let mut fixes = a.web.prefs["trackFixes"].as_object().cloned().unwrap_or_default();
        let mut trims = a.web.prefs["trackTrims"].as_object().cloned().unwrap_or_default();
        if reset || (title == t.title && artist == t.artist) { if reset { fixes.remove(&t.key); } } else {
            fixes.insert(t.key.clone(), serde_json::json!({ "title": title, "artist": artist }));
        }
        if start.unwrap_or(0) == 0 && end.is_none() { trims.remove(&t.key); } else {
            trims.insert(t.key.clone(), serde_json::json!({ "start": start.unwrap_or(0), "end": end.unwrap_or(0) }));
        }
        let (fixes, trims) = (serde_json::Value::Object(fixes), serde_json::Value::Object(trims));
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("trackFixes".into(), fixes.clone()); o.insert("trackTrims".into(), trims.clone()); }
        // where it's listed: the queue and the page (its title and artist, as you want them)
        let prefs = a.web.prefs.clone();
        let fix = |x: &mut Track| if x.key == t.key { if reset { x.title = t.title.clone(); x.artist = t.artist.clone(); } else if !title.is_empty() { x.title = title.clone(); x.artist = artist.clone(); } };
        a.queue.items.iter_mut().for_each(|q| fix(&mut q.t));
        a.page_all.iter_mut().for_each(fix);
        track::apply_fixes(&prefs, &mut a.page_all);
        paint_queue(&mut a);
        apply_filter(&mut a);
        (fixes, trims)
    };
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() {
        webstate::save_pref("trackFixes", fixes);
        webstate::save_pref("trackTrims", trims);
    }
    if let Some(cur) = shared.borrow().queue.current() { if cur.key == t.key { show_now(&ui, cur); } }
    ui.set_tp_open(false);
    ui.set_status(if reset { format!("\"{}\" is back as it was.", t.title) } else { "Track preferences saved.".to_string() }.into());
}

// ---------- the Streamer Mode page (as the web app's js/streamerpage.js) ----------
// While Stream-Safe Music is on: collections of music that's safe on stream, each made when it's
// opened (searched for on every platform that's on, then only what's safe kept, each song once):
// every artist or label cleared for streaming, Creative Commons music by genre, and your safe likes.
#[derive(Clone)]
enum StreamColl { Cleared(Vec<String>), Genre(String), Likes }

thread_local! {
    static STREAM_COLLS: RefCell<Vec<Vec<(String, StreamColl)>>> = const { RefCell::new(Vec::new()) };
}

fn open_streamer(shared: &Shared) {
    let ui = shared.borrow().ui.upgrade().unwrap();
    {
        let mut a = shared.borrow_mut();
        a.page = "streamer".into();
        a.token += 1;
    }
    ui.set_page("streamer".into());
    ui.set_page_kind("discover".into());
    ui.set_page_title("Streamer Mode".into());
    ui.set_page_sub("Music that's safe on stream: artists and labels cleared for streaming, Creative Commons music, and your safe likes.".into());
    ui.set_chips(ModelRc::default());
    let prefs = shared.borrow().web.prefs.clone();
    let shelves: Vec<(String, Vec<(String, StreamColl)>)> = vec![
        ("Cleared for streaming".into(), streamsafe::cleared_on(&prefs).into_iter().map(|(n, names)| (n, StreamColl::Cleared(names))).collect()),
        ("Creative Commons".into(), streamsafe::GENRES.iter().map(|g| (g.to_string(), StreamColl::Genre(g.to_string()))).collect()),
        ("From your likes".into(), vec![("Your Safe Likes".into(), StreamColl::Likes)]),
    ];
    let shelves: Vec<_> = shelves.into_iter().filter(|(_, l)| !l.is_empty()).collect();
    let sub = |c: &StreamColl| match c { StreamColl::Cleared(_) => "Cleared for streaming", StreamColl::Genre(_) => "Creative Commons", StreamColl::Likes => "From your likes" };
    let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().map(|(_, l)| Rc::new(VecModel::from(l.iter().map(|(n, c)| Tile { title: n.clone().into(), sub: sub(c).into(), art: slint::Image::default(), plat: "".into() }).collect::<Vec<_>>()))).collect();
    let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|((t, _), m)| Shelf { title: t.clone().into(), tiles: ModelRc::from(m.clone()) }).collect();
    ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
    SHELF_MODELS.with(|m| *m.borrow_mut() = models);
    STREAM_COLLS.with(|s| *s.borrow_mut() = shelves.into_iter().map(|(_, l)| l).collect());
}

/// A collection opened: made from every platform that's on, only what's safe, each song once, mixed
fn open_stream_coll(shared: &Shared, sh: usize, k: usize) {
    let Some((name, c)) = STREAM_COLLS.with(|s| s.borrow().get(sh).and_then(|l| l.get(k).cloned())) else { return };
    let (prefs, library) = { let a = shared.borrow(); (a.web.prefs.clone(), a.library.clone()) };
    show_tracks(shared, "streamcoll", &name, "Finding music that's safe on stream...".into(), Vec::new(), false);
    load_into(shared, "streamcoll", name.clone(), async move {
        let mut found: Vec<Track> = match &c {
            StreamColl::Likes => library,
            StreamColl::Cleared(names) => futures_util::future::join_all(names.iter().map(|n| sources::search(n))).await.into_iter().flatten().collect(),
            StreamColl::Genre(g) => {
                let qs = [g.clone(), format!("{g} creative commons"), format!("{g} royalty free"), format!("{g} no copyright")];
                futures_util::future::join_all(qs.iter().map(|q| sources::search(q))).await.into_iter().flatten().filter(streamsafe::open).collect()
            }
        };
        let mut seen = std::collections::HashSet::new();
        found.retain(|t| !t.is_mix() && streamsafe::ok(&prefs, t) && seen.insert(t.key.clone()));
        queue::shuffle_in_place(&mut found);
        let sub = if found.is_empty() { "Nothing safe on stream turned up here just now.".to_string() } else { format!("{}, all safe on stream.", plural(found.len(), "track", "tracks")) };
        (found, sub)
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
    if safe_skip(shared) {
        return;
    }
    let (t, token, engine, rt, weak, resume) = {
        let mut a = shared.borrow_mut();
        let Some(t) = a.queue.current().cloned() else { return };
        a.token += 1;
        // (the last track's measurement and fade aren't this one's, while it loads)
        a.cur_loud = None;
        FADE.with(|f| *f.borrow_mut() = (false, String::new()));
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
    // (the song before plays on until this one can: the seek bar shows its progress behind a marquee)
    ui.set_track_loading(true);
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
    STATUS_HOLD.with(|h| *h.borrow_mut() = None);
    // a track a mix brought in: it's playing already (not loaded again)
    let mixed = MIXED_IN.with(|m| m.borrow_mut().take()).filter(|(k, _)| *k == t.key);
    if mixed.is_none() { MIX.with(|m| *m.borrow_mut() = None); }
    rt.spawn(async move {
        // a Go+ track: never the preview, but its version on YouTube Music (crate::versions); a
        // MusicBrainz song the same way (MusicBrainz knows it, YouTube Music has it to play)
        let mb = matches!(t.src, track::Src::Catalog(_)) || listen_needs_version(&t);
        let (src, version) = if let Some((_, v)) = mixed {
            (Ok(None), v)
        } else if t.go_plus || mb {
            match versions::find(&t).await {
                Some(v) => (player::open_source(&Track::youtube(&v.yt), h.clone()).await.map(Some), Some(v)),
                None => (Err("there's no version of it on YouTube Music".to_string()), None),
            }
        } else {
            (player::open_source(&t, h.clone()).await.map(Some), None)
        };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                if shared.borrow().token != token {
                    return;
                }
                ui.set_track_loading(false);
                match src {
                    Ok(src) => {
                        // a mix goes out of it next time only if it isn't Spotify's own playback
                        MIX_OUT_OK.with(|c| c.set(version.is_some() || !matches!(t.src, track::Src::Spotify(_))));
                        // the YouTube video it plays from (itself, or its version), for SponsorBlock
                        let video = match (&t.src, &version) {
                            (_, Some(v)) => Some(v.yt.id.clone()),
                            (track::Src::YouTube(id), None) => Some(id.clone()),
                            _ => None,
                        };
                        sb_load(&shared, video);
                        rhythm_begin(&shared, &t, 0);
                        if integrations::on(&shared.borrow().web.prefs, "mixOn") && MIX_OUT_OK.with(|c| c.get()) { rhythm_ahead(&shared, t.clone()); }
                        let what = sound::loud_source(&t, version.as_ref().map(|v| v.yt.id.as_str()));
                        shared.borrow_mut().cur_loud = what.as_ref().map(|(k, id)| format!("{k}:{id}"));
                        FADE.with(|f| *f.borrow_mut() = (false, String::new()));
                        let known = what.as_ref().and_then(|(s, id)| shared.borrow().loud.get(&format!("{s}:{id}")).copied());
                        let db = sound::loud_db(&shared.borrow().web.prefs, &t, known);
                        // the track before ending with Fade Between Tracks on: both heard at once, this one coming in
                        let resumed = resume > 0 || trim_of(&shared.borrow().web.prefs, &t.key).0 > 0;
                        match (src, XFADE.with(|x| x.borrow_mut().take())) {
                            // (mixed in: playing already)
                            (None, _) => {}
                            (Some(src), Some((ms, at))) if !resumed && at.elapsed().as_secs() < 8 => {
                                engine.fade(None, 1.0, 200);
                                engine.crossfade(src, db, ms);
                            }
                            (Some(src), _) => {
                                match fade_now(&shared.borrow()) {
                                    Some(ms) => engine.fade(Some(0.0), 1.0, ms),
                                    // (fading set or not, a new track comes up over a short moment: never a hard cut)
                                    None => engine.fade(Some(0.0), 1.0, 450),
                                }
                                engine.load(src, db);
                            }
                        }
                        if let (Some((src_kind, id)), None) = (what, known) {
                            loud_measure(&shared, &t, src_kind, id, token);
                        }
                        // where you left off, or its start time (Track preferences)
                        let at = resume.max(trim_of(&shared.borrow().web.prefs, &t.key).0);
                        if at > 0 {
                            engine.seek(at);
                        }
                        engine.play();
                        ui.set_playing(true);
                        ui.set_status("".into());
                        if casting().is_some() { omnipulse_core::cast::track(cast_meta(&shared.borrow())); }
                        // which version a Go+ track is playing; the closest one's note stays while it plays
                        if let Some(v) = &version {
                            let what = if mb { "From MusicBrainz" } else { "Go+ track" };
                            if v.exact {
                                if !mb { ui.set_status("Go+ track: playing the full song from YouTube Music.".into()); }
                            } else {
                                let note = format!("{what}: no exact match, so it's playing the closest, \"{}\" by {}, from YouTube Music.", v.yt.title, v.yt.artists);
                                ui.set_status(note.clone().into());
                                STATUS_HOLD.with(|h| *h.borrow_mut() = Some(note));
                            }
                        }
                        {
                            let mut a = shared.borrow_mut();
                            a.closest = version.filter(|v| !v.exact).map(|v| (t.key.clone(), v.yt));
                            a.started_secs = omnipulse_core::now_secs();
                            integrations::lastfm_now_playing(&a.rt, &a.web.prefs, &t);
                        }
                        push_out(&shared, true);
                    }
                    // a Go+ track (or MusicBrainz song) with no version anywhere: skipped, not its preview
                    // listening along: nothing here plays it, so a song of yours stands in for it
                    Err(_) if listening().is_some() && !STAND_IN.with(|x| x.borrow().is_some()) => stand_in(&shared, &t),
                    Err(e) if t.go_plus || matches!(t.src, track::Src::Catalog(_)) => {
                        let what = if t.go_plus { "Go+ track" } else { "from MusicBrainz" };
                        ui.set_status(format!("Skipped \"{}\" ({what}): {e}.", t.title).into());
                        let s2 = shared.clone();
                        slint::Timer::single_shot(std::time::Duration::from_millis(400), move || step(&s2, 1));
                    }
                    // age-restricted on YouTube with no other upload: skipped, saying why
                    Err(e) if e == player::AGE_BLOCKED => {
                        let note = format!("Skipped \"{}\": {e}.", t.title);
                        let s2 = shared.clone();
                        slint::Timer::single_shot(std::time::Duration::from_millis(400), move || {
                            step(&s2, 1);
                            let n = note.clone();
                            let s3 = s2.clone();
                            // (after the next one has started)
                            slint::Timer::single_shot(std::time::Duration::from_millis(1200), move || {
                                if let Some(ui) = s3.borrow().ui.upgrade() { ui.set_status(n.into()); }
                            });
                        });
                    }
                    Err(e) => ui.set_status(format!("Couldn't play \"{}\": {e}", t.title).into()),
                }
            });
        });
    });
    let prefs = shared.borrow().web.prefs.clone();
    rt.spawn(async move {
        let art = player::fetch_art(t2.art.replace("-t200x200", "-t500x500"), 500).await;
        let lyrics = player::lyrics(&t2, &prefs).await;
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

// ---------- a song's tempo, beats and key (core::rhythm), for seamless mixing ----------
// On a song's first play (with nothing known of it yet), its first 30% is listened to as it plays (the
// engine's sound, before the volume: Spotify's too) and measured: its BPM, each beat's time, the
// downbeat and its key, kept by its key in state/rhythm.json. A seek or a skip meanwhile leaves it for
// another time; a crossfade's start is waited out (the song before is in it).
const RHYTHM_PART: f64 = 0.3;
const RHYTHM_MIN_MS: u64 = 15_000;

struct RhythmEar {
    key: String,
    /// where it started listening (ms into the track), and how much it has to hear
    from_ms: Option<u64>,
    want_ms: u64,
    samples: std::sync::Arc<std::sync::Mutex<Vec<f32>>>,
    rate: u32,
}

thread_local! {
    static RHYTHM_EAR: RefCell<Option<RhythmEar>> = const { RefCell::new(None) };
}

/// Its tempo and key as Now Playing shows them ("" before they're measured)
fn rhythm_line(key: &str) -> String {
    omnipulse_core::rhythm::known(key).map(|m| format!("{:.0} BPM · {} · {}", m.rhythm.bpm, m.rhythm.key, m.rhythm.camelot)).unwrap_or_default()
}

/// What a track's measured whole is kept as (its every beat, and each bar's loudness: where its intro
/// ends and its outro starts), for a mix out of it or into it
fn rhythm_full_key(key: &str) -> String { format!("full:{key}") }

/// (one measured before each bar's beat was: measured again)
fn rhythm_full(key: &str) -> Option<omnipulse_core::rhythm::Measured> {
    omnipulse_core::rhythm::known(&rhythm_full_key(key)).filter(|m| !m.rhythm.bar_beat.is_empty())
}

/// (tracks being measured whole right now, so each is once)
static RHYTHM_FULL_BUSY: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// A track measured whole (for a mix out of it or into it), if it isn't yet, from what the mix plays:
/// its YouTube Music version's, its SoundCloud stream's, a plugin's or its file's (never Spotify's).
/// What's measured of its start is kept too, if nothing is yet.
fn rhythm_ahead(shared: &Shared, t: Track) {
    if t.key.is_empty() || t.is_mix() || rhythm_full(&t.key).is_some() { return; }
    {
        let mut busy = RHYTHM_FULL_BUSY.lock().unwrap();
        if busy.contains(&t.key) { return; }
        busy.push(t.key.clone());
    }
    let rt = shared.borrow().rt.clone();
    rt.spawn(async move {
        let key = t.key.clone();
        rhythm_measure_whole(t).await;
        RHYTHM_FULL_BUSY.lock().unwrap().retain(|k| *k != key);
    });
}

async fn rhythm_measure_whole(t: Track) {
    let (kind, id) = match &t.src {
        track::Src::YouTube(id) => ("youtube", id.clone()),
        track::Src::Local(p) => ("local", p.clone()),
        track::Src::SoundCloud(id) if !t.go_plus && omnipulse_core::soundcloud::signed_in() => ("soundcloud", id.to_string()),
        _ if mix_via_youtube(&t) => match versions::find(&t).await { Some(v) => ("youtube", v.yt.id), None => return },
        _ => return,
    };
    // (all of it: up to 20 minutes)
    let Ok((mono, rate)) = omnipulse_core::loudness::mono_start(kind, &id, 1200.0).await else { return };
    let heard = mono.len() as u64 * 1000 / rate.max(1) as u64;
    let r = tokio::task::spawn_blocking(move || omnipulse_core::rhythm::analyse(&mono, rate)).await.ok().flatten();
    if let Some(r) = r {
        let m = omnipulse_core::rhythm::Measured { from_ms: 0, to_ms: heard, rhythm: r, v: omnipulse_core::rhythm::VERSION };
        if omnipulse_core::rhythm::known(&t.key).is_none() { omnipulse_core::rhythm::keep(&t.key, m.clone()); }
        omnipulse_core::rhythm::keep(&rhythm_full_key(&t.key), m);
    }
}

/// A track has started: listened to, if nothing's known of it yet
fn rhythm_begin(shared: &Shared, t: &Track, dur_ms: u64) {
    rhythm_stop(shared);
    if t.key.is_empty() || omnipulse_core::rhythm::known(&t.key).is_some() || t.is_mix() { return; }
    let want = ((dur_ms.max(t.ms) as f64 * RHYTHM_PART) as u64).max(RHYTHM_MIN_MS);
    let rate = shared.borrow().engine.output_rate();
    RHYTHM_EAR.with(|e| *e.borrow_mut() = Some(RhythmEar { key: t.key.clone(), from_ms: None, want_ms: want, samples: Default::default(), rate }));
}

fn rhythm_stop(shared: &Shared) {
    shared.borrow().engine.set_tap("rhythm", None);
    RHYTHM_EAR.with(|e| *e.borrow_mut() = None);
}

/// Each tick: listening starts once a crossfade's over, stops at a seek, and is measured once it's heard enough
fn rhythm_tick(shared: &Shared, pos: u64) {
    let engine = shared.borrow().engine.clone();
    let cur = shared.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default();
    let step = RHYTHM_EAR.with(|e| {
        let mut e = e.borrow_mut();
        let Some(l) = e.as_mut() else { return 0 };
        if l.key != cur { return 3; }
        match l.from_ms {
            None => {
                if engine.crossfading() || engine.is_paused() { return 0; }
                l.from_ms = Some(pos);
                let buf = l.samples.clone();
                engine.set_tap("rhythm", Some(Box::new(move |st: &[f32]| {
                    if let Ok(mut b) = buf.lock() { b.extend(st.chunks(2).map(|f| (f[0] + f.get(1).copied().unwrap_or(f[0])) * 0.5)); }
                })));
                0
            }
            Some(from) => {
                let heard = l.samples.lock().map(|b| b.len()).unwrap_or(0) as u64 * 1000 / l.rate.max(1) as u64;
                // (a seek: where it is isn't where listening says it is)
                if pos.abs_diff(from + heard) > 2500 { return 3; }
                if heard >= l.want_ms { 2 } else { 0 }
            }
        }
    });
    match step {
        2 => {
            engine.set_tap("rhythm", None);
            let Some(l) = RHYTHM_EAR.with(|e| e.borrow_mut().take()) else { return };
            let from = l.from_ms.unwrap_or(0);
            let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
            rt.spawn_blocking(move || {
                let mono = std::mem::take(&mut *l.samples.lock().unwrap());
                let heard = mono.len() as u64 * 1000 / l.rate.max(1) as u64;
                if let Some(r) = omnipulse_core::rhythm::analyse(&mono, l.rate) {
                    omnipulse_core::rhythm::keep(&l.key, omnipulse_core::rhythm::Measured { from_ms: from, to_ms: from + heard, rhythm: r, v: omnipulse_core::rhythm::VERSION });
                    // (still playing: Now Playing says so)
                    let key = l.key.clone();
                    let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                        if let Some(sh) = s.borrow().clone() { if sh.borrow().queue.current().is_some_and(|t| t.key == key) { ui.set_now_rhythm(rhythm_line(&key).into()); } }
                    }));
                }
            });
        }
        3 => rhythm_stop(shared),
        _ => {}
    }
}

// ---------- SponsorBlock (as the web app's js/sponsorblock.js) ----------
// What plays from YouTube skips the non-music parts of music videos (a skit before the song, an
// interview after it) that people have marked there; asked by a hash prefix (core::sponsorblock).
thread_local! {
    /// the video playing, its parts (once known), and the last part skipped (so a seek back isn't fought)
    static SB: RefCell<(String, Option<Vec<(f64, f64)>>, String)> = const { RefCell::new((String::new(), None, String::new())) };
}

fn sb_load(shared: &Shared, video: Option<String>) {
    let id = video.unwrap_or_default();
    SB.with(|b| *b.borrow_mut() = (id.clone(), None, String::new()));
    if id.is_empty() || !integrations::on(&shared.borrow().web.prefs, "sbOn") { return; }
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        let segs = omnipulse_core::sponsorblock::music_offtopic(&id).await.unwrap_or_default();
        let _ = weak.upgrade_in_event_loop(move |_| SB.with(|b| {
            let mut b = b.borrow_mut();
            if b.0 == id { b.1 = Some(segs); }
        }));
    });
}

/// Each tick while playing: a marked part skipped. True if it moved on to the next track.
fn sb_tick(shared: &Shared, pos: u64, dur: u64) -> bool {
    if listening().is_some() || !integrations::on(&shared.borrow().web.prefs, "sbOn") { return false; }   // (listening along: the host's app skips them)
    let at = pos as f64 / 1000.0;
    let Some((key, seg)) = SB.with(|b| {
        let b = b.borrow();
        let seg = b.1.as_ref()?.iter().copied().find(|&(a, e)| at >= a && at < e - 0.5)?;
        let key = format!("{}:{}", b.0, seg.0);
        // (you went back into it yourself)
        if b.2 == key && at > seg.0 + 1.5 { return None; }
        Some((key, seg))
    }) else { return false };
    SB.with(|b| b.borrow_mut().2 = key);
    let secs = (seg.1 - at.max(seg.0)).round() as u64;
    let ui = shared.borrow().ui.upgrade();
    // a part that runs to the end: the song's over
    if dur > 0 && seg.1 * 1000.0 >= dur as f64 - 1500.0 {
        if let Some(ui) = ui { ui.set_status(format!("Skipped the last {secs} s (not music, marked on SponsorBlock).").into()); }
        step(shared, 1);
        return true;
    }
    let engine = shared.borrow().engine.clone();
    engine.seek((seg.1 * 1000.0) as u64);
    engine.fade(Some(0.0), 1.0, 450);
    if let Some(ui) = ui { ui.set_status(format!("Skipped {secs} s that isn't music (marked on SponsorBlock).").into()); }
    false
}

// ---------- seamless mixing (crate::mix) ----------
enum MixPhase {
    /// the next one (its key) being opened
    Opening(String),
    /// handed to the engine, waiting for its moment
    Waiting { key: String, plan: mix::Plan, tempo: omnipulse_audio::Tempo, version: Option<versions::Version> },
    /// coming in (and then easing back to its own tempo)
    Mixing { plan: mix::Plan, tempo: omnipulse_audio::Tempo, eased_from: Option<std::time::Instant>, ease_ms: u64 },
}

thread_local! {
    static MIX: RefCell<Option<MixPhase>> = const { RefCell::new(None) };
    /// the track the queue moves on to at a mix's moment (start() doesn't load it: it's playing), with
    /// the YouTube Music version it plays as
    static MIXED_IN: RefCell<Option<(String, Option<versions::Version>)>> = const { RefCell::new(None) };
    /// whether the track playing is one a mix can go out of (it's not Spotify's own playback)
    static MIX_OUT_OK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// the playing track listened to again near its end, for its beat there: (its key, where it started,
    /// the sound so far, its rate; or the measurement made of it)
    static MIX_EAR: RefCell<Option<(String, u64, std::sync::Arc<std::sync::Mutex<Vec<f32>>>, u32)>> = const { RefCell::new(None) };
    static MIX_LOCAL: RefCell<Option<(String, omnipulse_core::rhythm::Measured)>> = const { RefCell::new(None) };
}

/// How long the playing track is listened to again before a mix, and how long before its moment
const MIX_EAR_MS: u64 = 14_000;
const MIX_EAR_AHEAD_MS: u64 = 28_000;

/// Near a mix's moment: the playing track's beat measured again there (a song's tempo drifts; what was
/// measured of its start may be off by its end), and lined up with the first measurement's bars
fn mix_ear(shared: &Shared, key: &str, pos: u64, at_ms: u64) {
    let engine = shared.borrow().engine.clone();
    let listening_to = MIX_EAR.with(|e| e.borrow().as_ref().map(|x| x.0.clone()));
    if MIX_LOCAL.with(|l| l.borrow().as_ref().is_some_and(|x| x.0 == key)) { return; }
    match listening_to {
        None if pos + MIX_EAR_AHEAD_MS >= at_ms && pos + MIX_EAR_MS + 6000 < at_ms => {
            let buf = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
            let b2 = buf.clone();
            engine.set_tap("mixear", Some(Box::new(move |st: &[f32]| {
                if let Ok(mut b) = b2.lock() { b.extend(st.chunks(2).map(|f| (f[0] + f.get(1).copied().unwrap_or(f[0])) * 0.5)); }
            })));
            MIX_EAR.with(|e| *e.borrow_mut() = Some((key.to_string(), pos, buf, engine.output_rate())));
        }
        Some(k) if k != key => { engine.set_tap("mixear", None); MIX_EAR.with(|e| *e.borrow_mut() = None); }
        Some(_) => {
            let (from, n, rate) = MIX_EAR.with(|e| e.borrow().as_ref().map(|x| (x.1, x.2.lock().map(|b| b.len()).unwrap_or(0), x.3))).unwrap_or((0, 0, 1));
            let heard = n as u64 * 1000 / rate.max(1) as u64;
            // (a seek meanwhile: what was heard isn't one stretch)
            if pos.abs_diff(from + heard) > 2500 { engine.set_tap("mixear", None); MIX_EAR.with(|e| *e.borrow_mut() = None); return; }
            if heard < MIX_EAR_MS { return; }
            engine.set_tap("mixear", None);
            let Some((k, from, buf, rate)) = MIX_EAR.with(|e| e.borrow_mut().take()) else { return };
            let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
            rt.spawn_blocking(move || {
                let mono = std::mem::take(&mut *buf.lock().unwrap());
                let first = omnipulse_core::rhythm::known(&k);
                let local = omnipulse_core::rhythm::analyse(&mono, rate).map(|r| omnipulse_core::rhythm::Measured { from_ms: from, to_ms: from + heard, rhythm: r, v: omnipulse_core::rhythm::VERSION });
                let lined = first.zip(local).and_then(|(f, l)| mix::relined(&f, &l));
                let _ = weak.upgrade_in_event_loop(move |_| {
                    // (none: the first measurement stands, marked as looked at)
                    let m = lined.or_else(|| omnipulse_core::rhythm::known(&k));
                    if let Some(m) = m { MIX_LOCAL.with(|l| *l.borrow_mut() = Some((k, m))); }
                });
            });
        }
        None => {}
    }
}

const MIX_BARS: u32 = 8;

/// A track the mix plays from YouTube Music: Go+, a catalogue's, a Spotify one (never Spotify's own
/// playback in a mix), or one listening along can't play here
fn mix_via_youtube(t: &Track) -> bool {
    t.go_plus || matches!(t.src, track::Src::Catalog(_) | track::Src::Spotify(_)) || listen_needs_version(t)
}

/// Each tick: a mix planned and got ready as the one playing nears its moment; eased back after
fn mix_tick(shared: &Shared, pos: u64, dur: u64) {
    // coming in: once the crossfade's over, back to its own tempo over a few beats
    let easing = MIX.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(MixPhase::Mixing { plan, tempo, eased_from, ease_ms }) = m.as_mut() {
            let engine = shared.borrow().engine.clone();
            if engine.crossfading() { return true; }
            let since = *eased_from.get_or_insert_with(std::time::Instant::now);
            let t = since.elapsed().as_millis() as u64;
            tempo.set(mix::ease(plan.rate, t, *ease_ms));
            if t >= *ease_ms { *m = None; }
            return true;
        }
        m.is_some()
    });
    if easing { return; }
    let (on, listening_now, next, cur) = {
        let a = shared.borrow();
        let next = if a.repeat == "one" { None } else { a.queue.qi.and_then(|i| a.queue.items.get(i + 1)).map(|q| q.t.clone()) };
        (integrations::on(&a.web.prefs, "mixOn"), listening().is_some(), next, a.queue.current().cloned())
    };
    let (Some(next), Some(cur)) = (next, cur) else { return };
    if !on || listening_now || !MIX_OUT_OK.with(|c| c.get()) || dur == 0 || next.is_mix() { return; }
    // (where its sound ends: measured, if it's before the file's end)
    let audible = shared.borrow().cur_loud.as_ref().and_then(|k| shared.borrow().analysis.get(k).and_then(|x| x["audible_ms"].as_u64())).filter(|&x| x > 0 && x < dur).unwrap_or(dur);
    // both measured whole: every beat tracked, so no listening again
    let whole = rhythm_full(&cur.key).zip(rhythm_full(&next.key)).and_then(|(a, b)| mix::plan_structure(&a, audible, pos, &b));
    let (plan, local) = match whole {
        Some(p) => (p, Some(())),
        None => {
            let (Some(a), Some(b)) = (omnipulse_core::rhythm::known(&cur.key), omnipulse_core::rhythm::known(&next.key)) else { return };
            // (the playing one's beat as measured again near its end, once it has been)
            let local = MIX_LOCAL.with(|l| l.borrow().as_ref().filter(|x| x.0 == cur.key).map(|x| x.1.clone()));
            let a = local.clone().unwrap_or(a);
            let Some(plan) = mix::plan(&a, audible, pos, &b, MIX_BARS) else { return };
            (plan, local.map(|_| ()))
        }
    };
    // listened to again first (measured only in part), then got ready about seven seconds ahead of its moment
    if local.is_none() {
        mix_ear(shared, &cur.key, pos, plan.at_ms);
        // (too late to listen again: as it was measured)
        if pos + MIX_EAR_MS + 6000 < plan.at_ms || MIX_EAR.with(|e| e.borrow().is_some()) { return; }
    }
    if pos + 7000 < plan.at_ms { return; }
    MIX.with(|m| *m.borrow_mut() = Some(MixPhase::Opening(next.key.clone())));
    let (rt, weak, prefs) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.web.prefs.clone()) };
    let h = rt.clone();
    rt.spawn(async move {
        let (src, version) = if mix_via_youtube(&next) {
            match versions::find(&next).await {
                Some(v) => (player::open_source(&Track::youtube(&v.yt), h).await, Some(v)),
                None => (Err("no version on YouTube Music".to_string()), None),
            }
        } else {
            (player::open_source(&next, h).await, None)
        };
        let tempo = omnipulse_audio::Tempo::new(plan.rate);
        let ready = tokio::task::spawn_blocking({
            let tempo = tempo.clone();
            let start = plan.start_ms;
            let semitones = plan.semitones;
            move || -> Result<Box<dyn omnipulse_audio::Source>, String> {
                let mut st = omnipulse_audio::Stretched::shifted(src?, tempo, semitones)?;
                omnipulse_audio::Source::seek(&mut st, start)?;
                Ok(Box::new(st))
            }
        }).await.map_err(|e| e.to_string()).and_then(|r| r);
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let still = MIX.with(|m| matches!(&*m.borrow(), Some(MixPhase::Opening(k)) if *k == next.key));
            if !still { return; }
            match ready {
                Ok(src) => {
                    let what = sound::loud_source(&next, version.as_ref().map(|v| v.yt.id.as_str()));
                    let known = what.as_ref().and_then(|(s, id)| shared.borrow().loud.get(&format!("{s}:{id}")).copied());
                    let db = sound::loud_db(&prefs, &next, known);
                    shared.borrow().engine.mix_at(src, db, plan.at_ms, plan.ms, Some(mix::handoff(&plan, set_row_on(&prefs, "mixFilter", false))));
                    MIX.with(|m| *m.borrow_mut() = Some(MixPhase::Waiting { key: next.key.clone(), plan, tempo, version }));
                }
                // (it couldn't be got ready: the usual way, then)
                Err(_) => MIX.with(|m| *m.borrow_mut() = None),
            }
        }));
    });
}

/// A mix's moment came: the next track is coming in (the queue moves on, without loading it again)
fn mix_started(shared: &Shared) {
    let Some(MixPhase::Waiting { key, plan, tempo, version }) = MIX.with(|m| m.borrow_mut().take()) else { return };
    let at = shared.borrow().queue.qi.map(|i| i + 1).filter(|&n| shared.borrow().queue.items.get(n).is_some_and(|q| q.t.key == key));
    let Some(n) = at else { return };
    // (each beat of the one coming in, at its own tempo, for easing back over sixteen of them)
    let ease_ms = omnipulse_core::rhythm::known(&key).map(|m| (16.0 * m.rhythm.period_ms) as u64).unwrap_or(8000);
    MIX.with(|m| *m.borrow_mut() = Some(MixPhase::Mixing { plan, tempo, eased_from: None, ease_ms }));
    MIXED_IN.with(|m| *m.borrow_mut() = Some((key, version)));
    shared.borrow_mut().queue.qi = Some(n);
    start(shared);
}

// ---------- preloading the next track (as the web app's js/preload.js) ----------
// Once the one playing is 70% through, or has a minute left (whichever comes first), what the next one
// needs before it can start is found: its YouTube Music version (a catalogue song, a Go+ track) and its
// stream's address (yt-dlp; both kept, crate::versions and core::youtube), so it starts straight away.
// Meanwhile the marquee glides over the seek bar (which stays usable).
const PRELOAD_AT: f64 = 0.7;
const PRELOAD_LEFT_MS: u64 = 60_000;

fn preload_due(pos: u64, dur: u64) -> bool {
    pos as f64 >= dur as f64 * PRELOAD_AT || dur.saturating_sub(pos) <= PRELOAD_LEFT_MS
}

thread_local! {
    /// the track looked for last (so it's done once)
    static PRELOADED: RefCell<String> = const { RefCell::new(String::new()) };
}

fn preload_next(shared: &Shared) {
    let next = {
        let a = shared.borrow();
        if a.repeat == "one" { return; }
        match a.queue.qi {
            Some(i) if i + 1 < a.queue.items.len() => a.queue.items[i + 1].t.clone(),
            Some(_) if a.repeat == "all" && a.queue.items.len() > 1 => a.queue.items[0].t.clone(),
            _ => return,
        }
    };
    if PRELOADED.with(|p| *p.borrow() == next.key) { return; }
    PRELOADED.with(|p| *p.borrow_mut() = next.key.clone());
    if integrations::on(&shared.borrow().web.prefs, "mixOn") { rhythm_ahead(shared, next.clone()); }
    let yt_id = |t: &Track| if let track::Src::YouTube(id) = &t.src { Some(id.clone()) } else { None };
    let via_yt = next.go_plus || matches!(next.src, track::Src::Catalog(_)) || listen_needs_version(&next);
    if !via_yt && yt_id(&next).is_none() { return; }   // (the others start at once anyway)
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    if let Some(ui) = weak.upgrade() { ui.set_preloading(true); }
    rt.spawn(async move {
        let id = if via_yt { versions::find(&next).await.map(|v| v.yt.id) } else { yt_id(&next) };
        if let Some(id) = id { let _ = omnipulse_core::youtube::stream_url(&id, false).await; }
        let _ = weak.upgrade_in_event_loop(|ui| ui.set_preloading(false));
    });
}

/// The player bar's track, without playing it (a restored session shows it paused)
fn show_now(ui: &AppWindow, t: &Track) {
    paint_marks(ui, t);
    ui.set_now_title(t.title.clone().into());
    ui.set_now_artist(t.artist.clone().into());
    ui.set_now_album(t.album.clone().into());
    let apart = !t.uploader.is_empty() && omnipulse_core::ranking::artist_key(&t.uploader) != omnipulse_core::ranking::artist_key(t.artist.split([',', '&']).next().unwrap_or(""));
    ui.set_now_uploader(if apart { t.uploader.clone().into() } else { "".into() });
    ui.set_now_rhythm(rhythm_line(&t.key).into());
    ui.set_now_plat(t.plat().into());
    ui.set_now_art(slint::Image::default());
    ui.set_now_art_big(slint::Image::default());
}

/// The bar's heart and star for this track
fn paint_marks(ui: &AppWindow, t: &Track) {
    SHARED.with(|s| {
        let Some(sh) = s.borrow().clone() else { return };
        let Ok(a) = sh.try_borrow() else { return };
        ui.set_can_like(!matches!(t.src, track::Src::Local(_)));
        ui.set_now_liked(a.library.iter().any(|x| x.key == t.key));
        ui.set_now_fav(marks::favourites(&a.web.favourites).iter().any(|x| x.key == t.key));
    });
}

/// The heart: likes or unlikes what's playing, where it's from (put right back if that fails)
fn like_toggle(shared: &Shared) {
    let (t, on, rt, weak) = {
        let mut a = shared.borrow_mut();
        let Some(t) = a.queue.current().cloned() else { return };
        let on = !a.library.iter().any(|x| x.key == t.key);
        if on { a.library.insert(0, t.clone()); } else { a.library.retain(|x| x.key != t.key); }
        (t, on, a.rt.clone(), a.ui.clone())
    };
    if let Some(ui) = weak.upgrade() {
        ui.set_now_liked(on);
        ui.set_status(format!("{} \"{}\".", if on { "Liked" } else { "Took the like off" }, t.title).into());
    }
    // a requested song: the widget cheers whoever asked for it, for a few seconds
    let by = shared.borrow().queue.qi.and_then(|i| shared.borrow().queue.items.get(i).map(|q| q.by.clone())).unwrap_or_default();
    if on && !by.is_empty() {
        integrations::like_banter(by.trim_end_matches(" (Jam)"));
        push_out(shared, true);
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(8100), move || push_out(&s, true));
    }
    rt.spawn(async move {
        let r = marks::set_like(&t, on).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            if let Err(e) = r {
                let mut a = sh.borrow_mut();
                if on { a.library.retain(|x| x.key != t.key); } else { a.library.insert(0, t.clone()); }
                if a.queue.current().is_some_and(|c| c.key == t.key) { ui.set_now_liked(!on); }
                ui.set_status(format!("Couldn't {} \"{}\": {e}", if on { "like" } else { "unlike" }, t.title).into());
            }
        }));
    });
}

/// The star: a favourite (which also likes it), or not
fn fav_toggle(shared: &Shared) {
    let (t, on, liked) = {
        let a = shared.borrow();
        let Some(t) = a.queue.current().cloned() else { return };
        let on = !marks::favourites(&a.web.favourites).iter().any(|x| x.key == t.key);
        let liked = a.library.iter().any(|x| x.key == t.key);
        (t, on, liked)
    };
    marks::set_favourite(&t, on);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_now_fav(on);
        ui.set_status(format!("{} \"{}\".", if on { "Starred" } else { "Took the star off" }, t.title).into());
    }
    if on && !liked { like_toggle(shared); }
}

/// Playing from a page: a list (Library, a playlist...) becomes the queue; one track from Search
/// takes the current one's place and leaves the rest of the queue as it was
fn play_list(shared: &Shared, list: Vec<Track>, i: usize) {
    XFADE.with(|x| *x.borrow_mut() = None);
    // listening along: in a Jam, the song goes to the host to play next; otherwise playing it here
    // would mean leaving, so that's said instead (the Jam button leaves)
    if let Some(kind) = listening() {
        let note = match list.get(i) {
            Some(t) if kind == "jam" => {
                listen_send(serde_json::json!({ "type": "add", "track": t.to_snapshot(), "next": true }));
                format!("Sent \"{}\" to the Jam, to play next.", t.title)
            }
            _ => "You're listening along: leave first (the button at the top) to play your own music.".to_string(),
        };
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
        return;
    }
    let (list, i) = {
        let a = shared.borrow();
        if streamsafe::on(&a.web.prefs) {
            let picked = list.get(i).map(|t| t.key.clone());
            let list: Vec<Track> = list.into_iter().filter(|t| streamsafe::ok(&a.web.prefs, t)).collect();
            if list.is_empty() {
                if let Some(ui) = a.ui.upgrade() { ui.set_status("Stream-Safe Music is on, and nothing here is safe on stream.".into()); }
                return;
            }
            let i = picked.and_then(|k| list.iter().position(|t| t.key == k)).unwrap_or(0);
            (list, i)
        } else {
            (list, i)
        }
    };
    {
        let mut a = shared.borrow_mut();
        if a.page == "search" {
            let Some(t) = list.get(i).cloned() else { return };
            a.queue.replace_current(t);
        } else {
            a.queue.set_list(list, i);
            a.queue_spaced = a.page == "library" || a.page == "playlist";
            // shuffle on: the rest after the one picked, shuffled (in order, it plays as it is)
            if a.shuffle_on {
                let lp = shuffle_basis(&a);
                let sp = spacing(&a);
                a.queue.shuffle_rest(&lp, sp);
            }
        }
        a.ap_failed = None;
    }
    start(shared);
}

fn step(shared: &Shared, d: i32) {
    if listening().is_some() {
        return;   // listening along: the host moves on, and so does this
    }
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
        let lp = shuffle_basis(&a);
        let sp = spacing(&a);
        a.queue.shuffle_rest(&lp, sp);
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
fn recent_window(prefs: &serde_json::Value) -> u64 {
    match prefs["recentWindow"].as_str() {
        Some("day") => 86_400_000,
        Some("month") => MONTH_MS,
        _ => 7 * 86_400_000,
    }
}

/// What shuffling goes by: when you last heard each track (long-unheard first), or, with repeat on,
/// nothing (plainly random: it all comes round again anyway)
fn shuffle_basis(a: &App) -> HashMap<String, u64> {
    if a.repeat == "off" { a.web.last_played.clone() } else { HashMap::new() }
}

fn spacing(a: &App) -> Option<(u64, u64)> {
    // (with repeat on, nothing is held back)
    (a.queue_spaced && a.repeat == "off").then(|| (recent_window(&a.web.prefs), now_ms()))
}

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
    let mix_on = integrations::on(&prefs, "mixOn");
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
            let prefs = &prefs;
            move |t: &Track| (sets || !t.is_mix()) && !(avoid && lp.get(&t.key).is_some_and(|&ts| now.saturating_sub(ts) < within)) && streamsafe::allowed(prefs, t)
        };
        let mut picks = queue::take_turns(lists.clone(), last_plat, &taken, ok(window));
        if picks.is_empty() && avoid {
            picks = queue::take_turns(lists, last_plat, &taken, ok(3 * 3_600_000));
        }
        // seamless mixing: of autoplay's picks (never what you queued), the ones measured to go with
        // this one (its tempo, and its key) first, so the mix has the best chance
        if mix_on {
            if let Some(a) = omnipulse_core::rhythm::known(&seed.key) {
                picks.sort_by_key(|t| std::cmp::Reverse(mix::fit(&a, omnipulse_core::rhythm::known(&t.key).as_ref())));
            }
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
    // the narrow layout below 980 px across, as the web app's
    let w = ui.window().size().to_logical(ui.window().scale_factor()).width;
    // (with the queue open too, the full sidebar needs more room: narrow up to 1100 px then)
    let narrow = w < if ui.get_queue_open() { 1100.0 } else { 980.0 };
    if narrow != ui.get_narrow() { ui.set_narrow(narrow); }
    // status messages go after a while, as the web app's: four seconds, or seven for a problem;
    // "Loading..." ones stay until they're replaced
    STATUS_SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        let now = ui.get_status().to_string();
        if now != seen.0 {
            *seen = (now, std::time::Instant::now());
            return;
        }
        if STATUS_HOLD.with(|h| h.borrow().as_deref() == Some(now.as_str())) {
            return;
        }
        let bad = { let l = now.to_lowercase(); l.starts_with("couldn") || l.starts_with("can't") || l.contains("failed") || l.contains("error") };
        let keep = if bad { 7 } else { 4 };
        if !now.is_empty() && !now.ends_with("...") && !now.ends_with('…') && seen.1.elapsed().as_secs() >= keep {
            ui.set_status("".into());
        }
    });
    let engine = shared.borrow().engine.clone();
    if !shared.borrow().loaded {
        return; // a restored session, not started yet: the bar shows where it was
    }
    let (pos, raw_dur) = (engine.position_ms(), engine.duration_ms());
    // a track's start and end times (Track preferences): it shows, and plays, as only that part
    let (t_start, t_end) = trim_now(shared);
    let dur = t_end.unwrap_or(raw_dur);
    let (vpos, vdur) = (pos.saturating_sub(t_start), dur.saturating_sub(t_start));
    ui.set_pos_text(fmt_ms(vpos).into());
    ui.set_dur_text(fmt_ms(vdur).into());
    ui.set_dur_secs(vdur as f32 / 1000.0);
    ui.set_pos_frac(if vdur > 0 { (vpos as f32 / vdur as f32).min(1.0) } else { 0.0 });
    // its end time: on to the next one (once)
    if let Some(end) = t_end {
        let here = format!("{}:{:?}", shared.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default(), shared.borrow().queue.qi);
        if !engine.is_paused() && pos >= end && TRIM_ENDED.with(|e| *e.borrow() != here) {
            TRIM_ENDED.with(|e| *e.borrow_mut() = here);
            if shared.borrow().repeat == "one" { start(shared); } else { step(shared, 1); }
            return;
        }
    }
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
        // (when the next track would be preloaded: 70% through, or a minute left, so autoplay's first pick
        // is preloaded too)
        last && dur > 0 && preload_due(pos, dur) && !a.ap_busy
    };
    if near_end {
        autoplay_fill(shared, false);
    }
    if !engine.is_paused() && dur > 0 && preload_due(pos, dur) {
        preload_next(shared);
    }
    if !engine.is_paused() && fade_and_silences(shared, pos, dur) {
        return;
    }
    if !engine.is_paused() && sb_tick(shared, pos, dur) {
        return;
    }
    rhythm_tick(shared, pos);
    if !engine.is_paused() { mix_tick(shared, pos, dur); }
    while let Ok(ev) = events.try_recv() {
        match ev {
            Event::Mixed => mix_started(shared),
            Event::Ended if shared.borrow().repeat == "one" => start(shared),
            Event::Ended if shared.borrow().closest.is_some() => {
                let (done, cur) = { let a = shared.borrow(); (a.closest.clone(), a.queue.current().cloned()) };
                if let (Some((key, yt)), Some(t)) = (done, cur) {
                    if t.key == key {
                        versions::keep(&t, &yt);
                        ui.set_status(format!("\"{}\" plays that version from now on.", t.title).into());
                    }
                }
                shared.borrow_mut().closest = None;
                step(shared, 1);
            }
            Event::Ended => step(shared, 1),
            Event::Error(e) => ui.set_status(e.into()),
        }
    }
}

thread_local! {
    /// the track (and its place in the queue) its end time has just ended, so it's done once
    static TRIM_ENDED: RefCell<String> = const { RefCell::new(String::new()) };
}

/// A track's start and end times, as the web app keeps them (prefs.trackTrims[id] = { start, end }, ms)
fn trim_of(prefs: &serde_json::Value, key: &str) -> (u64, Option<u64>) {
    let x = &prefs["trackTrims"][key];
    let start = x["start"].as_f64().unwrap_or(0.0).max(0.0) as u64;
    let end = x["end"].as_f64().filter(|e| *e > 0.0).map(|e| e as u64);
    (start, end.filter(|e| *e > start))
}

/// The playing track's (the host's while listening along: it plays as it is)
fn trim_now(shared: &Shared) -> (u64, Option<u64>) {
    let a = shared.borrow();
    match a.queue.current() {
        Some(t) => trim_of(&a.web.prefs, &t.key),
        None => (0, None),
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
    ui.set_dur_secs(t.ms as f32 / 1000.0);
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
        let by = a.queue.qi.and_then(|i| a.queue.items.get(i)).map(|q| q.by.as_str()).unwrap_or("");
        integrations::widget_update(prefs, t, by, pos, dur, playing, (&a.accent.0, &a.accent.1));
    }
    if presence {
        let join = listen_url(shared, &a, playing);
        integrations::presence(&a.rt, prefs, t, pos, playing, join);
    }
    host_push(&a);
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
        // its cover, for the Play / Pause keys (a size that fills one)
        "art": t.map(|t| t.art.replace("-t200x200", "-t300x300")).filter(|u| u.starts_with("http")).unwrap_or_default(),
        "shuffle": a.shuffle_on,
        "repeat": a.repeat,
        "liked": t.is_some_and(|t| a.library.iter().any(|x| x.key == t.key)),
        "canLike": t.is_some_and(|t| !matches!(t.src, track::Src::Local(_))),
        "favourite": t.is_some_and(|t| marks::favourites(&a.web.favourites).iter().any(|x| x.key == t.key)),
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
        "next" => skip_press(shared, 1),
        "previous" => skip_press(shared, -1),
        "shuffle" => {
            let mut a = shared.borrow_mut();
            let lp = shuffle_basis(&a);
            let sp = spacing(&a);
            a.queue.shuffle_rest(&lp, sp);
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

/// !wrongsong (as the web app's): takes back the viewer's last request; a moderator's takes back the
/// last one anyone made. One that's playing already is skipped.
fn wrong_song(shared: &Shared, id: u64, user: &str, is_mod: bool) {
    let reply = move |v: serde_json::Value| omnipulse_core::requests::request_result(id, v);
    let prefs = shared.borrow().web.prefs.clone();
    let who = if user.trim().is_empty() { "chat".to_string() } else { user.trim().to_string() };
    let base = vec![("user", who.clone())];
    if !integrations::set(&prefs, "req") { reply(req_fail(&prefs, &base, "closed", vec![])); return; }
    // the last one asked for: requests are queued in turn, so the last of them in the queue
    let found = {
        let a = shared.borrow();
        let from = a.queue.qi.unwrap_or(0);
        a.queue.items.iter().enumerate().skip(from).filter(|(_, q)| !q.by.is_empty() && (is_mod || q.by.eq_ignore_ascii_case(&who))).map(|(i, q)| (i, q.t.clone(), q.by.clone())).next_back()
    };
    let Some((i, t, by)) = found else {
        reply(req_fail(&prefs, &base, if is_mod { "noRequests" } else { "noRequest" }, vec![]));
        return;
    };
    if Some(i) == shared.borrow().queue.qi {
        step(shared, 1);
    } else {
        let mut a = shared.borrow_mut();
        a.queue.remove(i);
        paint_queue(&mut a);
        save_session(&a);
    }
    let vars: Vec<(&str, String)> = base.iter().cloned().chain([("title", t.title.clone()), ("artist", t.artist.clone()), ("by", by)]).collect();
    reply(serde_json::json!({ "ok": true, "title": t.title, "artist": t.artist, "message": requests::msg(&prefs, "wrongSong", &vars) }));
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
    // Limit Requests (on unless switched off): the most waiting, and the most per viewer
    let limited = integrations::on(&prefs, "reqLimits");
    if limited && pending.len() as u64 >= prefs["reqMax"].as_u64().unwrap_or(10) { reply(req_fail(&prefs, &base, "full", vec![])); return; }
    let mine = pending.iter().filter(|b| **b == who.to_lowercase()).count();
    if limited && !user.trim().is_empty() && mine as u64 >= prefs["reqPerUser"].as_u64().unwrap_or(2) {
        reply(req_fail(&prefs, &base, "perUser", vec![("count", mine.to_string())]));
        return;
    }
    let likes: Vec<Track> = a.library.iter().filter(|t| t.plat() != "local").cloned().collect();
    let prefer_radio = integrations::set(&prefs, "reqRadioEdit");
    let (rt, weak) = (a.rt.clone(), a.ui.clone());
    drop(a);
    rt.spawn(async move {
        let safe = { let p = prefs.clone(); move |t: &Track| streamsafe::allowed(&p, t) };
        let picked = requests::pick(&q, &plats, &likes, &local, prefer_radio, &safe).await;
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
                    // already coming up: it moves up, after the other requests (unless it's playing, or
                    // someone's requested it already)
                    let from = a.queue.qi.unwrap_or(0);
                    let at = a.queue.items.iter().enumerate().skip(from + 1).find(|(_, x)| x.t.key == t.key).map(|(i, x)| (i, x.by.is_empty()));
                    let playing = a.queue.qi.and_then(|i| a.queue.items.get(i)).is_some_and(|x| x.t.key == t.key);
                    if playing || at.is_some_and(|(_, free)| !free) {
                        reply(serde_json::json!({ "ok": false, "message": requests::msg(&prefs, "duplicate", &vars) }));
                        return;
                    }
                    if let Some((i, _)) = at { a.queue.remove(i); }
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
// ---------- Settings: the switches beyond Look ----------
// (section, label, note, key, on unless switched off)
const SET_ROWS: [(&str, &str, &str, &str, bool); 21] = [
    ("Library", "Sets & Podcasts in the Queue", "Lists and autoplay include anything over 15 minutes. Off: only when you pick one.", "queueSets", false),
    ("Library", "Autoplay", "Plays similar tracks when the queue runs out.", "autoplay", true),
    ("Library", "Seamless Mix", "Songs with a beat blend into the next on it, as a DJ mixes: at its tempo, on a downbeat, the bass handed over at once. With songs that play from YouTube Music or your files (never Spotify's own playback), once their beat is known.", "mixOn", true),
    ("Library", "Filter Sweep Transitions", "Seamless Mix hands over through filters instead: the song playing thins out as it goes, and the next comes in thin and opens up on the drop.", "mixFilter", false),
    ("Library", "Skip non-music parts", "What plays from YouTube skips the parts of music videos that aren't music (a skit before the song, an interview after it), as marked on SponsorBlock.", "sbOn", true),
    ("Library", "Favourites Page", "Your starred tracks on a page of their own, in the sidebar. The stars stay either way.", "favPage", true),
    ("Integrations", "SoundCloud", "Its likes, playlists and search.", "scOn", true),
    ("Integrations", "Spotify", "Its likes, playlists and search.", "spOn", true),
    ("Integrations", "YouTube Music", "Search, radio, and Go+ tracks in full.", "ytOn", true),
    ("Integrations", "Deezer and Apple Music catalogues", "Open music catalogues, no account needed: search, Discover and artist pages find their songs, and they play from YouTube Music.", "catOn", true),
    ("Integrations", "MusicBrainz", "An open music catalogue: search finds its songs too, and they play from YouTube Music.", "mbOn", true),
    ("Integrations", "Listen Along & Jams", "Listen together with friends, from your Discord status or a Jam invite. Off: no Jam button, and invites and requests are turned away (song requests aren't affected).", "jamsOn", true),
    ("Integrations", "Last.fm Scrobbling", "What you play goes to your Last.fm profile.", "scrobble", true),
    ("Streaming", "Streamer Mode", "Hides account names, keys, addresses and plugin settings. The title bar's button does this too.", "streamerMode", false),
    ("Streaming", "Extended", "Also leaves out music that isn't safe on stream: only open licences, and artists and labels cleared for streaming.", "streamSafe", false),
    ("Streaming", "Discord Status", "Shows what you're playing on your Discord profile.", "presence", false),
    ("Streaming", "Stream Widget", "A now-playing card for OBS, with alerts.", "widget", false),
    ("Streaming", "Song Requests", "Chat asks for songs through your bot.", "req", false),
    ("Streaming", "Limit Requests", "The most waiting, and the most per viewer. Off: chat can request as many songs as it likes.", "reqLimits", true),
    ("Streaming", "OmniPulse Offset", "Your music a second time, as its own app (\"OmniPulse Offset\"), to send to a stream mix only. Its offset and volume are the web app's (Settings > Streaming) for now.", "out2On", false),
    ("Streaming", "Stream Deck & Remote", "Control the app from a Stream Deck or another device.", "remoteOn", false),
];

fn set_row_on(prefs: &serde_json::Value, key: &str, dflt: bool) -> bool {
    prefs[key].as_bool().unwrap_or(dflt)
}

fn paint_settings(a: &App) {
    let Some(ui) = a.ui.upgrade() else { return };
    let streamer = integrations::set(&a.web.prefs, "streamerMode");
    let rows = |sec: &str| -> ModelRc<SetRow> {
        let mut list: Vec<SetRow> = SET_ROWS.iter().filter(|(s, ..)| *s == sec)
            // Extended only shows with Streamer Mode on
            .filter(|(_, _, _, key, _)| *key != "streamSafe" || streamer)
            .map(|(_, label, note, key, dflt)| {
                let on = if *key == "autoplay" { a.autoplay } else { set_row_on(&a.web.prefs, key, *dflt) };
                SetRow { head: Default::default(), label: (*label).into(), note: (*note).into(), on, key: (*key).into(), sub: *key == "streamSafe", more: false }
            }).collect();
        // the plugins installed (the web app adds and removes them; each one switches on and off here)
        if sec == "Integrations" && !streamer {
            for p in omnipulse_core::plugins::list() {
                let m = &p.manifest;
                let state = if !plugin_on(&a.web.prefs, &m.id) { "Off.".to_string() } else if plugrun::get(&m.id).is_some() { "Running.".to_string() } else { PLUG_ERR.with(|e| e.borrow().get(&m.id).cloned()).map(|e| format!("Couldn't start: {e}")).unwrap_or_else(|| "Starting...".into()) };
                let note = format!("{}{}{}", if m.description.is_empty() { String::new() } else { format!("{} ", m.description) }, if p.bundled { "Comes with OmniPulse. " } else { "" }, state);
                let more = m.settings.as_array().is_some_and(|l| !l.is_empty());
                list.push(SetRow { head: Default::default(), label: format!("Plugin: {}", m.name).into(), note: note.into(), on: plugin_on(&a.web.prefs, &m.id), key: format!("plugOn_{}", m.id).into(), sub: false, more });
            }
        }
        ModelRc::from(Rc::new(VecModel::from(list)))
    };
    ui.set_set_rows_integrations(rows("Integrations"));
    ui.set_set_rows_library(rows("Library"));
    ui.set_set_rows_streaming(rows("Streaming"));
    // the closed categories (the web app's setting too: by their titles)
    let closed = |t: &str| a.web.prefs["setClosed"][t].as_bool() == Some(true);
    ui.set_closed_look(closed("App & Look"));
    ui.set_closed_integrations(closed("Integrations"));
    ui.set_closed_library(closed("Library"));
    ui.set_closed_sound(closed("Sound"));
    ui.set_closed_streaming(closed("Streaming"));
}

fn section_flip(shared: &Shared, title: &str) {
    {
        let mut a = shared.borrow_mut();
        let now = a.web.prefs["setClosed"][title].as_bool() != Some(true);
        if !a.web.prefs["setClosed"].is_object() { a.web.prefs["setClosed"] = serde_json::json!({}); }
        a.web.prefs["setClosed"][title] = serde_json::json!(now);
        let all = a.web.prefs["setClosed"].clone();
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("setClosed", all); }
    }
    paint_settings(&shared.borrow());
}

/// A switch in Settings: kept with this app's settings, and done straight away
/// OmniPulse Offset started, changed or stopped as its settings say (crate::offset)
fn offset_apply(shared: &Shared) {
    let (engine, prefs) = { let a = shared.borrow(); (a.engine.clone(), a.web.prefs.clone()) };
    let note = match offset::apply(&engine, &prefs) {
        Ok(d) if d.is_empty() => return,
        Ok(d) => format!("OmniPulse Offset is playing on {d}."),
        Err(e) => e,
    };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
}

thread_local! {
    /// the plugin whose settings are open, and what they're set to meanwhile
    static PF_EDIT: RefCell<Option<(String, serde_json::Map<String, serde_json::Value>)>> = const { RefCell::new(None) };
}

/// A plugin's settings dialog: its manifest's fields, as they're set (prefs.pluginSettings, as the web app's)
fn pf_open(shared: &Shared, id: &str) {
    let Some(p) = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id) else { return };
    let now = plugrun::settings_of(&p.manifest, &shared.borrow().web.prefs);
    let vals = now.as_object().cloned().unwrap_or_default();
    PF_EDIT.with(|e| *e.borrow_mut() = Some((id.to_string(), vals)));
    pf_paint(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_pf_head(format!("{} settings", p.manifest.name).into());
        ui.set_pf_open(true);
    }
}

fn pf_fields(id: &str) -> Vec<serde_json::Value> {
    omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id).and_then(|p| p.manifest.settings.as_array().cloned()).unwrap_or_default()
}

fn pf_paint(shared: &Shared) {
    let Some((id, vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    let text = |v: &serde_json::Value| match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Null => String::new(), x => x.to_string() };
    let rows: Vec<PlugField> = pf_fields(&id).iter().filter_map(|f| {
        let key = f["key"].as_str()?.to_string();
        let kind = f["type"].as_str().unwrap_or("text").to_string();
        let options: Vec<slint::SharedString> = f["options"].as_array().into_iter().flatten().map(|o| text(o).into()).collect();
        Some(PlugField { label: f["label"].as_str().unwrap_or(&key).into(), note: f["note"].as_str().unwrap_or("").into(), value: vals.get(&key).map(text).unwrap_or_default().into(), kind: kind.into(), options: ModelRc::from(Rc::new(VecModel::from(options))), key: key.into() })
    }).collect();
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_fields(ModelRc::from(Rc::new(VecModel::from(rows)))); }
}

/// One setting changed in the dialog (a select's "next": on to its next option)
fn pf_set(shared: &Shared, i: usize, v: &str) {
    let Some((id, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    let Some(f) = pf_fields(&id).get(i).cloned() else { return };
    let Some(key) = f["key"].as_str().map(String::from) else { return };
    let value = match f["type"].as_str().unwrap_or("text") {
        "toggle" => serde_json::json!(v == "true"),
        "number" => v.trim().parse::<f64>().map(|n| serde_json::json!(n)).unwrap_or(serde_json::Value::Null),
        "select" => {
            let opts: Vec<serde_json::Value> = f["options"].as_array().cloned().unwrap_or_default();
            let at = opts.iter().position(|o| Some(o) == vals.get(&key)).map_or(0, |k| (k + 1) % opts.len().max(1));
            opts.get(at).cloned().unwrap_or_default()
        }
        _ => serde_json::json!(v),
    };
    vals.insert(key, value);
    let select = f["type"].as_str() == Some("select") || f["type"].as_str() == Some("toggle");
    PF_EDIT.with(|e| *e.borrow_mut() = Some((id, vals)));
    if select { pf_paint(shared); }   // (typed ones aren't drawn again while they're typed in)
}

/// Saved: the plugin starts again with them
fn pf_save(shared: &Shared) {
    let Some((id, vals)) = PF_EDIT.with(|e| e.borrow_mut().take()) else { return };
    let all = {
        let mut a = shared.borrow_mut();
        let mut all = a.web.prefs["pluginSettings"].as_object().cloned().unwrap_or_default();
        all.insert(id.clone(), serde_json::Value::Object(vals));
        let all = serde_json::Value::Object(all);
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pluginSettings".into(), all.clone()); }
        all
    };
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pluginSettings", all); }
    plugrun::stop(&id);
    plugins_start(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); ui.set_status("Plugin settings saved.".into()); }
}

/// A plugin: on unless switched off (the web app's prefs.plugOn_<id>)
fn plugin_on(prefs: &serde_json::Value, id: &str) -> bool {
    prefs[format!("plugOn_{id}")].as_bool() != Some(false)
}

thread_local! {
    /// why a plugin didn't start, by its id
    static PLUG_ERR: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// The installed plugins that are on, started (each on its own thread: crate::plugrun); the rest stopped
fn plugins_start(shared: &Shared) {
    let (prefs, rt) = { let a = shared.borrow(); (a.web.prefs.clone(), a.rt.clone()) };
    for p in omnipulse_core::plugins::list() {
        let id = p.manifest.id.clone();
        if !plugin_on(&prefs, &id) { plugrun::stop(&id); continue; }
        if plugrun::get(&id).is_some() { continue; }
        match plugrun::start(&p, &plugrun::settings_of(&p.manifest, &prefs), rt.clone()) {
            Ok(_) => PLUG_ERR.with(|e| e.borrow_mut().remove(&id)),
            Err(e) => PLUG_ERR.with(|x| x.borrow_mut().insert(id, e)),
        };
    }
    paint_settings(&shared.borrow());
}

fn set_toggle(shared: &Shared, key: &str) {
    if let Some(id) = key.strip_prefix("plugOn_") {
        let on = {
            let mut a = shared.borrow_mut();
            let on = !plugin_on(&a.web.prefs, id);
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert(key.into(), serde_json::json!(on)); }
            on
        };
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(key, serde_json::json!(on)); }
        plugins_start(shared);
        return;
    }
    let Some(&(_, label, _, key, dflt)) = SET_ROWS.iter().find(|r| r.3 == key) else { return };
    match key {
        "autoplay" => {
            let on = !shared.borrow().autoplay;
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_autoplay(on); ui.invoke_autoplay_changed(on); }
        }
        "streamSafe" => { let on = !streamsafe::on(&shared.borrow().web.prefs); safe_set(shared, on); }
        // Streamer Mode: Extended goes off with it
        "streamerMode" => {
            let on = !integrations::set(&shared.borrow().web.prefs, "streamerMode");
            if let Some(o) = shared.borrow_mut().web.prefs.as_object_mut() { o.insert("streamerMode".into(), serde_json::json!(on)); }
            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("streamerMode", serde_json::json!(on)); }
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_streamer(on); ui.set_status(if on { "Streamer Mode on." } else { "Streamer Mode off." }.into()); }
            if !on && streamsafe::on(&shared.borrow().web.prefs) { safe_set(shared, false); }
        }
        "scOn" | "spOn" | "ytOn" => plat_toggle(shared, match key { "spOn" => "spotify", "ytOn" => "youtube", _ => "soundcloud" }),
        _ => {
            let on = {
                let mut a = shared.borrow_mut();
                let on = !set_row_on(&a.web.prefs, key, dflt);
                if let Some(o) = a.web.prefs.as_object_mut() { o.insert(key.into(), serde_json::json!(on)); }
                on
            };
            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(key, serde_json::json!(on)); }
            // the servers stop, and start again with what's set now
            match key {
                "widget" => omnipulse_core::widget::widget_stop(),
                "req" => omnipulse_core::requests::requests_stop(),
                "remoteOn" => omnipulse_core::remote::remote_stop(),
                "presence" if !on => integrations::presence(&shared.borrow().rt, &shared.borrow().web.prefs, None, 0, false, None),
                _ => {}
            }
            if matches!(key, "widget" | "req" | "remoteOn") && on { outside_start(shared); }
            if key == "mbOn" { go_plus_set(&shared.borrow().web.prefs); }
            if key == "out2On" { offset_apply(shared); }
            // Listen Along & Jams off: out of any room, and the Jam button goes
            if key == "jamsOn" {
                if !on { ASK.with(|x| x.borrow_mut().take()); room_end(true); push_out(shared, true); }
                jam_paint(shared);
            }
            if key == "favPage" {
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_fav_page(on); }
                if !on && shared.borrow().page == "favourites" { open_page(shared, "library"); }
            }
            if key == "presence" && on { push_out(shared, true); }
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{label} {}.", if on { "on" } else { "off" }).into()); }
        }
    }
    paint_settings(&shared.borrow());
}

/// The Library and your playlists, from every platform you're signed in to (at the start, and again
/// after signing in or out)
fn library_load(shared: &Shared) {
    let (weak, rt) = { let a = shared.borrow(); (a.ui.clone(), a.rt.clone()) };
    {
        let yt_likes = shared.borrow().web.yt_likes.clone();
        let own = xpl::playlists(&shared.borrow().xpl);
        rt.spawn(async move {
            let t0 = std::time::Instant::now();
            let (sc, sp, lists) = tokio::join!(sources::soundcloud_likes(), sources::spotify_likes(500), sources::playlists());
            let mut all: Vec<Track> = Vec::new();
            all.extend(sc);
            all.extend(sp);
            all.extend(yt_likes);
            // YouTube Music likes made here (the web app's are already in)
            all.extend(marks::yt_likes());
            let mut seen = std::collections::HashSet::new();
            all.retain(|t| seen.insert(t.key.clone()));
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
                    sync_all(&shared, true);
                    if SYNC_EVERY.with(|x| x.borrow().is_none()) {
                        let s2 = shared.clone();
                        let every = slint::Timer::default();
                        every.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(600), move || sync_all(&s2, true));
                        SYNC_EVERY.with(|x| *x.borrow_mut() = Some(every));
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
}

// ---------- Settings > Your Music: the folders your own files are in ----------
fn folders_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let folders = omnipulse_core::local::local_folders();
    let st = omnipulse_core::local::local_status();
    let n = st["count"].as_u64().unwrap_or(0) as usize;
    ui.set_folders_note(if st["scanning"].as_bool() == Some(true) {
        format!("Looking through them... {} so far.", plural(n, "file", "files"))
    } else if folders.is_empty() {
        "Folders on this PC or a network drive. Their music shows in your Library.".to_string()
    } else {
        format!("{} found.", plural(n, "file", "files"))
    }.into());
    ui.set_folders(ModelRc::from(Rc::new(VecModel::from(folders.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()))));
}

thread_local! {
    static SCAN_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
}

/// Looks through the folders again; the Library loads again once that's done
fn folders_rescan(shared: &Shared) {
    omnipulse_core::local::local_scan();
    folders_paint(shared);
    let s = shared.clone();
    let t = slint::Timer::default();
    t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(700), move || {
        folders_paint(&s);
        if omnipulse_core::local::local_status()["scanning"].as_bool() != Some(true) {
            library_load(&s);
            SCAN_TIMER.with(|x| *x.borrow_mut() = None);
        }
    });
    SCAN_TIMER.with(|x| *x.borrow_mut() = Some(t));
}

fn folders_set(shared: &Shared, folders: Vec<String>) {
    match omnipulse_core::local::local_set_folders(folders) {
        Ok(_) => folders_rescan(shared),
        Err(e) => { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(e.into()); } }
    }
}

// ---------- Settings > Accounts ----------
// (key, name): SoundCloud and Spotify for your library and search, Spotify's own sign-in for playing
// it in full (Premium), your YouTube account for likes there, and Last.fm for scrobbling
// (Spotify is one sign-in, as the web app's: its own, the playback engine's, for Premium; a free account
// signs in through your own developer app instead: crates/core/src/spotify.rs)
const ACCOUNTS: [(&str, &str); 4] = [("soundcloud", "SoundCloud"), ("spotify", "Spotify"), ("youtube", "YouTube Account"), ("lastfm", "Last.fm")];

/// Spotify only lets Premium accounts sign in its own way
/// set once Spotify has turned its own sign-in down (a free account): the next sign-in is your app's
static SP_FREE_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const SP_FREE: &str = "That's a free Spotify account. Spotify only lets Premium accounts sign in straight away; for a free one, your own free Spotify app keys are set up first (a couple of minutes), then it signs in.";

thread_local! {
    /// who's signed in where (as the platforms say), and the sign-ins under way
    static ACCOUNT_NAMES: RefCell<HashMap<&'static str, String>> = RefCell::new(HashMap::new());
    static ACCOUNT_BUSY: RefCell<std::collections::HashSet<&'static str>> = RefCell::new(std::collections::HashSet::new());
}

fn account_signed(key: &str) -> bool {
    match key {
        "soundcloud" => omnipulse_core::soundcloud::signed_in(),
        "spotify" => omnipulse_core::spotify::spotify_signed_in(),
        "youtube" => omnipulse_core::yt_account::status()["connected"].as_bool() == Some(true),
        _ => omnipulse_core::lastfm::lastfm_status()["connected"].as_bool() == Some(true),
    }
}

fn accounts_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let rows: Vec<Account> = ACCOUNTS.iter().map(|&(key, name)| {
        let signed = account_signed(key);
        let busy = ACCOUNT_BUSY.with(|b| b.borrow().contains(key));
        let who = ACCOUNT_NAMES.with(|n| n.borrow().get(key).cloned()).filter(|s| !s.is_empty());
        let status = if busy {
            "Finish signing in in your browser.".to_string()
        } else if signed {
            match (key, who) {
                ("spotify", Some(w)) if omnipulse_core::spotify_engine::spe_signed_in() => format!("Signed in as {w} · Premium: Spotify plays in full here."),
                ("spotify", Some(w)) => format!("Signed in as {w} · Free: Spotify tracks play from YouTube Music."),
                (_, Some(w)) => format!("Signed in as {w}."),
                _ => "Signed in.".to_string(),
            }
        } else {
            match key {
                "soundcloud" => "Your SoundCloud likes, playlists and search.".to_string(),
                "spotify" => "Your Spotify likes, playlists and search, and with Premium, Spotify playing in full here.".to_string(),
                "youtube" if omnipulse_core::yt_account::status()["configured"].as_bool() != Some(true) => "Optional: your likes on YouTube too. It needs your own free Google keys first: Sign in sets them up.".to_string(),
                "youtube" => "Likes here become likes on YouTube and YouTube Music.".to_string(),
                _ if omnipulse_core::lastfm::lastfm_status()["configured"].as_bool() != Some(true) => "What you play on your Last.fm profile. It needs your own free Last.fm keys first: Sign in sets them up.".to_string(),
                _ => "What you play goes to your Last.fm profile.".to_string(),
            }
        };
        Account { name: name.into(), status: status.into(), signed, busy }
    }).collect();
    ui.set_accounts(ModelRc::from(Rc::new(VecModel::from(rows))));
}

/// Who's signed in, asked of the platforms (SoundCloud's and Spotify's names; the others keep theirs)
fn accounts_refresh(shared: &Shared) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    ACCOUNT_NAMES.with(|n| {
        let mut n = n.borrow_mut();
        if let Some(name) = omnipulse_core::yt_account::status()["name"].as_str() { n.insert("youtube", name.to_string()); }
        if let Some(name) = omnipulse_core::lastfm::lastfm_status()["name"].as_str() { n.insert("lastfm", name.to_string()); }
    });
    accounts_paint(shared);
    rt.spawn(async move {
        let sc = if omnipulse_core::soundcloud::signed_in() { omnipulse_core::soundcloud::api_get("/me".into()).await.ok().and_then(|v| v["username"].as_str().map(String::from)) } else { None };
        let sp = if omnipulse_core::spotify::spotify_signed_in() { omnipulse_core::spotify::spotify_api("GET".into(), "/me".into(), None).await.ok().and_then(|v| v["display_name"].as_str().or(v["id"].as_str()).map(String::from)) } else { None };
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            ACCOUNT_NAMES.with(|n| {
                let mut n = n.borrow_mut();
                n.insert("soundcloud", sc.unwrap_or_default());
                n.insert("spotify", sp.unwrap_or_default());
            });
            if let Some(shared) = s.borrow().clone() { accounts_paint(&shared); }
        }));
    });
}

/// An account's button: signs in (in your browser) or out
fn account_act(shared: &Shared, i: usize) {
    let Some(&(key, name)) = ACCOUNTS.get(i) else { return };
    if ACCOUNT_BUSY.with(|b| b.borrow().contains(key)) {
        return;
    }
    let out = account_signed(key);
    // signing in where your own keys are needed first: the key setup (then the sign-in)
    let plat = match key { "youtube" => Some("youtube"), "lastfm" => Some("lastfm"), _ => None };
    if let Some(plat) = plat.filter(|p| !out && !keysetup::has_keys(p)) {
        return ks_open(shared, plat, i);
    }
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    ACCOUNT_BUSY.with(|b| b.borrow_mut().insert(key));
    accounts_paint(shared);
    rt.spawn(async move {
        let done: Result<(), String> = match (key, out) {
            ("soundcloud", false) => omnipulse_core::soundcloud::login().await,
            ("soundcloud", true) => omnipulse_core::soundcloud::logout(),
            // Premium: Spotify's own sign-in; a free account (which it turns down): your developer app's
            ("spotify", false) if SP_FREE_SEEN.load(std::sync::atomic::Ordering::Relaxed) && keysetup::has_keys("spotify") => omnipulse_core::spotify::spotify_login().await,
            ("spotify", false) => match omnipulse_core::spotify_engine::spe_login().await {
                Err(e) if e.to_lowercase().contains("premium account required") => {
                    SP_FREE_SEEN.store(true, std::sync::atomic::Ordering::Relaxed);
                    if keysetup::has_keys("spotify") { omnipulse_core::spotify::spotify_login().await } else { Err(SP_FREE.to_string()) }
                }
                other => other,
            },
            ("spotify", true) => {
                let _ = omnipulse_core::spotify_engine::spe_logout().await;
                omnipulse_core::spotify::spotify_logout()
            }
            ("youtube", false) => omnipulse_core::yt_account::connect().await.map(|_| ()),
            ("youtube", true) => omnipulse_core::yt_account::disconnect().await,
            (_, false) => omnipulse_core::lastfm::lastfm_connect().await.map(|_| ()),
            (_, true) => omnipulse_core::lastfm::lastfm_disconnect(),
        };
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            ACCOUNT_BUSY.with(|b| b.borrow_mut().remove(key));
            let Some(shared) = s.borrow().clone() else { return };
            if matches!(&done, Err(e) if e == SP_FREE) {
                ui.set_status(SP_FREE.into());
                accounts_refresh(&shared);
                return ks_open(&shared, "spotify", i);
            }
            ui.set_status(match &done {
                Ok(()) if out => format!("Signed out of {name}."),
                Ok(()) => format!("Signed in to {name}."),
                Err(e) => format!("{name}: {e}"),
            }.into());
            accounts_refresh(&shared);
            // your library and playlists, from what you're signed in to now
            if done.is_ok() && matches!(key, "soundcloud" | "spotify") {
                paint_plats(&shared.borrow());
                library_load(&shared);
            }
        }));
    });
}

// ---------- your own keys (crate::keysetup) ----------
thread_local! {
    /// the platform being set up, the account it signs in to after, and the keys typed so far
    static KS: RefCell<Option<(String, usize, Vec<String>)>> = const { RefCell::new(None) };
}

fn ks_open(shared: &Shared, plat: &str, account: usize) {
    let Some(s) = keysetup::setup(plat) else { return };
    KS.with(|k| *k.borrow_mut() = Some((plat.to_string(), account, vec![String::new(); s.fields.len()])));
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    ui.set_ks_head(format!("Set up {}: your own keys", s.name).into());
    ui.set_ks_note(format!("{} They're kept on this PC, in OmniPulse's own folder.", s.note).into());
    let steps: Vec<KeyStep> = s.steps.iter().map(|st| KeyStep { text: st.text.into(), has_url: st.url.is_some(), copies: ModelRc::from(Rc::new(VecModel::from(st.copies.iter().map(|(l, _)| slint::SharedString::from(*l)).collect::<Vec<_>>()))) }).collect();
    ui.set_ks_steps(ModelRc::from(Rc::new(VecModel::from(steps))));
    ks_paint(shared);
    ui.set_ks_open(true);
}

fn ks_paint(shared: &Shared) {
    let Some((plat, _, vals)) = KS.with(|k| k.borrow().clone()) else { return };
    let Some(s) = keysetup::setup(&plat) else { return };
    let fields: Vec<KeyField> = s.fields.iter().zip(&vals).map(|((_, label, re), v)| KeyField { label: (*label).into(), value: v.clone().into(), ok: keysetup::looks_right(re, v) }).collect();
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_ks_fields(ModelRc::from(Rc::new(VecModel::from(fields)))); }
}

fn ks_save(shared: &Shared) {
    let Some((plat, account, vals)) = KS.with(|k| k.borrow().clone()) else { return };
    let Some(s) = keysetup::setup(&plat) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if s.fields.iter().zip(&vals).any(|((_, _, re), v)| !keysetup::looks_right(re, v)) {
        ui.set_ks_note("Not yet: each key has to look right (a tick beside it) before it's saved.".into());
        return;
    }
    let mut fields = serde_json::Map::new();
    for ((key, _, _), v) in s.fields.iter().zip(&vals) { fields.insert((*key).into(), serde_json::json!(v.trim())); }
    match omnipulse_core::config_set(&fields) {
        Ok(()) => {
            KS.with(|k| k.borrow_mut().take());
            ui.set_ks_open(false);
            ui.set_status(format!("Your {} keys are saved. Signing in...", s.name).into());
            account_act(shared, account);
        }
        Err(e) => ui.set_ks_note(format!("Couldn't save them: {e}").into()),
    }
}

// ---------- Fade Between Tracks and Skip Silences (as the web app's js/fade.js and js/loudness.js) ----------
thread_local! {
    /// fading out at the end (and the track that's moved on from, so it's only once)
    static FADE: RefCell<(bool, String)> = const { RefCell::new((false, String::new())) };
}

/// How early a crossfade starts asking for the next track's sound (it's preloaded: about this long)
const XFADE_LEAD_MS: u64 = 1000;

thread_local! {
    /// the next track's start comes in as a crossfade of this many ms (set as the one playing ends)
    /// (and when: it's for the track that comes next, even past ones Stream-Safe Music skips, for a moment)
    static XFADE: RefCell<Option<(u64, std::time::Instant)>> = const { RefCell::new(None) };
}

/// Each tick while playing: long silences jumped over, the fade out at the end. True if it moved on.
fn fade_and_silences(shared: &Shared, pos: u64, dur: u64) -> bool {
    let (an, key, fade, skip, engine) = {
        let a = shared.borrow();
        let Some(t) = a.queue.current() else { return false };
        let an = a.cur_loud.as_ref().and_then(|k| a.analysis.get(k)).cloned();
        (an, t.key.clone(), fade_out_ms(&a), sound::skip_silence_ms(&a.web.prefs), a.engine.clone())
    };
    let Some(an) = an else { return false };
    if dur == 0 {
        return false;
    }
    // a silence of the chosen length or more: over it, or on to the next track if it runs to the end
    // (listening along: the host's app skips them, or doesn't, and this follows it)
    if skip > 0 && listening().is_none() {
        let measured = an["analysed_ms"].as_u64().unwrap_or(0);
        for s in an["silences"].as_array().into_iter().flatten() {
            let (from, to) = (s[0].as_u64().unwrap_or(0), s[1].as_u64().unwrap_or(0));
            if to.saturating_sub(from) < skip || pos < from || pos + 500 >= to {
                continue;
            }
            if to + 1500 >= dur && measured + 1500 >= dur {
                step(shared, 1);
                return true;
            }
            engine.seek(to);
            host_push(&shared.borrow());   // (hosting: listeners jump with it)
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Skipped {} s of silence.", (to - from) / 1000).into()); }
            return false;
        }
    }
    let Some(ms) = fade else { return false };
    // where the sound ends: its measured end, if that's before the file's
    let audible = an["audible_ms"].as_u64().unwrap_or(0);
    let end = if audible > 0 && audible < dur { audible } else { dur };
    let left = end as i64 - pos as i64;
    let (out, moved) = FADE.with(|f| f.borrow().clone());
    // Fade Between Tracks as a true crossfade: the next one comes in over this one's last moments (both
    // heard at once), started a second early for its sound to be found (it's preloaded). Not listening
    // along (the host's app leads), and not repeating this one.
    if MIX.with(|m| m.borrow().is_some()) || shared.borrow().engine.mixing() { return false; }
    let next_up = { let a = shared.borrow(); a.repeat != "one" && a.queue.qi.is_some_and(|i| i + 1 < a.queue.items.len()) };
    if next_up && listening().is_none() && !out && left > 0 && left as u64 <= ms + XFADE_LEAD_MS && moved != key {
        FADE.with(|f| *f.borrow_mut() = (true, key.clone()));
        XFADE.with(|x| *x.borrow_mut() = Some((ms, std::time::Instant::now())));
        step(shared, 1);
        return true;
    }
    if !out && left > 0 && left as u64 <= ms {
        engine.fade(None, 0.0, left as u64);
        FADE.with(|f| f.borrow_mut().0 = true);
    } else if out && left > ms as i64 + 400 {
        // sought back from the end: up again
        engine.fade(None, 1.0, 200);
        FADE.with(|f| f.borrow_mut().0 = false);
    } else if out && left <= 0 && end + 300 < dur && moved != key {
        // faded out where the sound ends: on, without the quiet tail
        FADE.with(|f| f.borrow_mut().1 = key);
        step(shared, 1);
        return true;
    }
    false
}

// ---------- Settings > Sound (crate::sound) ----------
/// Measures a track's loudness, then plays it at the gain for that (if it's still the one playing)
fn loud_measure(shared: &Shared, t: &Track, kind: &'static str, id: String, token: u64) {
    {
        let p = &shared.borrow().web.prefs;
        if sound::loud_step(p) == "off" && sound::fade_ms(p).is_none() && sound::skip_silence_ms(p) == 0 {
            return;
        }
    }
    let (rt, weak, t) = (shared.borrow().rt.clone(), shared.borrow().ui.clone(), t.clone());
    rt.spawn(async move {
        let Ok(a) = omnipulse_core::loudness::analyse(kind, &id).await else { return };
        let Some(lufs) = a["lufs"].as_f64() else { return };
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            shared.borrow_mut().loud.insert(format!("{kind}:{id}"), lufs);
            shared.borrow_mut().analysis.insert(format!("{kind}:{id}"), a.clone());
            let a = shared.borrow();
            if a.token == token {
                a.engine.set_match_db(sound::loud_db(&a.web.prefs, &t, Some(lufs)), false);
            }
        }));
    });
}

/// The Sound settings shown, and the equalizer as heard
fn sound_paint(a: &App) {
    let Some(ui) = a.ui.upgrade() else { return };
    ui.set_loud_on(sound::loud_step(&a.web.prefs).into());
    ui.set_eq_on(a.web.prefs["eqOn"].as_bool() == Some(true));
    ui.set_fade_on(a.web.prefs["fadeOn"].as_bool() == Some(true));
    ui.set_fade_ms(a.web.prefs["fadeMs"].as_u64().unwrap_or(3000) as i32);
    ui.set_skip_silence((sound::skip_silence_ms(&a.web.prefs) / 1000) as i32);
    let bands = sound::eq_bands(&a.web.prefs);
    for (i, g) in bands.iter().enumerate() {
        if a.eq_model.row_data(i) != Some(*g) { a.eq_model.set_row_data(i, *g); }
    }
    a.engine.set_eq(&sound::eq_heard(&a.web.prefs));
}

fn sound_set(shared: &Shared, key: &str, v: serde_json::Value) {
    {
        let mut a = shared.borrow_mut();
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert(key.into(), v.clone()); }
    }
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(key, v); }
    sound_paint(&shared.borrow());
}

fn loud_pick(shared: &Shared, step: &str) {
    sound_set(shared, "loudMatch", serde_json::json!(step));
    // what's playing: at the new target straight away
    let (t, token) = { let a = shared.borrow(); (a.queue.current().cloned(), a.token) };
    let Some(t) = t else { return };
    let ver = shared.borrow().closest.as_ref().filter(|(k, _)| *k == t.key).map(|(_, v)| v.id.clone());
    let what = sound::loud_source(&t, ver.as_deref());
    let known = what.as_ref().and_then(|(s, id)| shared.borrow().loud.get(&format!("{s}:{id}")).copied());
    { let a = shared.borrow(); a.engine.set_match_db(sound::loud_db(&a.web.prefs, &t, known), false); }
    if let (Some((kind, id)), None) = (what, known) { loud_measure(shared, &t, kind, id, token); }
}

// ---------- your playlists on this PC (crate::xpl) ----------
/// The playlists changed: kept, and shown again (the sidebar, the menus, and the page if it's one)
fn xpl_changed(shared: &Shared) {
    xpl_shown(shared);
    sync_soon(shared, 2000);
}

/// Kept and shown again, without keeping the platforms in step (that's done already)
fn xpl_shown(shared: &Shared) {
    let reopen = {
        let mut a = shared.borrow_mut();
        xpl::save(&a.xpl);
        let mut lists = xpl::playlists(&a.xpl);
        lists.extend(a.playlists.drain(..).filter(|p| !xpl::is_own(&p.id)));
        a.playlists = lists;
        paint_sides(&mut a);
        match &a.here { Some(Place::Playlist(id)) => Some(id.clone()), _ => None }
    };
    if let Some(id) = reopen {
        open_place(shared, &Place::Playlist(id));
    }
}

/// A new playlist: made straight away, here, taking songs from any platform (and kept on SoundCloud
/// and Spotify by itself, crate::mirror). Right-click it to rename it.
fn pl_create(shared: &Shared, tracks: Vec<Track>) {
    let name = {
        let taken: std::collections::HashSet<String> = shared.borrow().playlists.iter().map(|p| p.title.clone()).collect();
        let mut name = "New Playlist".to_string();
        let mut n = 2;
        while taken.contains(&name) { name = format!("New Playlist {n}"); n += 1; }
        name
    };
    let id = xpl::create(&mut shared.borrow_mut().xpl, &name, &tracks);
    xpl_changed(shared);
    if tracks.is_empty() {
        go(shared, Place::Playlist(id));
    }
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_status(format!("Created \"{name}\"{}. Right-click it in the sidebar to rename it.", if tracks.is_empty() { "" } else { " with that track in it" }).into());
    }
}

// ---------- playlists as files (as the web app's js/playlistfiles.js, core::playlistfile) ----------
fn plat_name(plat: &str) -> &'static str {
    match plat { "soundcloud" => "SoundCloud", "spotify" => "Spotify", "youtube" => "YouTube Music", "local" => "Your files", "deezer" => "Deezer", "itunes" => "Apple Music", "musicbrainz" => "MusicBrainz", _ => "Plugin" }
}

/// Any playlist as an M3U (each track's file or link) or a CSV (title, artist, album, length, platform, link)
fn pl_export(shared: &Shared, p: sources::Playlist, kind: &str) {
    let safe: String = p.title.chars().map(|c| if "<>:\"/\\|?*".contains(c) { ' ' } else { c }).collect();
    let safe = if safe.trim().is_empty() { "Playlist".to_string() } else { safe.trim().to_string() };
    let dlg = rfd::FileDialog::new().set_title(format!("Export {}", p.title)).set_file_name(format!("{safe}.{}", if kind == "csv" { "csv" } else { "m3u8" }));
    let dlg = if kind == "csv" { dlg.add_filter("CSV", &["csv"]) } else { dlg.add_filter("M3U playlist", &["m3u8", "m3u"]) };
    let Some(dest) = dlg.save_file() else { return };
    let (rt, weak, prefs) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.web.prefs.clone()) };
    if let Some(ui) = weak.upgrade() { ui.set_status("Exporting...".into()); }
    let csv = kind == "csv";
    rt.spawn(async move {
        let mut list = sources::playlist_tracks(&p).await;
        track::apply_fixes(&prefs, &mut list);
        let rows: Vec<omnipulse_core::playlistfile::Row> = list.iter().map(|t| omnipulse_core::playlistfile::Row {
            title: t.title.clone(), artist: t.artist.clone(), album: t.album.clone(), ms: t.ms, isrc: String::new(),
            platform: plat_name(t.plat()).to_string(), link: t.web_link(),
            path: if let track::Src::Local(path) = &t.src { path.clone() } else { String::new() },
        }).collect();
        let text = if csv { omnipulse_core::playlistfile::csv_text(&rows) } else { omnipulse_core::playlistfile::m3u_text(&p.title, &rows) };
        let done = omnipulse_core::playlist_file_write(&dest.to_string_lossy(), &text);
        let n = rows.len();
        let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(match done {
            Ok(()) => format!("Exported \"{}\": {}.", p.title, plural(n, "song", "songs")),
            Err(e) => format!("Couldn't save it: {e}"),
        }.into()));
    });
}

/// A YouTube video's id in a link (youtube.com/watch?v=, music.youtube.com, youtu.be/)
fn yt_link_id(link: &str) -> Option<String> {
    let id = link.split_once("v=").map(|(_, r)| r).or_else(|| link.split_once("youtu.be/").map(|(_, r)| r))?;
    let id: String = id.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (id.len() == 11).then_some(id)
}

/// A song in a playlist file: the file itself, the link's own track, or else found by its title and
/// artist (in the catalogues, then YouTube Music)
async fn entry_track(e: &omnipulse_core::playlistfile::Entry, files: &[Track], catalog: bool) -> Option<Track> {
    if !e.path.is_empty() {
        let want = e.path.replace('/', "\\").to_lowercase();
        if let Some(f) = files.iter().find(|t| matches!(&t.src, track::Src::Local(p) if p.replace('/', "\\").to_lowercase() == want)) { return Some(f.clone()); }
    }
    let link = e.link.as_str();
    if !link.is_empty() {
        if let Some(id) = yt_link_id(link) {
            return Some(Track { src: track::Src::YouTube(id.clone()), key: format!("yt:{id}"), title: if e.title.is_empty() { "Untitled".into() } else { e.title.clone() }, artist: e.artist.clone(), ms: e.ms, ..Default::default() });
        }
        let sp = link.split("track/").nth(1).or_else(|| link.strip_prefix("spotify:track:")).map(|r| r.chars().take(22).collect::<String>());
        if let Some(id) = sp.filter(|id| id.len() == 22 && link.contains("spotify")) {
            if let Ok(v) = omnipulse_core::spotify::spotify_api("GET".into(), format!("/tracks/{id}?market=from_token"), None).await {
                if let Some(t) = Track::spotify(&v) { return Some(t); }
            }
        }
        if link.contains("soundcloud.com/") && omnipulse_core::soundcloud::signed_in() {
            if let Ok(v) = omnipulse_core::soundcloud::api_get(format!("/resolve?url={}", sources::urlencode(link.split(['?', '#']).next().unwrap_or(link)))).await {
                if let Some(t) = Track::soundcloud(&v) { return Some(t); }
            }
        }
        if let Some(id) = link.split("deezer.com/").nth(1).and_then(|r| r.split("track/").nth(1)).map(|r| r.chars().take_while(|c| c.is_ascii_digit()).collect::<String>()).filter(|id| !id.is_empty()) {
            if let Ok(v) = omnipulse_core::catalog::deezer(&format!("/track/{id}")).await {
                if let Some(t) = omnipulse_core::catalog::dz_track(&v, None).and_then(|x| Track::catalog(&x)) { return Some(t); }
            }
        }
    }
    if e.title.is_empty() { return None; }
    let want = track::song_key(&Track { title: e.title.clone(), artist: e.artist.clone(), ..Default::default() });
    let q = format!("{} {}", e.artist, e.title);
    if catalog {
        if let Ok(found) = omnipulse_core::catalog::search(q.trim()).await {
            if let Some(t) = found.iter().filter_map(Track::catalog).find(|t| track::song_key(t) == want) { return Some(t); }
        }
    }
    let found = omnipulse_core::youtube::search(q.trim()).await.ok()?;
    found.iter().map(Track::youtube).find(|t| track::song_key(t) == want)
}

fn pl_import(shared: &Shared) {
    let Some(src) = rfd::FileDialog::new().set_title("Import a playlist").add_filter("Playlist", &["m3u", "m3u8", "csv", "txt"]).pick_file() else { return };
    pl_import_file(shared, src);
}

fn pl_import_file(shared: &Shared, src: std::path::PathBuf) {
    let path = src.to_string_lossy().into_owned();
    let text = match omnipulse_core::playlist_file_read(&path) {
        Ok(t) => t,
        Err(e) => { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Couldn't read that file: {e}").into()); } return; }
    };
    let (name, entries) = if path.to_lowercase().ends_with(".csv") { (String::new(), omnipulse_core::playlistfile::csv_entries(&text)) } else { omnipulse_core::playlistfile::m3u_entries(&text) };
    let ui = shared.borrow().ui.upgrade();
    if entries.is_empty() { if let Some(ui) = ui { ui.set_status("No songs found in that file.".into()); } return; }
    let name = if name.is_empty() { src.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Imported".into()) } else { name };
    let (rt, weak, files, catalog) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.library.iter().filter(|t| t.plat() == "local").cloned().collect::<Vec<_>>(), catalog_on(&a.web.prefs)) };
    if let Some(ui) = ui { ui.set_status(format!("Importing \"{name}\": {}...", plural(entries.len(), "song", "songs")).into()); }
    rt.spawn(async move {
        let mut tracks = Vec::new();
        let mut missing = Vec::new();
        for e in &entries {
            match entry_track(e, &files, catalog).await {
                Some(t) => tracks.push(t),
                None => missing.push(if e.artist.is_empty() { e.title.clone() } else { format!("{} - {}", e.artist, e.title) }),
            }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if tracks.is_empty() { ui.set_status("None of its songs were found anywhere.".into()); return; }
            let id = xpl::create(&mut shared.borrow_mut().xpl, &name, &tracks);
            xpl_changed(&shared);
            go(&shared, Place::Playlist(id));
            let note = format!("Imported \"{name}\": {}{}", plural(tracks.len(), "song", "songs"), if missing.is_empty() { ".".to_string() } else { format!(", {} found anywhere (copied, to look for).", plural(missing.len(), "song wasn't", "songs weren't")) });
            if !missing.is_empty() { let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(missing.join("\n"))); }
            ui.set_status(note.into());
            STATUS_HOLD.with(|h| *h.borrow_mut() = None);
        }));
    });
}

// ---------- keeping them on the platforms (crate::mirror) ----------
thread_local! {
    static SYNC_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    static SYNC_EVERY: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    static JAM_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    static SYNC_BUSY: std::cell::Cell<(bool, bool)> = const { std::cell::Cell::new((false, false)) };
    static SYNC_SAID: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
}

fn sync_soon(shared: &Shared, ms: u64) {
    let s = shared.clone();
    let t = slint::Timer::default();
    t.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(ms), move || sync_all(&s, false));
    SYNC_TIMER.with(|x| *x.borrow_mut() = Some(t));
}

fn sync_plats(prefs: &serde_json::Value) -> Vec<&'static str> {
    let mut on = Vec::new();
    if plat_on(prefs, "soundcloud") && omnipulse_core::soundcloud::signed_in() { on.push("soundcloud"); }
    if plat_on(prefs, "spotify") && omnipulse_core::spotify::spotify_signed_in() { on.push("spotify"); }
    on
}

/// Every playlist here, on the platforms (pull: and what changed over there brought in)
fn sync_all(shared: &Shared, pull: bool) {
    // (never in the self-test: it would make playlists on your accounts)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() {
        return;
    }
    let (busy, _) = SYNC_BUSY.with(|b| b.get());
    if busy {
        SYNC_BUSY.with(|b| b.set((true, true)));
        return;
    }
    SYNC_BUSY.with(|b| b.set((true, false)));
    let (all, on, rt, weak) = { let a = shared.borrow(); (a.xpl.clone(), sync_plats(&a.web.prefs), a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        let mut done = Vec::new();
        for p in all.as_array().cloned().unwrap_or_default() {
            let before = json_sig(&p);
            let mut p = p;
            let out = mirror::sync_one(&mut p, &on, pull).await;
            done.push((before, p, out));
        }
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            sync_apply(&shared, done);
            let (_, again) = SYNC_BUSY.with(|b| b.get());
            SYNC_BUSY.with(|b| b.set((false, false)));
            if again { sync_soon(&shared, 500); }
        }));
    });
}

fn json_sig(p: &serde_json::Value) -> String {
    serde_json::json!([p["title"], p["tracks"]]).to_string()
}

fn sync_apply(shared: &Shared, done: Vec<(String, serde_json::Value, mirror::Outcome)>) {
    let mut changed = false;
    let mut again = false;
    let mut hide: Vec<String> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    {
        let mut a = shared.borrow_mut();
        let Some(list) = a.xpl.as_array_mut() else { return };
        for (before, p, out) in done {
            let Some(mine) = list.iter_mut().find(|x| x["id"] == p["id"]) else { continue };
            if json_sig(mine) == before {
                *mine = p;
                changed |= out.changed;
            } else {
                // changed here meanwhile: what was learned is kept, and it's written again
                for k in ["mirrors", "made"] { if p[k].is_object() { mine[k] = p[k].clone(); } }
                if p["follow"].is_object() { mine["follow"]["seen"] = p["follow"]["seen"].clone(); }
                again = true;
            }
            hide.extend(out.hide);
            notes.extend(out.errors);
        }
    }
    if !hide.is_empty() {
        let mut a = shared.borrow_mut();
        let mut h: Vec<serde_json::Value> = a.web.prefs["hiddenPlaylists"].as_array().cloned().unwrap_or_default();
        for x in hide { if !h.iter().any(|y| y.as_str() == Some(x.as_str())) { h.push(serde_json::json!(x)); } }
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("hiddenPlaylists".into(), serde_json::json!(h.clone())); }
        webstate::save_pref("hiddenPlaylists", serde_json::json!(h));
    }
    xpl::save(&shared.borrow().xpl);
    if changed { xpl_shown(shared); } else { paint_sides(&mut shared.borrow_mut()); }
    if let Some(n) = notes.into_iter().find(|n| SYNC_SAID.with(|s| s.borrow_mut().insert(n.clone()))) {
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(n.into()); }
    }
    if again { sync_soon(shared, 2000); }
}

fn hidden_set(shared: &Shared, id: &str, hide: bool) {
    let mut a = shared.borrow_mut();
    let mut h: Vec<serde_json::Value> = a.web.prefs["hiddenPlaylists"].as_array().cloned().unwrap_or_default();
    h.retain(|y| y.as_str() != Some(id));
    if hide { h.push(serde_json::json!(id)); }
    if let Some(o) = a.web.prefs.as_object_mut() { o.insert("hiddenPlaylists".into(), serde_json::json!(h.clone())); }
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("hiddenPlaylists", serde_json::json!(h)); }
}

/// A song added to one of the platforms' playlists: as it is, if it's yours and the song is from
/// there; otherwise that playlist becomes one here (crate::mirror::adopt)
fn pl_add_platform(shared: &Shared, p: sources::Playlist, t: Track) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let native = p.own && t.plat() == p.plat;
    rt.spawn(async move {
        if native {
            let done = match &t.src {
                track::Src::Spotify(uri) => omnipulse_core::spotify::spotify_api("POST".into(), format!("/playlists/{}/items", p.id), Some(serde_json::json!({ "uris": [uri] }))).await.map(|_| ()),
                track::Src::SoundCloud(n) => {
                    let n = *n;
                    match omnipulse_core::soundcloud::api_get(format!("/playlists/{}?show_tracks=true", p.id)).await {
                        Ok(v) => {
                            let mut ids: Vec<u64> = v["tracks"].as_array().into_iter().flatten().filter_map(|x| x["id"].as_u64()).collect();
                            if !ids.contains(&n) { ids.push(n); }
                            let tracks: Vec<serde_json::Value> = ids.iter().map(|x| serde_json::json!({ "id": x })).collect();
                            omnipulse_core::soundcloud::playlist_write("PUT".into(), format!("/playlists/{}", p.id), Some(serde_json::json!({ "playlist": { "tracks": tracks } }))).await.map(|_| ())
                        }
                        Err(e) => Err(e),
                    }
                }
                _ => Ok(()),
            };
            let note = match done { Ok(()) => format!("Added to {}.", p.title), Err(e) => format!("Could not add to the playlist: {e}") };
            let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(note.into()));
            return;
        }
        let copy = mirror::adopt(&p).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let id = copy["id"].as_str().unwrap_or("").to_string();
            {
                let mut a = shared.borrow_mut();
                if let Some(list) = a.xpl.as_array_mut() { list.insert(0, copy); } else { a.xpl = serde_json::json!([copy]); }
                xpl::add(&mut a.xpl, &id, std::slice::from_ref(&t));
            }
            collab_send(&shared, &id, collab::add_op(&t));
            hidden_set(&shared, &mirror::side_id(p.plat, &p.id), true);
            xpl_changed(&shared);
            let where_ = if p.plat == "spotify" { "Spotify" } else { "SoundCloud" };
            // (said once: from now on it's a playlist here)
            ui.set_status(if p.own { format!("\"{}\" now takes songs from any platform, and stays up to date on {where_}.", p.title) } else { format!("You can't edit this playlist on {where_}, so your changes are saved on this PC only.") }.into());
            let showing = matches!(&shared.borrow().here, Some(Place::Playlist(x)) if *x == p.id);
            if showing { go(&shared, Place::Playlist(id)); }
        }));
    });
}

// ---------- collaborative playlists (crate::collab, as the web app's js/collab.js) ----------
thread_local! {
    static COLLAB_PULLED: RefCell<HashMap<String, std::time::Instant>> = RefCell::new(HashMap::new());
}

fn collab_of(all: &serde_json::Value, id: &str) -> Option<serde_json::Value> {
    all.as_array()?.iter().find(|x| x["id"].as_str() == Some(id)).filter(|x| collab::is_collab(x)).map(|x| x["collab"].clone())
}

fn xpl_set(shared: &Shared, id: &str, f: impl FnOnce(&mut serde_json::Value)) {
    let mut a = shared.borrow_mut();
    if let Some(x) = a.xpl.as_array_mut().and_then(|l| l.iter_mut().find(|x| x["id"].as_str() == Some(id))) { f(x); }
}

/// Invite collaborators: a platform playlist is copied to this PC first (its original stays as it is);
/// the playlist is shared (if it isn't yet), made collaborative, and the invite link copied
fn collab_invite(shared: &Shared, p: sources::Playlist) {
    if let Some(c) = collab_of(&shared.borrow().xpl, &p.id) {
        if c["owner"].as_bool() != Some(true) { return share_status(shared, "You're collaborating on this playlist. Right-click it to leave.".into()); }
    }
    let (relay, by, rt, weak) = {
        let a = shared.borrow();
        let relay = a.web.prefs["relayUrl"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(share::RELAY_URL).trim_end_matches('/').to_string();
        (relay, my_name(&a), a.rt.clone(), a.ui.clone())
    };
    let was = shared.borrow().xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(p.id.as_str())).map(|x| x["share"].clone())).unwrap_or_default();
    share_status(shared, "Making it collaborative...".into());
    rt.spawn(async move {
        // (a platform playlist: its copy on this PC is what's shared)
        let copy = if xpl::is_own(&p.id) { None } else { Some(mirror::adopt(&p).await) };
        let pl = match &copy { Some(c) => xpl::playlists(&serde_json::json!([c.clone()])).into_iter().next().unwrap_or(p.clone()), None => p.clone() };
        let tracks = sources::playlist_tracks(&pl).await;
        let shared_at = share::share_playlist(&relay, &pl.title, &by, &tracks, &was).await;
        let done = match shared_at { Ok((_, info)) => collab::invite(&info).await.map(|(link, c)| (link, c, info)), Err(e) => Err(e) };
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            match done {
                Ok((link, c, info)) => {
                    if let Some(c) = copy.clone() {
                        let mut a = shared.borrow_mut();
                        if let Some(list) = a.xpl.as_array_mut() { list.insert(0, c); } else { a.xpl = serde_json::json!([c]); }
                    }
                    let id = pl.id.clone();
                    // (an invite again keeps the playlist's revision)
                    let had = collab_of(&shared.borrow().xpl, &id);
                    xpl_set(&shared, &id, |x| { x["share"] = info; x["collab"] = had.unwrap_or(c); });
                    if copy.is_some() { hidden_set(&shared, &mirror::side_id(p.plat, &p.id), true); }
                    xpl_changed(&shared);
                    share_status(&shared, match share::copy(&link) {
                        Ok(()) => format!("Invite link copied: friends who open it can add songs to \"{}\" and take them out, from their own OmniPulse.", pl.title),
                        Err(e) => e,
                    });
                }
                Err(e) => share_status(&shared, format!("Couldn't make it collaborative: {e}")),
            }
        }));
    });
}

/// Leaving (or, its owner, stopping it): the playlist stays here as yours
fn collab_leave(shared: &Shared, id: &str) {
    let Some(c) = collab_of(&shared.borrow().xpl, id) else { return };
    let owner = c["owner"].as_bool() == Some(true);
    let title = shared.borrow().playlists.iter().find(|p| p.id == id).map(|p| p.title.clone()).unwrap_or_default();
    let ask = if owner { format!("Stop \"{title}\" being collaborative? Everyone keeps a copy, but changes no longer go between you.") } else { format!("Leave \"{title}\"? You keep it as yours, but your changes and theirs no longer go between you.") };
    let sure = std::env::var_os("OMNIPULSE_SELFTEST").is_some()
        || rfd::MessageDialog::new().set_title(if owner { "Stop collaborating" } else { "Leave collaboration" }).set_description(ask).set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
    if !sure { return; }
    if owner { let c2 = c.clone(); shared.borrow().rt.spawn(async move { collab::stop(&c2).await; }); }
    xpl_set(shared, id, |x| { if let Some(o) = x.as_object_mut() { o.remove("collab"); } });
    xpl_changed(shared);
    share_status(shared, if owner { format!("\"{title}\" isn't collaborative any more.") } else { format!("You left \"{title}\". It's kept as yours.") });
}

/// A change here, to everyone in it (only up to date if it was the only one since; else theirs fetched)
fn collab_send(shared: &Shared, id: &str, op: serde_json::Value) {
    let Some(c) = collab_of(&shared.borrow().xpl, id) else { return };
    let (rt, weak, id) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), id.to_string()) };
    rt.spawn(async move {
        let r = collab::send(&c, op).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            match r {
                Ok(rev) if rev > 0 && rev == c["rev"].as_u64().unwrap_or(0) + 1 => { xpl_set(&shared, &id, |x| x["collab"]["rev"] = serde_json::json!(rev)); xpl::save(&shared.borrow().xpl); }
                Ok(rev) if rev > 0 => collab_pull(&shared, &id),
                Ok(_) => {}
                Err(e) => ui.set_status(format!("Couldn't send that change to everyone in the playlist: {e}").into()),
            }
        }));
    });
}

/// Everyone else's changes: the playlist as it is on the relay (your own files kept where they were)
fn collab_pull(shared: &Shared, id: &str) {
    let Some(c) = collab_of(&shared.borrow().xpl, id) else { return };
    let (Some(relay), Some(code)) = (c["relay"].as_str().map(String::from), c["code"].as_str().map(String::from)) else { return };
    let (rt, weak, id) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), id.to_string()) };
    rt.spawn(async move {
        let Ok(share) = collab::fetch(&relay, &code).await else { return };   // (offline: next time)
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let Some(x) = shared.borrow().xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(id.as_str())).cloned()) else { return };
            match collab::merged(&x, &share) {
                Err(()) => {
                    xpl_set(&shared, &id, |x| { if let Some(o) = x.as_object_mut() { o.remove("collab"); } });
                    xpl_shown(&shared);
                    ui.set_status(format!("\"{}\" isn't collaborative any more: it's kept as yours.", x["title"].as_str().unwrap_or("")).into());
                }
                Ok(None) => {}
                Ok(Some((tracks, title, rev))) => {
                    xpl_set(&shared, &id, |x| { x["tracks"] = serde_json::json!(tracks); x["title"] = serde_json::json!(title); x["collab"]["rev"] = serde_json::json!(rev); });
                    xpl_shown(&shared);
                }
            }
        }));
    });
}

fn collab_pull_all(shared: &Shared) {
    let ids: Vec<String> = shared.borrow().xpl.as_array().into_iter().flatten().filter(|x| collab::is_collab(x)).filter_map(|x| x["id"].as_str().map(String::from)).collect();
    for id in ids { collab_pull(shared, &id); }
}

/// Opening an invite (omnipulse://playlist/<code>?e=<edit key>): joined, as one of your playlists (or
/// the one you joined already, opened)
fn collab_join(shared: &Shared, code: String, relay: String, e: String) {
    let had = shared.borrow().xpl.as_array().into_iter().flatten().find(|x| x["collab"]["code"].as_str() == Some(code.as_str())).and_then(|x| x["id"].as_str().map(String::from));
    if let Some(id) = had { go(shared, Place::Playlist(id.clone())); return collab_pull(shared, &id); }
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        let share = collab::fetch(&relay, &code).await.ok().filter(|s| s["tracks"].is_array());
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let Some(share) = share else { ui.set_status("That playlist isn't shared any more.".into()); return };
            let title = share["title"].as_str().unwrap_or("Playlist").to_string();
            let by = share["by"].as_str().map(|b| format!(" from {b}")).unwrap_or_default();
            let sure = std::env::var_os("OMNIPULSE_SELFTEST").is_some()
                || rfd::MessageDialog::new().set_title("Join playlist").set_description(format!("Join \"{title}\"{by}? It's a collaborative playlist: you can add songs to it and take them out, and see everyone else's changes.")).set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
            if !sure { return; }
            let tracks: Vec<Track> = share["tracks"].as_array().into_iter().flatten().filter_map(|t| Track::from_snapshot(&t["snap"])).filter(|t| t.plat() != "local").collect();
            let id = xpl::create(&mut shared.borrow_mut().xpl, &title, &tracks);
            xpl_set(&shared, &id, |x| x["collab"] = serde_json::json!({ "code": code, "relay": relay, "k": e, "owner": false, "rev": share["rev"].as_u64().unwrap_or(0) }));
            xpl_changed(&shared);
            go(&shared, Place::Playlist(id));
            ui.set_status(format!("Joined \"{title}\". Your changes go to everyone in it, and theirs come to you.").into());
        }));
    });
}

fn name_ask(shared: &Shared, naming: Naming) {
    let (head, ok, text) = match &naming {
        Naming::Rename(id) => ("Rename playlist", "Save", shared.borrow().playlists.iter().find(|p| &p.id == id).map(|p| p.title.clone()).unwrap_or_default()),
    };
    shared.borrow_mut().naming = Some(naming);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_name_head(head.into());
        ui.set_name_ok(ok.into());
        ui.set_name_text(text.into());
        ui.set_name_open(true);
    }
}

fn name_done(shared: &Shared, name: &str) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    let Some(naming) = shared.borrow_mut().naming.take() else { return };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_name_open(false); }
    match naming {
        Naming::Rename(id) => {
            xpl::rename(&mut shared.borrow_mut().xpl, &id, &name);
            xpl_changed(shared);
        }
    }
}

fn pl_add(shared: &Shared, k: usize, t: Track) {
    let Some(id) = shared.borrow().add_map.get(k).cloned() else { return };
    if !xpl::is_own(&id) {
        let p = shared.borrow().playlists.iter().find(|p| p.id == id).cloned();
        if let Some(p) = p { pl_add_platform(shared, p, t); }
        return;
    }
    let title = shared.borrow().playlists.iter().find(|p| p.id == id).map(|p| p.title.clone()).unwrap_or_default();
    let n = xpl::add(&mut shared.borrow_mut().xpl, &id, std::slice::from_ref(&t));
    if n > 0 && t.plat() != "local" { collab_send(shared, &id, collab::add_op(&t)); }
    xpl_changed(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_status(if n > 0 { format!("Added \"{}\" to {title}.", t.title) } else { format!("\"{}\" is already in {title}.", t.title) }.into());
    }
}

fn pl_row_action(shared: &Shared, act: &str, t: Track) {
    match act {
        "plnew" => pl_create(shared, vec![t]),
        "plremove" => {
            let id = match &shared.borrow().here { Some(Place::Playlist(id)) if xpl::is_own(id) => id.clone(), _ => return };
            xpl::remove(&mut shared.borrow_mut().xpl, &id, &t.key);
            collab_send(shared, &id, collab::remove_op(&t.key));
            xpl_changed(shared);
        }
        _ => {
            if let Ok(k) = act.trim_start_matches("pl").parse::<usize>() {
                pl_add(shared, k, t);
            }
        }
    }
}

fn side_action(shared: &Shared, i: usize, act: &str) {
    let Some(p) = shared.borrow().side_map.get(i).and_then(|&at| shared.borrow().playlists.get(at).cloned()) else { return };
    if act == "share" {
        return share_playlist(shared, p);
    }
    if let Some(plat) = act.strip_prefix("link:") {
        return share_platform(shared, p, plat);
    }
    if let Some(kind) = act.strip_prefix("export:") {
        return pl_export(shared, p, kind);
    }
    if act == "collab" {
        return collab_invite(shared, p);
    }
    if act == "collab-leave" {
        return collab_leave(shared, &p.id);
    }
    if !xpl::is_own(&p.id) {
        return;
    }
    if act == "rename" {
        return name_ask(shared, Naming::Rename(p.id));
    }
    let sure = std::env::var_os("OMNIPULSE_SELFTEST").is_some()
        || rfd::MessageDialog::new().set_title("Delete playlist").set_description(format!("Delete \"{}\" from this PC?", p.title)).set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
    if !sure {
        return;
    }
    let gone = shared.borrow().xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(p.id.as_str())).cloned()).unwrap_or_default();
    for plat in mirror::PLATS {
        if gone["made"][plat].as_bool() == Some(true) {
            if let Some(mid) = gone["mirrors"][plat].as_str().map(String::from) {
                hidden_set(shared, &mirror::side_id(plat, &mid), false);
                if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { shared.borrow().rt.spawn(async move { mirror::remove(plat, &mid).await; }); }
            }
        }
    }
    if let Some(from) = gone["from"].as_str() { hidden_set(shared, from, false); }
    xpl::delete(&mut shared.borrow_mut().xpl, &p.id);
    let was_open = matches!(&shared.borrow().here, Some(Place::Playlist(id)) if *id == p.id);
    xpl_changed(shared);
    if was_open {
        go(shared, Place::Page("library".into()));
    }
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Playlist deleted.".into()); }
}

// ---------- sharing a playlist (crate::share) ----------
fn share_status(shared: &Shared, note: String) {
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
}

/// Its page on your relay, every platform together; the link copied (the same link as before, if
/// it was shared already: up to date now)
fn share_playlist(shared: &Shared, p: sources::Playlist) {
    let (relay, by, was, rt, weak) = {
        let a = shared.borrow();
        let relay = a.web.prefs["relayUrl"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(share::RELAY_URL).trim_end_matches('/').to_string();
        let by = a.web.prefs["togetherName"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
            .or_else(|| ACCOUNT_NAMES.with(|n| n.borrow().get("spotify").cloned()).filter(|s| !s.is_empty()))
            .unwrap_or_else(|| "Someone".into());
        let was = if xpl::is_own(&p.id) {
            a.xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(p.id.as_str()))).map(|x| x["share"].clone()).unwrap_or_default()
        } else {
            a.web.prefs["shares"][&p.id].clone()
        };
        (relay, by, was, a.rt.clone(), a.ui.clone())
    };
    share_status(shared, "Sharing...".into());
    rt.spawn(async move {
        let tracks = sources::playlist_tracks(&p).await;
        let done = share::share_playlist(&relay, &p.title, &by, &tracks, &was).await;
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            match done {
                Ok((url, info)) => {
                    let same = info == was;
                    {
                        let mut a = shared.borrow_mut();
                        if xpl::is_own(&p.id) {
                            if let Some(x) = a.xpl.as_array_mut().and_then(|l| l.iter_mut().find(|x| x["id"].as_str() == Some(p.id.as_str()))) { x["share"] = info; }
                            xpl::save(&a.xpl);
                        } else {
                            if !a.web.prefs["shares"].is_object() { a.web.prefs["shares"] = serde_json::json!({}); }
                            a.web.prefs["shares"][&p.id] = info;
                            let all = a.web.prefs["shares"].clone();
                            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("shares", all); }
                        }
                    }
                    share_status(&shared, match share::copy(&url) {
                        Ok(()) => format!("Link copied: anyone can see its songs on every platform, and open it in OmniPulse.{}", if same { " (The same link as before, now up to date.)" } else { "" }),
                        Err(e) => e,
                    });
                }
                Err(e) => share_status(&shared, format!("Couldn't share it: {e}")),
            }
        }));
    });
}

/// The SoundCloud or Spotify copy's own link
fn share_platform(shared: &Shared, p: sources::Playlist, plat: &str) {
    let (id, made, rt, weak) = {
        let a = shared.borrow();
        let made = a.xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(p.id.as_str()))).is_some_and(|x| x["made"][plat].as_bool() == Some(true));
        (share::platform_id(&p, &a.xpl, plat), made, a.rt.clone(), a.ui.clone())
    };
    let Some(id) = id else { return };
    let plat = plat.to_string();
    rt.spawn(async move {
        let link = share::platform_link(&plat, &id, made).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_status(match link.and_then(|u| share::copy(&u)) {
                Ok(()) => format!("{} link copied: only this playlist's songs that are on {}.", share::name(&plat), share::name(&plat)),
                Err(e) => format!("Couldn't get its link: {e}"),
            }.into());
        });
    });
}

// ---------- Listen Along and Jams, hosting (crate::together) ----------
thread_local! {
    /// the room you're hosting, if any
    static ROOM: RefCell<Option<together::Room>> = const { RefCell::new(None) };
    static MEMBERS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// a room being made (so it's only made once)
    static MAKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// someone asking for a Jam: (their ask's id, their name, when)
    /// someone asking: (id, name, since, join), join being "let me into your Jam" (else "start one")
    static ASK: RefCell<Option<(String, String, std::time::Instant, bool)>> = const { RefCell::new(None) };
}
const JAM_ASK_SECS: f32 = 60.0;

fn my_name(a: &App) -> String {
    a.web.prefs["togetherName"].as_str().map(str::trim).filter(|s| !s.is_empty()).map(String::from)
        .or_else(|| ACCOUNT_NAMES.with(|n| n.borrow().get("spotify").cloned()).filter(|s| !s.is_empty()))
        .or_else(|| ACCOUNT_NAMES.with(|n| n.borrow().get("soundcloud").cloned()).filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "Someone".into())
}

fn relay_of(a: &App) -> String {
    a.web.prefs["relayUrl"].as_str().map(str::trim).filter(|s| !s.is_empty()).unwrap_or(share::RELAY_URL).trim_end_matches('/').to_string()
}

fn room_kind() -> Option<String> {
    ROOM.with(|r| r.borrow().as_ref().map(|r| r.kind.clone()))
}

/// The Listen Along room's link for the Discord buttons: made while you're playing with Listen along
/// or Ask to Jam on (and not in a Jam), let go when not
fn listen_url(shared: &Shared, a: &App, playing: bool) -> Option<String> {
    let p = &a.web.prefs;
    let want = integrations::set(p, "presence") && (integrations::on(p, "presListen") || integrations::set(p, "presAsk")) && playing;
    match room_kind().as_deref() {
        Some("jam") => None,
        Some(_) if !want => { room_end(false); None }
        Some(_) => ROOM.with(|r| r.borrow().as_ref().map(|r| r.join_url.clone())),
        None if want && !MAKING.with(|m| m.get()) => {
            MAKING.with(|m| m.set(true));
            room_open(shared, "listen", None, |s| push_out(s, true));
            None
        }
        None => None,
    }
}

type RoomDone = Box<dyn FnOnce(&Shared)>;
thread_local! {
    /// what to do once the room being made is open: kept on the UI thread (it holds UI state, so it
    /// can't go along into the network task), and picked up when the answer comes back there
    static ROOM_DONE: RefCell<Option<RoomDone>> = const { RefCell::new(None) };
}

/// A room: made (or one made already), connected, and kept; then `done`
fn room_open(shared: &Shared, kind: &'static str, made: Option<serde_json::Value>, done: impl FnOnce(&Shared) + 'static) {
    let (rt, weak, relay, host) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), relay_of(&a), my_name(&a)) };
    ROOM_DONE.with(|d| *d.borrow_mut() = Some(Box::new(done)));
    let rt2 = rt.clone();
    rt.spawn(async move {
        let made = match made { Some(m) => Ok(m), None => together::create(&relay, kind, &host).await };
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            MAKING.with(|m| m.set(false));
            let Some(shared) = s.borrow().clone() else { return };
            match made {
                Ok(made) => {
                    room_end(false);
                    let room = together::connect(&rt2, &relay, kind, &made, |m| {
                        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { room_message(&sh, m); } }));
                    });
                    ROOM.with(|r| *r.borrow_mut() = Some(room));
                    jam_paint(&shared);
                    if let Some(done) = ROOM_DONE.with(|d| d.borrow_mut().take()) { done(&shared); }
                }
                Err(e) => {
                    ROOM_DONE.with(|d| d.borrow_mut().take());
                    ui.set_status(format!("Couldn't open a room on the relay: {e}").into());
                }
            }
        }));
    });
}

/// The room closed (the listeners told it's over, when `tell`)
fn room_end(tell: bool) {
    ROOM.with(|r| {
        if let Some(room) = r.borrow_mut().take() {
            if tell { let _ = room.tx.send(serde_json::json!({ "type": "end" }).to_string()); }
        }
    });
    MEMBERS.with(|m| m.borrow_mut().clear());
}

/// What's playing now, and next, to the room (as the web app's hostSnapshot)
fn host_push(a: &App) {
    let Some(tx) = ROOM.with(|r| r.borrow().as_ref().map(|r| r.tx.clone())) else { return };
    let cur = a.queue.current();
    let from = a.queue.qi.map_or(0, |i| i + 1);
    let next: Vec<serde_json::Value> = a.queue.items.iter().skip(from).take(20).map(|q| q.t.to_snapshot()).collect();
    let state = serde_json::json!({
        "v": 1, "host": my_name(a), "kind": room_kind().unwrap_or_default(),
        "playing": a.loaded && !a.engine.is_paused(), "pos": a.engine.position_ms(),
        "track": cur.map(|t| t.to_snapshot()), "next": next,
        // how this app plays: listeners use it too while they're in, so they stay in step
        "feel": { "fadeOn": a.web.prefs["fadeOn"].as_bool() == Some(true), "fadeMs": a.web.prefs["fadeMs"].as_u64().unwrap_or(3000), "skipSilence": a.web.prefs["skipSilence"].as_u64().unwrap_or(8) },
    });
    let _ = tx.send(serde_json::json!({ "type": "state", "state": state }).to_string());
}

/// What the room says
fn room_message(shared: &Shared, m: serde_json::Value) {
    match m["type"].as_str() {
        Some("open") => host_push(&shared.borrow()),
        Some("members") => {
            MEMBERS.with(|x| *x.borrow_mut() = m["list"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(String::from)).collect());
            jam_paint(shared);
        }
        // a Jam member's song: in the queue, as theirs (Play next: next, after the other requests)
        Some("add") if room_kind().as_deref() == Some("jam") => {
            let Some(t) = Track::from_snapshot(&m["track"]) else { return };
            let who = m["by"].as_str().unwrap_or("Someone").to_string();
            let by = format!("{who} (Jam)");
            let title = t.title.clone();
            let next = m["next"].as_bool().unwrap_or(false);
            {
                let mut a = shared.borrow_mut();
                let at = a.queue.add_request(t, &by);
                if next && at > 1 {
                    let i = a.queue.qi.unwrap_or(0) + at;
                    a.queue.move_next(i);
                }
                paint_queue(&mut a);
                save_session(&a);
            }
            let note = if next { format!("{who} put \"{title}\" on next.") } else { format!("{who} added \"{title}\" to the Jam.") };
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
            host_push(&shared.borrow());
        }
        // a Jam member moving a song: the same place among what's coming up, for everyone
        Some("move") if room_kind().as_deref() == Some("jam") => {
            let (Some(id), rel) = (m["id"].as_str(), m["to"].as_u64().unwrap_or(1).max(1) as usize) else { return };
            let moved = {
                let mut a = shared.borrow_mut();
                let qi = a.queue.qi.unwrap_or(0);
                let from = a.queue.items.iter().enumerate().skip(qi + 1).find(|(_, q)| q.t.key == id).map(|(i, _)| i);
                let to = (qi + rel).min(a.queue.items.len().saturating_sub(1));
                match from {
                    Some(f) if f != to => {
                        let q = a.queue.items.remove(f);
                        let title = q.t.title.clone();
                        a.queue.items.insert(to, q);
                        paint_queue(&mut a);
                        save_session(&a);
                        Some(title)
                    }
                    _ => None,
                }
            };
            if let Some(title) = moved {
                let who = m["by"].as_str().unwrap_or("Someone");
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{who} moved \"{title}\" in the Jam.").into()); }
                host_push(&shared.borrow());
            }
        }
        // a Jam member skipping (on, or back): for everyone
        Some("skip") if room_kind().as_deref() == Some("jam") => {
            let back = m["d"].as_i64() == Some(-1);
            step(shared, if back { -1 } else { 1 });
            let who = m["by"].as_str().unwrap_or("Someone");
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{who} {}", if back { "went back a song." } else { "skipped the song." }).into()); }
        }
        Some(kind @ ("ask" | "joinAsk")) => {
            let (Some(id), name) = (m["id"].as_str(), m["name"].as_str().unwrap_or("Someone")) else { return };
            let join = kind == "joinAsk";
            // Listen Along & Jams switched off: turned away; one at a time (the next is "busy", not declined)
            if !jams_on(&shared.borrow()) { ask_answer(id, join, false, false); return; }
            if ASK.with(|a| a.borrow().is_some()) || (!join && room_kind().as_deref() == Some("jam")) { ask_answer(id, join, false, true); return; }
            ASK.with(|a| *a.borrow_mut() = Some((id.to_string(), name.to_string(), std::time::Instant::now(), join)));
            jam_paint(shared);
        }
        Some("askGone") if ASK.with(|a| a.borrow().as_ref().is_some_and(|x| m["id"].as_str() == Some(x.0.as_str()))) => {
            ASK.with(|a| *a.borrow_mut() = None);
            jam_paint(shared);
        }
        _ => {}
    }
}

fn ask_reply(id: &str, ok: bool, url: &str) {
    ROOM.with(|r| { if let Some(room) = r.borrow().as_ref() { let _ = room.tx.send(serde_json::json!({ "type": "askReply", "id": id, "ok": ok, "url": url }).to_string()); } });
}

/// Answering someone asking: for a Jam (askReply) or into yours (admit). `busy`: not now, rather than
/// no (the relay only keeps a real no, so they can ask again)
fn ask_answer(id: &str, join: bool, ok: bool, busy: bool) {
    let kind = if join { "admit" } else { "askReply" };
    ROOM.with(|r| {
        if let Some(room) = r.borrow().as_ref() {
            // (let into your Jam: its invite goes with it, which their app opens)
            let url = if join && ok { room.join_url.clone() } else { String::new() };
            let _ = room.tx.send(serde_json::json!({ "type": kind, "id": id, "ok": ok, "busy": busy, "url": url }).to_string());
        }
    });
}

/// Settings > Listen Along & Jams (on unless switched off, as the web app's prefs.jamsOn)
fn jams_on(a: &App) -> bool {
    integrations::on(&a.web.prefs, "jamsOn")
}

/// The title bar: Start a Jam / Jam · n, or someone's ask (its ring emptying; declined when it runs out)
fn jam_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let ask = ASK.with(|a| a.borrow().clone());
    if let Some((id, name, at, join)) = ask {
        let left = 1.0 - at.elapsed().as_secs_f32() / JAM_ASK_SECS;
        if left <= 0.0 {
            // ran out: not now, rather than no
            ask_answer(&id, join, false, true);
            ASK.with(|a| *a.borrow_mut() = None);
            return jam_paint(shared);
        }
        ui.set_jam_ask(if join { format!("{name} wants to join your Jam") } else { format!("{name} wants you to Start a Jam") }.into());
        ui.set_jam_ring(together::ring_path(left).into());
    } else {
        ui.set_jam_ask("".into());
    }
    let members = MEMBERS.with(|m| m.borrow().clone());
    if let Some(kind) = listening() {
        ui.set_jam_on(true);
        ui.set_jam_hidden(false);
        ui.set_jam_label(if kind == "jam" { "Leave Jam" } else { "Leave Listen Along" }.into());
        ui.set_queue_room(if kind == "jam" { "In a Jam: the host's queue".into() } else { "Listening along: the host's queue".into() });
        ui.set_queue_room_host(false);
        return;
    }
    let jam = room_kind().as_deref() == Some("jam");
    ui.set_queue_room(if jam { format!("Your Jam · {} listening", members.len()).into() } else { "".into() });
    ui.set_queue_room_host(jam);
    ui.set_jam_on(jam);
    ui.set_jam_hidden(!jams_on(&shared.borrow()));
    ui.set_jam_label(if jam { if members.is_empty() { "Jam".into() } else { format!("Jam \u{b7} {}", members.len()) } } else { "Start a Jam".into() }.into());
    ui.set_jam_link(ROOM.with(|r| r.borrow().as_ref().filter(|r| r.kind == "jam").map(|r| r.join_url.clone())).unwrap_or_default().into());
    ui.set_jam_members(if members.is_empty() { "Nobody has joined yet.".to_string() } else { format!("In the Jam: {}", members.join(", ")) }.into());
}

fn jam_start(shared: &Shared) {
    room_open(shared, "jam", None, |s| {
        let link = ROOM.with(|r| r.borrow().as_ref().map(|r| r.join_url.clone())).unwrap_or_default();
        let note = match share::copy(&link) { Ok(()) => "Jam started. The invite link is copied: send it to the friends you want in.".to_string(), Err(_) => "Jam started.".to_string() };
        if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(note.into()); ui.set_jam_open(true); }
        push_out(s, true);
    });
}

/// Accepting an ask: the Jam made, its invite sent on the room they asked in, then the move to it
fn jam_accept(shared: &Shared) {
    let Some((id, name, _, join)) = ASK.with(|a| a.borrow_mut().take()) else { return };
    jam_paint(shared);
    // into your Jam: let in (they connect again by themselves)
    if join {
        ask_answer(&id, true, true, false);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Let {name} into your Jam.").into()); }
        return;
    }
    let (rt, weak, relay, host) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), relay_of(&a), my_name(&a)) };
    rt.spawn(async move {
        let made = together::create(&relay, "jam", &host).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            match made {
                Ok(made) => {
                    ask_reply(&id, true, made["joinUrl"].as_str().unwrap_or(""));
                    let sh = shared.clone();
                    slint::Timer::single_shot(std::time::Duration::from_millis(400), move || {
                        room_end(true);
                        let name = name.clone();
                        room_open(&sh, "jam", Some(made), move |s| {
                            if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(format!("Jam started with {name}. Invite more friends from the Jam button.").into()); }
                            push_out(s, true);
                        });
                    });
                }
                Err(e) => { ask_reply(&id, false, ""); ui.set_status(format!("Couldn't start a Jam: {e}").into()); }
            }
        }));
    });
}

// ---------- listening along: someone else's room (as the web app's listener side) ----------
// Your own queue is set aside while listening and back when you leave. The host's track plays
// through this app's accounts (or its YouTube Music version, when this app can't play it), at the
// host's position; their pauses and skips are followed. In a Jam, skipping and adding songs go to
// the host, who does them for everyone.
struct Listening {
    kind: String,
    host: String,
    tx: tokio::sync::mpsc::UnboundedSender<String>,
    saved: queue::Queue,
    /// the host's last state, and when it came
    state: Option<serde_json::Value>,
    at: std::time::Instant,
    /// the track lined up with the host's already (left alone after), and whether something moved since
    synced: Option<String>,
    resync: bool,
}

thread_local! {
    static LISTEN: RefCell<Option<Listening>> = const { RefCell::new(None) };
}

/// "jam" or "listen" while listening along
fn listening() -> Option<String> {
    LISTEN.with(|l| l.borrow().as_ref().map(|l| l.kind.clone()))
}

fn listen_send(m: serde_json::Value) {
    LISTEN.with(|l| { if let Some(l) = l.borrow().as_ref() { let _ = l.tx.send(m.to_string()); } });
}

/// Where the host is now: what they said, plus the time since (while they're playing)
fn listen_host_pos() -> u64 {
    LISTEN.with(|l| {
        let l = l.borrow();
        let Some(st) = l.as_ref().and_then(|l| l.state.as_ref().map(|s| (s, l.at))) else { return 0 };
        let pos = st.0["pos"].as_f64().unwrap_or(0.0).max(0.0) as u64;
        if st.0["playing"].as_bool() == Some(true) { pos + st.1.elapsed().as_millis() as u64 } else { pos }
    })
}

/// Whether this app can play the host's track as it is (else its YouTube Music version)
fn listen_needs_version(t: &Track) -> bool {
    if listening().is_none() {
        return false;
    }
    let ytdlp = omnipulse_core::tools::ytdlp_path().is_some();
    let ok = match &t.src {
        track::Src::SoundCloud(_) => omnipulse_core::soundcloud::signed_in() || ytdlp,
        track::Src::Spotify(_) => omnipulse_core::spotify::spotify_signed_in() && omnipulse_core::spotify_engine::spe_signed_in(),
        track::Src::YouTube(_) => ytdlp,
        track::Src::Local(p) => std::path::Path::new(p).exists(),
        track::Src::Catalog(_) => false,
        track::Src::Plugin(id, _) => plugrun::get(id).is_some(),
    };
    !ok && ytdlp
}

/// Listening along: something moved on your end (a pause, a scrub), so it's lined up again
fn listen_moved() {
    LISTEN.with(|l| { if let Some(l) = l.borrow_mut().as_mut() { l.resync = true; } });
}

/// Fade Between Tracks as it's set: listening along, the host's (from their state's "feel")
fn fade_now(a: &App) -> Option<u64> {
    let host = LISTEN.with(|l| l.borrow().as_ref().and_then(|l| l.state.as_ref().map(|s| s["feel"].clone()))).filter(|f| f.is_object());
    match host {
        Some(f) => sound::fade_ms(&f),
        None => sound::fade_ms(&a.web.prefs),
    }
}

/// How a track goes out into the next one: as Fade Between Tracks is set; with seamless mixing on, the
/// mix decides (a mix where it can, and where it can't, its own short crossfade, whatever fading's set)
fn fade_out_ms(a: &App) -> Option<u64> {
    if integrations::on(&a.web.prefs, "mixOn") && listening().is_none() { return Some(MIX_FALLBACK_MS); }
    fade_now(a)
}

/// The crossfade seamless mixing falls back on (no beat to mix on, or the next one couldn't be got ready)
const MIX_FALLBACK_MS: u64 = 6000;

/// A skip by hand (the buttons, media keys, the remote): listening, it goes to the Jam's host
fn skip_press(shared: &Shared, d: i32) {
    XFADE.with(|x| *x.borrow_mut() = None);
    let Some(kind) = listening() else { return step(shared, d) };
    let note = if kind == "jam" {
        listen_send(serde_json::json!({ "type": "skip", "d": if d < 0 { -1 } else { 1 } }));
        if d < 0 { "Going back a song for the Jam..." } else { "Skipping for the Jam..." }.to_string()
    } else {
        format!("{} picks what plays while you listen along.", LISTEN.with(|l| l.borrow().as_ref().map(|l| l.host.clone())).unwrap_or_default())
    };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
}

/// Opens an invite link (given to the app when it starts)
/// An omnipulse:// link (the app started with it, or handed it by a second one): a collaborative
/// playlist's invite joins it; a Jam's or a Listen Along's joins that
fn open_link(shared: &Shared, link: &str) {
    if let Some((code, relay, e)) = together::parse_playlist_link(link, &relay_of(&shared.borrow())) {
        if !e.is_empty() { return collab_join(shared, code, relay, e); }
    }
    listen_join(shared, link)
}

fn listen_join(shared: &Shared, link: &str) {
    let (rt, weak, default_relay, name, cid) = {
        let a = shared.borrow();
        let cid = a.web.prefs["clientId"].as_str().map(String::from).unwrap_or_else(|| format!("{:016x}", rand_u64()));
        (a.rt.clone(), a.ui.clone(), relay_of(&a), my_name(&a), cid)
    };
    let Some((kind, code, relay, k)) = together::parse_link(link, &default_relay) else {
        if let Some(ui) = weak.upgrade() { ui.set_status("That link isn't an OmniPulse invite.".into()); }
        return;
    };
    if !jams_on(&shared.borrow()) {
        if let Some(ui) = weak.upgrade() { ui.set_status("Listen Along & Jams are switched off (Settings > Integrations).".into()); }
        return;
    }
    // asking: the host answers (a Jam's invite, or let into theirs), then it's joined
    if kind == "ask" {
        return ask_send(shared, &relay, &code, &name, &cid);
    }
    if listening().is_some() { listen_leave(shared, true); }
    room_end(true);
    let ws = relay.replacen("http", "ws", 1);
    let url = format!("{ws}/ws/{code}?role=listener&name={}&cid={}{}", sources::urlencode(&name), sources::urlencode(&cid),
        if k.is_empty() { String::new() } else { format!("&k={}", sources::urlencode(&k)) });
    let tx = together::listen(&rt, url, |m| {
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { listen_message(&sh, m); } }));
    });
    let saved = std::mem::take(&mut shared.borrow_mut().queue);
    shared.borrow().engine.pause();
    LISTEN.with(|l| *l.borrow_mut() = Some(Listening { kind: kind.clone(), host: "the host".into(), tx, saved, state: None, at: std::time::Instant::now(), synced: None, resync: false }));
    if let Some(ui) = weak.upgrade() { ui.set_status(if kind == "jam" { "Joining the Jam..." } else { "Joining..." }.into()); }
    jam_paint(shared);
}

thread_local! {
    /// asking someone (their name, and the connection while it waits for their answer)
    static ASKING: RefCell<Option<(String, tokio::sync::mpsc::UnboundedSender<String>)>> = const { RefCell::new(None) };
}

/// Asks the host of a room (from their Discord status's Ask to Jam / Ask to join): their answer is a
/// Jam's invite (or being let into theirs), which is joined; a no, or no answer, is said
fn ask_send(shared: &Shared, relay: &str, code: &str, name: &str, cid: &str) {
    if ASKING.with(|a| a.borrow().is_some()) {
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("You've asked already: waiting for their answer.".into()); }
        return;
    }
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let (relay, code, name, cid) = (relay.to_string(), code.to_string(), name.to_string(), cid.to_string());
    let host_url = format!("{relay}/api/rooms/{code}");
    rt.clone().spawn(async move {
        let host = match reqwest::get(&host_url).await { Ok(r) => r.json::<serde_json::Value>().await.ok().and_then(|v| v["host"].as_str().map(String::from)), Err(_) => None };
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let Some(host) = host else { ui.set_status("That session has ended.".into()); return; };
            let url = format!("{}/ws/{code}?role=asker&name={}&cid={}", relay.replacen("http", "ws", 1), sources::urlencode(&name), sources::urlencode(&cid));
            let relay2 = relay.clone();
            let tx = together::listen(&shared.borrow().rt, url, move |m| {
                let relay = relay2.clone();
                let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { ask_answer_here(&sh, m, &relay); } }));
            });
            ASKING.with(|a| *a.borrow_mut() = Some((host.clone(), tx)));
            ui.set_status(format!("Asked {host}. Waiting for their answer...").into());
        }));
    });
}

/// An invite's web page (https://<relay>/jam/<code>?k=...) as the app link it opens
fn app_link(url: &str, relay: &str) -> String {
    if url.starts_with("omnipulse://") { return url.to_string(); }
    let rest = url.split_once("://").map(|x| x.1).unwrap_or(url);
    let (origin_host, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (path, query) = path.split_once('?').unwrap_or((path, ""));
    let k = query.split('&').find_map(|p| p.strip_prefix("k=")).unwrap_or("");
    let origin = if origin_host.is_empty() { relay.to_string() } else { format!("{}://{origin_host}", url.split("://").next().unwrap_or("https")) };
    format!("omnipulse://{path}?k={k}&relay={}", sources::urlencode(&origin))
}

/// The host's answer to your ask
fn ask_answer_here(shared: &Shared, m: serde_json::Value, relay: &str) {
    let Some(host) = ASKING.with(|a| a.borrow().as_ref().map(|x| x.0.clone())) else { return };
    let say = |t: String| if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(t.into()); };
    match m["type"].as_str() {
        Some("askReply") | Some("admitted") => {
            ASKING.with(|a| a.borrow_mut().take());
            let link = m["url"].as_str().unwrap_or("").to_string();
            let ok = m["type"].as_str() == Some("admitted") || m["ok"].as_bool() == Some(true);
            if ok && !link.is_empty() {
                // the Jam's invite (its web page's address), as the app link that page would open
                listen_join(shared, &app_link(&link, relay));
            } else if m["busy"].as_bool() == Some(true) {
                say(format!("{host} is busy with another request: ask again in a bit."));
            } else {
                say(format!("{host} didn't start a Jam this time."));
            }
        }
        Some("declined") => { ASKING.with(|a| a.borrow_mut().take()); say(format!("{host} didn't let you in this time.")); }
        Some("closed") if ASKING.with(|a| a.borrow_mut().take()).is_some() => say(format!("No answer from {host}.")),
        _ => {}
    }
}

fn listen_message(shared: &Shared, m: serde_json::Value) {
    if listening().is_none() {
        return;
    }
    match m["type"].as_str() {
        Some("hello") => {
            let host = m["host"].as_str().unwrap_or("the host").to_string();
            let kind = listening().unwrap_or_default();
            LISTEN.with(|l| { if let Some(l) = l.borrow_mut().as_mut() { l.host = host.clone(); } });
            let note = if kind == "jam" { format!("Joined {host}'s Jam. You can add songs and skip them too. Your own queue is set aside until you leave.") } else { format!("Listening along with {host}. Your own queue is set aside until you leave.") };
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
        }
        Some("state") => listen_follow(shared, m["state"].clone()),
        Some("ended") => {
            let host = LISTEN.with(|l| l.borrow().as_ref().map(|l| l.host.clone())).unwrap_or_default();
            listen_leave(shared, true);
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{host} ended the session. Back to your own queue.").into()); }
        }
        Some("declined") => {
            listen_leave(shared, true);
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("The host didn't let you in this time.".into()); }
        }
        Some("closed") => {
            listen_leave(shared, true);
            let why = m["why"].as_str().unwrap_or("the connection closed");
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Left the session: {why}. Back to your own queue.").into()); }
        }
        _ => {}
    }
}

/// The host's state: the same track (started at their position), playing or paused as they are,
/// and back in step if it drifts more than 3 seconds
fn listen_follow(shared: &Shared, st: serde_json::Value) {
    if listening().is_none() {
        return;   // (one still on its way after leaving)
    }
    // the host's end moved (as the web app's): a jump in their position beyond what the time since says,
    // or paused or played; otherwise a track lined up once is left alone
    LISTEN.with(|l| {
        let mut l = l.borrow_mut();
        let Some(l) = l.as_mut() else { return };
        if let Some(prev) = &l.state {
            if prev["track"]["id"] == st["track"]["id"] {
                let (pp, np) = (prev["playing"].as_bool() == Some(true), st["playing"].as_bool() == Some(true));
                let (ppos, npos) = (prev["pos"].as_f64().unwrap_or(0.0), st["pos"].as_f64().unwrap_or(0.0));
                let expect = if pp { ppos + l.at.elapsed().as_millis() as f64 } else { ppos };
                if pp != np || (npos - expect).abs() > 1500.0 { l.resync = true; }
            }
        }
        l.state = Some(st.clone());
        l.at = std::time::Instant::now();
    });
    let Some(target) = Track::from_snapshot(&st["track"]) else {
        if shared.borrow().loaded && !shared.borrow().engine.is_paused() { shared.borrow().engine.pause(); }
        return;
    };
    let next: Vec<Track> = st["next"].as_array().into_iter().flatten().filter_map(Track::from_snapshot).collect();
    let playing = st["playing"].as_bool() == Some(true);
    // (a stand-in playing for the host's track counts as it)
    let standing = STAND_IN.with(|x| x.borrow().as_ref().is_some_and(|k| *k == target.key));
    let same = standing || shared.borrow().queue.current().is_some_and(|c| c.key == target.key);
    if !standing { STAND_IN.with(|x| x.borrow_mut().take()); }
    if !same {
        {
            let mut a = shared.borrow_mut();
            a.queue.items = std::iter::once(target).chain(next).map(queue::QItem::mine).collect();
            a.queue.qi = Some(0);
            a.resume_ms = Some(listen_host_pos());
        }
        LISTEN.with(|l| { if let Some(l) = l.borrow_mut().as_mut() { l.synced = None; } });
        start(shared);
        if !playing { shared.borrow().engine.pause(); }
        return;
    }
    {
        let mut a = shared.borrow_mut();
        let cur = a.queue.items.first().cloned();
        a.queue.items = cur.into_iter().chain(next.into_iter().map(queue::QItem::mine)).collect();
        a.queue.qi = Some(0);
        paint_queue(&mut a);
    }
    let engine = shared.borrow().engine.clone();
    if !shared.borrow().loaded { return; }
    if playing && engine.is_paused() { engine.play(); } else if !playing && !engine.is_paused() { engine.pause(); }
    // (a stand-in plays from its own start: only the host's own track is kept in step)
    let key = shared.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default();
    let lined_up = LISTEN.with(|l| l.borrow().as_ref().is_some_and(|l| l.synced.as_deref() == Some(key.as_str()) && !l.resync));
    if playing && !standing && !lined_up {
        let (want, have) = (listen_host_pos(), engine.position_ms());
        // (back in step, smoothly: in again from quiet after the jump)
        if want.abs_diff(have) > 1000 { engine.seek(want); engine.fade(Some(0.0), 1.0, 800); }
        LISTEN.with(|l| { if let Some(l) = l.borrow_mut().as_mut() { l.synced = Some(key); l.resync = false; } });
    }
}

thread_local! {
    /// listening along: the host's track a song of yours is standing in for (its key)
    static STAND_IN: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// A song of yours about as long as what's left of the host's track (the closest that's at least as
/// long, or else the longest), from your likes and files that play here, as the web app's pickStandIn
fn stand_in(shared: &Shared, host: &Track) {
    let left = host.ms.saturating_sub(listen_host_pos()).max(30_000);
    let pick = {
        let a = shared.borrow();
        let pool: Vec<&Track> = a.library.iter().filter(|t| t.ms > 30_000 && !t.is_mix() && t.key != host.key && !listen_needs_version(t) && !matches!(t.src, track::Src::Catalog(_))).collect();
        let mut longer: Vec<&&Track> = pool.iter().filter(|t| t.ms >= left).collect();
        longer.sort_by_key(|t| t.ms);
        let k = (rand_u64() as usize) % longer.len().clamp(1, 3);
        longer.get(k).map(|t| (**t).clone()).or_else(|| pool.iter().max_by_key(|t| t.ms).map(|t| (*t).clone()))
    };
    let Some(pick) = pick else {
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("\"{}\" isn't available on your end, and there's nothing of yours to play meanwhile.", host.title).into()); }
        return;
    };
    STAND_IN.with(|x| *x.borrow_mut() = Some(host.key.clone()));
    let note = format!("\"{}\" isn't available on your end, so \"{}\" plays in its place until the host moves on.", host.title, pick.title);
    {
        let mut a = shared.borrow_mut();
        if let Some(q) = a.queue.items.get_mut(0) { q.t = pick; }
        a.queue.qi = Some(0);
    }
    start(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
}

/// Leaving (or the room's gone): your own queue back, paused where it was
fn listen_leave(shared: &Shared, quiet: bool) {
    let Some(l) = LISTEN.with(|l| l.borrow_mut().take()) else { return };
    STAND_IN.with(|x| x.borrow_mut().take());
    drop(l.tx);
    shared.borrow().engine.pause();
    {
        let mut a = shared.borrow_mut();
        a.queue = l.saved;
        a.loaded = false;
        paint_queue(&mut a);
    }
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_playing(false);
        match shared.borrow().queue.current() {
            Some(t) => show_now(&ui, t),
            // nothing of your own queued: the bar empties
            None => { ui.set_now_title("".into()); ui.set_now_artist("".into()); ui.set_pos_text("".into()); ui.set_dur_text("".into()); }
        }
        if !quiet { ui.set_status("You left. Back to your own queue.".into()); }
    }
    jam_paint(shared);
}

fn rand_u64() -> u64 {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.finish()
}

// ---------- Back Up & Restore (Settings > App; crates/core/src/backup.rs, as the web app's) ----------
// The backup's "web" part is the web app's saved data: here, the copy it last wrote for this app.
// This app's own settings and state are files in the data folder, and go in with the rest.
// ---------- updates (crate::updater) ----------
thread_local! {
    /// a newer release, once one's known
    static UPDATE: RefCell<Option<updater::Release>> = const { RefCell::new(None) };
}

fn update_check(shared: &Shared, said: bool) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    if let Some(ui) = weak.upgrade() { ui.set_update_note("Checking...".into()); }
    rt.spawn(async move {
        let r = updater::check().await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            match r {
                Ok(Some(rel)) => {
                    ui.set_update_note(format!("OmniPulse {} is out (this is {}). {}", rel.version, updater::VERSION, rel.notes).into());
                    ui.set_update_label("Update".into());
                    ui.set_status(format!("OmniPulse {} is out: Settings > App & Look > Updates.", rel.version).into());
                    UPDATE.with(|u| *u.borrow_mut() = Some(rel));
                }
                Ok(None) => {
                    ui.set_update_note(format!("OmniPulse {}: up to date.", updater::VERSION).into());
                    if said { ui.set_status("OmniPulse is up to date.".into()); }
                }
                Err(e) => ui.set_update_note(format!("Couldn't check for updates: {e}").into()),
            }
        });
    });
}

fn update_act(shared: &Shared) {
    let Some(rel) = UPDATE.with(|u| u.borrow().clone()) else { return update_check(shared, true) };
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    if let Some(ui) = weak.upgrade() { ui.set_update_note(format!("Downloading OmniPulse {}...", rel.version).into()); ui.set_update_label("...".into()); }
    rt.spawn(async move {
        let got = updater::download(&rel).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            match got.and_then(|p| updater::install(&p)) {
                Ok(()) => {
                    // (the installer starts the new one when it's done)
                    if let Some(sh) = s.borrow().clone() { save_session(&sh.borrow()); }
                    let _ = slint::quit_event_loop();
                }
                Err(e) => { ui.set_update_note(format!("Couldn't update: {e}").into()); ui.set_update_label("Update".into()); }
            }
        }));
    });
}

fn backup_web() -> String {
    let mut v = omnipulse_core::load_web_export().unwrap_or_else(|| serde_json::json!({}));
    if let Some(o) = v.as_object_mut() {
        o.retain(|k, _| k.starts_with("sc."));
    }
    v.to_string()
}

fn backup_make(shared: &Shared) {
    let day = {
        let d = omnipulse_core::now_secs() / 86_400;
        // days since 1970 to a date (civil from days, Howard Hinnant's)
        let z = d as i64 + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z - era * 146_097;
        let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let (dd, mm) = (doy - (153 * mp + 2) / 5 + 1, if mp < 10 { mp + 3 } else { mp - 9 });
        format!("{}-{mm:02}-{dd:02}", yoe + era * 400 + i64::from(mm <= 2))
    };
    let Some(dest) = rfd::FileDialog::new().set_title("Back up OmniPulse").set_file_name(format!("OmniPulse Backup {day}.zip")).add_filter("OmniPulse backup", &["zip"]).save_file() else { return };
    let msg = match omnipulse_core::backup::create(&dest.to_string_lossy(), &backup_web()) {
        Ok(n) => format!("Backed up: your settings and {n} {}.", if n == 1 { "file" } else { "files" }),
        Err(e) => format!("Couldn't back up: {e}"),
    };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(msg.into()); }
}

fn backup_restore(shared: &Shared) {
    let Some(src) = rfd::FileDialog::new().set_title("Restore an OmniPulse backup").add_filter("OmniPulse backup", &["zip"]).pick_file() else { return };
    let sure = rfd::MessageDialog::new()
        .set_title("Restore this backup?")
        .set_description("Your settings, favourites, playlists, stats and alerts are replaced with the backup's, and OmniPulse starts again.")
        .set_buttons(rfd::MessageButtons::OkCancel)
        .show();
    if sure != rfd::MessageDialogResult::Ok {
        return;
    }
    let status = |m: String| if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(m.into()); };
    let web = match omnipulse_core::backup::restore(&src.to_string_lossy()) {
        Ok(w) => w,
        Err(e) => return status(format!("Couldn't restore it: {e}")),
    };
    // the web app's part, where this app reads it (the web app takes it from the .zip itself)
    let mut v: serde_json::Value = serde_json::from_str(&web).unwrap_or_else(|_| serde_json::json!({}));
    v["savedAt"] = serde_json::json!(now_ms());
    if let Err(e) = omnipulse_core::save_web_export(&v.to_string()) {
        return status(format!("The files are back, but the settings couldn't be: {e}"));
    }
    // and again, with it all read fresh
    if let Ok(me) = std::env::current_exe() {
        let _ = std::process::Command::new(me).spawn();
    }
    let _ = slint::quit_event_loop();
}

// ---------- Streamer Mode and Stream-Safe Music (as the web app's) ----------
fn streamer_step(shared: &Shared) {
    let (private, safe) = {
        let a = shared.borrow();
        (integrations::set(&a.web.prefs, "streamerMode"), streamsafe::on(&a.web.prefs))
    };
    let set_private = |on: bool| {
        if let Some(o) = shared.borrow_mut().web.prefs.as_object_mut() { o.insert("streamerMode".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("streamerMode", serde_json::json!(on)); }
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_streamer(on); }
    };
    if !private && !safe {
        set_private(true);
        if let Some(ui) = shared.borrow().ui.upgrade() {
            ui.set_status("Streamer Mode: account names, keys and addresses are hidden. Click again to leave out copyrighted music too.".into());
        }
    } else if private && !safe {
        safe_set(shared, true);
    } else {
        set_private(false);
        safe_set(shared, false);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Streamer Mode off.".into()); }
    }
}

fn safe_set(shared: &Shared, on: bool) {
    {
        let mut a = shared.borrow_mut();
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("streamSafe".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("streamSafe", serde_json::json!(on)); }
        a.ap_failed = None;
        apply_filter(&mut a);
        paint_settings(&a);
        if let Some(ui) = a.ui.upgrade() {
            ui.set_stream_safe(on);
            let private = integrations::set(&a.web.prefs, "streamerMode");
            ui.set_status(if on { format!("Streamer Mode: only music that's safe on stream shows and plays{}", if private { ", and private info is hidden." } else { "." }) } else { "Stream-Safe Music off.".to_string() }.into());
        }
    }
    // the Streamer Mode page takes Discover's place while it's on (and gives it back after)
    let page = shared.borrow().page.clone();
    if on && page == "discover" { open_streamer(shared); }
    if !on && (page == "streamer" || page == "streamcoll") { open_page(shared, "discover"); }
    // what's playing now, if it isn't safe, goes
    if on && shared.borrow().loaded && !shared.borrow().queue.current().is_some_and(|t| streamsafe::ok(&shared.borrow().web.prefs, t)) {
        start(shared);
    }
}

/// With Stream-Safe Music on, a track that isn't safe: on to the next one in the queue that is, or
/// autoplay's, or a stop. True if the track was skipped.
fn safe_skip(shared: &Shared) -> bool {
    let (next, title) = {
        let a = shared.borrow();
        let Some(t) = a.queue.current() else { return false };
        if streamsafe::allowed(&a.web.prefs, t) {
            return false;
        }
        let from = a.queue.qi.unwrap_or(0);
        (a.queue.items.iter().enumerate().skip(from + 1).find(|(_, q)| streamsafe::ok(&a.web.prefs, &q.t)).map(|(k, _)| k), t.title.clone())
    };
    match next {
        Some(k) => {
            shared.borrow_mut().queue.qi = Some(k);
            start(shared);
        }
        None => {
            // to the end: autoplay finds safe ones from there, if it's on
            let last = shared.borrow().queue.items.len().saturating_sub(1);
            shared.borrow_mut().queue.qi = Some(last);
            let ap = shared.borrow().queue.current().is_some_and(|t| streamsafe::ok(&shared.borrow().web.prefs, t));
            if ap {
                start(shared);
                return true;
            }
            let engine = shared.borrow().engine.clone();
            engine.stop();
            shared.borrow_mut().loaded = false;
            if let Some(ui) = shared.borrow().ui.upgrade() {
                ui.set_playing(false);
                ui.set_status(format!("Stream-Safe Music: \"{title}\" isn't safe on stream, and nothing after it in the queue is.").into());
            }
        }
    }
    true
}

// ---------- musical alerts (as js/alertmedia.js): the widget says it's showing one ----------
// The music pauses, the alert's sound plays here (after its delay, at its measured level and the
// alerts' volume), and the music fades back in once it's done.
fn alert_sound_file(kind: &str) -> Option<std::path::PathBuf> {
    let dir = omnipulse_core::data_dir().ok()?.join("alert-sounds");
    std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| p.file_stem().and_then(|s| s.to_str()) == Some(kind))
}

fn alert_shown(shared: &Shared, kind: &str) {
    let (e, vol, gain, engine, music_vol) = {
        let a = shared.borrow();
        let e = a.web.prefs["alertEdit"][kind].clone();
        let lv = &a.web.prefs["alertLevels"][kind];
        let gain = lv["auto"].as_f64().unwrap_or(0.0) + lv["adj"].as_f64().unwrap_or(0.0);
        let vol = a.web.prefs["alertVol"].as_f64().unwrap_or(0.8);
        let music_vol = a.ui.upgrade().map_or(0.8, |ui| ui.get_volume() / 100.0);
        (e, vol, gain, a.engine.clone(), music_vol)
    };
    if e["music"].as_bool() != Some(true) { return; }
    let Some(path) = alert_sound_file(kind) else { return };
    let Ok(src) = omnipulse_audio::Decoded::open_file(&path) else { return };
    let device = webstate::native_pref("nativeDevice").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    let Ok(player) = omnipulse_audio::Engine::new(&device) else { return };
    let player = Arc::new(player);
    let was_playing = shared.borrow().loaded && !engine.is_paused();
    if was_playing { engine.pause(); }
    player.load(Box::new(src), 0.0);
    player.set_volume(((vol * 10f64.powf(gain / 20.0)) as f32).clamp(0.0, 1.0));
    let events = player.events();
    let delay = e["sdelay"].as_u64().unwrap_or(0);
    let p2 = player.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(delay), move || p2.play());
    // done: the music back, faded in over a second and a half
    let s = shared.clone();
    let t = slint::Timer::default();
    let mut fading: Option<u32> = None;
    t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(100), move || {
        let _keep = &player;
        if fading.is_none() {
            let ended = events.as_ref().is_some_and(|rx| rx.try_iter().any(|ev| matches!(ev, omnipulse_audio::Event::Ended | omnipulse_audio::Event::Error(_))));
            if !ended { return; }
            if !was_playing || !s.borrow().engine.is_paused() { ALERT_TIMER.with(|x| *x.borrow_mut() = None); return; }
            let e = s.borrow().engine.clone();
            e.set_volume(0.0);
            e.play();
            fading = Some(0);
            return;
        }
        let n = fading.unwrap() + 1;
        fading = Some(n);
        s.borrow().engine.set_volume(music_vol * (n as f32 / 15.0).min(1.0));
        if n >= 15 { ALERT_TIMER.with(|x| *x.borrow_mut() = None); }
    });
    ALERT_TIMER.with(|x| *x.borrow_mut() = Some(t));
}

fn outside_start(shared: &Shared) {
    let prefs = shared.borrow().web.prefs.clone();
    omnipulse_core::widget::set_shown_handler(std::sync::Arc::new(|kind: &str| {
        let kind = kind.to_string();
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { alert_shown(&sh, &kind); } }));
    }));
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
            omnipulse_core::requests::set_asker(std::sync::Arc::new(|id, ask: serde_json::Value| {
                let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| {
                    let Some(sh) = s.borrow().clone() else { return };
                    let user = ask["user"].as_str().unwrap_or("").to_string();
                    if ask["action"].as_str() == Some("wrongsong") {
                        wrong_song(&sh, id, &user, ask["mod"].as_bool() == Some(true));
                    } else {
                        song_request(&sh, id, ask["q"].as_str().unwrap_or("").to_string(), user);
                    }
                }));
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
        "next" => skip_press(shared, 1),
        "prev" => skip_press(shared, -1),
        "shuffle" => {
            let mut a = shared.borrow_mut();
            let lp = shuffle_basis(&a);
            let sp = spacing(&a);
            a.queue.shuffle_rest(&lp, sp);
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
    // Settings > App > Rounded Corners (the web app's setting too)
    let rounded = prefs["appRounded"].as_bool() != Some(false);
    t.set_panel_radius(if rounded { 10.0 } else { 0.0 });
    ui.set_rounded(rounded);
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
    ui.set_tab_right(prefs["tabAlign"].as_str() == Some("right"));
    ui.set_player_style(prefs["playerStyle"].as_str().filter(|s| matches!(*s, "bar" | "float" | "minimal")).unwrap_or("tab").into());
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
    listen_moved();   // (listening along: your pause or play brings it back in step with the host)
    // (casting: what the device had buffered while paused is old, so it starts afresh)
    if engine.is_paused() { engine.play(); ui.set_playing(true); if casting().is_some() { omnipulse_core::cast::reload(); } } else { engine.pause(); ui.set_playing(false); }
    save_session(&shared.borrow());
    push_out(shared, true);
}

fn main() -> Result<(), slint::PlatformError> {
    // one OmniPulse at a time: a second one hands its link (or "show") to the first, and closes
    let link_arg = std::env::args().find(|x| x.starts_with("omnipulse://"));
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() {
        let claimed = instance::claim(link_arg.clone(), |msg| {
            let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| {
                if let Some(sh) = s.borrow().clone() {
                    desktop_action(&sh, "show");
                    if msg.starts_with("omnipulse://") { open_link(&sh, &msg); }
                }
            }));
        });
        if claimed.is_err() { return Ok(()); }
    }
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
    // the output device picked last time (in the bar's menu), if it's still there
    if let Some(name) = webstate::native_pref("nativeDevice").and_then(|v| v.as_str().map(String::from)).filter(|n| !n.is_empty()) {
        let _ = engine.set_device(&name);
    }
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
        here: None, history: Vec::new(), artists: None, fav_only: false, page_query: String::new(), picked: Default::default(), pick_anchor: None, shuffle_on: false, shuffle_backup: None, repeat: "off".into(), closest: None,
        autoplay: true, ap_busy: false, ap_then_play: false, ap_failed: None, resume_ms: None, loaded: false, accent: ("#8e9bff".into(), "#8e9bff".into()), started_secs: 0, token: 0,
        lyrics: Vec::new(), heard: (String::new(), 0.0, false), last_pos: 0, xpl: xpl::load(), naming: None,
        loud: HashMap::new(), eq_model: Rc::new(VecModel::from(vec![0.0f32; 10])), add_map: Vec::new(), queue_spaced: false, analysis: HashMap::new(), cur_loud: None,
    }));
    // (before anything is listed: Go+ tracks only while YouTube Music can play them)
    go_plus_set(&shared.borrow().web.prefs);

    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    // the plugins that are on, each started on its own thread
    // the plugins that come with OmniPulse (beside it, in plugins/): installed, or brought up to date
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("plugins"))).filter(|d| d.is_dir()) {
        omnipulse_core::plugins::install_bundled(&dir);
    }
    plugins_start(&shared);
    // OmniPulse Offset, if it's on (not in the self-test: it would play)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { offset_apply(&shared); }
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
    lib_sort_paint(&shared.borrow());
    let s = shared.clone();
    ui.on_sort_pick(move |by| lib_sort_pick(&s, &by));
    let s = shared.clone();
    ui.on_backup_make(move || backup_make(&s));
    // Jams
    let s = shared.clone();
    ui.on_jam_clicked(move || {
        if listening().is_some() {
            listen_leave(&s, false);
        } else if room_kind().as_deref() == Some("jam") {
            if let Some(ui) = s.borrow().ui.upgrade() { jam_paint(&s); ui.set_jam_open(true); }
        } else {
            jam_start(&s);
        }
    });
    let s = shared.clone();
    ui.on_jam_accept(move || jam_accept(&s));
    let s = shared.clone();
    ui.on_jam_decline(move || {
        if let Some((id, _, _, join)) = ASK.with(|a| a.borrow_mut().take()) { ask_answer(&id, join, false, false); }
        jam_paint(&s);
    });
    let s = shared.clone();
    ui.on_jam_copy(move || {
        let link = ROOM.with(|r| r.borrow().as_ref().map(|r| r.join_url.clone())).unwrap_or_default();
        if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(match share::copy(&link) { Ok(()) => "Invite link copied.".to_string(), Err(e) => e }.into()); }
    });
    let s = shared.clone();
    ui.on_jam_end(move || {
        room_end(true);
        if let Some(ui) = s.borrow().ui.upgrade() { ui.set_jam_open(false); ui.set_status("Ended. Listeners carry on with their own music.".into()); }
        jam_paint(&s);
        push_out(&s, true);
    });
    jam_paint(&shared);
    {
        // the ask's ring, and what's playing to the room every few seconds (drift is caught)
        let s = shared.clone();
        let t = slint::Timer::default();
        let mut n = 0u32;
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
            n += 1;
            if ASK.with(|a| a.borrow().is_some()) { jam_paint(&s); }
            if n.is_multiple_of(32) { host_push(&s.borrow()); }
        });
        JAM_TIMER.with(|x| *x.borrow_mut() = Some(t));
    }
    accounts_refresh(&shared);
    let s = shared.clone();
    ui.on_account_act(move |i| account_act(&s, i as usize));
    folders_paint(&shared);
    let s = shared.clone();
    ui.on_folder_add(move || {
        let Some(dir) = rfd::FileDialog::new().set_title("Add a music folder").pick_folder() else { return };
        let mut folders = omnipulse_core::local::local_folders();
        folders.push(dir.to_string_lossy().into_owned());
        folders_set(&s, folders);
    });
    let s = shared.clone();
    ui.on_folder_remove(move |i| {
        let mut folders = omnipulse_core::local::local_folders();
        if (i as usize) < folders.len() { folders.remove(i as usize); }
        folders_set(&s, folders);
    });
    let s = shared.clone();
    ui.on_folders_rescan(move || folders_rescan(&s));
    ui.set_eq_gains(ModelRc::from(shared.borrow().eq_model.clone()));
    ui.set_eq_presets(ModelRc::from(Rc::new(VecModel::from(sound::EQ_PRESETS.iter().map(|(n, _)| slint::SharedString::from(*n)).collect::<Vec<_>>()))));
    sound_paint(&shared.borrow());
    let s = shared.clone();
    ui.on_loud_pick(move |step| loud_pick(&s, &step));
    let s = shared.clone();
    ui.on_fade_on_changed(move |on| sound_set(&s, "fadeOn", serde_json::json!(on)));
    let s = shared.clone();
    ui.on_fade_ms_pick(move |ms| sound_set(&s, "fadeMs", serde_json::json!(ms)));
    let s = shared.clone();
    ui.on_skip_pick(move |secs| sound_set(&s, "skipSilence", serde_json::json!(secs)));
    let s = shared.clone();
    ui.on_eq_on_changed(move |on| sound_set(&s, "eqOn", serde_json::json!(on)));
    let s = shared.clone();
    ui.on_eq_band(move |i, g| {
        let mut bands = sound::eq_bands(&s.borrow().web.prefs);
        if let Some(b) = bands.get_mut(i as usize) { *b = g.clamp(-12.0, 12.0); }
        sound_set(&s, "eq", serde_json::json!(bands));
    });
    let s = shared.clone();
    ui.on_eq_preset(move |i| {
        if let Some((_, g)) = sound::EQ_PRESETS.get(i as usize) { sound_set(&s, "eq", serde_json::json!(g)); }
    });
    let s = shared.clone();
    ui.on_pl_new(move || pl_create(&s, Vec::new()));
    let s = shared.clone();
    ui.on_genre_search(move |g| go(&s, Place::Search(g.to_string())));
    let s = shared.clone();
    ui.on_pl_import(move || pl_import(&s));
    let s = shared.clone();
    ui.on_update_act(move || update_act(&s));
    // a newer version: asked about a little after starting (not in the self-test)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() {
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(20), move || update_check(&s, false));
    }
    let s = shared.clone();
    ui.on_name_done(move |name| name_done(&s, &name));
    let s = shared.clone();
    ui.on_side_action(move |i, act| side_action(&s, i as usize, &act));
    let s = shared.clone();
    ui.on_bar_pl(move |k| {
        let Some(t) = s.borrow().queue.current().cloned() else { return };
        if k < 0 { pl_create(&s, vec![t]); } else { pl_add(&s, k as usize, t); }
    });
    let s = shared.clone();
    ui.on_backup_restore(move || backup_restore(&s));
    let s = shared.clone();
    ui.on_rounded_changed(move |on| set_look_pref(&s, "appRounded", serde_json::json!(on)));
    let s = shared.clone();
    ui.on_player_style_picked(move |style| set_look_pref(&s, "playerStyle", serde_json::json!(style.as_str())));
    let s = shared.clone();
    ui.on_tab_align_picked(move |side| set_look_pref(&s, "tabAlign", serde_json::json!(side.as_str())));
    let s = shared.clone();
    ui.on_plat_toggle(move |p| plat_toggle(&s, &p));
    // Streamer Mode (as the web app's): kept with the rest of this app's settings
    ui.set_streamer(integrations::set(&shared.borrow().web.prefs, "streamerMode"));
    ui.set_stream_safe(streamsafe::on(&shared.borrow().web.prefs));
    let s = shared.clone();
    ui.on_streamer_step(move || streamer_step(&s));
    let s = shared.clone();
    ui.on_streamer_changed(move |on| {
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("streamerMode".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("streamerMode", serde_json::json!(on)); }
    });
    paint_plats(&shared.borrow());
    ui.set_fav_page(integrations::on(&shared.borrow().web.prefs, "favPage"));
    // an invite link the app was opened with (omnipulse://jam/... or join/...): joined once it's up
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    if let Some(link) = link_arg {
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(1500), move || open_link(&s, &link));
    }
    // everyone's changes to your collaborative playlists: a moment after starting, then every minute
    {
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(5), move || collab_pull_all(&s));
        let s = shared.clone();
        let t = slint::Timer::default();
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(collab::PULL_SECS), move || collab_pull_all(&s));
        std::mem::forget(t);
    }
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
    library_load(&shared);

    let s = shared.clone();
    ui.on_nav(move |p| go(&s, Place::Page(p.into())));
    let s = shared.clone();
    ui.on_back(move || go_back(&s));
    // a page fetched again (Discover, Rediscover, Sets & Podcasts, Artists)
    let s = shared.clone();
    ui.on_refresh(move || {
        let page = s.borrow().page.clone();
        if page == "artists" { s.borrow_mut().artists = None; }
        open_page(&s, &page);
    });
    let s = shared.clone();
    ui.on_set_toggle(move |key| set_toggle(&s, &key));
    let s = shared.clone();
    ui.on_section_flip(move |t| section_flip(&s, &t));
    // the key setup
    let s = shared.clone();
    ui.on_ks_open_url(move |i| {
        let Some(plat) = KS.with(|k| k.borrow().as_ref().map(|x| x.0.clone())) else { return };
        if let Some(url) = keysetup::setup(&plat).and_then(|st| st.steps.get(i as usize).and_then(|x| x.url)) {
            if let Err(e) = omnipulse_core::open_url(url.to_string()) { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(e.into()); } }
        }
    });
    let s = shared.clone();
    ui.on_ks_copy(move |i, k| {
        let Some(plat) = KS.with(|x| x.borrow().as_ref().map(|x| x.0.clone())) else { return };
        if let Some((label, value)) = keysetup::setup(&plat).and_then(|st| st.steps.get(i as usize).and_then(|x| x.copies.get(k as usize).cloned())) {
            if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(match share::copy(&value) { Ok(()) => format!("{label} copied."), Err(e) => e }.into()); }
        }
    });
    let s = shared.clone();
    ui.on_ks_set(move |i, v| {
        KS.with(|k| { if let Some(x) = k.borrow_mut().as_mut() { if let Some(f) = x.2.get_mut(i as usize) { *f = v.to_string(); } } });
        // (the ticks: only the labels change while a key is typed)
        let Some((plat, _, vals)) = KS.with(|k| k.borrow().clone()) else { return };
        let Some(st) = keysetup::setup(&plat) else { return };
        if let Some(ui) = s.borrow().ui.upgrade() {
            let model = ui.get_ks_fields();
            if let (Some(mut f), Some((_, _, re))) = (model.row_data(i as usize), st.fields.get(i as usize)) {
                f.value = v.clone();
                f.ok = keysetup::looks_right(re, &vals[i as usize]);
                model.set_row_data(i as usize, f);
            }
        }
    });
    let s = shared.clone();
    ui.on_ks_save(move || ks_save(&s));
    let s = shared.clone();
    ui.on_set_more(move |key| { if let Some(id) = key.strip_prefix("plugOn_") { pf_open(&s, id); } });
    let s = shared.clone();
    ui.on_pf_set(move |i, v| pf_set(&s, i as usize, &v));
    let s = shared.clone();
    ui.on_pf_save(move || pf_save(&s));
    paint_settings(&shared.borrow());
    let s = shared.clone();
    ui.on_like_toggle(move || like_toggle(&s));
    let s = shared.clone();
    ui.on_fav_toggle(move || fav_toggle(&s));
    // where the sound plays: Windows' devices, the one in use marked; kept for next time
    {
        let weak = ui.as_weak();
        ui.on_devices_want(move || {
            let Some(ui) = weak.upgrade() else { return };
            let mut list = vec!["Windows' default".to_string()];
            list.extend(omnipulse_audio::Engine::devices());
            let want = webstate::native_pref("nativeDevice").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
            ui.set_device_on(list.iter().position(|d| *d == want).map_or(0, |i| i as i32));
            // cast devices on the network (found in the background: the menu has more next time it opens)
            let _ = omnipulse_core::cast::discover_start();
            let cast = omnipulse_core::cast::devices();
            let on = casting();
            if let Some(i) = on.as_ref().and_then(|n| cast.iter().position(|d| d.name == *n)) { ui.set_device_on((list.len() + i) as i32); }
            list.extend(cast.iter().map(|d| format!("Cast to {}", d.name)));
            CAST_LIST.with(|c| *c.borrow_mut() = cast);
            ui.set_devices(ModelRc::from(Rc::new(VecModel::from(list.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()))));
        });
    }
    let s = shared.clone();
    ui.on_device_pick(move |i| {
        let Some(ui) = s.borrow().ui.upgrade() else { return };
        let Some(name) = ui.get_devices().row_data(i as usize).map(|d| d.to_string()) else { return };
        // a cast device (they come after Windows' devices)
        if let Some(d) = name.strip_prefix("Cast to ").and_then(|n| CAST_LIST.with(|c| c.borrow().iter().find(|d| d.name == n).cloned())) {
            if casting().as_deref() == Some(d.name.as_str()) { return; }
            if casting().is_some() { omnipulse_core::cast::stop(); cast_end(&s, None); }
            ui.set_device_on(i);
            return cast_start(&s, d);
        }
        if casting().is_some() { cast_stop(&s); }
        let engine = s.borrow().engine.clone();
        let use_name = if i == 0 { String::new() } else { name.clone() };
        match engine.set_device(&use_name) {
            Ok(()) => {
                ui.set_device_on(i);
                webstate::save_pref("nativeDevice", serde_json::json!(use_name));
                ui.set_status(format!("Playing on {name}.").into());
            }
            Err(e) => ui.set_status(format!("Couldn't play on {name}: {e}").into()),
        }
    });
    // Track preferences
    let s = shared.clone();
    ui.on_tp_save(move || tp_save(&s, false));
    let s = shared.clone();
    ui.on_tp_reset(move || tp_save(&s, true));
    let s = shared.clone();
    ui.on_tp_version(move |k| {
        let (Some(t), Some(c)) = (TP_TRACK.with(|x| x.borrow().clone()), TP_VERSIONS.with(|v| v.borrow().get(k as usize).cloned())) else { return };
        // (kept: it plays that one from now on, as the web app's ytChooseMatch)
        versions::keep(&t, &c);
        if let Some(ui) = s.borrow().ui.upgrade() { ui.set_tp_version_on(k); ui.set_status(format!("\"{}\" plays \"{}\" from YouTube Music from now on.", t.title, c.title).into()); }
    });
    let s = shared.clone();
    ui.on_tp_link(move |link| {
        // (as the web app's: a pasted link's video, kept as what it plays)
        let (Some(t), Some(id)) = (TP_TRACK.with(|x| x.borrow().clone()), yt_link_id(&link)) else { return };
        if TP_VERSIONS.with(|v| v.borrow().first().is_some_and(|c| c.id == id)) { return; }
        let (rt, weak) = { let a = s.borrow(); (a.rt.clone(), a.ui.clone()) };
        rt.spawn(async move {
            let found = omnipulse_core::youtube::radio(&id).await.ok().and_then(|l| l.into_iter().find(|c| c.id == id));
            let _ = weak.upgrade_in_event_loop(move |ui| {
                let Some(c) = found else { ui.set_status("That link's video couldn't be found on YouTube Music.".into()); return };
                versions::keep(&t, &c);
                TP_VERSIONS.with(|v| { let mut v = v.borrow_mut(); v.retain(|x| x.id != c.id); v.insert(0, c.clone()); });
                let names: Vec<slint::SharedString> = TP_VERSIONS.with(|v| v.borrow().iter().map(|c| format!("{} · {} · {}", c.title, c.artists, fmt_ms(c.duration_ms)).into()).collect());
                ui.set_tp_versions(ModelRc::from(Rc::new(VecModel::from(names))));
                ui.set_tp_version_on(0);
                ui.set_status(format!("\"{}\" plays \"{}\" from YouTube Music from now on.", t.title, c.title).into());
            });
        });
    });
    let s = shared.clone();
    ui.on_tp_now(move |which| {
        let pos = s.borrow().engine.position_ms();
        if let Some(ui) = s.borrow().ui.upgrade() { if which == 0 { ui.set_tp_start(fmt_time(pos).into()); } else { ui.set_tp_end(fmt_time(pos).into()); } }
    });
    // the track's menu
    let s = shared.clone();
    ui.on_more_want(move || {
        let a = s.borrow();
        let Some(ui) = a.ui.upgrade() else { return };
        let mut items: Vec<slint::SharedString> = Vec::new();
        if let Some(t) = a.queue.current() {
            if !t.web_link().is_empty() { items.push(format!("Open on {}", match t.plat() { "spotify" => "Spotify", "youtube" => "YouTube Music", _ => "SoundCloud" }).into()); }
            if !t.artist.is_empty() { items.push(format!("Search for {}", t.artist.split(", ").next().unwrap_or("")).into()); }
            items.push("Search for this song".into());
            items.push("Track preferences...".into());
            for (plat, _) in share::links(t) { items.push(format!("Copy {} link", share::name(plat)).into()); }
        }
        ui.set_more_items(ModelRc::from(Rc::new(VecModel::from(items))));
    });
    let s = shared.clone();
    ui.on_more_pick(move |i| {
        let Some(ui) = s.borrow().ui.upgrade() else { return };
        let Some(item) = ui.get_more_items().row_data(i as usize).map(|x| x.to_string()) else { return };
        let Some(t) = s.borrow().queue.current().cloned() else { return };
        if item == "Track preferences..." { return tp_open(&s, &t); }
        if let Some(name) = item.strip_prefix("Copy ").and_then(|x| x.strip_suffix(" link")) {
            if let Some((plat, _)) = share::PLATS.iter().find(|(_, n)| *n == name) { copy_track_link(&s, &t, plat); }
        } else if item.starts_with("Open on") {
            if let Err(e) = omnipulse_core::open_url(t.web_link()) { ui.set_status(e.into()); }
        } else {
            let q = if item == "Search for this song" { format!("{} {}", t.artist.split(", ").next().unwrap_or(""), t.title) } else { t.artist.split(", ").next().unwrap_or("").to_string() };
            ui.set_search_text(q.clone().into());
            go(&s, Place::Search(q));
        }
    });
    let s = shared.clone();
    ui.on_artist(move |i| go(&s, Place::Artist(i as usize)));
    let s = shared.clone();
    ui.on_side(move |i| {
        let at = s.borrow().side_map.get(i as usize).copied();
        let id = at.and_then(|at| s.borrow().playlists.get(at).map(|p| p.id.clone()));
        if let Some(id) = id { go(&s, Place::Playlist(id)); }
    });
    let s = shared.clone();
    ui.on_search(move |q| { if !q.trim().is_empty() { go(&s, Place::Search(q.trim().into())); } });
    let s = shared.clone();
    ui.on_search_edited(move |q| {
        let mut a = s.borrow_mut();
        if a.page == "search" || a.page == "settings" { return; }   // (there, Enter searches)
        a.page_query = q.trim().to_string();
        apply_filter(&mut a);
    });
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
        if page == "library" && i == 3 {
            return library_albums(&s);
        }
        if ui.get_page_kind() == "albumgrid" { ui.set_page_kind("tracks".into()); }
        let mut a = s.borrow_mut();
        a.chip = i as usize;
        apply_filter(&mut a);
    });
    let s = shared.clone();
    ui.on_album_tile(move |i| album_open(&s, i as usize));
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
        if s.borrow().page == "search" { return open_found(&s, sh as usize, k as usize); }
        if s.borrow().page == "streamer" { return open_stream_coll(&s, sh as usize, k as usize); }
        if s.borrow().page == "discover" { return discover_tile(&s, sh as usize, k as usize); }
        let list = s.borrow().shelves.get(sh as usize).cloned().unwrap_or_default();
        play_list(&s, list, k as usize);
    });
    let s = shared.clone();
    ui.on_taste_pick(move || taste_open(&s, 1));
    let s = shared.clone();
    ui.on_taste_toggle(move |r, k| taste_toggle(&s, r as usize, k as usize));
    let s = shared.clone();
    ui.on_taste_save(move || taste_save(&s));
    let s = shared.clone();
    ui.on_taste_back(move || taste_open(&s, 1));
    let s = shared.clone();
    ui.on_taste_skip(move || taste_done(&s));
    let s = shared.clone();
    ui.on_lucky(move || {
        // a station in the vein of something you like, made only of music you haven't liked anywhere
        let (seeds, rt, weak, liked) = {
            let a = s.borrow();
            let mut pool: Vec<Track> = a.library.iter().filter(|t| !t.is_mix()).cloned().collect();
            queue::shuffle_in_place(&mut pool);
            (pool.into_iter().take(3).collect::<Vec<_>>(), a.rt.clone(), a.ui.clone(), track::Liked::of(&a.library))
        };
        if seeds.is_empty() { return; }
        if let Some(ui) = weak.upgrade() { ui.set_status(format!("Feeling lucky: new music like {}...", seeds[0].artist).into()); }
        rt.spawn(async move {
            // (one that comes back too short is passed over for the next)
            for seed in seeds {
                let list: Vec<Track> = sources::related(&seed).await.into_iter().filter(|t| !t.is_mix() && !liked.has(t)).collect();
                if list.len() < 8 { continue; }
                let who = seed.artist.clone();
                let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| if let Some(sh) = s.borrow().clone() {
                    play_list(&sh, list, 0);
                    ui.set_status(format!("Feeling lucky: new music like {who}, none of it in your likes.").into());
                }));
                return;
            }
            let _ = weak.upgrade_in_event_loop(|ui| ui.set_status("Couldn't find enough new music for a mix just now. Try again in a bit.".into()));
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
        // a picked one: every one that's picked (in the page's order)
        if a.picked.len() > 1 && a.picked.contains(&t.key) && !act.starts_with("link:") && listening().is_none() {
            let all: Vec<Track> = a.page_tracks.iter().filter(|x| a.picked.contains(&x.key)).cloned().collect();
            let n = all.len();
            drop(a);
            match act.as_str() {
                "plnew" => pl_create(&s, all),
                a if a.starts_with("pl") => { for x in all { pl_row_action(&s, a, x); } }
                _ => {
                    let empty = s.borrow().queue.current().is_none();
                    {
                        let mut a = s.borrow_mut();
                        // (Play Next in reverse, so they play in the page's order)
                        if act == "next" { for x in all.into_iter().rev() { a.queue.play_next(x); } } else { for x in all { a.queue.add(x); } }
                        paint_queue(&mut a);
                        save_session(&a);
                    }
                    if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(format!("{} {n} tracks.", if act == "next" { "Playing next:" } else { "Added to the queue:" }).into()); }
                    if empty { start(&s); }
                }
            }
            return;
        }
        if act.starts_with("pl") {
            drop(a);
            return pl_row_action(&s, &act, t);
        }
        if let Some(plat) = act.strip_prefix("link:") {
            drop(a);
            return copy_track_link(&s, &t, plat);
        }
        if let Some(kind) = listening() {
            drop(a);
            let note = if kind != "jam" {
                "You're listening along: only a Jam takes songs from listeners.".to_string()
            } else {
                listen_send(serde_json::json!({ "type": "add", "track": t.to_snapshot(), "next": act == "next" }));
                format!("Sent \"{}\" to the Jam{}", t.title, if act == "next" { ", to play next." } else { "." })
            };
            if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(note.into()); }
            return;
        }
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
    // picking tracks on the page
    let s = shared.clone();
    ui.on_row_pick(move |i, mode| {
        let mut a = s.borrow_mut();
        let i = i as usize;
        let Some(key) = a.page_tracks.get(i).map(|t| t.key.clone()) else { return };
        match (mode, a.pick_anchor) {
            (2, Some(from)) => {
                let (lo, hi) = (from.min(i), from.max(i));
                let keys: Vec<String> = a.page_tracks[lo..=hi.min(a.page_tracks.len() - 1)].iter().map(|t| t.key.clone()).collect();
                a.picked.extend(keys);
            }
            _ => { if !a.picked.remove(&key) { a.picked.insert(key); } a.pick_anchor = Some(i); }
        }
        pick_paint(&mut a);
    });
    let s = shared.clone();
    ui.on_pick_all(move || {
        let mut a = s.borrow_mut();
        let keys: Vec<String> = a.page_tracks.iter().map(|t| t.key.clone()).collect();
        a.picked.extend(keys);
        pick_paint(&mut a);
    });
    let s = shared.clone();
    ui.on_pick_none(move || {
        let mut a = s.borrow_mut();
        if a.picked.is_empty() { return; }
        a.picked.clear();
        a.pick_anchor = None;
        pick_paint(&mut a);
    });
    let s = shared.clone();
    ui.on_queue_action(move |i, act| {
        let mut a = s.borrow_mut();
        let Some(at) = a.panel_map.get(i as usize).copied() else { return };
        if let Some(plat) = act.strip_prefix("link:") {
            let Some(t) = a.queue.items.get(at).map(|q| q.t.clone()) else { return };
            drop(a);
            return copy_track_link(&s, &t, plat);
        }
        match act.as_str() {
            "next" => a.queue.move_next(at),
            "remove" => a.queue.remove(at),
            _ => {
                let lp = shuffle_basis(&a);
                let sp = spacing(&a);
                a.queue.shuffle_rest(&lp, sp);
            }
        }
        paint_queue(&mut a);
        save_session(&a);
    });
    let s = shared.clone();
    ui.on_shuffle(move || {
        let (list, spaced) = {
            let a = s.borrow();
            let spaced = a.page == "library" || a.page == "playlist";
            let list = queue::fair_shuffle(a.page_tracks.clone(), &shuffle_basis(&a));
            (if spaced && a.repeat == "off" { queue::space_recent(list, &a.web.last_played, recent_window(&a.web.prefs), now_ms()) } else { list }, spaced)
        };
        {
            let mut a = s.borrow_mut();
            a.queue.set_list(list, 0);
            a.queue_spaced = spaced;
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
    ui.on_next(move || skip_press(&s, 1));
    let s = shared.clone();
    ui.on_prev(move || skip_press(&s, -1));
    let s = shared.clone();
    ui.on_toggle(move || toggle(&s));
    let s = shared.clone();
    ui.on_seek(move |f| {
        let engine = s.borrow().engine.clone();
        listen_moved();   // (listening along: scrubbing here brings it back in step with the host)
        // within the part that plays (a track's start and end times)
        let (from, to) = trim_now(&s);
        let d = to.unwrap_or(engine.duration_ms()).saturating_sub(from);
        if d > 0 {
            engine.seek(from + (f.clamp(0.0, 1.0) as f64 * d as f64) as u64);
            let s2 = s.clone();
            slint::Timer::single_shot(std::time::Duration::from_millis(300), move || push_out(&s2, true));
        }
    });
    // (casting: the slider is the device's volume)
    ui.on_volume_changed({ let engine = engine.clone(); move |v| if casting().is_some() { omnipulse_core::cast::set_volume(v as f64) } else { engine.set_volume(v) } });

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
        // (OMNIPULSE_SELFTEST_JOIN=<invite link>: only joining someone's room, following it, skipping and leaving)
        let join = std::env::var("OMNIPULSE_SELFTEST_JOIN").ok();
        let full: [(u64, &str); 46] = [(9, "play"), (13, "library"), (13, "pagefind"), (14, "pageall"), (14, "pick"), (14, "picknone"), (14, "scoff"), (15, "scon"), (15, "albums"), (18, "albumsn"), (18, "discover"), (24, "rediscover"), (27, "sets"), (31, "stats"), (35, "now"), (36, "tprefs"), (37, "tpclose"), (39, "search"), (41, "found"), (42, "settings"), (43, "plugset"), (44, "plugclose"), (44, "keys"), (45, "keysclose"), (45, "light"), (45, "collapse"), (46, "artists"), (54, "artist"), (60, "autoplay"), (70, "narrow"), (72, "goplus"), (80, "tabright"), (81, "pbar"), (82, "pfloat"), (83, "pminimal"), (84, "musical"), (88, "musical2"), (90, "safe"), (90, "streamer"), (91, "streamcoll"), (91, "sort"), (93, "xpl"), (95, "xpl2"), (97, "fade"), (101, "fade2"), (103, "end")];
        // (OMNIPULSE_SELFTEST_PLAN=discover: only Discover, its shelves, and the first mix it has opened)
        let only_discover = std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("discover");
        let plan: Vec<(u64, &str)> = if join.is_some() { vec![(6, "join"), (14, "joined"), (16, "jamskip"), (19, "leave"), (21, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("mix") { vec![(9, "play"), (10, "mixprep"), (70, "mixseek"), (82, "mixcheck"), (100, "mixcheck"), (106, "mixcheck"), (130, "mixcheck"), (132, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("rhythm") { vec![(9, "play"), (100, "rhythm"), (101, "end")] }
            else if only_discover { vec![(9, "play"), (11, "discover"), (30, "disc"), (45, "discmix"), (12, "import"), (46, "taste"), (47, "taste2"), (48, "taste3"), (70, "tasted"), (71, "marquee"), (72, "marqueeoff"), (73, "end")] } else { full.to_vec() };
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
                // typing in the search bar filters the Library, as you type
                "pagefind" => { let n = ui.get_rows().row_count(); ui.invoke_search_edited("crunkz".into()); println!("selftest: page filter: {n} rows, typing \"crunkz\": {}", ui.get_rows().row_count()); }
                // what the search found besides tracks
                "found" => println!("selftest: search shelves {}", ui.get_shelves().iter().map(|sh| format!("{} {}", sh.title, sh.tiles.row_count())).collect::<Vec<_>>().join(", ")),
                // with Stream-Safe Music on: the Streamer Mode page, and one of its collections
                "streamer" => { ui.invoke_nav("discover".into()); println!("selftest: streamer page \"{}\", shelves {}", ui.get_page_title(), ui.get_shelves().iter().map(|sh| format!("{} {}", sh.title, sh.tiles.row_count())).collect::<Vec<_>>().join(", ")); }
                "streamcoll" => ui.invoke_tile(0, 0),
                // Track preferences for what's playing, for its picture
                "tprefs" => { if let Some(t) = s.borrow().queue.current().cloned() { tp_open(&s, &t); } println!("selftest: track preferences open {}, \"{}\" by {}", ui.get_tp_open(), ui.get_tp_title(), ui.get_tp_artist()); }
                "tpclose" => ui.set_tp_open(false),
                // picking: two rows, a range with Shift, then none
                "pick" => { ui.invoke_row_pick(0, 1); ui.invoke_row_pick(4, 2); println!("selftest: picked {} (rows marked {}), status \"{}\"", s.borrow().picked.len(), ui.get_rows().iter().filter(|r| r.picked).count(), ui.get_status()); }
                "picknone" => { ui.invoke_pick_none(); println!("selftest: picked after Escape {}", s.borrow().picked.len()); }
                // the Library's Albums chip
                "albums" => { ui.invoke_nav("library".into()); ui.invoke_chip(3); }
                "albumsn" => println!("selftest: library albums {}, kind {}, \"{}\"", ui.get_album_tiles().row_count(), ui.get_page_kind(), ui.get_page_sub()),
                // a plugin's settings dialog (the Internet Archive's, if it's here), and a choice moved on
                "plugset" => { pf_open(&s, "internet-archive"); let before = ui.get_pf_fields().iter().map(|f| format!("{}={}", f.key, f.value)).collect::<Vec<_>>().join(" "); ui.invoke_pf_set(2, "".into()); println!("selftest: plugin settings {} -> {}", before, ui.get_pf_fields().iter().map(|f| format!("{}={}", f.key, f.value)).collect::<Vec<_>>().join(" ")); }
                "plugclose" => { PF_EDIT.with(|e| e.borrow_mut().take()); ui.set_pf_open(false); }
                // the key setup (YouTube's), for its picture; nothing is saved
                "keys" => { ks_open(&s, "youtube", 2); ui.invoke_ks_set(0, "123-abc.apps.googleusercontent.com".into()); println!("selftest: key setup \"{}\", steps {}, fields {}", ui.get_ks_head(), ui.get_ks_steps().row_count(), ui.get_ks_fields().iter().map(|f| format!("{}:{}", f.label, f.ok)).collect::<Vec<_>>().join(" ")); }
                "keysclose" => { KS.with(|k| k.borrow_mut().take()); ui.set_ks_open(false); }
                "pageall" => { ui.invoke_search_edited("".into()); println!("selftest: page filter cleared: {} rows", ui.get_rows().row_count()); }
                "join" => { if let Some(link) = &join { listen_join(&s, link); } }
                "joined" => println!("selftest: listening {:?}, playing \"{}\" at {}, queue {}, jam button \"{}\", status \"{}\"", listening(), ui.get_now_title(), ui.get_pos_text(), s.borrow().queue.items.len(), ui.get_jam_label(), ui.get_status()),
                "jamskip" => ui.invoke_next(),
                "leave" => { ui.invoke_jam_clicked(); println!("selftest: left: listening {:?}, queue {}, status \"{}\"", listening(), s.borrow().queue.items.len(), ui.get_status()); }
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
                // a Go+ track: it plays from YouTube Music, never its preview
                "goplus" => {
                    open_page(&s, "library");
                    let at = s.borrow().page_tracks.iter().position(|t| t.go_plus);
                    println!("selftest: go+ tracks listed {}", s.borrow().page_tracks.iter().filter(|t| t.go_plus).count());
                    if let Some(i) = at { ui.invoke_play_row(i as i32); }
                }
                // a musical alert while music plays: the music pauses, then comes back (a short tone,
                // written for the test and taken away after)
                "musical" => {
                    ui.invoke_player_style_picked("tab".into());
                    let dir = omnipulse_core::data_dir().unwrap().join("alert-sounds");
                    let _ = std::fs::create_dir_all(&dir);
                    let rate = 22050u32;
                    let n = rate as usize; // a second
                    let mut wav = Vec::new();
                    wav.extend(b"RIFF"); wav.extend(((36 + n * 2) as u32).to_le_bytes()); wav.extend(b"WAVEfmt ");
                    wav.extend(16u32.to_le_bytes()); wav.extend(1u16.to_le_bytes()); wav.extend(1u16.to_le_bytes());
                    wav.extend(rate.to_le_bytes()); wav.extend((rate * 2).to_le_bytes()); wav.extend(2u16.to_le_bytes()); wav.extend(16u16.to_le_bytes());
                    wav.extend(b"data"); wav.extend(((n * 2) as u32).to_le_bytes());
                    for i in 0..n { wav.extend((((i as f32 / rate as f32 * 2.0 * std::f32::consts::PI * 440.0).sin() * 3000.0) as i16).to_le_bytes()); }
                    let _ = std::fs::write(dir.join("selftest-alert.wav"), wav);
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() {
                        o.insert("alertEdit".into(), serde_json::json!({ "selftest-alert": { "music": true } }));
                    }
                    let before = !s.borrow().engine.is_paused();
                    alert_shown(&s, "selftest-alert");
                    println!("selftest: musical alert: playing before {before}, paused for it {}", s.borrow().engine.is_paused());
                }
                "musical2" => {
                    println!("selftest: musical alert: music back after it {}", !s.borrow().engine.is_paused());
                    ui.invoke_tab_align_picked("left".into());
                    let _ = std::fs::remove_file(omnipulse_core::data_dir().unwrap().join("alert-sounds").join("selftest-alert.wav"));
                }
                // a playlist on this PC: named (its picture), made with a track in it, another added, deleted
                "xpl" => {
                    let t = s.borrow().library.first().cloned();
                    if let Some(t) = t { pl_create(&s, vec![t]); }
                    let id = xpl::playlists(&s.borrow().xpl).first().map(|p| p.id.clone());
                    if let Some(id) = id { name_ask(&s, Naming::Rename(id)); }
                }
                "xpl2" => {
                    name_done(&s, "Self-test list");
                    let second = s.borrow().library.get(1).cloned();
                    if let Some(t) = second { pl_add(&s, 0, t); }
                    let n = xpl::playlists(&s.borrow().xpl).first().map(|p| (p.title.clone(), p.tracks.as_ref().map_or(0, Vec::len)));
                    println!("selftest: playlist {:?}, own lists {}, status \"{}\"", n, ui.get_own_lists().row_count(), ui.get_status());
                    let at = s.borrow().side_map.iter().position(|&i| s.borrow().playlists[i].title == "Self-test list");
                    if let Some(i) = at { side_action(&s, i, "delete"); }
                    println!("selftest: after delete, own lists {}", ui.get_own_lists().row_count());
                }
                // Fade Between Tracks: the sound "ends" just ahead (a made-up measurement), so it fades
                // out there and moves on
                "fade" => {
                    let pos = s.borrow().engine.position_ms();
                    let key = s.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default();
                    {
                        let mut a = s.borrow_mut();
                        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("fadeOn".into(), serde_json::json!(true)); o.insert("fadeMs".into(), serde_json::json!(2000)); }
                        a.analysis.insert("selftest".into(), serde_json::json!({ "audible_ms": pos + 1200, "analysed_ms": 0, "silences": [] }));
                        a.cur_loud = Some("selftest".into());
                    }
                    println!("selftest: fade from {key} at {pos} ms");
                    // (a moment later: both heard at once, the next one coming in)
                    let s2 = s.clone();
                    slint::Timer::single_shot(std::time::Duration::from_millis(1400), move || {
                        let a = s2.borrow();
                        println!("selftest: crossfading {}, now {}, at {} ms", a.engine.crossfading(), a.queue.current().map(|t| t.key.clone()).unwrap_or_default(), a.engine.position_ms());
                    });
                }
                "fade2" => {
                    let key = s.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default();
                    println!("selftest: fade: now playing {key}, fading out {}, moved on from {:?}", FADE.with(|f| f.borrow().0), FADE.with(|f| f.borrow().1.clone()));
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("fadeOn".into(), serde_json::json!(false)); }
                }
                // Settings: App & Look closed (its picture), then open again for the rest
                "collapse" => { section_flip(&s, "App & Look"); slint::Timer::single_shot(std::time::Duration::from_millis(1600), { let s = s.clone(); move || section_flip(&s, "App & Look") }); }
                // the player styles, for their pictures (the tab on the left again first; back to Tab after)
                "pbar" => { ui.invoke_tab_align_picked("left".into()); ui.invoke_player_style_picked("bar".into()); }
                "pfloat" => ui.invoke_player_style_picked("float".into()),
                "pminimal" => ui.invoke_player_style_picked("minimal".into()),
                // the Library by artist, Z to A
                "sort" => {
                    open_page(&s, "library");
                    ui.invoke_sort_pick("artist".into());
                    ui.invoke_sort_pick("artist".into());
                    let first: Vec<String> = s.borrow().page_tracks.iter().take(3).map(|t| t.artist.clone()).collect();
                    println!("selftest: sort {} {}, first {:?}", ui.get_sort_by(), ui.get_sort_dir(), first);
                }
                // the Streamer Mode button twice: private info, then only what is safe on stream
                "safe" => {
                    ui.invoke_streamer_step();
                    ui.invoke_streamer_step();
                    println!("selftest: stream-safe {}, rows shown {}, status \"{}\"", ui.get_stream_safe(), ui.get_rows().row_count(), ui.get_status());
                }
                // the track info tab at the right end, for its picture (and back after)
                "tabright" => { ui.invoke_tab_align_picked("right".into()); println!("selftest: tab right {}", ui.get_tab_right()); }
                // the narrow layout, for its picture
                "narrow" => { ui.window().set_size(slint::LogicalSize::new(900.0, 700.0)); }
                // the first artist you follow
                "artist" => { println!("selftest: following {} artists", ui.get_artists().row_count()); if ui.get_artists().row_count() > 0 { ui.invoke_artist(0); } }
                // Discover's shelves, then its first mix (or station) opened
                "disc" => {
                    println!("selftest: discover shelves {}", ui.get_shelves().iter().map(|sh| format!("{} {}", sh.title, sh.tiles.row_count())).collect::<Vec<_>>().join(", "));
                    let at = DISCOVER.with(|d| d.borrow().iter().position(|sh| sh.key == "mixes").map(|i| (i, 0)));
                    if let Some((i, k)) = at { ui.invoke_tile(i as i32, k as i32); }
                }
                "taste" => { ui.invoke_taste_pick(); println!("selftest: pick genres open {}, rows {}", ui.get_taste_open(), ui.get_taste_rows().row_count()); }
                "taste2" => {
                    // Rock and Electronic (rows of four: Pop, Hip-hop & Rap, Rock, Dance / Electronic, ...)
                    ui.invoke_taste_toggle(0, 2);
                    ui.invoke_taste_toggle(1, 0);
                    ui.invoke_taste_save();
                    println!("selftest: fine-tune step {}, groups {:?}", ui.get_taste_step(), ui.get_taste_rows().iter().filter(|r| r.head != "").map(|r| r.head.to_string()).collect::<Vec<_>>());
                }
                "taste3" => {
                    // the first subgenre of each group (Classic Rock, Dubstep)
                    let heads: Vec<usize> = ui.get_taste_rows().iter().enumerate().filter(|(_, r)| r.head != "").map(|(i, _)| i).collect();
                    for h in heads { ui.invoke_taste_toggle(h as i32 + 1, 0); }
                    ui.invoke_taste_save();
                    println!("selftest: picked {} / {}, open {}, status \"{}\"", s.borrow().web.prefs["tasteGenres"], s.borrow().web.prefs["tasteSubgenres"], ui.get_taste_open(), ui.get_status());
                }
                // the seek bar's marquee, for its picture
                "marquee" => ui.set_track_loading(true),
                // a mix: the next few measured ahead; then one at about this one's tempo moved up next, and
                // the one playing put a few seconds before its moment; then how it went
                "mixprep" => {
                    let next: Vec<Track> = { let a = s.borrow(); let i = a.queue.qi.unwrap_or(0); a.queue.items.iter().skip(i + 1).take(6).map(|q| q.t.clone()).collect() };
                    for t in next { rhythm_ahead(&s, t); }
                    println!("selftest: mix: measuring the next 6 ahead");
                }
                "mixseek" => {
                    let (cur, qi) = { let a = s.borrow(); (a.queue.current().cloned(), a.queue.qi.unwrap_or(0)) };
                    let Some(cur) = cur else { return };
                    let Some(a) = omnipulse_core::rhythm::known(&cur.key) else { println!("selftest: mix: the one playing isn't measured"); return };
                    let dur = s.borrow().engine.duration_ms();
                    let cands: Vec<(usize, Track)> = { let q = &s.borrow().queue; q.items.iter().enumerate().skip(qi + 1).take(6).map(|(i, x)| (i, x.t.clone())).collect() };
                    // (measured whole: the plan from its structure, as the mix makes it; else from what's measured of their starts)
                    let fa = rhythm_full(&cur.key);
                    println!("selftest: mix: the one playing measured whole: {}", fa.is_some());
                    let found = cands.into_iter().find_map(|(i, t)| match (&fa, rhythm_full(&t.key)) {
                        (Some(fa), Some(fb)) => mix::plan_structure(fa, dur, 0, &fb).map(|p| (i, t, fb, p)),
                        _ => omnipulse_core::rhythm::known(&t.key).and_then(|b| mix::plan(&a, dur, 0, &b, MIX_BARS).map(|p| (i, t, b, p))),
                    });
                    match found {
                        Some((i, t, b, p)) => {
                            { let mut a2 = s.borrow_mut(); let item = a2.queue.items.remove(i); a2.queue.items.insert(qi + 1, item); }
                            s.borrow().engine.seek(p.at_ms.saturating_sub(30_000));
                            println!("selftest: mix: {} ({} BPM, {}) into {} ({} BPM, {}) at {} ms over {} ms, from {} ms in, at {:.4}x, {:+} semitones", cur.title, a.rhythm.bpm, a.rhythm.camelot, t.title, b.rhythm.bpm, b.rhythm.camelot, p.at_ms, p.ms, p.start_ms, p.rate, p.semitones);
                        }
                        None => println!("selftest: mix: none of the next 6 has a tempo close enough (or isn't measured): {:?}", { let q = &s.borrow().queue; q.items.iter().skip(qi + 1).take(6).map(|x| omnipulse_core::rhythm::known(&x.t.key).map(|m| m.rhythm.bpm)).collect::<Vec<_>>() }),
                    }
                }
                "mixcheck" => {
                    let a = s.borrow();
                    let phase = MIX.with(|m| match &*m.borrow() { None => "none".to_string(), Some(MixPhase::Opening(_)) => "opening".into(), Some(MixPhase::Waiting { .. }) => "waiting".into(), Some(MixPhase::Mixing { tempo, .. }) => format!("mixing at {:.4}x", tempo.get()) });
                    let ear = MIX_EAR.with(|e| e.borrow().as_ref().map(|x| format!("listening again from {} ms", x.1))).or_else(|| MIX_LOCAL.with(|l| l.borrow().as_ref().map(|x| format!("measured again: {} BPM, downbeat {}, from {} ms", x.1.rhythm.bpm, x.1.rhythm.downbeat, x.1.from_ms)))).unwrap_or_default();
                    println!("selftest: mix: {phase}, engine crossfading {} mixing {}, now \"{}\" at {} ms; {ear}", a.engine.crossfading(), a.engine.mixing(), a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.engine.position_ms());
                }
                // what was measured of the track playing (its first 30%, as it played)
                "rhythm" => {
                    let key = s.borrow().queue.current().map(|t| format!("{} ({})", t.key, t.title)).unwrap_or_default();
                    let k = s.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default();
                    match omnipulse_core::rhythm::known(&k) {
                        Some(m) => println!("selftest: rhythm of {key}: {} BPM (confidence {:.2}), {} ({}), first beat {:.0} ms after {} ms, {} beats, downbeat {}", m.rhythm.bpm, m.rhythm.tempo_confidence, m.rhythm.key, m.rhythm.camelot, m.rhythm.first_beat_ms, m.from_ms, m.rhythm.beats.len(), m.rhythm.downbeat),
                        None => println!("selftest: rhythm of {key}: not measured (listening {})", RHYTHM_EAR.with(|e| e.borrow().as_ref().map(|l| format!("from {:?}, {} samples, wants {} ms", l.from_ms, l.samples.lock().map(|b| b.len()).unwrap_or(0), l.want_ms))).unwrap_or_default()),
                    }
                }
                // OMNIPULSE_SELFTEST_IMPORT=<an M3U or CSV>: imported as a playlist on this PC (shown after)
                "import" => if let Ok(f) = std::env::var("OMNIPULSE_SELFTEST_IMPORT") { pl_import_file(&s, f.into()); },
                "marqueeoff" => ui.set_track_loading(false),
                "tasted" => {
                    println!("selftest: imported {:?}, status \"{}\"", s.borrow().xpl.as_array().into_iter().flatten().filter(|x| x["title"].as_str() == Some("Import test")).map(|x| x["tracks"].as_array().map(|t| t.iter().map(|y| format!("{} [{}]", y["title"].as_str().unwrap_or(""), y["source"].as_str().unwrap_or(""))).collect::<Vec<_>>()).unwrap_or_default()).collect::<Vec<_>>(), ui.get_status());
                    println!("selftest: discover after picking: {}", ui.get_shelves().iter().map(|sh| format!("{} {}", sh.title, sh.tiles.row_count())).collect::<Vec<_>>().join(", "));
                }
                "discmix" => println!("selftest: mix \"{}\" ({}): {} tracks, first {:?}", ui.get_page_title(), ui.get_page_sub(), s.borrow().page_tracks.len(), s.borrow().page_tracks.iter().take(4).map(|t| format!("{} - {} [{}]", t.artist, t.title, t.plat())).collect::<Vec<_>>()),
                "end" => {
                    println!("selftest: status \"{}\"", ui.get_status());
                    println!("selftest: settings rows {}", ui.get_set_rows_integrations().iter().chain(ui.get_set_rows_library().iter()).chain(ui.get_set_rows_streaming().iter()).map(|r| format!("{}{}", r.label, if r.on { "+" } else { "-" })).collect::<Vec<_>>().join(", "));
                    println!("selftest: remote state {}", remote_state(&s.borrow()));
                    println!("selftest: folders {:?}, {}", ui.get_folders().iter().map(|f| f.to_string()).collect::<Vec<_>>(), ui.get_folders_note());
                    println!("selftest: share: first rows {:?}, sidebar {:?}", ui.get_rows().iter().take(3).map(|r| (r.sc_link, r.sp_link, r.yt_link)).collect::<Vec<_>>(), ui.get_sides().iter().take(3).map(|x| (x.title.to_string(), x.share, x.sc, x.sp)).collect::<Vec<_>>());
                    println!("selftest: accounts {}", ui.get_accounts().iter().map(|r| format!("{}: {}{}", r.name, if r.signed { "in, " } else { "out, " }, r.status)).collect::<Vec<_>>().join(" | "));
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
    static ARTIST_MODEL: RefCell<Option<Rc<VecModel<Item>>>> = const { RefCell::new(None) };
    /// the status line as it was last seen, and since when (it goes after a while)
    static STATUS_SEEN: RefCell<(String, std::time::Instant)> = RefCell::new((String::new(), std::time::Instant::now()));
    /// a note that stays while its track plays (a Go+ track's closest version)
    static STATUS_HOLD: RefCell<Option<String>> = const { RefCell::new(None) };
    /// a musical alert playing: the timer that brings the music back after it
    static ALERT_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    #[allow(clippy::type_complexity)]
    static STAT_MODELS: RefCell<Option<(Rc<VecModel<Item>>, Rc<VecModel<Item>>)>> = const { RefCell::new(None) };
}

#[cfg(test)]
mod trim_tests {
    #[test]
    fn start_and_end_times() {
        let p = serde_json::json!({ "trackTrims": { "1": { "start": 10000, "end": 50000 }, "2": { "start": 0, "end": 0 }, "3": { "start": 9000, "end": 5000 } } });
        assert_eq!(super::trim_of(&p, "1"), (10000, Some(50000)));
        assert_eq!(super::trim_of(&p, "2"), (0, None));
        assert_eq!(super::trim_of(&p, "3"), (9000, None));   // an end before the start doesn't count
        assert_eq!(super::trim_of(&p, "x"), (0, None));
    }
}

#[cfg(test)]
mod time_tests {
    #[test]
    fn typed_times() {
        assert_eq!(super::parse_time("1:05"), Ok(Some(65_000)));
        assert_eq!(super::parse_time("0:10.5"), Ok(Some(10_500)));
        assert_eq!(super::parse_time(" "), Ok(None));
        assert!(super::parse_time("1:xx").is_err());
        assert_eq!(super::fmt_time(65_000), "1:05");
        assert_eq!(super::fmt_time(10_500), "0:10.5");
    }
}

#[cfg(test)]
mod link_tests {
    #[test]
    fn invite_pages_as_app_links() {
        assert_eq!(super::app_link("https://relay.test/jam/abcd1234?k=Xy9", "https://d"), "omnipulse://jam/abcd1234?k=Xy9&relay=https%3A%2F%2Frelay.test");
        assert_eq!(super::app_link("omnipulse://jam/abcd1234?k=1", "https://d"), "omnipulse://jam/abcd1234?k=1");
    }
}
