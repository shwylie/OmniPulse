// OmniPulse, native (the rewrite, Phase 3): one window, one process. Pages for Discover, Rediscover,
// the Library, Favourites, Sets & Podcasts, Stats, Search, your playlists and Now Playing (with
// lyrics), played through omnipulse-audio. Built beside the web app, which keeps working until this
// one does everything; the web app's favourites, stats and playlists come from the copy it writes.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod keysetup;
mod slimport;
mod offset;
mod plugrun;
mod collab;
mod desktop;
mod instance;
// (a measuring build: every allocation counted, so the heap in use can be told from the rest)
#[cfg(feature = "memcount")]
mod memcount {
    use std::alloc::{GlobalAlloc, Layout, System};
    use std::sync::atomic::{AtomicIsize, Ordering};
    pub static LIVE: AtomicIsize = AtomicIsize::new(0);
    pub struct Counting;
    unsafe impl GlobalAlloc for Counting {
        unsafe fn alloc(&self, l: Layout) -> *mut u8 { LIVE.fetch_add(l.size() as isize, Ordering::Relaxed); unsafe { System.alloc(l) } }
        unsafe fn dealloc(&self, p: *mut u8, l: Layout) { LIVE.fetch_sub(l.size() as isize, Ordering::Relaxed); unsafe { System.dealloc(p, l) } }
        unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 { LIVE.fetch_add(n as isize - l.size() as isize, Ordering::Relaxed); unsafe { System.realloc(p, l, n) } }
    }
    #[global_allocator]
    static A: Counting = Counting;
}
mod aimusic;
mod loading;
mod viz;
mod gpuviz;
mod streamdeck;
mod socials;
mod smart;
use omnipulse_mix::plan::Plan;
mod updater;
mod discover;
mod lang;
mod panels;
mod flows;
mod botimport;
mod procwatch;
mod docksets;
mod botpage;
mod hide;
mod hud;
mod lookalike;
mod apple_import;
mod alerts;
mod integrations;
mod look;
mod marks;
mod mirror;
mod titles;
mod together;
mod versions;
mod vgroup;
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
    /// the track that place is in: another track started (you picked something else) starts from its beginning
    resume_key: String,
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
    /// the title bar's name (Settings > App > Display Name), from right-clicking it
    AppName,
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
    let t0 = std::time::Instant::now();
    let chip = app.chip;
    let prefs = app.web.prefs.clone();
    let favs: std::collections::HashSet<&str> = app.web.favourites.iter().map(|t| t.key.as_str()).collect();
    let fav_only = app.fav_only;
    // (as the web app's js/pagefind.js: word by word, in the title, artist or album)
    let want = sources::words(&app.page_query);
    let found = |t: &Track| want.is_empty() || { let have = sources::words(&format!("{} {} {} {}", t.title, t.artist, t.album, t.genre)); want.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str()))) };
    // (Search, with Hide likes: what you've liked left out)
    let mine: std::collections::HashSet<&str> = if app.page == "search" && prefs["hideLikes"].as_bool() == Some(true) { app.library.iter().map(|t| t.key.as_str()).collect() } else { Default::default() };
    let shown = |t: &&Track| plat_on(&prefs, t.plat()) && (!fav_only || favs.contains(t.key.as_str())) && !mine.contains(t.key.as_str()) && !hide::no_version(&prefs, t) && streamsafe::allowed(&prefs, t) && found(t);
    let mut filtered: Vec<Track> = match (app.page.as_str(), chip) {
        // (Search, as the web app's: its tracks without the sets under All and Tracks, which have a
        // shelf of their own; only them under Sets & Podcasts; none for the other categories)
        ("search", 0 | 1) => app.page_all.iter().filter(shown).filter(|t| !t.is_mix()).cloned().collect(),
        ("search", 5) => app.page_all.iter().filter(shown).filter(|t| t.is_mix()).cloned().collect(),
        ("search", _) => Vec::new(),
        ("library" | "rediscover" | "favourites" | "playlist" | "artist", 1) => app.page_all.iter().filter(shown).filter(|t| !t.is_mix()).cloned().collect(),
        ("library" | "rediscover" | "favourites" | "playlist", 2) => app.page_all.iter().filter(shown).filter(|t| t.is_mix()).cloned().collect(),
        _ => app.page_all.iter().filter(shown).cloned().collect(),
    };
    if app.page == "library" {
        lib_sort(&prefs, &mut filtered);
    }
    page_timing(&format!("  {} filtered ({} of {})", app.page, filtered.len(), app.page_all.len()), t0);
    // (an artist's page: each song's versions under it, opened beneath it, as the web app's)
    let mut ver_label: std::collections::HashMap<String, String> = Default::default();
    let mut ver_rows: std::collections::HashSet<String> = Default::default();
    if app.page == "artist" {
        let open = VER_OPEN.with(|o| o.borrow().clone());
        let mut out = Vec::new();
        for (head, vs) in vgroup::group(filtered, "") {
            if !vs.is_empty() {
                let n = vs.len();
                ver_label.insert(head.key.clone(), if open.contains(&head.key) { "Hide versions".into() } else { format!("{n} more version{}", if n == 1 { "" } else { "s" }) });
            }
            let shown = open.contains(&head.key);
            out.push(head);
            if shown { for v in vs { ver_rows.insert(v.key.clone()); out.push(v); } }
        }
        filtered = out;
    }
    let playing = app.queue.current().map(|t| t.key.clone()).unwrap_or_default();
    let liked: std::collections::HashSet<&str> = app.library.iter().map(|t| t.key.as_str()).collect();
    let _ = &liked;
    let marks = marks_of(app);
    let albums_here = app.chip == 0 && !app.fav_only && matches!(app.page.as_str(), "library" | "rediscover" | "artist");
    let (alb_key, albs, alb_pics) = PAGE_ALBUMS.with(|p| p.borrow().clone());
    let title_now = app.ui.upgrade().map(|u| u.get_page_title().to_string()).unwrap_or_default();
    let albs = if albums_here && alb_key == page_key(&app.page, &title_now) { albs } else { Vec::new() };
    let t_rows = std::time::Instant::now();
    let mut rows_out: Vec<Row> = filtered.iter().map(|t| Row { versions: ver_label.get(&t.key).cloned().unwrap_or_default().into(), version: ver_rows.contains(&t.key), ..row_menu(app, &marks, t, Row { picked: app.picked.contains(&t.key), ..t.row(t.key == playing) }) }).collect::<Vec<_>>();
    page_timing("  rows made", t_rows);
    // (one before every sixth track)
    // (not among a song's versions opened beneath it: before the next song instead)
    let mut at = 0;
    for (k, f) in albs.iter().enumerate() {
        at = at.max((k + 1) * 6);
        while rows_out.get(at).is_some_and(|r| r.version) { at += 1; }
        let Some(r) = rows_out.get_mut(at) else { break };
        at += 1;
        let (title, by, _, plat) = f.tile();
        r.alb = title.into();
        r.alb_by = by.into();
        r.alb_plat = plat.into();
        r.alb_i = k as i32;
        if let Some(px) = alb_pics.get(&k) { r.alb_art = slint::Image::from_rgba8(px.clone()); }
    }
    let t_set = std::time::Instant::now();
    app.rows.set_vec(rows_out);
    page_timing("  rows given to the list", t_set);
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

/// A page's name in the web app's pageFilters (its views), and the chip it keeps (All, Tracks, Sets & Podcasts)
fn filter_view(page: &str) -> &str { if page == "library" { "likes" } else { page } }
const FILTER_KEYS: [&str; 3] = ["all", "tracks", "mixes"];

fn show_tracks(shared: &Shared, page: &str, title: &str, sub: String, mut tracks: Vec<Track>, chips: bool) {
    let mut app = shared.borrow_mut();
    track::apply_fixes(&app.web.prefs, &mut tracks);
    let Some(ui) = app.ui.upgrade() else { return };
    if page == "search" && app.page != "search" { ui.set_shelves(ModelRc::default()); }
    app.page = page.to_string();
    app.page_all = tracks;
    app.chip = 0;
    app.page_query.clear();   // (a page opened afresh shows all of it)
    if let Some(ui) = app.ui.upgrade() { ui.set_page_find_note("".into()); }
    app.picked.clear();
    app.pick_anchor = None;
    // Search: its first tracks, then what else it found (shelves); Show all lists every track
    let kind = if page == "search" { "search" } else { "tracks" };
    ui.set_page(page.into());
    ui.set_page_kind(kind.into());
    ui.set_page_title(title.into());
    ui.set_page_sub(sub.into());
    album_head_paint(&app, &ui);
    // (the Library has an Albums chip too: your saved albums, as a grid)
    let mut names: Vec<slint::SharedString> = if chips { FILTER_CHIPS.iter().map(|c| (*c).into()).collect() } else { Vec::new() };
    if chips && (page == "library" || page == "rediscover") { names.push("Albums".into()); }
    if chips && page == "artist" { names = vec!["All".into(), "Tracks".into(), "Albums".into()]; }
    if chips && page == "search" { names = ["All", "Tracks", "Artists", "Albums", "Playlists", "Sets & Podcasts"].iter().map(|c| (*c).into()).collect(); }
    ui.set_chips(ModelRc::from(Rc::new(VecModel::from(names))));
    // (the chip last picked on this page)
    let kept = if chips { FILTER_KEYS.iter().position(|k| app.web.prefs["pageFilters"][filter_view(page)].as_str() == Some(k)).unwrap_or(0) } else { 0 };
    let kept = if (page == "artist" && kept > 1) || page == "search" { 0 } else { kept };
    app.chip = kept;
    ui.set_chip_on(kept as i32);
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
    /// any artist, by name (Go to Artist): found on the platforms that are on
    ArtistNamed(String),
}

/// Opens a place, remembering the one you were on (so Back returns to it)
fn go(shared: &Shared, place: Place) {
    // (another page picked from Settings: Settings closed, as its own Close does, and left out of Back's
    // history: Back goes to where you were before it)
    let settings = Place::Page("settings".into());
    let leaving_settings = place != settings && shared.borrow().here.as_ref() == Some(&settings);
    if leaving_settings {
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(ui) = shared.borrow().ui.upgrade() {
            ui.set_pf_open(false);
            ui.set_set_cat("".into());
            ui.set_set_query("".into());
            ui.set_search_text("".into());
        }
        shared.borrow_mut().here = None;
    }
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
    // (where you were, for next time: a page of the sidebar, or a playlist)
    let keep = match &place {
        Place::Page(p) if matches!(p.as_str(), "discover" | "rediscover" | "library" | "artists" | "stats" | "favourites" | "sets") => Some(serde_json::json!({ "page": p })),
        Place::Playlist(id) => Some(serde_json::json!({ "playlist": id })),
        _ => None,
    };
    if let Some(k) = keep { if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("lastPlace", k); } }
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
        Place::ArtistNamed(n) => open_artist_named(shared, n),
    }
    let a = shared.borrow();
    if let Some(ui) = a.ui.upgrade() {
        ui.set_can_back(!a.history.is_empty());
        ui.set_own_page(matches!(place, Place::Playlist(id) if xpl::is_own(id)));
    }
}

/// OMNIPULSE_PAGE_TIMING: how long a page takes to open, and its parts (printed)
fn page_timing(what: &str, since: std::time::Instant) {
    if std::env::var_os("OMNIPULSE_PAGE_TIMING").is_some() { println!("page timing: {what}: {:.1} ms", since.elapsed().as_secs_f64() * 1000.0); }
}

thread_local! {
    /// (OMNIPULSE_PAGE_TIMING: when the page open now was opened, and whether its covers are all in yet)
    static PAGE_T0: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

fn open_page(shared: &Shared, page: &str) {
    let t0 = std::time::Instant::now();
    PAGE_T0.with(|p| p.set(Some(t0)));
    open_page_now(shared, page);
    page_timing(&format!("{page} opened"), t0);
    // (and drawn: the next frame, once the event loop's had it)
    let page = page.to_string();
    slint::Timer::single_shot(std::time::Duration::ZERO, move || page_timing(&format!("{page} to the next turn of the event loop"), t0));
}

fn open_page_now(shared: &Shared, page: &str) {
    let ui = shared.borrow().ui.upgrade().unwrap();
    ui.set_side_on(-1);
    match page {
        "library" => {
            let lib = shared.borrow().library.clone();
            let sub = format!("Everything you've liked on each platform, plus your music files, in one list. {}.", plural(lib.len(), "track", "tracks"));
            show_tracks(shared, "library", "Library", sub, lib, true);
            page_albums_load(shared, "library", "Library");
        }
        "favourites" => {
            let fav = marks::favourites(&shared.borrow().web.favourites);
            let sub = if fav.is_empty() { "Your starred tracks show here. Until this app can star tracks, they come from the web app.".to_string() } else { format!("Starred tracks from every platform. {}.", plural(fav.len(), "track", "tracks")) };
            show_tracks(shared, "favourites", "Favourites", sub, fav, true);
        }
        "sets" => {
            // long ones from your likes, then from the artists you follow
            let mine: Vec<Track> = shared.borrow().library.iter().filter(|t| t.is_mix()).cloned().collect();
            let liked_only = shared.borrow().web.prefs["setsLiked"].as_bool() == Some(true);
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_sets_liked(liked_only); }
            let n = mine.len();
            show_tracks(shared, "sets", "Sets & Podcasts", if liked_only { format!("Long sets, mixes and podcasts in your likes: anything over 15 minutes. {}.", plural(n, "of them", "of them")) } else { "Long sets, mixes and podcasts: anything over 15 minutes.".into() }, mine.clone(), false);
            if liked_only { return; }
            load_into(shared, "sets", "Sets & Podcasts".into(), async move {
                let more: Vec<Track> = sources::following_tracks(300).await.into_iter().filter(|t| t.is_mix() && !mine.iter().any(|m| m.key == t.key)).collect();
                let n = mine.len() + more.len();
                let mut all = mine;
                all.extend(more);
                (all, format!("Long sets, mixes and podcasts: anything over 15 minutes. {}.", plural(n, "of them", "of them")))
            });
        }
        "rediscover" => {
            let (mut pool, h) = { let a = shared.borrow(); (a.library.clone(), heard(&a)) };
            let cut = now_ms().saturating_sub(MONTH_MS);
            pool.retain(|t| h.last(t) < cut);
            { let prefs = shared.borrow().web.prefs.clone(); pool.retain(|t| hide::rec_ok(&prefs, t) && plat_on(&prefs, t.plat())); }
            queue::shuffle_in_place(&mut pool);
            pool.sort_by_key(|t| h.last(t));   // never played first
            // (an even split: one platform, then the next, as the web app's alternatePlatforms)
            let pool = queue::alternate_platforms(pool);
            let none = { let a = shared.borrow(); a.web.favourites.is_empty() && !a.library.iter().any(|t| t.plat() != "local") };
            // (nothing liked yet: asked for some, as the web app's card)
            let sub = if none { "For better recommendations, add some music to your likes or favourites with the heart or the star on any song. Discover and Rediscover will be built around it.".to_string() } else if pool.is_empty() { "You've played all your likes recently.".to_string() } else { "Likes you haven't heard in a month or more, the longest-forgotten first.".to_string() };
            show_tracks(shared, "rediscover", "Rediscover", sub, pool, true);
            page_albums_load(shared, "rediscover", "Rediscover");
        }
        // (while Stream-Safe Music is on, the Streamer Mode page stands in for Discover)
        "discover" if streamsafe::on(&shared.borrow().web.prefs) => open_streamer(shared),
        "streamer" => open_streamer(shared),
        "discover" => open_discover(shared),
        "artists" => open_artists(shared),
        "stats" => { let p = stats_period(&shared.borrow().web.prefs); open_stats(shared, p) }
        "settings" => {
            let mut a = shared.borrow_mut();
            // (from another page: its categories first, as Windows' Settings opens)
            if a.page != "settings" { ui.set_set_cat("".into()); }
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
    let mut add = |id: &str, name: &str, ready: bool| list.push(Plat { id: id.into(), on: ready && plat_on(&a.web.prefs, id), name: name.into(), ready, d: Default::default() });
    add("soundcloud", "SoundCloud", omnipulse_core::soundcloud::signed_in());
    add("spotify", "Spotify", omnipulse_core::spotify::spotify_signed_in());
    add("youtube", "YouTube Music", omnipulse_core::tools::ytdlp_path().is_some());
    // (and the plugins asked to be here, each its own icon: on and off as it's switched in Settings > Plugins)
    ui.set_suite_shown(false);
    for p in omnipulse_core::plugins::list() {
        let m = &p.manifest;
        if plugrun::settings_of(m, &a.web.prefs)[PLUG_TITLE_BAR].as_bool() != Some(true) { continue; }
        let d = if m.icon.trim().is_empty() { "M12 2a10 10 0 1 0 0 20 10 10 0 1 0 0-20z".to_string() } else { m.icon.clone() };
        // (the Stream Suite: its own button, beside Streamer Mode, lit while it runs)
        // (switched off in Settings > Plugins: not shown at all)
        if m.id == botpage::BOT {
            let on = plugin_on(&a.web.prefs, &m.id);
            ui.set_suite_shown(on);
            ui.set_suite_icon(d.into());
            ui.set_suite_on(on && plugin_runs(&a.web.prefs, &m.id));
            continue;
        }
        list.push(Plat { id: format!("plugin:{}", m.id).into(), on: plugin_on(&a.web.prefs, &m.id), name: m.name.clone().into(), ready: true, d: d.into() });
    }
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
        // (the ones you hid, with Hidden & Not Recommended on)
        .filter(|&i| !hide::on(&prefs) || !prefs["hiddenByYou"].as_array().is_some_and(|l| l.iter().any(|x| x.as_str() == Some(a.playlists[i].id.as_str()))))
        // (and the playlists, while Stream-Safe Music is on)
        .filter(|_| !streamsafe::on(&prefs))
        .collect();
    // (in your order: pinned ones first, as pinned; then as you've arranged them; new ones after, as they come)
    let ids = |k: &str| -> Vec<String> { prefs[k].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect() };
    let (pinned, order) = (ids("sidePinned"), ids("sideOrder"));
    let pl = &a.playlists;
    let sid = |i: usize| mirror::side_id(pl[i].plat, &pl[i].id);
    let mut map = a.side_map.clone();
    map.sort_by_key(|&i| { let id = sid(i); (pinned.iter().position(|p| *p == id).unwrap_or(usize::MAX), order.iter().position(|o| *o == id).unwrap_or(usize::MAX)) });
    let pins: std::collections::HashSet<String> = pinned.into_iter().collect();
    a.side_map = map;
    let sides: Vec<Side> = a.side_map.iter().map(|&i| Side { title: a.playlists[i].title.clone().into(), plat: if xpl::is_own(&a.playlists[i].id) { "xpl".into() } else { a.playlists[i].plat.into() }, own: xpl::is_own(&a.playlists[i].id), share: a.playlists[i].plat != "local" || xpl::is_own(&a.playlists[i].id), sc: share::platform_id(&a.playlists[i], &a.xpl, "soundcloud").is_some(), sp: share::platform_id(&a.playlists[i], &a.xpl, "spotify").is_some(), collab: collab_of(&a.xpl, &a.playlists[i].id).is_some(), collab_owner: collab_of(&a.xpl, &a.playlists[i].id).is_some_and(|c| c["owner"].as_bool() == Some(true)), merged: a.xpl.as_array().is_some_and(|l| l.iter().any(|x| x["id"].as_str() == Some(a.playlists[i].id.as_str()) && x["merged"].as_array().is_some_and(|m| !m.is_empty()))), pinned: pins.contains(&mirror::side_id(a.playlists[i].plat, &a.playlists[i].id)) }).collect();
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

/// A plugin's own setting, kept with its others: its icon in the title bar's platforms
const PLUG_TITLE_BAR: &str = "_titleBar";

fn plat_toggle(shared: &Shared, plat: &str) {
    // (the Suite's button: started or stopped, as its own switch in Streaming)
    if plat == "suite" {
        set_toggle(shared, &format!("plugRun_{}", botpage::BOT));
        return paint_plats(&shared.borrow());
    }
    // (a plugin's: switched on or off, as in Settings > Plugins)
    if let Some(id) = plat.strip_prefix("plugin:") {
        set_toggle(shared, &format!("plugOn_{id}"));
        paint_plats(&shared.borrow());
        return;
    }
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
    // (its header: as its sidebar menu has it)
    {
        let a = shared.borrow();
        let own = xpl::is_own(&p.id);
        let collab = collab_of(&a.xpl, &p.id);
        ui.set_pl_share(p.plat != "local" || own);
        ui.set_pl_sc(share::platform_id(&p, &a.xpl, "soundcloud").is_some());
        ui.set_pl_sp(share::platform_id(&p, &a.xpl, "spotify").is_some());
        ui.set_pl_own(own);
        ui.set_pl_merged(a.xpl.as_array().is_some_and(|l| l.iter().any(|x| x["id"].as_str() == Some(p.id.as_str()) && x["merged"].as_array().is_some_and(|m| !m.is_empty()))));
        ui.set_pl_collab(match &collab { Some(c) if c["owner"].as_bool() == Some(true) => "Copy invite link".into(), Some(_) => "".into(), None if p.plat != "local" || own => "Invite collaborators".into(), None => "".into() });
    }
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
    let local: Vec<Track> = {
        let a = shared.borrow();
        if !plat_on(&a.web.prefs, "local") { Vec::new() } else {
            let want = requests::words(&q);
            a.library.iter().filter(|t| t.plat() == "local").filter(|t| { let have = requests::words(&format!("{} {} {}", t.title, t.artist, t.album)); !want.is_empty() && want.iter().all(|w| have.contains(w)) }).take(50).cloned().collect()
        }
    };
    let token = { let mut a = shared.borrow_mut(); a.token += 1; a.token };
    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    rt.spawn(async move {
        let q2 = q.clone();
        let late = tokio::spawn(async move { sources::search_plugins(&q2).await });
        let mut t = match sources::genre_of(&q) {
            // (a genre: its tracks found by genre, then the rest, as the web app's)
            Some(g) => { let (a, b) = tokio::join!(sources::search_genre(&g), sources::search(&q)); sources::once_each(a.into_iter().chain(b).collect()) }
            None => sources::search(&q).await,
        };
        // (your music files too, with that platform on, as the web app's: ranked with the rest)
        if !local.is_empty() { t = sources::rank(&q, sources::interleave(vec![t, local])); }
        let title = format!("Results for \"{q}\"");
        let sub = if t.is_empty() { "Nothing found.".to_string() } else { plural(t.len(), "track", "tracks") + ", best match first." };
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if shared.borrow().token != token { return; }
            show_tracks(&shared, "search", &title, sub, t, true);
            search_paint(&shared);
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

thread_local! {
    /// a search's other finds, for its categories: its genre (if it's one), artists, albums, playlists
    static SEARCH: RefCell<(Option<String>, Vec<sources::Found>, Vec<sources::Found>, Vec<sources::Found>)> = const { RefCell::new((None, Vec::new(), Vec::new(), Vec::new())) };
    /// what each shelf shown on the Search page is (for a tile's click)
    static SEARCH_MAP: RefCell<Vec<SearchShelf>> = const { RefCell::new(Vec::new()) };
    /// the pictures fetched for the Search page's tiles, by address
    static SEARCH_ART: RefCell<HashMap<String, slint::SharedPixelBuffer<slint::Rgba8Pixel>>> = RefCell::new(HashMap::new());
}

#[derive(Clone)]
enum SearchShelf { Genre(String), Found(Vec<sources::Found>), Sets(Vec<Track>) }

/// The Search page's shelves, for the category chosen (as the web app's paintSearchShelves): under All,
/// its genre's (if it's one), then Artists, Albums, Playlists and Sets & Podcasts, a row each; a
/// category on its own opened up
fn search_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if shared.borrow().page != "search" { return; }
    let chip = shared.borrow().chip;
    let (genre, artists, albums, lists) = SEARCH.with(|s| s.borrow().clone());
    let sets: Vec<Track> = shared.borrow().page_all.iter().filter(|t| t.is_mix()).take(24).cloned().collect();
    let cat = catalog_on(&shared.borrow().web.prefs);
    let mut shelves: Vec<(String, SearchShelf, bool)> = Vec::new();
    let all = chip == 0;
    if all { if let Some(g) = &genre { if cat { shelves.push((sources::title_case(g), SearchShelf::Genre(g.clone()), false)); } } }
    let artists_title = match &genre { Some(g) => format!("Top {} artists", sources::title_case(g)), None => "Artists".to_string() };
    if (all || chip == 2) && !artists.is_empty() { shelves.push((artists_title, SearchShelf::Found(artists), chip == 2)); }
    if (all || chip == 3) && !albums.is_empty() { shelves.push(("Albums".into(), SearchShelf::Found(albums), chip == 3)); }
    if (all || chip == 4) && !lists.is_empty() { shelves.push(("Playlists".into(), SearchShelf::Found(lists), chip == 4)); }
    if all && !sets.is_empty() { shelves.push(("Sets & Podcasts".into(), SearchShelf::Sets(sets), false)); }
    let mut wanted: Vec<(usize, usize, String)> = Vec::new();
    let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().enumerate().map(|(s, (_, sh, _))| {
        let tiles: Vec<Tile> = match sh {
            SearchShelf::Genre(g) => vec![Tile { title: format!("Trending {}", sources::title_case(g)).into(), sub: "Trending now".into(), genre: true, c1: look::hsl(g.chars().fold(7u32, |h, c| (h * 31 + c as u32) % 360) as f32, 0.7, 45.0).slint(), c2: look::hsl(((g.chars().fold(7u32, |h, c| (h * 31 + c as u32) % 360) + 40) % 360) as f32, 0.6, 16.0).slint(), ..Default::default() }],
            SearchShelf::Found(l) => l.iter().map(|f| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), plat: plat.into(), ..Default::default() } }).collect(),
            SearchShelf::Sets(l) => { let a = shared.borrow(); let m = marks_of(&a); l.iter().map(|t| Tile { title: t.title.clone().into(), sub: t.artist.clone().into(), plat: t.plat().into(), track: true, row: row_menu(&a, &m, t, t.row(false)), ..Default::default() }).collect() }
        };
        let urls: Vec<String> = match sh { SearchShelf::Genre(_) => Vec::new(), SearchShelf::Found(l) => l.iter().map(|f| f.tile().2).collect(), SearchShelf::Sets(l) => l.iter().map(thumb_url).collect() };
        let tiles: Vec<Tile> = tiles.into_iter().enumerate().map(|(k, mut t)| {
            if let Some(url) = urls.get(k) { match SEARCH_ART.with(|c| c.borrow().get(url).cloned()) { Some(px) => t.art = slint::Image::from_rgba8(px), None if !url.is_empty() => wanted.push((s, k, url.clone())), None => {} } }
            t
        }).collect();
        Rc::new(VecModel::from(tiles))
    }).collect();
    let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|((title, _, open), m)| Shelf { title: title.clone().into(), tiles: ModelRc::from(m.clone()), open: *open, gone: false, shown: 0 }).collect();
    ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
    SHELF_MODELS.with(|m| *m.borrow_mut() = models);
    SEARCH_MAP.with(|m| *m.borrow_mut() = shelves.into_iter().map(|(_, sh, _)| sh).collect());
    // (their pictures, as they come; kept for the other categories)
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let gen = SEARCH_GEN.with(|g| { g.set(g.get() + 1); g.get() });
    rt.spawn(async move {
        for (s, k, url) in wanted {
            if let Some(px) = player::fetch_art(url.clone(), art_px(170.0)).await {
                let _ = weak.upgrade_in_event_loop(move |_| {
                    SEARCH_ART.with(|c| { let mut c = c.borrow_mut(); if c.len() >= 240 { c.clear(); } c.insert(url, px.clone()); });
                    if SEARCH_GEN.with(|g| g.get()) != gen { return; }
                    SHELF_MODELS.with(|m| { if let Some(model) = m.borrow().get(s) { if let Some(mut t) = model.row_data(k) { t.art = slint::Image::from_rgba8(px); model.set_row_data(k, t); } } });
                });
            }
        }
    });
}

thread_local! {
    static SEARCH_GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// A tile on the Search page: an artist's page, an album or playlist, a set, or its genre's mix
fn search_tile(shared: &Shared, sh: usize, k: usize) {
    let Some(shelf) = SEARCH_MAP.with(|m| m.borrow().get(sh).cloned()) else { return };
    match shelf {
        SearchShelf::Genre(g) => open_mix(shared, discover::Mix { title: format!("Trending {}", sources::title_case(&g)), sub: "Trending".into(), art: String::new(), plat: "deezer", desc: format!("What's trending in {}, from what Deezer's playlists of it agree on.", sources::title_case(&g)), how: discover::How::Genre(sources::title_case(&g)) }),
        // (an artist: their page, as the web app's)
        SearchShelf::Found(l) => match l.get(k) {
            Some(sources::Found::Artist { name, .. }) => go(shared, Place::ArtistNamed(name.clone())),
            Some(f) => { FOUND.with(|x| *x.borrow_mut() = vec![vec![f.clone()]]); open_found(shared, 0, 0); }
            None => {}
        },
        SearchShelf::Sets(l) => play_list(shared, l, k),
    }
}

/// Search's shelves: artists, albums and playlists from every platform, with their pictures as they come
fn search_shelves(shared: &Shared, q: String) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    FOUND.with(|f| f.borrow_mut().clear());
    SEARCH.with(|s| *s.borrow_mut() = (sources::genre_of(&q), Vec::new(), Vec::new(), Vec::new()));
    SEARCH_ART.with(|c| c.borrow_mut().clear());
    search_paint(shared);
    rt.spawn(async move {
        let genre = sources::genre_of(&q);
        let ((artists, albums, lists), top) = tokio::join!(sources::search_found(&q), async { match &genre { Some(g) => sources::genre_artists(g).await, None => Vec::new() } });
        // (a genre: its best-known artists, as the web app's "Top <genre> artists")
        let artists = if genre.is_some() && !top.is_empty() { top } else { artists };
        let q2 = q.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if shared.borrow().page != "search" || !ui.get_page_title().contains(q2.as_str()) { return; }
            SEARCH.with(|s| *s.borrow_mut() = (genre, artists, albums, lists));
            search_paint(&shared);
        }));
    });
}

thread_local! {
    /// a page's albums, among its tracks (one every six, as the web app's injectAlbums): which page
    /// (its kind and title), the albums, and their covers as they come
    static PAGE_ALBUMS: RefCell<(String, Vec<sources::Found>, HashMap<usize, slint::SharedPixelBuffer<slint::Rgba8Pixel>>)> = RefCell::new((String::new(), Vec::new(), HashMap::new()));
}

fn page_key(page: &str, title: &str) -> String { format!("{page}\u{1}{title}") }

/// A page's albums found (the Library's saved ones, a few for Rediscover, an artist's): shown among its
/// tracks, their covers fetched
fn page_albums_set(shared: &Shared, key: String, list: Vec<sources::Found>) {
    let arts: Vec<String> = list.iter().map(|f| f.tile().2).collect();
    PAGE_ALBUMS.with(|p| *p.borrow_mut() = (key.clone(), list, HashMap::new()));
    apply_filter(&mut shared.borrow_mut());
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        for (k, url) in arts.into_iter().enumerate().take(40) {
            if url.is_empty() { continue; }
            if let Some(px) = player::fetch_art(url, 120).await {
                let key = key.clone();
                let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
                    let Some(sh) = s.borrow().clone() else { return };
                    let same = PAGE_ALBUMS.with(|p| { let mut p = p.borrow_mut(); if p.0 != key { return false; } p.2.insert(k, px.clone()); true });
                    if !same { return; }
                    let a = sh.borrow();
                    for i in 0..a.rows.row_count() {
                        if let Some(mut r) = a.rows.row_data(i) { if r.alb_i == k as i32 && r.alb != "" { r.alb_art = slint::Image::from_rgba8(px.clone()); a.rows.set_row_data(i, r); } }
                    }
                }));
            }
        }
    });
}

/// The albums for the Library and Rediscover (Rediscover: six of them, mixed), once the page shows
fn page_albums_load(shared: &Shared, page: &str, title: &str) {
    let key = page_key(page, title);
    if PAGE_ALBUMS.with(|p| p.borrow().0 == key) { return; }
    PAGE_ALBUMS.with(|p| *p.borrow_mut() = (key.clone(), Vec::new(), HashMap::new()));
    let (rt, weak, files) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.library.iter().filter(|t| t.plat() == "local").cloned().collect::<Vec<_>>()) };
    let few = page == "rediscover";
    rt.spawn(async move {
        let mut list = sources::saved_albums().await;
        list.extend(sources::local_albums(&files));
        if few { for i in (1..list.len()).rev() { let j = (rand_u64() as usize) % (i + 1); list.swap(i, j); } list.truncate(6); }
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { if PAGE_ALBUMS.with(|p| p.borrow().0 == key) { page_albums_set(&sh, key, list); } } }));
    });
}

/// A page's albums as the Albums chip's grid (Rediscover's)
fn page_albums_grid(shared: &Shared) {
    let list = PAGE_ALBUMS.with(|p| p.borrow().1.clone());
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    ui.set_page_kind("albumgrid".into());
    let pics = PAGE_ALBUMS.with(|p| p.borrow().2.clone());
    let model = Rc::new(VecModel::from(list.iter().enumerate().map(|(k, f)| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), art: pics.get(&k).map(|px| slint::Image::from_rgba8(px.clone())).unwrap_or_default(), plat: plat.into(), ..Default::default() } }).collect::<Vec<_>>()));
    ui.set_album_tiles(ModelRc::from(model.clone()));
    if list.is_empty() { ui.set_status("No albums here.".into()); }
    ALBUM_MODEL.with(|m| *m.borrow_mut() = Some(model));
    ALBUMS.with(|a| *a.borrow_mut() = list);
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
            let model = Rc::new(VecModel::from(list.iter().map(|f| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into(), genre: false, c1: Default::default(), c2: Default::default(), track: false, liked: false, row: Default::default(), miss: false } }).collect::<Vec<_>>()));
            ui.set_album_tiles(ModelRc::from(model.clone()));
            ui.set_page_sub(if n == 0 { "No saved albums yet. Save some on Spotify, like some on SoundCloud, or add music files that have album names.".to_string() } else { format!("{}: saved on Spotify, liked on SoundCloud, and your files'.", plural(n, "album", "albums")) }.into());
            ALBUM_MODEL.with(|m| *m.borrow_mut() = Some(model));
            ALBUMS.with(|a| *a.borrow_mut() = list);
        });
        for (k, url) in arts.into_iter().enumerate() {
            if url.is_empty() { continue; }
            if let Some(px) = player::fetch_art(url, art_px(170.0)).await {
                let _ = weak2.upgrade_in_event_loop(move |_| ALBUM_MODEL.with(|m| {
                    if let Some(model) = m.borrow().as_ref() { if let Some(mut t) = model.row_data(k) { t.art = slint::Image::from_rgba8(px); model.set_row_data(k, t); } }
                }));
            }
        }
    });
}

/// The album a page is (its header's): its title, who by, where from and its cover's address
#[derive(Clone)]
struct AlbumHead { title: String, by: String, plat: &'static str, img: String }

thread_local! {
    static PAGE_ALBUM: RefCell<Option<AlbumHead>> = const { RefCell::new(None) };
    /// (its cover as fetched, and for which address)
    static ALBUM_HEAD_ART: RefCell<String> = const { RefCell::new(String::new()) };
}

/// An album being opened as a page (or something else: None): its header
fn album_head_for(f: Option<&sources::Found>) {
    let h = match f {
        Some(sources::Found::Album { plat, title, by, img, .. }) => Some(AlbumHead { title: title.clone(), by: by.clone(), plat, img: img.clone() }),
        _ => None,
    };
    PAGE_ALBUM.with(|p| *p.borrow_mut() = h);
}

/// An album's page, its header: shown (with its cover, what's on it, liked or not) while the page is that
/// album's, else hidden
fn album_head_paint(a: &App, ui: &AppWindow) {
    let page = ui.get_page();
    let title = ui.get_page_title();
    let head = PAGE_ALBUM.with(|p| p.borrow().clone()).filter(|h| (page == "album" || page == "found") && h.title == title.as_str());
    let Some(h) = head else {
        if ui.get_album_head() { ui.set_album_head(false); }
        return;
    };
    ui.set_album_head(true);
    ui.set_album_head_by(h.by.clone().into());
    ui.set_album_head_plat(h.plat.into());
    let list = &a.page_all;
    let info = if list.is_empty() {
        ui.get_page_sub().to_string()
    } else {
        let year = list.iter().find_map(|t| t.date.get(..4).filter(|y| y.chars().all(|c| c.is_ascii_digit()))).map(|y| format!("{y} \u{b7} ")).unwrap_or_default();
        let mins = (list.iter().map(|t| t.ms).sum::<u64>() as f64 / 60_000.0).round() as u64;
        format!("{year}{}{}", plural(list.len(), "song", "songs"), if mins > 0 { format!(" \u{b7} {} min", mins) } else { String::new() })
    };
    ui.set_album_head_info(info.into());
    let likeable: Vec<&Track> = list.iter().filter(|t| !matches!(t.src, track::Src::Local(_))).collect();
    ui.set_album_head_can_like(!likeable.is_empty());
    ui.set_album_head_liked(!likeable.is_empty() && likeable.iter().all(|t| a.library.iter().any(|x| x.key == t.key)));
    // its cover: the album's own, else its first song's
    let url = if h.img.is_empty() { list.first().map(|t| t.art.clone()).unwrap_or_default() } else { h.img.clone() };
    if ALBUM_HEAD_ART.with(|x| *x.borrow() == url) { return; }
    ALBUM_HEAD_ART.with(|x| *x.borrow_mut() = url.clone());
    ui.set_album_head_art(slint::Image::default());
    if url.is_empty() { return; }
    let weak = a.ui.clone();
    a.rt.spawn(async move {
        let Some(px) = player::fetch_art(url.clone(), art_px(168.0)).await else { return };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if ALBUM_HEAD_ART.with(|x| *x.borrow() == url) { ui.set_album_head_art(slint::Image::from_rgba8(px)); }
        });
    });
}

/// Like Album: every song on it liked (or, all of them liked already, the likes taken off)
fn album_like(shared: &Shared) {
    let (todo, on, title) = {
        let a = shared.borrow();
        let likeable: Vec<Track> = a.page_all.iter().filter(|t| !matches!(t.src, track::Src::Local(_))).cloned().collect();
        let liked = |t: &Track| a.library.iter().any(|x| x.key == t.key);
        let all = !likeable.is_empty() && likeable.iter().all(liked);
        // (liking: only those not liked yet; unliking: all of them)
        let todo: Vec<Track> = likeable.iter().filter(|t| liked(t) == all).cloned().collect();
        let title = a.ui.upgrade().map(|u| u.get_page_title().to_string()).unwrap_or_default();
        (todo, !all, title)
    };
    let n = todo.len();
    for t in todo { like_track(shared, t); }
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_status(format!("{} {} on \"{title}\".", if on { "Liked" } else { "Took the like off" }, plural(n, "song", "songs")).into());
    }
}

/// An album on the Albums grid: its tracks, as a page
fn album_open(shared: &Shared, i: usize) {
    let Some(f) = ALBUMS.with(|a| a.borrow().get(i).cloned()) else { return };
    album_head_for(Some(&f));
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
    album_head_for(Some(&f));
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
    let everywhere = shared.borrow().web.prefs["followEverywhere"].as_bool() != Some(false) && std::env::var_os("OMNIPULSE_SELFTEST").is_none();
    // (the YouTube Music ones, kept on this PC, with YouTube Music on)
    let yt_follows = if plat_on(&shared.borrow().web.prefs, "youtube") { marks::yt_follows(&shared.borrow().web.yt_follows) } else { Vec::new() };
    rt.spawn(async move {
        let mut list = sources::followed_artists(yt_follows).await;
        // (Follow Everywhere: new follows on one platform, on the other too)
        let also = if everywhere { sources::follow_new_everywhere(&mut list).await } else { Vec::new() };
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if !also.is_empty() { ui.set_status(format!("Followed {}{} on your other platform too.", also.iter().take(3).cloned().collect::<Vec<_>>().join(", "), if also.len() > 3 { " and others" } else { "" }).into()); }
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
        on_sc: a.sc.is_some(),
        on_sp: a.sp.is_some(),
        on_yt: a.yt.is_some(),
        row: Default::default(),
    }).collect();
    // (following everyone from one platform on the other too, as the web app's header menu)
    let (sc_on, sp_on) = { let p = &shared.borrow().web.prefs; (plat_on(p, "soundcloud"), plat_on(p, "spotify")) };
    let (to_sp, to_sc) = (list.iter().filter(|a| a.sc.is_some() && a.sp.is_none()).count(), list.iter().filter(|a| a.sp.is_some() && a.sc.is_none()).count());
    let mut batch: Vec<slint::SharedString> = Vec::new();
    let (mut batch_to, mut batch_tip): (Vec<slint::SharedString>, Vec<slint::SharedString>) = (Vec::new(), Vec::new());
    if sc_on && sp_on && to_sp > 0 { batch.push(format!("Follow {to_sp}").into()); batch_to.push("spotify".into()); batch_tip.push(format!("Follow the {} you follow on SoundCloud on Spotify too", plural(to_sp, "artist", "artists")).into()); }
    if sc_on && sp_on && to_sc > 0 { batch.push(format!("Follow {to_sc}").into()); batch_to.push("soundcloud".into()); batch_tip.push(format!("Follow the {} you follow on Spotify on SoundCloud too", plural(to_sc, "artist", "artists")).into()); }
    ui.set_artists_batch_to(ModelRc::from(Rc::new(VecModel::from(batch_to))));
    ui.set_artists_batch_tip(ModelRc::from(Rc::new(VecModel::from(batch_tip))));
    ARTISTS_BATCH.with(|b| *b.borrow_mut() = [(sc_on && sp_on && to_sp > 0).then_some("spotify"), (sc_on && sp_on && to_sc > 0).then_some("soundcloud")].into_iter().flatten().collect());
    ui.set_artists_batch(ModelRc::from(Rc::new(VecModel::from(batch))));
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

// ---------- following, on an artist's page ----------
thread_local! {
    /// the artist whose page is open (in the Artists list), and whether Unfollow was pressed once
    /// (pressed again, it unfollows); the platforms they've been unfollowed on, so Follow can undo it
    static FOLLOW: RefCell<(Option<usize>, bool, Vec<(&'static str, String)>)> = const { RefCell::new((None, false, Vec::new())) };
}


thread_local! {
    /// an artist's page opened by name, not followed: where they were found (to follow them there)
    static NAMED: RefCell<Option<sources::Artist>> = const { RefCell::new(None) };
}

fn follow_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let (at, asked, off) = FOLLOW.with(|f| f.borrow().clone());
    ui.set_follow_sc("".into());
    ui.set_follow_sp("".into());
    ui.set_follow_yt("".into());
    let yt_ok = plat_on(&shared.borrow().web.prefs, "youtube") && omnipulse_core::tools::ytdlp_path().is_some();
    let Some(a) = at.and_then(|i| shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned())) else {
        // (by name: Follow, where they were found)
        let found = NAMED.with(|n| n.borrow().clone()).filter(|n| !n.plats().is_empty());
        ui.set_follow_label(if found.is_some() { "Follow".into() } else { "".into() });
        ui.set_follow_on(false);
        if let Some(n) = &found {
            if n.sc.is_some() { ui.set_follow_sc("Follow on SoundCloud".into()); }
            if n.sp.is_some() { ui.set_follow_sp("Follow on Spotify".into()); }
            if n.yt.is_some() { ui.set_follow_yt("Follow on YouTube Music".into()); }
        }
        ui.set_follow_more("".into());
        return;
    };
    ui.set_follow_sc(if a.sc.is_some() { "Unfollow on SoundCloud".into() } else if omnipulse_core::soundcloud::signed_in() { "Follow on SoundCloud".into() } else { "".into() });
    ui.set_follow_sp(if a.sp.is_some() { "Unfollow on Spotify".into() } else if omnipulse_core::spotify::spotify_signed_in() { "Follow on Spotify".into() } else { "".into() });
    ui.set_follow_yt(if a.yt.is_some() { "Unfollow on YouTube Music".into() } else if yt_ok { "Follow on YouTube Music".into() } else { "".into() });
    let names = |l: Vec<&str>| l.into_iter().map(plat_name).collect::<Vec<_>>().join(" and ");
    if a.plats().is_empty() {
        ui.set_follow_label(if off.is_empty() { "".into() } else { "Follow".into() });
        ui.set_follow_on(false);
    } else {
        ui.set_follow_label(if asked { format!("Unfollow on {}?", names(a.plats())) } else { "Unfollow".into() }.into());
        ui.set_follow_on(true);
    }
    // (followed on one, signed in to the other: Also follow there)
    // (the platform: shown as its symbol, "Also follow")
    let more = if a.sc.is_none() && !a.plats().is_empty() && omnipulse_core::soundcloud::signed_in() { "soundcloud" }
        else if a.sp.is_none() && !a.plats().is_empty() && omnipulse_core::spotify::spotify_signed_in() { "spotify" }
        else if a.yt.is_none() && !a.plats().is_empty() && yt_ok { "youtube" } else { "" };
    ui.set_follow_more(more.into());
}

/// The artist's entry in the Artists list changed
fn artist_set(shared: &Shared, i: usize, f: impl FnOnce(&mut sources::Artist)) {
    let change = {
        let mut x = shared.borrow_mut();
        x.artists.as_mut().and_then(|l| l.get_mut(i)).map(|a| { let was = a.yt.clone(); f(a); (was, a.yt.clone(), a.name.clone(), a.img.clone()) })
    };
    // (YouTube Music's follows are this PC's: kept, or let go, as they change)
    if let Some((was, now, name, img)) = change {
        if was != now {
            let web = shared.borrow().web.yt_follows.clone();
            if let Some(id) = &was { marks::set_yt_follow(&web, id, &name, &img, false); }
            if let Some(id) = &now { marks::set_yt_follow(&web, id, &name, &img, true); }
        }
    }
    follow_paint(shared);
}

fn plat_of(plat: &str) -> &'static str { match plat { "spotify" => "spotify", "youtube" => "youtube", _ => "soundcloud" } }

/// Follow on an artist's page opened by name: on the platforms they were found on (only `only`, if given),
/// then they're in your Artists
fn follow_named(shared: &Shared, only: Option<&'static str>) {
    let Some(n) = NAMED.with(|x| x.borrow().clone()) else { return };
    let plats: Vec<(&'static str, String)> = [n.sc.map(|x| ("soundcloud", x.to_string())), n.sp.clone().map(|x| ("spotify", x)), n.yt.clone().map(|x| ("youtube", x))].into_iter().flatten().filter(|(p, _)| only.is_none_or(|o| o == *p)).collect();
    let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
    rt.spawn(async move {
        let mut done: Vec<(&'static str, String)> = Vec::new();
        for (p, id) in &plats { if sources::set_follow(p, id, true).await.is_ok() { done.push((*p, id.clone())); } }
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            if done.is_empty() { ui.set_status(format!("Couldn't follow {}.", n.name).into()); return; }
            let a = sources::Artist { name: n.name.clone(), img: n.img.clone(), sc: done.iter().find(|d| d.0 == "soundcloud").and_then(|d| d.1.parse().ok()), sp: done.iter().find(|d| d.0 == "spotify").map(|d| d.1.clone()), yt: done.iter().find(|d| d.0 == "youtube").map(|d| d.1.clone()) };
            if let Some(id) = &a.yt { let web = sh.borrow().web.yt_follows.clone(); marks::set_yt_follow(&web, id, &a.name, &a.img, true); }
            let i = { let mut x = sh.borrow_mut(); let l = x.artists.get_or_insert_with(Vec::new); l.push(a); l.len() - 1 };
            NAMED.with(|x| x.borrow_mut().take());
            FOLLOW.with(|f| *f.borrow_mut() = (Some(i), false, Vec::new()));
            follow_paint(&sh);
            ui.set_status(format!("Following {} on {}.", n.name, done.iter().map(|d| plat_name(d.0)).collect::<Vec<_>>().join(" and ")).into());
        }));
    });
}

/// The follow button's right-click: one platform, followed or unfollowed
fn follow_plat(shared: &Shared, plat: &str) {
    let plat: &'static str = plat_of(plat);
    let at = FOLLOW.with(|f| f.borrow().0);
    let Some(i) = at else { return follow_named(shared, Some(plat)) };
    let Some(a) = shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned()) else { return };
    let have = a.id_on(plat);
    match have {
        // (followed there: unfollowed there only)
        Some(id) => {
            let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
            rt.spawn(async move {
                let r = sources::set_follow(plat, &id, false).await;
                let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                    let Some(sh) = s.borrow().clone() else { return };
                    match r {
                        Ok(()) => { artist_set(&sh, i, |x| x.set_on(plat, None)); ui.set_status(format!("Unfollowed {} on {}.", a.name, plat_name(plat)).into()); }
                        Err(e) => ui.set_status(format!("Couldn't unfollow on {}: {e}", plat_name(plat)).into()),
                    }
                }));
            });
        }
        None => artist_act(shared, i, &format!("follow:{plat}")),
    }
}

fn follow_click(shared: &Shared) {
    let (at, asked, off) = FOLLOW.with(|f| f.borrow().clone());
    let Some(i) = at else { return follow_named(shared, None) };
    let Some(a) = shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned()) else { return };
    let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
    if a.plats().is_empty() {
        // followed again where they were unfollowed
        rt.spawn(async move {
            let mut back = Vec::new();
            for (p, id) in &off { if sources::set_follow(p, id, true).await.is_ok() { back.push((*p, id.clone())); } }
            let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                let Some(sh) = s.borrow().clone() else { return };
                FOLLOW.with(|f| f.borrow_mut().2.clear());
                let name = a.name.clone();
                artist_set(&sh, i, |x| for (p, id) in &back { x.set_on(p, Some(id.clone())); });
                ui.set_status(format!("Following {name} again.").into());
            }));
        });
        return;
    }
    // Unfollow: asked once, done on the second press
    if !asked {
        FOLLOW.with(|f| f.borrow_mut().1 = true);
        follow_paint(shared);
        return;
    }
    let plats: Vec<(&'static str, String)> = [a.sc.map(|n| ("soundcloud", n.to_string())), a.sp.clone().map(|id| ("spotify", id)), a.yt.clone().map(|id| ("youtube", id))].into_iter().flatten().collect();
    rt.spawn(async move {
        let mut done = Vec::new();
        let mut failed = Vec::new();
        for (p, id) in &plats {
            match sources::set_follow(p, id, false).await { Ok(()) => done.push((*p, id.clone())), Err(e) => failed.push(format!("{}: {e}", plat_name(p))) }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            FOLLOW.with(|f| { let mut f = f.borrow_mut(); f.1 = false; f.2 = done.clone(); });
            let name = a.name.clone();
            artist_set(&sh, i, |x| for (p, _) in &done { x.set_on(p, None); });
            ui.set_status(if failed.is_empty() { format!("Unfollowed {name}.") } else { format!("Couldn't unfollow everywhere. {}", failed.join(" ")) }.into());
        }));
    });
}

thread_local! {
    /// the Artists page's batch lines: the platform each follows everyone on
    static ARTISTS_BATCH: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// An artist's menu on the Artists page: Follow on SoundCloud / Spotify Too, Unfollow Everywhere
fn artist_act(shared: &Shared, i: usize, act: &str) {
    let Some(a) = shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned()) else { return };
    let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
    if let Some(plat) = act.strip_prefix("follow:") {
        let plat: &'static str = plat_of(plat);
        if let Some(ui) = weak.upgrade() { ui.set_status(format!("Looking for {} on {}...", a.name, plat_name(plat)).into()); }
        rt.spawn(async move {
            let r = match sources::find_artist_on(plat, &a.name).await {
                None => Err(format!("{} isn't on {} (by that name).", a.name, plat_name(plat))),
                Some(id) => sources::set_follow(plat, &id, true).await.map(|_| id).map_err(|e| format!("Couldn't follow on {}: {e}", plat_name(plat))),
            };
            let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                let Some(sh) = s.borrow().clone() else { return };
                match r {
                    Ok(id) => { artist_set(&sh, i, |x| x.set_on(plat, Some(id.clone()))); ui.set_status(format!("Following {} on {}.", a.name, plat_name(plat)).into()); }
                    Err(e) => ui.set_status(e.into()),
                }
            }));
        });
        return;
    }
    // Unfollow Everywhere: asked first (as the web app's)
    let on: Vec<&str> = a.plats().into_iter().map(plat_name).collect();
    let text = format!("Unfollow {} on {}?", a.name, on.join(" and "));
    ask(shared, "Unfollow", &text, "Unfollow", move |_| {
        let plats: Vec<(&'static str, String)> = [a.sc.map(|n| ("soundcloud", n.to_string())), a.sp.clone().map(|id| ("spotify", id)), a.yt.clone().map(|id| ("youtube", id))].into_iter().flatten().collect();
        rt.spawn(async move {
            let mut done = Vec::new();
            for (p, id) in &plats { if sources::set_follow(p, id, false).await.is_ok() { done.push(*p); } }
            let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                let Some(sh) = s.borrow().clone() else { return };
                artist_set(&sh, i, |x| for p in &done { x.set_on(p, None); });
                ui.set_status(format!("Unfollowed {}.", a.name).into());
            }));
        });
    });
}

/// Following everyone you follow on one platform on the other too (the web app's batchFollow)
fn artists_batch(shared: &Shared, k: usize) {
    let Some(to) = ARTISTS_BATCH.with(|b| b.borrow().get(k).copied()) else { return };
    let Some(list) = shared.borrow().artists.clone() else { return };
    let todo: Vec<(usize, sources::Artist)> = list.into_iter().enumerate().filter(|(_, a)| if to == "spotify" { a.sc.is_some() && a.sp.is_none() } else { a.sp.is_some() && a.sc.is_none() }).collect();
    let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
    let n = todo.len();
    rt.spawn(async move {
        let (mut done, mut missing) = (0usize, 0usize);
        for (k, (i, a)) in todo.into_iter().enumerate() {
            let w = weak.clone();
            let _ = w.upgrade_in_event_loop(move |ui| ui.set_status(format!("Following on {}: {} of {n}...", plat_name(to), k + 1).into()));
            match sources::find_artist_on(to, &a.name).await {
                Some(id) if sources::set_follow(to, &id, true).await.is_ok() => {
                    done += 1;
                    let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { artist_set(&sh, i, |x| x.set_on(to, Some(id.clone()))); } }));
                }
                _ => missing += 1,
            }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            ui.set_status(format!("Following {} more on {}.{}", plural(done, "artist", "artists"), plat_name(to), if missing > 0 { format!(" {} couldn't be found there.", plural(missing, "artist", "artists")) } else { String::new() }).into());
            SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { if let Some(l) = sh.borrow().artists.clone() { paint_artists(&sh, &l); } } });
        });
    });
}

fn follow_more_click(shared: &Shared) {
    let at = FOLLOW.with(|f| f.borrow().0);
    let Some(i) = at else { return };
    let Some(a) = shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned()) else { return };
    // (the platform the Also Follow button shows)
    let plat = match shared.borrow().ui.upgrade().map(|u| u.get_follow_more().to_string()) { Some(p) if !p.is_empty() => plat_of(&p), _ => if a.sc.is_none() { "soundcloud" } else { "spotify" } };
    let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
    if let Some(ui) = weak.upgrade() { ui.set_status(format!("Looking for {} on {}...", a.name, plat_name(plat)).into()); }
    rt.spawn(async move {
        let r = match sources::find_artist_on(plat, &a.name).await {
            None => Err(format!("{} isn't on {} (by that name).", a.name, plat_name(plat))),
            Some(id) => sources::set_follow(plat, &id, true).await.map(|_| id).map_err(|e| format!("Couldn't follow on {}: {e}", plat_name(plat))),
        };
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            match r {
                Ok(id) => {
                    artist_set(&sh, i, |x| x.set_on(plat, Some(id.clone())));
                    ui.set_status(format!("Following {} on {} too.", a.name, plat_name(plat)).into());
                }
                Err(e) => ui.set_status(e.into()),
            }
        }));
    });
}

fn open_artist(shared: &Shared, i: usize) {
    let Some(a) = shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned()) else { return };
    FOLLOW.with(|f| *f.borrow_mut() = (Some(i), false, Vec::new()));
    follow_paint(shared);
    let name = |p: &str| match p { "spotify" => "Spotify", _ => "SoundCloud" };
    let on = a.plats().into_iter().map(name).collect::<Vec<_>>().join(" and ");
    show_tracks(shared, "artist", &a.name, "Loading...".into(), Vec::new(), true);
    let cat = catalog_on(&shared.borrow().web.prefs);
    let token = shared.borrow().token;
    artist_extras(shared, &a.name, a.sc, a.sp.clone(), token);
    load_into(shared, "artist", a.name.clone(), async move {
        let (t, dz) = tokio::join!(sources::artist_tracks(&a), artist_top(&a.name, cat));
        let t = sources::once_each(sources::interleave(vec![t, dz]));
        let sub = format!("You follow them on {on}. {}.", plural(t.len(), "track", "tracks"));
        (t, sub)
    });
}

/// An artist's page from their name (the web app's openArtistNamed): what you like by them first, then
/// what they've put out on SoundCloud and Spotify (found by name) and their best known on Deezer
fn open_artist_named(shared: &Shared, name: &str) {
    // (one you follow: their own page)
    let followed = shared.borrow().artists.as_ref().and_then(|l| l.iter().position(|a| omnipulse_core::ranking::artist_key(&a.name) == omnipulse_core::ranking::artist_key(name)));
    if let Some(i) = followed { return open_artist(shared, i); }
    FOLLOW.with(|f| *f.borrow_mut() = (None, false, Vec::new()));
    NAMED.with(|n| n.borrow_mut().take());
    follow_paint(shared);
    let key = omnipulse_core::ranking::artist_key(name);
    let mine: Vec<Track> = shared.borrow().library.iter().filter(|t| t.artist.split([',', '&']).any(|a| omnipulse_core::ranking::artist_key(a.trim()) == key)).cloned().collect();
    show_tracks(shared, "artist", name, "Loading...".into(), mine.clone(), true);
    let token = shared.borrow().token;
    artist_extras(shared, name, None, None, token);
    let (cat, sc_on, sp_on, yt_on) = { let p = &shared.borrow().web.prefs; (catalog_on(p), plat_on(p, "soundcloud"), plat_on(p, "spotify"), plat_on(p, "youtube") && omnipulse_core::tools::ytdlp_path().is_some()) };
    let name = name.to_string();
    load_into(shared, "artist", name.clone(), async move {
        let (sc, sp) = tokio::join!(
            async { if sc_on { sources::find_artist_on("soundcloud", &name).await.and_then(|x| x.parse().ok()) } else { None } },
            async { if sp_on { sources::find_artist_on("spotify", &name).await } else { None } },
        );
        let yt = if yt_on { sources::find_artist_on("youtube", &name).await } else { None };
        let a = sources::Artist { name: name.clone(), img: String::new(), sc, sp, yt };
        {
            let found = a.clone();
            let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { NAMED.with(|n| *n.borrow_mut() = Some(found)); if let Some(sh) = s.borrow().clone() { follow_paint(&sh); } }));
        }
        let on = a.plats().into_iter().map(|p| if p == "spotify" { "Spotify" } else { "SoundCloud" }).collect::<Vec<_>>().join(" and ");
        let (t, dz) = tokio::join!(sources::artist_tracks(&a), artist_top(&name, cat));
        let t = sources::once_each(sources::interleave(vec![mine.clone(), t, dz]));
        let liked = if mine.is_empty() { String::new() } else { format!("{} you like first. ", plural(mine.len(), "song", "songs")) };
        let sub = format!("{liked}{}{}.", if on.is_empty() { String::new() } else { format!("From {on} and more. ") }, plural(t.len(), "track", "tracks"));
        (t, sub)
    });
}

thread_local! {
    /// the artist page open: their name and where they are (for its Albums chip)
    static ARTIST_NOW: RefCell<Option<(String, sources::Profiles)>> = const { RefCell::new(None) };
}

/// An artist's page, beyond its tracks (as the web app's): where they are, with how many follow them
/// there (each platform's symbol), their picture, their Wikipedia bio, and their songs on YouTube Music
fn artist_extras(shared: &Shared, name: &str, sc: Option<u64>, sp: Option<String>, token: u64) {
    let (rt, weak, prefs, files) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.web.prefs.clone(), a.library.iter().filter(|t| t.plat() == "local").cloned().collect::<Vec<_>>()) };
    if let Some(ui) = weak.upgrade() { ui.set_artist_on(ModelRc::default()); ui.set_artist_bio("".into()); ui.set_artist_bio_url("".into()); ui.set_page_art(slint::Image::default()); ui.set_page_art_on(false); }
    ARTIST_NOW.with(|n| *n.borrow_mut() = Some((name.to_string(), sources::Profiles::default())));
    let k = omnipulse_core::ranking::artist_key(name);
    let n_files = files.iter().filter(|t| t.artist.split([',', '&']).any(|a| omnipulse_core::ranking::artist_key(a.trim()) == k)).count();
    let (yt_on, dz_on, sc_on, sp_on) = (plat_on(&prefs, "youtube") && omnipulse_core::tools::ytdlp_path().is_some(), catalog_on(&prefs), plat_on(&prefs, "soundcloud"), plat_on(&prefs, "spotify"));
    let name = name.to_string();
    rt.spawn(async move {
        let (p, bio) = tokio::join!(sources::profiles(&name, if sc_on { sc } else { None }, if sp_on { sp } else { None }, yt_on, dz_on), omnipulse_core::bio::artist(&name));
        let p = sources::Profiles { sc: if sc_on { p.sc } else { None }, sp: if sp_on { p.sp } else { None }, ..p };
        if std::env::var_os("OMNIPULSE_SELFTEST").is_some() { println!("selftest: profiles {p:?}"); }
        let yt_songs: Vec<Track> = match &p.yt { Some((id, _)) => omnipulse_core::youtube::artist(id).await.map(|pg| pg.songs.iter().map(Track::youtube).collect()).unwrap_or_default(), None => Vec::new() };
        let art = if p.img.is_empty() { None } else { player::fetch_round(p.img.clone(), 200).await };
        let bio = bio.unwrap_or_default();
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if shared.borrow().token != token && shared.borrow().page != "artist" { return; }
            if ui.get_page() != "artist" || ui.get_page_title() != name.as_str() { return; }
            let count = |n: u64| { let s = n.to_string(); let mut o = String::new(); for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { o.push(','); } o.push(c); } o };
            let mut on: Vec<ArtistOn> = Vec::new();
            if let Some((_, f)) = p.sc { on.push(ArtistOn { plat: "soundcloud".into(), text: format!("{} followers", count(f)).into() }); }
            if let Some((_, f)) = &p.sp { on.push(ArtistOn { plat: "spotify".into(), text: if *f > 0 { format!("{} followers", count(*f)) } else { "Spotify".into() }.into() }); }
            if let Some((_, subs)) = &p.yt { on.push(ArtistOn { plat: "youtube".into(), text: if subs.is_empty() { "YouTube Music".into() } else { subs.clone().into() } }); }
            if let Some((_, f)) = p.dz { on.push(ArtistOn { plat: "deezer".into(), text: if f > 0 { format!("{} fans", count(f)) } else { "Deezer".into() }.into() }); }
            if n_files > 0 { on.push(ArtistOn { plat: "local".into(), text: plural(n_files, "file", "files").into() }); }
            ui.set_artist_on(ModelRc::from(Rc::new(VecModel::from(on))));
            if let Some(px) = art { ui.set_page_art(slint::Image::from_rgba8(px)); ui.set_page_art_on(true); }
            ui.set_artist_bio(bio["text"].as_str().unwrap_or("").trim().into());
            ui.set_artist_bio_url(bio["url"].as_str().unwrap_or("").into());
            ARTIST_NOW.with(|n| *n.borrow_mut() = Some((name.clone(), p.clone())));
            {
                let (rt, files) = { let a = shared.borrow(); (a.rt.clone(), a.library.iter().filter(|t| t.plat() == "local").cloned().collect::<Vec<_>>()) };
                let (n2, p2, key) = (name.clone(), p.clone(), page_key("artist", &name));
                PAGE_ALBUMS.with(|x| *x.borrow_mut() = (key.clone(), Vec::new(), HashMap::new()));
                let weak = ui.as_weak();
                rt.spawn(async move {
                    let list = sources::artist_albums(&n2, &p2, &files).await;
                    let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { if PAGE_ALBUMS.with(|x| x.borrow().0 == key) { page_albums_set(&sh, key, list); } } }));
                });
            }
            // (their YouTube Music songs join the page, each song once)
            if !yt_songs.is_empty() {
                let mut a = shared.borrow_mut();
                let mut all = std::mem::take(&mut a.page_all);
                all.extend(yt_songs);
                a.page_all = sources::once_each(all);
                apply_filter(&mut a);
            }
        }));
    });
}

/// The artist page's Albums chip: their albums and singles from every platform, as tiles
fn artist_albums(shared: &Shared) {
    let Some((name, p)) = ARTIST_NOW.with(|n| n.borrow().clone()) else { return };
    let (rt, weak, files) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.library.iter().filter(|t| t.plat() == "local").cloned().collect::<Vec<_>>()) };
    if let Some(ui) = weak.upgrade() { ui.set_page_kind("albumgrid".into()); ui.set_album_tiles(ModelRc::default()); }
    rt.spawn(async move {
        let list = sources::artist_albums(&name, &p, &files).await;
        let arts: Vec<String> = list.iter().map(|f| f.tile().2).collect();
        let weak2 = weak.clone();
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if ui.get_page_kind() != "albumgrid" || ui.get_page() != "artist" { return; }
            let model = Rc::new(VecModel::from(list.iter().map(|f| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into(), genre: false, c1: Default::default(), c2: Default::default(), track: false, liked: false, row: Default::default(), miss: false } }).collect::<Vec<_>>()));
            ui.set_album_tiles(ModelRc::from(model.clone()));
            if list.is_empty() { ui.set_status(format!("No albums by {name} turned up.").into()); }
            ALBUM_MODEL.with(|m| *m.borrow_mut() = Some(model));
            ALBUMS.with(|a| *a.borrow_mut() = list);
        });
        for (k, url) in arts.into_iter().enumerate() {
            if url.is_empty() { continue; }
            if let Some(px) = player::fetch_art(url, art_px(170.0)).await {
                let _ = weak2.upgrade_in_event_loop(move |_| ALBUM_MODEL.with(|m| {
                    if let Some(model) = m.borrow().as_ref() { if let Some(mut t) = model.row_data(k) { t.art = slint::Image::from_rgba8(px); model.set_row_data(k, t); } }
                }));
            }
        }
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

thread_local! {
    /// Discover's feed as last made (for its platforms and language), with its covers: shown again when
    /// you come back, until it's refreshed (as the web app's feedCache)
    static FEED_CACHE: RefCell<Option<(String, Vec<discover::Shelf>, Vec<Rc<VecModel<Tile>>>)>> = const { RefCell::new(None) };
}

fn feed_key(prefs: &serde_json::Value) -> String {
    format!("{}|{}|{}|{}|{}", plat_on(prefs, "soundcloud"), plat_on(prefs, "spotify"), plat_on(prefs, "youtube"), catalog_on(prefs), discover::lang_of(prefs))
}

/// What's typed in the bar, over the page's shelves (as the web app's pageFilterApply): each tile kept
/// if every word starts a word of its name or the line under it. (shown, of)
fn page_find_shelves(shared: &Shared, q: &str) -> (usize, usize) {
    let want = sources::words(q);
    let Some(ui) = shared.borrow().ui.upgrade() else { return (0, 0) };
    let shelves = ui.get_shelves();
    let (mut shown, mut of) = (0, 0);
    SHELF_MODELS.with(|m| {
        for (s, model) in m.borrow().iter().enumerate() {
            let mut here = 0;
            for k in 0..model.row_count() {
                let Some(mut t) = model.row_data(k) else { continue };
                let have = sources::words(&format!("{} {}", t.title, t.sub));
                let ok = want.is_empty() || want.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str())));
                if ok { here += 1; }
                if t.miss == ok { t.miss = !ok; model.set_row_data(k, t); }
            }
            shown += here;
            of += model.row_count();
            if let Some(mut sh) = shelves.row_data(s) {
                let (gone, n) = (here == 0 && !want.is_empty(), if want.is_empty() { 0 } else { here as i32 });
                if sh.gone != gone || sh.shown != n { sh.gone = gone; sh.shown = n; shelves.set_row_data(s, sh); }
            }
        }
    });
    (shown, of)
}

/// Discover: the feed made last time, if it's for the same platforms and language; else made afresh
fn open_discover(shared: &Shared) {
    let key = feed_key(&shared.borrow().web.prefs);
    let hit = FEED_CACHE.with(|c| c.borrow().as_ref().filter(|x| x.0 == key).map(|x| (x.1.clone(), x.2.clone())));
    let Some((shelves, models)) = hit else { return open_discover_fresh(shared) };
    let ui = shared.borrow().ui.upgrade().unwrap();
    {
        let mut a = shared.borrow_mut();
        a.page = "discover".into();
        a.token += 1;
        a.shelves = shelves.iter().map(|sh| sh.items.iter().filter_map(|i| if let discover::Item::Track(t) = i { Some(t.clone()) } else { None }).collect()).collect();
    }
    ui.set_page("discover".into());
    ui.set_page_kind("discover".into());
    ui.set_page_title("Discover".into());
    ui.set_page_sub("Mixes and stations made from what you typically like to listen to.".into());
    ui.set_chips(ModelRc::default());
    let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|(sh, m)| Shelf { title: sh.title.clone().into(), tiles: ModelRc::from(m.clone()), open: false, gone: false, shown: 0 }).collect();
    ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
    ui.set_status("".into());
    DISCOVER.with(|d| *d.borrow_mut() = shelves);
    SHELF_MODELS.with(|m| *m.borrow_mut() = models);
}

/// Discover made afresh (its refresh button, a new language or genres)
fn open_discover_fresh(shared: &Shared) {
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
        ui.set_taste_label("Get Better Recommendations".into());
        let lang = discover::lang_of(&a.web.prefs);
        ui.set_lang_label(lang::CHOICES.iter().find(|(v, _)| *v == lang).map(|(_, n)| *n).unwrap_or("Any language").into());
        ui.set_lang_at(lang::CHOICES.iter().position(|(v, _)| *v == lang).unwrap_or(0) as i32);
    }
    let (input, rt, weak, token) = {
        let a = shared.borrow();
        let (catalog, _, soundcloud) = discover_flags(&a.web.prefs);
        let plays = a.web.plays.iter().map(|p| (p.artist.clone(), p.art.clone(), p.genre.clone())).collect();
        (discover::Input { library: a.library.clone(), favourites: a.web.favourites.clone(), plays, prefs: a.web.prefs.clone(), catalog, soundcloud }, a.rt.clone(), a.ui.clone(), a.token)
    };
    {
        let mut steps: Vec<(&'static str, &'static str)> = Vec::new();
        if omnipulse_core::spotify::spotify_signed_in() { steps.push(("spotify", "Spotify")); }
        if input.soundcloud { steps.push(("soundcloud", "SoundCloud")); }
        if input.catalog { steps.push(("deezer", "Deezer")); }
        loading::begin(&["discover"], &steps);
    }
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
                    // (a genre or subgenre: a coloured cover with its name, as the web app's: its colour from its name)
                    let genre = matches!(i, discover::Item::Mix(m) if matches!(m.how, discover::How::Sub(_) | discover::How::Genre(_)));
                    let hue = title.chars().fold(7u32, |h, c| (h * 31 + c as u32) % 360) as f32;
                    let (c1, c2) = if genre { (look::hsl(hue, 0.7, 45.0).slint(), look::hsl((hue + 40.0) % 360.0, 0.6, 16.0).slint()) } else { Default::default() };
                    let (is_track, liked) = match i { discover::Item::Track(t) => (true, shared.borrow().library.iter().any(|x| x.key == t.key)), _ => (false, false) };
                    // (a song's: its menu, the same as anywhere else)
                    let row = match i { discover::Item::Track(t) => { let a = shared.borrow(); let m = marks_of(&a); row_menu(&a, &m, t, t.row(false)) }, _ => Row::default() };
                    Tile { title: title.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into(), genre, c1, c2, track: is_track, liked, row, miss: false }
                }).collect::<Vec<_>>()))).collect();
                let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|(sh, m)| Shelf { title: sh.title.clone().into(), tiles: ModelRc::from(m.clone()), open: false, gone: false, shown: 0 }).collect();
                ui.set_status(if rows.is_empty() { "Play or star a few songs, and Discover will fill up with music for you.".into() } else { "".into() });
                ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
                loading::end(&["discover"]);
                shared.borrow_mut().shelves = shelves.iter().map(|sh| sh.items.iter().filter_map(|i| if let discover::Item::Track(t) = i { Some(t.clone()) } else { None }).collect()).collect();
                let key = feed_key(&shared.borrow().web.prefs);
                FEED_CACHE.with(|c| *c.borrow_mut() = Some((key, shelves.clone(), models.clone())));
                DISCOVER.with(|d| *d.borrow_mut() = shelves);
                SHELF_MODELS.with(|m| *m.borrow_mut() = models);
            });
        });
        // the covers, as they come
        for (s, list) in arts.into_iter().enumerate() {
            for (k, url) in list.into_iter().enumerate() {
                if url.is_empty() { continue; }
                if let Some(px) = player::fetch_art(url, art_px(170.0)).await {
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
        ui.set_taste_label("Get Better Recommendations".into());
        ui.set_status(if g.is_empty() { "No genres picked.".into() } else { format!("Thanks. Discover starts from {} now.", if sg.is_empty() { g.join(", ") } else { sg.join(", ") }).into() });
    }
    open_discover_fresh(shared);
}

thread_local! {
    /// the Discover mix whose page is open (for its refresh button)
    static OPEN_MIX: RefCell<Option<discover::Mix>> = const { RefCell::new(None) };
}

/// A mix's page refreshed: made again (for a subgenre's or a genre's, new music for its title)
fn mix_refresh(shared: &Shared) {
    let Some(m) = OPEN_MIX.with(|o| o.borrow().clone()) else { return };
    let (liked, flags) = { let a = shared.borrow(); (track::Liked::of(&a.library), discover_flags(&a.web.prefs)) };
    let avoid: std::collections::HashSet<String> = shared.borrow().page_all.iter().map(|t| t.key.clone()).collect();
    let title = shared.borrow().ui.upgrade().map(|ui| ui.get_page_title().to_string()).unwrap_or_else(|| m.title.clone());
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("New music for {}...", m.title).into()); }
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let token = { let mut a = shared.borrow_mut(); a.token += 1; a.token };
    rt.spawn(async move {
        let list = discover::mix_again(&m, flags, &liked, &avoid).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if shared.borrow().token != token { return; }
            if list.is_empty() { ui.set_status("Nothing new came back for this one: try again in a moment.".into()); return; }
            ui.set_status("".into());
            show_tracks(&shared, "mix", &title, m.desc.clone(), list, false);
        }));
    });
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
        discover::Item::Mix(m) => open_mix(shared, m),
    }
}

/// A mix (Discover's, or a genre's from Search): put together, then shown as a page
fn open_mix(shared: &Shared, m: discover::Mix) {
    {
        {
            let (liked, flags) = { let a = shared.borrow(); (track::Liked::of(&a.library), discover_flags(&a.web.prefs)) };
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Putting {} together...", m.title).into()); }
            let title = if matches!(m.how, discover::How::Crew(_, true)) { format!("{} station", m.title) } else { m.title.clone() };
            let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
            let token = { let mut a = shared.borrow_mut(); a.token += 1; a.token };
            OPEN_MIX.with(|o| *o.borrow_mut() = Some(m.clone()));
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
    // (a Spotify song too, while Seamless Mix plays it from YouTube Music: its version picked here is the one it mixes)
    let via_mix = matches!(t.src, track::Src::Spotify(_)) && smart_on(&shared.borrow().web.prefs);
    let shown = t.go_plus || matches!(t.src, track::Src::Catalog(_)) || via_mix;
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_tp_versions(ModelRc::default()); ui.set_tp_version_on(-1);
        ui.set_tp_versions_shown(shown);
        ui.set_tp_versions_note(if via_mix { "With Seamless Mix on, this song plays from YouTube Music. Pick the right version, or search for it below." } else { "" }.into());
    }
    if !shown { return; }
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

thread_local! {
    /// Track preferences' cover: None as it is, Some("") its own again, Some(address) one picked (only its address
    /// is ever kept: the picture's fetched from there, as any cover is)
    static TP_ART: RefCell<Option<String>> = const { RefCell::new(None) };
    /// the covers found for it (Change Cover), by their addresses, as shown
    static TP_COVERS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// Track preferences' cover: the one it has now, shown small, and nothing picked yet
fn tp_cover_open(shared: &Shared, t: &Track) {
    TP_ART.with(|a| *a.borrow_mut() = None);
    TP_COVERS.with(|c| c.borrow_mut().clear());
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let picked = shared.borrow().web.prefs["trackFixes"][&t.key]["art"].as_str().is_some_and(|a| !a.is_empty());
    ui.set_tp_cover_changed(picked);
    ui.set_tp_cover_note(if picked { "A cover you picked." } else { "Its own cover." }.into());
    ui.set_tp_covers(ModelRc::default());
    ui.set_tp_covers_open(false);
    ui.set_tp_cover(slint::Image::default());
    tp_cover_show(shared, t.art.clone(), t.key.clone());
}

/// The cover at `url`, shown small in Track preferences (while it's still open on `key`)
fn tp_cover_show(shared: &Shared, url: String, key: String) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        let px = player::fetch_art(url, 112).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if TP_TRACK.with(|x| x.borrow().as_ref().map(|x| x.key.clone())) != Some(key) { return; }
            match px {
                Some(px) => ui.set_tp_cover(slint::Image::from_rgba8(px)),
                None => ui.set_tp_cover_note("That address isn't a picture OmniPulse can open.".into()),
            }
        });
    });
}

/// Change Cover: the covers its releases have on Deezer and iTunes, looked for now, shown to pick from
fn tp_cover_find(shared: &Shared) {
    let Some(t) = TP_TRACK.with(|x| x.borrow().clone()) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let (artist, title) = (ui.get_tp_artist().to_string(), ui.get_tp_title().to_string());
    ui.set_tp_covers_open(true);
    ui.set_tp_cover_note("Looking for covers...".into());
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        let found = omnipulse_core::catalog::covers(&artist, &title).await;
        let mut shown = Vec::new();
        for (url, _) in found { if let Some(px) = player::fetch_art(url.clone(), 96).await { shown.push((url, px)); } }
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if TP_TRACK.with(|x| x.borrow().as_ref().map(|x| x.key.clone())) != Some(t.key.clone()) { return; }
            ui.set_tp_cover_note(if shown.is_empty() { "No other covers found. You can paste an image address below." } else { "Click one to use it, or paste an image address below." }.into());
            TP_COVERS.with(|c| *c.borrow_mut() = shown.iter().map(|(u, _)| u.clone()).collect());
            let imgs: Vec<slint::Image> = shown.into_iter().map(|(_, px)| slint::Image::from_rgba8(px)).collect();
            ui.set_tp_covers(ModelRc::from(Rc::new(VecModel::from(imgs))));
        });
    });
}

/// A cover picked from those found (its place), or its own again (-1); kept when Save is clicked
fn tp_cover_pick(shared: &Shared, i: i32) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if i < 0 {
        TP_ART.with(|a| *a.borrow_mut() = Some(String::new()));
        ui.set_tp_cover_changed(false);
        ui.set_tp_cover_note("Its own cover comes back when you click Save.".into());
        let own = TP_TRACK.with(|x| x.borrow().clone()).map(|t| (shared.borrow().web.prefs["trackFixes"][&t.key]["artOwn"].as_str().unwrap_or(&t.art).to_string(), t.key));
        if let Some((url, key)) = own { tp_cover_show(shared, url, key); }
        return;
    }
    let Some(url) = TP_COVERS.with(|c| c.borrow().get(i as usize).cloned()) else { return };
    if let Some(img) = ui.get_tp_covers().row_data(i as usize) { ui.set_tp_cover(img); }
    TP_ART.with(|a| *a.borrow_mut() = Some(url));
    ui.set_tp_cover_changed(true);
    ui.set_tp_cover_note("It's used once you click Save.".into());
}

/// An image's address pasted in, for its cover
fn tp_cover_url(shared: &Shared, url: &str) {
    let url = url.trim().to_string();
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if !(url.starts_with("https://") || url.starts_with("http://")) { ui.set_tp_cover_note("Paste an image's address, starting with https://.".into()); return; }
    let Some(t) = TP_TRACK.with(|x| x.borrow().clone()) else { return };
    TP_ART.with(|a| *a.borrow_mut() = Some(url.clone()));
    ui.set_tp_cover_changed(true);
    ui.set_tp_cover_note("It's used once you click Save.".into());
    tp_cover_show(shared, url, t.key);
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

/// A title that's really a file name ("thescript-forthefirsttime(8barzremix).mp3"): a guess at its
/// title and artist (as the web app's guessFromFilename); anything else as it is
fn guess_from_filename(t: &Track) -> (String, String) {
    static EXT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"(?i)\.(mp3|wav|flac|m4a|aac|ogg|aiff?)$").unwrap());
    static SPLIT: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new(r"^([^-]+?)\s*-\s*(.+)$").unwrap());
    let looks = EXT.is_match(&t.title) || (!t.title.contains(char::is_whitespace) && t.title.contains(['-', '_']));
    if !looks { return (t.title.clone(), t.artist.clone()); }
    let s = EXT.replace(&t.title, "").replace('_', " ").trim().to_string();
    match SPLIT.captures(&s) {
        Some(m) => (m[2].trim().to_string(), m[1].trim().to_string()),
        None => (s, t.artist.clone()),
    }
}

fn tp_open(shared: &Shared, t: &Track) {
    let (start, end) = trim_of(&shared.borrow().web.prefs, &t.key);
    TP_TRACK.with(|x| *x.borrow_mut() = Some(t.clone()));
    if let Some(ui) = shared.borrow().ui.upgrade() {
        // (a title that's really a file name, not fixed yet: a guess at it filled in, as the web app's)
        let fixed = shared.borrow().web.prefs["trackFixes"].get(&t.key).is_some();
        let (title, artist) = if fixed { (t.title.clone(), t.artist.clone()) } else { guess_from_filename(t) };
        ui.set_tp_title(title.into());
        ui.set_tp_artist(artist.into());
        ui.set_tp_start(if start > 0 { fmt_time(start) } else { String::new() }.into());
        ui.set_tp_end(end.map(fmt_time).unwrap_or_default().into());
        ui.set_tp_note(format!("It's {} long.", fmt_ms(t.ms)).into());
        ui.set_tp_beat(beat_shift(&shared.borrow().web.prefs, &t.key));
        ui.set_tp_phrase(phrase_shift(&shared.borrow().web.prefs, &t.key));
        ui.set_tp_beat_shown(smart_on(&shared.borrow().web.prefs));
        ui.set_tp_open(true);
    }
    tp_bar(shared);
    tp_versions(shared, t);
    tp_cover_open(shared, t);
}

/// The track's length, for Track preferences' bar: as playing, else as listed
fn tp_len(shared: &Shared) -> u64 {
    let Some(t) = TP_TRACK.with(|x| x.borrow().clone()) else { return 0 };
    let a = shared.borrow();
    if a.queue.current().is_some_and(|c| c.key == t.key) && a.engine.duration_ms() > 0 { a.engine.duration_ms() } else { t.ms }
}

/// The bar painted from the start and end as written
fn tp_bar(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let len = tp_len(shared).max(1) as f32;
    let start = parse_time(&ui.get_tp_start()).ok().flatten().unwrap_or(0) as f32;
    let end = parse_time(&ui.get_tp_end()).ok().flatten().map_or(len, |e| e as f32);
    ui.set_tp_from((start / len).clamp(0.0, 1.0));
    ui.set_tp_to((end / len).clamp(0.0, 1.0));
}

/// The bar dragged: the start (0) or the end (1) where it's let go, a second apart at least; the end at
/// the track's own end (within a quarter second) is none
fn tp_drag(shared: &Shared, frac: f32, which: i32) {
    // (a time typed: the bar again)
    if which < 0 { return tp_bar(shared); }
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let len = tp_len(shared);
    if len == 0 { return; }
    let at = (frac as f64 * len as f64) as u64;
    let start = parse_time(&ui.get_tp_start()).ok().flatten().unwrap_or(0);
    let end = parse_time(&ui.get_tp_end()).ok().flatten().unwrap_or(len);
    if which == 0 {
        let v = at.min(end.saturating_sub(1000));
        ui.set_tp_start(if v == 0 { String::new() } else { fmt_time(v) }.into());
    } else {
        let v = at.max(start + 1000);
        ui.set_tp_end(if v + 250 >= len { String::new() } else { fmt_time(v) }.into());
    }
    tp_bar(shared);
}

thread_local! {
    /// a preview playing (Track preferences): the track, and where it stops
    static TP_PREVIEW: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
}

/// How long a preview of a start or an end plays
const PREVIEW_MS: u64 = 8000;

/// A preview of the start (0) or the end (1) as written: the track played from there, for a few seconds
fn tp_preview(shared: &Shared, which: i32) {
    let Some(t) = TP_TRACK.with(|x| x.borrow().clone()) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let (Ok(start), Ok(end)) = (parse_time(&ui.get_tp_start()), parse_time(&ui.get_tp_end())) else { ui.set_tp_note("Times are m:ss, like 1:05 (or empty).".into()); return };
    let playing = shared.borrow().queue.current().is_some_and(|c| c.key == t.key) && shared.borrow().loaded;
    if !playing {
        // (not playing yet: it's played now, then previewed once it's going)
        { let mut a = shared.borrow_mut(); a.queue.play_next(t.clone()); }
        step(shared, 1);
        let s2 = shared.clone();
        let tries = std::rc::Rc::new(std::cell::Cell::new(0));
        let timer = slint::Timer::default();
        timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(300), move || {
            tries.set(tries.get() + 1);
            let ready = s2.borrow().loaded && s2.borrow().queue.current().is_some_and(|c| c.key == t.key) && s2.borrow().engine.duration_ms() > 0;
            if ready || tries.get() > 50 {
                TP_PREVIEW_WAIT.with(|w| w.borrow_mut().take());
                if ready { tp_preview(&s2, which); }
            }
        });
        TP_PREVIEW_WAIT.with(|w| *w.borrow_mut() = Some(timer));
        ui.set_status("Getting it ready to preview...".into());
        return;
    }
    let len = tp_len(shared);
    let end = end.unwrap_or(len);
    let from = if which == 0 { start.unwrap_or(0) } else { end.saturating_sub(PREVIEW_MS).max(start.unwrap_or(0)) };
    let until = if which == 0 { (from + PREVIEW_MS).min(end) } else { end };
    TP_PREVIEW.with(|p| *p.borrow_mut() = Some((t.key.clone(), until)));
    let engine = shared.borrow().engine.clone();
    engine.seek(from);
    if engine.is_paused() { toggle(shared); }
    ui.set_status(format!("Previewing {}...", if which == 0 { "the start" } else { "the end" }).into());
}

thread_local! {
    static TP_PREVIEW_WAIT: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
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
    // (its beat 1, moved by hand: kept apart, by track)
    // (its beat 1 and its phrases' start, moved by hand: kept apart, by track)
    for (pref, n) in [("beatShifts", if reset { 0 } else { ui.get_tp_beat() }), ("phraseShifts", if reset { 0 } else { ui.get_tp_phrase() })] {
        let v = {
            let mut a = shared.borrow_mut();
            let mut m = a.web.prefs[pref].as_object().cloned().unwrap_or_default();
            if n == 0 { m.remove(&t.key); } else { m.insert(t.key.clone(), serde_json::json!(n)); }
            let v = serde_json::Value::Object(m);
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert(pref.into(), v.clone()); }
            v
        };
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(pref, v); }
    }
    // (a pair already decided with it: decided again)
    SMART_PAIR.with(|p| *p.borrow_mut() = None);
    // (its cover, if it's changed: the new one's address, for where it's shown now)
    let mut art_now: Option<String> = None;
    let mut art_back: Option<String> = None;
    let (fixes, trims) = {
        let mut a = shared.borrow_mut();
        let mut fixes = a.web.prefs["trackFixes"].as_object().cloned().unwrap_or_default();
        let mut trims = a.web.prefs["trackTrims"].as_object().cloned().unwrap_or_default();
        // (its title and artist, as you want them, and its cover: an address picked, its own kept beside it so it can
        // come back; only addresses, the pictures fetched from there as any cover is)
        let art = TP_ART.with(|a| a.borrow_mut().take());
        if reset {
            // (and its own cover back, if one was picked)
            art_back = fixes.get(&t.key).and_then(|f| f["artOwn"].as_str()).map(String::from);
            fixes.remove(&t.key);
        } else {
            let mut fx = fixes.get(&t.key).and_then(|v| v.as_object().cloned()).unwrap_or_default();
            if title == t.title && artist == t.artist { if !fx.contains_key("art") { fx.clear(); } } else { fx.insert("title".into(), serde_json::json!(title)); fx.insert("artist".into(), serde_json::json!(artist)); }
            match art.as_deref() {
                Some("") => { if let Some(own) = fx.remove("artOwn") { art_back.replace(own.as_str().unwrap_or("").to_string()); } fx.remove("art"); }
                Some(url) => { if !fx.contains_key("artOwn") { fx.insert("artOwn".into(), serde_json::json!(t.art)); } fx.insert("art".into(), serde_json::json!(url)); }
                None => {}
            }
            if fx.is_empty() { fixes.remove(&t.key); } else { fixes.insert(t.key.clone(), serde_json::Value::Object(fx)); }
        }
        let new_art = if reset { art_back.take() } else { match art.as_deref() { Some("") => art_back.take(), Some(u) => Some(u.to_string()), None => None } };
        if let Some(u) = &new_art {
            a.queue.items.iter_mut().filter(|q| q.t.key == t.key).for_each(|q| q.t.art = u.clone());
            a.page_all.iter_mut().filter(|x| x.key == t.key).for_each(|x| x.art = u.clone());
        }
        art_now = new_art;
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
    // (playing now, with a new cover: it's fetched and shown)
    if let Some(url) = art_now.filter(|_| shared.borrow().queue.current().is_some_and(|c| c.key == t.key)) {
        let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
        rt.spawn(async move {
            let px = player::fetch_art(url, 500).await;
            let _ = weak.upgrade_in_event_loop(move |ui| if let Some(px) = px { let img = slint::Image::from_rgba8(px); ui.set_now_art(img.clone()); ui.set_now_art_big(img); });
        });
    }
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
    /// each collection's tracks once made (by its name), as the web app's streamColl: opened again at
    /// once, and what autoplay picks from while Stream-Safe Music is on
    static STREAM_BUILT: RefCell<HashMap<String, Vec<Track>>> = RefCell::new(HashMap::new());
}

/// A collection made: from every platform that's on, only what's safe, each song once, mixed
async fn stream_build(c: &StreamColl, prefs: &serde_json::Value, library: Vec<Track>) -> Vec<Track> {
    let mut found: Vec<Track> = match c {
        StreamColl::Likes => library,
        // (an artist or label cleared: its names, four at most, searched everywhere)
        StreamColl::Cleared(names) => futures_util::future::join_all(names.iter().take(4).map(|n| sources::search(n))).await.into_iter().flatten().collect(),
        StreamColl::Genre(g) => {
            let qs = [g.clone(), format!("{g} creative commons"), format!("{g} royalty free"), format!("{g} no copyright")];
            futures_util::future::join_all(qs.iter().map(|q| sources::search(q))).await.into_iter().flatten().filter(streamsafe::open).collect()
        }
    };
    let mut seen = std::collections::HashSet::new();
    found.retain(|t| !t.is_mix() && streamsafe::ok(prefs, t) && seen.insert(t.key.clone()));
    queue::shuffle_in_place(&mut found);
    found
}

/// Autoplay with Stream-Safe Music on: from the collections only (those made, and a couple more if
/// there's little to pick from), as the web app's streamAutoplayPool
async fn stream_autoplay_pool(prefs: serde_json::Value, library: Vec<Track>, have: Vec<Track>, todo: Vec<(String, StreamColl)>) -> (Vec<Track>, Vec<(String, Vec<Track>)>) {
    let mut have = have;
    let mut made = Vec::new();
    if have.len() < 30 {
        for (name, c) in todo.into_iter().take(2) {
            let got = stream_build(&c, &prefs, library.clone()).await;
            have.extend(got.iter().cloned());
            made.push((name, got));
        }
    }
    queue::shuffle_in_place(&mut have);
    (have, made)
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
    ui.set_page_sub("Music that's safe on stream: artists and labels cleared for streaming, Creative Commons music, and the likes you've marked safe.".into());
    ui.set_chips(ModelRc::default());
    let prefs = shared.borrow().web.prefs.clone();
    let shelves: Vec<(String, Vec<(String, StreamColl)>)> = vec![
        ("Cleared for streaming".into(), streamsafe::cleared_on(&prefs).into_iter().map(|(n, names)| (n, StreamColl::Cleared(names))).collect()),
        ("Creative Commons".into(), streamsafe::GENRES.iter().map(|g| (g.to_string(), StreamColl::Genre(g.to_string()))).collect()),
        ("From your likes".into(), vec![("Your Safe Likes".into(), StreamColl::Likes)]),
    ];
    let shelves: Vec<_> = shelves.into_iter().filter(|(_, l)| !l.is_empty()).collect();
    let sub = |c: &StreamColl| match c { StreamColl::Cleared(_) => "Cleared for streaming", StreamColl::Genre(_) => "Creative Commons", StreamColl::Likes => "From your likes" };
    let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().map(|(_, l)| Rc::new(VecModel::from(l.iter().map(|(n, c)| Tile { title: n.clone().into(), sub: sub(c).into(), art: slint::Image::default(), plat: "".into(), genre: false, c1: Default::default(), c2: Default::default(), track: false, liked: false, row: Default::default(), miss: false }).collect::<Vec<_>>()))).collect();
    let rows: Vec<Shelf> = shelves.iter().zip(&models).map(|((t, _), m)| Shelf { title: t.clone().into(), tiles: ModelRc::from(m.clone()), open: false, gone: false, shown: 0 }).collect();
    ui.set_shelves(ModelRc::from(Rc::new(VecModel::from(rows))));
    SHELF_MODELS.with(|m| *m.borrow_mut() = models);
    STREAM_COLLS.with(|s| *s.borrow_mut() = shelves.into_iter().map(|(_, l)| l).collect());
}

/// A collection opened: made from every platform that's on, only what's safe, each song once, mixed
fn open_stream_coll(shared: &Shared, sh: usize, k: usize) {
    let Some((name, c)) = STREAM_COLLS.with(|s| s.borrow().get(sh).and_then(|l| l.get(k).cloned())) else { return };
    let (prefs, library) = { let a = shared.borrow(); (a.web.prefs.clone(), a.library.clone()) };
    let sub_of = |found: &[Track]| if found.is_empty() { "Nothing safe on stream turned up here just now.".to_string() } else { format!("{}, all safe on stream.", plural(found.len(), "track", "tracks")) };
    // (made already: at once)
    if let Some(found) = STREAM_BUILT.with(|b| b.borrow().get(&name).cloned()) {
        let sub = sub_of(&found);
        return show_tracks(shared, "streamcoll", &name, sub, found, false);
    }
    show_tracks(shared, "streamcoll", &name, "Finding music that's safe on stream...".into(), Vec::new(), false);
    let key = name.clone();
    load_into(shared, "streamcoll", name.clone(), async move {
        let found = stream_build(&c, &prefs, library).await;
        let keep = found.clone();
        let _ = slint::invoke_from_event_loop(move || STREAM_BUILT.with(|b| { b.borrow_mut().insert(key, keep); }));
        let sub = sub_of(&found);
        (found, sub)
    });
}

// ---------- Stats ----------
// (the web app's statsPeriod: week, month, year, all; this month and this year by the calendar, as its)
const PERIOD_KEYS: [&str; 4] = ["week", "month", "year", "all"];

/// How long ago this month (first = false) or this year began, by this PC's clock, in ms
fn since_start_of(year: bool) -> u64 {
    #[cfg(windows)]
    {
        let t = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
        let (y, m, d) = (t.wYear as u64, t.wMonth as u64, t.wDay as u64);
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let before: [u64; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        let days = if year { before[(m - 1) as usize] + d - 1 + u64::from(leap && m > 2) } else { d - 1 };
        ((days * 86_400 + t.wHour as u64 * 3600 + t.wMinute as u64 * 60 + t.wSecond as u64) * 1000) + t.wMilliseconds as u64
    }
    #[cfg(not(windows))]
    { if year { 365 * 86_400_000 } else { MONTH_MS } }
}

/// The period last picked (This Month unless one was)
fn stats_period(prefs: &serde_json::Value) -> usize {
    PERIOD_KEYS.iter().position(|k| prefs["statsPeriod"].as_str() == Some(k)).unwrap_or(1)
}

/// A length of time as the web app's fmtHours: "2.5 hours", "45 minutes"
fn fmt_hours(ms: u64) -> String {
    let h = ms as f64 / 3_600_000.0;
    if h >= 10.0 { format!("{h:.0} hours") } else if h >= 1.0 { format!("{h:.1} hours") } else { format!("{} minutes", ms / 60_000) }
}

/// A day number (days since 1970) as (year, month, day)
fn civil_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

thread_local! {
    /// Stats' card looked at more closely ("" for them all), and its ranked list's rows: (what a click
    /// does, its cover's address)
    static STATS_DETAIL: RefCell<String> = const { RefCell::new(String::new()) };
    static STATS_ROWS: RefCell<Vec<(StatsRow, String)>> = const { RefCell::new(Vec::new()) };
    static STATS_BLOCKS: RefCell<Option<Rc<VecModel<StatBlock>>>> = const { RefCell::new(None) };
}

#[derive(Clone)]
enum StatsRow { Artist(String), Song(usize) }

/// A recorded play as a track to play: this app's own copy if it has one, else made from what was kept
fn play_track(a: &App, p: &webstate::Play) -> Option<Track> {
    if let Some(t) = a.library.iter().chain(a.queue.items.iter().map(|q| &q.t)).find(|t| t.key == p.id) { return Some(t.clone()); }
    let id = p.id.clone();
    let src = if p.src.is_empty() { "soundcloud" } else { p.src.as_str() };
    let mut snap = serde_json::json!({ "id": id, "title": p.title, "duration": p.ms, "artwork_url": p.art, "genre": p.genre, "source": src, "user": { "username": p.artist }, "displayArtist": p.artist });
    if src == "soundcloud" { snap["id"] = serde_json::json!(id.trim_start_matches("sc:")); }
    Track::from_snapshot(&snap)
}

/// Stats' closer look at one card (the web app's renderStatsDetail): its heading, and its blocks
fn stats_detail(a: &App, ui: &AppWindow, list: &[&webstate::Play], since: u64, period: usize, which: &str) -> Rc<VecModel<StatBlock>> {
    let label = PERIOD_CHIPS[period].to_lowercase();
    let ms: u64 = list.iter().map(|p| p.ms).sum();
    let day_of = |ts: u64| ((ts / 3_600_000) as i64 + local_offset_hours()).div_euclid(24);
    let first_artist = |p: &webstate::Play| p.artist.split(',').next().unwrap_or("").trim().to_string();
    let block = |kind: &str, kicker: &str| StatBlock { kind: kind.into(), kicker: kicker.into(), big: "".into(), sub: "".into(), wide: true, bars: ModelRc::default(), rows: ModelRc::default(), cells: ModelRc::default() };
    let bars = |v: Vec<(String, f64, String)>| { let top = v.iter().map(|x| x.1).fold(1e-9, f64::max); ModelRc::from(Rc::new(VecModel::from(v.into_iter().map(|(l, n, t)| StatBar { label: l.into(), v: (n / top) as f32, tip: t.into() }).collect::<Vec<_>>()))) };
    let months = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let days_names = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
    let weekday = |d: i64| (d + 4).rem_euclid(7) as usize;   // (day 0, 1 January 1970, was a Thursday)
    let mut out: Vec<StatBlock> = Vec::new();
    let mut rows: Vec<(StatsRow, String)> = Vec::new();
    let head: String;
    match which {
        "time" => {
            head = format!("You listened for {} {label}", fmt_hours(ms));
            let by_month = period >= 2;
            let mut m: std::collections::BTreeMap<i64, (u64, usize)> = Default::default();
            for p in list {
                let d = day_of(p.ts);
                let k = if by_month { let (y, mo, _) = civil_date(d); y * 12 + mo as i64 } else { d };
                let e = m.entry(k).or_default();
                e.0 += p.ms;
                e.1 += 1;
            }
            let items: Vec<(String, f64, String)> = m.iter().rev().take(60).collect::<Vec<_>>().into_iter().rev().map(|(k, (ms, n))| {
                let lab = if by_month { months[((k - 1).rem_euclid(12)) as usize].to_string() } else { let (_, mo, dd) = civil_date(*k); format!("{dd} {}", months[(mo - 1) as usize]) };
                (lab, *ms as f64, format!("{}, {}", fmt_hours(*ms), plural(*n, "play", "plays")))
            }).collect();
            let mut b = block("bars", if by_month { "BY MONTH" } else { "BY DAY" });
            b.bars = bars(items);
            out.push(b);
            let span = ((now_ms().saturating_sub(list[0].ts.max(since))) as f64 / 86_400_000.0).ceil().max(1.0);
            let mut b = block("big", "ON AVERAGE");
            b.big = fmt_hours((ms as f64 / span) as u64).into();
            b.sub = format!("a day, {} in all", plural(list.len(), "play", "plays")).into();
            out.push(b);
            let mut per: HashMap<i64, (u64, u64)> = HashMap::new();
            for p in list { let e = per.entry(day_of(p.ts)).or_default(); e.0 += p.ms; e.1 = p.ts; }
            if let Some((d, (dms, _))) = per.into_iter().max_by_key(|x| x.1 .0) {
                let (_, mo, dd) = civil_date(d);
                let mut b = block("big", "YOUR BIGGEST DAY");
                b.big = format!("{} {dd} {}", days_names[weekday(d)], ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][(mo - 1) as usize]).into();
                b.sub = fmt_hours(dms).into();
                out.push(b);
            }
        }
        "artists" | "songs" => {
            let songs = which == "songs";
            let mut tally: HashMap<String, (usize, u64, &webstate::Play)> = HashMap::new();
            for p in list {
                let k = if songs { format!("{}|{}", p.title, p.artist) } else { first_artist(p) };
                let e = tally.entry(k).or_insert((0, 0, p));
                e.0 += 1;
                e.1 += p.ms;
                e.2 = p;
            }
            let mut v: Vec<(String, (usize, u64, &webstate::Play))> = tally.into_iter().collect();
            v.sort_by(|x, y| y.1 .0.cmp(&x.1 .0).then(y.1 .1.cmp(&x.1 .1)));
            v.truncate(50);
            let top = v.first().map_or(1, |x| x.1 .0).max(1);
            head = format!("Your top {} {label}", if songs { "songs" } else { "artists" });
            let mut b = block("rows", "");
            let mut tracks: Vec<Track> = Vec::new();
            let marks = marks_of(a);
            b.rows = ModelRc::from(Rc::new(VecModel::from(v.iter().enumerate().map(|(i, (k, (n, ms, p)))| {
                let mut row = Row::default();
                if songs {
                    if let Some(t) = play_track(a, p) { row = row_menu(a, &marks, &t, t.row(false)); tracks.push(t); rows.push((StatsRow::Song(tracks.len() - 1), p.art.clone())); } else { rows.push((StatsRow::Song(usize::MAX), p.art.clone())); }
                } else { rows.push((StatsRow::Artist(k.clone()), p.art.clone())); }
                let song = row.title != "";
                StatRow { row, song, rank: (i + 1) as i32, name: if songs { p.title.clone().into() } else { k.clone().into() }, sub: if songs { format!("{} \u{b7} {} \u{b7} {}", p.artist, plural(*n, "play", "plays"), fmt_hours(*ms)).into() } else { format!("{} \u{b7} {}", plural(*n, "play", "plays"), fmt_hours(*ms)).into() }, share: *n as f32 / top as f32, art: slint::Image::default() }
            }).collect::<Vec<_>>())));
            STATS_TRACKS.with(|t| *t.borrow_mut() = tracks);
            out.push(b);
        }
        "when" => {
            let mut hours = [0f64; 24];
            let mut days = [0f64; 7];
            for p in list {
                let h = ((p.ts / 3_600_000) as i64 + local_offset_hours()).rem_euclid(24) as usize;
                let w = if p.ms > 0 { p.ms } else { 180_000 } as f64;
                hours[h] += w;
                days[weekday(day_of(p.ts))] += w;
            }
            head = format!("You're a {}", ui.get_st_kind());
            let hname = |h: usize| match h { 0 => "12a".to_string(), 12 => "12p".to_string(), h if h < 12 => format!("{h}a"), h => format!("{}p", h - 12) };
            let mut b = block("bars", "BY HOUR");
            b.bars = bars((0..24).map(|h| (if h % 3 == 0 { hname(h) } else { String::new() }, hours[h], fmt_hours(hours[h] as u64))).collect());
            out.push(b);
            let mut b = block("bars", "BY DAY");
            b.bars = bars((0..7).map(|d| (days_names[d][..3].to_string(), days[d], fmt_hours(days[d] as u64))).collect());
            out.push(b);
            let mut b = block("text", "");
            b.sub = ui.get_st_kind_sub();
            out.push(b);
        }
        "where" => {
            head = "Where it came from".into();
            let mut plats: HashMap<String, (usize, u64)> = HashMap::new();
            for p in list { let e = plats.entry(if p.src.is_empty() { "soundcloud".into() } else { p.src.clone() }).or_default(); e.0 += 1; e.1 += p.ms; }
            let mut v: Vec<_> = plats.into_iter().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1 .0));
            let pname = |p: &str| plat_name(p).to_string();
            for (pl, (n, pms)) in v {
                let mut tops: HashMap<String, usize> = HashMap::new();
                for p in list.iter().filter(|p| (if p.src.is_empty() { "soundcloud" } else { p.src.as_str() }) == pl) { *tops.entry(first_artist(p)).or_default() += 1; }
                let mut tops: Vec<_> = tops.into_iter().collect();
                tops.sort_by_key(|x| std::cmp::Reverse(x.1));
                let mut b = block("big", &pname(&pl).to_uppercase());
                b.big = format!("{}%", n * 100 / list.len()).into();
                b.sub = format!("{} \u{b7} {}{}", plural(n, "play", "plays"), fmt_hours(pms), if tops.is_empty() { String::new() } else { format!(". Most played: {}", tops.iter().take(3).map(|x| x.0.clone()).collect::<Vec<_>>().join(", ")) }).into();
                out.push(b);
            }
        }
        _ => {
            // a streak: the last twelve weeks, a square a day, and the best run
            let all = &a.web.plays;
            let mut per: HashMap<i64, usize> = HashMap::new();
            for p in all { *per.entry(day_of(p.ts)).or_default() += 1; }
            let today = day_of(now_ms());
            let start = today - 83 - weekday(today) as i64;
            let cells: Vec<i32> = (start..=today).map(|d| match per.get(&d).copied().unwrap_or(0) { 0 => 0, n if n > 20 => 3, n if n > 5 => 2, _ => 1 }).collect();
            head = format!("{} in a row", ui.get_st_streak());
            let mut b = block("cal", "THE LAST TWELVE WEEKS");
            b.cells = ModelRc::from(Rc::new(VecModel::from(cells)));
            out.push(b);
            let mut sorted: Vec<i64> = per.keys().copied().collect();
            sorted.sort();
            let (mut best, mut run) = (0usize, 0usize);
            for (i, d) in sorted.iter().enumerate() { run = if i > 0 && d - sorted[i - 1] == 1 { run + 1 } else { 1 }; best = best.max(run); }
            let mut b = block("big", "YOUR BEST");
            b.big = plural(best, "day", "days").into();
            b.sub = "in a row".into();
            out.push(b);
        }
    }
    STATS_ROWS.with(|r| *r.borrow_mut() = rows);
    ui.set_st_detail_head(head.into());
    let model = Rc::new(VecModel::from(out));
    ui.set_st_blocks(ModelRc::from(model.clone()));
    model
}

thread_local! {
    /// the closer look at your top songs: the songs, to play from one of them down
    static STATS_TRACKS: RefCell<Vec<Track>> = const { RefCell::new(Vec::new()) };
    /// the overview's top songs, playable, and its top artists' names
    static STATS_TOP: RefCell<(Vec<Track>, Vec<String>)> = const { RefCell::new((Vec::new(), Vec::new())) };
}

/// Clear Listening History: asked once more (a second click within five seconds), then done
fn stats_clear(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if !ui.get_st_clear_asking() {
        ui.set_st_clear_asking(true);
        let weak = ui.as_weak();
        slint::Timer::single_shot(std::time::Duration::from_secs(5), move || { if let Some(ui) = weak.upgrade() { ui.set_st_clear_asking(false); } });
        return;
    }
    ui.set_st_clear_asking(false);
    let now = now_ms();
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::clear_plays(now); }
    {
        let mut a = shared.borrow_mut();
        a.web.plays.clear();
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("statsClearedAt".into(), serde_json::json!(now)); }
    }
    ui.set_status("Listening history cleared.".into());
    let period = shared.borrow().chip;
    open_stats(shared, period);
}

fn open_stats(shared: &Shared, period: usize) {
    // (kept for next time, as the web app's)
    if stats_period(&shared.borrow().web.prefs) != period {
        let v = serde_json::json!(PERIOD_KEYS[period.min(3)]);
        if let Some(o) = shared.borrow_mut().web.prefs.as_object_mut() { o.insert("statsPeriod".into(), v.clone()); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("statsPeriod", v); }
    }
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
        1 => now.saturating_sub(since_start_of(false)),
        2 => now.saturating_sub(since_start_of(true)),
        _ => 0,
    };
    let list: Vec<&webstate::Play> = a.web.plays.iter().filter(|p| p.ts >= since).collect();
    let which = STATS_DETAIL.with(|d| d.borrow().clone());
    ui.set_st_detail(if list.is_empty() { "".into() } else { which.clone().into() });
    if list.is_empty() {
        ui.set_st_empty(true);
        ui.set_st_big("Nothing yet".into());
        ui.set_st_sub("Play some music and your stats will grow here.".into());
        ui.set_st_songs(ModelRc::default());
        ui.set_st_artists(ModelRc::default());
        return;
    }
    ui.set_st_empty(false);
    if !which.is_empty() {
        let blocks = stats_detail(&a, &ui, &list, since, period, &which);
        let weak = a.ui.clone();
        drop(a);
        // (the ranked lists' covers, as they come)
        let urls: Vec<String> = STATS_ROWS.with(|r| r.borrow().iter().map(|x| x.1.clone()).collect());
        STATS_BLOCKS.with(|m| *m.borrow_mut() = Some(blocks));
        shared.borrow().rt.spawn(async move {
            for (k, u) in urls.into_iter().enumerate().take(50) {
                if let Some(px) = player::fetch_art(u, 96).await {
                    let _ = weak.upgrade_in_event_loop(move |_| STATS_BLOCKS.with(|m| {
                        let Some(model) = m.borrow().clone() else { return };
                        let Some(mut b) = model.row_data(0) else { return };
                        let rows = b.rows.clone();
                        if let Some(mut r) = rows.row_data(k) { r.art = slint::Image::from_rgba8(px); rows.set_row_data(k, r); }
                        b.rows = rows;
                        let _ = b;
                    }));
                }
            }
        });
        return;
    }
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
        // (where the sound came from: a song found on MusicBrainz, Deezer or Apple Music plays from YouTube Music)
        let src = match p.src.as_str() { "" => "soundcloud", "musicbrainz" | "deezer" | "itunes" | "catalog" => "youtube", s => s };
        *plats.entry(src.to_string()).or_insert(0) += 1;
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
    // (playable as they are: a click on one plays the top songs from it; an artist opens their page)
    let marks = marks_of(&a);
    let playable: Vec<_> = songs.iter().take(5).filter_map(|(n, p)| play_track(&a, p).map(|t| (*n, p, t))).collect();
    let song_items: Vec<Item> = playable.iter().map(|(n, p, t)| Item { name: p.title.clone().into(), sub: format!("{} · {}", p.artist, plural(*n, "play", "plays")).into(), art: slint::Image::default(), plat: p.src.clone().into(), row: row_menu(&a, &marks, t, t.row(false)), ..Default::default() }).collect();
    let top: Vec<Track> = playable.into_iter().map(|x| x.2).collect();
    STATS_TOP.with(|t| *t.borrow_mut() = (top, artists.iter().take(6).map(|(n, _)| n.clone()).collect()));
    let artist_items: Vec<Item> = artists.iter().take(6).map(|(name, (n, p))| Item { name: name.clone().into(), sub: plural(*n, "play", "plays").into(), art: slint::Image::default(), plat: p.src.clone().into(), ..Default::default() }).collect();
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
    // (and the best run there's been, as the web app's)
    let mut sorted: Vec<i64> = days.iter().copied().collect();
    sorted.sort();
    let (mut best, mut run) = (0usize, 0usize);
    for (i, d) in sorted.iter().enumerate() { run = if i > 0 && d - sorted[i - 1] == 1 { run + 1 } else { 1 }; best = best.max(run); }
    ui.set_st_streak(plural(streak, "day", "days").into());
    ui.set_st_streak_sub((if best > streak { format!("in a row, your best is {}", plural(best, "day", "days")) } else if streak > 1 { "in a row: your best yet".to_string() } else { "in a row".to_string() }).into());
    let mut plat_list: Vec<_> = plats.into_iter().collect();
    plat_list.sort_by_key(|x| std::cmp::Reverse(x.1));
    // (each by its own name: Deezer's, Apple Music's, MusicBrainz's and plugins' plays aren't SoundCloud's)
    let pname = |p: &str| plat_name(p);
    ui.set_st_plats(ModelRc::from(Rc::new(VecModel::from(plat_list.iter().map(|(p, n)| Item { name: pname(p).into(), sub: format!("{}%", n * 100 / list.len()).into(), art: slint::Image::default(), plat: p.clone().into(), ..Default::default() }).collect::<Vec<_>>()))));
    // the song on repeat (played most in one day, three times or more), then a few facts, different each
    // visit (the web app's statFacts)
    let day_of = |ts: u64| ((ts / 3_600_000) as i64 + local_offset_hours()).div_euclid(24);
    let when = |ts: u64| { let d = day_of(ts); let (_, m, dd) = civil_date(d); format!("{} {} {}", ["Thursday", "Friday", "Saturday", "Sunday", "Monday", "Tuesday", "Wednesday"][d.rem_euclid(7) as usize], dd, ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"][(m - 1) as usize]) };
    let mut facts: Vec<(String, String)> = Vec::new();
    {
        let mut per_day: HashMap<(i64, &str, &str), (usize, u64)> = HashMap::new();
        for p in &list { let e = per_day.entry((day_of(p.ts), p.title.as_str(), p.artist.as_str())).or_insert((0, 0)); e.0 += 1; e.1 = e.1.max(p.ts); }
        if let Some(((_, title, artist), (n, ts))) = per_day.into_iter().max_by_key(|x| x.1 .0).filter(|x| x.1 .0 > 2) {
            facts.push(("On repeat".into(), format!("\u{201c}{title}\u{201d} by {artist}: {} in one day, {}.", plural(n, "time", "times"), when(ts))));
        }
    }
    let mut extra: Vec<(String, String)> = Vec::new();
    if let Some((_, p)) = songs.first() {
        let ds: std::collections::HashSet<i64> = list.iter().filter(|x| x.title == p.title && x.artist == p.artist).map(|x| day_of(x.ts)).collect();
        if ds.len() > 1 { extra.push(("Day after day".into(), format!("You played \u{201c}{}\u{201d} on {}.", p.title, plural(ds.len(), "different day", "different days")))); }
    }
    let once = artists.iter().filter(|a| a.1 .0 == 1).count();
    if once > 2 { extra.push(("Explorer".into(), format!("{} got just one play. Plenty to dig into.", plural(once, "artist", "artists")))); }
    if since > 0 {
        let before: std::collections::HashSet<String> = a.web.plays.iter().filter(|p| p.ts < since).map(first_artist).collect();
        let fresh = artists.iter().filter(|(k, _)| !before.contains(k)).count();
        if fresh > 0 { extra.push(("New to you".into(), format!("{} new to you in this time.", if fresh == 1 { "1 artist was".to_string() } else { format!("{fresh} artists were") }))); }
    }
    let short = list.iter().filter(|p| p.ms > 0 && p.ms < 180_000).count();
    if list.len() > 10 { extra.push(("Short and sweet".into(), format!("{}% of your plays were under 3 minutes.", short * 100 / list.len()))); }
    let long = list.iter().filter(|p| p.ms > 15 * 60_000).count();
    if long > 0 { extra.push(("In it for the long haul".into(), format!("{} over 15 minutes made the cut.", if long == 1 { "1 set or podcast".to_string() } else { format!("{long} sets and podcasts") }))); }
    // (the longest session: plays less than 30 minutes apart)
    {
        let (mut best, mut cur): ((u64, u64, usize), Option<(u64, u64, u64, usize)>) = ((0, 0, 0), None);
        for p in &list {
            let c = match cur { Some(c) if p.ts.saturating_sub(c.1) <= 30 * 60_000 => c, _ => (p.ts, p.ts, 0, 0) };
            let c = (c.0, p.ts, c.2 + p.ms, c.3 + 1);
            if c.2 > best.0 { best = (c.2, c.0, c.3); }
            cur = Some(c);
        }
        if best.2 > 3 { extra.push(("Marathon".into(), format!("Your longest session ran {}, {} in a row, on {}.", fmt_hours(best.0), plural(best.2, "track", "tracks"), when(best.1)))); }
    }
    {
        let mut genres: HashMap<String, (usize, String)> = HashMap::new();
        for p in list.iter().filter(|p| !p.genre.is_empty()) { let e = genres.entry(p.genre.to_lowercase()).or_insert((0, p.genre.clone())); e.0 += 1; }
        if let Some((_, (n, g))) = genres.into_iter().max_by_key(|x| x.1 .0).filter(|x| x.1 .0 > 2) { extra.push(("Your sound".into(), format!("{g} came up the most, {}.", plural(n, "time", "times")))); }
    }
    if let Some((k, (n, _))) = artists.iter().find(|(k, (n, _))| *n >= 5 && a.web.plays.iter().filter(|p| first_artist(p) == **k).count() >= n * 2) {
        extra.push(("Old favourite".into(), format!("{k} is still on your mind: {} now, and more before that.", plural(*n, "play", "plays"))));
    }
    if let Some(first) = a.web.plays.first().filter(|p| now.saturating_sub(p.ts) >= 2 * 86_400_000) {
        let days_ago = (now.saturating_sub(first.ts)) / 86_400_000;
        extra.push(("Since the start".into(), format!("Your listening here goes back {}.", plural(days_ago as usize, "day", "days"))));
    }
    queue::shuffle_in_place(&mut extra);
    extra.truncate(3);
    facts.extend(extra);
    ui.set_st_fact_heads(ModelRc::from(Rc::new(VecModel::from(facts.iter().map(|f| slint::SharedString::from(f.0.to_uppercase())).collect::<Vec<_>>()))));
    ui.set_st_facts(ModelRc::from(Rc::new(VecModel::from(facts.into_iter().map(|f| f.1.into()).collect::<Vec<slint::SharedString>>()))));
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
const ART_KEEP: usize = 300;

/// The screen's scale (100 is 100%), for covers decoded at the size they're shown
static UI_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(100);

/// A cover's size in pixels, for one shown at `logical` px on this screen (at least that, at most twice it)
fn art_px(logical: f32) -> u32 {
    let scale = (UI_SCALE.load(std::sync::atomic::Ordering::Relaxed) as f32 / 100.0).clamp(1.0, 2.0);
    (logical * scale).ceil() as u32
}
const ART_AT_ONCE: usize = 16;

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

/// how many of the queue's rows get their covers
const QUEUE_ART: usize = 60;

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
        // the queue's covers too (its first rows: the ones in sight, as the web app's queue)
        for (k, (_, q, _)) in a.queue.panel().iter().enumerate().take(QUEUE_ART.min(a.qrows.row_count())) {
            let url = thumb_url(&q.t);
            if url.is_empty() {
                continue;
            }
            match c.map.get(&url) {
                Some(img) => {
                    if let Some(mut r) = a.qrows.row_data(k) {
                        if r.art.size().width == 0 && img.size().width > 0 {
                            r.art = img.clone();
                            a.qrows.set_row_data(k, r);
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
            let px = player::fetch_art(url.clone(), art_px(44.0)).await;
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
                // (the covers in sight all in: how long since the page opened)
                if ROW_ART.with(|c| c.borrow().loading.is_empty()) {
                    if let Some(t0) = PAGE_T0.with(|p| p.take()) { page_timing("covers in sight all in", t0); }
                }
            });
        });
    }
}

// ---------- the queue panel ----------
/// A row's menu, as the web app's track menu: its artists, its uploader, its like and star, where it
/// opens, Stream-Safe Music's Allow / Stop allowing
/// (what's liked and starred, once for a whole list of rows)
struct Marks { liked: std::collections::HashSet<String>, favs: std::collections::HashSet<String> }

fn marks_of(a: &App) -> Marks {
    Marks { liked: a.library.iter().map(|t| t.key.clone()).collect(), favs: marks::favourites(&a.web.favourites).into_iter().map(|t| t.key).collect() }
}

fn row_menu(a: &App, m: &Marks, t: &Track, mut r: Row) -> Row {
    let names: Vec<slint::SharedString> = t.artist.split([',', '&']).map(|x| x.trim()).filter(|x| !x.is_empty()).map(Into::into).collect();
    r.artists = ModelRc::from(Rc::new(VecModel::from(names)));
    let apart = !t.uploader.is_empty() && omnipulse_core::ranking::artist_key(&t.uploader) != omnipulse_core::ranking::artist_key(t.artist.split([',', '&']).next().unwrap_or(""));
    r.uploader = if apart { t.uploader.clone().into() } else { "".into() };
    r.can_like = !matches!(t.src, track::Src::Local(_));
    r.liked = m.liked.contains(&t.key);
    r.fav_on = true;
    r.fav = m.favs.contains(&t.key);
    r.open_label = if t.web_link().is_empty() { "".into() } else { match t.plat() { "spotify" => "Open in Spotify", "youtube" => "Open in YouTube Music", "musicbrainz" => "Open on MusicBrainz", "deezer" => "Open on Deezer", "itunes" => "Open in Apple Music", "local" => "", _ => "Open on SoundCloud" }.into() };
    r.cleared = streamsafe::menu_label(&a.web.prefs, t).map(|x| x.0).unwrap_or_default().into();
    r
}

/// A choice from a track's menu (the page's, or the queue's) that isn't about the queue or a playlist:
/// true if it was one of them
/// Play Next or Add to Queue, from anywhere (as the web app's playNext / addToQueue): with Stream-Safe
/// Music on, only what's safe; listening along in a Jam, to the host; otherwise into the queue
fn queue_add(shared: &Shared, t: Track, next: bool) {
    if streamsafe::on(&shared.borrow().web.prefs) && !streamsafe::ok(&shared.borrow().web.prefs, &t) {
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Stream-Safe Music is on, and that track isn't safe on stream.".into()); }
        return;
    }
    if let Some(kind) = listening() {
        let note = if kind != "jam" {
            "You're listening along. Only a Jam takes songs from listeners.".to_string()
        } else {
            listen_send(serde_json::json!({ "type": "add", "track": t.to_snapshot(), "next": next }));
            format!("Sent \"{}\" to the Jam{}", t.title, if next { ", to play next." } else { "." })
        };
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
        return;
    }
    let empty = shared.borrow().queue.current().is_none();
    { let mut a = shared.borrow_mut(); if next { a.queue.play_next(t.clone()); } else { a.queue.add(t.clone()); } paint_queue(&mut a); save_session(&a); }
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{} \"{}\".", if next { "Playing next:" } else { "Added to the queue:" }, t.title).into()); }
    if empty { start(shared); }
}

/// A song's menu's choice, wherever it was right-clicked (a tile, Stats, the queue; a page's rows have
/// their own, for what's picked): to the queue, a playlist, not recommended, a link, and the rest
fn menu_act(shared: &Shared, t: Track, act: &str) {
    if act == "next" || act == "add" { return queue_add(shared, t, act == "next"); }
    if act.starts_with("pl") { return pl_row_action(shared, act, t); }
    if let Some(what) = act.strip_prefix("hide:") { return hide_track(shared, &t, what); }
    if let Some(plat) = act.strip_prefix("link:") { return copy_track_link(shared, &t, plat); }
    if act == "moreversions" {
        let (who, _) = versions::guess_names(&t);
        let q = format!("{} {}", if who.is_empty() { t.artist.clone() } else { who }, vgroup::base_title(&t, ""));
        return go(shared, Place::Search(q.trim().into()));
    }
    track_act(shared, &t, act);
}

/// 1234567 as "1,234,567"
fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() { if i > 0 && (s.len() - i) % 3 == 0 { out.push(','); } out.push(c); }
    out
}

fn track_act(shared: &Shared, t: &Track, act: &str) -> bool {
    match act {
        // (the player's platform badge: who put it there, or its artist)
        "platform" => return track_act(shared, t, if t.uploader.is_empty() { "artist:0" } else { "uploader" }),
        "like" => like_track(shared, t.clone()),
        "fav" => fav_track(shared, t.clone()),
        "uploader" => go(shared, Place::ArtistNamed(t.uploader.clone())),
        "open" => { let _ = omnipulse_core::open_url(t.web_link()); }
        "prefs" => tp_open(shared, t),
        "cleared" => {
            let (prefs, note) = { let a = shared.borrow(); match streamsafe::menu_run(&a.web.prefs, t) { Some(x) => x, None => return true } };
            pref_put(shared, "streamCleared", prefs);
            let mut a = shared.borrow_mut();
            apply_filter(&mut a);
            paint_queue(&mut a);
            if let Some(ui) = a.ui.upgrade() { ui.set_status(note.into()); }
        }
        // (a click on the artist's name, at a share of its width (in thousandths): the one of its names there)
        a if a.starts_with("artistat:") => {
            let at = a[9..].parse::<f64>().unwrap_or(0.0) / 1000.0;
            let chars: Vec<char> = t.artist.chars().collect();
            let i = ((at * chars.len() as f64) as usize).min(chars.len().saturating_sub(1));
            let k = chars[..i].iter().filter(|c| matches!(c, ',' | '&')).count();
            return track_act(shared, t, &format!("artist:{k}"));
        }
        a if a.starts_with("artist:") => {
            let k: usize = a[7..].parse().unwrap_or(0);
            let name = t.artist.split([',', '&']).map(|x| x.trim()).filter(|x| !x.is_empty()).nth(k).unwrap_or(&t.artist).to_string();
            go(shared, Place::ArtistNamed(name));
        }
        _ => return false,
    }
    true
}

/// The song coming next (DJ decks): its name, its artist and its cover, once the cover's in
fn paint_next(a: &App) {
    let Some(ui) = a.ui.upgrade() else { return };
    let nq = a.queue.qi.and_then(|i| a.queue.items.get(i + 1));
    // (and for the Stream Widget's Now and Next: who asked for it too)
    integrations::set_next(nq.map(|q| &q.t), nq.map(|q| q.by.as_str()).unwrap_or(""));
    let next = nq.map(|q| &q.t);
    ui.set_next_title(next.map(|t| t.title.clone()).unwrap_or_default().into());
    ui.set_next_artist(next.map(|t| t.artist.clone()).unwrap_or_default().into());
    if let Some(img) = next.and_then(|t| ROW_ART.with(|c| c.borrow().map.get(&thumb_url(t)).cloned())) { ui.set_next_art(img); }
}

/// The playing song's shape, for Waveform: each bar's loudness from Seamless Mix's measuring, 0 to 1 (at most
/// 160 of them); none until it's been measured
fn paint_wave(ui: &AppWindow, key: &str) {
    let m = rhythm_full(key).or_else(|| omnipulse_core::rhythm::known(key));
    let db: Vec<f32> = m.map(|m| m.rhythm.bar_db.clone()).unwrap_or_default();
    WAVE_FOR.with(|w| *w.borrow_mut() = (key.to_string(), db.len()));
    if db.is_empty() { ui.set_now_wave(ModelRc::default()); return; }
    let step = (db.len() as f32 / 160.0).max(1.0);
    let n = ((db.len() as f32) / step).floor().max(1.0) as usize;
    let top = db.iter().cloned().fold(f32::MIN, f32::max);
    let v: Vec<f32> = (0..n).map(|i| {
        let a = (i as f32 * step) as usize;
        let b = (((i + 1) as f32 * step) as usize).clamp(a + 1, db.len());
        let x = db[a..b].iter().sum::<f32>() / (b - a) as f32;
        // (24 dB under the loudest bar: silent; the loudest: full)
        ((x - (top - 24.0)) / 24.0).clamp(0.06, 1.0)
    }).collect();
    ui.set_now_wave(ModelRc::from(Rc::new(VecModel::from(v))));
}

thread_local! {
    /// the song the waveform's of, and how many bars it had (measured since: drawn again)
    static WAVE_FOR: RefCell<(String, usize)> = const { RefCell::new((String::new(), 0)) };
}

/// The cover, blurred (Cover Strip): shrunk to a few pixels, which the bar stretches back out smoothly
fn art_blur(ui: &AppWindow) {
    let Some(px) = ui.get_now_art().to_rgba8() else { return ui.set_now_art_blur(slint::Image::default()) };
    let Some(img) = image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()) else { return };
    // (small, then blurred: smooth however far it's stretched, by either renderer)
    let small = image::imageops::blur(&image::imageops::resize(&img, 96, 48, image::imageops::FilterType::Triangle), 6.0);
    let out = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::clone_from_slice(small.as_raw(), small.width(), small.height());
    ui.set_now_art_blur(slint::Image::from_rgba8(out));
}

fn paint_queue(a: &mut App) {
    paint_next(a);
    rhythm_wanted_set(a);
    // (a mix under way: the queue as it was, the one going out playing, the one coming in lit)
    let real = a.queue.qi;
    let shown = mix_show_qi().filter(|&p| p < a.queue.items.len());
    if shown.is_some() { a.queue.qi = shown; }
    let coming = shown.and(real);
    paint_queue_rows(a, coming);
    a.queue.qi = real;
}

fn paint_queue_rows(a: &mut App, coming: Option<usize>) {
    // (Stream-Safe Music on: what isn't safe isn't shown, as the web app's; its heading goes to the next)
    let safe = streamsafe::on(&a.web.prefs).then(|| a.web.prefs.clone());
    let mut carry = String::new();
    let panel: Vec<(usize, &queue::QItem, String)> = a.queue.panel().into_iter().filter_map(|(i, q, head)| {
        let hide = safe.as_ref().is_some_and(|p| Some(i) != a.queue.qi && !streamsafe::ok(p, &q.t));
        if hide { if !head.is_empty() { carry = head; } return None; }
        let head = if head.is_empty() && !carry.is_empty() { std::mem::take(&mut carry) } else { carry.clear(); head };
        Some((i, q, head))
    }).collect();
    let marks = marks_of(a);
    let map: Vec<usize> = panel.iter().map(|p| p.0).collect();
    let rows: Vec<Row> = panel
        .iter()
        .map(|(i, q, head)| {
            let mut r = row_menu(a, &marks, &q.t, q.t.row(Some(*i) == a.queue.qi));
            r.head = head.as_str().into();
            r.auto = q.auto;
            // (as the web app's: "requested by" a chat request; one that isn't safe on stream greyed)
            r.added_by = q.by.clone().into();

            r.coming = Some(*i) == coming;
            // (its cover, if it's in already; the rest come in rows_art)
            if let Some(img) = ROW_ART.with(|c| c.borrow().map.get(&thumb_url(&q.t)).cloned()) { r.art = img; }
            r
        })
        .collect();
    a.panel_map = map;
    slint::Timer::single_shot(std::time::Duration::ZERO, || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { rows_art(&sh); } }));
    a.qrows.set_vec(rows);
    if let Some(ui) = a.ui.upgrade() {
        ui.set_queue_requests(if integrations::set(&a.web.prefs, "req") { a.queue.pending_requests().count() as i32 } else { 0 });
        ui.set_autoplay(a.autoplay);
        ui.set_autoplay_note(autoplay_note(a).into());
    }
}

fn autoplay_note(a: &App) -> String {
    if !a.autoplay {
        "When this is off, playback stops at the end of your queue.".into()
    } else if a.ap_busy {
        "Finding similar tracks...".into()
    } else if a.ap_failed.is_some() && a.ap_failed.as_deref() == a.queue.current().map(|t| t.key.as_str()) {
        "Autoplay couldn't find anything similar to this track.".into()
    } else {
        String::new()
    }
}

thread_local! {
    /// with Stream-Safe Music on, autoplay is its own (the web app's apStream): your usual one, kept
    /// meanwhile, to go back to
    static AP_USUAL: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Autoplay as Stream-Safe Music has it or not: on, the stream's (apStream, on unless switched off);
/// off, your usual one back
fn autoplay_for_safe(a: &mut App, safe: bool) {
    if safe {
        if AP_USUAL.with(|u| u.get()).is_none() { AP_USUAL.with(|u| u.set(Some(a.autoplay))); }
        a.autoplay = a.web.prefs["apStream"].as_bool() != Some(false);
    } else if let Some(usual) = AP_USUAL.with(|u| u.take()) {
        a.autoplay = usual;
    }
    if !a.autoplay { a.queue.drop_autoplay(); }
    if let Some(ui) = a.ui.upgrade() { ui.set_autoplay(a.autoplay); }
}

fn session_of(a: &App) -> serde_json::Value {
    let pos = a.resume_ms.unwrap_or_else(|| a.engine.position_ms());
    let mut v = a.queue.to_session(pos);
    // (your usual autoplay: the stream's is kept as apStream)
    v["autoplay"] = serde_json::json!(AP_USUAL.with(|u| u.get()).unwrap_or(a.autoplay));
    v
}

/// The session kept (written when it's changed, at most every 15 s: see queue::save_session)
fn save_session(a: &App) {
    queue::save_session(&session_of(a));
}

/// The session written at once: the app closing, restarting or going to the tray
fn save_session_now(a: &App) {
    queue::save_session_now(&session_of(a));
}

// ---------- playing ----------
fn start(shared: &Shared) {
    if std::env::var_os("OMNIPULSE_STEP_DEBUG").is_some() {
        let a = shared.borrow();
        println!("start debug: \"{}\" (qi {:?} of {}), engine at {} of {} ms, xfade {:?}", a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.queue.qi, a.queue.items.len(), a.engine.position_ms(), a.engine.duration_ms(), XFADE.with(|x| x.borrow().map(|v| v.0)));
    }
    if safe_skip(shared) {
        return;
    }
    // (the song change, for rating it: from what, to what, how, and why it wasn't mixed if it wasn't)
    if let Some(t) = shared.borrow().queue.current().cloned() {
        let how = CHANGE_HOW.with(|h| h.replace("picked"));
        let prev = LAST_PLAYING.with(|l| l.borrow_mut().replace(t.clone()));
        if prev.as_ref().is_some_and(|p| p.key != t.key || how == "ended") {
            let why = MIX_WHY.with(|m| m.borrow().clone()).filter(|(a, b, _)| prev.as_ref().is_some_and(|p| p.key == *a) && *b == t.key).map(|x| x.2);
            LAST_CHANGE.with(|c| *c.borrow_mut() = Some(SongChange { from: prev, to: t, how, why, at: std::time::Instant::now() }));
        }
    }
    // (anything started is what's shown: a mix being shown is over)
    if MIX_SHOW.with(|m| m.borrow_mut().take()).is_some() {
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.global::<Theme>().set_pulsing(false); }
    }
    let (t, token, engine, rt, weak, resume) = {
        let mut a = shared.borrow_mut();
        let Some(t) = a.queue.current().cloned() else { return };
        a.token += 1;
        // (the last track's measurement and fade aren't this one's, while it loads)
        a.cur_loud = None;
        FADE.with(|f| *f.borrow_mut() = (false, String::new()));
        a.heard = (t.key.clone(), 0.0, false);
        SCROBBLED.with(|x| x.set(false));
        a.last_pos = 0;
        a.lyrics.clear();
        a.loaded = true;
        // (the place kept for the restored track: only if it's that track starting, not another you picked)
        let for_this = std::mem::take(&mut a.resume_key) == t.key;
        let resume = a.resume_ms.take().filter(|&ms| for_this && ms > 3000 && (t.ms == 0 || ms + 3000 < t.ms)).unwrap_or(0);
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
    // (Seamless Mix or Smart Transitions on: Spotify songs from their YouTube Music version, so they can be
    // mixed: Spotify's own player plays one song at a time, never two crossfading)
    let mix_on_now = smart_on(&shared.borrow().web.prefs);
    // (Smart Transitions' fade: it comes in from its first sound, its silence before that left out)
    let smart_from = SMART_START.with(|x| x.borrow_mut().take()).filter(|(k, _)| *k == t.key && resume == 0 && trim_of(&shared.borrow().web.prefs, &t.key).0 == 0).map(|x| x.1);
    rt.spawn(async move {
        // a Go+ track: never the preview, but its version on YouTube Music (crate::versions); a
        // MusicBrainz song the same way (MusicBrainz knows it, YouTube Music has it to play)
        // (and a Spotify song with a free account signed in, or none for playback: from YouTube Music too)
        // (with Seamless Mix on, a Spotify song plays from YouTube Music too, its Spotify details kept,
        // so it mixes in and out like any other; Spotify's own playback if there's no version of it)
        let sp_free = matches!(t.src, track::Src::Spotify(_)) && (!sp_plays_here() || mix_on_now);
        let mb = matches!(t.src, track::Src::Catalog(_)) || listen_needs_version(&t) || sp_free;
        let (src, version) = if let Some((_, v)) = mixed {
            (Ok(None), v)
        } else if t.go_plus {
            full_elsewhere(&t, h.clone()).await
        } else if mb {
            // (a Spotify song with only the closest there is: Spotify's own, if it plays here)
            match versions::find(&t).await.filter(|v| v.exact || !(matches!(t.src, track::Src::Spotify(_)) && sp_plays_here())) {
                Some(v) => (player::open_source(&Track::youtube(&v.yt), h.clone()).await.map(Some), Some(v)),
                None if matches!(t.src, track::Src::Spotify(_)) && sp_plays_here() => (player::open_source(&t, h.clone()).await.map(Some), None),
                None => (Err("there's no version of it on YouTube Music".to_string()), None),
            }
        } else {
            let r = player::open_source(&t, h.clone()).await.map(Some);
            // (SoundCloud with only a preview for it, whatever it's called: no stream it can play, or a "full"
            // one that's 30 seconds of a longer song: the full song from elsewhere, as the web app's)
            let preview = matches!(t.src, track::Src::SoundCloud(_)) && match &r {
                Err(_) => true,
                Ok(Some(s)) => t.ms > 60_000 && s.duration_ms().is_some_and(|d| d > 0 && d < 35_000),
                Ok(None) => false,
            };
            if preview { full_elsewhere(&t, h.clone()).await } else { (r, None) }
        };
        let src = match (src, smart_from) {
            (Ok(Some(mut s)), Some(ms)) => tokio::task::spawn_blocking(move || { let _ = s.seek(ms); s }).await.map(Some).map_err(|e| e.to_string()),
            (src, _) => src,
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
                        if (smart_on(&shared.borrow().web.prefs)) && MIX_OUT_OK.with(|c| c.get()) { rhythm_ahead(&shared, t.clone()); }
                        let what = sound::loud_source(&t, version.as_ref().map(|v| v.yt.id.as_str()));
                        shared.borrow_mut().cur_loud = what.as_ref().map(|(k, id)| format!("{k}:{id}"));
                        FADE.with(|f| *f.borrow_mut() = (false, String::new()));
                        let known = what.as_ref().and_then(|(s, id)| shared.borrow().loud.get(&format!("{s}:{id}")).copied());
                        let db = sound::loud_db(&shared.borrow().web.prefs, &t, known, version.is_some());
                        // the track before ending with Fade Between Tracks on: both heard at once, this one coming in
                        let resumed = resume > 0 || trim_of(&shared.borrow().web.prefs, &t.key).0 > 0;
                        match (src, XFADE.with(|x| x.borrow_mut().take())) {
                            // (mixed in: playing already)
                            (None, _) => {}
                            (Some(src), Some((ms, at))) if !resumed && at.elapsed().as_secs() < 8 => {
                                engine.fade(None, 1.0, 200);
                                // (Smart Transitions': the voices handed over halfway, if it's long enough)
                                let smart = SMART_XFADE.with(|x| x.replace(false));
                                match smart::fade_handoff(ms, smart && set_row_on(&shared.borrow().web.prefs, "smartVoices", true)) {
                                    Some(h) => engine.crossfade_with(src, db, ms, h),
                                    None => engine.crossfade(src, db, ms),
                                }
                                XFADE_PULSE.with(|x| x.set(Some(std::time::Instant::now() + std::time::Duration::from_millis(ms))));
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
                                say_with(&ui, &note, vec![("Choose a version", StatusAct::Prefs(t.clone())), ("Skip", StatusAct::Skip)]);
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
        let (lyrics, from) = player::lyrics(&t2, &prefs).await;
        let _ = weak2.upgrade_in_event_loop(move |ui| {
            SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                if shared.borrow().token < token {
                    return;
                }
                let art_px = art.clone();
                let weak = ui.as_weak();
                let show = move || {
                    let Some(ui) = weak.upgrade() else { return };
                    let Some(shared) = SHARED.with(|s| s.borrow().clone()) else { return };
                    if shared.borrow().token != token { return; }
                    if let Some(px) = art {
                        // (a crossfade under way, album-art colours on: the accent pulses to the new cover's
                        // and back while both are heard, then takes it, as a mix's does)
                        let fading = XFADE_PULSE.with(|x| x.take()).and_then(|until| until.checked_duration_since(std::time::Instant::now()));
                        let pulse = fading.filter(|_| integrations::set(&shared.borrow().web.prefs, "artAccent"));
                        shared.borrow_mut().accent = integrations::accents(&px);
                        match pulse {
                            Some(left) => {
                                mix_pulse(&shared, &ui, &px);
                                let w = weak.clone();
                                slint::Timer::single_shot(left, move || {
                                    let Some(ui) = w.upgrade() else { return };
                                    let Some(shared) = SHARED.with(|s| s.borrow().clone()) else { return };
                                    if shared.borrow().token != token { return; }
                                    ui.global::<Theme>().set_pulsing(false);
                                    apply_look(&shared);
                                });
                            }
                            None => apply_look(&shared),
                        }
                        push_out(&shared, false);
                        let img = slint::Image::from_rgba8(px);
                        ui.set_now_art(img.clone());
                        ui.set_now_art_big(img);
                    }
                    let prefs = shared.borrow().web.prefs.clone();
                    show_lyrics(&shared, &ui, &prefs, lyrics, &from);
                };
                // (a mix under way: the one going out's shown till it's over; with album-art colours on, its
                // accent fading to the next cover's and back meanwhile)
                let held = MIX_SHOW.with(|m| m.borrow().is_some());
                if held { if let Some(px) = &art_px { mix_pulse(&shared, &ui, px); } }
                if held { MIX_SHOW.with(|m| if let Some(x) = m.borrow_mut().as_mut() { x.pending = Some(Box::new(show)); }); } else { show(); }
            });
        });
    });
}

thread_local! {
    /// the other places Now Playing offers lyrics from (their ids, as listed)
    static LY_OTHERS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// A song's lyrics shown, with where they're from and the other places to try ("Lyrics from KuGou.
/// Other sources: LRCLIB, NetEase, ..."), as the web app's
fn show_lyrics(shared: &Shared, ui: &AppWindow, prefs: &serde_json::Value, lyrics: Vec<player::Line>, from: &str) {
    // (a remix with none of its own: the original song's, said so)
    let (from, original) = match from.strip_suffix("+original") { Some(f) => (f, true), None => (from, false) };
    ui.set_lyrics_note(if lyrics.is_empty() { "No lyrics found for this one.".into() } else if original { "These are the original song's lyrics, so they may not match this remix. They aren't timed.".into() } else if lyrics.iter().all(|l| l.ms == 0) { "These lyrics aren't timed.".into() } else { "".into() });
    ui.set_lyrics_from(if from.is_empty() { "".into() } else { format!("Lyrics from {}.", player::lyrics_name(from)).into() });
    let others: Vec<String> = player::lyrics_order(prefs).into_iter().filter(|p| p != from).collect();
    ui.set_lyrics_others(ModelRc::from(Rc::new(VecModel::from(others.iter().map(|p| slint::SharedString::from(player::lyrics_name(p))).collect::<Vec<_>>()))));
    LY_OTHERS.with(|o| *o.borrow_mut() = others);
    let a = &mut *shared.borrow_mut();
    a.lyric_lines.set_vec(lyrics.iter().map(|l| Line { text: l.text.clone().into() }).collect::<Vec<_>>());
    a.lyrics = lyrics;
}

/// One of the other places picked: the song's lyrics from there (kept as its choice), or a note that
/// it has none
fn lyrics_pick(shared: &Shared, i: usize) {
    let Some(id) = LY_OTHERS.with(|o| o.borrow().get(i).cloned()) else { return };
    let Some(t) = shared.borrow().queue.current().cloned() else { return };
    let (rt, weak, token) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone(), a.token) };
    if let Some(ui) = weak.upgrade() { ui.set_status(format!("Lyrics from {}...", player::lyrics_name(&id)).into()); }
    rt.spawn(async move {
        let lyrics = player::lyrics_from(&t, &id).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            if shared.borrow().token != token { return; }
            if lyrics.is_empty() { ui.set_status(format!("{} has no lyrics for this one.", player::lyrics_name(&id)).into()); return; }
            ui.set_status("".into());
            player::lyrics_pick_set(&t.key, &id);
            let prefs = shared.borrow().web.prefs.clone();
            show_lyrics(&shared, &ui, &prefs, lyrics, &id);
        }));
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
/// (the songs worth measuring now: the one playing and the next few; kept as the queue moves)
static RHYTHM_WANTED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

/// The songs worth measuring now, as the queue stands (the one playing and the next two)
fn rhythm_wanted_set(a: &App) {
    let from = a.queue.qi.unwrap_or(0);
    let keys: Vec<String> = a.queue.items.iter().skip(from).take(3).map(|q| q.t.key.clone()).collect();
    if let Ok(mut w) = RHYTHM_WANTED.lock() { *w = keys; }
}

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
    // (what's worth measuring, as the queue stands now; this one too, if it's asked for outside it)
    rhythm_wanted_set(&shared.borrow());
    if let Ok(mut w) = RHYTHM_WANTED.lock() { if !w.contains(&t.key) { w.push(t.key.clone()); } }
    let rt = shared.borrow().rt.clone();
    rt.spawn(async move {
        let key = t.key.clone();
        // (one song measured at a time, and only one that's still playing or next by its turn: memory and
        // CPU kept to one song's worth, however fast songs change)
        static ONE: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
        if let Ok(_turn) = ONE.acquire().await {
            if RHYTHM_WANTED.lock().map(|w| w.iter().any(|k| *k == key)).unwrap_or(true) { rhythm_measure_whole(t).await; }
        }
        RHYTHM_FULL_BUSY.lock().unwrap().retain(|k| *k != key);
    });
}

async fn rhythm_measure_whole(t: Track) {
    // (why one couldn't be measured, and how long one took: in the log, for the ratings' reviews)
    let note = |w: &str| omnipulse_core::diag::note(&format!("Seamless Mix couldn't measure {}: {w}", t.title));
    let began = std::time::Instant::now();
    let (kind, id) = match &t.src {
        track::Src::YouTube(id) => ("youtube", id.clone()),
        track::Src::Local(p) => ("local", p.clone()),
        track::Src::SoundCloud(id) if !t.go_plus && omnipulse_core::soundcloud::signed_in() => ("soundcloud", id.to_string()),
        track::Src::SoundCloud(_) if t.go_plus => return note("a SoundCloud Go+ song (only its preview can be got)"),
        _ if mix_via_youtube(&t) => match versions::find(&t).await { Some(v) => ("youtube", v.yt.id), None => return note("no YouTube Music version found") },
        _ => return note("nothing it plays from can be measured"),
    };
    // (all of it: up to 20 minutes)
    let (mono, rate) = match omnipulse_core::loudness::mono_start(kind, &id, 1200.0).await { Ok(x) => x, Err(e) => return note(&format!("couldn't get its sound ({e})")) };
    let got = began.elapsed().as_secs_f64();
    let heard = mono.len() as u64 * 1000 / rate.max(1) as u64;
    let r = tokio::task::spawn_blocking(move || omnipulse_core::rhythm::analyse(&mono, rate)).await.ok().flatten();
    if r.is_none() { return note("no beat found"); }
    omnipulse_core::diag::note(&format!("Seamless Mix measured {} in {:.0} s ({got:.0} s getting its sound)", t.title, began.elapsed().as_secs_f64()));
    if let Some(r) = r {
        let m = omnipulse_core::rhythm::Measured { from_ms: 0, to_ms: heard, rhythm: r, v: omnipulse_core::rhythm::VERSION };
        if omnipulse_core::rhythm::known(&t.key).is_none() { omnipulse_core::rhythm::keep(&t.key, m.clone()); }
        omnipulse_core::rhythm::keep(&rhythm_full_key(&t.key), m);
    }
}

/// The next track's loudness measured ahead (seamless mixing on): a mix brings it in at its own match,
/// not a guess that moves once it's measured. Of what the mix plays (its YouTube Music version, if it
/// plays as that); kept as a measurement as it plays is.
fn loud_ahead(shared: &Shared, t: Track) {
    if (sound::loud_step(&shared.borrow().web.prefs) == "off" && !smart_on(&shared.borrow().web.prefs)) || t.is_mix() { return; }
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        let version = if mix_via_youtube(&t) { versions::find(&t).await } else { None };
        let Some((kind, id)) = sound::loud_source(&t, version.as_ref().map(|v| v.yt.id.as_str())) else { return };
        let Ok(a) = omnipulse_core::loudness::analyse(kind, &id).await else { return };
        let Some(lufs) = a["lufs"].as_f64() else { return };
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let key = format!("{kind}:{id}");
            shared.borrow_mut().loud.insert(key.clone(), lufs);
            shared.borrow_mut().analysis.entry(key).or_insert(a);
        }));
    });
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
    // (a mix under way: where the next one comes in is the mix's choice; a part it's come into is
    // left as it is, then and after, never jumped out of with a seek and a fade in)
    if MIX.with(|m| m.borrow().is_some()) || mix_show_pos().is_some() {
        SB.with(|b| b.borrow_mut().2 = key);
        return false;
    }
    SB.with(|b| b.borrow_mut().2 = key);
    let secs = (seg.1 - at.max(seg.0)).round() as u64;
    let ui = shared.borrow().ui.upgrade();
    // a part that runs to the end: the song's over
    if dur > 0 && seg.1 * 1000.0 >= dur as f64 - 1500.0 {
        if let Some(ui) = ui { ui.set_status(format!("Skipped the last {secs} s, which isn't music (marked on SponsorBlock).").into()); }
        step(shared, 1);
        return true;
    }
    let engine = shared.borrow().engine.clone();
    engine.seek((seg.1 * 1000.0) as u64);
    engine.fade(Some(0.0), 1.0, 450);
    if let Some(ui) = ui { ui.set_status(format!("Skipped {secs} s that isn't music (marked on SponsorBlock).").into()); }
    false
}

// ---------- Seamless Mix (crate::smart, its planner: crates/mix) ----------
enum MixPhase {
    /// the next one (its key) being opened
    Opening(String),
    /// handed to the engine, waiting for its moment
    /// (the one playing's tempo, when the mix slows it into the next one's)
    /// (its key shift, if it has one: held through the mix, then bent back to its own key)
    Waiting { key: String, plan: Plan, tempo: omnipulse_audio::Tempo, version: Option<versions::Version>, out: Option<omnipulse_audio::Tempo>, pitch: Option<omnipulse_audio::Tempo> },
    /// coming in (and then easing back to its own tempo); the one going out slowed as it goes, if it is
    Mixing { plan: Plan, tempo: omnipulse_audio::Tempo, eased_from: Option<std::time::Instant>, ease_ms: u64, out: Option<omnipulse_audio::Tempo>, began: std::time::Instant, pitch: Option<omnipulse_audio::Tempo> },
}

thread_local! {
    /// songs' timed lyrics, by track (empty: none that fit), and those being found (for the voices' handoff)
    static MIX_WORDS: RefCell<HashMap<String, Vec<(u64, String)>>> = RefCell::new(HashMap::new());
    static MIX_WORDS_BUSY: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
    static MIX: RefCell<Option<MixPhase>> = const { RefCell::new(None) };
    /// a mix's key shift bending back to the track's own key once the mix is over: (its pitch, where it
    /// was, since when, over how long)
    static PITCH_BACK: RefCell<Option<(omnipulse_audio::Tempo, f64, std::time::Instant, u64)>> = const { RefCell::new(None) };
    /// A Seamless Mix blend riding both songs into one key: the one going out's key handle, and the shift
    /// the next one starts at (semitones); both ridden to the next one's own key by the swap
    static KEY_RIDE: RefCell<Option<(omnipulse_audio::Tempo, i32)>> = const { RefCell::new(None) };

    /// the track the queue moves on to at a mix's moment (start() doesn't load it: it's playing), with
    /// the YouTube Music version it plays as
    static MIXED_IN: RefCell<Option<(String, Option<versions::Version>)>> = const { RefCell::new(None) };
    /// whether the track playing is one a mix can go out of (it's not Spotify's own playback)
    static MIX_OUT_OK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A track the mix plays from YouTube Music: Go+, a catalogue's, a Spotify one (never Spotify's own
/// playback in a mix), or one listening along can't play here
/// A SoundCloud song it only has a preview of (Go+): the full song from YouTube Music (yt-dlp), else from
/// Spotify (playing here, Premium), as the web app's playFullElsewhere
async fn full_elsewhere(t: &Track, h: tokio::runtime::Handle) -> (Result<Option<Box<dyn omnipulse_audio::Source>>, String>, Option<versions::Version>) {
    if let Some(v) = versions::find(t).await {
        return (player::open_source(&Track::youtube(&v.yt), h).await.map(Some), Some(v));
    }
    if sp_plays_here() {
        if let Some(id) = mirror::counterpart(t, "spotify").await {
            let uri = if id.contains(':') { id } else { format!("spotify:track:{id}") };
            let sp = Track { src: track::Src::Spotify(uri.clone()), key: format!("sp:{}", uri.rsplit(':').next().unwrap_or("")), ..t.clone() };
            omnipulse_core::diag::note(&format!("Go+: \"{}\" plays from Spotify, as there's no YouTube Music version", t.title));
            return (player::open_source(&sp, h).await.map(Some), None);
        }
    }
    (Err("SoundCloud only has a preview of it, and there's no full version on YouTube Music or Spotify".to_string()), None)
}

fn mix_via_youtube(t: &Track) -> bool {
    t.go_plus || matches!(t.src, track::Src::Catalog(_) | track::Src::Spotify(_)) || listen_needs_version(t)
}

// ---------- Seamless Mix's cost, audited (the engine's work and the planning; never measuring songs,
// which is always on) ----------
#[derive(Default)]
struct MixAudit {
    /// planning: ns, times
    plan_ns: u64,
    plans: u64,
    /// the mix under way: the engine's work when it began, its kind, what it's from and to
    began: Option<(omnipulse_audio::Perf, &'static str, String)>,
    /// each mix: its kind, its share of a core while it ran, how long
    mixes: Vec<serde_json::Value>,
}

thread_local! {
    static MIX_AUDIT: RefCell<MixAudit> = RefCell::new(MixAudit::default());
}

fn mix_kind(p: &Plan) -> &'static str {
    if p.tape_stop { "tape stop" } else if p.style == Some(smart::ROLL) { smart::ROLL } else { "smart blend" }
}

fn audit_began(shared: &Shared, plan: &Plan, what: String) {
    let perf = shared.borrow().engine.perf();
    MIX_AUDIT.with(|a| a.borrow_mut().began = Some((perf, mix_kind(plan), what)));
}

/// A mix over: what it cost, noted (the log, state/native-mix-audit.json, Settings > Sound)
fn audit_over(shared: &Shared) {
    let (engine, ui) = { let a = shared.borrow(); (a.engine.clone(), a.ui.clone()) };
    let now = engine.perf();
    let rate = engine.rate();
    MIX_AUDIT.with(|a| {
        let mut a = a.borrow_mut();
        let Some((was, kind, what)) = a.began.take() else { return };
        let d = (now.mixing.0.saturating_sub(was.mixing.0), now.mixing.1.saturating_sub(was.mixing.1));
        let Some(load) = omnipulse_audio::Perf::load(d, rate) else { return };
        let secs = d.1 as f64 / rate.max(1) as f64;
        omnipulse_core::diag::note(&format!("Seamless Mix audit: {kind} mix ({what}): {:.2}% of a core over {secs:.1} s", load * 100.0));
        a.mixes.push(serde_json::json!({ "kind": kind, "what": what, "load": load, "secs": secs, "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) }));
        let n = a.mixes.len();
        if n > 50 { a.mixes.drain(..n - 50); }
        let pct = |x: (u64, u64)| omnipulse_audio::Perf::load(x, rate).map(|l| l * 100.0);
        let plan_ms = if a.plans > 0 { a.plan_ns as f64 / a.plans as f64 / 1e6 } else { 0.0 };
        let file = serde_json::json!({
            "note": "Seamless Mix's cost: the audio engine's work (share of one core) mixing, through the stretch (a mix's tempo, and after), and playing as it is; and the planning. Measuring songs isn't counted.",
            "mixing": pct(now.mixing), "stretched": pct(now.stretched), "plain": pct(now.plain), "planMs": plan_ms, "plans": a.plans, "mixes": a.mixes,
        });
        if let Ok(d) = omnipulse_core::data_dir() { let _ = std::fs::write(d.join("state").join("native-mix-audit.json"), serde_json::to_string_pretty(&file).unwrap_or_default()); }
        if let Some(ui) = ui.upgrade() {
            let f = |x: Option<f64>| x.map_or("not yet".to_string(), |v| format!("{v:.1}% of a core"));
            ui.set_mix_audit(format!("Mixing: {} ({} mixes, the last {:.1}%). Through the stretch (a mix's tempo, and after it): {}. Playing as it is: {}. Planning: {plan_ms:.2} ms a time. Measuring songs isn't counted. Kept in state\\native-mix-audit.json.", f(pct(now.mixing)), a.mixes.len(), load * 100.0, f(pct(now.stretched)), f(pct(now.plain))).into());
        }
    });
}

/// Each tick: a mix planned and got ready as the one playing nears its moment; eased back after
/// A key shift's only for the mix (the two keys together while both play): bent back to the track's own
/// key over a few beats once the one going out has gone, never left on for the rest of it
fn pitch_back_start(pitch: &Option<omnipulse_audio::Tempo>, ms: u64) {
    if let Some(p) = pitch {
        if PITCH_BACK.with(|b| b.borrow().is_none()) && (p.get() - 1.0).abs() > 1e-6 {
            PITCH_BACK.with(|b| *b.borrow_mut() = Some((p.clone(), p.get(), std::time::Instant::now(), ms.max(1))));
        }
    }
}

fn pitch_back_tick() {
    PITCH_BACK.with(|b| {
        let mut b = b.borrow_mut();
        let Some((p, from, since, ms)) = b.as_ref() else { return };
        let x = (since.elapsed().as_millis() as f64 / *ms as f64).min(1.0);
        // (eased in and out: in semitones, evenly, so it's a bend, not a slide that rushes at either end)
        let s = x * x * (3.0 - 2.0 * x);
        p.set(from.powf(1.0 - s));
        if x >= 1.0 { p.set(1.0); *b = None; }
    });
}

fn mix_tick(shared: &Shared, pos: u64, dur: u64) {
    pitch_back_tick();
    // coming in: once the crossfade's over, back to its own tempo over a few beats
    let easing = MIX.with(|m| {
        let mut m = m.borrow_mut();
        if let Some(MixPhase::Mixing { plan, tempo, eased_from, ease_ms, out, began, pitch }) = m.as_mut() {
            let engine = shared.borrow().engine.clone();
            // (its key shift only matters while both are heard: back to its own key over the mix's last
            // quarter, as the one going out fades, so it's at its own pitch by the end, never after)
            // (a key ride: the next one from the playing one's key to its own, the one playing from its own to
            // the next one's, together, eased as the tempo is, the two a semitone or two apart all the way:
            // in key with each other throughout, the next one in its own key by the swap)
            if let Some((ap, s)) = KEY_RIDE.with(|k| k.borrow().clone()) {
                let x = (began.elapsed().as_millis() as f64 / plan.swap_ms.max(1) as f64).min(1.0);
                let e = x * x * (3.0 - 2.0 * x);
                if let Some(bp) = pitch { bp.set(2f64.powf(s as f64 * (1.0 - e) / 12.0)); }
                ap.set(2f64.powf(-(s as f64) * e / 12.0));
            } else if began.elapsed().as_millis() as u64 >= plan.ms * 3 / 4 { pitch_back_start(pitch, (plan.ms / 4).max(1)); }
            let engine = engine;
            // (the one going out slowed into the next one's tempo as far through as the mix is, and the next
            // one with it, from the other's tempo to its own: at its own by the end, nothing to ease back)
            if let Some(o) = out.clone() {
                let x = began.elapsed().as_millis() as f64 / plan.ramp_ms.max(1) as f64;
                o.set(omnipulse_mix::plan::out_ramp(plan.out_tempo, x));
                tempo.set(omnipulse_mix::plan::in_ramp(plan.out_tempo, x));
                if engine.crossfading() { return true; }
                tempo.set(1.0);
                pitch_back_start(pitch, *ease_ms);
                KEY_RIDE.with(|k| *k.borrow_mut() = None);
                *m = None;
                let s3 = shared.clone();
                slint::Timer::single_shot(std::time::Duration::from_millis(MIX_HEARD_MS), move || mix_show_end(&s3));
                let s2 = shared.clone();
                slint::Timer::single_shot(std::time::Duration::ZERO, move || audit_over(&s2));
                return true;
            }
            if engine.crossfading() { return true; }
            pitch_back_start(pitch, *ease_ms);
            if eased_from.is_none() {
                let s3 = shared.clone();
                slint::Timer::single_shot(std::time::Duration::from_millis(MIX_HEARD_MS), move || mix_show_end(&s3));
            }
            let since = *eased_from.get_or_insert_with(std::time::Instant::now);
            let t = since.elapsed().as_millis() as u64;
            tempo.set(omnipulse_mix::plan::ease(plan.rate, t, *ease_ms));
            if t >= *ease_ms { *m = None; }
            // (the crossfade itself over: what it cost)
            if MIX_AUDIT.with(|a| a.borrow().began.is_some()) {
                let s2 = shared.clone();
                slint::Timer::single_shot(std::time::Duration::ZERO, move || audit_over(&s2));
            }
            return true;
        }
        m.is_some()
    });
    if easing { return; }
    // (a song still loading: the engine's place is the one before's)
    if shared.borrow().ui.upgrade().is_some_and(|u| u.get_track_loading()) { return; }
    let ((), listening_now, next, cur) = {
        let a = shared.borrow();
        let next = if a.repeat == "one" { None } else { a.queue.qi.and_then(|i| a.queue.items.get(i + 1)).map(|q| q.t.clone()) };
        ((), listening().is_some(), next, a.queue.current().cloned())
    };
    let (Some(next), Some(cur)) = (next, cur) else { return };
    // (OMNIPULSE_MIX_DEBUG: why it isn't mixing, each second)
    let pair = (cur.key.clone(), next.key.clone());
    let why = |w: &str| {
        // (kept: if this pair changes over unmixed, a rating of it says why)
        MIX_WHY.with(|m| *m.borrow_mut() = Some((pair.0.clone(), pair.1.clone(), w.to_string())));
        if std::env::var_os("OMNIPULSE_MIX_DEBUG").is_some() && pos % 1000 < 260 { eprintln!("mix debug at {pos}: {w}"); }
    };
    // (a Jam going: its queue plays as it is, unmixed, till it ends)
    let listening_now = listening_now || room_kind().as_deref() == Some("jam");
    // (Smart Transitions' blends on the beat: the same way in, its own plan)
    let smart = smart_blends_on(&shared.borrow().web.prefs);
    if !smart || listening_now || !MIX_OUT_OK.with(|c| c.get()) || dur == 0 || next.is_mix() || cur.is_mix() { why(&format!("on {smart}, listening {listening_now}, out ok {}, dur {dur}, a set or podcast (next {}, playing {}): they play, unmixed", MIX_OUT_OK.with(|c| c.get()), next.is_mix(), cur.is_mix())); return; }
    // (Stream-Safe Music: never mixed into one it doesn't allow; the usual way skips it)
    if !streamsafe::allowed(&shared.borrow().web.prefs, &next) { why("Stream-Safe Music doesn't allow the next one"); return; }
    let plan = {
        let Some((a, b)) = smart_pair(&shared.borrow(), dur) else { return };
        let Some(plan) = smart_decide(shared, &a, &b, pos, false) else { return };
        // (sought past its moment: a fade instead)
        if pos > plan.at_ms { SMART_PAIR.with(|s| { if let Some(x) = s.borrow_mut().as_mut() { x.2 = None; x.3 = "sought past the blend".into(); } }); return; }
        if pos + 7000 < plan.at_ms { return; }
        plan
    };
    MIX.with(|m| *m.borrow_mut() = Some(MixPhase::Opening(next.key.clone())));
    let cur_key = cur.key.clone();
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
            move || -> Result<(Box<dyn omnipulse_audio::Source>, Option<omnipulse_audio::Tempo>), String> {
                let mut st = omnipulse_audio::Stretched::shifted(src?, tempo, semitones)?;
                omnipulse_audio::Source::seek(&mut st, start)?;
                let pitch = (semitones != 0).then(|| st.pitch());
                Ok((Box::new(st), pitch))
            }
        }).await.map_err(|e| e.to_string()).and_then(|r| r);
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let still = MIX.with(|m| matches!(&*m.borrow(), Some(MixPhase::Opening(k)) if *k == next.key));
            if !still { return; }
            match ready {
                Ok((src, pitch)) => {
                    let what = sound::loud_source(&next, version.as_ref().map(|v| v.yt.id.as_str()));
                    let known = what.as_ref().and_then(|(s, id)| shared.borrow().loud.get(&format!("{s}:{id}")).copied());
                    let db = sound::loud_db(&prefs, &next, known, version.is_some());
                    // (slowed into the next one's tempo: from here, the one playing goes through the stretch, as it was until the mix)
                    // (and a key ride: the one playing's key able to move too)
                    let ride = smart && plan.semitones != 0 && pitch.is_some();
                    let out = (plan.out_tempo != 1.0 || ride).then(|| omnipulse_audio::Tempo::new(1.0));
                    KEY_RIDE.with(|k| *k.borrow_mut() = None);
                    if let Some(o) = &out {
                        if ride {
                            let ap = omnipulse_audio::Tempo::new(1.0);
                            shared.borrow().engine.stretch_playing_bent(o.clone(), ap.clone());
                            KEY_RIDE.with(|k| *k.borrow_mut() = Some((ap, plan.semitones)));
                        } else { shared.borrow().engine.stretch_playing(o.clone()); }
                    }
                    let handoff = smart::handoff(&plan, set_row_on(&prefs, "smartVoices", true) && smart_both_sing(&cur_key, &next.key, (plan.at_ms, plan.at_ms + plan.ms), (plan.start_ms, plan.start_ms + (plan.ms as f64 * plan.rate) as u64)));
                    shared.borrow().engine.mix_at(src, db, plan.at_ms, plan.ms, Some(handoff));
                    MIX.with(|m| *m.borrow_mut() = Some(MixPhase::Waiting { key: next.key.clone(), plan, tempo, version, out, pitch }));
                }
                // (it couldn't be got ready: the usual way, then)
                Err(e) => {
                    omnipulse_core::diag::note(&format!("Seamless Mix couldn't get the next song ready: {e}"));
                    MIX.with(|m| *m.borrow_mut() = None);
                    // (Smart Transitions: a fade instead)
                    if smart { SMART_PAIR.with(|s| { if let Some(x) = s.borrow_mut().as_mut() { x.2 = None; x.3 = format!("couldn't be got ready: {e}"); } }); }
                }
            }
        }));
    });
}

// ---------- Seamless Mix feedback (the Stream Deck's Good Transition / Bad Transition keys) ----------
// Each press is a line in state/mix-feedback.jsonl: the verdict, how long after the transition began,
// and how it was mixed (both songs as measured, the plan, the settings). No sound is kept, and nothing's
// written without a press: no rating is never taken as a good one.
thread_local! {
    /// the last transition: what it was, and when it began
    static LAST_MIX: RefCell<Option<(serde_json::Value, std::time::Instant)>> = const { RefCell::new(None) };
    /// the last song change of any kind (mixed or not), and the one playing before it
    static LAST_CHANGE: RefCell<Option<SongChange>> = const { RefCell::new(None) };
    static LAST_PLAYING: RefCell<Option<Track>> = const { RefCell::new(None) };
    /// how the next change comes about: "mixed", "ended" (on to the next, unmixed), else "picked" (a
    /// skip, a click, a queue change)
    static CHANGE_HOW: std::cell::Cell<&'static str> = const { std::cell::Cell::new("picked") };
    /// the mixer's last word on the pair coming up (the one playing, the next one): why it isn't mixing
    static MIX_WHY: RefCell<Option<(String, String, String)>> = const { RefCell::new(None) };
}

struct SongChange {
    from: Option<Track>,
    to: Track,
    how: &'static str,
    why: Option<String>,
    at: std::time::Instant,
}

/// One side of a transition, as it was measured (no sound: its numbers)
fn mix_side(t: &Track) -> serde_json::Value {
    let m = rhythm_full(&t.key).or_else(|| omnipulse_core::rhythm::known(&t.key));
    serde_json::json!({
        "key": t.key, "title": t.title, "artist": t.artist, "platform": t.plat(), "ms": t.ms,
        "viaYouTube": mix_via_youtube(t),
        "measured": m.map(|m| serde_json::json!({
            "whole": rhythm_full(&t.key).is_some(),
            "fromMs": m.from_ms, "toMs": m.to_ms,
            "bpm": m.rhythm.bpm, "tempoConfidence": m.rhythm.tempo_confidence,
            "key": m.rhythm.key, "camelot": m.rhythm.camelot, "keyConfidence": m.rhythm.key_confidence,
            "periodMs": m.rhythm.period_ms, "downbeat": m.rhythm.downbeat,
            // (how sure its downbeat is, as a whole and at its start and end; Beat This!'s tempo)
            "downbeatConfidence": m.rhythm.downbeat_confidence, "headDownbeat": m.rhythm.head_downbeat, "tailDownbeat": m.rhythm.tail_downbeat, "modelBpm": m.rhythm.model_bpm,
            // (each bar's loudness, dB, rounded: where its intro, drops and outro are)
            "barDb": m.rhythm.bar_db.iter().map(|x| (*x as f64 * 10.0).round() / 10.0).collect::<Vec<_>>(),
        })),
    })
}

fn mix_record(a: &App, plan: &Plan, from: Option<&Track>, to: Option<&Track>) -> serde_json::Value {
    let p = &a.web.prefs;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    serde_json::json!({
        "id": now,
        "kind": mix_kind(plan),
        "smart": smart_on(p),
        "plan": plan,
        "from": from.map(mix_side),
        "to": to.map(mix_side),
        "settings": { "smartBlend": set_row_on(p, "smartBlend", true), "smartKeys": set_row_on(p, "smartKeys", true), "smartTape": set_row_on(p, "smartTape", false), "smartRoll": set_row_on(p, "smartRoll", false), "smartVoices": set_row_on(p, "smartVoices", true), "smartMax": p["smartMax"].as_u64().unwrap_or(10) },
        "version": env!("CARGO_PKG_VERSION"),
    })
}

/// The last transition rated: a line in state/mix-feedback.jsonl, or why it couldn't be
fn mix_feedback(shared: &Shared, good: bool) -> Result<String, &'static str> {
    // (what's rated: the last song change. Mixed, its transition; not, the change as it was, and why the
    // mixer left it, so a rating never lands on an older transition)
    let change = LAST_CHANGE.with(|c| c.borrow().as_ref().map(|c| (c.to.key.clone(), c.how, c.at)));
    let mixed = LAST_MIX.with(|l| l.borrow().clone());
    let (rec, began) = match (change, mixed) {
        (Some((to, how, at)), mixed) if how != "mixed" || mixed.as_ref().is_none_or(|(r, _)| r["to"]["key"].as_str() != Some(to.as_str())) => {
            let a = shared.borrow();
            let rec = LAST_CHANGE.with(|c| c.borrow().as_ref().map(|c| {
                let p = &a.web.prefs;
                serde_json::json!({
                    "id": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0).saturating_sub(c.at.elapsed().as_millis() as u64),
                    "kind": if c.why.as_deref().is_some_and(|w| w.starts_with("smart fade")) { "smart fade" } else { "unmixed" },
                    "smart": smart_on(p),
                    // ("ended": played to its end and on to the next unmixed; "picked": a skip or a click, by hand)
                    "how": how,
                    "whyNotMixed": c.why,
                    "mixOn": smart_on(p),
                    "plan": null,
                    "from": c.from.as_ref().map(mix_side),
                    "to": mix_side(&c.to),
                    "settings": { "smartBlend": set_row_on(p, "smartBlend", true), "smartKeys": set_row_on(p, "smartKeys", true), "smartTape": set_row_on(p, "smartTape", false), "smartRoll": set_row_on(p, "smartRoll", false), "smartVoices": set_row_on(p, "smartVoices", true), "smartMax": p["smartMax"].as_u64().unwrap_or(10) },
                    "version": env!("CARGO_PKG_VERSION"),
                })
            })).ok_or("No song change to rate yet.")?;
            (rec, at)
        }
        (_, Some(m)) => m,
        _ => return Err("No transition to rate yet."),
    };
    let line = serde_json::json!({
        "verdict": if good { "good" } else { "bad" },
        // (what the two mean, as the one rating uses them: "bad" isn't "it sounded bad")
        "meaning": if good { "good: keep mixing pairs like this this way" } else { "bad: there's a better way to mix this pair (not necessarily a bad transition)" },
        "at": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0),
        // (rated late, it may be of a later one by ear: how long after this one it was)
        "secsAfter": began.elapsed().as_secs_f64().round(),
        "mix": rec,
    });
    // (a self-test writes to its own folder, never your ratings)
    let dir = match std::env::var_os("OMNIPULSE_SELFTEST") { Some(d) => std::path::PathBuf::from(d), None => omnipulse_core::data_dir().map_err(|_| "Couldn't find the data folder.")?.join("state") };
    let _ = std::fs::create_dir_all(&dir);
    use std::io::Write as _;
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("mix-feedback.jsonl")).map_err(|_| "Couldn't write the feedback file.")?;
    writeln!(f, "{line}").map_err(|_| "Couldn't write the feedback file.")?;
    let what = format!("{} into {}", rec["from"]["title"].as_str().unwrap_or("?"), rec["to"]["title"].as_str().unwrap_or("?"));
    let _ = &began;
    // (all of them: those sent and cleared too)
    let n = ["mix-feedback.jsonl", "mix-feedback-tally.jsonl"].iter().map(|f| std::fs::read_to_string(dir.join(f)).map(|s| s.lines().count()).unwrap_or(0)).sum::<usize>();
    let note = format!("Transition rated {}: {what} ({}). {n} ratings so far.", if good { "good" } else { "bad" }, rec["kind"].as_str().unwrap_or(""));
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.clone().into()); }
    Ok(note)
}

/// Your ratings not yet sent, sent to the developer (the relay keeps them), once each launch: all of them
/// after the last one that got there (state/mix-feedback-sent: how many lines have), so if it couldn't be
/// reached they go next launch (200 at a time)
fn mix_ratings_send(shared: &Shared, dir: std::path::PathBuf) {
    let rt = shared.borrow().rt.clone();
    rt.spawn(async move {
        let Ok(text) = std::fs::read_to_string(dir.join("mix-feedback.jsonl")) else { return };
        let sent: usize = std::fs::read_to_string(dir.join("mix-feedback-sent")).ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0);
        let lines: Vec<serde_json::Value> = text.lines().skip(sent).take(200).filter_map(|l| serde_json::from_str(l).ok()).collect();
        if lines.is_empty() { return; }
        let install = install_id(&dir);
        let n = text.lines().skip(sent).take(200).count();
        let r = reqwest::Client::new()
            .post(format!("{}/api/mix-ratings", share::RELAY_URL))
            .json(&serde_json::json!({ "install": install, "lines": lines }))
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await;
        if !r.is_ok_and(|r| r.status().is_success()) { return; }
        // (they're there: cleared from here, on the window's thread, as ratings are written there)
        let _ = slint::invoke_from_event_loop(move || mix_ratings_clear(&dir, sent + n));
    });
}

/// The first `upto` ratings (sent) taken out of mix-feedback.jsonl, any rated since kept; each one's kind and
/// verdict kept in mix-feedback-tally.jsonl (all Smart Transitions needs of them: smart_ratings)
fn mix_ratings_clear(dir: &std::path::Path, upto: usize) {
    let file = dir.join("mix-feedback.jsonl");
    let Ok(text) = std::fs::read_to_string(&file) else { return };
    let lines: Vec<&str> = text.lines().collect();
    let upto = upto.min(lines.len());
    let tally: String = lines[..upto].iter().filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .map(|v| format!("{}\n", serde_json::json!({ "kind": v["mix"]["kind"], "verdict": v["verdict"] }))).collect();
    use std::io::Write as _;
    let Ok(mut t) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("mix-feedback-tally.jsonl")) else { return };
    if t.write_all(tally.as_bytes()).is_err() { return; }
    let rest: String = lines[upto..].iter().map(|l| format!("{l}\n")).collect();
    if std::fs::write(&file, rest).is_ok() { let _ = std::fs::write(dir.join("mix-feedback-sent"), "0"); }
}

// ---------- what's shown while a mix is under way ----------
// The one going out is what's playing until the mix is over: the player bar, Now Playing (its cover,
// lyrics, place in it) and the queue's Now Playing stay its; the one coming in is lit in the queue. Then
// everything's the new one's at once (its cover and lyrics, kept back meanwhile).
struct MixShow {
    prev_qi: usize,
    /// where the one going out was when the mix began, and when that was (its place since: counted on)
    from_ms: u64,
    began: std::time::Instant,
    dur: u64,
    /// its lyrics (timed), for following along
    lyrics: Vec<player::Line>,
    /// the new one's cover and lyrics, arrived meanwhile: shown once it's over
    pending: Option<Box<dyn FnOnce()>>,
}

/// (the engine makes its sound this far ahead of what's heard: the mix is over, as heard, that much later)
const MIX_HEARD_MS: u64 = 400;

thread_local! {
    /// the songs on an artist's page whose versions are open beneath them (by key)
    static VER_OPEN: RefCell<std::collections::HashSet<String>> = RefCell::new(Default::default());
    /// a Last.fm scrobble sent for what's playing
    static SCROBBLED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// paused a moment (0.6 s): your own accent back, not the cover's (as the web app's)
    static ART_PAUSED: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// the visualiser's: its 30-a-second timer while it runs, and the bands it shows
    static VIZ: RefCell<Option<(slint::Timer, Rc<VecModel<f32>>)>> = const { RefCell::new(None) };
    /// what the loading view shows: its change count and page
    static LOAD_SHOWN: RefCell<(u64, String)> = const { RefCell::new((u64::MAX, String::new())) };
    static PAUSED_SINCE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// a crossfade's end (Fade Between Tracks), for the accent's pulse meanwhile
    static XFADE_PULSE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// the tracks Ctrl+C copied, for Ctrl+V
    static PICK_CLIP: RefCell<Vec<Track>> = const { RefCell::new(Vec::new()) };
    /// the last place opened again at start (once)
    static LAST_PLACE_DONE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static SESSION_SAVED: std::cell::Cell<std::time::Instant> = std::cell::Cell::new(std::time::Instant::now());
}

thread_local! {
    static MIX_SHOW: RefCell<Option<MixShow>> = const { RefCell::new(None) };
    static SHOWN_DUR: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// The one going out's place and length, while a mix is under way (None: no mix being shown)
fn mix_show_pos() -> Option<(u64, u64)> {
    // (from where the mix began in it, as heard: the engine is that far ahead of the speakers)
    MIX_SHOW.with(|m| m.borrow().as_ref().map(|x| ((x.from_ms + (x.began.elapsed().as_millis() as u64).saturating_sub(MIX_HEARD_MS)).min(x.dur), x.dur)))
}

/// The queue's row shown as playing while a mix is under way (the one going out)
fn mix_show_qi() -> Option<usize> {
    MIX_SHOW.with(|m| m.borrow().as_ref().map(|x| x.prev_qi))
}

/// The accent's slow fade to the next cover's colours and back, while a mix is under way (album-art
/// colours on only: picked colours stay as they are)
fn mix_pulse(shared: &Shared, ui: &AppWindow, px: &slint::SharedPixelBuffer<slint::Rgba8Pixel>) {
    let prefs = shared.borrow().web.prefs.clone();
    if !integrations::set(&prefs, "artAccent") { return; }
    let (step, hue, strength) = look::from_prefs(&prefs);
    let light = look::palette(step, hue, strength).light;
    let (a1, a2) = integrations::accents(px);
    let Some(first) = look::Rgb::parse(&a1) else { return };
    let c1 = look::tune_art(first, light);
    let c2 = look::Rgb::parse(&a2).map(|c| look::tune_art(c, light)).unwrap_or_else(|| look::tune_art_shade(first, light));
    let t = ui.global::<Theme>();
    t.set_accent_next(c1.slint());
    t.set_accent_2_next(c2.slint());
    t.set_pulsing(true);
}

/// The mix over: the new one shown as playing, everywhere
fn mix_show_end(shared: &Shared) {
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.global::<Theme>().set_pulsing(false); }
    let Some(x) = MIX_SHOW.with(|m| m.borrow_mut().take()) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let Some(t) = shared.borrow().queue.current().cloned() else { return };
    let (art, big) = (ui.get_now_art(), ui.get_now_art_big());
    show_now(&ui, &t);
    // (its cover and lyrics, if they came meanwhile; else they're shown as they come)
    match x.pending {
        Some(f) => f(),
        None => { ui.set_now_art(art); ui.set_now_art_big(big); ui.set_lyrics_note("Looking for lyrics...".into()); }
    }
    paint_queue(&mut shared.borrow_mut());
    push_out(shared, false);
}

/// A mix's moment came: the next track is coming in (the queue moves on, without loading it again)
fn mix_started(shared: &Shared) {
    let Some(MixPhase::Waiting { key, plan, tempo, version, out, pitch }) = MIX.with(|m| m.borrow_mut().take()) else { return };
    // (where the mix begins in the one going out: its place, shown, from there)
    let plan_at = plan.at_ms;
    let at = shared.borrow().queue.qi.map(|i| i + 1).filter(|&n| shared.borrow().queue.items.get(n).is_some_and(|q| q.t.key == key));
    let Some(n) = at else { omnipulse_core::diag::note("Seamless Mix: the next song changed before the mix started"); return };
    // (each beat of the one coming in, at its own tempo, for easing back over sixteen of them)
    let ease_ms = omnipulse_core::rhythm::known(&key).map(|m| (16.0 * m.rhythm.period_ms) as u64).unwrap_or(8000);
    // (Smart Transitions' blends: back to its own tempo slowly enough not to be heard)
    let ease_ms = if smart_on(&shared.borrow().web.prefs) && plan.beatmatched {
        let left = shared.borrow().queue.items.get(n).map(|q| q.t.ms).unwrap_or(0).saturating_sub(plan.start_ms);
        smart::ease_ms(left)
    } else { ease_ms };
    audit_began(shared, &plan, format!("{} into {}", shared.borrow().queue.current().map(|t| t.title.clone()).unwrap_or_default(), shared.borrow().queue.items.get(n).map(|q| q.t.title.clone()).unwrap_or_default()));
    // (what it was, for rating it from the Stream Deck)
    let rec = {
        let a = shared.borrow();
        mix_record(&a, &plan, a.queue.current(), a.queue.items.get(n).map(|q| &q.t))
    };
    LAST_MIX.with(|l| *l.borrow_mut() = Some((rec, std::time::Instant::now())));
    // (its crossfader: the one playing into the next)
    let pair = { let a = shared.borrow(); a.queue.current().cloned().zip(a.queue.items.get(n).map(|q| q.t.clone())) };
    if let Some((from, to)) = pair { xf_begin(shared, from, to, plan.ms); }
    if let Some(ui) = shared.borrow().ui.upgrade() {
        match plan.style {
            Some("roll") => ui.set_status("Roll.".into()),
            _ => {}
        }
    }
    // (the slowing of the one going out and the next one with it, kept going on a thread of its own as well:
    // the same ramp mix_tick sets, so a busy window can't leave the two part-way, the next one stuck slowed)
    if let Some(o) = out.clone() {
        let (t, target, ramp) = (tempo.clone(), plan.out_tempo, plan.ramp_ms.max(1));
        let began = std::time::Instant::now();
        std::thread::spawn(move || loop {
            let x = began.elapsed().as_millis() as f64 / ramp as f64;
            o.set(omnipulse_mix::plan::out_ramp(target, x));
            t.set(omnipulse_mix::plan::in_ramp(target, x));
            if x >= 1.0 { break; }
            std::thread::sleep(std::time::Duration::from_millis(20));
        });
    }
    MIX.with(|m| *m.borrow_mut() = Some(MixPhase::Mixing { plan, tempo, eased_from: None, ease_ms, out, began: std::time::Instant::now(), pitch }));
    MIXED_IN.with(|m| *m.borrow_mut() = Some((key, version)));
    CHANGE_HOW.with(|h| h.set("mixed"));
    // (the one going out, as it's shown now: kept shown until the mix is over)
    let keep = {
        let a = shared.borrow();
        let ui = a.ui.upgrade();
        match (a.queue.current().cloned(), a.queue.qi, ui) {
            (Some(prev), Some(qi), Some(ui)) => Some((prev, qi, plan_at, SHOWN_DUR.with(|d| d.get()).max(1), a.lyrics.clone(), (ui.get_now_art(), ui.get_now_art_big()), a.lyric_lines.iter().collect::<Vec<_>>(), ui.get_lyric_on(), ui.get_lyrics_note(), ui.get_lyrics_from())),
            _ => None,
        }
    };
    shared.borrow_mut().queue.qi = Some(n);
    start(shared);
    if let Some((prev, prev_qi, from_ms, dur, lyrics, (art, big), lines, on, note, from)) = keep {
        MIX_SHOW.with(|m| *m.borrow_mut() = Some(MixShow { prev_qi, from_ms, began: std::time::Instant::now(), dur, lyrics, pending: None }));
        if let Some(ui) = shared.borrow().ui.upgrade() {
            show_now(&ui, &prev);
            ui.set_now_art(art);
            ui.set_now_art_big(big);
            ui.set_track_loading(false);
            shared.borrow().lyric_lines.set_vec(lines);
            ui.set_lyric_on(on);
            ui.set_lyrics_note(note);
            ui.set_lyrics_from(from);
        }
        paint_queue(&mut shared.borrow_mut());
    }
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
    // (Seamless Mix: its intro and first sound, measured ahead)
    if smart_on(&shared.borrow().web.prefs) {
        if smart_same_source(&next) { rhythm_ahead(shared, next.clone()); }
        loud_ahead(shared, next.clone());
    }
    let yt_id = |t: &Track| if let track::Src::YouTube(id) = &t.src { Some(id.clone()) } else { None };
    // (a Spotify song from YouTube Music: with a free account, or Seamless Mix on)
    let sp_yt = matches!(next.src, track::Src::Spotify(_)) && (!sp_plays_here() || smart_on(&shared.borrow().web.prefs));
    let via_yt = next.go_plus || matches!(next.src, track::Src::Catalog(_)) || listen_needs_version(&next) || sp_yt;
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
    paint_wave(ui, &t.key);
    let trimmed = SHARED.with(|s| s.borrow().as_ref().and_then(|sh| sh.try_borrow().ok().map(|a| { let (st, en) = trim_of(&a.web.prefs, &t.key); st > 0 || en.is_some() }))).unwrap_or(false);
    ui.set_now_trimmed(trimmed);
    ui.set_now_title(t.title.clone().into());
    ui.set_now_artist(t.artist.clone().into());
    ui.set_now_album(t.album.clone().into());
    let names: Vec<slint::SharedString> = t.artist.split([',', '&']).map(|x| x.trim()).filter(|x| !x.is_empty()).map(Into::into).collect();
    ui.set_now_artists(ModelRc::from(Rc::new(VecModel::from(names))));
    // (genre, year and plays, as the web app's npMeta)
    let mut bits: Vec<String> = Vec::new();
    if !t.genre.is_empty() { bits.push(t.genre.clone()); }
    if t.date.len() >= 4 && t.date[..4].chars().all(|c| c.is_ascii_digit()) { bits.push(t.date[..4].to_string()); }
    if t.plays > 0 && t.plat() != "spotify" { bits.push(format!("{} plays", thousands(t.plays))); }
    ui.set_now_meta(bits.join(", ").into());
    let apart = !t.uploader.is_empty() && omnipulse_core::ranking::artist_key(&t.uploader) != omnipulse_core::ranking::artist_key(t.artist.split([',', '&']).next().unwrap_or(""));
    ui.set_now_uploader(if apart { t.uploader.clone().into() } else { "".into() });
    ui.set_now_rhythm(rhythm_line(&t.key).into());
    ui.set_now_plat(t.plat().into());
    ui.set_now_open(if t.web_link().is_empty() { "".into() } else { match t.plat() { "spotify" => "Open in Spotify", "youtube" => "Open in YouTube Music", "musicbrainz" => "Open on MusicBrainz", "deezer" => "Open on Deezer", "local" => "", _ => "Open on SoundCloud" }.into() });
    ui.set_now_art(slint::Image::default());
    ui.set_now_art_big(slint::Image::default());
    now_desc(ui, t);
}

thread_local! {
    /// SoundCloud songs' descriptions, as they've been fetched (by track)
    static DESCS: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// Now Playing's description (the web app's npDesc): a SoundCloud song's, its first 600 characters
fn now_desc(ui: &AppWindow, t: &Track) {
    ui.set_now_desc("".into());
    let track::Src::SoundCloud(id) = &t.src else { return };
    if let Some(d) = DESCS.with(|m| m.borrow().get(&t.key).cloned()) { ui.set_now_desc(d.into()); return; }
    let (id, key, weak) = (*id, t.key.clone(), ui.as_weak());
    // (later, once whoever called this is done with the app: it can be in the middle of changing it)
    slint::Timer::single_shot(std::time::Duration::ZERO, move || SHARED.with(|s| {
        let Some(sh) = s.borrow().clone() else { return };
        let Ok(app) = sh.try_borrow() else { return };
        app.rt.spawn(async move {
            let v = omnipulse_core::soundcloud::api_get(format!("/tracks/{id}")).await.unwrap_or_default();
            let d: String = v["description"].as_str().unwrap_or("").trim().chars().take(600).collect();
            let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                DESCS.with(|m| m.borrow_mut().insert(key.clone(), d.clone()));
                let still = s.borrow().as_ref().is_some_and(|sh| sh.try_borrow().is_ok_and(|a| a.queue.current().is_some_and(|c| c.key == key)));
                if still { ui.set_now_desc(d.into()); }
            }));
        });
    }));
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


// ---------- the chatbot's Actions page (crate::flows: what's in an action; the plugin runs them) ----------
thread_local! {
    /// the actions as they're being changed, and which is picked (-1: none)
    static FLOW: RefCell<(Vec<serde_json::Value>, i32)> = const { RefCell::new((Vec::new(), -1)) };
    /// (the Actions list: its folded groups, and what's searched for)
    static FLOW_FIND: RefCell<String> = const { RefCell::new(String::new()) };
    /// (the list as it was before each change to it (a delete, a paste...), for Ctrl+Z; and what was undone,
    /// for Ctrl+Y)
    static FLOW_UNDO: RefCell<Vec<(Vec<serde_json::Value>, i32)>> = const { RefCell::new(Vec::new()) };
    /// (what the Actions page's pickers list, from the Suite: your rewards on Twitch and Kick, OBS's scenes and
    /// sources)
    static PICKS: RefCell<serde_json::Value> = const { RefCell::new(serde_json::Value::Null) };
    static FLOW_REDO: RefCell<Vec<(Vec<serde_json::Value>, i32)>> = const { RefCell::new(Vec::new()) };
    /// a save waiting (a moment after the last change), by its number
    static FLOW_SAVE: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

const BOT: &str = "chatbot";

/// The chatbot's actions, as kept; none yet: its old way's commands, as actions
fn flows_load(shared: &Shared) -> Vec<serde_json::Value> {
    let prefs = shared.borrow().web.prefs.clone();
    let mine = &prefs["pluginSettings"][BOT];
    if let Some(a) = mine["actions"].as_array() { return a.clone(); }
    let commands = mine["commands"].as_str().map(String::from).or_else(|| {
        omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == BOT).and_then(|p| plugrun::settings_of(&p.manifest, &prefs)["commands"].as_str().map(String::from))
    }).unwrap_or_default();
    flows::from_commands(&commands)
}

fn flows_open(shared: &Shared) {
    let list = flows_load(shared);
    let sel = if list.is_empty() { -1 } else { 0 };
    FLOW.with(|f| *f.borrow_mut() = (list, sel));
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let strs = |v: Vec<&str>| ModelRc::from(Rc::new(VecModel::from(v.into_iter().map(slint::SharedString::from).collect::<Vec<_>>())));
    ui.set_flow_trigger_types(strs(flows::TRIGGERS.iter().map(|k| k.label).collect()));
    ui.set_flow_step_types(strs(flows::STEPS.iter().map(|k| k.label).collect()));
    ui.set_flow_trigger_menu(ModelRc::from(Rc::new(VecModel::from(menu_entries(&flows::trigger_menu(), "")))));
    ui.set_flow_step_menu(ModelRc::from(Rc::new(VecModel::from(menu_entries(&flows::step_menu(), "")))));
    ui.set_flow_templates(strs(flows::templates().into_iter().map(|t| t.0).collect()));
    flows_paint(shared);
    ui.set_flow_open(true);
    picks_refresh(shared);
}

/// A menu's entries: each category's heading, then what's in it; hit: what matches the search (its name or
/// its category's)
fn menu_entries(menu: &[flows::MenuItem], find: &str) -> Vec<MenuEntry> {
    let find = find.trim().to_lowercase();
    let mut out = Vec::new();
    let mut last = "";
    for m in menu {
        if m.cat != last { out.push(MenuEntry { cat: m.cat.into(), label: "".into(), head: true, hit: false }); last = m.cat; }
        let hit = !find.is_empty() && format!("{} {}", m.cat, m.label).to_lowercase().contains(&find);
        out.push(MenuEntry { cat: m.cat.into(), label: m.label.clone().into(), head: false, hit });
    }
    out
}

/// The menu item at an entry (its headings counted, as the page has them)
fn menu_at(menu: &[flows::MenuItem], k: i32) -> Option<&flows::MenuItem> {
    let mut n = -1;
    let mut last = "";
    for m in menu {
        if m.cat != last { n += 1; last = m.cat; if n == k { return None; } }
        n += 1;
        if n == k { return Some(m); }
    }
    None
}

/// A trigger or a step, for its card: which kind (of the list), what it says, its fields as they're set
fn flow_node(list: &'static [flows::Kind], v: &serde_json::Value) -> FlowNode {
    let id = v["type"].as_str().unwrap_or("");
    let at = list.iter().position(|k| k.id == id).map(|i| i as i32).unwrap_or(-1);
    // (its name, as its menu has it: "Twitch > Channel Points Reward")
    let menu = if std::ptr::eq(list, flows::TRIGGERS) { flows::trigger_menu() } else { flows::step_menu() };
    let label: slint::SharedString = flows::menu_find(&menu, v).map(|m| format!("{} \u{203a} {}", m.cat, m.label)).unwrap_or_default().into();
    let Some(k) = flows::kind_of(list, id) else { return FlowNode { kind_at: at, note: "".into(), fields: ModelRc::default(), depth: 0, label } };
    let fields: Vec<FlowField> = k.fields.iter().map(|fl| {
        let now = &v[fl.key];
        let value = match now { serde_json::Value::String(s) => s.clone(), serde_json::Value::Bool(b) => b.to_string(), serde_json::Value::Null => fl.default.to_string(), serde_json::Value::Number(n) => n.as_f64().map(|x| if x.fract() == 0.0 { format!("{}", x as i64) } else { x.to_string() }).unwrap_or_default(), x => x.to_string() };
        // (a program's: the ones open now, as the card's painted)
        let options: Vec<slint::SharedString> = if fl.kind == "program" { procwatch::programs().into_iter().map(Into::into).collect() }
            else if matches!(fl.kind, "scene" | "source" | "reward" | "mscene" | "mlayer" | "meffect" | "mtrack" | "sscene" | "ssource") { pick_options(fl.kind).into_iter().map(Into::into).collect() }
            else { fl.options.iter().map(|o| o.1.into()).collect() };
        let at = fl.options.iter().position(|o| o.0 == value).map(|i| i as i32).unwrap_or(0);
        FlowField { key: fl.key.into(), label: fl.label.into(), note: fl.note.into(), kind: fl.kind.into(), value: value.into(), options: ModelRc::from(Rc::new(VecModel::from(options))), at }
    }).collect();
    FlowNode { kind_at: at, note: k.note.into(), fields: ModelRc::from(Rc::new(VecModel::from(fields))), depth: 0, label }
}

fn flow_group(a: &serde_json::Value) -> String { a["group"].as_str().unwrap_or("").trim().to_string() }

thread_local! {
    /// the actions Ctrl+clicked (where they are in the list), for grouping several at once
    static FLOW_MARKS: RefCell<Vec<usize>> = const { RefCell::new(Vec::new()) };
}

/// The groups there are: the actions', and the ones made with New Group that nothing's in yet (A to Z)
fn flow_groups(list: &[serde_json::Value], prefs: &serde_json::Value) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let made = prefs["flowGroups"].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(String::from));
    for g in list.iter().map(flow_group).chain(made) {
        if !g.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&g)) { out.push(g); }
    }
    out.sort_by_key(|g| g.to_lowercase());
    out
}

/// The groups made with New Group that nothing's in yet
fn flow_groups_empty(list: &[serde_json::Value], prefs: &serde_json::Value) -> Vec<String> {
    prefs["flowGroups"].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(String::from))
        .filter(|g| !list.iter().any(|a| flow_group(a).eq_ignore_ascii_case(g))).collect()
}

/// The groups folded in the Actions list (kept, as Settings' other choices)
fn flow_shut(prefs: &serde_json::Value) -> Vec<String> {
    prefs["flowFolded"].as_array().into_iter().flatten().filter_map(|g| g.as_str().map(String::from)).collect()
}

/// The list in order: by group (A to Z, the ungrouped last), each as it was within its group; the picked one
/// still picked
fn flows_sort(sel: i32, list: &mut Vec<serde_json::Value>) -> i32 {
    let mut keyed: Vec<(usize, serde_json::Value)> = std::mem::take(list).into_iter().enumerate().collect();
    keyed.sort_by_key(|(_, a)| { let g = flow_group(a); (g.is_empty(), g.to_lowercase()) });
    let at = keyed.iter().position(|(i, _)| *i as i32 == sel).map_or(-1, |p| p as i32);
    // (the ones Ctrl+clicked: where they are now)
    FLOW_MARKS.with(|m| { let mut m = m.borrow_mut(); *m = m.iter().filter_map(|&x| keyed.iter().position(|(i, _)| *i == x)).collect(); });
    *list = keyed.into_iter().map(|(_, a)| a).collect();
    at
}

/// Each action for the list: its group's heading on the first of each, hidden in a folded group or when it
/// isn't what's searched for (searching opens every group)
fn flow_items(list: &[serde_json::Value], shut: &[String], find: &str) -> Vec<FlowItem> {
    let find = find.trim().to_lowercase();
    let fits = |a: &serde_json::Value| find.is_empty() || format!("{} {} {}", a["name"].as_str().unwrap_or(""), flow_group(a), flows::summary(a)).to_lowercase().contains(&find);
    let grouped = list.iter().any(|a| !flow_group(a).is_empty());
    let mut out = Vec::with_capacity(list.len());
    let mut last: Option<String> = None;
    for a in list {
        let g = flow_group(a);
        let count = list.iter().filter(|x| flow_group(x).eq_ignore_ascii_case(&g) && fits(x)).count();
        let head = grouped && count > 0 && fits(a) && last.as_deref().is_none_or(|l| !l.eq_ignore_ascii_case(&g));
        if head { last = Some(g.clone()); }
        let is_shut = find.is_empty() && shut.iter().any(|s| s.eq_ignore_ascii_case(&g));
        let marked = FLOW_MARKS.with(|m| m.borrow().contains(&out.len()));
        out.push(FlowItem { name: a["name"].as_str().unwrap_or("").into(), summary: flows::summary(a).into(), on: a["on"].as_bool() != Some(false), group: g.into(), head, shown: fits(a) && !(grouped && is_shut), count: count as i32, shut: is_shut, marked });
    }
    out
}

/// The groups that are empty yet, as headings after the rest (after the actions, so each action's row is
/// still where it is in the list)
fn flow_items_empty(empty: &[String], find: &str) -> Vec<FlowItem> {
    if !find.trim().is_empty() { return Vec::new(); }
    empty.iter().map(|g| FlowItem { name: "".into(), summary: "".into(), on: true, group: g.as_str().into(), head: true, shown: false, count: 0, shut: false, marked: false }).collect()
}

/// The page drawn again: the list, and the picked one's flow
fn flows_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    // (in their groups' order: put there as it's painted, not as a group's typed in)
    let (list, sel) = FLOW.with(|f| { let mut f = f.borrow_mut(); let (mut list, sel) = (std::mem::take(&mut f.0), f.1); let sel = flows_sort(sel, &mut list); *f = (list, sel); f.clone() });
    let shut = flow_shut(&shared.borrow().web.prefs);
    let find = FLOW_FIND.with(|f| f.borrow().clone());
    // (New Group's ones, once an action's in them, are the actions' own)
    let empty = flow_groups_empty(&list, &shared.borrow().web.prefs);
    let made = shared.borrow().web.prefs["flowGroups"].as_array().map_or(0, |a| a.len());
    if made != empty.len() { pref_put(shared, "flowGroups", serde_json::json!(empty)); }
    let mut items = flow_items(&list, &shut, &find);
    // (headings shown for them alone too, when nothing else is grouped)
    if !empty.is_empty() && !items.iter().any(|i| i.head) {
        let mut first = true;
        for it in items.iter_mut() { if it.shown && first { it.head = true; first = false; } }
        let n = items.iter().filter(|i| i.shown).count() as i32;
        if let Some(h) = items.iter_mut().find(|i| i.head) { h.count = n; }
    }
    items.extend(flow_items_empty(&empty, &find));
    ui.set_flow_items(ModelRc::from(Rc::new(VecModel::from(items))));
    ui.set_flow_groups(ModelRc::from(Rc::new(VecModel::from(flow_groups(&list, &shared.borrow().web.prefs).into_iter().map(slint::SharedString::from).collect::<Vec<_>>()))));
    ui.set_flow_marked(FLOW_MARKS.with(|m| m.borrow().len()) as i32);
    ui.set_flow_sel(sel);
    let Some(a) = (sel >= 0).then(|| list.get(sel as usize)).flatten() else { return };
    ui.set_flow_name(a["name"].as_str().unwrap_or("").into());
    ui.set_flow_group(a["group"].as_str().unwrap_or("").into());
    ui.set_flow_on(a["on"].as_bool() != Some(false));
    ui.set_flow_queued(a["queued"].as_bool() == Some(true));
    ui.set_flow_trigger(flow_node(flows::TRIGGERS, &a["trigger"]));
    // (each step in under the If, Repeat or Pick it's in: its Otherwise and its end level with its start)
    let mut depth = 0i32;
    let steps: Vec<FlowNode> = a["steps"].as_array().into_iter().flatten().map(|s| {
        let t = s["type"].as_str().unwrap_or("");
        if matches!(t, "endif" | "endrepeat" | "endrandom" | "else") { depth = (depth - 1).max(0); }
        let mut n = flow_node(flows::STEPS, s);
        n.depth = depth;
        if matches!(t, "ifblock" | "repeat" | "randomblock" | "else") { depth += 1; }
        n
    }).collect();
    ui.set_flow_steps(ModelRc::from(Rc::new(VecModel::from(steps))));
}

/// The picked one's line in the list, drawn again where it is (its name or trigger typed in)
fn flows_item_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let (list, sel) = FLOW.with(|f| f.borrow().clone());
    if (sel >= 0) && (sel as usize) < list.len() {
        let m = ui.get_flow_items();
        let shut = flow_shut(&shared.borrow().web.prefs);
        let find = FLOW_FIND.with(|f| f.borrow().clone());
        let mut items = flow_items(&list, &shut, &find);
        // (the picked one's row only: the rest as they were, headings and all, till it's painted again)
        if (sel as usize) < m.row_count() {
            let mut it = items.swap_remove(sel as usize);
            if let Some(was) = m.row_data(sel as usize) { it.head = was.head; it.shown = true; it.count = was.count; it.shut = was.shut; }
            m.set_row_data(sel as usize, it);
        }
    }
}

/// A change to the picked action (its JSON), then kept a moment after the last one
fn flow_change(shared: &Shared, f: impl FnOnce(&mut serde_json::Value)) {
    FLOW.with(|x| { let mut x = x.borrow_mut(); let sel = x.1; if sel >= 0 { if let Some(a) = x.0.get_mut(sel as usize) { f(a); } } });
    flows_save_soon(shared);
}

fn flows_save_soon(shared: &Shared) {
    let n = FLOW_SAVE.with(|c| { c.set(c.get() + 1); c.get() });
    let s = shared.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(600), move || { if FLOW_SAVE.with(|c| c.get()) == n { flows_save(&s); } });
}

/// The actions kept with the chatbot's settings, and given to it as it runs
fn flows_save(shared: &Shared) {
    let list = FLOW.with(|f| f.borrow().0.clone());
    let all = {
        let mut a = shared.borrow_mut();
        let mut all = a.web.prefs["pluginSettings"].as_object().cloned().unwrap_or_default();
        let mut mine = all.get(BOT).and_then(|v| v.as_object()).cloned().unwrap_or_default();
        mine.insert("actions".into(), serde_json::json!(list));
        all.insert(BOT.into(), serde_json::Value::Object(mine));
        let all = serde_json::Value::Object(all);
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pluginSettings".into(), all.clone()); }
        all
    };
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pluginSettings", all); }
    if let Some(p) = plugrun::get(BOT) {
        let rt = shared.borrow().rt.clone();
        rt.spawn(async move { let _ = p.call("setActions", vec![serde_json::json!(list)]).await; });
    }
    hotkeys_update(shared);
}

// ---------- file dialogs that don't hold up the window ----------
// Windows' file dialog waits till it's closed. On the window's own thread everything waits with it (the music
// goes on, but a Seamless Mix's tempo changes stop part-way), so each opens on a thread of its own instead.

#[derive(Clone, Copy)]
enum Ask { Open, Save, Folder }

thread_local! {
    /// a dialog open (one at a time, as when they held the window)
    static PATH_ASKING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// what to do with the path chosen, kept here (it isn't Send) till the dialog's closed
    static ASK_THEN: RefCell<Option<Box<dyn FnOnce(std::path::PathBuf)>>> = const { RefCell::new(None) };
    /// the path chosen, for the action run again (ask_path)
    static PATH_ASKED: RefCell<Option<std::path::PathBuf>> = const { RefCell::new(None) };
}

/// The dialog opened on a thread of its own; once a path's chosen, `then` has it (nothing, if it's cancelled)
fn ask_then(dlg: rfd::FileDialog, how: Ask, then: impl FnOnce(std::path::PathBuf) + 'static) {
    if PATH_ASKING.with(|a| a.replace(true)) { return; }
    ASK_THEN.with(|a| *a.borrow_mut() = Some(Box::new(then)));
    std::thread::spawn(move || {
        let p = match how { Ask::Open => dlg.pick_file(), Ask::Save => dlg.save_file(), Ask::Folder => dlg.pick_folder() };
        let _ = slint::invoke_from_event_loop(move || {
            PATH_ASKING.with(|a| a.set(false));
            let then = ASK_THEN.with(|a| a.borrow_mut().take());
            if let (Some(then), Some(p)) = (then, p) { then(p); }
        });
    });
}

/// As ask_then, for an action with more to do after its dialog: the first time, the dialog's opened and it's
/// None (the action ends there); once a path's chosen the action's run again (`again`), and it's Some this time
fn ask_path(dlg: rfd::FileDialog, how: Ask, again: impl FnOnce() + 'static) -> Option<std::path::PathBuf> {
    if let Some(p) = PATH_ASKED.with(|a| a.borrow_mut().take()) { return Some(p); }
    ask_then(dlg, how, move |p| {
        PATH_ASKED.with(|a| *a.borrow_mut() = Some(p));
        again();
        PATH_ASKED.with(|a| a.borrow_mut().take());
    });
    None
}

/// Back Up: the actions, and the quotes, variables and points the chatbot keeps, in a file of their own
fn flows_backup(shared: &Shared) {
    flows_save(shared);
    let list = FLOW.with(|f| f.borrow().0.clone());
    let kept = shared.borrow().web.prefs["pluginSettings"][BOT].clone();
    let file = format!("OmniPulse Stream Suite {}.omnibot", chrono_date());
    let Some(path) = ask_path(rfd::FileDialog::new().set_title("Back up the Stream Suite").set_file_name(&file).add_filter("OmniPulse Stream Suite backup", &["omnibot"]), Ask::Save, { let s = shared.clone(); move || flows_backup(&s) }) else { return };
    let note = match std::fs::write(&path, serde_json::to_string_pretty(&botimport::backup(&list, &kept)).unwrap_or_default()) {
        Ok(()) => format!("Backed up {} to {}.", plural(list.len(), "action", "actions"), path.file_name().and_then(|n| n.to_str()).unwrap_or("it")),
        Err(e) => format!("Couldn't back it up: {e}"),
    };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
}

/// Today, as YYYY-MM-DD (a backup's name)
fn chrono_date() -> String {
    let days = omnipulse_core::now_secs() / 86400;
    // (from the days since 1970: the civil date, Howard Hinnant's way)
    let z = days as i64 + 719468;
    let era = z.div_euclid(146097);
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    format!("{:04}-{:02}-{:02}", yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

/// A backup restored: its actions over those of the same name (the rest added), its quotes added, and from
/// OmniPulse's own its variables and points; kept, and the chatbot started again with them
fn flows_restored(shared: &Shared, r: Result<botimport::Restored, String>) {
    let r = match r {
        Ok(r) => r,
        Err(e) => { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Couldn't restore it: {e}").into()); } return; }
    };
    bot_keep(shared, "imported", serde_json::json!(true));
    let mut list = FLOW.with(|f| f.borrow().0.clone());
    let all = {
        let mut a = shared.borrow_mut();
        let mut all = a.web.prefs["pluginSettings"].as_object().cloned().unwrap_or_default();
        let mut mine = all.get(BOT).and_then(|v| v.as_object()).cloned().unwrap_or_default();
        let (n, q) = botimport::merge(&mut list, &mut mine, &r);
        mine.insert("actions".into(), serde_json::json!(list));
        all.insert(BOT.into(), serde_json::Value::Object(mine));
        let all = serde_json::Value::Object(all);
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pluginSettings".into(), all.clone()); }
        if let Some(ui) = a.ui.upgrade() {
            let triggers = botimport::missed_triggers(&r);
            let steps = r.missed.len() - triggers;
            let mut missed = String::new();
            if steps > 0 { missed += &format!(" {} had no match here: kept as notes in their actions.", plural(steps, "step", "steps")); }
            if triggers > 0 { missed += &format!(" {} had no match here: those actions are Only When Run.", plural(triggers, "trigger", "triggers")); }
            ui.set_status(format!("Restored {}{} from {}.{missed}", plural(n, "action", "actions"), if q > 0 { format!(" and {}", plural(q, "quote", "quotes")) } else { String::new() }, r.from).into());
        }
        all
    };
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pluginSettings", all); }
    // (the first of them picked)
    let first = r.actions.first().and_then(|a| a["name"].as_str()).and_then(|n| list.iter().position(|x| x["name"].as_str().is_some_and(|y| y.eq_ignore_ascii_case(n)))).map(|i| i as i32).unwrap_or(if list.is_empty() { -1 } else { 0 });
    FLOW.with(|f| *f.borrow_mut() = (list, first));
    flows_paint(shared);
    // (its quotes, variables and points: the chatbot started again, to have them)
    if plugrun::get(BOT).is_some() { plugrun::stop(BOT); plugins_start(shared); }
}

/// A name no other action has: this one, else with a number after it
fn flow_name_free(list: &[serde_json::Value], want: &str) -> String {
    let taken = |n: &str| list.iter().any(|a| a["name"].as_str().is_some_and(|x| x.eq_ignore_ascii_case(n)));
    if !taken(want) { return want.to_string(); }
    (2..).map(|k| format!("{want} {k}")).find(|n| !taken(n)).unwrap_or_else(|| want.to_string())
}

const PICK_AGAIN: &str = "\u{21bb} Look Again";
const PICK_NEW_REWARD: &str = "+ Make a New Reward\u{2026}";

/// A reward picker's rewards: each name once, with where it's from and its ids
fn pick_rewards() -> Vec<(String, Vec<String>, Vec<String>, i64)> {
    let mut out: Vec<(String, Vec<String>, Vec<String>, i64)> = Vec::new();
    PICKS.with(|p| for r in p.borrow()["rewards"].as_array().into_iter().flatten() {
        let (title, id, plat) = (r["title"].as_str().unwrap_or("").to_string(), r["id"].as_str().unwrap_or("").to_string(), r["platform"].as_str().unwrap_or("twitch").to_string());
        if title.is_empty() { continue; }
        match out.iter_mut().find(|x| x.0.eq_ignore_ascii_case(&title)) {
            Some(x) => { x.1.push(plat); x.2.push(id); }
            None => out.push((title, vec![plat], vec![id], r["cost"].as_i64().unwrap_or(0))),
        }
    });
    out
}

/// The options a picker shows: what's there, then Look Again (and for a reward, Make a New Reward first)
fn pick_options(kind: &str) -> Vec<String> {
    let mut o: Vec<String> = match kind {
        "reward" => pick_rewards().into_iter().map(|(t, plats, _, cost)| format!("{t}  \u{b7}  {}{}", plats.iter().map(|p| if p == "kick" { "Kick" } else { "Twitch" }).collect::<Vec<_>>().join(", "), if cost > 0 { format!(", {cost}") } else { String::new() })).collect(),
        // (OBS's scenes and sources; Meld Studio's scenes, layers, effects and tracks; Streamlabs Desktop's)
        k @ ("scene" | "source" | "mscene" | "mlayer" | "meffect" | "mtrack" | "sscene" | "ssource") => {
            let key = match k { "scene" => "scenes", "source" => "sources", "mscene" => "meldScenes", "mlayer" => "meldLayers", "meffect" => "meldEffects", "mtrack" => "meldTracks", "sscene" => "slobsScenes", _ => "slobsSources" };
            PICKS.with(|p| p.borrow()[key].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect())
        }
        _ => Vec::new(),
    };
    if kind == "reward" { o.push(PICK_NEW_REWARD.into()); }
    o.push(PICK_AGAIN.into());
    o
}

/// The pickers' lists, asked of the Suite again (the Actions page opening, or Look Again)
fn picks_refresh(shared: &Shared) {
    let Some(p) = plugrun::get(BOT) else { return };
    let rt = shared.borrow().rt.clone();
    rt.spawn(async move {
        let Ok(v) = p.call("pickLists", vec![]).await else { return };
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| {
            PICKS.with(|x| *x.borrow_mut() = v);
            let Some(sh) = s.borrow().clone() else { return };
            let open = sh.borrow().ui.upgrade().is_some_and(|ui| ui.get_flow_open());
            if open { flows_paint(&sh); }
        }));
    });
}

/// A reward picked: its name, its ids (Twitch's and Kick's, if both have it) and where it's from
fn reward_set(t: &mut serde_json::Value, title: &str, plats: &[String], ids: &[String]) {
    t["reward"] = serde_json::json!(title);
    t["rewardid"] = serde_json::json!(ids.iter().filter(|x| !x.is_empty()).cloned().collect::<Vec<_>>().join(", "));
    let twitch = plats.iter().any(|p| p != "kick");
    let kick = plats.iter().any(|p| p == "kick");
    t["from"] = serde_json::json!(if twitch && kick { "any" } else if kick { "kick" } else { "twitch" });
}

/// The wizard's reward: made by the Suite on your channel; then given to the action whose trigger is a
/// reward (the one open), or an action made for it
fn reward_make(shared: &Shared, f: RewardForm) {
    let Some(p) = plugrun::get(BOT) else {
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Switch the Suite on first, and sign in to Twitch or Kick.".into()); }
        return;
    };
    let c = f.colour;
    let form = serde_json::json!({
        "title": f.title.trim(), "cost": f.cost.trim().parse::<i64>().unwrap_or(500).max(1), "prompt": f.prompt.trim(), "input": f.input,
        "cooldown": f.cooldown.trim().parse::<i64>().unwrap_or(0).max(0), "perStream": f.per_stream.trim().parse::<i64>().unwrap_or(0).max(0),
        "perUser": f.per_user.trim().parse::<i64>().unwrap_or(0).max(0), "skip": f.skip, "where": f.r#where.as_str(),
        "colour": format!("#{:02X}{:02X}{:02X}", c.red(), c.green(), c.blue()),
    });
    if form["title"].as_str().unwrap_or("").is_empty() { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Give it a name first.".into()); } return; }
    let want_action = f.action;
    let rt = shared.borrow().rt.clone();
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Making it...".into()); }
    rt.spawn(async move {
        let got = p.call("createReward", vec![form.clone()]).await;
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            let say = |t: String| if let Some(ui) = sh.borrow().ui.upgrade() { ui.set_status(t.into()); };
            let v = match got { Ok(v) => v, Err(e) => return say(format!("Couldn't make it: {e}")) };
            let made: Vec<(String, String)> = v["made"].as_array().into_iter().flatten().filter_map(|m| Some((m["platform"].as_str()?.to_string(), m["id"].as_str()?.to_string()))).collect();
            let failed = v["failed"].as_str().unwrap_or("").to_string();
            if made.is_empty() { return say(format!("Couldn't make it: {failed}")); }
            if let Some(ui) = sh.borrow().ui.upgrade() { ui.set_reward_make_open(false); }
            let title = form["title"].as_str().unwrap_or("").to_string();
            let (plats, ids): (Vec<String>, Vec<String>) = made.into_iter().unzip();
            // (the open action's trigger, if it's a reward; else a new action, if asked for)
            let open_is_reward = FLOW.with(|fl| { let fl = fl.borrow(); (fl.1 >= 0).then(|| fl.0.get(fl.1 as usize).map(|a| a["trigger"]["type"] == "event" && a["trigger"]["event"] == "reward")).flatten().unwrap_or(false) });
            if open_is_reward {
                flow_snap();
                flow_change(&sh, |a| reward_set(&mut a["trigger"], &title, &plats, &ids));
            } else if want_action {
                flow_snap();
                let mut trig = flows::defaults(flows::TRIGGERS, "event");
                trig["event"] = serde_json::json!("reward");
                reward_set(&mut trig, &title, &plats, &ids);
                let mut step = flows::defaults(flows::STEPS, "status");
                step["text"] = serde_json::json!(format!("%user% redeemed {title}"));
                FLOW.with(|fl| {
                    let mut fl = fl.borrow_mut();
                    let name = flow_name_free(&fl.0, &title);
                    fl.0.push(serde_json::json!({ "name": name, "on": true, "trigger": trig, "steps": [step] }));
                    fl.1 = fl.0.len() as i32 - 1;
                });
                flows_save(&sh);
            }
            flows_paint(&sh);
            picks_refresh(&sh);
            say(if failed.is_empty() { format!("Made {title}.") } else { format!("Made {title}, but not everywhere: {failed}") });
        }));
    });
}

/// The list as it is, kept to come back to (Ctrl+Z), before it's changed; what was undone forgotten
fn flow_snap() {
    let now = FLOW.with(|f| f.borrow().clone());
    FLOW_UNDO.with(|u| { let mut u = u.borrow_mut(); u.push(now); if u.len() > 50 { u.remove(0); } });
    FLOW_REDO.with(|r| r.borrow_mut().clear());
}

/// Ctrl+Z (back: true) or Ctrl+Y: the list as it was, or as it was undone
fn flow_undo(shared: &Shared, back: bool) {
    let (from, to) = if back { (&FLOW_UNDO, &FLOW_REDO) } else { (&FLOW_REDO, &FLOW_UNDO) };
    let Some(was) = from.with(|x| x.borrow_mut().pop()) else {
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if back { "Nothing to undo." } else { "Nothing to redo." }.into()); }
        return;
    };
    let now = FLOW.with(|f| std::mem::replace(&mut *f.borrow_mut(), was));
    to.with(|x| x.borrow_mut().push(now));
    flows_paint(shared);
    flows_save(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if back { "Undone. Ctrl+Y does it again." } else { "Done again." }.into()); }
}

/// The picked action, for the clipboard: as OmniPulse's own, so Ctrl+V knows it
fn flow_clip(shared: &Shared, cut: bool) {
    let Some(a) = FLOW.with(|f| { let f = f.borrow(); (f.1 >= 0).then(|| f.0.get(f.1 as usize).cloned()).flatten() }) else { return };
    let text = serde_json::to_string_pretty(&serde_json::json!({ "omnipulseAction": 1, "action": a })).unwrap_or_default();
    let ok = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)).is_ok();
    let name = a["name"].as_str().unwrap_or("it").to_string();
    if cut && ok {
        flow_snap();
        FLOW.with(|f| { let mut f = f.borrow_mut(); let sel = f.1; if sel >= 0 && (sel as usize) < f.0.len() { f.0.remove(sel as usize); } f.1 = (sel.min(f.0.len() as i32 - 1)).max(if f.0.is_empty() { -1 } else { 0 }); });
        flows_paint(shared);
        flows_save(shared);
    }
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_status(if !ok { "Couldn't copy it.".to_string() } else if cut { format!("Cut {name}. Ctrl+V puts it back, Ctrl+Z too.") } else { format!("Copied {name}. Ctrl+V pastes a copy.") }.into());
    }
}

/// Ctrl+V: an action copied from OmniPulse, as a new one after the picked one (a name of its own); anything
/// else, as Paste does (a Streamer.bot import code)
fn flow_clip_paste(shared: &Shared) {
    let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
    let v: serde_json::Value = serde_json::from_str(text.trim()).unwrap_or(serde_json::Value::Null);
    let list: Vec<serde_json::Value> = if v["omnipulseAction"].is_number() { vec![v["action"].clone()] }
        else if v["trigger"].is_object() && v["steps"].is_array() { vec![v.clone()] }
        else { Vec::new() };
    if list.is_empty() {
        if text.trim().is_empty() { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Nothing to paste.".into()); } return; }
        flow_snap();
        return flows_restored(shared, botimport::restore_text(&text));
    }
    flow_snap();
    FLOW.with(|f| {
        let mut f = f.borrow_mut();
        let mut at = if f.1 >= 0 { f.1 as usize + 1 } else { f.0.len() };
        for mut a in list {
            let name = flow_name_free(&f.0, a["name"].as_str().unwrap_or("Action"));
            a["name"] = serde_json::json!(name);
            at = at.min(f.0.len());
            f.0.insert(at, a);
            f.1 = at as i32;
            at += 1;
        }
    });
    flows_paint(shared);
    flows_save(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Pasted. Ctrl+Z takes it away again.".into()); }
}

fn flows_wire(ui: &AppWindow, shared: &Shared) {
    let s = shared.clone();
    ui.on_flow_act(move |act| {
        match act.as_str() {
            "close" => { flows_save(&s); if let Some(ui) = s.borrow().ui.upgrade() { ui.set_flow_open(false); } }
            "undo" => flow_undo(&s, true),
            "redo" => flow_undo(&s, false),
            "clip-copy" => flow_clip(&s, false),
            "clip-cut" => flow_clip(&s, true),
            "clip-paste" => flow_clip_paste(&s),
            // (a new group: named, and kept till an action's put in it; from the Group box, the open one put in it)
            "group-new" | "group-new-here" => {
                let here = act == "group-new-here";
                let has = FLOW.with(|f| flow_groups(&f.borrow().0, &s.borrow().web.prefs));
                ask_input(&s, "New Group", if here { "Name the group. This action goes in it." } else { "Name the group. To add actions, use the Group box or right-click them." }, "Make It", "", "A name, like Alerts", Vec::new(), move |s, name| {
                    let name = name.trim().to_string();
                    if name.is_empty() { return; }
                    if !has.iter().any(|g| g.eq_ignore_ascii_case(&name)) {
                        let mut made = s.borrow().web.prefs["flowGroups"].as_array().cloned().unwrap_or_default();
                        made.push(serde_json::json!(name));
                        pref_put(s, "flowGroups", serde_json::Value::Array(made));
                    }
                    if here { flow_change(s, |a| a["group"] = serde_json::json!(name)); flows_save(s); }
                    flows_paint(s);
                });
            }
            // (an empty group taken away)
            w if w.starts_with("group-drop:") => {
                let g = w["group-drop:".len()..].to_string();
                let made: Vec<serde_json::Value> = s.borrow().web.prefs["flowGroups"].as_array().cloned().unwrap_or_default().into_iter().filter(|x| !x.as_str().is_some_and(|x| x.eq_ignore_ascii_case(&g))).collect();
                pref_put(&s, "flowGroups", serde_json::Value::Array(made));
                flows_paint(&s);
            }
            // (right-clicked: it, or the ones picked with it, put in a group named, or taken out of theirs)
            w if w.starts_with("group-marked:") || w.starts_with("ungroup:") => {
                let out = w.starts_with("ungroup:");
                let Ok(at) = w.split_once(':').map(|x| x.1).unwrap_or("").parse::<usize>() else { return };
                let marks = FLOW_MARKS.with(|m| m.borrow().clone());
                let which: Vec<usize> = if marks.contains(&at) { marks } else { vec![at] };
                let put = move |s: &Shared, name: String| {
                    flow_snap();
                    FLOW.with(|f| { let mut f = f.borrow_mut(); for &i in &which { if let Some(a) = f.0.get_mut(i) { a["group"] = serde_json::json!(name.trim()); } } });
                    FLOW_MARKS.with(|m| m.borrow_mut().clear());
                    flows_paint(s);
                    flows_save(s);
                };
                if out { return put(&s, String::new()); }
                let (has, now) = FLOW.with(|f| { let f = f.borrow(); (flow_groups(&f.0, &s.borrow().web.prefs), f.0.get(at).map(flow_group).unwrap_or_default()) });
                let n = FLOW_MARKS.with(|m| m.borrow().len()).max(1);
                let (title, text) = if n > 1 && FLOW_MARKS.with(|m| m.borrow().contains(&at)) { (format!("Group {n} Actions"), "Type a group's name, or pick one. They all go in it.") } else { ("Put It in a Group".to_string(), "Type a group's name, or pick one.") };
                ask_input(&s, &title, text, "Group", &now, "A name, like Alerts", has, move |s, name| { if !name.trim().is_empty() { put(s, name); } });
            }
            // (Up and Down: the one before or after, in the list as it's shown)
            "key-up" | "key-down" => {
                flows_save(&s);
                FLOW.with(|f| { let mut f = f.borrow_mut(); let n = f.0.len() as i32; if n > 0 { f.1 = if act == "key-up" { (f.1 - 1).max(0) } else { (f.1 + 1).min(n - 1) }; } });
                flows_paint(&s);
            }
            // (the Delete key: gone, with a way back)
            "key-delete" => {
                let name = FLOW.with(|f| { let f = f.borrow(); (f.1 >= 0).then(|| f.0.get(f.1 as usize).and_then(|a| a["name"].as_str().map(String::from))).flatten() });
                let Some(name) = name else { return };
                flow_snap();
                FLOW.with(|f| { let mut f = f.borrow_mut(); let sel = f.1; if sel >= 0 && (sel as usize) < f.0.len() { f.0.remove(sel as usize); } f.1 = (sel.min(f.0.len() as i32 - 1)).max(if f.0.is_empty() { -1 } else { 0 }); });
                flows_paint(&s);
                flows_save(&s);
                if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(format!("Deleted {name}. Ctrl+Z brings it back.").into()); }
            }
            "delete" => {
                flow_snap();
                FLOW.with(|f| { let mut f = f.borrow_mut(); let sel = f.1; if sel >= 0 && (sel as usize) < f.0.len() { f.0.remove(sel as usize); } f.1 = (sel.min(f.0.len() as i32 - 1)).max(if f.0.is_empty() { -1 } else { 0 }); });
                flows_paint(&s);
                flows_save(&s);
            }
            "copy" => {
                flow_snap();
                FLOW.with(|f| {
                    let mut f = f.borrow_mut();
                    let sel = f.1;
                    if let Some(mut a) = (sel >= 0).then(|| f.0.get(sel as usize).cloned()).flatten() {
                        let name = flow_name_free(&f.0, a["name"].as_str().unwrap_or("Action"));
                        a["name"] = serde_json::json!(name);
                        f.0.push(a);
                        f.1 = f.0.len() as i32 - 1;
                    }
                });
                flows_paint(&s);
                flows_save(&s);
            }
            "backup" => flows_backup(&s),
            "queued" => { flow_change(&s, |a| { let on = a["queued"].as_bool() != Some(true); a["queued"] = serde_json::json!(on); }); flows_paint(&s); }
            "restore" => {
                let dlg = rfd::FileDialog::new().set_title("A chatbot backup to restore")
                    .add_filter("Chatbot backups", &["omnibot", "json", "sb", "txt", "zip", "miubackup", "miucommand"])
                    .add_filter("OmniPulse", &["omnibot", "json"]).add_filter("Streamer.bot", &["sb", "txt", "zip", "json"]).add_filter("Mix It Up", &["miubackup", "miucommand"]);
                let s = s.clone();
                ask_then(dlg, Ask::Open, move |path| {
                    flow_snap();
                    flows_restored(&s, botimport::restore_file(&path));
                });
            }
            "paste" => {
                let text = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
                if text.trim().is_empty() { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status("Nothing to paste. In Streamer.bot, click Export, then copy the code it gives you.".into()); } return; }
                flow_snap();
                flows_restored(&s, botimport::restore_text(&text));
            }
            "run" => {
                flows_save(&s);
                let name = FLOW.with(|f| { let f = f.borrow(); (f.1 >= 0).then(|| f.0.get(f.1 as usize).and_then(|a| a["name"].as_str().map(String::from))).flatten() });
                let (Some(name), Some(p)) = (name, plugrun::get(BOT)) else {
                    if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status("Switch the chatbot on first (Settings > Streaming), and sign in to Twitch.".into()); }
                    return;
                };
                let (rt, weak) = { let a = s.borrow(); (a.rt.clone(), a.ui.clone()) };
                rt.spawn(async move {
                    // (its new actions given to it first, then run)
                    let r = p.call("runAction", vec![serde_json::json!(name)]).await;
                    let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(match r { Ok(v) => v.as_str().map(String::from).unwrap_or_else(|| "Done.".into()), Err(e) => e }.into()));
                });
            }
            _ => {}
        }
    });
    let s = shared.clone();
    ui.on_flow_pick(move |i| { flows_save(&s); FLOW_MARKS.with(|m| m.borrow_mut().clear()); FLOW.with(|f| f.borrow_mut().1 = i); flows_paint(&s); });
    // (Ctrl+click: picked as well, or let go; the one open counts, the first time)
    let s = shared.clone();
    ui.on_flow_mark(move |i| {
        let i = i.max(0) as usize;
        let sel = FLOW.with(|f| f.borrow().1);
        FLOW_MARKS.with(|m| {
            let mut m = m.borrow_mut();
            if m.is_empty() && sel >= 0 && sel as usize != i { m.push(sel as usize); }
            if let Some(p) = m.iter().position(|&x| x == i) { m.remove(p); } else { m.push(i); }
        });
        flows_paint(&s);
    });
    // (a group picked from the Group box's list: in it, and put where it goes in the list)
    let s = shared.clone();
    ui.on_flow_group_set(move |g| { flow_change(&s, |a| a["group"] = serde_json::json!(g.trim())); flows_paint(&s); flows_save(&s); });
    ui.on_reward_make({ let s = shared.clone(); move |f| reward_make(&s, f) });
    let s = shared.clone();
    ui.on_flow_new(move |k| {
        let Some((_, mut a)) = flows::templates().into_iter().nth(k.max(0) as usize) else { return };
        flow_snap();
        FLOW.with(|f| {
            let mut f = f.borrow_mut();
            let name = flow_name_free(&f.0, a["name"].as_str().unwrap_or("New Action"));
            a["name"] = serde_json::json!(name);
            f.0.push(a);
            f.1 = f.0.len() as i32 - 1;
        });
        flows_paint(&s);
        flows_save(&s);
    });
    let s = shared.clone();
    ui.on_recent_window_set({ let s = shared.clone(); move |w| { pref_put(&s, "recentWindow", serde_json::json!(w.as_str())); if let Some(ui) = s.borrow().ui.upgrade() { ui.set_recent_window(w); } } });
    ui.on_flow_name_edited(move |t| { flow_change(&s, |a| a["name"] = serde_json::json!(t.as_str())); flows_item_paint(&s); });
    let s = shared.clone();
    ui.on_flow_group_fold(move |g| {
        let mut shut = flow_shut(&s.borrow().web.prefs);
        if let Some(i) = shut.iter().position(|x| x.eq_ignore_ascii_case(&g)) { shut.remove(i); } else { shut.push(g.to_string()); }
        pref_put(&s, "flowFolded", serde_json::json!(shut));
        flows_paint(&s);
    });
    let s = shared.clone();
    ui.on_flow_find(move |t| { FLOW_FIND.with(|f| *f.borrow_mut() = t.to_string()); flows_paint(&s); });
    let s = shared.clone();
    ui.on_flow_group_edited(move |t| { flow_change(&s, |a| a["group"] = serde_json::json!(t.trim())); flows_item_paint(&s); });
    let s = shared.clone();
    ui.on_flow_on_toggled(move |on| { flow_change(&s, |a| a["on"] = serde_json::json!(on)); flows_item_paint(&s); });
    let s = shared.clone();
    ui.on_flow_trigger_type(move |k| {
        let menu = flows::trigger_menu();
        let Some(m) = menu_at(&menu, k) else { return };
        let mut t = flows::defaults(flows::TRIGGERS, m.id);
        if let (Some(o), Some(p)) = (t.as_object_mut(), m.preset.as_object()) { for (k, v) in p { o.insert(k.clone(), v.clone()); } }
        flow_change(&s, |a| a["trigger"] = t);
        flows_paint(&s);
    });
    let s = shared.clone();
    ui.on_flow_menu_query(move |trigger, t| {
        let Some(ui) = s.borrow().ui.upgrade() else { return };
        if trigger { ui.set_flow_trigger_menu(ModelRc::from(Rc::new(VecModel::from(menu_entries(&flows::trigger_menu(), &t))))); }
        else { ui.set_flow_step_menu(ModelRc::from(Rc::new(VecModel::from(menu_entries(&flows::step_menu(), &t))))); }
    });
    let s = shared.clone();
    ui.on_flow_step_type(move |i, k| {
        let menu = flows::step_menu();
        let Some(kind) = menu_at(&menu, k).and_then(|m| flows::kind_of(flows::STEPS, m.id)) else { return };
        flow_change(&s, |a| { if let Some(st) = a["steps"].get_mut(i as usize) { *st = flows::defaults(flows::STEPS, kind.id); } });
        flows_paint(&s);
    });
    let s = shared.clone();
    ui.on_flow_step_add(move |k| {
        let menu = flows::step_menu();
        let Some(kind) = menu_at(&menu, k).and_then(|m| flows::kind_of(flows::STEPS, m.id)) else { return };
        flow_change(&s, |a| {
            if !a["steps"].is_array() { a["steps"] = serde_json::json!([]); }
            if let Some(l) = a["steps"].as_array_mut() {
                l.push(flows::defaults(flows::STEPS, kind.id));
                let end = match kind.id { "ifblock" => Some("endif"), "repeat" => Some("endrepeat"), "randomblock" => Some("endrandom"), _ => None };
                if let Some(e) = end { l.push(flows::defaults(flows::STEPS, e)); }
            }
        });
        flows_paint(&s);
    });
    let s = shared.clone();
    ui.on_flow_step_move(move |i, by| {
        flow_change(&s, |a| {
            let Some(l) = a["steps"].as_array_mut() else { return };
            let i = i as usize;
            if i >= l.len() { return; }
            if by == 0 { l.remove(i); return; }
            let j = i as i32 + by;
            if j >= 0 && (j as usize) < l.len() { l.swap(i, j as usize); }
        });
        flows_paint(&s);
    });
    // a field changed: its value kept as its kind has it (a choice: its option; a switch: true or false; a
    // number: a number); a choice or a switch drawn again, a box typed in left as it is
    let s = shared.clone();
    ui.on_flow_field(move |node, i, v| {
        let (list, sel) = FLOW.with(|f| f.borrow().clone());
        let Some(a) = (sel >= 0).then(|| list.get(sel as usize)).flatten() else { return };
        let (kinds, at): (&'static [flows::Kind], &serde_json::Value) = if node < 0 { (flows::TRIGGERS, &a["trigger"]) } else { (flows::STEPS, &a["steps"][node as usize]) };
        let Some(kind) = flows::kind_of(kinds, at["type"].as_str().unwrap_or("")) else { return };
        let Some(fl) = kind.fields.get(i.max(0) as usize) else { return };
        let value = match fl.kind {
            "select" => { let k = v.strip_prefix('\u{2}').and_then(|k| k.parse::<usize>().ok()).unwrap_or(0); serde_json::json!(fl.options.get(k).map(|o| o.0).unwrap_or(fl.default)) }
            // (a scene, a source or a reward picked; or Look Again, or Make a New Reward)
            "scene" | "source" | "reward" | "mscene" | "mlayer" | "meffect" | "mtrack" | "sscene" | "ssource" => match v.strip_prefix('\u{2}').and_then(|k| k.parse::<usize>().ok()) {
                Some(k) => {
                    let opts = pick_options(fl.kind);
                    let Some(o) = opts.get(k) else { return };
                    if o == PICK_AGAIN { picks_refresh(&s); if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status("Looking again...".into()); } return; }
                    if o == PICK_NEW_REWARD { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_reward_make_open(true); } return; }
                    if fl.kind == "reward" {
                        let Some((title, plats, ids, _)) = pick_rewards().into_iter().nth(k) else { return };
                        flow_change(&s, |a| { let t = if node < 0 { &mut a["trigger"] } else { &mut a["steps"][node as usize] }; reward_set(t, &title, &plats, &ids); });
                        return flows_paint(&s);
                    }
                    serde_json::json!(o)
                }
                None => serde_json::json!(v.as_str()),
            },
            // (one picked from the open ones: as they were listed, its name)
            "program" => match v.strip_prefix('\u{2}').and_then(|k| k.parse::<usize>().ok()) {
                Some(k) => match procwatch::programs_listed().get(k) { Some(n) => serde_json::json!(n), None => return },
                None => serde_json::json!(v.as_str()),
            },
            "toggle" => serde_json::json!(v == "true"),
            "number" => v.trim().parse::<f64>().map(|n| serde_json::json!(n)).unwrap_or_else(|_| serde_json::json!(v.as_str())),
            _ => serde_json::json!(v.as_str()),
        };
        let key = fl.key;
        flow_change(&s, |a| { let t = if node < 0 { &mut a["trigger"] } else { &mut a["steps"][node as usize] }; t[key] = value; });
        if matches!(fl.kind, "select" | "toggle") || (matches!(fl.kind, "program" | "scene" | "source" | "mscene" | "mlayer" | "meffect" | "mtrack" | "sscene" | "ssource") && v.starts_with('\u{2}')) { flows_paint(&s); } else { flows_item_paint(&s); }
    });
}

thread_local! {
    /// what was playing, and how many requests were waiting, as the chatbot was last told
    static BOT_SEEN: RefCell<(String, usize)> = RefCell::new((String::new(), 0));
}

/// Each tick: a song starting, or one requested, told to the plugins that run actions on them
fn plugins_app_events(shared: &Shared) {
    let (key, song, requested) = {
        let a = shared.borrow();
        let t = a.queue.current();
        let req = a.queue.items.iter().filter(|q| !q.by.is_empty()).count();
        let last = a.queue.items.iter().rev().find(|q| !q.by.is_empty()).map(|q| (q.t.title.clone(), q.t.artist.clone(), q.by.clone()));
        (t.map(|t| t.key.clone()).unwrap_or_default(), t.map(|t| (t.title.clone(), t.artist.clone())), (req, last))
    };
    let (was_key, was_req) = BOT_SEEN.with(|b| b.borrow().clone());
    BOT_SEEN.with(|b| *b.borrow_mut() = (key.clone(), requested.0));
    let mut tell: Vec<(&str, serde_json::Value)> = Vec::new();
    if !key.is_empty() && key != was_key && !was_key.is_empty() {
        if let Some((title, artist)) = song { tell.push(("start", serde_json::json!({ "title": title, "artist": artist }))); }
    }
    if requested.0 > was_req {
        if let Some((title, artist, by)) = requested.1 { tell.push(("request", serde_json::json!({ "title": title, "artist": artist, "user": by.trim_end_matches(" (Jam)") }))); }
    }
    if tell.is_empty() { return; }
    let rt = shared.borrow().rt.clone();
    for p in omnipulse_core::plugins::list() {
        let Some(run) = plugrun::get(&p.manifest.id).filter(|r| r.can("appEvent")) else { continue };
        for (kind, data) in tell.clone() {
            let run = run.clone();
            rt.spawn(async move { let _ = run.call("appEvent", vec![serde_json::json!(kind), data]).await; });
        }
    }
}

/// A Seamless Mix under way, for its crossfader: the two songs, how long it takes, when it was begun, and
/// whether the engine's started it yet
struct Xf { from: Track, to: Track, ms: u64, began: std::time::Instant, engaged: bool }

thread_local! {
    static XF: RefCell<Option<Xf>> = const { RefCell::new(None) };
}

/// A Seamless Mix begun (a blend, a roll, a tape stop or its fade): its crossfader shown, with both covers
fn xf_begin(shared: &Shared, from: Track, to: Track, ms: u64) {
    let (fa, ta) = (from.art.clone(), to.art.clone());
    let label = format!("Into {}", to.title);
    XF.with(|x| *x.borrow_mut() = Some(Xf { from, to, ms, began: std::time::Instant::now(), engaged: false }));
    let a = shared.borrow();
    let Some(ui) = a.ui.upgrade() else { return };
    ui.set_xf_label(label.into());
    ui.set_xf_frac(0.0);
    ui.set_xf_from_art(slint::Image::default());
    ui.set_xf_to_art(slint::Image::default());
    let weak = a.ui.clone();
    a.rt.spawn(async move {
        let (f, t) = tokio::join!(player::fetch_art(fa, art_px(34.0)), player::fetch_art(ta, art_px(34.0)));
        let _ = weak.upgrade_in_event_loop(move |ui| {
            if let Some(px) = f { ui.set_xf_from_art(slint::Image::from_rgba8(px)); }
            if let Some(px) = t { ui.set_xf_to_art(slint::Image::from_rgba8(px)); }
        });
    });
}

/// Each tick: the crossfader where it's up to, shown while the engine's crossfade runs (and a moment
/// before it starts); gone, and the widget told, once it's over
fn xf_tick(shared: &Shared, ui: &AppWindow) {
    let p = shared.borrow().engine.mix_progress();
    let state = XF.with(|x| x.borrow().as_ref().map(|x| (x.began.elapsed(), x.engaged)));
    let Some((since, engaged)) = state else {
        if ui.get_xf_on() { ui.set_xf_on(false); }
        return;
    };
    if p.is_none() && (engaged || since > std::time::Duration::from_secs(4)) {
        XF.with(|x| *x.borrow_mut() = None);
        ui.set_xf_on(false);
        if let Ok(mut c) = integrations::CROSSFADE.lock() { *c = None; }
        push_out(shared, false);
        return;
    }
    if !ui.get_xf_on() { ui.set_xf_on(true); }
    ui.set_xf_frac(p.unwrap_or(0.0));
    // (the engine's begun it: the widget's crossfader timed from where it is)
    if let Some(p) = p.filter(|_| !engaged) {
        let x = XF.with(|x| x.borrow_mut().as_mut().map(|x| { x.engaged = true; (x.from.clone(), x.to.clone(), x.ms) }));
        if let Some((from, to, ms)) = x {
            let side = |t: &Track| serde_json::json!({ "title": t.title, "artist": t.artist, "artwork": t.art.replace("-t200x200", "-t300x300") });
            let started = now_ms().saturating_sub((p as f64 * ms as f64) as u64);
            if let Ok(mut c) = integrations::CROSSFADE.lock() { *c = Some(serde_json::json!({ "from": side(&from), "to": side(&to), "ms": ms, "started_at": started })); }
            push_out(shared, false);
        }
    }
}

/// The heart: likes or unlikes what's playing, where it's from (put right back if that fails)
fn like_toggle(shared: &Shared) {
    let Some(t) = shared.borrow().queue.current().cloned() else { return };
    like_track(shared, t);
}

/// A track liked, or the like taken off, where it's from (put right back if that fails)
fn like_track(shared: &Shared, t: Track) {
    let playing = shared.borrow().queue.current().is_some_and(|c| c.key == t.key);
    let (t, on, rt, weak) = {
        let mut a = shared.borrow_mut();
        let on = !a.library.iter().any(|x| x.key == t.key);
        if on { a.library.insert(0, t.clone()); } else { a.library.retain(|x| x.key != t.key); }
        (t, on, a.rt.clone(), a.ui.clone())
    };
    if let Some(ui) = weak.upgrade() {
        if playing { ui.set_now_liked(on); }
        ui.set_status(format!("{} \"{}\".", if on { "Liked" } else { "Took the like off" }, t.title).into());
        album_head_paint(&shared.borrow(), &ui);
    }
    { let mut a = shared.borrow_mut(); apply_filter(&mut a); paint_queue(&mut a); }
    // a requested song: the widget cheers whoever asked for it, for a few seconds
    let by = if playing { shared.borrow().queue.qi.and_then(|i| shared.borrow().queue.items.get(i).map(|q| q.by.clone())).unwrap_or_default() } else { String::new() };
    if on && !by.is_empty() {
        let prefs = shared.borrow().web.prefs.clone();
        integrations::like_banter(&prefs, by.trim_end_matches(" (Jam)"));
        push_out(shared, true);
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(8100), move || push_out(&s, true));
    }
    let mirror = shared.borrow().web.prefs["ytMirror"].as_bool() == Some(true);
    rt.spawn(async move {
        let r = marks::set_like(&t, on).await;
        // (and on YouTube: its own song, or the same song with Like Other Tracks There Too)
        let yt = if r.is_ok() { marks::youtube_too(&t, on, mirror).await } else { None };
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            if let Some(note) = yt { ui.set_status(note.into()); }
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
    let Some(t) = shared.borrow().queue.current().cloned() else { return };
    fav_track(shared, t);
}

/// A track starred (a favourite, which likes it too), or not
fn fav_track(shared: &Shared, t: Track) {
    let playing = shared.borrow().queue.current().is_some_and(|c| c.key == t.key);
    let (t, on, liked) = {
        let a = shared.borrow();
        let on = !marks::favourites(&a.web.favourites).iter().any(|x| x.key == t.key);
        let liked = a.library.iter().any(|x| x.key == t.key);
        (t, on, liked)
    };
    marks::set_favourite(&t, on);
    { let mut a = shared.borrow_mut(); apply_filter(&mut a); paint_queue(&mut a); }
    if let Some(ui) = shared.borrow().ui.upgrade() {
        if playing { ui.set_now_fav(on); }
        ui.set_status(format!("{} \"{}\".", if on { "Starred" } else { "Took the star off" }, t.title).into());
    }
    if on && !liked { like_track(shared, t); }
}

/// Playing from a page: a list (Library, a playlist...) becomes the queue; one track from Search
/// takes the current one's place and leaves the rest of the queue as it was
fn play_list(shared: &Shared, list: Vec<Track>, i: usize) {
    no_pulse(shared);
    // listening along: in a Jam, the song goes to the host to play next; otherwise playing it here
    // would mean leaving, so that's said instead (the Jam button leaves)
    if let Some(kind) = listening() {
        let note = match list.get(i) {
            Some(t) if kind == "jam" => {
                listen_send(serde_json::json!({ "type": "add", "track": t.to_snapshot(), "next": true }));
                format!("Sent \"{}\" to the Jam, to play next.", t.title)
            }
            _ => "You're listening along. To play your own music, leave first with the button at the top.".to_string(),
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
    // Sets & Podcasts in the Queue off: the long ones left out of a list (not the one picked, and not on
    // Sets & Podcasts itself); Avoid Recent Plays: what you heard lately left out of what plays from a
    // list (not the one picked, not from a search, not your Library or a playlist, whose recent plays are
    // spaced out instead), unless that leaves too little (as the web app's startList)
    let (list, i) = {
        let a = shared.borrow();
        let (mut list, mut i) = (list, i);
        let picked = list.get(i).map(|t| t.key.clone());
        if !a.web.prefs["queueSets"].as_bool().unwrap_or(false) && a.page != "sets" {
            let kept: Vec<Track> = list.iter().filter(|t| !t.is_mix() || Some(&t.key) == picked.as_ref()).cloned().collect();
            if !kept.is_empty() { list = kept; }
        }
        let spaced = a.page == "library" || a.page == "playlist";
        if a.repeat == "off" && !spaced && pref_on(&a.web.prefs, "avoidRecent") && a.page != "search" && list.len() > 1 {
            let h = heard(&a);
            let win = recent_window(&a.web.prefs);
            let rest: Vec<&Track> = list.iter().filter(|t| Some(&t.key) != picked.as_ref()).collect();
            let fresh: Vec<&Track> = rest.iter().copied().filter(|t| !h.within(t, win)).collect();
            if fresh.len() < rest.len() && fresh.len() >= 3.min(rest.len().div_ceil(3)) {
                let keep: std::collections::HashSet<&str> = fresh.iter().map(|t| t.key.as_str()).collect();
                list = list.iter().filter(|t| Some(&t.key) == picked.as_ref() || keep.contains(t.key.as_str())).cloned().collect();
            }
        }
        if let Some(k) = picked { i = list.iter().position(|t| t.key == k).unwrap_or(0); }
        (list, i)
    };
    {
        let mut a = shared.borrow_mut();
        // (from a search or an artist's page: it only takes the place of what's playing, and the queue
        // you had carries on after it)
        if (a.page == "search" || a.page == "artist") && a.queue.current().is_some() {
            let Some(t) = list.get(i).cloned() else { return };
            a.queue.replace_current(t);
        } else {
            a.queue.set_list(list, i);
            // (its name, for the queue's heading: "Next From <it>")
            a.queue.name = a.ui.upgrade().map(|u| u.get_page_title().to_string()).unwrap_or_default();
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
    if std::env::var_os("OMNIPULSE_STEP_DEBUG").is_some() {
        let a = shared.borrow();
        let status = a.ui.upgrade().map(|u| u.get_status().to_string()).unwrap_or_default();
        println!("step debug: {d:+} from \"{}\" at {} of {} ms; status \"{status}\"; why {:?}", a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.engine.position_ms(), a.engine.duration_ms(), MIX_WHY.with(|m| m.borrow().as_ref().map(|x| x.2.clone())));
    }
    if listening().is_some() {
        return;   // listening along: the host moves on, and so does this
    }
    let engine = shared.borrow().engine.clone();
    // (back to where the track's set to start: its own start time, as the web app's)
    let from = trim_now(shared).0;
    if d < 0 && engine.position_ms() > from + 3000 {
        engine.seek(from);
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
            // (stopped: the accent you picked again, not the last cover's)
            apply_look(shared);
            if let Some(ui) = shared.borrow().ui.upgrade() {
                ui.set_playing(false);
            }
            push_out(shared, true);
        }
        // back from the first: it starts again
        None if d < 0 => engine.seek(from),
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

/// When you last heard each song: by its key, and by its name ("artist|title", so one heard on Spotify
/// counts when it turns up on SoundCloud), what you scrobbled to Last.fm included (as the web app's
/// lastHeard: its history and heardSongs)
#[derive(Clone, Default)]
struct Heard { by_key: HashMap<String, u64>, by_song: HashMap<String, u64> }

impl Heard {
    fn last(&self, t: &Track) -> u64 {
        let k = track::song_key(t);
        let by_song = if k.is_empty() { 0 } else { self.by_song.get(&k).copied().unwrap_or(0) };
        self.by_key.get(&t.key).copied().unwrap_or(0).max(by_song)
    }
    fn within(&self, t: &Track, ms: u64) -> bool { now_ms().saturating_sub(self.last(t)) < ms }
    /// For what's given, by key (what shuffling and spacing go by)
    fn map_for<'a>(&self, list: impl Iterator<Item = &'a Track>) -> HashMap<String, u64> {
        list.filter_map(|t| { let l = self.last(t); (l > 0).then(|| (t.key.clone(), l)) }).collect()
    }
}

thread_local! {
    /// the songs heard, by name: from your plays (made once), what plays here, and Last.fm's scrobbles
    static HEARD_SONGS: RefCell<Option<HashMap<String, u64>>> = const { RefCell::new(None) };
    /// when Last.fm's scrobbles were last asked for (every 30 minutes, while signed in there)
    static LFM_HEARD_AT: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
}

fn heard_song(key: String, ts: u64) {
    if key.is_empty() { return; }
    HEARD_SONGS.with(|h| { if let Some(m) = h.borrow_mut().as_mut() { let e = m.entry(key).or_insert(0); *e = (*e).max(ts); } });
}

fn heard(a: &App) -> Heard {
    HEARD_SONGS.with(|h| {
        if h.borrow().is_none() {
            let mut m = HashMap::new();
            for p in &a.web.plays {
                let k = track::song_key(&Track { title: p.title.clone(), artist: p.artist.clone(), ..Default::default() });
                if !k.is_empty() { let e = m.entry(k).or_insert(0); *e = (*e).max(p.ts); }
            }
            *h.borrow_mut() = Some(m);
        }
        Heard { by_key: a.web.last_played.clone(), by_song: h.borrow().clone().unwrap_or_default() }
    })
}

/// Last.fm (signed in there): what you scrobbled within the time frame, on any device, counts as heard
/// too; asked every 30 minutes (the web app's lfmHeardSync)
fn lfm_heard_sync(shared: &Shared) {
    if LFM_HEARD_AT.with(|t| t.get()).is_some_and(|t| t.elapsed().as_secs() < 1800) { return; }
    if omnipulse_core::lastfm::lastfm_status()["connected"].as_bool() != Some(true) { return; }
    LFM_HEARD_AT.with(|t| t.set(Some(std::time::Instant::now())));
    let (rt, weak, prefs) = { let a = shared.borrow(); let _ = heard(&a); (a.rt.clone(), a.ui.clone(), a.web.prefs.clone()) };
    let from = (now_ms().saturating_sub(recent_window(&prefs)) / 1000) as i64;
    let pages = if prefs["recentWindow"].as_str() == Some("month") { 10 } else { 5 };
    rt.spawn(async move {
        let Ok(list) = omnipulse_core::lastfm::lastfm_recent(from, pages).await else {
            // (try again in five minutes)
            let _ = weak.upgrade_in_event_loop(|_| LFM_HEARD_AT.with(|t| t.set(std::time::Instant::now().checked_sub(std::time::Duration::from_secs(1500)))));
            return;
        };
        let got: Vec<(String, u64)> = list.as_array().into_iter().flatten().map(|x| (track::song_key(&Track { title: x["title"].as_str().unwrap_or("").into(), artist: x["artist"].as_str().unwrap_or("").into(), ..Default::default() }), x["ts"].as_u64().unwrap_or(0))).collect();
        let _ = weak.upgrade_in_event_loop(move |_| { for (k, ts) in got { heard_song(k, ts); } });
    });
}

// ---------- Smart Transitions (crate::smart; docs/dj-transitions.md) ----------
/// Smart Transitions on (Settings > Sound, WIP: off unless switched on; never with Seamless Mix)
fn smart_on(prefs: &serde_json::Value) -> bool {
    set_row_on(prefs, "smartOn", false)
}

/// Its blends on the beat too (its page's Beatmatched Blends), not only fades
fn smart_blends_on(prefs: &serde_json::Value) -> bool {
    smart_on(prefs) && set_row_on(prefs, "smartBlend", true)
}

/// Its longest fade (its page's Longest Fade, s): 10 unless set
fn smart_max_ms(prefs: &serde_json::Value) -> u64 {
    prefs["smartMax"].as_u64().unwrap_or(10).clamp(3, 30) * 1000
}

/// With neither song's outro or intro measured: a fixed fade this long (Mixxx's; at most the longest)
const SMART_FIXED_MS: u64 = 6000;

thread_local! {
    /// the song the next one was measured ahead for (once each)
    static NEXT_AHEAD: RefCell<String> = const { RefCell::new(String::new()) };
}

thread_local! {
    /// the pair coming up (the one playing, the next one) and what's decided for it, once: a blend on
    /// the beat (its plan), or a fade (None), and why
    static SMART_PAIR: RefCell<Option<(String, String, Option<Plan>, String)>> = const { RefCell::new(None) };
    /// where the next one starts from (its first sound) as a fade brings it in: its key, ms
    static SMART_START: RefCell<Option<(String, u64)>> = const { RefCell::new(None) };
    /// the crossfade coming is Smart Transitions' (its voices handed over halfway)
    static SMART_XFADE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A song as Smart Transitions knows it (what's measured of it is of what it plays from)
struct SmartSong {
    t: Track,
    rhythm: Option<omnipulse_core::rhythm::Measured>,
    dur_ms: u64,
    audible_ms: u64,
    first_ms: u64,
    stated: Option<f64>,
    /// its key from outside (SongBPM), in place of our own reading
    key_known: bool,
}

impl SmartSong {
    fn side(&self) -> smart::Side<'_> {
        smart::Side { dur_ms: self.dur_ms, audible_ms: self.audible_ms, first_ms: self.first_ms, rhythm: self.rhythm.as_ref(), stated_bpm: self.stated, key_known: self.key_known }
    }
    /// Its key known, if our reading and SongBPM's agree. Neither is right often enough alone to shift a
    /// song's pitch by (ours and Spotify's analysis each put a song a fifth or its relative key out), but
    /// they go wrong differently: when they agree, it's that key
    fn with_key(mut self) -> Self {
        if let (Some(m), Some(Some((tonic, minor)))) = (self.rhythm.as_mut(), ext_key(&self.t)) {
            if m.rhythm.key == omnipulse_core::rhythm::key_name(tonic, minor) {
                m.rhythm.key_confidence = 1.0;
                self.key_known = true;
            }
        }
        self
    }
}

// ---------- songs' keys from outside (core::songbpm), kept in state/song-keys.json ----------
thread_local! {
    /// each song's key (tonic, minor), or none found (and when it was looked for, s)
    static EXT_KEYS: RefCell<Option<HashMap<String, serde_json::Value>>> = const { RefCell::new(None) };
    static EXT_KEYS_BUSY: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
}
/// None found: looked for again after this long
const EXT_KEY_RETRY_SECS: u64 = 7 * 86_400;

fn ext_keys_file() -> Option<std::path::PathBuf> {
    omnipulse_core::data_dir().ok().map(|d| d.join("state").join("song-keys.json"))
}

fn with_ext_keys<R>(f: impl FnOnce(&mut HashMap<String, serde_json::Value>) -> R) -> R {
    EXT_KEYS.with(|k| {
        let mut k = k.borrow_mut();
        let m = k.get_or_insert_with(|| ext_keys_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default());
        f(m)
    })
}

/// A song's key from outside: Some(Some(key)) found, Some(None) not found (or not this version), None
/// not looked for yet (it's looked for now, in the background)
fn ext_key(t: &Track) -> Option<Option<(usize, bool)>> {
    let saved = with_ext_keys(|m| m.get(&t.key).cloned());
    if let Some(v) = &saved {
        if let (Some(tonic), Some(minor)) = (v["tonic"].as_u64(), v["minor"].as_bool()) { return Some(Some((tonic as usize, minor))); }
        if omnipulse_core::now_secs().saturating_sub(v["at"].as_u64().unwrap_or(0)) < EXT_KEY_RETRY_SECS { return Some(None); }
    }
    if EXT_KEYS_BUSY.with(|b| !b.borrow_mut().insert(t.key.clone())) { return None; }
    let Some(rt) = SHARED.with(|s| s.borrow().as_ref().map(|sh| sh.borrow().rt.clone())) else { return None };
    let weak = SHARED.with(|s| s.borrow().as_ref().map(|sh| sh.borrow().ui.clone()));
    let t = t.clone();
    // (the first artist; the title as it is, its version kept, without who's featured or upload tags)
    let artist = t.artist.split(',').next().unwrap_or("").trim().to_string();
    let title = omnipulse_core::ranking::featured(&versions::match_title(&t)).1;
    rt.spawn(async move {
        let found = omnipulse_core::songbpm::lookup(&artist, &title, t.ms).await;
        let v = match &found {
            Ok(Some(k)) => Some(serde_json::json!({ "tonic": k.tonic, "minor": k.minor, "bpm": k.bpm, "ms": k.ms, "url": k.url })),
            Ok(None) => Some(serde_json::json!({ "none": true, "at": omnipulse_core::now_secs() })),
            Err(_) => None,
        };
        let Some(weak) = weak else { return };
        let _ = weak.upgrade_in_event_loop(move |_| {
            EXT_KEYS_BUSY.with(|b| b.borrow_mut().remove(&t.key));
            let Some(v) = v else { return };
            let text = with_ext_keys(|m| { m.insert(t.key.clone(), v); serde_json::to_string(m).ok() });
            if let (Some(p), Some(text)) = (ext_keys_file(), text) { let _ = std::fs::write(p, text); }
        });
    });
    None
}

/// Where a song's sound starts: after the silence it starts with, if its loudness analysis has one
fn first_sound(an: &serde_json::Value) -> u64 {
    an["silences"].as_array().and_then(|s| s.first()).filter(|s| s[0].as_u64() == Some(0)).and_then(|s| s[1].as_u64()).unwrap_or(0)
}

/// The one playing and the next one, as Smart Transitions knows them (None: nothing next)
fn smart_pair(a: &App, dur: u64) -> Option<(SmartSong, SmartSong)> {
    if a.repeat == "one" { return None; }
    let i = a.queue.qi?;
    let (cur, next) = (a.queue.items.get(i)?.t.clone(), a.queue.items.get(i + 1)?.t.clone());
    let stated = |t: &Track| smart::stated_bpm(&t.title).or_else(|| DESCS.with(|m| m.borrow().get(&t.key).and_then(|d| smart::stated_bpm(d))));
    // (measured whole, of what it plays from: never a Spotify song measured from its YouTube Music
    // version while Spotify's own playback plays it)
    // (its beat 1 where you've moved it, if you have)
    // (where it's mixed sure if the whole song isn't, unless you've moved its beat 1 yourself: the end of the
    // one playing, the start of the next one)
    let measured = |t: &Track, same: bool, end: bool| if same && !t.is_mix() { rhythm_full(&t.key).map(|m| {
        let shift = beat_shift(&a.web.prefs, &t.key);
        let mut m = if shift == 0 { smart::local_downbeat(&m, end) } else { smart::shifted(&m, shift) };
        m.rhythm.phrase_shift = phrase_shift(&a.web.prefs, &t.key);
        m
    }) } else { None };
    let an = a.cur_loud.as_ref().and_then(|k| a.analysis.get(k));
    let audible = an.and_then(|x| x["audible_ms"].as_u64()).unwrap_or(0);
    let a_side = SmartSong { rhythm: measured(&cur, MIX_OUT_OK.with(|c| c.get()), true), dur_ms: dur, audible_ms: audible, first_ms: 0, stated: stated(&cur), t: cur, key_known: false }.with_key();
    let v = if mix_via_youtube(&next) { versions::saved_id(&next.key) } else { None };
    let nan = sound::loud_source(&next, v.as_deref()).and_then(|(k, id)| a.analysis.get(&format!("{k}:{id}")));
    let b_dur = nan.and_then(|x| x["duration_ms"].as_u64()).filter(|&d| d > 0).unwrap_or(next.ms);
    let b_side = SmartSong { rhythm: measured(&next, smart_same_source(&next), false), dur_ms: b_dur, audible_ms: nan.and_then(|x| x["audible_ms"].as_u64()).unwrap_or(0), first_ms: nan.map(first_sound).unwrap_or(0), stated: stated(&next), t: next, key_known: false }.with_key();
    Some((a_side, b_side))
}

/// It plays from what was measured of it (a Spotify song plays from its YouTube Music version only
/// when Spotify can't play it here)
fn smart_same_source(t: &Track) -> bool {
    // (with Smart Transitions on, it plays from its YouTube Music version when there is one: start())
    !matches!(t.src, track::Src::Spotify(_)) || !sp_plays_here() || SHARED.with(|s| s.borrow().as_ref().and_then(|sh| sh.try_borrow().ok().map(|a| smart_on(&a.web.prefs))).unwrap_or(false))
}

/// Both songs sing over their parts of a transition (ms in each), by their timed lyrics (fetched as a
/// pair's decided: smart_lyrics); not known, or not timed: not (a centre cut where nobody sings only
/// thins the music)
fn smart_both_sing(a: &str, b: &str, a_at: (u64, u64), b_at: (u64, u64)) -> bool {
    let l = |k: &str| MIX_WORDS.with(|w| w.borrow().get(k).cloned());
    matches!((l(a), l(b)), (Some(la), Some(lb)) if smart::sings(&la, a_at.0, a_at.1) && smart::sings(&lb, b_at.0, b_at.1))
}

/// A song's timed lyrics fetched (once; kept with Wordplay's), for the voices' handoff
fn smart_lyrics(shared: &Shared, t: &Track) {
    if MIX_WORDS.with(|w| w.borrow().contains_key(&t.key)) || MIX_WORDS_BUSY.with(|b| !b.borrow_mut().insert(t.key.clone())) { return; }
    let (t, rt, weak) = { let a = shared.borrow(); (t.clone(), a.rt.clone(), a.ui.clone()) };
    let secs = t.ms / 1000;
    rt.spawn(async move {
        let l = player::mix_lyrics(&t, secs).await;
        let _ = weak.upgrade_in_event_loop(move |_| {
            MIX_WORDS.with(|w| w.borrow_mut().insert(t.key.clone(), l));
            MIX_WORDS_BUSY.with(|b| b.borrow_mut().remove(&t.key));
        });
    });
}

/// Your ratings so far: those sent and cleared (state/mix-feedback-tally.jsonl: their kinds and verdicts),
/// and those not sent yet (state/mix-feedback.jsonl): each one's kind, and good or not
fn smart_ratings() -> Vec<(String, bool)> {
    let Ok(d) = omnipulse_core::data_dir() else { return Vec::new() };
    let read = |f: &str| std::fs::read_to_string(d.join("state").join(f)).unwrap_or_default();
    let (tally, text) = (read("mix-feedback-tally.jsonl"), read("mix-feedback.jsonl"));
    let kind = |v: &serde_json::Value| v["kind"].as_str().or_else(|| v["mix"]["kind"].as_str()).map(String::from);
    tally.lines().chain(text.lines()).filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok()).filter_map(|v| Some((kind(&v)?, v["verdict"].as_str()? == "good"))).collect()
}

/// What's decided for the pair coming up, once: a blend on the beat (Some) or a fade (None). A blend's
/// decided as soon as both are measured whole; `late` (a fade's moment come): a fade, if it's not
/// decided by then. Not decided yet: None, and nothing kept.
fn smart_decide(shared: &Shared, a: &SmartSong, b: &SmartSong, pos: u64, late: bool) -> Option<Plan> {
    if let Some((x, y, p, _)) = SMART_PAIR.with(|s| s.borrow().clone()) {
        if x == a.t.key && y == b.t.key { return p; }
    }
    let prefs = shared.borrow().web.prefs.clone();
    let (sa, sb) = (a.side(), b.side());
    smart::KEYS.store(set_row_on(&prefs, "smartKeys", true), std::sync::atomic::Ordering::Relaxed);
    let decided: Option<(Option<Plan>, String)> = if !smart_blends_on(&prefs) {
        Some((None, "blends switched off".into()))
    } else if !streamsafe::allowed(&prefs, &b.t) || b.t.is_mix() {
        Some((None, "not one to blend into".into()))
    } else if a.rhythm.is_some() && b.rhythm.is_some() && !late && set_row_on(&prefs, "smartKeys", true) && (EXT_KEYS_BUSY.with(|k| k.borrow().contains(&a.t.key) || k.borrow().contains(&b.t.key))) {
        // (their keys still being looked for: decided once they're in, for a key ride by them)
        None
    } else if a.rhythm.is_some() && b.rhythm.is_some() {
        match smart::blend_blocked(&sa, &sb) {
            None => match smart::blend(&sa, &sb, pos) {
                Some(p) => Some((Some(p), "on the beat".into())),
                None => Some((None, "no phrase to blend on".into())),
            },
            Some(why) => {
                // Tier 2: a roll (switched on, and not turned down by your ratings), else a tape stop (switched
                // on, and with your ratings behind it)
                let ratings = smart_ratings();
                let roll = (set_row_on(&prefs, "smartRoll", false) && smart::not_rejected(&ratings, smart::ROLL)).then(|| smart::roll(&sa, &sb, pos)).flatten();
                let extra = roll.or_else(|| (set_row_on(&prefs, "smartTape", false) && smart::backed(&ratings, "tape stop")).then(|| omnipulse_mix::tape::tape(&sa, &sb, pos)).flatten());
                Some((extra, why.into()))
            }
        }
    } else if late {
        // (which wasn't, and whether it was being measured or still waiting its turn: for the reviews)
        let busy = |k: &str| RHYTHM_FULL_BUSY.lock().map(|b| b.iter().any(|x| x == k)).unwrap_or(false);
        let state = |s: &SmartSong| if s.rhythm.is_some() { None } else { Some(format!("{} ({})", s.t.title, if busy(&s.t.key) { "being measured or waiting its turn" } else { "not being measured" })) };
        let which: Vec<String> = [state(a), state(b)].into_iter().flatten().collect();
        omnipulse_core::diag::note(&format!("Seamless Mix: not measured in time: {}", which.join(", ")));
        Some((None, "not both measured in time".into()))
    } else {
        None
    };
    let (mut p, mut why) = decided?;
    // (never before halfway through the one playing: a fade instead, which keeps to it)
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms };
    if p.as_ref().is_some_and(|x| x.at_ms < smart::never_before_ms(end)) { p = None; why = "it would start before halfway".into(); }
    // (a blend that starts the next one past its first sung words: a fade instead, from its start: its
    // lyrics, where they're known, are never cut into)
    if let Some(plan) = p.as_ref().filter(|x| !x.tape_stop && x.style.is_none()) {
        let first_sung = MIX_WORDS.with(|w| w.borrow().get(&b.t.key).and_then(|l| l.first().map(|x| x.0)));
        if first_sung.is_some_and(|f| f < plan.start_ms) { p = None; why = "it would start past the next one's first words".into(); }
    }
    SMART_PAIR.with(|s| *s.borrow_mut() = Some((a.t.key.clone(), b.t.key.clone(), p.clone(), why)));
    p
}

/// Tier 0 each tick (Smart Transitions on, something next): the next one in over the one playing's outro
/// and its own intro, the silences at either end left out; unless a blend on the beat is planned for the
/// pair (crate::mix_tick's). None: not Smart Transitions' to do (Fade Between Tracks as set).
fn smart_fade_tick(shared: &Shared, key: &str, pos: u64, dur: u64) -> Option<bool> {
    if !smart_on(&shared.borrow().web.prefs) || listening().is_some() || room_kind().as_deref() == Some("jam") { return None; }
    // (a song just starting, its length not known yet: nothing to time a transition by. Without this, its end
    // read as 0 and it moved straight on, and the next, and the next, each measured as it went)
    if dur == 0 || pos < 1500 { return Some(false); }
    // (the next one still loading: what the engine says is the one before's place and length, not its own;
    // read as its, it was nearly over, so on it went, and on, through the queue)
    if shared.borrow().ui.upgrade().is_some_and(|u| u.get_track_loading()) { return Some(false); }
    let (a, b) = smart_pair(&shared.borrow(), dur)?;
    // (their lyrics, for whether both sing where they meet)
    // (and for never starting the next one past its first words)
    smart_lyrics(shared, &a.t);
    smart_lyrics(shared, &b.t);
    if MIX.with(|m| m.borrow().is_some()) || shared.borrow().engine.mixing() { return Some(false); }
    let (out, moved) = FADE.with(|f| f.borrow().clone());
    if out || moved == key { return Some(false); }
    let f = smart::fade(&a.side(), &b.side(), smart_max_ms(&shared.borrow().web.prefs), SMART_FIXED_MS);
    let left = f.out_to_ms as i64 - pos as i64;
    if std::env::var_os("OMNIPULSE_MIX_DEBUG").is_some() && pos % 1000 < 260 { eprintln!("smart debug at {pos}: {f:?}, left {left}, audible {} of {}, outro {:?}, intro {:?}", a.audible_ms, a.dur_ms, a.rhythm.as_ref().and_then(smart::outro_from), b.rhythm.as_ref().and_then(smart::intro_to)); }
    if left > (f.len_ms() + XFADE_LEAD_MS) as i64 {
        smart_decide(shared, &a, &b, pos, false);
        return Some(false);
    }
    if smart_decide(shared, &a, &b, pos, true).is_some() { return Some(false); }
    // (what's left of it, if the moment's gone by: a blend that couldn't be got ready, a seek)
    let ms = f.len_ms().min((left.max(0) as u64).max(smart::MIN_FADE_MS));
    let why = SMART_PAIR.with(|s| s.borrow().as_ref().map(|x| x.3.clone())).unwrap_or_default();
    // (in the log too, why it's a fade and not a blend: what to look at when they're all fades)
    omnipulse_core::diag::note(&format!("Seamless Mix used a fade instead of a blend ({} into {}): {}", a.t.title, b.t.title, if why.is_empty() { "neither measured yet" } else { why.as_str() }));
    MIX_WHY.with(|m| *m.borrow_mut() = Some((a.t.key.clone(), b.t.key.clone(), format!("smart fade: {ms} ms from {} to {}, the next one from {} ms (not on the beat: {why})", f.out_from_ms, f.out_to_ms, f.in_from_ms))));
    FADE.with(|fd| *fd.borrow_mut() = (true, key.to_string()));
    XFADE.with(|x| *x.borrow_mut() = Some((ms, std::time::Instant::now())));
    SMART_START.with(|x| *x.borrow_mut() = (f.in_from_ms > 0).then(|| (b.t.key.clone(), f.in_from_ms)));
    // (the voices handed over halfway only if both sing there)
    SMART_XFADE.with(|x| x.set(smart_both_sing(&a.t.key, &b.t.key, (f.out_from_ms, f.out_to_ms), (f.in_from_ms, f.in_from_ms + f.len_ms()))));
    CHANGE_HOW.with(|h| h.set("ended"));
    // (its crossfader: the one playing into the next)
    xf_begin(shared, a.t.clone(), b.t.clone(), ms);
    step(shared, 1);
    Some(true)
}

/// What shuffling goes by: when you last heard each track (long-unheard first), or, with repeat on,
/// nothing (plainly random: it all comes round again anyway)
fn shuffle_basis(a: &App) -> HashMap<String, u64> {
    if a.repeat != "off" { return HashMap::new(); }
    let h = heard(a);
    let mut m = h.map_for(a.queue.items.iter().map(|q| &q.t).chain(a.page_tracks.iter()));
    for (k, v) in &h.by_key { m.entry(k.clone()).or_insert(*v); }
    m
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
        (seed, a.rt.clone(), a.ui.clone(), local, last_plat, taken, a.web.prefs.clone(), heard(&a))
    };
    // (SoundCloud's similar tracks from a SoundCloud song played earlier too, picked at random, as the
    // web app's)
    let earlier: Option<Track> = {
        let a = shared.borrow();
        let before: Vec<&Track> = a.queue.items.iter().take(a.queue.qi.unwrap_or(0)).map(|q| &q.t).filter(|t| t.plat() == "soundcloud" && t.key != seed.key).collect();
        (!before.is_empty()).then(|| before[(rand_u64() as usize) % before.len()].clone())
    };
    // (Stream-Safe Music on: the Streamer page's collections are what it picks from)
    let stream = streamsafe::on(&prefs).then(|| {
        let have: Vec<Track> = STREAM_BUILT.with(|b| b.borrow().values().flatten().cloned().collect());
        let built: std::collections::HashSet<String> = STREAM_BUILT.with(|b| b.borrow().keys().cloned().collect());
        let mut todo: Vec<(String, StreamColl)> = {
            let p = &prefs;
            let mut all: Vec<(String, StreamColl)> = streamsafe::cleared_on(p).into_iter().map(|(n, names)| (n, StreamColl::Cleared(names))).collect();
            all.extend(streamsafe::GENRES.iter().map(|g| (g.to_string(), StreamColl::Genre(g.to_string()))));
            all.push(("Your Safe Likes".into(), StreamColl::Likes));
            all.into_iter().filter(|(n, _)| !built.contains(n)).collect()
        };
        for i in (1..todo.len()).rev() { let j = (rand_u64() as usize) % (i + 1); todo.swap(i, j); }
        (have, todo, shared.borrow().library.clone())
    });
    rt.spawn(async move {
        if let Some((have, todo, library)) = stream {
            let (pool, made) = stream_autoplay_pool(prefs.clone(), library, have, todo).await;
            let _ = weak.upgrade_in_event_loop(move |_| STREAM_BUILT.with(|b| { for (n, l) in made { b.borrow_mut().insert(n, l); } }));
            let lists = vec![("stream", pool)];
            let avoid = pref_on(&prefs, "avoidRecent");
            let window = recent_window(&prefs);
            let now = now_ms();
            let h = &last_played;
            let mut picks = queue::take_turns(lists.clone(), "", &taken, |t: &Track| !(avoid && now.saturating_sub(h.last(t)) < window));
            if picks.is_empty() && avoid { picks = queue::take_turns(lists, "", &taken, |t: &Track| now.saturating_sub(h.last(t)) >= 3 * 3_600_000); }
            let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { autoplay_done(&sh, &seed, picks); } }));
            return;
        }
        let want = |p: &str, key: &str| pref_on(&prefs, key) && (p != "local" || !local.is_empty());
        let (sc, sp, yt) = tokio::join!(
            async { if want("soundcloud", "scOn") { sources::autoplay_soundcloud(&seed, earlier.as_ref()).await } else { Vec::new() } },
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
        // (and only the language of recommendations, as the web app's langOk: what can't be placed stays)
        let want_lang = discover::lang_of(&prefs);
        let lang_ok = move |t: &Track| { if want_lang == "any" { return true; } let d = lang::detect(&t.title); d == "unknown" || d.ends_with('?') || d == want_lang };
        let ok = |within: u64| {
            let h = &last_played;
            let prefs = &prefs;
            let lang_ok = lang_ok.clone();
            move |t: &Track| (sets || !t.is_mix()) && !(avoid && now.saturating_sub(h.last(t)) < within) && streamsafe::allowed(prefs, t) && hide::rec_ok(prefs, t) && !hide::no_version(prefs, t) && lang_ok(t)
        };
        let mut picks = queue::take_turns(lists.clone(), last_plat, &taken, ok(window));
        if picks.is_empty() && avoid {
            picks = queue::take_turns(lists, last_plat, &taken, ok(3 * 3_600_000));
        }
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { autoplay_done(&sh, &seed, picks); } }));
    });
    true
}

/// Autoplay's picks in: after the queue (unless you started something else meanwhile, or switched it
/// off), said, and on to them if that was asked
fn autoplay_done(shared: &Shared, seed: &Track, picks: Vec<Track>) {
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
        let n = picks.len();
        a.queue.items.extend(picks.into_iter().map(queue::QItem::auto));
        paint_queue(&mut a);
        // (said, as the web app says it: how many, and what's next)
        if n > 0 {
            if let (Some(next), Some(ui)) = (a.queue.qi.and_then(|i| a.queue.items.get(i + 1)).map(|q| q.t.clone()), a.ui.upgrade()) {
                ui.set_status(format!("Autoplay added {} to the queue. Up next: {} by {}.", plural(n, "similar song", "similar songs"), next.title, next.artist).into());
            }
        }
        save_session(&a);
        then_play && a.queue.qi.is_some_and(|i| i + 1 < a.queue.items.len())
    };
    if go {
        step(shared, 1);
    }
}

// four times a second: the position, the lyric being sung, counting the play, autoplay, the
// session, the track's end
thread_local! {
    /// the graphic EQ's curve box, as wide for its height as the page draws it
    static EQ_ASPECT: std::cell::Cell<f32> = const { std::cell::Cell::new(4.0) };
}

/// The graphic EQ's curve through its bands (in a box 240 high, as wide as the page's: +12 dB at the top, -12 at
/// the foot), smooth between them; and the same closed down to 0 dB, for the wash under it
fn eq_curve(bands: &[f32]) -> (String, String) {
    if bands.len() < 2 { return (String::new(), String::new()); }
    let n = bands.len() - 1;
    let w = 240.0 * EQ_ASPECT.with(|a| a.get());
    let pt = |i: usize| (i as f32 * w / n as f32, (12.0 - bands[i].clamp(-12.0, 12.0)) / 24.0 * 240.0);
    let mut d = { let (x, y) = pt(0); format!("M {x:.1} {y:.1}") };
    // (each stretch a curve, its handles from the bands either side: Catmull-Rom, as Bézier)
    for i in 0..n {
        let (p0, p1, p2, p3) = (pt(i.saturating_sub(1)), pt(i), pt(i + 1), pt((i + 2).min(n)));
        let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
        let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
        d += &format!(" C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1}", c1.0, c1.1, c2.0, c2.1, p2.0, p2.1);
    }
    let fill = format!("{d} L {w:.1} 120 L 0 120 Z");
    (d, fill)
}

// ---------- the first-run setup: its steps, the first time OmniPulse opens ----------
// 0 welcome, 1 your music, 2 bring it with you, 3 your taste (only with no accounts signed in: nothing to
// start from), 4 how it sounds, 5 do you stream?, 6 all set. Each is kept as it's set (its own settings, as
// Settings keeps them); finished or skipped, it's not shown again (Run Setup Again, in App & Look).

/// To be shown: never set up (a new install: nothing signed in, nothing in the library, no theme picked)
fn setup_wanted(a: &App) -> bool {
    if a.web.prefs["firstRunDone"].as_bool() == Some(true) || std::env::var_os("OMNIPULSE_SELFTEST").is_some() { return false; }
    let fresh = !omnipulse_core::soundcloud::signed_in() && !account_signed("spotify") && a.web.prefs["themeStep"].is_null() && a.web.prefs["themeName"].is_null() && a.web.prefs["theme"].is_null();
    fresh
}

fn setup_show(shared: &Shared, at: i32) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    ui.set_setup_web(omnipulse_core::load_web_export().is_some());
    ui.set_setup_at(at);
    ui.set_setup_open(true);
}

fn setup_done(shared: &Shared) {
    pref_put(shared, "firstRunDone", serde_json::json!(true));
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_setup_open(false); }
}

fn setup_go(shared: &Shared, how: &str) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let at = ui.get_setup_at();
    // (the fourth step passed over: its genres are on the Discover step now)
    let skip_taste = |n: i32| if n == 3 { if at > 3 { 2 } else { 4 } } else { n };
    match how {
        "next" => setup_show(shared, skip_taste((at + 1).min(6))),
        "back" => setup_show(shared, skip_taste((at - 1).max(0))),
        "done" => setup_done(shared),
        "again" => setup_show(shared, 0),
        // (streaming: the Suite turned on, its own setup opened next; or nothing)
        "stream-yes" => {
            setup_done(shared);
            if !plugin_on(&shared.borrow().web.prefs, botpage::BOT) { set_toggle(shared, &format!("plugOn_{}", botpage::BOT)); }
            if !plugin_runs(&shared.borrow().web.prefs, botpage::BOT) { set_toggle(shared, &format!("plugRun_{}", botpage::BOT)); }
            plugins_start(shared);
            open_page_now(shared, "settings");
            ui.set_set_cat("Streaming".into());
            pf_open(shared, botpage::BOT);
        }
        "stream-no" => setup_show(shared, 6),
        _ => {}
    }
}

fn tick(shared: &Shared, events: &std::sync::mpsc::Receiver<Event>, n: u64) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    // (every couple of seconds: the next song's cover once it's in, and the song's shape once it's measured)
    if n % 8 == 4 {
        let a = shared.borrow();
        paint_next(&a);
        if let Some(t) = a.queue.current() {
            let now = rhythm_full(&t.key).or_else(|| omnipulse_core::rhythm::known(&t.key)).map(|m| m.rhythm.bar_db.len()).unwrap_or(0);
            if WAVE_FOR.with(|w| { let w = w.borrow(); w.0 != t.key || w.1 != now }) { paint_wave(&ui, &t.key); }
        }
    }
    // (album-art colours: your own accent while paused, the cover's again on play)
    {
        // (paused by you: not a song being loaded after a skip, which held the engine still long enough to
        // flash your accent in and the cover's back, as if a transition were under way)
        let paused = { let a = shared.borrow(); a.loaded && a.engine.is_paused() && !ui.get_track_loading() };
        let since = PAUSED_SINCE.with(|p| { if !paused { p.set(None); } else if p.get().is_none() { p.set(Some(std::time::Instant::now())); } p.get() });
        let want = since.is_some_and(|s| s.elapsed().as_millis() >= 600);
        if want != ART_PAUSED.with(|p| p.get()) { ART_PAUSED.with(|p| p.set(want)); apply_look(shared); }
    }
    if n % 40 == 8 { lfm_heard_sync(shared); }
    // (the beat model let go once it's sat unused a minute: loaded again when a song's next measured)
    if n % 120 == 60 { omnipulse_core::rhythm::release_model(std::time::Duration::from_secs(60)); }
    // Now Playing's visualiser: running only while it's shown, playing, with a style picked
    viz_update(shared, &ui);
    #[cfg(feature = "memcount")]
    if n % 20 == 0 && std::env::var_os("OMNIPULSE_SELFTEST").is_some() {
        let a = shared.borrow();
        println!("memcount: heap {:.1} MB; page {}, library {}, page tracks {}", memcount::LIVE.load(std::sync::atomic::Ordering::Relaxed) as f64 / 1048576.0, a.page, a.library.len(), a.page_tracks.len());
    }
    UI_SCALE.store((ui.window().scale_factor() * 100.0).round() as u32, std::sync::atomic::Ordering::Relaxed);
    // a page loading: its platforms, ticked as each answers
    {
        let page = shared.borrow().page.clone();
        let (n, steps) = loading::shown(&page);
        if LOAD_SHOWN.with(|x| x.replace((n, page.clone()))) != (n, page.clone()) {
            ui.set_load_page(if steps.is_empty() { "".into() } else { page.into() });
            ui.set_load_steps(ModelRc::from(Rc::new(VecModel::from(steps.into_iter().map(|(p, name, done)| LoadStep { plat: p.into(), name: name.into(), done }).collect::<Vec<_>>()))));
        }
    }
    // a mix playing: the title bar's button and the progress bar show it
    let mixing = shared.borrow().engine.mixing();
    if ui.get_mixing() != mixing { ui.set_mixing(mixing); }
    xf_tick(shared, &ui);
    plugins_app_events(shared);
    windows_now(shared, &ui);
    // (YouTube Music's helpers being set up: how far, each second, while Settings is open)
    if n.is_multiple_of(4) && ui.get_page() == "settings" && omnipulse_core::tools::progress()["busy"].as_bool() == Some(true) { helpers_paint(shared); }
    if n == 8 { helpers_start(shared); }
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
        if now != seen.0 && STATUS_ACTS.with(|s| s.borrow().0 != now) && ui.get_status_actions().row_count() > 0 {
            ui.set_status_actions(ModelRc::default());
            STATUS_ACTS.with(|s| *s.borrow_mut() = (String::new(), Vec::new()));
        }
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
    // (a widget's settings page open: its pop-out preview kept to what's on the page, as it plays)
    widget_preview(shared);
    let engine = shared.borrow().engine.clone();
    if !shared.borrow().loaded {
        return; // a restored session, not started yet: the bar shows where it was
    }
    let (pos, raw_dur) = (engine.position_ms(), engine.duration_ms());
    // (where you are, kept every few seconds while playing, as the web app's: a crash or a forced
    // close loses no more than that)
    if !engine.is_paused() && SESSION_SAVED.with(|t| t.get().elapsed().as_secs() >= 5) {
        SESSION_SAVED.with(|t| t.set(std::time::Instant::now()));
        save_session(&shared.borrow());
    }
    // a gap heard mid-track (the sound ran out before the next of it came): noted, with what else was
    // going on, so it can be tracked down
    let gap = engine.take_gap_ms();
    if gap >= 20 && pos > 2000 && pos + 2000 < raw_dur && !engine.is_paused() {
        let measuring = RHYTHM_FULL_BUSY.lock().map(|b| b.len()).unwrap_or(0);
        let mix = MIX.with(|m| match &*m.borrow() { None => "none", Some(MixPhase::Opening(_)) => "opening the next", Some(MixPhase::Waiting { .. }) => "waiting", Some(MixPhase::Mixing { .. }) => "mixing" });
        let title = shared.borrow().queue.current().map(|t| t.title.clone()).unwrap_or_default();
        omnipulse_core::diag::note(&format!("playback gap: {gap} ms at {} in \"{title}\" (measuring {measuring} whole, mix {mix}, preloading {})", fmt_ms(pos), ui.get_preloading()));
    }
    // a track's start and end times (Track preferences): it shows, and plays, as only that part
    let (t_start, t_end) = trim_now(shared);
    let dur = t_end.unwrap_or(raw_dur);
    // (a mix under way: the one going out's place, as it's the one shown; its lyrics followed)
    let (vpos, vdur) = match mix_show_pos() {
        Some((p, d)) => {
            MIX_SHOW.with(|m| if let Some(x) = m.borrow().as_ref() {
                if let Some(i) = x.lyrics.iter().rposition(|l| l.ms > 0 && l.ms <= p + 200) { if ui.get_lyric_on() != i as i32 { ui.set_lyric_on(i as i32); } }
            });
            (p, d)
        }
        None => (pos.saturating_sub(t_start), dur.saturating_sub(t_start)),
    };
    // (the length shown, kept: a mix that starts shows the one going out at it, not the engine's, which is
    // the incoming one's by then)
    if mix_show_pos().is_none() { SHOWN_DUR.with(|d| d.set(vdur)); }
    ui.set_pos_text(fmt_ms(vpos).into());
    ui.set_dur_text(fmt_ms(vdur).into());
    ui.set_dur_secs(vdur as f32 / 1000.0);
    ui.set_pos_frac(if vdur > 0 { (vpos as f32 / vdur as f32).min(1.0) } else { 0.0 });
    // its end time: on to the next one (once)
    if let Some(end) = t_end {
        let here = format!("{}:{:?}", shared.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default(), shared.borrow().queue.qi);
        if !engine.is_paused() && pos >= end && TRIM_ENDED.with(|e| *e.borrow() != here) {
            TRIM_ENDED.with(|e| *e.borrow_mut() = here);
            ui.set_status(format!("Ended at {}, the end time you set for this track.", fmt_time(end)).into());
            if shared.borrow().repeat == "one" { start(shared); } else { step(shared, 1); }
            return;
        }
    }
    if let Some((k, until)) = TP_PREVIEW.with(|p| p.borrow().clone()) {
        let here = shared.borrow().queue.current().is_some_and(|c| c.key == k);
        if !here { TP_PREVIEW.with(|p| p.borrow_mut().take()); }
        else if pos + 150 >= until && !engine.is_paused() { TP_PREVIEW.with(|p| p.borrow_mut().take()); toggle(shared); ui.set_status("Preview over.".into()); }
    }
    // (a song still loading: the engine's place is the one before's; its lyrics wait for its own)
    let loading = ui.get_track_loading();
    // Smart Transitions: the next song measured as soon as the transition into this one is over (not
    // later on in this one), so it's ready for the next transition
    if !loading && pos > 3000 && !engine.mixing() && !engine.crossfading() && smart_on(&shared.borrow().web.prefs) {
        let (cur, next) = { let a = shared.borrow(); (a.queue.current().map(|t| t.key.clone()), a.queue.qi.and_then(|i| a.queue.items.get(i + 1)).map(|q| q.t.clone())) };
        if let (Some(cur), Some(next)) = (cur, next) {
            if NEXT_AHEAD.with(|n| n.replace(cur.clone())) != cur {
                if smart_same_source(&next) { rhythm_ahead(shared, next.clone()); }
                loud_ahead(shared, next);
            }
        }
    }
    let near_end = {
        let mut a = shared.borrow_mut();
        if loading { if ui.get_lyric_on() != -1 { ui.set_lyric_on(-1); } }
        else if let Some(i) = a.lyrics.iter().rposition(|l| l.ms > 0 && l.ms <= pos + 200).filter(|_| mix_show_pos().is_none()) {
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
                heard_song(track::song_key(&t), now_ms());
            }
        }
        // a scrobble, by Last.fm's rule (as the web app's): a track over 30 s, once half of it (or four
        // minutes) has been heard
        if a.heard.2 && !SCROBBLED.with(|x| x.get()) && dur > 30_000 && a.heard.1 >= (dur as f32 / 2000.0).min(240.0) {
            SCROBBLED.with(|x| x.set(true));
            if let Some(t) = a.queue.current().cloned() { integrations::lastfm_scrobble(&a.rt, &a.web.prefs, &t, a.started_secs); }
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
    let dbg = std::env::var_os("OMNIPULSE_MIX_DEBUG").is_some() && pos % 1000 < 260;
    if !engine.is_paused() && fade_and_silences(shared, pos, dur) {
        if dbg { eprintln!("mix debug at {pos}: fades and silences took the tick"); }
        return;
    }
    if !engine.is_paused() && sb_tick(shared, pos, dur) {
        if dbg { eprintln!("mix debug at {pos}: SponsorBlock took the tick"); }
        return;
    }
    if dbg { eprintln!("mix debug at {pos}: paused {}", engine.is_paused()); }
    rhythm_tick(shared, pos);
    if !engine.is_paused() { mix_tick(shared, pos, dur); }
    while let Ok(ev) = events.try_recv() {
        match ev {
            Event::Mixed => mix_started(shared),
            Event::Ended if shared.borrow().repeat == "one" => { CHANGE_HOW.with(|h| h.set("ended")); start(shared) }
            Event::Ended if shared.borrow().closest.is_some() => {
                let (done, cur) = { let a = shared.borrow(); (a.closest.clone(), a.queue.current().cloned()) };
                if let (Some((key, yt)), Some(t)) = (done, cur) {
                    if t.key == key {
                        versions::keep(&t, &yt);
                        // (its artist shown as that version's, as the web app's)
                        let theirs = yt.artists.clone();
                        if !theirs.is_empty() && theirs.to_lowercase() != t.artist.to_lowercase() {
                            let mut fixes = shared.borrow().web.prefs["trackFixes"].as_object().cloned().unwrap_or_default();
                            let title = fixes.get(&t.key).and_then(|f| f["title"].as_str().map(String::from)).unwrap_or(t.title.clone());
                            fixes.insert(t.key.clone(), serde_json::json!({ "title": title, "artist": theirs }));
                            pref_put(shared, "trackFixes", serde_json::Value::Object(fixes));
                        }
                        ui.set_status(format!("\"{}\" plays that version from now on{}.", t.title, if theirs.is_empty() { String::new() } else { format!(", by {theirs}") }).into());
                    }
                }
                shared.borrow_mut().closest = None;
                CHANGE_HOW.with(|h| h.set("ended"));
                step(shared, 1);
            }
            Event::Ended => { CHANGE_HOW.with(|h| h.set("ended")); step(shared, 1) }
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

/// A track's beat 1 as moved by hand (Track Preferences: beats, + later; 0 as measured)
fn beat_shift(prefs: &serde_json::Value, key: &str) -> i32 {
    prefs["beatShifts"][key].as_i64().unwrap_or(0).clamp(-8, 8) as i32
}

/// Where a track's phrases start, as moved by hand (Track Preferences: bars, + later; 0 as measured)
fn phrase_shift(prefs: &serde_json::Value, key: &str) -> i32 {
    prefs["phraseShifts"][key].as_i64().unwrap_or(0).clamp(-3, 3) as i32
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
    let safe = streamsafe::on(&a.web.prefs);
    autoplay_for_safe(&mut a, safe);
    a.resume_ms = Some(pos);
    a.resume_key = a.queue.current().map(|c| c.key.clone()).unwrap_or_default();
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
    // (a song still loading: the engine's place and length are the one before's: its own start, not those)
    let (pos, dur) = if a.ui.upgrade().is_some_and(|u| u.get_track_loading()) { (0, 0) } else { (pos, dur) };
    let t = a.queue.current();
    let prefs = &a.web.prefs;
    if integrations::set(prefs, "widget") || integrations::set(prefs, "req") || integrations::set(prefs, "lyricsWidget") || integrations::set(prefs, "socials") {
        let by = a.queue.qi.and_then(|i| a.queue.items.get(i)).map(|q| q.by.as_str()).unwrap_or("");
        integrations::widget_update(prefs, t, by, pos, dur, playing, (&a.accent.0, &a.accent.1), &a.lyrics);
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
        // (the chatbot's actions, by name: a Chatbot Action key's settings list them)
        "chatbotActions": a.web.prefs["pluginSettings"][BOT]["actions"].as_array().map(|l| l.iter().filter_map(|x| x["name"].as_str().map(String::from)).collect::<Vec<_>>()).unwrap_or_default(),
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
        // (Shuffle on or off, as the player bar's; like, favourite and repeat as theirs)
        "shuffle" => shuffle_toggle(shared),
        "like" => { let t = shared.borrow().queue.current().cloned(); if let Some(t) = t { like_track(shared, t); } }
        "favourite" => { let t = shared.borrow().queue.current().cloned(); if let Some(t) = t { fav_track(shared, t); } }
        "repeat" => { let ui = shared.borrow().ui.upgrade(); if let Some(ui) = ui { ui.invoke_repeat_cycle(); } }
        // (only what's playing stays, as the web app's)
        "clear-queue" => {
            let mut a = shared.borrow_mut();
            match a.queue.qi {
                Some(i) => { let cur = a.queue.items.remove(i); a.queue.items = vec![cur]; a.queue.qi = Some(0); }
                None => a.queue.items.clear(),
            }
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
            let now = !shared.borrow().web.prefs["queueSets"].as_bool().unwrap_or(false);
            pref_put(shared, "queueSets", serde_json::json!(now));
            shared.borrow_mut().ap_failed = None;
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if now { "Sets & Podcasts are back in the queue and autoplay." } else { "Sets & Podcasts are out of the queue and autoplay." }.into()); }
        }
        "state" => {}
        "mix-good" | "mix-bad" => if let Err(e) = mix_feedback(shared, action == "mix-good") { note = Some(e) },
        // (the Suite's Raid Shoutout: its latest raider shown, said back on the app's status line)
        "raid-shoutout" => {
            match plugrun::get(botpage::BOT) {
                Some(p) => {
                    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
                    rt.spawn(async move {
                        let said = p.call("suiteShoutout", vec![]).await.map(|v| v.as_str().unwrap_or("").to_string()).unwrap_or_else(|e| e);
                        let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(said.into()));
                    });
                }
                None => note = Some("Turn the Stream Suite on first."),
            }
        }
        // (a Chatbot Action key: that action run, as the Actions page's Run Now runs it)
        a if a.starts_with("chatbot:") => match plugrun::get(BOT) {
            Some(p) => {
                let (name, rt, weak) = (a[8..].to_string(), shared.borrow().rt.clone(), shared.borrow().ui.clone());
                rt.spawn(async move {
                    let r = p.call("runAction", vec![serde_json::json!(name)]).await;
                    let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(match r { Ok(v) => v.as_str().map(String::from).unwrap_or_else(|| "Done.".into()), Err(e) => e }.into()));
                });
            }
            None => note = Some("The chatbot isn't on. Turn it on in Settings > Plugins."),
        },
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
/// Where a request's answer goes: the requests server (to the bot that asked), or a plugin (a chatbot)
type ReqReply = std::sync::Arc<dyn Fn(serde_json::Value) + Send + Sync>;

fn to_server(id: u64) -> ReqReply { std::sync::Arc::new(move |v| omnipulse_core::requests::request_result(id, v)) }

/// Song requests open: Settings > Streaming's switch, or, from the OmniPulse Chatbot while it's on, the
/// chatbot's own (its Song Requests, in its page: Streaming's is put away while it's on)
fn req_open(prefs: &serde_json::Value, from_bot: bool) -> bool {
    if from_bot && plugin_runs(prefs, botpage::BOT) { return prefs["pluginSettings"][botpage::BOT]["requests"].as_bool() != Some(false); }
    integrations::set(prefs, "req")
}

fn wrong_song(shared: &Shared, reply: ReqReply, user: &str, is_mod: bool, from_bot: bool) {
    let prefs = shared.borrow().web.prefs.clone();
    let who = if user.trim().is_empty() { "chat".to_string() } else { user.trim().to_string() };
    let base = vec![("user", who.clone())];
    if !req_open(&prefs, from_bot) { reply(req_fail(&prefs, &base, "closed", vec![])); return; }
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
fn song_request(shared: &Shared, reply: ReqReply, q: String, user: String, from_bot: bool) {
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
    // (and the ones without an account, as search has them: MusicBrainz, the catalogues, plugins)
    let yt_ok = omnipulse_core::tools::ytdlp_path().is_some();
    if yt_ok && sources::MUSICBRAINZ.load(std::sync::atomic::Ordering::Relaxed) { plats.push("musicbrainz"); }
    if yt_ok && sources::CATALOG.load(std::sync::atomic::Ordering::Relaxed) { plats.push("catalog"); }
    if !plugrun::players().is_empty() { plats.push("plugins"); }
    let names = plats.iter().map(|p| match *p { "soundcloud" => "SoundCloud", "spotify" => "Spotify", "local" => "your files", "musicbrainz" => "MusicBrainz", "catalog" => "the catalogues", "plugins" => "your plugins", _ => "YouTube Music" }).collect::<Vec<_>>().join(" or ");
    let base = vec![("user", who.clone()), ("query", q.clone()), ("platform", names)];
    if !req_open(&prefs, from_bot) || plats.is_empty() { reply(req_fail(&prefs, &base, "closed", vec![])); return; }
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
    // Keep Out AI Music: its list ready first; what's turned away for being AI noted, for its own reply
    let no_ai = integrations::set(&prefs, "reqNoAi");
    let ai_seen = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    rt.spawn(async move {
        if no_ai { aimusic::refresh().await; }
        let safe = { let (p, seen) = (prefs.clone(), ai_seen.clone()); move |t: &Track| {
            if !streamsafe::allowed(&p, t) { return false; }
            if no_ai && aimusic::is_ai(t) { seen.store(true, std::sync::atomic::Ordering::Relaxed); return false; }
            true
        } };
        let picked = requests::pick(&q, &plats, &likes, &local, prefer_radio, &safe).await;
        // (nothing but AI music found: said so, not "not safe")
        let picked = match picked { Err("notSafe") if ai_seen.load(std::sync::atomic::Ordering::Relaxed) => Err("noAi"), p => p };
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
const SET_ROWS: [(&str, &str, &str, &str, bool); 29] = [
    ("Library", "Sets & Podcasts in the Queue", "Lists and autoplay include anything over 15 minutes. When this is off, they only play when you pick one.", "queueSets", false),
    ("Library", "Autoplay", "Plays similar tracks when the queue runs out.", "autoplay", true),
    ("Library", "Hidden & Not Recommended", "Right-click a track to stop it, its album or its artist from being recommended. When this is off, nothing is left out, but your lists are kept.", "hideOn", true),
    ("Library", "Avoid Recent Plays", "Skips songs you've heard lately in what the app picks for you. What you choose yourself still plays.", "avoidRecent", true),
    ("Sound", "Seamless Mix (WIP)", "Still being built. Songs change over like DJ software does it: on the beat and on a phrase, with tempo and key matched. Use the gear to adjust it. When it's off, songs change over as Fade Between Tracks is set.", "smartOn", false),
    ("Library", "Skip non-music parts", "When music plays from YouTube, skips the parts of music videos that aren't music, like a skit before the song or an interview after it, as marked on SponsorBlock.", "sbOn", true),
    ("Library", "Favourites Page", "Your starred tracks on a page of their own, in the sidebar. The stars stay either way.", "favPage", true),
    ("Library", "Sets & Podcasts Page", "A page for anything over 15 minutes, in the sidebar. Right-click it there to hide it.", "setsOn", true),
    ("Library", "Stats", "Your listening, in the sidebar. Right-click it there to hide it.", "statsShown", true),
    ("Integrations", "SoundCloud", "Your likes, playlists and search.", "scOn", true),
    ("Integrations", "Spotify", "Your likes, playlists and search.", "spOn", true),
    ("Integrations", "YouTube Music", "Search, radio, and Go+ tracks in full.", "ytOn", true),
    ("Integrations", "Deezer and Apple Music catalogues", "Open music catalogues, with no account needed. Search, Discover and artist pages find their songs, and they play from YouTube Music.", "catOn", true),
    ("Integrations", "MusicBrainz", "An open music catalogue. Search finds its songs too, and they play from YouTube Music.", "mbOn", true),
    ("Integrations", "Listen Along & Jams", "Listen together with friends, from your Discord status or a Jam invite. When this is off, there's no Jam button, and invites and requests are turned away. Song requests aren't affected.", "jamsOn", true),
    ("Integrations", "Last.fm Scrobbling", "What you play goes to your Last.fm profile.", "scrobble", true),
    ("Integrations", "Like Other Tracks There Too", "When your YouTube account is connected, liking a SoundCloud or Spotify track also likes it on YouTube.", "ytMirror", false),
    ("Integrations", "Follow Everywhere", "Following an artist on one platform follows them on the others.", "followEverywhere", true),
    ("Integrations", "Chromecast", "Play to speakers and TVs on your network. They show up with Windows' sound devices.", "castOn", false),
    ("Streaming", "Streamer Mode", "Hides account names, keys, addresses and plugin settings. The title bar's button does this too.", "streamerMode", false),
    ("Streaming", "Extended", "Also leaves out music that isn't safe on stream. Only openly licensed music, and artists and labels cleared for streaming, play.", "streamSafe", false),
    ("Integrations", "Discord Status", "Shows what you're playing on your Discord profile.", "presence", false),
    ("Streaming", "Stream Widget", "A now-playing card for OBS, with alerts.", "widget", false),
    ("Streaming", "Lyrics Widget", "Shows the playing song's lyrics in OBS, karaoke style. The line being sung lights up, with the next lines below it.", "lyricsWidget", false),
    ("Streaming", "Social Plug", "Your handles on other platforms, in turn, for OBS.", "socials", false),
    ("Streaming", "Alerts", "Shows follows, subs, bits, raids and more from your bot, on the widget.", "alertsOn", false),
    ("Streaming", "Song Requests", "Chat asks for songs through your bot.", "req", false),
    ("Streaming", "OmniPulse Offset", "Your music a second time, as its own app (\"OmniPulse Offset\"), to send to a stream mix only, with an offset and a volume of its own. Use this to sync up with mic delay.", "out2On", false),
    ("Streaming", "Stream Deck & Remote", "Control the app from a Stream Deck or another device.", "remoteOn", false),
];

fn set_row_on(prefs: &serde_json::Value, key: &str, dflt: bool) -> bool {
    prefs[key].as_bool().unwrap_or(dflt)
}

fn paint_settings(a: &App) {
    let Some(ui) = a.ui.upgrade() else { return };
    SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { let sh2 = sh.clone(); slint::Timer::single_shot(std::time::Duration::ZERO, move || { helpers_paint(&sh2); apple_paint(&sh2); }); } });
    let streamer = integrations::set(&a.web.prefs, "streamerMode");
    let rows = |sec: &str| -> ModelRc<SetRow> {
        let mut list: Vec<SetRow> = SET_ROWS.iter().filter(|(s, ..)| *s == sec)
            // Extended only shows with Streamer Mode on
            .filter(|(_, _, _, key, _)| *key != "streamSafe" || streamer)
            // (the platforms' own switches are with their accounts)
            .filter(|(_, _, _, key, _)| !matches!(*key, "scOn" | "spOn" | "ytOn" | "scrobble" | "ytMirror"))
            // (Alerts: inside the Stream Widget's page, as its path says, not a row of its own here)
            .filter(|(_, _, _, key, _)| *key != "alertsOn")
            .map(|(_, label, note, key, dflt)| {
                let on = if *key == "autoplay" { a.autoplay } else { set_row_on(&a.web.prefs, key, *dflt) };
                // (its settings: in a panel, with it on; Streamer Mode leaves the addresses and keys out)
                let more = on && panels::of_switch(key).is_some() && !(streamer && matches!(*key, "req" | "remoteOn"));
                SetRow { head: Default::default(), label: (*label).into(), note: (*note).into(), on, key: (*key).into(), sub: matches!(*key, "streamSafe" | "alertsOn"), more, button: Default::default() }
            }).collect();
        // Lyrics: where they come from, and in which order (its own page, as the web app's)
        if sec == "Integrations" {
            list.push(SetRow { head: Default::default(), label: "Lyrics".into(), note: "Where lyrics come from, and in which order.".into(), on: false, key: "lyricsSources".into(), sub: false, more: false, button: "Sources".into() });
        }
        // a plugin whose settings live here (the chatbot's, in Streaming), on on the Plugins page: its own switch
        // (it running or not) and its settings, there either way
        if sec != "Plugins" {
            for p in omnipulse_core::plugins::list() {
                let m = &p.manifest;
                if m.section != sec || !plugin_on(&a.web.prefs, &m.id) { continue; }
                let note = format!("{}{}", if m.description.is_empty() { String::new() } else { format!("{} ", m.description) }, plugin_state(&a.web.prefs, m));
                list.push(SetRow { head: Default::default(), label: place_label(m).into(), note: note.into(), on: plugin_runs(&a.web.prefs, &m.id), key: format!("plugRun_{}", m.id).into(), sub: false, more: !streamer, button: Default::default() });
            }
        }
        // the plugins installed (the web app adds and removes them; each one switches on and off here)
        if sec == "Plugins" && !streamer {
            for p in omnipulse_core::plugins::list() {
                let m = &p.manifest;
                let state = plugin_state(&a.web.prefs, m);
                let home = if m.section.is_empty() { String::new() } else { format!("You'll find its settings in Settings > {}. Turning it off here removes it from there too. ", m.section) };
                let note = format!("{}{}{home}{}", if m.description.is_empty() { String::new() } else { format!("{} ", m.description) }, if p.bundled { "Comes with OmniPulse. " } else { "" }, state);
                // (its settings, from here too: a plugin with a page elsewhere (the chatbot's, in Streaming) goes there, once it's on)
                let more = if m.section.is_empty() { m.settings.as_array().is_some_and(|l| !l.is_empty()) } else { plugin_on(&a.web.prefs, &m.id) };
                list.push(SetRow { head: Default::default(), label: plugin_label(m).into(), note: note.into(), on: plugin_on(&a.web.prefs, &m.id), key: format!("plugOn_{}", m.id).into(), sub: false, more, button: Default::default() });
            }
            // (the plugins: adding one, and taking out the ones you've added; it stays at the top)
            list.insert(0, SetRow { head: Default::default(), label: "Add a Plugin".into(), note: "Music sources made by others. Each is walled off and only reaches the sites it lists.".into(), on: false, key: "plugAdd".into(), sub: false, more: false, button: "Add".into() });
        }
        ModelRc::from(Rc::new(VecModel::from(list)))
    };
    ui.set_set_rows_integrations(rows("Integrations"));
    ui.set_set_rows_library(rows("Library"));
    ui.set_recent_window(a.web.prefs["recentWindow"].as_str().unwrap_or("week").into());
    ui.set_set_rows_sound(rows("Sound"));
    ui.set_set_rows_plugins(rows("Plugins"));
    ui.set_widget_on(set_row_on(&a.web.prefs, "widget", false));
    ui.set_bot_on(plugin_on(&a.web.prefs, botpage::BOT));
    ui.set_widget_port(a.web.prefs["widgetPort"].as_i64().filter(|p| (1024..65536).contains(p)).unwrap_or(7878) as i32);
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
    /// what they were when the page opened (what's changed since: not saved yet)
    static PF_ORIG: RefCell<serde_json::Map<String, serde_json::Value>> = RefCell::new(serde_json::Map::new());
    /// where the page was being left for, while it asks whether to save
    static LEAVE_TO: RefCell<String> = const { RefCell::new(String::new()) };
    /// the chatbot's page: what the chatbot last said is going on, the sheet open, the setup step asked for,
    /// what's been tried in its chat, and the sheet's and step's lines as last drawn (not drawn again unchanged)
    static BOT_STATUS: RefCell<serde_json::Value> = const { RefCell::new(serde_json::Value::Null) };
    static BOT_SHEET: RefCell<String> = const { RefCell::new(String::new()) };
    static BOT_FOCUS: RefCell<String> = const { RefCell::new(String::new()) };
    static BOT_RESTART: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static BOT_TRIED: RefCell<Vec<(String, String)>> = const { RefCell::new(Vec::new()) };
    static BOT_DRAWN: RefCell<(String, String)> = const { RefCell::new((String::new(), String::new())) };
}

/// A field typed in: its pulse, once it's changed (only its own row, so what's being typed stays put)
fn pf_paint_changed(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let Some((_, vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    let model = ui.get_pf_fields();
    for i in 0..model.row_count() {
        let Some(mut row) = model.row_data(i) else { continue };
        let changed = row.kind != "togglecfg" && pf_changed(row.key.as_str());
        if row.changed == changed { continue; }
        row.changed = changed;
        if matches!(row.kind.as_str(), "text" | "password" | "number") {
            row.value = vals.get(row.key.as_str()).map(|v| match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Null => String::new(), x => x.to_string() }).unwrap_or_default().into();
        }
        model.set_row_data(i, row);
    }
}

/// How many of the page's settings are changed and not saved yet
fn pf_changes() -> usize {
    let Some((_, vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return 0 };
    PF_ORIG.with(|o| {
        let o = o.borrow();
        let mut keys: Vec<&String> = vals.keys().chain(o.keys()).collect();
        keys.sort();
        keys.dedup();
        keys.into_iter().filter(|k| vals.get(*k) != o.get(*k)).count()
    })
}

fn pf_changed(key: &str) -> bool {
    let now = PF_EDIT.with(|e| e.borrow().as_ref().and_then(|(_, v)| v.get(key).cloned()));
    now != PF_ORIG.with(|o| o.borrow().get(key).cloned())
}

/// The page's Save, pulsing while there's something to save
fn pf_mark(shared: &Shared) {
    let n = pf_changes();
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_dirty(n > 0); ui.set_pf_changes(n as i32); }
}

/// Leaving the page (Back, its X, a crumb): asked first when something's unsaved
/// Within the Suite (one of its settings pages, or its Widgets or Docks, back to another of its pages): nothing
/// unsaved asked about or lost, as it's all the Suite's, kept until Save
fn suite_inner(shared: &Shared, to: &str) -> bool {
    if !PF_EDIT.with(|e| e.borrow().as_ref().is_some_and(|(id, _)| id == botpage::BOT)) { return false; }
    let target = match to.strip_prefix("crumb:").and_then(|i| i.parse::<usize>().ok()) { Some(i) => PF_CRUMBS.with(|c| c.borrow().get(i).cloned()).unwrap_or_default(), None => to.to_string() };
    let sheet = BOT_SHEET.with(|b| !b.borrow().is_empty());
    let widgets = BOT_WIDGETS.with(|w| !w.borrow().is_empty());
    if target == format!("plugin:{}", botpage::BOT) && (sheet || widgets) {
        BOT_SHEET.with(|b| b.borrow_mut().clear());
        BOT_WIDGETS.with(|w| w.borrow_mut().clear());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_bot_widgets("".into()); }
        bot_sheet_crumbs(shared, "");
        bot_paint(shared);
        return true;
    }
    if (target == "botwidgets" || target == "botdocks" || target == "botcreators") && sheet {
        BOT_SHEET.with(|b| b.borrow_mut().clear());
        bot_widgets_open(shared, &target[3..]);
        bot_paint(shared);
        return true;
    }
    false
}

fn pf_leave(shared: &Shared, to: &str) {
    if suite_inner(shared, to) { return; }
    if PF_EDIT.with(|e| e.borrow().is_some()) && pf_changes() > 0 {
        LEAVE_TO.with(|l| *l.borrow_mut() = to.to_string());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_leave_ask(true); }
        return;
    }
    leave_do(shared, to);
}

fn leave_do(shared: &Shared, to: &str) {
    if let Some(i) = to.strip_prefix("crumb:").and_then(|i| i.parse::<usize>().ok()) { return crumb_go(shared, i); }
    if let Some(name) = to.strip_prefix("panel:") { PF_EDIT.with(|e| e.borrow_mut().take()); return panel_open(shared, name); }
    // (back to the Suite's Widgets or Docks, from one's own page)
    if to == "botwidgets" || to == "botdocks" || to == "botcreators" { PF_EDIT.with(|e| e.borrow_mut().take()); pf_open(shared, botpage::BOT); return bot_widgets_open(shared, &to[3..]); }
    if let Some(id) = to.strip_prefix("plugin:") {
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let (Some(ui), Some(sec)) = (shared.borrow().ui.upgrade(), omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id).map(|p| p.manifest.section).filter(|x| !x.is_empty())) { ui.set_set_cat(sec.into()); }
        return pf_open(shared, id);
    }
    if to == "close" { return settings_close(shared); }
    if to == "back" {
        let up = PF_CRUMBS.with(|c| c.borrow().len());
        if up > 0 { return crumb_go(shared, up - 1); }
    }
    PF_EDIT.with(|e| e.borrow_mut().take());
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); }
}

/// The question's answer: saved and left, left as it was, or stayed; or (the page's own Put Back) as it was, staying
fn leave_choice(shared: &Shared, c: &str) {
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_leave_ask(false); }
    let to = LEAVE_TO.with(|l| std::mem::take(&mut *l.borrow_mut()));
    match c {
        "save" => { pf_save(shared); if !to.is_empty() { leave_do(shared, &to); } }
        "discard" => leave_do(shared, if to.is_empty() { "back" } else { &to }),
        "cancel" => leave_do(shared, "back"),
        "discard-stay" => {
            let orig = PF_ORIG.with(|o| o.borrow().clone());
            PF_EDIT.with(|e| if let Some((_, v)) = e.borrow_mut().as_mut() { *v = orig; });
            BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default());
            pf_paint(shared);
            pf_mark(shared);
        }
        _ => {}
    }
}

// ---------- the chatbot's own page ----------

fn bot_manifest() -> Option<omnipulse_core::plugins::Manifest> {
    // (kept for a few seconds: listing the plugins reads every one's files, its code too, and a click on its
    // page asks for this several times)
    thread_local! { static KEPT: RefCell<Option<(std::time::Instant, Option<omnipulse_core::plugins::Manifest>)>> = const { RefCell::new(None) }; }
    if let Some(m) = KEPT.with(|k| k.borrow().as_ref().filter(|(at, _)| at.elapsed() < std::time::Duration::from_secs(5)).map(|(_, m)| m.clone())) { return m; }
    let m = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == botpage::BOT).map(|p| p.manifest);
    KEPT.with(|k| *k.borrow_mut() = Some((std::time::Instant::now(), m.clone())));
    m
}

/// The chatbot's settings as the page shows them: as saved, the page's changes over them
fn bot_current(shared: &Shared) -> serde_json::Map<String, serde_json::Value> {
    let saved = bot_manifest().map(|m| plugrun::settings_of(&m, &shared.borrow().web.prefs)).and_then(|v| v.as_object().cloned()).unwrap_or_default();
    let edit = PF_EDIT.with(|e| e.borrow().as_ref().filter(|(id, _)| id == botpage::BOT).map(|(_, v)| v.clone())).unwrap_or_default();
    PF_ORIG.with(|o| botpage::current(&saved, &edit, &o.borrow()))
}

fn bot_fields(list: Vec<botpage::Field>) -> (Vec<BotField>, String) {
    // (a page with a preview is split: its column narrow, as Panels' is)
    let split_page = list.iter().any(|x| x.kind == "suiteprev");
    let sig = list.iter().map(|x| format!("{}|{}|{}|{}|{}|{}", x.key, x.kind, x.value, x.label, x.extra, if x.kind == "suiteprev" { format!("{}{}", x.note, x.n) } else { String::new() })).collect::<Vec<_>>().join("\n");
    // (a row of choices: its extra, as "id=Name|id=Name", for the page to draw)
    let strs = |v: Vec<String>| ModelRc::from(Rc::new(VecModel::from(v.into_iter().map(slint::SharedString::from).collect::<Vec<_>>())));
    let rows = list.into_iter().map(|x| {
        let (ids, names): (Vec<String>, Vec<String>) = if x.kind == "chips" || x.kind == "buttons" { x.extra.split('|').filter_map(|o| o.split_once('=')).map(|(i, n)| (i.to_string(), n.to_string())).unzip() }
            // (a list: its items, from what's kept, or the default in extra)
            else if x.kind == "list" || x.kind == "piclist" { (Vec::new(), botpage::list_items(if x.value.is_empty() { &x.extra } else { &x.value })) }
            else { (Vec::new(), Vec::new()) };
        // (a Suite preview's colours: "#a,#b" in its note; none, the accent)
        let (c1, c2) = x.note.split_once(',').filter(|_| x.kind == "suiteprev" && !x.note.is_empty())
            .map(|(a, b)| (look::Rgb::parse(a).map(|c| c.slint()).unwrap_or_default(), look::Rgb::parse(b).map(|c| c.slint()).unwrap_or_default()))
            .unwrap_or_default();
        // (a split page's column is narrow: a row of choices too wide for it, a dropdown, so none's cut off)
        let split = BOT_SHEET.with(|b| matches!(b.borrow().as_str(), "panels" | "cam")) || split_page;
        let wide = names.iter().map(|n| n.chars().count() as f32 * 7.4 + 34.0).sum::<f32>() > 440.0;
        let (kind, n) = if x.kind == "chips" && split && wide { ("choose", ids.iter().position(|i| *i == x.value).map_or(-1, |i| i as i32)) }
            // (a link row: 1 when it opens a web page, so it shows as one; an action here shows as done here)
            else if x.kind == "link" { (x.kind, i32::from(x.extra.starts_with("url:") || x.extra.starts_with("http"))) }
            else { (x.kind, x.n) };
        BotField { changed: !x.key.is_empty() && !x.key.contains(':') && pf_changed(&x.key), key: x.key.into(), kind: kind.into(), label: x.label.into(), note: x.note.into(), value: x.value.into(), extra: x.extra.into(), n, opts: strs(names), opt_ids: strs(ids), c1, c2 }
    }).collect();
    (rows, sig)
}

fn bot_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let cur = bot_current(shared);
    let st = BOT_STATUS.with(|b| b.borrow().clone());
    let running = plugrun::get(botpage::BOT).is_some() && st["running"].as_bool() != Some(false);
    fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> { ModelRc::from(Rc::new(VecModel::from(v))) }
    let setup = !matches!(cur.get("setupDone"), Some(serde_json::Value::Bool(true)));
    ui.set_bot_setup(setup);
    let (name, sub, stats) = botpage::card(&cur, &st, running);
    ui.set_bot_name(name.into());
    ui.set_bot_sub(sub.into());
    ui.set_bot_stats(stats.into());
    ui.set_bot_running(running);
    // (a page of its own open (a widget's, say): the Suite's overview under it isn't seen, so it isn't drawn
    // again with each change there; it is when the page closes)
    let sheet_open = BOT_SHEET.with(|b| !b.borrow().is_empty());
    let focus = BOT_FOCUS.with(|f| f.borrow().clone());
    let now = botpage::step_now(&cur, &focus);
    if !sheet_open {
    ui.set_bot_tiles(model(botpage::tiles(&cur, &st, running).into_iter().map(|(id, name, state, l1, l2)| BotTile { id: id.into(), name: name.into(), state: state.into(), line1: l1.into(), line2: l2.into() }).collect()));
    ui.set_bot_feats(model(botpage::feats(&cur, &st).into_iter().map(|(id, name, note, on, open)| BotFeat { id: id.into(), name: name.into(), note: note.into(), on, open, changed: pf_changed(botpage::key_of(&format!("feat:{id}"))) && id != "actions" && id != "games" }).collect()));
    ui.set_bot_steps(model(botpage::steps(&cur).into_iter().map(|(id, title, note, done, optional)| BotStep { id: id.into(), title: title.into(), note: note.into(), state: botpage::step_state(&cur, id, done, &now).into(), optional }).collect()));
    let log: Vec<BotLine> = st["recent"].as_array().into_iter().flatten().take(8).map(|l| BotLine { who: l["who"].as_str().unwrap_or("").into(), heard: l["heard"].as_str().unwrap_or("").into(), did: l["did"].as_str().unwrap_or("").into(), platform: l["platform"].as_str().unwrap_or("twitch").into() }).collect();
    ui.set_bot_log(model(log));
    ui.set_bot_examples(model(botpage::examples(&cur).into_iter().map(slint::SharedString::from).collect()));
    ui.set_bot_tried(model(BOT_TRIED.with(|t| t.borrow().iter().map(|(w, x)| BotLine { who: w.as_str().into(), heard: Default::default(), did: x.as_str().into(), platform: "twitch".into() }).collect())));
    }
    // (a sheet's and the step's lines: drawn again only when they've changed, so what's being typed stays put)
    let sheet = BOT_SHEET.with(|b| b.borrow().clone());
    let (title, mut lines) = botpage::sheet(&sheet, &cur);
    { let m = theme_accents(&shared.borrow().web.prefs); for l in lines.iter_mut().filter(|l| l.kind == "suiteprev" && l.note.is_empty()) { l.note = format!("{},{}", m.0, m.1); } }
    let port = shared.borrow().web.prefs["widgetPort"].as_u64().unwrap_or(7878);
    for l in lines.iter_mut() { if l.extra.contains("{port}") { l.extra = l.extra.replace("{port}", &port.to_string()); } }
    // (Game HUD Frames' editor: the screenshot, and each frame as fractions of it)
    if sheet == "hud" {
        let g = ui.global::<HudLook>();
        let path = cur.get("hudShotPath").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let had = HUD_SHOT.with(|h| h.borrow().clone());
        if had != path {
            g.set_shot(slint::Image::load_from_path(std::path::Path::new(&path)).unwrap_or_default());
            HUD_SHOT.with(|h| *h.borrow_mut() = path.clone());
        }
        let (iw, ih) = (cur.get("hudW").and_then(|v| v.as_f64()).unwrap_or(1920.0) as f32, cur.get("hudH").and_then(|v| v.as_f64()).unwrap_or(1080.0) as f32);
        g.set_ratio(ih / iw.max(1.0));
        let sel = cur.get("hudSel").and_then(|v| v.as_f64()).unwrap_or(-1.0) as i64;
        let rects: Vec<HudRect> = hud::frames(cur.get("hudFrames").and_then(|v| v.as_str()).unwrap_or("[]")).iter().enumerate().map(|(i, f)| {
            let a = |k: &str| f[k].as_f64().unwrap_or(0.0) as f32;
            HudRect { x: a("x") / iw, y: a("y") / ih, w: a("w") / iw, h: a("h") / ih, label: f["label"].as_str().unwrap_or("").into(), picked: i as i64 == sel }
        }).collect();
        g.set_frames(ModelRc::from(Rc::new(VecModel::from(rects))));
        let t = ui.global::<Theme>();
        g.set_c1(botpage::suite_colours(&cur).and_then(|(a, _)| look::Rgb::parse(&a)).or_else(|| look::Rgb::parse(&theme_accents(&shared.borrow().web.prefs).0)).map(|c| c.slint()).unwrap_or(t.get_accent()));
    }
    // (the panels' preview: each one's title and line, the set's look)
    if sheet == "panels" {
        let g = ui.global::<PanelLook>();
        let list = cur.get("panelList").and_then(|v| v.as_str()).filter(|v| !v.trim().is_empty()).unwrap_or("About Me | My Music | PC Specs | Sponsors | Donate").to_string();
        let ext = cur.get("panelExtended").and_then(|v| v.as_bool()) == Some(true);
        let icons = cur.get("panelIcons").and_then(|v| v.as_bool()) != Some(false);
        // (a line too long for beside its title (about 36 letters there, 44 without an icon): extended, its lines
        // as wrapped at about 44 letters, a new paragraph at each " / ")
        let item = |title: &str, line: &str| {
            let long = !line.is_empty() && (ext || line.chars().count() > if icons { 36 } else { 44 });
            let lines = line.split(" / ").map(|p| p.chars().count().div_ceil(44).max(1)).sum::<usize>() as i32;
            PanelItem { title: title.into(), line: line.replace(" / ", "\n").into(), body: long, lines, icon: botpage::panel_icon(title).into(), img: Default::default(), has_img: false }
        };
        let mut items: Vec<PanelItem> = list.split('|').map(|p| p.trim()).filter(|p| !p.is_empty()).take(12).map(|p| match p.split_once(':') { Some((a, b)) => item(a.trim(), b.trim()), None => item(p, "") }).collect();
        // (your own icons, in the panels' order: a picture copied in, shown in its place)
        let own = cur.get("panelOwnIcons").and_then(|v| v.as_str()).unwrap_or("").to_string();
        for (it, u) in items.iter_mut().zip(own.split('|').map(|u| u.trim().to_string())) {
            if let Some(name) = u.strip_prefix("/overlay/") {
                // (each picture read from disk once, not on every change to the page)
                thread_local! { static PICS: RefCell<HashMap<String, slint::Image>> = RefCell::new(HashMap::new()); }
                let img = PICS.with(|p| p.borrow().get(name).cloned()).or_else(|| {
                    let img = omnipulse_core::data_dir().ok().map(|d| d.join("overlay-media").join(name)).and_then(|f| slint::Image::load_from_path(&f).ok())?;
                    PICS.with(|p| p.borrow_mut().insert(name.to_string(), img.clone()));
                    Some(img)
                });
                if let Some(img) = img { it.img = img; it.has_img = true; }
            }
        }
        if cur.get("panelPowered").and_then(|v| v.as_bool()) != Some(false) { items.push(item("Powered by OmniPulse", if ext { "This stream's music, chatbot, alerts and overlays all run on OmniPulse." } else { "Music, chatbot and stream tools in one app" })); }
        g.set_items(ModelRc::from(Rc::new(VecModel::from(items))));
        let or = |k: &str, d: &str| cur.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).unwrap_or(d).to_string();
        g.set_theme(or("panelTheme", "dark").into());
        g.set_shape(or("panelShape", "rounded").into());
        g.set_height(or("panelHeight", "medium").into());
        g.set_icons(cur.get("panelIcons").and_then(|v| v.as_bool()) != Some(false));
        g.set_logo_colours(or("panelLogoColours", "theme").into());
        let t = ui.global::<Theme>();
        let (c1, c2) = botpage::panel_colours(&cur).map(|(a, b)| (look::Rgb::parse(&a).map(|c| c.slint()).unwrap_or(t.get_accent()), look::Rgb::parse(&b).map(|c| c.slint()).unwrap_or(t.get_accent_2()))).unwrap_or_else(|| { let m = theme_accents(&shared.borrow().web.prefs); (look::Rgb::parse(&m.0).map(|c| c.slint()).unwrap_or(t.get_accent()), look::Rgb::parse(&m.1).map(|c| c.slint()).unwrap_or(t.get_accent_2())) });
        g.set_c1(c1);
        g.set_c2(c2);
        // (your own colours for the words and the background, if they're set)
        let own = |k: &str| botpage::hex_of(&cur, k).and_then(|h| look::Rgb::parse(&h)).map(|c| c.slint());
        // (your words' colour, unless it'd be lost on the background: black or white instead, as panels.html)
        let plain = |th: &str| -> Option<slint::Color> {
            let hex = match th { "dark" => "#14111f", "neon" => "#080614", "comic" => "#ffffff", "y2k" => "#fdf6ff", "pastel" => "#fff6fb", "pixel" => "#1b1030", "paper" => "#fbf6e9", "bauhaus" => "#f2ede4",
                "terminal" => "#0a0f0c", "watercolour" => "#fffdf8", "galaxy" => "#05030f", "ticket" => "#ffe9c7", "synthwave" => "#2a0846", "holo" => "#e9e0ff", "cyber" => "#0b1020", "glass" => "#2a2733", _ => return None };
            look::Rgb::parse(hex).map(|c| c.slint())
        };
        let rel = |c: slint::Color| { let l = |v: u8| { let v = v as f64 / 255.0; if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) } }; 0.2126 * l(c.red()) + 0.7152 * l(c.green()) + 0.0722 * l(c.blue()) };
        let ink = own("panelInk").map(|ink| match own("panelBg").or_else(|| plain(&or("panelTheme", "dark"))) {
            Some(bg) => { let (a, b) = (rel(ink), rel(bg)); if (a.max(b) + 0.05) / (a.min(b) + 0.05) < 3.0 { if b > 0.18 { slint::Color::from_rgb_u8(0x11, 0x11, 0x11) } else { slint::Color::from_rgb_u8(0xff, 0xff, 0xff) } } else { ink } }
            None => ink,
        });
        g.set_ink_on(ink.is_some());
        g.set_ink(ink.unwrap_or_default());
        g.set_bg_on(own("panelBg").is_some());
        g.set_bg(own("panelBg").unwrap_or_default());
        // (Terminal's type: the first colour if it's bright enough to read on black, as panels.html has it)
        let lum = 0.299 * c1.red() as f32 + 0.587 * c1.green() as f32 + 0.114 * c1.blue() as f32;
        g.set_term_ink(if lum / 255.0 > 0.35 { c1 } else { slint::Color::from_rgb_u8(0x5d, 0xff, 0x9c) });
    }
    // (the webcam overlay's preview: its layout, as the page lays it out)
    if sheet == "cam" {
        let g = ui.global::<CamLook>();
        // (only the webcam's area: round the camera and its parts, with a little room, not the whole canvas;
        // each piece as a fraction of that, and its shape as tall as it is wide)
        let all = botpage::cam_layout(&cur);
        let (mut x0, mut y0, mut x1, mut y1) = (1f32, 1f32, 0f32, 0f32);
        for (_, _, x, y, w, h) in &all { x0 = x0.min(*x); y0 = y0.min(*y); x1 = x1.max(x + w); y1 = y1.max(y + h); }
        let pad = 0.02;
        let (x0, y0, x1, y1) = ((x0 - pad).max(0.0), (y0 - pad * 1.78).max(0.0), (x1 + pad).min(1.0), (y1 + pad * 1.78).min(1.0));
        let (bw, bh) = ((x1 - x0).max(0.05), (y1 - y0).max(0.05));
        let rects: Vec<CamRect> = all.into_iter().map(|(id, label, x, y, w, h)| CamRect { id: id.into(), label: label.into(), x: (x - x0) / bw, y: (y - y0) / bh, w: w / bw, h: h / bh }).collect();
        g.set_rects(ModelRc::from(Rc::new(VecModel::from(rects))));
        // (its height for its width, as the canvas is 16:9)
        g.set_ratio((bh * 9.0) / (bw * 16.0));
        let or = |k: &str, d: &str| cur.get(k).and_then(|v| v.as_str()).filter(|v| !v.is_empty()).unwrap_or(d).to_string();
        g.set_shape(or("camShape", "rounded").into());
        g.set_border(or("camBorder", "glow").into());
        g.set_theme(or("camTheme", "dark").into());
        let t = ui.global::<Theme>();
        let (c1, c2) = botpage::suite_colours(&cur).map(|(a, b)| (look::Rgb::parse(&a).map(|c| c.slint()).unwrap_or(t.get_accent()), look::Rgb::parse(&b).map(|c| c.slint()).unwrap_or(t.get_accent_2()))).unwrap_or_else(|| { let m = theme_accents(&shared.borrow().web.prefs); (look::Rgb::parse(&m.0).map(|c| c.slint()).unwrap_or(t.get_accent()), look::Rgb::parse(&m.1).map(|c| c.slint()).unwrap_or(t.get_accent_2())) });
        g.set_c1(c1);
        g.set_c2(c2);
    }
    let split = sheet != "panels" && sheet != "cam" && lines.iter().any(|l| l.kind == "suiteprev");
    let (rows, sig) = bot_fields(lines);
    let (srows, ssig) = bot_fields(if setup { botpage::step_fields(&now, &cur) } else { Vec::new() });
    let (was, swas) = BOT_DRAWN.with(|d| d.borrow().clone());
    if sig != was || sheet != ui.get_bot_sheet().as_str() {
        let same = sheet == ui.get_bot_sheet().as_str();
        let now = ui.get_bot_sheet_fields();
        let fits = same && now.row_count() == rows.len() && (0..rows.len()).all(|i| now.row_data(i).is_some_and(|r| r.kind == rows[i].kind && r.key == rows[i].key));
        if fits { for (i, r) in rows.into_iter().enumerate() { if now.row_data(i).as_ref() != Some(&r) { now.set_row_data(i, r); } } }
        else { ui.set_bot_sheet_fields(model(rows)); }
    }
    if ssig != swas { ui.set_bot_step_fields(model(srows)); }
    BOT_DRAWN.with(|d| *d.borrow_mut() = (sig, ssig));
    // (a widget's page with a preview: split, the preview kept in sight on the right)
    ui.set_bot_sheet_split(split);
    ui.set_bot_sheet(sheet.into());
    ui.set_bot_sheet_title(title.into());
}

/// What the chatbot says is going on: asked now, then every couple of seconds while its page is open
fn bot_poll(shared: &Shared) {
    let open = shared.borrow().ui.upgrade().map(|ui| ui.get_pf_open() && ui.get_bot_page()).unwrap_or(false);
    if !open { return; }
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    if let Some(p) = plugrun::get(botpage::BOT) {
        rt.spawn(async move {
            let st = p.call("status", vec![]).await.unwrap_or(serde_json::Value::Null);
            let _ = weak.upgrade_in_event_loop(move |_| {
                BOT_STATUS.with(|b| *b.borrow_mut() = st);
                SHARED.with(|s| if let Some(sh) = s.borrow().clone() { bot_paint(&sh); });
            });
        });
    } else {
        BOT_STATUS.with(|b| *b.borrow_mut() = serde_json::Value::Null);
        bot_paint(shared);
    }
    let s = shared.clone();
    slint::Timer::single_shot(std::time::Duration::from_secs(2), move || bot_poll(&s));
}

/// The chatbot's settings kept at once, without its page's Save (its setup's progress: nothing to restart for)
fn bot_keep(shared: &Shared, key: &str, value: serde_json::Value) {
    let all = {
        let mut a = shared.borrow_mut();
        let mut all = a.web.prefs["pluginSettings"].as_object().cloned().unwrap_or_default();
        let mut mine = all.get(botpage::BOT).and_then(|v| v.as_object()).cloned().unwrap_or_default();
        mine.insert(key.into(), value.clone());
        all.insert(botpage::BOT.into(), serde_json::Value::Object(mine));
        let all = serde_json::Value::Object(all);
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pluginSettings".into(), all.clone()); }
        all
    };
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pluginSettings", all); }
    PF_EDIT.with(|e| if let Some((_, v)) = e.borrow_mut().as_mut() { v.insert(key.into(), value.clone()); });
    PF_ORIG.with(|o| { o.borrow_mut().insert(key.into(), value); });
}

/// What setup kept as it went: the chatbot restarted with it (once you move on, so a sign-in isn't cut off)
fn bot_restart_kept(shared: &Shared) {
    if !BOT_RESTART.with(|r| r.replace(false)) { return; }
    if plugrun::get(botpage::BOT).is_some() { plugrun::stop(botpage::BOT); plugins_start(shared); }
}

thread_local! {
    /// (the HUD editor's screenshot, as loaded: its file)
    static HUD_SHOT: RefCell<String> = const { RefCell::new(String::new()) };
    /// (a nudge's step: 1 pixel, or 10)
    static HUD_STEP: std::cell::Cell<f64> = const { std::cell::Cell::new(1.0) };
}

/// The panels' look and list, as a template keeps them
const PANEL_TPL_KEYS: [&str; 14] = ["panelTheme", "panelShape", "panelHeight", "panelIcons", "panelExtended", "panelList", "suiteColors", "suiteC1", "suiteC2", "panelColors", "panelC1", "panelC2", "panelInk", "panelBg"];

fn bot_set(shared: &Shared, key: &str, v: &str) {
    let key = botpage::key_of(key).to_string();
    // (a theme's font: "theme:font" picked, or typed (any Google Font), kept in themeFonts for that theme)
    if key == "fontPick" || key == "fontCustom" {
        let cur = bot_current(shared);
        let theme = if key == "fontPick" { v.split_once(':').map(|x| x.0).unwrap_or("dark").to_string() } else {
            ["suiteTheme", "panelTheme", "camTheme"].iter().find(|k| botpage::sheet(&BOT_SHEET.with(|b| b.borrow().clone()), &cur).1.iter().any(|f| f.key == **k)).and_then(|k| cur.get(*k).and_then(|x| x.as_str()).filter(|x| !x.is_empty()).map(String::from)).unwrap_or_else(|| "dark".into())
        };
        let font: String = if key == "fontPick" { v.split_once(':').map(|x| x.1).unwrap_or("").to_string() } else { v.trim().chars().filter(|c| c.is_alphanumeric() || *c == ' ').take(60).collect() };
        let mut all: serde_json::Map<String, serde_json::Value> = cur.get("themeFonts").and_then(|t| t.as_str()).and_then(|t| serde_json::from_str(t).ok()).unwrap_or_default();
        if font.is_empty() { all.remove(&theme); } else { all.insert(theme, serde_json::json!(font)); }
        bot_set(shared, "themeFonts", &serde_json::Value::Object(all).to_string());
        if key == "fontPick" { bot_paint(shared); }
        return;
    }
    // (a HUD frame picked from its list, as a click on it is)
    if key == "hudPick" { bot_set(shared, "hudSel", v); bot_paint(shared); return; }
    // (a HUD frame's own (the picked one's), or a nudge: in hudFrames, not settings of their own)
    if key.starts_with("hudF_") || key == "hudNudge" {
        let cur = bot_current(shared);
        let mut frames = hud::frames(cur.get("hudFrames").and_then(|v| v.as_str()).unwrap_or("[]"));
        let sel = cur.get("hudSel").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        let Some(fr) = (sel >= 0.0).then(|| frames.get_mut(sel as usize)).flatten() else { return };
        let mut paint = true;
        match key.as_str() {
            "hudF_style" => fr["style"] = serde_json::json!(v),
            "hudF_label" => { fr["label"] = serde_json::json!(v); paint = false; }
            "hudF_pad" => fr["pad"] = serde_json::json!(v.parse::<f64>().unwrap_or(6.0)),
            "hudF_pulse" => fr["pulse"] = serde_json::json!(v == "true"),
            _ => {
                let step = HUD_STEP.with(|s| s.get());
                if v == "big" { HUD_STEP.with(|s| s.set(if step > 1.0 { 1.0 } else { 10.0 })); bot_paint(shared); return; }
                let at = |k: &str| fr[k].as_f64().unwrap_or(0.0);
                let (x, y, w, h) = (at("x"), at("y"), at("w"), at("h"));
                let (x, y, w, h) = match v { "left" => (x - step, y, w, h), "right" => (x + step, y, w, h), "up" => (x, y - step, w, h), "down" => (x, y + step, w, h),
                    "wider" => (x - step / 2.0, y, w + step, h), "narrower" => (x + step / 2.0, y, (w - step).max(4.0), h), "taller" => (x, y - step / 2.0, w, h + step), "shorter" => (x, y + step / 2.0, w, (h - step).max(4.0)), _ => (x, y, w, h) };
                fr["x"] = serde_json::json!(x.round()); fr["y"] = serde_json::json!(y.round()); fr["w"] = serde_json::json!(w.round()); fr["h"] = serde_json::json!(h.round());
            }
        }
        bot_set(shared, "hudFrames", &serde_json::Value::Array(frames).to_string());
        if paint { bot_paint(shared); }
        return;
    }
    // (a panel template loaded (its settings set again) or deleted, not kept as a setting)
    if key == "panelTplLoad" || key == "panelTplDel" {
        let cur = bot_current(shared);
        let mut all: Vec<serde_json::Value> = cur.get("panelTemplates").and_then(|t| t.as_str()).and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).and_then(|t| t.as_array().cloned()).unwrap_or_default();
        let name = v.to_string();
        if key == "panelTplDel" {
            all.retain(|t| t["name"].as_str() != Some(name.as_str()));
            bot_set(shared, "panelTemplates", &serde_json::Value::Array(all).to_string());
        } else if let Some(t) = all.iter().find(|t| t["name"].as_str() == Some(name.as_str())) {
            for k in PANEL_TPL_KEYS {
                let val = &t["set"][k];
                let s = match val { serde_json::Value::String(s) => s.clone(), serde_json::Value::Bool(b) => b.to_string(), serde_json::Value::Null => continue, x => x.to_string() };
                bot_set(shared, k, &s);
            }
            bot_set(shared, "panelTplName", &name);
        }
        bot_paint(shared);
        return;
    }
    let manifest = bot_manifest().map(|m| m.settings).unwrap_or_default();
    let value = botpage::value_of(&manifest, &key, v);
    // (a choice, like a theme's chip: seen at once too, so the preview shows it)
    let select = manifest.as_array().into_iter().flatten().any(|x| x["key"] == key.as_str() && x["type"] == "select");
    let cur = bot_current(shared);
    // (and anything picked on the page (a row of chips, a dropdown): seen at once; only what's typed waits)
    let picked = botpage::sheet(&BOT_SHEET.with(|b| b.borrow().clone()), &cur).1.iter().any(|f| f.key == key && f.kind == "chips");
    let toggle = value.is_boolean() || select || picked || matches!(key.as_str(), "srWho" | "goalReset" | "goalStyle" | "goalShape" | "goalDrops");
    // (during setup: kept as it's set, no Save needed; the chatbot restarted with it when you move on)
    let in_setup = !cur.get("setupDone").and_then(|v| v.as_bool()).unwrap_or(false)
        && botpage::steps(&cur).iter().any(|x| botpage::step_fields(x.0, &cur).iter().any(|f| f.key == key));
    if in_setup {
        bot_keep(shared, &key, value);
        BOT_RESTART.with(|r| r.set(true));
        pf_mark(shared);
        if toggle { bot_paint(shared); }
        return;
    }
    PF_EDIT.with(|e| if let Some((_, vals)) = e.borrow_mut().as_mut() { vals.insert(key, value); });
    pf_mark(shared);
    // (a switch, or who can request: seen at once, pulsing; typing: left as it's typed)
    if toggle { bot_paint(shared); }
}

/// A webhook's own id on the relay (Ko-fi's, Throne's, Fourthwall's): 24 random letters and digits, its secret
fn hook_id() -> String {
    (0..24u64).map(|k| { use std::hash::{BuildHasher, Hasher}; let mut h = std::collections::hash_map::RandomState::new().build_hasher(); h.write_u64(k); h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)); char::from_digit((h.finish() % 36) as u32, 36).unwrap_or('0') }).collect()
}

fn bot_act(shared: &Shared, act: &str) {
    let say = |t: &str| if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(t.into()); };
    // (a list's row typed into, taken out, or one added: kept as "a | b | c"; the page drawn again unless it's typing)
    if let Some(rest) = act.strip_prefix("list:") {
        let mut parts = rest.splitn(4, ':');
        let (key, what) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        let cur = bot_current(shared);
        let had = cur.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let shown = botpage::sheet(&BOT_SHEET.with(|b| b.borrow().clone()), &cur).1.into_iter().find(|f| f.key == key).map(|f| f.extra).unwrap_or_default();
        let mut items = botpage::list_items(if had.is_empty() { &shown } else { &had });
        let at = parts.next().and_then(|i| i.parse::<usize>().ok());
        match (what, at) {
            ("set", Some(i)) if i < items.len() => { items[i] = parts.next().unwrap_or("").replace('|', "/"); }
            ("del", Some(i)) if i < items.len() => { items.remove(i); }
            ("add", _) if items.len() < 16 => { items.push(String::new()); }
            // (a picture of yours for the row: chosen, and copied into the widget's own folder (its address there))
            ("pick", Some(i)) => {
                let Some(p) = ask_path(rfd::FileDialog::new().set_title("A picture for this panel's icon").add_filter("Pictures", &["png", "jpg", "jpeg", "gif", "webp"]), Ask::Open, { let (s, a) = (shared.clone(), act.to_string()); move || bot_act(&s, &a) }) else { return };
                match omnipulse_core::widget::overlay_media(&p.to_string_lossy(), false) {
                    Ok(url) => { while items.len() <= i { items.push(String::new()); } items[i] = url; }
                    Err(e) => { say(&e); return; }
                }
            }
            _ => return,
        }
        // (an empty row kept while it's being typed into: "a |  | c")
        bot_set(shared, key, &items.join(" | "));
        if what != "set" { bot_paint(shared); }
        return;
    }
    // (Alerts: their settings are the Stream Widget's Alerts page, the same as without the Suite)
    if act == "sheet:alerts" { return panel_open(shared, "alerts"); }
    // (a dock's or the Multichat Overlay's gear: its own page)
    if let Some(p) = act.strip_prefix("dockset:") { return panel_open(shared, p); }
    // (Widgets, or Docks: their own page, under the Suite's)
    if act == "widgets" || act == "docks" || act == "creators" { return bot_widgets_open(shared, act); }
    // (Game HUD Frames: a screenshot chosen; a box drawn (snapped), a frame picked, snapped again, taken away)
    if let Some(rest) = act.strip_prefix("hud:") {
        let cur = bot_current(shared);
        let mut frames = hud::frames(cur.get("hudFrames").and_then(|v| v.as_str()).unwrap_or("[]"));
        let (iw, ih) = (cur.get("hudW").and_then(|v| v.as_f64()).unwrap_or(1920.0) as f32, cur.get("hudH").and_then(|v| v.as_f64()).unwrap_or(1080.0) as f32);
        let path = cur.get("hudShotPath").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let gray = || image::open(&path).ok().map(|i| i.to_luma8());
        let sel = cur.get("hudSel").and_then(|v| v.as_f64()).unwrap_or(-1.0);
        let nums = |s: &str| s.split(',').filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>();
        // (a screenshot pasted: the clipboard's picture, kept as a PNG in OmniPulse's folder, then as a chosen one)
        let pasted = if rest == "paste" {
            let img = arboard::Clipboard::new().and_then(|mut c| c.get_image());
            let Ok(img) = img else { say("There's no picture on the clipboard. Take one with Win+Shift+S over your game, then paste."); return };
            let Some(buf) = image::RgbaImage::from_raw(img.width as u32, img.height as u32, img.bytes.into_owned()) else { say("That picture couldn't be read."); return };
            let Some(dir) = omnipulse_core::data_dir().ok().map(|d| d.join("hud")) else { return };
            let _ = std::fs::create_dir_all(&dir);
            let file = dir.join(format!("screenshot-{}.png", omnipulse_core::now_secs()));
            if let Err(e) = buf.save(&file) { say(&format!("Couldn't keep the screenshot: {e}")); return; }
            Some(file)
        } else { None };
        if rest == "shot" || pasted.is_some() {
            let p = match pasted { Some(p) => p, None => { let Some(p) = ask_path(rfd::FileDialog::new().set_title("A screenshot of your game").add_filter("Pictures", &["png", "jpg", "jpeg", "webp"]), Ask::Open, { let (s, a) = (shared.clone(), act.to_string()); move || bot_act(&s, &a) }) else { return }; p } };
            let file = p.to_string_lossy().to_string();
            let Ok((w, h)) = image::image_dimensions(&p) else { say("That picture couldn't be read."); return };
            match omnipulse_core::widget::overlay_media(&file, false) {
                Ok(url) => { bot_set(shared, "hudShot", &url); bot_set(shared, "hudShotPath", &file); bot_set(shared, "hudW", &w.to_string()); bot_set(shared, "hudH", &h.to_string()); bot_set(shared, "hudSel", "-1"); }
                Err(e) => { say(&e); return; }
            }
        } else if let Some(r) = rest.strip_prefix("add:") {
            // (a box, as fractions of the screenshot: in its pixels, snapped to the element inside)
            let n = nums(r);
            if n.len() == 4 {
                let (x0, y0, x1, y1) = (n[0].min(n[2]) * iw, n[1].min(n[3]) * ih, n[0].max(n[2]) * iw, n[1].max(n[3]) * ih);
                let (x, y, w, h) = gray().map(|g| hud::snap(&g, x0, y0, x1 - x0, y1 - y0)).unwrap_or((x0, y0, x1 - x0, y1 - y0));
                let f = hud::frame(x, y, w, h, frames.last());
                frames.push(f);
                bot_set(shared, "hudSel", &(frames.len() - 1).to_string());
            }
        } else if let Some(r) = rest.strip_prefix("pick:") {
            let n = nums(r);
            if n.len() == 2 {
                let (px, py) = (n[0] * iw, n[1] * ih);
                let hit = frames.iter().rposition(|f| { let a = |k: &str| f[k].as_f64().unwrap_or(0.0) as f32; px >= a("x") && px <= a("x") + a("w") && py >= a("y") && py <= a("y") + a("h") });
                bot_set(shared, "hudSel", &hit.map(|i| i as i64).unwrap_or(-1).to_string());
            }
        } else if rest == "snap" {
            if let (Some(g), Some(f)) = (gray(), (sel >= 0.0).then(|| frames.get_mut(sel as usize)).flatten()) {
                let a = |k: &str| f[k].as_f64().unwrap_or(0.0) as f32;
                let (x, y, w, h) = hud::snap(&g, a("x"), a("y"), a("w"), a("h"));
                f["x"] = serde_json::json!(x); f["y"] = serde_json::json!(y); f["w"] = serde_json::json!(w); f["h"] = serde_json::json!(h);
            }
        } else if rest == "del" && sel >= 0.0 && (sel as usize) < frames.len() {
            frames.remove(sel as usize);
            bot_set(shared, "hudSel", "-1");
        }
        bot_set(shared, "hudFrames", &serde_json::Value::Array(frames).to_string());
        bot_paint(shared);
        return;
    }
    // (the panels' look and list, saved as a template by its name: one of the same name replaced)
    if act == "tpl:save" {
        let cur = bot_current(shared);
        let name = cur.get("panelTplName").and_then(|v| v.as_str()).map(str::trim).filter(|n| !n.is_empty()).unwrap_or("My panels").to_string();
        let set: serde_json::Map<String, serde_json::Value> = PANEL_TPL_KEYS.iter().filter_map(|k| cur.get(*k).map(|v| (k.to_string(), v.clone()))).collect();
        let mut all: Vec<serde_json::Value> = cur.get("panelTemplates").and_then(|t| t.as_str()).and_then(|t| serde_json::from_str::<serde_json::Value>(t).ok()).and_then(|t| t.as_array().cloned()).unwrap_or_default();
        all.retain(|t| t["name"].as_str() != Some(name.as_str()));
        all.push(serde_json::json!({ "name": name, "set": set }));
        bot_set(shared, "panelTemplates", &serde_json::Value::Array(all).to_string());
        bot_paint(shared);
        say(&format!("Saved the template \"{name}\". Click Save to keep it."));
        return;
    }
    if let Some(id) = act.strip_prefix("sheet:") {
        // (Ko-fi's: a webhook address of your own on the relay, made the first time)
        // (and Throne's and Fourthwall's, each its own, the same way)
        for (kind, key) in [("throne", "throneUrl"), ("fourthwall", "fourthwallUrl")] {
            if (id == "tips" || id == kind) && bot_current(shared).get(key).and_then(|v| v.as_str()).unwrap_or("").is_empty() {
                bot_keep(shared, key, serde_json::json!(format!("{}/api/hook/{kind}/{}", share::RELAY_URL, hook_id())));
            }
        }
        if (id == "tips" || id == "kofi") && bot_current(shared).get("kofiUrl").and_then(|v| v.as_str()).unwrap_or("").is_empty() {
            let hook = hook_id();
            bot_keep(shared, "kofiUrl", serde_json::json!(format!("{}/api/hook/kofi/{hook}", share::RELAY_URL)));
        }
        BOT_SHEET.with(|b| *b.borrow_mut() = id.to_string());
        bot_sheet_crumbs(shared, id);
        return bot_paint(shared);
    }
    if let Some(url) = act.strip_prefix("url:") { if let Err(e) = omnipulse_core::open_url(url.to_string()) { say(&e); } return; }
    // (one goal's own address, for it alone in OBS: /goals/follow, /goals/sub...)
    if let Some(k) = act.strip_prefix("goaladdr:") {
        let id = match k { "goalFollow" => "follow", "goalSub" => "sub", "goalBits" => "bits", "goalDono" => "donation", _ => "common" };
        let port = shared.borrow().web.prefs["widgetPort"].as_u64().unwrap_or(7878);
        let url = format!("http://127.0.0.1:{port}/goals/{id}");
        if share::copy(&url).is_ok() { say(&format!("Copied {url}. Add it as a Browser source for this goal alone.")); }
        return;
    }
    // (a widget switched on or off: kept in widgetsOff, the running Suite told at once)
    if let Some(id) = act.strip_prefix("woff:") {
        let cur = bot_current(shared);
        let mut off: Vec<String> = cur.get("widgetsOff").and_then(|v| v.as_str()).unwrap_or("").split('|').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        // (the hype meter switched off the old way (hypeOn): on again here is on)
        let hype_was_off = id == "hype" && cur.get("hypeOn").and_then(|v| v.as_bool()) == Some(false);
        if hype_was_off { bot_keep(shared, "hypeOn", serde_json::json!(true)); }
        let now_on = if hype_was_off { off.retain(|x| x != id); true } else if let Some(i) = off.iter().position(|x| x == id) { off.remove(i); true } else { off.push(id.to_string()); false };
        let v = off.join("|");
        bot_keep(shared, "widgetsOff", serde_json::json!(v));
        if let Some(p) = plugrun::get(botpage::BOT) {
            let rt = shared.borrow().rt.clone();
            let vals = if hype_was_off { serde_json::json!({ "widgetsOff": v, "hypeOn": true }) } else { serde_json::json!({ "widgetsOff": v }) };
            rt.spawn(async move { let _ = p.call("applySettings", vec![vals]).await; });
        }
        if let Some(ui) = shared.borrow().ui.upgrade() { let w = ui.global::<Wid>(); w.set_rev(w.get_rev() + 1); }
        say(if now_on { "Switched on." } else { "Switched off: its page in OBS shows nothing, and costs nothing." });
        return;
    }
    // (a platform's row in Tips and Donations: opened, or shut)
    if let Some(id) = act.strip_prefix("fold:") {
        botpage::TIPS_OPEN.with(|o| { let mut o = o.borrow_mut(); if !o.remove(id) { o.insert(id.to_string()); } });
        return bot_paint(shared);
    }
    if let Some(text) = act.strip_prefix("copy:") { if share::copy(text).is_ok() { say("Copied."); } return; }
    // (its widgets: switched here, their settings their own pages, asked first if something here isn't saved)
    if let Some(key) = act.strip_prefix("row-toggle:") { set_toggle(shared, key); return bot_paint(shared); }
    if let Some(key) = act.strip_prefix("row-more:") { if let Some(name) = panels::of_switch(key) { pf_leave(shared, &format!("panel:{name}")); } return; }
    if let Some(how) = act.strip_prefix("import:") {
        flows_open(shared);
        // (the window taken first: its Paste and Restore borrow the app's state themselves)
        let ui = shared.borrow().ui.upgrade();
        if let Some(ui) = ui { ui.invoke_flow_act(if how == "paste" { "paste".into() } else { "restore".into() }); }
        return;
    }
    if let Some(rest) = act.strip_prefix("call:") {
        let (method, arg) = rest.split_once(':').unwrap_or((rest, ""));
        let (method, arg) = (method.to_string(), arg.to_string());
        let Some(p) = plugrun::get(botpage::BOT) else { return say("Turn the Suite on first.") };
        let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
        rt.spawn(async move {
            let r = p.call(&method, vec![serde_json::json!(arg)]).await;
            let note = match r { Ok(v) => v.as_str().unwrap_or("Done.").to_string(), Err(e) => e };
            let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(note.into()));
        });
        return;
    }
    if let Some(id) = act.strip_prefix("step:") { BOT_FOCUS.with(|f| *f.borrow_mut() = id.to_string()); BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default()); return bot_paint(shared); }
    match act {
        "run" => { set_toggle(shared, &format!("plugRun_{}", botpage::BOT)); return bot_poll_soon(shared); }
        "actions" => return flows_open(shared),
        "setup:done" => { bot_restart_kept(shared); bot_keep(shared, "setupDone", serde_json::json!(true)); BOT_FOCUS.with(|f| f.borrow_mut().clear()); return bot_paint(shared); }
        "setup:again" => { bot_keep(shared, "setupDone", serde_json::json!(false)); bot_keep(shared, "setupSkip", serde_json::json!([])); BOT_FOCUS.with(|f| f.borrow_mut().clear()); BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default()); return bot_paint(shared); }
        _ => {}
    }
    if let Some(id) = act.strip_prefix("skip:").or_else(|| act.strip_prefix("next:")) {
        // (Next: what's been set on the step saved first; a step not done: skipped for now)
        if act.starts_with("next:") && pf_changes() > 0 { pf_save(shared); }
        bot_restart_kept(shared);
        let cur = bot_current(shared);
        let done = botpage::steps(&cur).into_iter().find(|x| x.0 == id).map(|x| x.3).unwrap_or(false);
        if id == "features" && act.starts_with("next:") { bot_keep(shared, "setupDone", serde_json::json!(true)); }
        else if !done {
            let mut skip: Vec<serde_json::Value> = cur.get("setupSkip").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            if !skip.iter().any(|x| x == id) { skip.push(serde_json::json!(id)); }
            bot_keep(shared, "setupSkip", serde_json::json!(skip));
        }
        BOT_FOCUS.with(|f| f.borrow_mut().clear());
        BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default());
        return bot_paint(shared);
    }
    // an account's icon: its sign-in (here, or its link copied) or its sign-out, with the page's values as they are
    let Some(method) = botpage::method(act) else { return };
    if act.contains(":kick:") && !act.starts_with("out:") { bot_keep(shared, "kickOn", serde_json::json!(true)); pf_mark(shared); }
    let Some(p) = plugrun::get(botpage::BOT) else {
        // (not running: started, then asked again)
        if !plugin_runs(&shared.borrow().web.prefs, botpage::BOT) { set_toggle(shared, &format!("plugRun_{}", botpage::BOT)); }
        plugins_start(shared);
        let (s2, a2) = (shared.clone(), act.to_string());
        say("Starting the chatbot...");
        slint::Timer::single_shot(std::time::Duration::from_millis(900), move || if plugrun::get(botpage::BOT).is_some() { bot_act(&s2, &a2); });
        return;
    };
    let mut vals = bot_current(shared);
    botpage::sign_in_extras(&mut vals, share::RELAY_URL);
    let vals = serde_json::Value::Object(vals);
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let method = method.to_string();
    rt.spawn(async move {
        let r = p.call(&method, vec![vals]).await;
        let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(match r { Ok(v) => v.as_str().map(String::from).unwrap_or_else(|| "Done.".into()), Err(e) => e }.into()));
    });
    bot_poll_soon(shared);
}

fn bot_poll_soon(shared: &Shared) {
    let s = shared.clone();
    slint::Timer::single_shot(std::time::Duration::from_millis(700), move || { BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default()); bot_paint(&s); });
}

/// A line typed in its chat to try it: what it'd say and do, nothing sent, nothing done
fn bot_try(shared: &Shared, text: &str) {
    BOT_TRIED.with(|t| { let mut t = t.borrow_mut(); t.push(("You".into(), text.to_string())); });
    let Some(p) = plugrun::get(botpage::BOT) else {
        BOT_TRIED.with(|t| t.borrow_mut().push(("Bot".into(), "(The chatbot is stopped. Start it with the button on its card to try it.)".into())));
        return bot_paint(shared);
    };
    bot_paint(shared);
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    let text = text.to_string();
    rt.spawn(async move {
        let r = p.call("tryChat", vec![serde_json::json!(text), serde_json::json!("twitch"), serde_json::json!(false)]).await;
        let _ = weak.upgrade_in_event_loop(move |_| {
            let lines: Vec<String> = match r { Ok(v) => v.as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect(), Err(e) => vec![format!("(couldn't: {e})")] };
            BOT_TRIED.with(|t| {
                let mut t = t.borrow_mut();
                if lines.is_empty() { t.push(("Bot".into(), "(Nothing would answer that. No command or action matches it.)".into())); }
                for l in lines { t.push(("Bot".into(), l)); }
                let n = t.len();
                if n > 8 { t.drain(..n - 8); }
            });
            SHARED.with(|s| if let Some(sh) = s.borrow().clone() { bot_paint(&sh); });
        });
    });
}

// ---------- Settings panels (crate::panels): a switch's settings, in the plugins' dialog ----------
thread_local! {
    /// the open panel's fields, as drawn (for what a press, a choice or a copy is of)
    static PANEL_FIELDS: RefCell<Vec<serde_json::Value>> = const { RefCell::new(Vec::new()) };
}

/// The keys the servers need, made once (as the web app makes them when they're switched on)
fn panel_keys(shared: &Shared) {
    let need: Vec<&str> = {
        let p = &shared.borrow().web.prefs;
        let mut v = Vec::new();
        if integrations::set(p, "req") && p["reqKey"].as_str().unwrap_or("").is_empty() { v.push("reqKey"); }
        if integrations::set(p, "remoteOn") && p["remoteKey"].as_str().unwrap_or("").is_empty() { v.push("remoteKey"); }
        if integrations::set(p, "botRemote") && p["alertKey"].as_str().unwrap_or("").is_empty() { v.push("alertKey"); }
        v
    };
    for k in need { pref_put(shared, k, serde_json::json!(panels::new_key())); }
}

fn pref_put(shared: &Shared, key: &str, v: serde_json::Value) {
    if let Some(o) = shared.borrow_mut().web.prefs.as_object_mut() { o.insert(key.into(), v.clone()); }
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(key, v); }
}

thread_local! {
    /// where each part of the open panel's path goes back to ("root", "cat:<category>", "panel:<name>")
    static PF_CRUMBS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    /// Settings' search: what it found, in order
    static SET_HITS: RefCell<Vec<SetFound>> = const { RefCell::new(Vec::new()) };
}

fn crumbs_set(ui: &AppWindow, path: Vec<(String, String)>) {
    ui.set_pf_crumbs(ModelRc::from(Rc::new(VecModel::from(path.iter().map(|(l, _)| slint::SharedString::from(l.as_str())).collect::<Vec<_>>()))));
    ui.set_pf_crumb_chars(path.iter().map(|(l, _)| l.chars().count() as i32).sum());
    PF_CRUMBS.with(|c| *c.borrow_mut() = path.into_iter().map(|(_, t)| t).collect());
}

/// A part of the panel's path clicked: back to it
fn crumb_go(shared: &Shared, i: usize) {
    let Some(to) = PF_CRUMBS.with(|c| c.borrow().get(i).cloned()) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if let Some(n) = to.strip_prefix("panel:") { panel_open(shared, n); return; }
    // (the Suite's Widgets or Docks: its page, then that one)
    if to == "botwidgets" || to == "botdocks" || to == "botcreators" { PF_EDIT.with(|e| e.borrow_mut().take()); pf_open(shared, botpage::BOT); return bot_widgets_open(shared, &to[3..]); }
    if let Some(id) = to.strip_prefix("plugin:") {
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(sec) = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id).map(|p| p.manifest.section).filter(|x| !x.is_empty()) { ui.set_set_cat(sec.into()); }
        return pf_open(shared, id);
    }
    PF_EDIT.with(|e| e.borrow_mut().take());
    ui.set_pf_open(false);
    ui.set_set_cat(to.strip_prefix("cat:").unwrap_or("").into());
}

/// Settings closed (its X), from wherever in it: back to the page before it
fn settings_close(shared: &Shared) {
    PF_EDIT.with(|e| e.borrow_mut().take());
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_pf_open(false);
        ui.set_set_cat("".into());
        ui.set_set_query("".into());
        ui.set_search_text("".into());
    }
    if shared.borrow().history.is_empty() { open_page(shared, "discover"); } else { go_back(shared); }
}

/// A setting Settings' search can find
#[derive(Clone)]
struct SetFound { path: String, label: String, note: String, cat: String, panel: String, key: String, toggle: bool }

/// The settings drawn by hand (not from SET_ROWS): their category, name and description
const SET_HAND: [(&str, &str, &str); 27] = [
    ("About", "Updates", "Checks for a newer version a little after it starts, and installs it."),
    ("About", "Bugs and Ideas", "Report a bug or suggest a feature, to the developer."),
    ("About", "Run Setup Again", "The steps from when OmniPulse first opened."),
    ("About", "Support OmniPulse", "Say thanks with a coffee on Ko-fi."),
    ("App & Look", "Theme", "Light to dark."),
    ("App & Look", "Tint", "A colour through the theme."),
    ("App & Look", "Tint Strength", "How much of the tint."),
    ("App & Look", "Accent", "The colour of buttons, the progress bar and what's playing."),
    ("App & Look", "Colours from Album Art", "Both colours come from the cover that's playing."),
    ("App & Look", "Player Style", "Tab, Classic Bar, Floating or Minimal."),
    ("App & Look", "Title Bar Colour", "The title bar like the player bar, subtle, or plain."),
    ("App & Look", "Track Info Tab", "Which end of the player bar the tab sits at."),
    ("App & Look", "List Density", "How tightly tracks are listed. Compact fits more on screen."),
    ("App & Look", "Visualiser", "The music as it plays: Bars, Mirror, Peaks, Blocks, Wave, Centre or a Glow, on Now Playing, in the player, or full screen."),
    ("App & Look", "Display Name", "Shown in the title bar and taskbar."),
    ("App & Look", "Minimize to Tray", "Minimizing hides the window in the tray, and the music keeps playing. Use the tray's menu to open it again or quit."),
    ("App & Look", "Taskbar Widget", "What's playing, always on show in the taskbar by the clock, with its controls."),
    ("App & Look", "Hardware Acceleration", "Draws the app with your graphics card. Takes effect the next time OmniPulse starts."),
    ("App & Look", "Animations", "Fades and motion."),
    ("App & Look", "Rounded Corners", "Rounds the corners of the sidebar, the page and the queue. Turn it off for square corners."),
    ("About", "Back Up & Restore", "Your settings, keys, favourites, playlists, stats and alerts, in one .zip."),
    ("Integrations", "Accounts", "SoundCloud, Spotify, YouTube Music and Last.fm: signing in and out, and each one's switch."),
    ("Library", "Music Folders", "Folders on this PC or a network drive. Their music shows in your Library."),
    ("Sound", "Match Loudness", "Every track played at the same loudness."),
    ("Sound", "Fade Between Tracks", "Each track fades out where its sound ends, and the next fades in."),
    ("Sound", "Skip Silences", "Jumps over long silences, and moves on when the rest is silent."),
    ("Sound", "Equalizer", "Ten bands."),
];

/// Every setting there is, for searching: the hand-drawn ones, the switches, each panel's fields, the plugins
fn settings_index(a: &App) -> Vec<SetFound> {
    let mut out = Vec::new();
    let path = |cat: &str| if cat.is_empty() { "Settings".to_string() } else { format!("Settings \u{203a} {cat}") };
    for (cat, label, note) in SET_HAND {
        out.push(SetFound { path: path(cat), label: label.into(), note: note.into(), cat: cat.into(), panel: String::new(), key: String::new(), toggle: false });
    }
    for (cat, label, note, key, _) in SET_ROWS.iter().copied() {
        let cat = if matches!(key, "scOn" | "spOn" | "ytOn" | "scrobble") { "Integrations" } else { cat };
        // (Alerts: found in the Stream Widget's page)
        if key == "alertsOn" {
            out.push(SetFound { path: format!("{} \u{203a} {}", path(cat), panels::title("widget")), label: label.into(), note: note.into(), cat: cat.into(), panel: "widget".into(), key: key.into(), toggle: true });
            continue;
        }
        out.push(SetFound { path: path(cat), label: label.into(), note: note.into(), cat: cat.into(), panel: String::new(), key: key.into(), toggle: true });
    }
    let ip = omnipulse_core::lan_ip().unwrap_or_else(|| "THIS-PC".into());
    for (name, cat) in [("requests", "Streaming"), ("replies", "Streaming"), ("smart", "Sound"), ("lyricsw", "Streaming"), ("discord", "Integrations"), ("together", "Integrations"), ("lyrics", "Integrations"), ("widget", "Streaming"), ("alerts", "Streaming"), ("alertlist", "Streaming"), ("remote", "Streaming"), ("offset", "Streaming"), ("safe", "Streaming"), ("recent", "Library"), ("hidden", "Library"), ("plugins", "Plugins")] {
        let title = panels::title(name);
        out.push(SetFound { path: path(cat), label: title.clone(), note: String::new(), cat: cat.into(), panel: name.into(), key: String::new(), toggle: false });
        let vals = panels::values(name, &a.web.prefs);
        for f in panels::fields(name, &vals, &a.web.prefs, &ip) {
            let label = f["label"].as_str().unwrap_or("").trim().to_string();
            if label.is_empty() || matches!(f["type"].as_str(), Some("note" | "row")) { continue; }
            out.push(SetFound { path: format!("{} \u{203a} {title}", path(cat)), label, note: f["note"].as_str().unwrap_or("").into(), cat: cat.into(), panel: name.into(), key: String::new(), toggle: false });
        }
    }
    for pl in omnipulse_core::plugins::list() {
        out.push(SetFound { path: path("Plugins"), label: pl.manifest.name.clone(), note: pl.manifest.description.clone(), cat: "Plugins".into(), panel: String::new(), key: format!("plugOn_{}", pl.manifest.id), toggle: true });
    }
    out
}

/// Settings' search, as it's typed: every word in a setting's name, description or path; its name's first
fn settings_search(shared: &Shared, q: &str) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let q = q.trim().to_lowercase();
    ui.set_set_query(q.clone().into());
    if q.is_empty() { SET_HITS.with(|h| h.borrow_mut().clear()); return; }
    let words: Vec<&str> = q.split_whitespace().collect();
    let a = shared.borrow();
    let mut hits: Vec<(u8, SetFound)> = settings_index(&a).into_iter().filter_map(|f| {
        let (l, all) = (f.label.to_lowercase(), format!("{} {} {}", f.label, f.note, f.path).to_lowercase());
        if !words.iter().all(|w| all.contains(w)) { return None; }
        Some((if l.starts_with(&q) { 0 } else if words.iter().all(|w| l.contains(w)) { 1 } else { 2 }, f))
    }).collect();
    hits.sort_by_key(|h| h.0);
    let found: Vec<SetFound> = hits.into_iter().map(|h| h.1).take(60).collect();
    let rows: Vec<SetHit> = found.iter().map(|f| {
        let on = if f.key == "autoplay" { a.autoplay } else if let Some(id) = f.key.strip_prefix("plugOn_") { plugin_on(&a.web.prefs, id) } else { SET_ROWS.iter().find(|r| r.3 == f.key).map(|r| set_row_on(&a.web.prefs, r.3, r.4)).unwrap_or(false) };
        SetHit { path: f.path.clone().into(), label: f.label.clone().into(), note: f.note.clone().into(), cat: f.cat.clone().into(), panel: f.panel.clone().into(), key: f.key.clone().into(), on, toggle: f.toggle }
    }).collect();
    ui.set_set_hits(ModelRc::from(Rc::new(VecModel::from(rows))));
    SET_HITS.with(|h| *h.borrow_mut() = found);
}

/// A setting found, clicked: its category (and its panel), the search over
fn set_hit(shared: &Shared, i: usize) {
    let Some(f) = SET_HITS.with(|h| h.borrow().get(i).cloned()) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    ui.set_set_query("".into());
    ui.set_search_text("".into());
    ui.set_set_cat(f.cat.clone().into());
    if !f.panel.is_empty() { panel_open(shared, &f.panel); }
    // (what was found: lit where it shows (its page scrolls to it), for a few seconds)
    let label = f.label.clone();
    ui.global::<Hi>().set_label(label.clone().into());
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(4500), move || {
        if let Some(ui) = weak.upgrade() { if ui.global::<Hi>().get_label() == label.as_str() { ui.global::<Hi>().set_label("".into()); } }
    });
}

fn panel_open(shared: &Shared, name: &str) {
    panel_keys(shared);
    if name == "plugins" { panels::PLUG_PENDING.with(|p| p.borrow_mut().take()); }
    let vals = panels::values(name, &shared.borrow().web.prefs);
    PF_ORIG.with(|o| *o.borrow_mut() = vals.clone());
    PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:{name}"), vals)));
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_bot_page(false); ui.set_leave_ask(false); }
    pf_paint(shared);
    pf_mark(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        let head = match name.strip_prefix("alert:") { Some(k) => alerts::name(&shared.borrow().web.prefs, k), None => panels::output_of(name).map(|(_, n)| format!("Output {n}")).unwrap_or_else(|| panels::title(name)) };
        let output = panels::output_of(name).map(|(k, _)| k.to_string());
        let name = output.as_deref().unwrap_or(name);
        let cat = match name {
            "requests" | "replies" | "widget" | "remote" | "offset" | "alerts" | "alertlist" | "safe" | "lyricsw" | "socials" | "dockchat" | "dockfeed" | "multichat" => "Streaming",
            n if n.starts_with("social:") => "Streaming",
            "youtube" => "Integrations",
            n if n.starts_with("alert:") => "Streaming",
            "plugins" => "Plugins",
            "smart" => "Sound",
            "discord" | "together" | "lyrics" => "Integrations",
            _ => "Library",
        };
        let mut path = vec![("Settings".to_string(), "root".to_string()), (cat.to_string(), format!("cat:{cat}"))];
        let widgety = matches!(name, "widget" | "lyricsw" | "alerts" | "alertlist" | "requests" | "replies" | "socials" | "dockchat" | "dockfeed" | "multichat") || name.starts_with("alert:") || name.starts_with("social:");
        if widgety && plugin_on(&shared.borrow().web.prefs, botpage::BOT) {
            if let Some(m) = bot_manifest() { path.push((place_label(&m), format!("plugin:{}", botpage::BOT))); }
            let which = BOT_WIDGETS.with(|w| w.borrow().clone());
            if !which.is_empty() && name != "requests" && name != "replies" { path.push((widgets_title(&which).into(), format!("bot{which}"))); }
        }
        match name {
            "alerts" => path.push((panels::title("widget"), "panel:widget".into())),
            "alertlist" => { path.push((panels::title("widget"), "panel:widget".into())); path.push((panels::title("alerts"), "panel:alerts".into())); }
            "replies" => path.push((panels::title("requests"), "panel:requests".into())),
            n if n.starts_with("social:") => path.push((panels::title("socials"), "panel:socials".into())),
            n if n.starts_with("alert:") => {
                path.push((panels::title("widget"), "panel:widget".into()));
                path.push((panels::title("alerts"), "panel:alerts".into()));
                path.push((panels::title("alertlist"), "panel:alertlist".into()));
            }
            _ => {}
        }
        // (an output's page: its widget's page above it)
        if output.is_some() { path.push((panels::title(name), format!("panel:{name}"))); }
        crumbs_set(&ui, path);
        ui.set_pf_head(head.into());
        ui.set_pf_open(true);
    }
}

/// The Lyrics Widget's look as it's set, for its page's preview
/// The Multichat Overlay's preview: its look as its page has it now, with made-up chat (Twitch's and Kick's,
/// as Chat From says)
fn chat_preview(ui: &AppWindow, vals: &serde_json::Map<String, serde_json::Value>) {
    let num = |k: &str, d: f64| vals.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).unwrap_or(d);
    let on = |k: &str| vals.get(k).and_then(|v| v.as_bool()) != Some(false);
    let from = vals.get("from").and_then(|v| v.as_str()).unwrap_or("all").to_string();
    ui.set_pf_chat(ChatLook {
        style: vals.get("style").and_then(|v| v.as_str()).unwrap_or("plain").into(),
        size: num("size", 20.0) as f32, bg: num("bg", 0.0) as f32, shadow: on("shadow"),
        top: vals.get("dir").and_then(|v| v.as_str()) == Some("top"), badges: on("badges"), marks: on("marks"), from: from.clone().into(),
        above: vals.get("name").and_then(|v| v.as_str()) == Some("above"),
        right: vals.get("align").and_then(|v| v.as_str()) == Some("right"),
    });
    let c = |h: &str| look::Rgb::parse(h).map(|x| x.slint()).unwrap_or(slint::Color::from_rgb_u8(255, 255, 255));
    let all = [
        ("PixelPanda", "that transition was so clean", false, "#ff7a59", "SUB"), ("kickfan_92", "hello from Kick!", true, "#53fc18", ""),
        ("ModMaya", "remember to hydrate chat", false, "#3ccd6c", "MOD"), ("lofi_larry", "what's this song?", false, "#7aa2ff", ""),
        ("Zed", "GG that was close", true, "#ffd24a", "VIP"), ("Nova", "first time here, love the vibes", false, "#e05ce0", ""),
        ("RaidLeader", "we came from the raid!", false, "#ff4f8b", "SUB"), ("quietcat", "lurking but enjoying it", true, "#5ad1ff", ""),
    ];
    let list: Vec<ChatSample> = all.iter().filter(|m| from == "all" || (from == "kick") == m.2)
        .map(|m| ChatSample { name: m.0.into(), text: m.1.into(), kick: m.2, colour: c(m.3), badge: m.4.into() }).collect();
    ui.set_pf_chat_samples(ModelRc::from(Rc::new(VecModel::from(list))));
}

/// The Social Plug's preview, as its page has it now (before it's saved): its platforms switched on with a
/// handle, or samples while there are none
fn social_preview(ui: &AppWindow, prefs: &serde_json::Value, vals: &serde_json::Map<String, serde_json::Value>) {
    let mut p = prefs.clone();
    if let Some(o) = p.as_object_mut() {
        for (k, v) in vals { if !k.starts_with("on:") { o.insert(k.clone(), v.clone()); } }
        o.insert("socials".into(), socials::list_save(prefs, vals));
    }
    let w = socials::widget_json(&p);
    let colour = |s: &str| look::Rgb::parse(s).map(|c| c.slint()).unwrap_or(slint::Color::from_rgb_u8(0x8e, 0x9b, 0xff));
    let mut list: Vec<serde_json::Value> = w["list"].as_array().cloned().unwrap_or_default();
    let sample = list.is_empty();
    if sample {
        list = ["twitch", "youtube", "tiktok"].iter().map(|id| { let pl = socials::platform(id); serde_json::json!({ "platform": id, "name": pl.1, "handle": "@yourname", "col1": pl.2, "col2": pl.3 }) }).collect();
    }
    let items: Vec<SocialItem> = list.iter().map(|x| {
        let name = x["name"].as_str().unwrap_or("").to_string();
        SocialItem {
            name: name.clone().into(),
            handle: x["handle"].as_str().unwrap_or("").into(),
            c1: colour(x["col1"].as_str().unwrap_or("")),
            c2: colour(x["col2"].as_str().unwrap_or("")),
            d: socials::icon_path(x["platform"].as_str().unwrap_or("")).into(),
            letter: name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_default().into(),
        }
    }).collect();
    ui.set_pf_social_items(ModelRc::from(Rc::new(VecModel::from(items))));
    ui.set_pf_social(SocialLook {
        style: w["style"].as_str().unwrap_or("card").into(),
        coloured: w["coloured"].as_bool() == Some(true),
        align: w["align"].as_str().unwrap_or("left").into(),
        rounded: w["rounded"].as_bool() != Some(false),
        all: w["all"].as_bool() == Some(true),
        secs: w["secs"].as_u64().unwrap_or(8) as i32,
        motion: w["motion"].as_str().unwrap_or("rise").into(),
        sample,
    });
}

fn lyrics_look(v: &serde_json::Map<String, serde_json::Value>) -> LyricsLook {
    let col = |k: &str, d: &str| {
        let h = v.get(k).and_then(|x| x.as_str()).unwrap_or(d).trim_start_matches('#').to_string();
        let n = u32::from_str_radix(if h.len() == 6 { &h } else { d.trim_start_matches('#') }, 16).unwrap_or(0xffffff);
        slint::Color::from_rgb_u8((n >> 16) as u8, (n >> 8) as u8, n as u8)
    };
    let on = |k: &str, d: bool| v.get(k).and_then(|x| x.as_bool()).unwrap_or(d);
    let num = |k: &str, d: f64| v.get(k).and_then(|x| x.as_f64()).unwrap_or(d);
    let st = |k: &str, d: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or(d).to_string();
    LyricsLook {
        color: col("lyricsColor", "#ffffff"),
        hi: col("lyricsHi", "#8e9bff"),
        own_hi: on("lyricsOwnHi", false),
        dim: num("lyricsDim", 0.45) as f32,
        size: num("lyricsSize", 48.0) as f32,
        align: st("lyricsAlign", "center").into(),
        style: st("lyricsStyle", "karaoke").into(),
        shadow: on("lyricsShadow", true),
        outline: on("lyricsOutline", false),
        outline_color: col("lyricsOutlineColor", "#000000"),
        boxed: on("lyricsBox", false),
        box_color: col("lyricsBoxColor", "#0a0a0e"),
        box_opacity: num("lyricsBoxOpacity", 0.62) as f32,
        title: on("lyricsTitle", true),
        before: on("lyricsBefore", false),
        lines: num("lyricsLines", 3.0) as i32,
        upper: st("lyricsTransform", "none") == "uppercase",
        place: st("lyricsPlace", "").into(),
    }
}

fn panel_paint(shared: &Shared, name: &str, vals: &serde_json::Map<String, serde_json::Value>) {
    let fields = panels::fields(name, vals, &shared.borrow().web.prefs, &omnipulse_core::lan_ip().unwrap_or_else(|| "THIS-PC".into()));
    let text = |v: &serde_json::Value| match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Null => String::new(), x => x.to_string() };
    let rows: Vec<PlugField> = fields.iter().map(|f| {
        let key = f["key"].as_str().unwrap_or("").to_string();
        let kind = f["type"].as_str().unwrap_or("text").to_string();
        // (a switch with its own settings, as Settings' switches: switched at once, so it's as the prefs have
        // it, not the page's copy of its values)
        let now = if kind == "togglecfg" { serde_json::json!(set_row_on(&shared.borrow().web.prefs, &key, false)) } else { vals.get(&key).cloned().unwrap_or_default() };
        let labels = || -> Vec<slint::SharedString> { f["labels"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(slint::SharedString::from)).collect() };
        let options: Vec<slint::SharedString> = match kind.as_str() {
            "row" | "select" | "designpick" | "stepper" | "wave" => labels(),
            "range" | "rangenum" => vec![f["min"].to_string().into(), f["max"].to_string().into(), f["step"].to_string().into(), f["unit"].as_str().unwrap_or("").into()],
            "togglecfg" => vec!["Configure".into()],
            _ => Vec::new(),
        };
        // (where it is among its options; a colour: its value as 0xRRGGBB, -1 if it isn't one)
        let at: i32 = match kind.as_str() {
            "select" | "designpick" | "stepper" => f["options"].as_array().and_then(|o| o.iter().position(|o| panels::same(o, &now))).unwrap_or(0) as i32,
            "color" => now.as_str().and_then(|c| c.strip_prefix('#')).filter(|c| c.len() == 6).and_then(|c| i32::from_str_radix(c, 16).ok()).unwrap_or(-1),
            _ => 0,
        };
        let value = match kind.as_str() {
            "row" | "togglecfg" => if now.as_bool() == Some(true) { "true".into() } else { "false".into() },
            "select" | "designpick" | "stepper" => f["labels"][at.max(0) as usize].as_str().unwrap_or("").to_string(),
            "range" | "rangenum" => now.as_f64().unwrap_or(f["min"].as_f64().unwrap_or(0.0)).to_string(),
            "toggle" => if now.as_bool() == Some(true) { "true".into() } else { "false".into() },
            "info" | "output" | "button" => f["value"].as_str().unwrap_or("").to_string(),
            _ => text(&now),
        };
        PlugField { changed: kind != "togglecfg" && pf_changed(&key), label: f["label"].as_str().unwrap_or("").into(), note: f["note"].as_str().unwrap_or("").into(), value: value.into(), kind: kind.into(), options: ModelRc::from(Rc::new(VecModel::from(options))), key: key.into(), at }
    }).collect();
    PANEL_FIELDS.with(|p| *p.borrow_mut() = fields);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_pf_fields(ModelRc::from(Rc::new(VecModel::from(rows))));
        // (the Lyrics Widget's page: a preview of its look, as it's set)
        ui.set_pf_preview(if name == "lyricsw" { "lyrics".into() } else if name == "socials" { "socials".into() } else if name == "multichat" { "multichat".into() } else if name == "alerts" { "alerts".into() } else if name == "widget" { "widget".into() } else { "".into() });
        if name == "lyricsw" { ui.set_pf_lyrics(lyrics_look(vals)); }
        if name == "socials" { social_preview(&ui, &shared.borrow().web.prefs, vals); }
        if name == "multichat" { chat_preview(&ui, vals); }
        if name == "widget" {
            let t = ui.global::<Theme>();
            let own = vals.get("widgetOwnCol").and_then(|v| v.as_bool()) == Some(true);
            let col = |k: &str, d: slint::Color| if own { vals.get(k).and_then(|v| v.as_str()).and_then(look::Rgb::parse).map(|c| c.slint()).unwrap_or(d) } else { d };
            let on = |k: &str, d: bool| vals.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
            ui.set_pf_widget(WidgetLook {
                design: vals.get("widgetDesign").and_then(|v| v.as_str()).unwrap_or("card").into(),
                right: vals.get("widgetAlign").and_then(|v| v.as_str()) == Some("right"),
                rounded: on("widgetRounded", true),
                text: vals.get("widgetText").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                c1: col("widgetCol1", t.get_accent()), c2: col("widgetCol2", t.get_accent_2()),
                spin: on("wSpin", true), requester: on("wRequester", true),
            });
        }
        if name == "alerts" {
            let t = ui.global::<Theme>();
            let own = vals.get("alertOwnCol").and_then(|v| v.as_bool()) == Some(true);
            let col = |k: &str, d: slint::Color| if own { vals.get(k).and_then(|v| v.as_str()).and_then(look::Rgb::parse).map(|c| c.slint()).unwrap_or(d) } else { d };
            ui.set_pf_alert(AlertLook {
                theme: vals.get("alertTheme").and_then(|v| v.as_str()).unwrap_or("widget").into(),
                c1: col("alertCol1", t.get_accent()), c2: col("alertCol2", t.get_accent_2()),
                glow: vals.get("alertDir").and_then(|v| v.as_str()) == Some("glow"),
                icon: vals.get("alertIcon").and_then(|v| v.as_f64()).unwrap_or(1.0) as f32,
                transition: vals.get("alertTrans").and_then(|v| v.as_str()).unwrap_or("seamless").into(),
                anim: vals.get("alertAnim").and_then(|v| v.as_str()).unwrap_or("slide").into(),
                in_widget: vals.get("alertInWidget").and_then(|v| v.as_bool()) == Some(true),
            });
        }
    }
}

/// A field changed (a choice: on to its next; a button: pressed, at once)
fn panel_set(shared: &Shared, name: &str, mut vals: serde_json::Map<String, serde_json::Value>, i: usize, v: &str) {
    let Some(f) = PANEL_FIELDS.with(|p| p.borrow().get(i).cloned()) else { return };
    let key = f["key"].as_str().unwrap_or("").to_string();
    let changed = key.clone();
    let redraw = match f["type"].as_str().unwrap_or("") {
        "toggle" => { vals.insert(key, serde_json::json!(v == "true")); true }
        "select" | "designpick" | "stepper" => {
            let opts = f["options"].as_array().cloned().unwrap_or_default();
            let now = vals.get(&key).cloned().unwrap_or_default();
            // (picked from its list, or − / +: by its place; else on to the next)
            let at = match v.strip_prefix('\u{2}').and_then(|k| k.parse::<i64>().ok()) {
                Some(k) => k.clamp(0, opts.len() as i64 - 1).max(0) as usize,
                None => opts.iter().position(|o| panels::same(o, &now)).map_or(0, |k| (k + 1) % opts.len().max(1)),
            };
            if let Some(o) = opts.get(at) { vals.insert(key, o.clone()); }
            true
        }
        "range" | "rangenum" => {
            let (min, max, step) = (f["min"].as_f64().unwrap_or(0.0), f["max"].as_f64().unwrap_or(1.0), f["step"].as_f64().unwrap_or(0.01).max(1e-6));
            let typed = v.starts_with('\u{5}');
            let Ok(x) = v.trim_start_matches(['\u{4}', '\u{5}']).trim().parse::<f64>() else { return };
            let x = ((x.clamp(min, max) - min) / step).round() * step + min;
            vals.insert(key, serde_json::json!((x * 1000.0).round() / 1000.0));
            // (slid: not drawn again under the pointer; typed in: drawn again, the slider where it is)
            typed
        }
        "color" => {
            if v == "\u{3}pick" {
                let now = vals.get(&key).and_then(|c| c.as_str()).unwrap_or("#8e9bff").to_string();
                // (picked on its own thread, then set as if typed in)
                desktop::pick_color_then(&now, move |c| SHARED.with(|s| if let Some(sh) = s.borrow().clone() { pf_set(&sh, i, &c); }));
                return;
            } else {
                let c = v.trim().trim_start_matches('#');
                if c.len() != 6 || u32::from_str_radix(c, 16).is_err() { return; }
                vals.insert(key, serde_json::json!(format!("#{}", c.to_lowercase())));
            }
            true
        }
        // a switch with its own settings: switched at once (as the others in Settings are), Configure opens them
        "togglecfg" => {
            PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:{name}"), vals.clone())));
            if v == "btn:0" { if let Some(p) = f["press"].as_str() { panel_press(shared, p); } return; }
            let on = v == "true";
            if set_row_on(&shared.borrow().web.prefs, &key, false) != on { set_toggle(shared, &key); }
            return pf_paint(shared);
        }
        "button" => { PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:{name}"), vals.clone()))); panel_press(shared, &key); return pf_paint(shared); }
        // (another output's card: its look, or taken away)
        "output" => {
            PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:{name}"), vals.clone())));
            let rest = key.strip_prefix("out:").unwrap_or("");
            return panel_press(shared, &format!("{}:{rest}", if v == "btn:del" { "outDel" } else { "outEdit" }));
        }
        // a row of a list (the alerts'): its switch, or one of its buttons
        "row" => {
            if let Some(n) = v.strip_prefix("btn:").and_then(|n| n.parse::<usize>().ok()) {
                PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:{name}"), vals.clone())));
                if let Some(act) = f["actions"][n].as_str() { panel_press(shared, act); }
                return pf_paint(shared);
            }
            vals.insert(key, serde_json::json!(v == "true"));
            true
        }
        _ => { if let Some(x) = panels::typed(&key, v) { vals.insert(key, x); } false }
    };
    // (the Lyrics Widget's theme: its whole look at once; a part of it changed after, Custom)
    let mut redraw = redraw;
    if name == "lyricsw" || panels::output_of(name).is_some_and(|(k, _)| k == "lyricsw") {
        if changed == "lyricsTheme" {
            for (k, x) in panels::lyrics_theme(vals.get("lyricsTheme").and_then(|t| t.as_str()).unwrap_or("custom")) { vals.insert(k.to_string(), x); }
            redraw = true;
        } else if panels::LYRICS_THEME_KEYS.contains(&changed.as_str()) && vals.get("lyricsTheme").and_then(|t| t.as_str()).unwrap_or("custom") != "custom" {
            vals.insert("lyricsTheme".into(), serde_json::json!("custom"));
            redraw = true;
        }
    }
    PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:{name}"), vals.clone())));
    if redraw { panel_paint(shared, name, &vals); }
}

/// The alerts' list, as it is (its switches), kept before going elsewhere from it
fn alerts_list_keep(shared: &Shared) {
    let Some((id, vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    if id != "panel:alertlist" { return; }
    let changes = alerts::list_save(&shared.borrow().web.prefs, &vals);
    for (k, v) in changes { pref_put(shared, k, v); }
}

/// One of the open panel's values, set from here (a file picked), and the panel drawn again
fn panel_put(shared: &Shared, key: &str, v: serde_json::Value) {
    let Some((id, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    vals.insert(key.to_string(), v);
    PF_EDIT.with(|e| *e.borrow_mut() = Some((id, vals)));
    pf_paint(shared);
}

/// A panel's button: done at once
/// A widget's other outputs, as they are now (what's listed), kept
fn outputs_put(shared: &Shared, kind: &str, outs: Vec<serde_json::Map<String, serde_json::Value>>) {
    let key = if kind == "lyricsw" { "lyricsOutputs" } else { "widgetOutputs" };
    pref_put(shared, key, serde_json::Value::Array(outs.into_iter().map(serde_json::Value::Object).collect()));
    push_out(shared, true);
}

/// The app theme's own two accents (App & Look), whatever the album art's doing: "Your Theme's" colours
fn theme_accents(prefs: &serde_json::Value) -> (String, String) {
    let (a, b) = theme_rgb(prefs);
    (a.hex(), b.hex())
}

/// This install's name: random, made once, so a tester's ratings or reports can be told apart (not who they are)
fn install_id(dir: &std::path::Path) -> String {
    let id_file = dir.join("install-id");
    match std::fs::read_to_string(&id_file) {
        Ok(s) if s.trim().len() >= 8 => s.trim().to_string(),
        _ => {
            let id: String = (0..16u64).map(|k| { use std::hash::{BuildHasher, Hasher}; let mut h = std::collections::hash_map::RandomState::new().build_hasher(); h.write_u64(k); char::from_digit((h.finish() % 36) as u32, 36).unwrap_or('0') }).collect();
            let _ = std::fs::write(&id_file, &id);
            id
        }
    }
}

/// A bug report or an idea, from its page: to the relay, for the developer; the page emptied once it's there
fn report_send(shared: &Shared) {
    let vals = PF_EDIT.with(|e| e.borrow().as_ref().map(|(_, v)| v.clone())).unwrap_or_default();
    let get = |k: &str| vals.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let say = |t: &str| if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(t.into()); };
    let (title, text) = (get("reportTitle"), get("reportText"));
    if text.len() < 10 { return say("Say a little more about it first, in the big box."); }
    let kind = if get("reportKind") == "feature" { "feature" } else { "bug" };
    let with_info = vals.get("reportInfo").and_then(|v| v.as_bool()) != Some(false);
    let (dir, prefs, rt) = { let a = shared.borrow(); (omnipulse_core::data_dir().ok().map(|d| d.join("state")), a.web.prefs.clone(), a.rt.clone()) };
    let Some(dir) = dir else { return say("Couldn't find OmniPulse's folder to send it from.") };
    let os = if with_info { windows_version() } else { String::new() };
    let extra = if with_info {
        let plugs: Vec<String> = prefs.as_object().map(|o| o.iter().filter(|(k, v)| k.starts_with("plugOn_") && v.as_bool() == Some(true)).map(|(k, _)| k[7..].to_string()).collect()).unwrap_or_default();
        format!("player: {}; theme: {}; plugins: {}", prefs["playerStyle"].as_str().unwrap_or("tab"), prefs["theme"].as_str().unwrap_or("default"), if plugs.is_empty() { "none".into() } else { plugs.join(", ") })
    } else { String::new() };
    let body = serde_json::json!({ "install": install_id(&dir), "kind": kind, "title": title, "text": text, "contact": get("reportContact"), "version": if with_info { env!("CARGO_PKG_VERSION") } else { "" }, "os": os, "extra": extra });
    say("Sending...");
    let weak = shared.borrow().ui.clone();
    rt.spawn(async move {
        let r = reqwest::Client::new().post(format!("{}/api/reports", share::RELAY_URL)).json(&body).timeout(std::time::Duration::from_secs(20)).send().await;
        let msg = match r {
            Ok(r) if r.status().is_success() => Ok(()),
            Ok(r) => Err(r.json::<serde_json::Value>().await.ok().and_then(|v| v["error"].as_str().map(String::from)).unwrap_or_else(|| "the relay didn't take it".into())),
            Err(_) => Err("couldn't reach OmniPulse's relay (are you online?)".into()),
        };
        let _ = slint::invoke_from_event_loop(move || {
            let Some(ui) = weak.upgrade() else { return };
            match msg {
                Ok(()) => {
                    ui.set_status(if kind == "bug" { "Sent. Thank you: that helps fix it." } else { "Sent. Thank you for the idea!" }.into());
                    // (emptied, ready for another)
                    PF_EDIT.with(|e| if let Some((_, v)) = e.borrow_mut().as_mut() { for k in ["reportTitle", "reportText"] { v.insert(k.into(), serde_json::json!("")); } });
                    SHARED.with(|x| if let Some(sh) = x.borrow().clone() { pf_paint(&sh); });
                }
                Err(e) => ui.set_status(format!("Not sent: {e}").into()),
            }
        });
    });
}

/// Windows' version, as "Windows 11 (26200)"
fn windows_version() -> String {
    let build = std::process::Command::new("cmd").args(["/c", "ver"]).output().ok().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
    let n = build.split('.').nth(2).and_then(|b| b.trim_end_matches(|c: char| !c.is_ascii_digit()).parse::<u32>().ok()).unwrap_or(0);
    if n == 0 { "Windows".into() } else { format!("Windows {} ({n})", if n >= 22000 { 11 } else { 10 }) }
}

fn panel_press(shared: &Shared, key: &str) {
    if key == "reportSend" { return report_send(shared); }
    if key == "alSlImport" { return sl_import_ask(shared); }
    if key == "ytHelpersSetup" { return helpers_act(shared, "setup"); }
    // (the Stream Deck plugin: given to Stream Deck to install, as it asks)
    if key == "sdInstall" {
        let note = match streamdeck::install() { Ok(()) => "Opened in Stream Deck. Confirm there to install it.".to_string(), Err(e) => e };
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
        return;
    }
    // (the Social Plug's platforms)
    if key == "socAdd" {
        let (id, all) = socials::added(&shared.borrow().web.prefs);
        pref_put(shared, "socials", all);
        push_out(shared, true);
        return pf_leave(shared, &format!("panel:social:{id}"));
    }
    if let Some(id) = key.strip_prefix("socEdit:") { return pf_leave(shared, &format!("panel:social:{id}")); }
    if let Some((id, by)) = key.strip_prefix("socUp:").map(|i| (i, -1)).or_else(|| key.strip_prefix("socDown:").map(|i| (i, 1))) {
        let all = socials::moved(&shared.borrow().web.prefs, id, by);
        pref_put(shared, "socials", all);
        push_out(shared, true);
        return pf_paint(shared);
    }
    if let Some(id) = key.strip_prefix("socIcon:") {
        let Some(path) = ask_path(rfd::FileDialog::new().set_title("Choose an icon for this platform").add_filter("Pictures", &omnipulse_core::widget::IMAGE_EXTS), Ask::Open, { let (s, k) = (shared.clone(), key.to_string()); move || panel_press(&s, &k) }) else { return };
        match omnipulse_core::widget::alert_media_set(&format!("social-{id}"), "image", &path.display().to_string()) {
            Ok(file) => {
                let all = socials::with(&shared.borrow().web.prefs, id, |m| { m.insert("icon".into(), serde_json::json!(file)); m.insert("iconAt".into(), serde_json::json!(now_ms())); });
                pref_put(shared, "socials", all);
                push_out(shared, true);
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Icon added.".into()); }
            }
            Err(e) => if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(e.into()); },
        }
        return pf_paint(shared);
    }
    if let Some(id) = key.strip_prefix("socIconClear:") {
        let all = socials::with(&shared.borrow().web.prefs, id, |m| { m.remove("icon"); });
        pref_put(shared, "socials", all);
        push_out(shared, true);
        return pf_paint(shared);
    }
    if let Some(id) = key.strip_prefix("socDel:") {
        let all = socials::without(&shared.borrow().web.prefs, id);
        pref_put(shared, "socials", all);
        push_out(shared, true);
        PF_EDIT.with(|e| e.borrow_mut().take());
        return panel_open(shared, "socials");
    }
    // (a widget's other outputs: another added (looking as the first does), one taken away, or its look changed)
    if let Some(kind) = key.strip_prefix("outAdd:") {
        let mut outs = integrations::outputs_of(&shared.borrow().web.prefs, kind);
        outs.push(serde_json::Map::new());
        let n = outs.len() + 1;
        outputs_put(shared, kind, outs);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Output {n} added. Copy its address into OBS.").into()); }
        return pf_paint(shared);
    }
    if let Some((kind, n)) = key.strip_prefix("outDel:").and_then(|r| r.split_once(':')).and_then(|(k, n)| n.parse::<usize>().ok().map(|n| (k.to_string(), n))) {
        let mut outs = integrations::outputs_of(&shared.borrow().web.prefs, &kind);
        if n >= 2 && n - 2 < outs.len() { outs.remove(n - 2); }
        outputs_put(shared, &kind, outs);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Output {n} removed. The ones after it moved up a number.").into()); }
        return pf_paint(shared);
    }
    if let Some(rest) = key.strip_prefix("outEdit:") {
        if let Some((kind, n)) = rest.split_once(':') { return pf_leave(shared, &format!("panel:{kind}@{n}")); }
    }
    if key == "ytHelpersRemove" { return helpers_act(shared, "remove"); }
    let note = match key {
        "reqClear" => {
            let mut a = shared.borrow_mut();
            let cur = a.queue.qi;
            let before = a.queue.items.len();
            let keep: Vec<bool> = a.queue.items.iter().enumerate().map(|(i, q)| q.by.is_empty() || Some(i) == cur).collect();
            let mut k = 0;
            a.queue.items.retain(|_| { let x = keep[k]; k += 1; x });
            if let Some(c) = cur { a.queue.qi = Some(keep[..c].iter().filter(|x| **x).count()); }
            let n = before - a.queue.items.len();
            paint_queue(&mut a);
            save_session(&a);
            if n == 0 { "No requests waiting.".to_string() } else { format!("Cleared {}.", plural(n, "request", "requests")) }
        }
        "reqNewKey" | "remoteNewKey" => {
            let pref = if key == "reqNewKey" { "reqKey" } else { "remoteKey" };
            pref_put(shared, pref, serde_json::json!(panels::new_key()));
            if key == "reqNewKey" { omnipulse_core::requests::requests_stop(); } else { omnipulse_core::remote::remote_stop(); }
            outside_start(shared);
            "Made a new key. The addresses now use it.".to_string()
        }
        // ---------- the alerts (from the Stream Widget's page) ----------
        k if k.starts_with("lyUp:") || k.starts_with("lyDown:") => {
            let (id, by) = match k.strip_prefix("lyUp:") { Some(id) => (id.to_string(), -1), None => (k[7..].to_string(), 1) };
            if let Some((n, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) {
                panels::lyrics_move(&mut vals, &id, by);
                PF_EDIT.with(|e| *e.borrow_mut() = Some((n, vals)));
            }
            return;
        }
        "reqLimitsReset" => {
            if let Some((n, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) {
                vals.insert("reqLimits".into(), serde_json::json!(true));
                vals.insert("reqMax".into(), serde_json::json!(10));
                vals.insert("reqPerUser".into(), serde_json::json!(2));
                PF_EDIT.with(|e| *e.borrow_mut() = Some((n, vals)));
            }
            "Back to on, 10 waiting, 2 per viewer (Save keeps it).".to_string()
        }
        // the widget being set up, in a window of its own
        "previewOpen" => {
            let name = PF_EDIT.with(|e| e.borrow().as_ref().and_then(|x| x.0.strip_prefix("panel:").map(String::from))).unwrap_or_default();
            widget_preview_open(shared, &name);
            return;
        }
        "alertsSwitch" => {
            set_toggle(shared, "alertsOn");
            return;
        }
        // (Chat Replies: from Song Requests, its page saved first)
        // (Song Requests, the chatbot taking them: its own page, asked first if something here isn't saved)
        "botOpen" => { pf_leave(shared, &format!("plugin:{}", botpage::BOT)); return; }
        "repliesOpen" => {
            if let Some((_, vals)) = PF_EDIT.with(|e| e.borrow().clone()) { for k in panels::keys_of("requests") { if let Some(v) = vals.get(k) { pref_put(shared, k, v.clone()); } } }
            panel_open(shared, "replies");
            return;
        }
        "repliesReset" => {
            if let Some((n, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) {
                for (k, _) in requests::MSG_KEYS { vals.insert(format!("msg:{k}"), serde_json::json!("")); }
                PF_EDIT.with(|e| *e.borrow_mut() = Some((n, vals)));
            }
            "Every reply back to its default (Save keeps it).".to_string()
        }
        "alertsOpen" => {
            // (the widget's page saved first, as alList does the alerts')
            if let Some((_, vals)) = PF_EDIT.with(|e| e.borrow().clone()) { for k in panels::keys_of("widget") { if let Some(v) = vals.get(k) { pref_put(shared, k, v.clone()); } } }
            panel_open(shared, "alerts");
            return;
        }
        "alList" => {
            // (the Alerts page saved first, then its list)
            if let Some((_, vals)) = PF_EDIT.with(|e| e.borrow().clone()) { for k in panels::keys_of("alerts") { if let Some(v) = vals.get(k) { pref_put(shared, k, v.clone()); } } }
            panel_open(shared, "alertlist");
            return;
        }
        k if k.starts_with("alEdit:") => {
            alerts_list_keep(shared);
            panel_open(shared, &format!("alert:{}", &k[7..]));
            return;
        }
        k if k.starts_with("alTest:") => {
            alerts_list_keep(shared);
            if !integrations::set(&shared.borrow().web.prefs, "widget") { "Switch on the widget first.".to_string() } else {
                push_out(shared, true);
                match omnipulse_core::widget::alert_push(&alerts::test_alert(&k[7..])) { Ok(_) => "Sent a test alert to the widget.".to_string(), Err(e) => e }
            }
        }
        k if k.starts_with("alUp:") || k.starts_with("alDown:") => {
            let (kind, by) = if let Some(x) = k.strip_prefix("alUp:") { (x, -1) } else { (&k[7..], 1) };
            let order = alerts::moved(&shared.borrow().web.prefs, kind, by);
            pref_put(shared, "alertOrder", order);
            push_out(shared, true);
            return;
        }
        "alNew" => {
            alerts_list_keep(shared);
            let (id, all) = alerts::added(&shared.borrow().web.prefs);
            pref_put(shared, "alertCustom", all);
            panel_open(shared, &format!("alert:{id}"));
            return;
        }
        k if k.starts_with("alPic:") || k.starts_with("alSnd:") => {
            let (kind, image) = if let Some(x) = k.strip_prefix("alPic:") { (x.to_string(), true) } else { (k[6..].to_string(), false) };
            let (title, exts): (&str, &[&str]) = if image { ("Choose a picture, GIF or video for this alert", &["gif", "png", "webp", "jpg", "jpeg", "webm", "mp4"]) } else { ("Choose a sound for this alert", &["mp3", "wav", "ogg", "m4a", "aac", "flac"]) };
            let Some(path) = ask_path(rfd::FileDialog::new().set_title(title).add_filter(if image { "Pictures" } else { "Sounds" }, exts), Ask::Open, { let (s, k) = (shared.clone(), key.to_string()); move || panel_press(&s, &k) }) else { return };
            match omnipulse_core::widget::alert_media_set(&kind, if image { "image" } else { "sound" }, &path.display().to_string()) {
                Ok(file) => {
                    let key = if image { "alertImages" } else { "alertSounds" };
                    let mut all = shared.borrow().web.prefs[key].as_object().cloned().unwrap_or_default();
                    all.insert(kind.clone(), serde_json::json!({ "name": file, "at": now_ms() }));
                    pref_put(shared, key, serde_json::Value::Object(all));
                    // (a new picture starts uncropped, as the web app's; on the page too)
                    if image {
                        let mut edits = shared.borrow().web.prefs["alertEdit"].as_object().cloned().unwrap_or_default();
                        if let Some(o) = edits.get_mut(&kind).and_then(|x| x.as_object_mut()) { o.remove("crop"); }
                        pref_put(shared, "alertEdit", serde_json::Value::Object(edits));
                        for (key, v) in [("crop.fit", serde_json::json!("contain")), ("crop.z", serde_json::json!(1.0)), ("crop.x", serde_json::json!(50.0)), ("crop.y", serde_json::json!(50.0))] { panel_put(shared, key, v); }
                    }
                    // (a new sound measured, so it plays about as loud as the others)
                    let note = if image { format!("{file} shows with this alert.") } else {
                        match omnipulse_core::loudness::alert_levels(&kind) {
                            Ok(m) => {
                                let mut lv = shared.borrow().web.prefs["alertLevels"].as_object().cloned().unwrap_or_default();
                                let mut m = m;
                                m["adj"] = serde_json::json!(0);
                                lv.insert(kind.clone(), m);
                                pref_put(shared, "alertLevels", serde_json::Value::Object(lv));
                                format!("{file} plays with this alert.")
                            }
                            Err(e) => format!("{file} plays with this alert, but couldn't be measured: {e}"),
                        }
                    };
                    push_out(shared, true);
                    note
                }
                Err(e) => e,
            }
        }
        // a collage picture (cladd:<n>:<kind>, clrm:<n>:<kind>), kept as <kind>-<n+1> as the web app keeps it
        k if k.starts_with("cladd:") || k.starts_with("clrm:") => {
            let add = k.starts_with("cladd:");
            let rest = k.split_once(':').map(|x| x.1).unwrap_or("");
            let (n, kind) = rest.split_once(':').unwrap_or(("0", ""));
            let Ok(n) = n.parse::<usize>() else { return };
            let file_kind = format!("{kind}-{}", n + 1);
            let mut all = shared.borrow().web.prefs["alertCollage"].as_object().cloned().unwrap_or_default();
            let mut list: Vec<serde_json::Value> = all.get(kind).and_then(|x| x.as_array()).cloned().unwrap_or_default();
            while list.len() <= n { list.push(serde_json::Value::Null); }
            let note = if add {
                let Some(path) = ask_path(rfd::FileDialog::new().set_title("Choose a picture or GIF for this alert's collage").add_filter("Pictures", &["gif", "png", "webp", "jpg", "jpeg"]), Ask::Open, { let (s, k) = (shared.clone(), key.to_string()); move || panel_press(&s, &k) }) else { return };
                match omnipulse_core::widget::alert_media_set(&file_kind, "collage", &path.display().to_string()) {
                    Ok(file) => { list[n] = serde_json::json!({ "name": file, "at": now_ms() }); format!("{file} is in its collage.") }
                    Err(e) => { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(e.into()); } return; }
                }
            } else {
                omnipulse_core::widget::alert_media_clear(&file_kind, "collage");
                list[n] = serde_json::Value::Null;
                "Taken out of its collage.".to_string()
            };
            while list.last().is_some_and(|x| x.is_null()) { list.pop(); }
            if list.iter().any(|x| !x.is_null()) { all.insert(kind.to_string(), serde_json::json!(list)); } else { all.remove(kind); }
            pref_put(shared, "alertCollage", serde_json::Value::Object(all));
            push_out(shared, true);
            note
        }
        k if k.starts_with("alPicClear:") || k.starts_with("alSndClear:") => {
            let image = k.starts_with("alPicClear:");
            let kind = &k[11..];
            omnipulse_core::widget::alert_media_clear(kind, if image { "image" } else { "sound" });
            for key in if image { vec!["alertImages"] } else { vec!["alertSounds", "alertLevels"] } {
                let mut all = shared.borrow().web.prefs[key].as_object().cloned().unwrap_or_default();
                all.remove(kind);
                pref_put(shared, key, serde_json::Value::Object(all));
            }
            push_out(shared, true);
            if image { "The icon is showing again.".to_string() } else { "The sound is removed.".to_string() }
        }
        k if k.starts_with("alDelete:") => {
            let kind = k[9..].to_string();
            let changes = alerts::deleted(&shared.borrow().web.prefs, &kind);
            for (key, v) in changes { pref_put(shared, key, v); }
            for slot in ["sound", "image"] { omnipulse_core::widget::alert_media_clear(&kind, slot); }
            for n in 1..=6 { omnipulse_core::widget::alert_media_clear(&format!("{kind}-{n}"), "collage"); }
            push_out(shared, true);
            panel_open(shared, "alertlist");
            return;
        }
        k if k.starts_with("alReset:") => {
            let mut all = shared.borrow().web.prefs["alertEdit"].as_object().cloned().unwrap_or_default();
            all.remove(&k[8..]);
            pref_put(shared, "alertEdit", serde_json::Value::Object(all));
            let vals = alerts::edit_values(&shared.borrow().web.prefs, &k[8..]);
            PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:alert:{}", &k[8..]), vals)));
            push_out(shared, true);
            "Back to how it was.".to_string()
        }
        "plugFile" => {
            let Some(path) = ask_path(rfd::FileDialog::new().set_title("A plugin's manifest").add_filter("Plugin manifest", &["json"]), Ask::Open, { let (s, k) = (shared.clone(), key.to_string()); move || panel_press(&s, &k) }) else { return };
            panel_put(shared, "plugSource", serde_json::json!(path.display().to_string()));
            return panel_press(shared, "plugCheck");
        }
        "plugCheck" => {
            let src = PF_EDIT.with(|e| e.borrow().as_ref().and_then(|(_, v)| v.get("plugSource").and_then(|x| x.as_str()).map(String::from))).unwrap_or_default();
            if src.trim().is_empty() { "Give it a plugin's address or file first.".to_string() } else {
                let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
                rt.spawn(async move {
                    let r = omnipulse_core::plugins::inspect(&src).await;
                    let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                        let Some(sh) = s.borrow().clone() else { return };
                        ui.set_status(match &r { Ok(p) => format!("Read {}: what it would be able to do is below.", p.manifest.name), Err(_) => "Couldn't read it.".to_string() }.into());
                        panels::PLUG_PENDING.with(|p| *p.borrow_mut() = Some(r));
                        pf_paint(&sh);
                    }));
                });
                "Reading it...".to_string()
            }
        }
        "plugInstall" => {
            let Some(Ok(pkg)) = panels::PLUG_PENDING.with(|p| p.borrow_mut().take()) else { return };
            let src = PF_EDIT.with(|e| e.borrow().as_ref().and_then(|(_, v)| v.get("plugSource").and_then(|x| x.as_str()).map(String::from))).unwrap_or_default();
            let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
            rt.spawn(async move {
                let r = omnipulse_core::plugins::install(&src, &pkg.digest).await;
                let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                    let Some(sh) = s.borrow().clone() else { return };
                    match r {
                        Ok(m) => {
                            plugrun::stop(&m.id);
                            plugins_start(&sh);
                            panel_put(&sh, "plugSource", serde_json::json!(""));
                            ui.set_status(format!("Added {}.", m.name).into());
                        }
                        Err(e) => ui.set_status(format!("Couldn't add it: {e}").into()),
                    }
                    pf_paint(&sh);
                    paint_settings(&sh.borrow());
                }));
            });
            "Adding it...".to_string()
        }
        k if k.starts_with("plugRemove:") => {
            let id = &k["plugRemove:".len()..];
            let name = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id).map(|p| p.manifest.name).unwrap_or_else(|| id.to_string());
            let id = id.to_string();
            let text = format!("Remove {name}? Its songs will be removed from your search and Library.");
            return ask(shared, "Remove plugin", &text, "Remove", move |sh| {
                plugrun::stop(&id);
                let r = omnipulse_core::plugins::remove(&id);
                paint_settings(&sh.borrow());
                if let Some(ui) = sh.borrow().ui.upgrade() { ui.set_status(match r { Ok(()) => format!("Removed {name}."), Err(e) => format!("Couldn't remove it: {e}") }.into()); }
            });
        }
        k if k.starts_with("un") && k.contains(':') => {
            let (list, key) = k[2..].split_once(':').unwrap_or(("", ""));
            let changes = hide::unhide(&shared.borrow().web.prefs, list, key);
            for (k, v) in changes { pref_put(shared, k, v); }
            hide_applied(shared);
            if list == "p" { let mut a = shared.borrow_mut(); paint_sides(&mut a); }
            match list { "v" | "p" => "It shows again.".to_string(), "all" => "They can all be recommended again.".to_string(), _ => "It can be recommended again.".to_string() }
        }
        k if k.starts_with("plugFiles:") => {
            match omnipulse_core::plugins::folder_of(&k["plugFiles:".len()..]) {
                Ok(dir) => { let _ = std::process::Command::new("explorer").arg(dir).spawn(); return; }
                Err(e) => format!("Couldn't open its folder: {e}"),
            }
        }
        // an alert's two colours, the other way round
        "alSwap" => {
            let vals = PF_EDIT.with(|e| e.borrow().as_ref().map(|x| x.1.clone())).unwrap_or_default();
            let (a, b) = (vals.get("e.color").cloned().unwrap_or_default(), vals.get("e.color2").cloned().unwrap_or_default());
            panel_put(shared, "e.color", b);
            panel_put(shared, "e.color2", a);
            return;
        }
        // an alert's sound measured again (its own adjustment kept)
        k if k.starts_with("alMeasure:") => {
            let kind = k["alMeasure:".len()..].to_string();
            match omnipulse_core::loudness::alert_levels(&kind) {
                Ok(mut m) => {
                    let mut lv = shared.borrow().web.prefs["alertLevels"].as_object().cloned().unwrap_or_default();
                    m["adj"] = lv.get(&kind).map(|x| x["adj"].clone()).filter(|a| !a.is_null()).unwrap_or(serde_json::json!(0));
                    lv.insert(kind, m);
                    pref_put(shared, "alertLevels", serde_json::Value::Object(lv));
                    push_out(shared, true);
                    "Measured again.".to_string()
                }
                Err(e) => format!("Couldn't measure it: {e}"),
            }
        }
        // What Counts as Safe: one added, at once (its switches kept as they are now)
        "safeAdd" => {
            let vals = PF_EDIT.with(|e| e.borrow().as_ref().map(|x| x.1.clone())).unwrap_or_default();
            let (name, kind) = (vals.get("safeName").and_then(|v| v.as_str()).unwrap_or("").to_string(), vals.get("safeKind").and_then(|v| v.as_str()).unwrap_or("artist").to_string());
            let mut prefs = shared.borrow().web.prefs.clone();
            prefs["streamCleared"] = streamsafe::panel_save(&prefs, &vals);
            match streamsafe::add(&prefs, &kind, &name) {
                Some(v) => { pref_put(shared, "streamCleared", v); panel_open(shared, "safe"); format!("{} is cleared for streaming.", name.trim()) }
                None if name.trim().is_empty() => "Write its name first.".to_string(),
                None => format!("{} is already on the list.", name.trim()),
            }
        }
        "widgetHide" => {
            let key = shared.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default();
            if let Ok(mut h) = integrations::WIDGET_HIDDEN.lock() { *h = key; }
            let s2 = shared.clone();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || push_out(&s2, false));
            "The widget's card is hidden until the next track.".to_string()
        }
        "out2Restart" => {
            omnipulse_core::omni_input::stop();
            offset_apply(shared);
            "OmniPulse Offset started again.".to_string()
        }
        _ => return,
    };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
}

/// Don't recommend this track (its album, its artist): saved, out of the page if it's a page of picks for
/// you, and out of autoplay's picks still to come
fn hide_track(shared: &Shared, t: &Track, what: &str) {
    let Some((changes, said)) = hide::hide(&shared.borrow().web.prefs, t, what) else { return };
    for (k, v) in changes { pref_put(shared, k, v); }
    hide_applied(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(said.into()); }
}

fn hide_applied(shared: &Shared) {
    let prefs = shared.borrow().web.prefs.clone();
    discover::hide_set(&prefs);
    let mut a = shared.borrow_mut();
    let picks = matches!(a.page.as_str(), "discover" | "rediscover" | "mix");
    if picks { a.page_all.retain(|x| hide::rec_ok(&prefs, x)); }
    apply_filter(&mut a);
    let cur = a.queue.qi.unwrap_or(0);
    let mut i = 0;
    a.queue.items.retain(|q| { let keep = i <= cur || !q.auto || hide::rec_ok(&prefs, &q.t); i += 1; keep });
    paint_queue(&mut a);
}

/// Saved: into the prefs, and what uses them started again with them
/// The settings page open, if it's one of a widget's (the Stream Widget, its alerts, the Lyrics Widget): the
/// widget's state with what's on the page so far, for its pop-out preview; none open, the preview over
fn widget_preview(shared: &Shared) {
    let Some((id, vals)) = PF_EDIT.with(|e| e.borrow().clone()) else {
        if omnipulse_core::widget::preview_on() { omnipulse_core::widget::preview_set(None); }
        return;
    };
    let Some(name) = id.strip_prefix("panel:") else { return };
    if !matches!(name, "widget" | "lyricsw" | "alerts" | "alertlist") && !name.starts_with("alert:") { return; }
    let a = shared.borrow();
    let mut prefs = a.web.prefs.clone();
    let changes: Vec<(String, serde_json::Value)> = if let Some(k) = name.strip_prefix("alert:") {
        alerts::edit_save(&a.web.prefs, k, &vals).into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    } else if name == "alertlist" {
        alerts::list_save(&a.web.prefs, &vals).into_iter().map(|(k, v)| (k.to_string(), v)).collect()
    } else {
        vals.into_iter().collect()
    };
    if let Some(o) = prefs.as_object_mut() {
        for (k, v) in changes { o.insert(k, v); }
        // (shown whether they're switched on yet or not)
        for k in ["widget", "lyricsWidget", "alertsOn"] { o.insert(k.into(), serde_json::json!(true)); }
    }
    let loading = a.ui.upgrade().is_some_and(|u| u.get_track_loading());
    let (pos, dur) = if loading { (0, 0) } else { (a.engine.position_ms(), a.engine.duration_ms()) };
    let by = a.queue.qi.and_then(|i| a.queue.items.get(i)).map(|q| q.by.as_str()).unwrap_or("");
    let mut state = integrations::widget_state(&prefs, a.queue.current(), by, pos, dur, a.loaded && !a.engine.is_paused(), (&a.accent.0, &a.accent.1), &a.lyrics);
    // (an alert's page: that one shown over and over; the alerts' pages: the first of them)
    let kind = name.strip_prefix("alert:").map(String::from).or_else(|| matches!(name, "alerts" | "alertlist").then(|| alerts::order(&prefs).into_iter().next()).flatten());
    if let (Some(o), Some(k)) = (state.as_object_mut(), kind) {
        let test = alerts::KINDS.iter().find(|x| x.0 == k).and_then(|x| serde_json::from_str::<serde_json::Value>(x.2).ok()).unwrap_or_else(|| serde_json::json!({ "user": "Someone" }));
        let mut al = test.as_object().cloned().unwrap_or_default();
        al.insert("type".into(), serde_json::json!(k));
        o.insert("preview_alert".into(), serde_json::Value::Object(al));
    }
    omnipulse_core::widget::preview_set(Some(state));
}

/// The pop-out preview: the widget being set up, in a window of its own (Edge's app window, no browser
/// around it; else the default browser), as it'll look, changing as the page does
fn widget_preview_open(shared: &Shared, name: &str) {
    let prefs = shared.borrow().web.prefs.clone();
    let port = prefs["widgetPort"].as_u64().unwrap_or(7878);
    // (its server, if neither widget's on yet: for the preview)
    let mut on = prefs.clone();
    if let Some(o) = on.as_object_mut() { o.insert("widget".into(), serde_json::json!(true)); }
    if !omnipulse_core::widget::preview_on() || integrations::widget_start(&prefs).is_none() { let _ = integrations::widget_start(&on); }
    widget_preview(shared);
    let (path, w, h) = if name == "lyricsw" { ("lyrics", 1100, 420) } else { ("", 720, 360) };
    let url = format!("http://127.0.0.1:{port}/{path}?preview=1");
    // (in OmniPulse's own window, named for it: Task Manager lists it under OmniPulse; the browser only if
    // that can't open)
    let title = format!("{} Preview", panels::title(if name.starts_with("alert") { "alerts" } else { name }));
    if let Err(e) = web_preview(name, &title, &url, w, h) {
        eprintln!("preview window: {e}");
        let _ = std::process::Command::new("cmd").args(["/C", "start", "", &url]).spawn();
    }
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("The preview is open. It updates as you change the settings, before you save.".into()); }
}

fn panel_save(shared: &Shared, name: &str, vals: serde_json::Map<String, serde_json::Value>) {
    if let Some(id) = name.strip_prefix("social:") {
        let all = socials::edit_save(&shared.borrow().web.prefs, id, &vals);
        pref_put(shared, "socials", all);
        push_out(shared, true);
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Saved.".into()); }
        return panel_open(shared, "socials");
    }
    if name == "socials" {
        let all = socials::list_save(&shared.borrow().web.prefs, &vals);
        pref_put(shared, "socials", all);
    }
    if docksets::page_of(name).is_some() {
        let all = docksets::saved(&shared.borrow().web.prefs, name, &vals);
        omnipulse_core::docks::page_settings_set(all.clone());
        pref_put(shared, "dockSet", all);
        PF_EDIT.with(|e| e.borrow_mut().take());
        // (you stay where you were: the page again, as it's saved)
        panel_open(shared, name);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{} saved. OBS shows it within a few seconds.", docksets::title(name)).into()); }
        return;
    }
    // (an output's look: kept as what differs from the first's, then back to the widget's page)
    if let Some((kind, n)) = panels::output_of(name) {
        let first = panels::values(kind, &shared.borrow().web.prefs);
        let over: serde_json::Map<String, serde_json::Value> = panels::look_keys(kind).into_iter()
            .filter_map(|k| vals.get(k).filter(|v| first.get(k).is_none_or(|f| !panels::same(f, v))).map(|v| (k.to_string(), v.clone())))
            .collect();
        let mut outs = integrations::outputs_of(&shared.borrow().web.prefs, kind);
        while outs.len() < n - 1 { outs.push(serde_json::Map::new()); }
        outs[n - 2] = over;
        outputs_put(shared, kind, outs);
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Output {n}'s look saved.").into()); }
        let kind = kind.to_string();
        return panel_open(shared, &kind);
    }
    // (the alerts: the ones switched off; an alert's page, its own settings, then back to the list)
    if name == "alertlist" || name.starts_with("alert:") {
        let changes = { let p = &shared.borrow().web.prefs; if let Some(k) = name.strip_prefix("alert:") { alerts::edit_save(p, k, &vals) } else { alerts::list_save(p, &vals) } };
        for (k, v) in changes { pref_put(shared, k, v); }
        push_out(shared, true);
        if name.starts_with("alert:") { panel_open(shared, "alertlist"); return; }
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); ui.set_status("Alerts saved.".into()); }
        return;
    }
    // (What Counts as Safe: its switches, into the list)
    if name == "safe" {
        let v = streamsafe::panel_save(&shared.borrow().web.prefs, &vals);
        pref_put(shared, "streamCleared", v);
        PF_EDIT.with(|e| e.borrow_mut().take());
        { let mut a = shared.borrow_mut(); apply_filter(&mut a); paint_queue(&mut a); }
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); ui.set_status("What counts as safe is saved.".into()); }
        return;
    }
    // (Chat Replies: the ones in your own words, as the web app keeps them in reqMsgs)
    if name == "replies" {
        let mut m = serde_json::Map::new();
        for (k, _) in requests::MSG_KEYS {
            if let Some(v) = vals.get(&format!("msg:{k}")).and_then(|v| v.as_str()).map(str::trim).filter(|v| !v.is_empty()) { m.insert(k.into(), serde_json::json!(v)); }
        }
        pref_put(shared, "reqMsgs", serde_json::Value::Object(m));
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); ui.set_status("Chat replies saved.".into()); }
        return;
    }
    // (Lyrics: their order, as the web app keeps it)
    if name == "lyrics" {
        pref_put(shared, "lyricsOrder", panels::lyrics_save(&vals));
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); ui.set_status("Lyrics sources saved.".into()); }
        return;
    }
    // (Plugins: adding and removing are done as they're pressed; nothing to keep)
    if name == "plugins" {
        PF_EDIT.with(|e| e.borrow_mut().take());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); }
        return;
    }
    for k in panels::keys_of(name) {
        if let Some(v) = vals.get(k) { pref_put(shared, k, v.clone()); }
    }
    if name == "requests" && vals.keys().any(|k| k.starts_with("bot:")) {
        let all = {
            let mut a = shared.borrow_mut();
            let mut all = a.web.prefs["pluginSettings"].as_object().cloned().unwrap_or_default();
            let mut mine = all.get(botpage::BOT).and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let mut changed = serde_json::Map::new();
            for (k, v) in vals.iter().filter_map(|(k, v)| k.strip_prefix("bot:").map(|k| (k, v))) {
                // (numbers typed as text: kept as numbers, as the chatbot's own page keeps them)
                let v = match (k, v) { ("srCooldown" | "srCost", serde_json::Value::String(t)) => serde_json::json!(t.trim().parse::<u64>().unwrap_or(0)), _ => v.clone() };
                if mine.get(k) != Some(&v) { mine.insert(k.into(), v.clone()); changed.insert(k.into(), v); }
            }
            all.insert(botpage::BOT.into(), serde_json::Value::Object(mine));
            let all = serde_json::Value::Object(all);
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pluginSettings".into(), all.clone()); }
            (!changed.is_empty()).then_some((all, changed))
        };
        if let Some((all, changed)) = all {
            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pluginSettings", all); }
            if let Some(p) = plugrun::get(botpage::BOT) {
                // (only how things look or behave changed: given to the running Suite, nothing restarted; an
                // account, a connection or a command changed: restarted, so it connects with them)
                if changed.keys().all(|k| botpage::live_key(k)) {
                    let rt = shared.borrow().rt.clone();
                    let vals = serde_json::Value::Object(changed);
                    rt.spawn(async move { let _ = p.call("applySettings", vec![vals]).await; });
                } else { plugrun::stop(botpage::BOT); plugins_start(shared); }
            }
        }
    }
    panel_keys(shared);
    match name {
        "requests" => { omnipulse_core::requests::requests_stop(); outside_start(shared); }
        // (the server restarted only if it's no longer wanted, or its port or access changed: else the docks
        // in OBS lost it for a moment with every save)
        "widget" | "alerts" => {
            if !integrations::widget_wanted(&shared.borrow().web.prefs) { omnipulse_core::widget::widget_stop(); }
            outside_start(shared);
            push_out(shared, true);
        }
        "remote" => { omnipulse_core::remote::remote_stop(); outside_start(shared); }
        "offset" => offset_apply(shared),
        _ => {}
    }
    PF_EDIT.with(|e| e.borrow_mut().take());
    paint_settings(&shared.borrow());
    // (you stay where you were: the page again, as it's saved)
    panel_open(shared, name);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{} settings saved.", panels::title(name)).into()); }
}

/// A plugin's settings dialog: its manifest's fields, as they're set (prefs.pluginSettings, as the web app's)
fn pf_open(shared: &Shared, id: &str) {
    let Some(p) = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id) else { return };
    let now = plugrun::settings_of(&p.manifest, &shared.borrow().web.prefs);
    let vals = now.as_object().cloned().unwrap_or_default();
    PF_ORIG.with(|o| *o.borrow_mut() = vals.clone());
    PF_EDIT.with(|e| *e.borrow_mut() = Some((id.to_string(), vals)));
    let bot = id == botpage::BOT;
    if bot { BOT_SHEET.with(|b| b.borrow_mut().clear()); BOT_FOCUS.with(|f| f.borrow_mut().clear()); BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default()); BOT_WIDGETS.with(|w| w.borrow_mut().clear()); }
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_bot_widgets("".into()); }
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_bot_page(bot); ui.set_leave_ask(false); }
    pf_paint(shared);
    pf_mark(shared);
    if bot { bot_poll(shared); }
    if let Some(ui) = shared.borrow().ui.upgrade() {
        let cat = if p.manifest.section.is_empty() { "Plugins".to_string() } else { p.manifest.section.clone() };
        crumbs_set(&ui, vec![("Settings".into(), "root".into()), (cat.clone(), format!("cat:{cat}"))]);
        ui.set_pf_head(place_label(&p.manifest).into());
        ui.set_pf_open(true);
    }
}

fn pf_fields(id: &str) -> Vec<serde_json::Value> {
    let Some(p) = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id) else { return Vec::new() };
    // (every plugin's: whether its icon is in the title bar; then its manifest's own)
    let mut f = vec![serde_json::json!({ "key": PLUG_TITLE_BAR, "label": "Show in the Title Bar", "type": "toggle", "default": false,
        "note": "Shows its icon next to SoundCloud, Spotify and YouTube Music. Click the icon to turn the plugin on or off, and right-click it for these settings." })];
    f.extend(p.manifest.settings.as_array().cloned().unwrap_or_default());
    f
}

fn pf_paint(shared: &Shared) {
    let Some((id, vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    if let Some(name) = id.strip_prefix("panel:") { return panel_paint(shared, name, &vals); }
    if id == botpage::BOT { return bot_paint(shared); }
    let text = |v: &serde_json::Value| match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Null => String::new(), x => x.to_string() };
    let rows: Vec<PlugField> = pf_fields(&id).iter().filter_map(|f| {
        let key = f["key"].as_str()?.to_string();
        let kind = f["type"].as_str().unwrap_or("text").to_string();
        let options: Vec<slint::SharedString> = f["options"].as_array().into_iter().flatten().map(|o| text(o).into()).collect();
        let at = f["options"].as_array().and_then(|o| o.iter().position(|o| Some(o) == vals.get(&key))).unwrap_or(0) as i32;
        // (the chatbot's Actions: a button that opens their page)
        let (kind, press) = if kind == "actions" { ("button".to_string(), "Open Actions") } else { let p = f["press"].as_str().unwrap_or("Go"); (kind, p) };
        Some(PlugField { changed: pf_changed(&key), label: f["label"].as_str().unwrap_or(&key).into(), note: f["note"].as_str().unwrap_or("").into(), value: if kind == "button" { press.into() } else { vals.get(&key).map(text).unwrap_or_default().into() }, kind: kind.into(), options: ModelRc::from(Rc::new(VecModel::from(options))), key: key.into(), at })
    }).collect();
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_fields(ModelRc::from(Rc::new(VecModel::from(rows)))); }
}

/// One setting changed in the dialog (a select's "next": on to its next option)
fn pf_set(shared: &Shared, i: usize, v: &str) {
    let Some((id, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    if let Some(name) = id.strip_prefix("panel:") { return panel_set(shared, name, vals, i, v); }
    let Some(f) = pf_fields(&id).get(i).cloned() else { return };
    let Some(key) = f["key"].as_str().map(String::from) else { return };
    // (the chatbot's Actions: a page of its own)
    if f["type"].as_str() == Some("actions") { return flows_open(shared); }
    // (a button of its own: the method it names called, with the page's values as they are, unsaved; its
    // answer said)
    if f["type"].as_str() == Some("button") {
        let (Some(p), Some(method)) = (plugrun::get(&id), f["call"].as_str().map(String::from)) else {
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Switch the plugin on (and save) first.".into()); }
            return;
        };
        let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
        let now = serde_json::Value::Object(vals.clone());
        rt.spawn(async move {
            let r = p.call(&method, vec![now]).await;
            let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(match r { Ok(v) => v.as_str().map(String::from).unwrap_or_else(|| "Done.".into()), Err(e) => e }.into()));
        });
        return;
    }
    let value = match f["type"].as_str().unwrap_or("text") {
        "toggle" => serde_json::json!(v == "true"),
        "number" => v.trim().parse::<f64>().map(|n| serde_json::json!(n)).unwrap_or(serde_json::Value::Null),
        "select" => {
            let opts: Vec<serde_json::Value> = f["options"].as_array().cloned().unwrap_or_default();
            let at = match v.strip_prefix('\u{2}').and_then(|k| k.parse::<usize>().ok()) {
                Some(k) => k.min(opts.len().saturating_sub(1)),
                None => opts.iter().position(|o| Some(o) == vals.get(&key)).map_or(0, |k| (k + 1) % opts.len().max(1)),
            };
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
    if let Some(name) = id.strip_prefix("panel:") { return panel_save(shared, name, vals); }
    let mut fields: Vec<String> = pf_fields(&id).iter().filter(|f| !matches!(f["type"].as_str(), Some("button" | "actions"))).filter_map(|f| f["key"].as_str().map(String::from)).collect();
    // (the Suite's pages have settings of their own the manifest doesn't list (the visualiser's scene, the credits'
    // parts...): whatever was changed on the page is kept too)
    if id == botpage::BOT {
        let orig = PF_ORIG.with(|o| o.borrow().clone());
        for (k, v) in &vals { if !fields.contains(k) && orig.get(k) != Some(v) { fields.push(k.clone()); } }
    }
    let all = {
        let mut a = shared.borrow_mut();
        let mut all = a.web.prefs["pluginSettings"].as_object().cloned().unwrap_or_default();
        // (its fields over what's kept: a sign-in's tokens or its actions, kept while the page was open, stay)
        let mut mine = all.get(&id).and_then(|v| v.as_object()).cloned().unwrap_or_default();
        for k in fields { if let Some(v) = vals.get(&k) { mine.insert(k, v.clone()); } }
        all.insert(id.clone(), serde_json::Value::Object(mine));
        let all = serde_json::Value::Object(all);
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pluginSettings".into(), all.clone()); }
        all
    };
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pluginSettings", all); }
    plugrun::stop(&id);
    plugins_start(shared);
    paint_plats(&shared.borrow());
    if id == botpage::BOT {
        PF_ORIG.with(|o| *o.borrow_mut() = vals.clone());
        PF_EDIT.with(|e| *e.borrow_mut() = Some((id, vals)));
        BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default());
        pf_mark(shared);
        bot_paint(shared);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Chatbot settings saved.".into()); }
        return;
    }
    PF_ORIG.with(|o| o.borrow_mut().clear());
    pf_mark(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); ui.set_status("Plugin settings saved.".into()); }
}

/// A plugin: on unless switched off (the web app's prefs.plugOn_<id>). The Plugins page's switch: its master
/// switch, off leaving no trace of it anywhere else
fn plugin_on(prefs: &serde_json::Value, id: &str) -> bool {
    prefs[format!("plugOn_{id}")].as_bool() != Some(false)
}

/// A plugin running: on (its master switch), and, for one whose settings live in a category of their own
/// (the chatbot's in Streaming), switched on there too (prefs.plugRun_<id>). Off there, its settings stay
fn plugin_runs(prefs: &serde_json::Value, id: &str) -> bool {
    plugin_on(prefs, id) && prefs[format!("plugRun_{id}")].as_bool() != Some(false)
}

/// A plugin's name as shown: "(WIP)" after it while it's still being built
fn plugin_label(m: &omnipulse_core::plugins::Manifest) -> String {
    if m.wip { format!("{} (WIP)", m.name) } else { m.name.clone() }
}

/// Where a plugin's page is, as its path says it: the Stream Suite is "Suite" (Settings > Streaming > Suite)
fn place_label(m: &omnipulse_core::plugins::Manifest) -> String {
    if m.id == botpage::BOT { "Suite".into() } else { plugin_label(m) }
}

/// A plugin's state, for its row: off, running, starting or why it couldn't
fn plugin_state(prefs: &serde_json::Value, m: &omnipulse_core::plugins::Manifest) -> String {
    if !plugin_runs(prefs, &m.id) { "Off.".to_string() } else if plugrun::get(&m.id).is_some() { "Running.".to_string() } else { PLUG_ERR.with(|e| e.borrow().get(&m.id).cloned()).map(|e| format!("Couldn't start: {e}")).unwrap_or_else(|| "Starting...".into()) }
}

thread_local! {
    /// why a plugin didn't start, by its id
    static PLUG_ERR: RefCell<HashMap<String, String>> = RefCell::new(HashMap::new());
}

/// The installed plugins that are on, started (each on its own thread: crate::plugrun); the rest stopped
fn plugins_start(shared: &Shared) {
    hotkeys_update(shared);
    let (prefs, rt) = { let a = shared.borrow(); (a.web.prefs.clone(), a.rt.clone()) };
    panels::SUITE_ON.store(plugin_runs(&prefs, botpage::BOT), std::sync::atomic::Ordering::Relaxed);
    for p in omnipulse_core::plugins::list() {
        let id = p.manifest.id.clone();
        if !plugin_runs(&prefs, &id) { plugrun::stop(&id); continue; }
        if plugrun::get(&id).is_some() { continue; }
        // (a self-test's: never your accounts: their sign-ins left out and every connection off, so a test never
        // reaches Twitch, Kick, OBS or the tip services as you)
        let mut set = plugrun::settings_of(&p.manifest, &prefs);
        if std::env::var_os("OMNIPULSE_SELFTEST").is_some() {
            if let Some(o) = set.as_object_mut() {
                let secret = |k: &str| { let k = k.to_lowercase(); ["token", "secret", "password", "refresh", "verification", "jwt", "userid", "login", "channel", "url", "address"].iter().any(|w| k.contains(w)) };
                o.retain(|k, _| !secret(k));
                for k in ["kickOn", "obsOn", "meldOn", "slobsOn", "throneOn", "ytOn", "youtubeOn"] { o.insert(k.into(), serde_json::json!(false)); }
            }
        }
        match plugrun::start(&p, &set, rt.clone()) {
            Ok(_) => PLUG_ERR.with(|e| e.borrow_mut().remove(&id)),
            Err(e) => PLUG_ERR.with(|x| x.borrow_mut().insert(id, e)),
        };
    }
    paint_settings(&shared.borrow());
}

// ---------- the sidebar's pages: your order, and the ones you've hidden ----------
const NAV_IDS: [&str; 7] = ["navDiscover", "navRediscover", "navLikes", "navFavourites", "navSets", "navArtists", "navStats"];

fn nav_ids(prefs: &serde_json::Value) -> Vec<String> {
    let order: Vec<String> = prefs["navOrder"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    let rank = |id: &str| order.iter().position(|o| o == id).unwrap_or(order.len() + NAV_IDS.iter().position(|n| *n == id).unwrap_or(0));
    let mut ids: Vec<String> = NAV_IDS.iter().map(|s| s.to_string()).collect();
    ids.sort_by_key(|id| rank(id));
    let off = |key: &str| prefs[key].as_bool() == Some(false);
    ids.retain(|id| !(id == "navFavourites" && off("favPage")) && !(id == "navSets" && off("setsOn")) && !(id == "navStats" && off("statsShown")));
    // (Stream-Safe Music on: the Streamer Mode page stands in for Discover, Rediscover, the Library,
    // Favourites, Sets & Podcasts and Artists, which are hidden meanwhile, as the web app's)
    if streamsafe::on(prefs) { ids.retain(|id| id == "navDiscover" || id == "navStats"); }
    ids
}

fn nav_paint(shared: &Shared) {
    let ids = nav_ids(&shared.borrow().web.prefs);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_nav_ids(ModelRc::from(Rc::new(VecModel::from(ids.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()))));
    }
}

/// A page dragged onto another: it goes above it (all of them kept in the order, the hidden too)
fn nav_move(shared: &Shared, from: &str, before: &str) {
    if from == before { return; }
    let mut order = { let a = shared.borrow(); let mut all = nav_ids(&serde_json::json!({ "navOrder": a.web.prefs["navOrder"].clone() })); for n in NAV_IDS { if !all.iter().any(|x| x == n) { all.push(n.to_string()); } } all };
    order.retain(|x| x != from);
    let at = order.iter().position(|x| x == before).unwrap_or(order.len());
    order.insert(at, from.to_string());
    let v = serde_json::json!(order);
    if let Some(o) = shared.borrow_mut().web.prefs.as_object_mut() { o.insert("navOrder".into(), v.clone()); }
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("navOrder", v); }
    nav_paint(shared);
}

fn set_toggle(shared: &Shared, key: &str) {
    if let Some(id) = key.strip_prefix("plugRun_") {
        let on = {
            let mut a = shared.borrow_mut();
            let on = !plugin_runs(&a.web.prefs, id);
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert(key.into(), serde_json::json!(on)); }
            on
        };
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(key, serde_json::json!(on)); }
        plugins_start(shared);
        paint_settings(&shared.borrow());
        paint_plats(&shared.borrow());
        return;
    }
    if let Some(id) = key.strip_prefix("plugOn_") {
        let on = {
            let mut a = shared.borrow_mut();
            let on = !plugin_on(&a.web.prefs, id);
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert(key.into(), serde_json::json!(on)); }
            on
        };
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(key, serde_json::json!(on)); }
        plugins_start(shared);
        // (the title bar's button for it: gone or back)
        paint_plats(&shared.borrow());
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
            accounts_paint(shared);
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
            if key == "smartOn" {
                SMART_PAIR.with(|p| *p.borrow_mut() = None);
                if on { PRELOADED.with(|p| p.borrow_mut().clear()); preload_next(shared); }
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if on { "Seamless Mix is on. It's WIP: some transitions may not sound right yet." } else { "Seamless Mix is off." }.into()); }
            }
            // the servers stop, and start again with what's set now
            match key {
                "widget" | "alertsOn" | "lyricsWidget" | "socials" => if !integrations::widget_wanted(&shared.borrow().web.prefs) { omnipulse_core::widget::widget_stop() },
                "req" => omnipulse_core::requests::requests_stop(),
                "remoteOn" => omnipulse_core::remote::remote_stop(),
                "presence" if !on => integrations::presence(&shared.borrow().rt, &shared.borrow().web.prefs, None, 0, false, None),
                _ => {}
            }
            if matches!(key, "widget" | "req" | "remoteOn" | "alertsOn" | "lyricsWidget" | "socials") && (on || matches!(key, "alertsOn" | "lyricsWidget" | "socials")) { panel_keys(shared); outside_start(shared); }
            if key == "mbOn" { go_plus_set(&shared.borrow().web.prefs); }
            if key == "out2On" { offset_apply(shared); }
            // Listen Along & Jams off: out of any room, and the Jam button goes
            if key == "jamsOn" {
                if !on { ASK.with(|x| x.borrow_mut().take()); room_end(true); push_out(shared, true); }
                jam_paint(shared);
            }
            if key == "hideOn" { hide_applied(shared); }
            // Chromecast off while casting: the cast ends, and the sound comes back here
            if key == "castOn" && !on && casting().is_some() {
                omnipulse_core::cast::stop();
                cast_end(shared, Some("Chromecast off: playing here again.".into()));
            }
            if key == "smartOn" { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_mix_on(on); } }
            // (Seamless Mix off: its beat model let go now, not kept for nothing)
            if key == "smartOn" && !on { omnipulse_core::rhythm::release_model(std::time::Duration::ZERO); }
            if key == "favPage" {
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_fav_page(on); }
                if !on && shared.borrow().page == "favourites" { open_page(shared, "library"); }
            }
            // a page switched off: out of the sidebar (and off it, if it's open)
            if matches!(key, "favPage" | "setsOn" | "statsShown") {
                nav_paint(shared);
                let page = match key { "setsOn" => "sets", "statsShown" => "stats", _ => "favourites" };
                if !on && shared.borrow().page == page { open_page(shared, "library"); }
            }
            if key == "presence" && on { push_out(shared, true); }
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{label} {}.", if on { "on" } else { "off" }).into()); }
        }
    }
    paint_settings(&shared.borrow());
    if matches!(key, "scOn" | "spOn" | "ytOn" | "scrobble") { accounts_paint(shared); }
}

/// The Library and your playlists, from every platform you're signed in to (at the start, and again
/// after signing in or out)
fn library_load(shared: &Shared) {
    let (weak, rt) = { let a = shared.borrow(); (a.ui.clone(), a.rt.clone()) };
    {
        let yt_likes = shared.borrow().web.yt_likes.clone();
        let own = xpl::playlists(&shared.borrow().xpl);
        {
            let mut steps: Vec<(&'static str, &'static str)> = Vec::new();
            if omnipulse_core::soundcloud::signed_in() { steps.push(("soundcloud", "SoundCloud")); }
            if omnipulse_core::spotify::spotify_signed_in() { steps.push(("spotify", "Spotify")); }
            loading::begin(&["library", "rediscover"], &steps);
        }
        rt.spawn(async move {
            let t0 = std::time::Instant::now();
            let sc = async { let x = sources::soundcloud_likes().await; loading::done("soundcloud"); x };
            let sp = async { let x = sources::spotify_likes(500).await; loading::done("spotify"); x };
            let (sc, sp, lists) = tokio::join!(sc, sp, sources::playlists());
            // (each platform's likes newest first, taking turns: the Library mixes them track by track, not
            // one platform's then the next's; YouTube Music's, the web app's and those made here, as one)
            let mut yt = yt_likes;
            yt.extend(marks::yt_likes());
            let mut all: Vec<Track> = sources::interleave(vec![sc, sp, yt]);
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
                        loading::end(&["library", "rediscover"]);
                        let prefs = a.web.prefs.clone();
                        track::apply_fixes(&prefs, &mut a.library);
                        a.playlists = playlists;
                        paint_sides(&mut a);
                        paint_plats(&a);
                    }
                    // (Discover made before your likes came in: made from next to nothing, so made again with them,
                    // at once if it's open)
                    FEED_CACHE.with(|c| *c.borrow_mut() = None);
                    if shared.borrow().page == "discover" { open_discover_fresh(&shared); }
                    // (playlists on this PC stay here: nothing's kept in step by itself; Export puts them on the platforms)
                    // (where you left off last time, once: unless you've gone somewhere since it started)
                    let last = LAST_PLACE_DONE.with(|d| !d.replace(true)).then(|| webstate::native_pref("lastPlace")).flatten();
                    let untouched = shared.borrow().page == "library" && shared.borrow().history.is_empty();
                    let back_to = last.filter(|_| untouched).and_then(|v| {
                        if let Some(id) = v["playlist"].as_str() { return shared.borrow().playlists.iter().any(|p| p.id == id).then(|| Place::Playlist(id.to_string())); }
                        v["page"].as_str().filter(|p| *p != "library").map(|p| Place::Page(p.to_string()))
                    });
                    if let Some(place) = back_to {
                        go(&shared, place);
                    } else if shared.borrow().page == "library" {
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
const ACCOUNTS: [(&str, &str); 4] = [("soundcloud", "SoundCloud"), ("spotify", "Spotify"), ("youtube", "YouTube Music"), ("lastfm", "Last.fm")];
/// Each account's platform switch, with it in Accounts (not in the list below it)
fn account_switch(key: &str) -> &'static str { match key { "soundcloud" => "scOn", "spotify" => "spOn", "youtube" => "ytOn", _ => "scrobble" } }

/// Spotify only lets Premium accounts sign in its own way
/// set once Spotify has turned its own sign-in down (a free account): the next sign-in is your app's
static SP_FREE_SEEN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

const SP_FREE: &str = "That's a free Spotify account. Spotify only lets Premium accounts sign in directly. A free account can sign in through the Spotify app of someone with Premium who adds you to it. The steps are open.";

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
    let prefs = shared.borrow().web.prefs.clone();
    // (Streamer Mode: who you're signed in as isn't shown)
    let private = integrations::set(&prefs, "streamerMode");
    let rows: Vec<Account> = ACCOUNTS.iter().map(|&(key, name)| {
        let signed = account_signed(key);
        let busy = ACCOUNT_BUSY.with(|b| b.borrow().contains(key));
        let who = if private { None } else { ACCOUNT_NAMES.with(|n| n.borrow().get(key).cloned()).filter(|s| !s.is_empty()) };
        // (its tile's line: who, short; all of it, in its tip)
        let short = if busy { "Signing in...".to_string() } else if !signed { "Not signed in".to_string() } else {
            let plan = if key == "spotify" && omnipulse_core::spotify_engine::spe_signed_in() { match omnipulse_core::spotify_engine::spe_premium() { Some(false) => " · Free", _ => " · Premium" } } else { "" };
            format!("{}{plan}", who.clone().unwrap_or_else(|| "Signed in".into()))
        };
        let status = if busy {
            "Finish signing in in your browser.".to_string()
        } else if signed {
            match (key, who) {
                ("spotify", Some(w)) if omnipulse_core::spotify_engine::spe_signed_in() && omnipulse_core::spotify_engine::spe_premium() == Some(false) => format!("Signed in as {w} · Free: your likes and playlists are here; Spotify songs play from YouTube Music."),
                ("spotify", Some(w)) if omnipulse_core::spotify_engine::spe_signed_in() => format!("Signed in as {w} · Premium: Spotify plays in full here."),
                ("spotify", Some(w)) => format!("Signed in as {w} · Free: Spotify tracks play from YouTube Music."),
                (_, Some(w)) => format!("Signed in as {w}."),
                ("spotify", None) if private && omnipulse_core::spotify_engine::spe_premium() == Some(false) => "Signed in (your name's hidden in Streamer Mode) · Free: Spotify songs play from YouTube Music.".to_string(),
                ("spotify", None) if private => "Signed in (your name is hidden in Streamer Mode) · Premium, so Spotify plays in full here.".to_string(),
                _ if private => "Signed in (your name is hidden in Streamer Mode).".to_string(),
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
        // (Spotify, signed in: your own app keys, optional, for an allowance of requests of your own)
        let keys = if key == "spotify" && signed && !busy { if keysetup::has_keys("spotify") { "Change keys" } else { "Own keys" } } else { "" };
        let status = if key == "spotify" && signed && omnipulse_core::spotify::spotify_app_signed_in() { format!("{status} Requests go through your own app keys.") } else { status };
        let sw = account_switch(key);
        let on = SET_ROWS.iter().find(|r| r.3 == sw).map(|r| set_row_on(&prefs, sw, r.4)).unwrap_or(true);
        // (each platform's plans, as the web app's Manage subscription links)
        let manage = match key { "soundcloud" => "https://soundcloud.com/you/subscriptions", "spotify" => "https://www.spotify.com/account/subscription/manage/", "youtube" => "https://www.youtube.com/paid_memberships", _ => "https://www.last.fm/settings/subscription" };
        Account { short: short.into(), name: name.into(), status: status.into(), signed, busy, keys: keys.into(), switch: sw.into(), on, manage: if signed { manage.into() } else { "".into() }, settings: if key == "youtube" { "youtube".into() } else { "".into() } }
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
        // (Spotify limiting the Web API, which the playback sign-in shares with everyone: its own login's username)
        let sp = match sp {
            None if omnipulse_core::spotify_engine::spe_signed_in() => omnipulse_core::spotify_engine::spe_account().await.ok().and_then(|v| v["id"].as_str().filter(|x| !x.is_empty()).map(String::from)),
            sp => sp,
        };
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
        KS_SIGNED_IN.with(|k| k.set(false));
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
                KS_SIGNED_IN.with(|k| k.set(false));
                return ks_open(&shared, "spotify-free", i);
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

thread_local! {
    /// the key setup opened for an account signed in already (its own keys, optional): saved, it
    /// signs in with them beside it, rather than pressing the account's button (Sign out)
    static KS_SIGNED_IN: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

const SP_OWN_KEYS: &str = "Optional. Spotify limits how many requests each app can make. The normal sign-in shares one limit with everyone who uses it, so OmniPulse sometimes has to wait. Your own keys give you a limit of your own, and the normal sign-in stays as a backup. Spotify requires the app's owner to have Premium. With a free account, use the Client ID of someone with Premium who has added you.";

fn ks_open(shared: &Shared, plat: &str, account: usize) {
    let Some(s) = keysetup::setup(plat) else { return };
    KS.with(|k| *k.borrow_mut() = Some((plat.to_string(), account, vec![String::new(); s.fields.len()])));
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    ui.set_ks_head(format!("Set up {}: your own keys", s.name).into());
    let why = if KS_SIGNED_IN.with(|k| k.get()) && plat == "spotify" { format!("{SP_OWN_KEYS} ") } else { String::new() };
    ui.set_ks_note(format!("{why}{} They're kept on this PC, in OmniPulse's own folder.", s.note).into());
    let steps: Vec<KeyStep> = s.steps.iter().map(|st| KeyStep { text: st.text.into(), has_url: st.url.is_some(), copies: ModelRc::from(Rc::new(VecModel::from(st.copies.iter().map(|(l, _)| slint::SharedString::from(*l)).collect::<Vec<_>>()))) }).collect();
    ui.set_ks_steps(ModelRc::from(Rc::new(VecModel::from(steps))));
    ks_paint(shared);
    ui.set_ks_open(true);
}

/// Keys found (pasted, or in a file): each into its box, and what was found said
fn ks_take(shared: &Shared, found: Vec<(usize, String)>) {
    let Some(plat) = KS.with(|k| k.borrow().as_ref().map(|x| x.0.clone())) else { return };
    let Some(s) = keysetup::setup(&plat) else { return };
    KS.with(|k| { if let Some(x) = k.borrow_mut().as_mut() { for (i, v) in &found { if let Some(f) = x.2.get_mut(*i) { *f = v.clone(); } } } });
    ks_paint(shared);
    let names: Vec<&str> = found.iter().filter_map(|(i, _)| s.fields.get(*i).map(|f| f.1)).collect();
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_ks_note(format!("Found the {}.", names.join(" and ")).into()); }
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
        ui.set_ks_note("Not yet. Each key needs a tick beside it before it can be saved.".into());
        return;
    }
    let mut fields = serde_json::Map::new();
    for ((key, _, _), v) in s.fields.iter().zip(&vals) { fields.insert((*key).into(), serde_json::json!(v.trim())); }
    match omnipulse_core::config_set(&fields) {
        Ok(()) => {
            KS.with(|k| k.borrow_mut().take());
            ui.set_ks_open(false);
            ui.set_status(format!("Your {} keys are saved. Signing in...", s.name).into());
            if KS_SIGNED_IN.with(|k| k.replace(false)) {
                // signed in already: with your own app too, beside it
                let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
                rt.spawn(async move {
                    let r = omnipulse_core::spotify::spotify_login().await;
                    let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                        ui.set_status(match &r { Ok(()) => "Signed in with your own Spotify keys. Your requests now go through them.".to_string(), Err(e) => format!("Couldn't sign in with your keys: {e}") }.into());
                        if let Some(sh) = s.borrow().clone() { accounts_refresh(&sh); }
                    }));
                });
                return;
            }
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

thread_local! {
    /// when a song that isn't measured went quiet (Skip Silences, by its level)
    static QUIET_SINCE: std::cell::Cell<Option<std::time::Instant>> = const { std::cell::Cell::new(None) };
    /// where the watch saw it last (it only counts while the position moves)
    static LAST_WATCH_POS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// What isn't measured: silent (below -55 dB, its peaks below -40, as the measurement's) for that long
/// while it plays, and not in its last 3 s: on to the next one. True if it moved on.
fn silence_watch(shared: &Shared, engine: &Engine, pos: u64, dur: u64, skip: u64) -> bool {
    let moving = LAST_WATCH_POS.with(|p| { let was = p.get(); p.set(pos); pos > was });
    if skip == 0 || listening().is_some() || MIX.with(|m| m.borrow().is_some()) { QUIET_SINCE.with(|q| q.set(None)); return false; }
    if !moving { return false; }
    let (rms, peak) = engine.level();
    if !(rms < -55.0 && peak < -40.0) { QUIET_SINCE.with(|q| q.set(None)); return false; }
    let since = QUIET_SINCE.with(|q| { if q.get().is_none() { q.set(Some(std::time::Instant::now())); } q.get().unwrap() });
    if since.elapsed().as_millis() as u64 >= skip && (dur == 0 || dur.saturating_sub(pos) > 3000) {
        QUIET_SINCE.with(|q| q.set(None));
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Moved on after {} s of silence.", skip / 1000).into()); }
        step(shared, 1);
        return true;
    }
    false
}

/// Each tick while playing: long silences jumped over, the fade out at the end. True if it moved on.
fn fade_and_silences(shared: &Shared, pos: u64, dur: u64) -> bool {
    let (an, key, fade, skip, engine) = {
        let a = shared.borrow();
        let Some(t) = a.queue.current() else { return false };
        let an = a.cur_loud.as_ref().and_then(|k| a.analysis.get(k)).cloned();
        (an, t.key.clone(), fade_out_ms(&a), sound::skip_silence_ms(&a.web.prefs), a.engine.clone())
    };
    // (not measured, as Spotify's songs aren't, or past what was measured of a long set: its level
    // watched as it plays, and on to the next after that long in silence, as the web app's)
    let measured_to = an.as_ref().map(|a| a["analysed_ms"].as_u64().unwrap_or(0));
    if measured_to.is_none_or(|m| m > 0 && pos >= m) {
        if silence_watch(shared, &engine, pos, dur, skip) { return true; }
    } else {
        QUIET_SINCE.with(|q| q.set(None));
    }
    // Smart Transitions: the next one in over this one's outro, the silences left out (crate::smart)
    let smart_tier0 = match smart_fade_tick(shared, &key, pos, dur) { Some(true) => return true, Some(false) => true, None => false };
    let Some(an) = an else { return false };
    let _ = &key;
    if dur == 0 {
        return false;
    }
    // a silence of the chosen length or more: over it, or on to the next track if it runs to the end
    // (listening along: the host's app skips them, or doesn't, and this follows it; never mid-mix)
    if skip > 0 && listening().is_none() && MIX.with(|m| m.borrow().is_none()) {
        let measured = an["analysed_ms"].as_u64().unwrap_or(0);
        for s in an["silences"].as_array().into_iter().flatten() {
            let (from, to) = (s[0].as_u64().unwrap_or(0), s[1].as_u64().unwrap_or(0));
            if to.saturating_sub(from) < skip || pos < from || pos + 500 >= to {
                continue;
            }
            if to + 1500 >= dur && measured + 1500 >= dur {
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Moved on after {} s of silence.", (dur.saturating_sub(from)) / 1000).into()); }
                step(shared, 1);
                return true;
            }
            engine.seek(to);
            host_push(&shared.borrow());   // (hosting: listeners jump with it)
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Skipped {} s of silence.", (to - from) / 1000).into()); }
            return false;
        }
    }
    // (Smart Transitions: theirs, above)
    if smart_tier0 { return false; }
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
        if sound::loud_step(p) == "off" && sound::fade_ms(p).is_none() && sound::skip_silence_ms(p) == 0 && !smart_on(p) {
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
                a.engine.set_match_db(sound::loud_db(&a.web.prefs, &t, Some(lufs), kind == "youtube"), false);
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
    ui.set_smart_max(a.web.prefs["smartMax"].as_u64().unwrap_or(10) as i32);
    let bands = sound::eq_bands(&a.web.prefs);
    for (i, g) in bands.iter().enumerate() {
        if a.eq_model.row_data(i) != Some(*g) { a.eq_model.set_row_data(i, *g); }
    }
    let (line, fill) = eq_curve(&bands);
    ui.set_eq_curve(line.into());
    ui.set_eq_curve_fill(fill.into());
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
    // (what's actually playing, as it was started: its YouTube Music version, if it plays from one)
    let cur = shared.borrow().cur_loud.clone();
    let what: Option<(&'static str, String)> = match cur.as_deref().and_then(|k| k.split_once(':')) {
        Some(("youtube", id)) => Some(("youtube", id.to_string())),
        Some(("soundcloud", id)) => Some(("soundcloud", id.to_string())),
        Some(("local", id)) => Some(("local", id.to_string())),
        _ => sound::loud_source(&t, None),
    };
    let ver = what.as_ref().is_some_and(|(k, _)| *k == "youtube") && !matches!(t.src, track::Src::YouTube(_));
    let known = what.as_ref().and_then(|(s, id)| shared.borrow().loud.get(&format!("{s}:{id}")).copied());
    { let a = shared.borrow(); let db = sound::loud_db(&a.web.prefs, &t, known, ver); if std::env::var_os("OMNIPULSE_SELFTEST").is_some() { println!("selftest: loudness {step}: {db:+.1} dB ({:?}, measured {known:?})", what); } a.engine.set_match_db(db, false); }
    // (and Spotify's own levelling, for its songs it plays: from the next one)
    let (on, target) = sound::spotify_norm(&shared.borrow().web.prefs);
    omnipulse_core::spotify_engine::spe_set_normalisation(on, target);
    if let (Some((kind, id)), None) = (what, known) { loud_measure(shared, &t, kind, id, token); }
}

// ---------- your playlists on this PC (crate::xpl) ----------
/// The playlists changed: kept, and shown again (the sidebar, the menus, and the page if it's one)
fn xpl_changed(shared: &Shared) {
    xpl_shown(shared);
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
    let Some(dest) = ask_path(dlg, Ask::Save, { let (s, p, k) = (shared.clone(), p.clone(), kind.to_string()); move || pl_export(&s, p, &k) }) else { return };
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
    let s = shared.clone();
    ask_then(rfd::FileDialog::new().set_title("Import a playlist").add_filter("Playlist", &["m3u", "m3u8", "csv", "txt"]), Ask::Open, move |src| pl_import_file(&s, src));
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
            if tracks.is_empty() { say_with(&ui, "None of its songs were found anywhere.", vec![("Copy the missing", StatusAct::Copy(missing.join("\n")))]); return; }
            let id = xpl::create(&mut shared.borrow_mut().xpl, &name, &tracks);
            xpl_changed(&shared);
            go(&shared, Place::Playlist(id));
            let note = format!("Imported \"{name}\": {}{}", plural(tracks.len(), "song", "songs"), if missing.is_empty() { ".".to_string() } else { format!(", {} found anywhere (copied, to look for).", plural(missing.len(), "song wasn't", "songs weren't")) });
            if missing.is_empty() { ui.set_status(note.into()); STATUS_HOLD.with(|h| *h.borrow_mut() = None); }
            else { say_with(&ui, &note.replace(" (copied, to look for)", ""), vec![("Copy the missing", StatusAct::Copy(missing.join("\n")))]); }
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

fn sync_plats(prefs: &serde_json::Value) -> Vec<&'static str> {
    let mut on = Vec::new();
    if plat_on(prefs, "soundcloud") && omnipulse_core::soundcloud::signed_in() { on.push("soundcloud"); }
    if plat_on(prefs, "spotify") && omnipulse_core::spotify::spotify_signed_in() { on.push("spotify"); }
    on
}

/// Export (or export again): a playlist here put on SoundCloud and Spotify, each with its own songs
/// (crate::mirror::export), when you ask
fn pl_export_platforms(shared: &Shared, id: &str) {
    let (p, on, rt, weak) = {
        let a = shared.borrow();
        let Some(p) = a.xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(id)).cloned()) else { return };
        (p, sync_plats(&a.web.prefs), a.rt.clone(), a.ui.clone())
    };
    let title = p["title"].as_str().unwrap_or("Playlist").to_string();
    if on.is_empty() {
        if let Some(ui) = weak.upgrade() { ui.set_status("Sign in to SoundCloud or Spotify to export it there.".into()); }
        return;
    }
    if let Some(ui) = weak.upgrade() { ui.set_status(format!("Exporting \"{title}\"...").into()); }
    let id = id.to_string();
    rt.spawn(async move {
        let mut p = p;
        let out = mirror::export(&mut p, &on).await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let exports = p["exports"].clone();
            xpl_set(&shared, &id, |x| { x["exports"] = exports; });
            xpl::save(&shared.borrow().xpl);
            for h in &out.hide { hidden_set(&shared, h, true); }
            paint_sides(&mut shared.borrow_mut());
            let name = |p: &str| if p == "spotify" { "Spotify" } else { "SoundCloud" };
            let done: Vec<String> = out.done.iter().map(|(p, n)| format!("{} to {}", plural(*n, &format!("{} song", name(p)), &format!("{} songs", name(p))), name(p))).collect();
            let note = if !out.errors.is_empty() { out.errors.join(" ") }
                else if done.is_empty() { format!("\"{title}\" has no SoundCloud or Spotify songs to export.") }
                else { format!("Exported \"{title}\": {}.", done.join(", ")) };
            ui.set_status(note.into());
        }));
    });
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
            let by = my_name(&shared.borrow());
            collab_send(&shared, &id, collab::add_op(&t, &by));
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
                        Ok(()) => format!("Invite link copied. Friends who open it can add songs to \"{}\" and remove them, from their own OmniPulse.", pl.title),
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
    let ask_text = if owner { format!("Stop \"{title}\" being collaborative? Everyone keeps a copy, but changes no longer go between you.") } else { format!("Leave \"{title}\"? You keep it as yours, but your changes and theirs no longer go between you.") };
    let id = id.to_string();
    ask(shared, if owner { "Stop collaborating" } else { "Leave collaboration" }, &ask_text, if owner { "Stop" } else { "Leave" }, move |shared| {
        if owner { let c2 = c.clone(); shared.borrow().rt.spawn(async move { collab::stop(&c2).await; }); }
        xpl_set(shared, &id, |x| { if let Some(o) = x.as_object_mut() { o.remove("collab"); } });
        xpl_changed(shared);
        share_status(shared, if owner { format!("\"{title}\" isn't collaborative any more.") } else { format!("You left \"{title}\". It's kept as yours.") });
    });
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
            let text = format!("Join \"{title}\"{by}? It's a shared playlist. You can add and remove songs, and see everyone else's changes.");
            ask(&shared, "Join playlist", &text, "Join", move |shared| {
                let tracks: Vec<Track> = share["tracks"].as_array().into_iter().flatten().map(collab::with_who(&share)).filter_map(|s| Track::from_snapshot(&s)).filter(|t| t.plat() != "local").collect();
                let id = xpl::create(&mut shared.borrow_mut().xpl, &title, &tracks);
                xpl_set(shared, &id, |x| x["collab"] = serde_json::json!({ "code": code, "relay": relay, "k": e, "owner": false, "rev": share["rev"].as_u64().unwrap_or(0) }));
                xpl_changed(shared);
                go(shared, Place::Playlist(id));
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Joined \"{title}\". Your changes go to everyone in it, and theirs come to you.").into()); }
            });
        }));
    });
}

fn name_ask(shared: &Shared, naming: Naming) {
    let (head, ok, text) = match &naming {
        Naming::Rename(id) => ("Rename playlist", "Save", shared.borrow().playlists.iter().find(|p| &p.id == id).map(|p| p.title.clone()).unwrap_or_default()),
        Naming::AppName => ("The app's name", "Save", shared.borrow().web.prefs["appName"].as_str().filter(|n| !n.trim().is_empty()).unwrap_or("OmniPulse").to_string()),
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
        Naming::AppName => set_look_pref(shared, "appName", serde_json::json!(if name == "OmniPulse" { String::new() } else { name })),
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
    // (a collaborative playlist: added by you, as everyone sees it)
    let collab = shared.borrow().xpl.as_array().is_some_and(|l| l.iter().any(|x| x["id"].as_str() == Some(id.as_str()) && x["collab"].is_object()));
    let by = my_name(&shared.borrow());
    let t = if collab { Track { added_by: by.clone(), ..t } } else { t };
    let n = xpl::add(&mut shared.borrow_mut().xpl, &id, std::slice::from_ref(&t));
    if n > 0 && t.plat() != "local" { collab_send(shared, &id, collab::add_op(&t, &by)); }
    xpl_changed(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        ui.set_status(if n > 0 { format!("Added \"{}\" to {title}.", t.title) } else { format!("\"{}\" is already in {title}.", t.title) }.into());
    }
}

// ---------- drag and drop (the web app's js/dnd.js) ----------
// What's dragged: a track of the page ("t:<row>"; one of several picked: all of them), or a playlist
// of the sidebar ("p:<side>"). What takes it: a playlist of yours in the sidebar or the open one
// ("side:<i>", "page") takes tracks, and another playlist's songs (combined, as pasting them in:
// what's there already left out); a platform playlist takes one track (adopted, as adding to it from
// the menu does); the queue takes tracks.
#[derive(Clone)]
enum Dragged { Tracks(Vec<Track>), List(usize) }

fn dragged(a: &App, data: &slint::DataTransfer) -> Option<Dragged> {
    let text = data.plain_text().ok()?;
    // the playing song (the player bar's), or a queue row
    if text == "now" { return a.queue.current().cloned().map(|t| Dragged::Tracks(vec![t])); }
    if let Some(i) = text.strip_prefix("q:").and_then(|x| x.parse::<usize>().ok()) {
        let at = *a.panel_map.get(i)?;
        return a.queue.items.get(at).map(|q| Dragged::Tracks(vec![q.t.clone()]));
    }
    if let Some(i) = text.strip_prefix("t:").and_then(|x| x.parse::<usize>().ok()) {
        let t = a.page_tracks.get(i)?;
        let many: Vec<Track> = if a.picked.contains(&t.key) && a.picked.len() > 1 { a.page_tracks.iter().filter(|x| a.picked.contains(&x.key)).cloned().collect() } else { vec![t.clone()] };
        return Some(Dragged::Tracks(many));
    }
    let i = text.strip_prefix("p:")?.parse::<usize>().ok()?;
    a.side_map.get(i).copied().map(Dragged::List)
}

/// The playlist a drop target is (its place in a.playlists)
fn drop_list(a: &App, target: &str) -> Option<usize> {
    if target == "page" {
        return match &a.here { Some(Place::Playlist(id)) => a.playlists.iter().position(|p| &p.id == id), _ => None };
    }
    let i = target.strip_prefix("side:")?.parse::<usize>().ok()?;
    a.side_map.get(i).copied()
}

fn drop_ok(a: &App, target: &str, what: &Dragged) -> bool {
    if target == "queue" { return matches!(what, Dragged::Tracks(_)); }
    // (before a queue row: one of what's next, not before what's playing)
    if let Some(j) = target.strip_prefix("qrow:").and_then(|x| x.parse::<usize>().ok()) {
        let at = a.panel_map.get(j).copied();
        return matches!(what, Dragged::Tracks(_)) && listening().is_none() && at.is_some_and(|at| a.queue.qi.is_none_or(|qi| at > qi));
    }
    let Some(at) = drop_list(a, target) else { return false };
    let p = &a.playlists[at];
    match what {
        // (onto one that isn't yours, a platform's: it's made a playlist on this PC first, theirs left as it
        // is; Export puts it on the platforms after, if wanted)
        Dragged::Tracks(_) => xpl::is_own(&p.id) || p.plat != "local",
        Dragged::List(from) => *from != at && (xpl::is_own(&p.id) || p.plat != "local"),
    }
}

/// A drop on a queue row: a queue row moved there, or tracks put in before it
fn drop_queue_row(shared: &Shared, j: usize, from: Option<usize>, what: Dragged) {
    let Dragged::Tracks(list) = what else { return };
    let mut a = shared.borrow_mut();
    let Some(&to) = a.panel_map.get(j) else { return };
    match from.and_then(|i| a.panel_map.get(i).copied()) {
        Some(at) if at != to => {
            // (it joins the part it's dropped in, as the web app's: Autoplay's picks, what you queued, or
            // the rest of the list; a chat request stays one)
            let (into_auto, into_mine) = a.queue.items.get(to).map(|x| (x.auto, x.queued || !x.by.is_empty())).unwrap_or((false, false));
            let mut item = a.queue.items.remove(at);
            item.auto = into_auto;
            if !into_auto && item.by.is_empty() { item.queued = into_mine; }
            let to = if at < to { to - 1 } else { to };
            a.queue.items.insert(to, item);
            if let Some(qi) = a.queue.qi { if at < qi && to >= qi { a.queue.qi = Some(qi - 1); } else if at > qi && to <= qi { a.queue.qi = Some(qi + 1); } }
        }
        Some(_) => return,
        None => { for (k, t) in list.into_iter().enumerate() { a.queue.items.insert(to + k, queue::QItem::queued(t)); } }
    }
    paint_queue(&mut a);
    save_session(&a);
}

fn drop_do(shared: &Shared, target: &str, what: Dragged) {
    if target == "queue" {
        if let Dragged::Tracks(list) = what {
            let n = list.len();
            // (in a Jam: sent to it, as Add to Queue does)
            if let Some(kind) = listening() {
                if kind == "jam" { for t in &list { listen_send(serde_json::json!({ "type": "add", "track": t.to_snapshot(), "next": false })); } }
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if kind == "jam" { format!("Sent {} to the Jam.", plural(n, "track", "tracks")) } else { "You're listening along. Only a Jam takes songs from listeners.".to_string() }.into()); }
                return;
            }
            let empty = {
                let mut a = shared.borrow_mut();
                let empty = a.queue.current().is_none();
                for t in list { a.queue.add(t); }
                paint_queue(&mut a);
                save_session(&a);
                empty
            };
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if n == 1 { "Added to the queue.".to_string() } else { format!("Added {n} tracks to the queue.") }.into()); }
            if empty { start(shared); }
        }
        return;
    }
    let Some(at) = drop_list(&shared.borrow(), target) else { return };
    let into = shared.borrow().playlists[at].clone();
    match what {
        // (one track onto a platform playlist: as adding it from the menu; more, or a playlist: it's adopted
        // first, a playlist here kept in step with the platform's, as the web app does)
        Dragged::Tracks(list) if !xpl::is_own(&into.id) && into.own && list.len() == 1 => { if let Some(t) = list.into_iter().next() { pl_add_platform(shared, into, t); } }
        Dragged::Tracks(list) if !xpl::is_own(&into.id) => pl_adopt_add(shared, into, None, list),
        Dragged::Tracks(list) => pl_add_many(shared, &into.id, list),
        Dragged::List(from) if !xpl::is_own(&into.id) => {
            let src = shared.borrow().playlists.get(from).cloned();
            if let Some(src) = src { pl_adopt_add(shared, into, Some(src), Vec::new()); }
        }
        Dragged::List(from) => {
            let Some(src) = shared.borrow().playlists.get(from).cloned() else { return };
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Adding \"{}\" to \"{}\"...", src.title, into.title).into()); }
            let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
            let id = into.id.clone();
            rt.spawn(async move {
                let list = sources::playlist_tracks(&src).await;
                let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                    let Some(shared) = s.borrow().clone() else { return };
                    if list.is_empty() { ui.set_status(format!("\"{}\" is empty.", src.title).into()); return; }
                    MERGING.with(|m| *m.borrow_mut() = Some((id.clone(), src.title.clone(), src.id.clone())));
                    pl_add_many(&shared, &id, list);
                }));
            });
        }
    }
}

/// A platform playlist of yours adopted (a playlist here, kept in step with the platform's), then
/// tracks into it: these, or another playlist's
fn pl_adopt_add(shared: &Shared, into: sources::Playlist, src: Option<sources::Playlist>, list: Vec<Track>) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    if let Some(ui) = weak.upgrade() { ui.set_status(format!("Adding to \"{}\"...", into.title).into()); }
    rt.spawn(async move {
        let list = match &src { Some(p) => sources::playlist_tracks(p).await, None => list };
        if list.is_empty() {
            let name = src.map(|p| p.title).unwrap_or_default();
            let _ = weak.upgrade_in_event_loop(move |ui| ui.set_status(format!("\"{name}\" is empty.").into()));
            return;
        }
        let copy = mirror::adopt(&into).await;
        let _ = weak.upgrade_in_event_loop(move |_ui| SHARED.with(|s| {
            let Some(shared) = s.borrow().clone() else { return };
            let id = copy["id"].as_str().unwrap_or("").to_string();
            {
                let mut a = shared.borrow_mut();
                if let Some(l) = a.xpl.as_array_mut() { l.insert(0, copy); } else { a.xpl = serde_json::json!([copy]); }
            }
            hidden_set(&shared, &mirror::side_id(into.plat, &into.id), true);
            xpl_changed(&shared);
            let showing = matches!(&shared.borrow().here, Some(Place::Playlist(x)) if *x == into.id);
            if let Some(p) = &src { MERGING.with(|m| *m.borrow_mut() = Some((id.clone(), p.title.clone(), p.id.clone()))); }
            pl_add_many(&shared, &id, list);
            if showing { go(&shared, Place::Playlist(id)); }
        }));
    });
}

thread_local! {
    /// a playlist being combined into another: (into, from's title, from's id), for its record
    static MERGING: RefCell<Option<(String, String, String)>> = const { RefCell::new(None) };
}

/// Unmerge Playlists: what each playlist combined into this one brought taken out again (what was in
/// it before stays); one that's gone from the sidebar is made again, on this PC
fn pl_unmerge(shared: &Shared, id: &str) {
    let Some(p) = shared.borrow().xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(id)).cloned()) else { return };
    let merged = p["merged"].as_array().cloned().unwrap_or_default();
    if merged.is_empty() { return; }
    let mut remade = 0;
    for m in &merged {
        for k in m["keys"].as_array().into_iter().flatten().filter_map(|k| k.as_str()) { xpl::remove(&mut shared.borrow_mut().xpl, id, k); }
        let from = m["from"].as_str().unwrap_or("");
        let there = shared.borrow().playlists.iter().any(|x| x.id == from);
        if !there {
            let tracks: Vec<Track> = m["tracks"].as_array().into_iter().flatten().filter_map(Track::from_snapshot).collect();
            if !tracks.is_empty() { xpl::create(&mut shared.borrow_mut().xpl, m["title"].as_str().unwrap_or("Playlist"), &tracks); remade += 1; }
        }
    }
    xpl_set(shared, id, |x| { if let Some(o) = x.as_object_mut() { o.remove("merged"); } });
    xpl_changed(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        let names: Vec<String> = merged.iter().filter_map(|m| m["title"].as_str().map(|t| format!("\u{201c}{t}\u{201d}"))).collect();
        ui.set_status(format!("Unmerged {} from \u{201c}{}\u{201d}.{}", names.join(" and "), p["title"].as_str().unwrap_or(""), if remade > 0 { format!(" {} made again on this PC.", plural(remade, "playlist was", "playlists were")) } else { String::new() }).into());
    }
}

thread_local! {
    /// Look-alikes being asked about: the playlist, what's added as it is, and each one to decide
    static LA: RefCell<Option<(String, Vec<Track>, Vec<lookalike::Conflict>)>> = const { RefCell::new(None) };
}

fn la_paint(ui: &AppWindow) {
    LA.with(|l| {
        let Some((_, _, c)) = l.borrow().clone() else { return };
        ui.set_la_head(if c.len() == 1 { "This might already be here".into() } else { format!("{} of these might already be here", c.len()).into() });
        ui.set_la_rows(ModelRc::from(Rc::new(VecModel::from(c.iter().map(|x| LookAlike { new: lookalike::line(&x.new).into(), old: lookalike::line(&x.twin).into(), choice: x.choice.label().into() }).collect::<Vec<_>>()))));
    });
}

/// Several songs added to a playlist here: exactly the same ones left out; look-alikes asked about first
fn pl_add_many(shared: &Shared, id: &str, list: Vec<Track>) {
    let have: Vec<Track> = shared.borrow().playlists.iter().find(|p| p.id == id).and_then(|p| p.tracks.clone()).unwrap_or_default();
    let (add, ask) = lookalike::plan(&list, &have);
    if ask.is_empty() || std::env::var_os("OMNIPULSE_SELFTEST").is_some() { return pl_add_now(shared, id, list, Vec::new()); }
    LA.with(|l| *l.borrow_mut() = Some((id.to_string(), add, ask)));
    if let Some(ui) = shared.borrow().ui.upgrade() { la_paint(&ui); ui.set_la_open(true); }
}

/// The look-alikes answered (Add), or not (Cancel: nothing added)
fn la_done(shared: &Shared, ok: bool) {
    let Some((id, add, ask)) = LA.with(|l| l.borrow_mut().take()) else { return };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_la_open(false); }
    if !ok { MERGING.with(|m| m.borrow_mut().take()); return; }
    let (add, replace) = lookalike::decide(add, &ask);
    pl_add_now(shared, &id, add, replace);
}

/// Tracks into a playlist of yours (what's there already left out), said; shown again if it's open
fn pl_add_now(shared: &Shared, id: &str, list: Vec<Track>, replace: Vec<(String, Track)>) {
    // (a look-alike swapped for the new one: the one there out, the new one in)
    for (old, _) in &replace { xpl::remove(&mut shared.borrow_mut().xpl, id, old); }
    let list: Vec<Track> = list.into_iter().chain(replace.into_iter().map(|r| r.1)).collect();
    let title = shared.borrow().playlists.iter().find(|p| p.id == id).map(|p| p.title.clone()).unwrap_or_default();
    let collab = shared.borrow().xpl.as_array().is_some_and(|l| l.iter().any(|x| x["id"].as_str() == Some(id) && x["collab"].is_object()));
    let by = my_name(&shared.borrow());
    let list: Vec<Track> = if collab { list.into_iter().map(|t| Track { added_by: by.clone(), ..t }).collect() } else { list };
    let had: std::collections::HashSet<String> = shared.borrow().playlists.iter().find(|p| p.id == id).and_then(|p| p.tracks.as_ref()).map(|t| t.iter().map(|x| x.key.clone()).collect()).unwrap_or_default();
    let n = xpl::add(&mut shared.borrow_mut().xpl, id, &list);
    // (a playlist combined into this one: what it brought, kept, for Unmerge Playlists)
    if let Some((into, title, from)) = MERGING.with(|m| m.borrow_mut().take()) {
        if into == id {
            let added: Vec<&Track> = list.iter().filter(|t| !had.contains(&t.key)).collect();
            let rec = serde_json::json!({ "title": title, "from": from, "keys": added.iter().map(|t| t.key.clone()).collect::<Vec<_>>(), "tracks": added.iter().map(|t| xpl::snap(t)).collect::<Vec<_>>() });
            xpl_set(shared, id, |x| { if !x["merged"].is_array() { x["merged"] = serde_json::json!([]); } x["merged"].as_array_mut().unwrap().push(rec); });
        }
    }
    for t in list.iter().filter(|t| t.plat() != "local" && !had.contains(&t.key)) { collab_send(shared, id, collab::add_op(t, &by)); }
    xpl_changed(shared);
    let left = list.len().saturating_sub(n);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        let there = if left > 0 { format!(" ({} there already)", plural(left, "was", "were")) } else { String::new() };
        ui.set_status(if n > 0 { format!("Added {} to {title}{there}.", plural(n, "track", "tracks")) } else { format!("Nothing new for {title}{there}.") }.into());
    }
    let showing = matches!(&shared.borrow().here, Some(Place::Playlist(x)) if x == id);
    if showing && n > 0 { go(shared, Place::Playlist(id.to_string())); }
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

/// A sidebar playlist (its place there) moved before another's place (the end: after them all): the order as
/// shown kept as yours. A pinned one stays among the pinned (and an unpinned one among the rest), its pin
/// order moved instead if it's dropped among them
fn side_move(shared: &Shared, from: usize, to: usize) {
    let (mut ids, pinned) = {
        let a = shared.borrow();
        let ids: Vec<String> = a.side_map.iter().map(|&i| mirror::side_id(a.playlists[i].plat, &a.playlists[i].id)).collect();
        let pinned: Vec<String> = a.web.prefs["sidePinned"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
        (ids, pinned)
    };
    if from >= ids.len() || to > ids.len() { return; }
    let id = ids.remove(from);
    let to = if to > from { to - 1 } else { to };
    ids.insert(to, id);
    // (the pinned ones' order: as they now stand)
    let pinned: Vec<String> = ids.iter().filter(|x| pinned.contains(x)).cloned().collect();
    pref_put(shared, "sidePinned", serde_json::json!(pinned));
    pref_put(shared, "sideOrder", serde_json::json!(ids));
    paint_sides(&mut shared.borrow_mut());
}

fn side_action(shared: &Shared, i: usize, act: &str) {
    let Some(p) = shared.borrow().side_map.get(i).and_then(|&at| shared.borrow().playlists.get(at).cloned()) else { return };
    // (pinned to the top, or not: the last pinned, below those pinned before it)
    if act == "pin" {
        let id = mirror::side_id(p.plat, &p.id);
        let mut pinned: Vec<String> = shared.borrow().web.prefs["sidePinned"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
        let now = if let Some(k) = pinned.iter().position(|x| *x == id) { pinned.remove(k); false } else { pinned.push(id); true };
        pref_put(shared, "sidePinned", serde_json::json!(pinned));
        paint_sides(&mut shared.borrow_mut());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if now { format!("{} pinned to the top.", p.title) } else { format!("{} unpinned.", p.title) }.into()); }
        return;
    }
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
    // a platform playlist made one on this PC (all its songs, from any platform, kept in step on the
    // platforms), the original hidden from the sidebar, as the web app's Switch to a playlist on this PC
    if act == "switch" {
        let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
        if let Some(ui) = weak.upgrade() { ui.set_status(format!("Copying \"{}\"...", p.title).into()); }
        rt.spawn(async move {
            let copy = mirror::adopt(&p).await;
            let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                let Some(shared) = s.borrow().clone() else { return };
                let id = copy["id"].as_str().unwrap_or("").to_string();
                { let mut a = shared.borrow_mut(); if let Some(l) = a.xpl.as_array_mut() { l.insert(0, copy); } else { a.xpl = serde_json::json!([copy]); } }
                hidden_set(&shared, &mirror::side_id(p.plat, &p.id), true);
                xpl_changed(&shared);
                go(&shared, Place::Playlist(id));
                ui.set_status(format!("\"{}\" is a playlist on this PC now, kept on the platforms too.", p.title).into());
            }));
        });
        return;
    }
    // Export to SoundCloud & Spotify (and again); Unmerge Playlists
    if act == "export" { return pl_export_platforms(shared, &p.id); }
    if act == "unmerge" { return pl_unmerge(shared, &p.id); }
    // hidden from the sidebar (kept: shown again from Settings > Library > Hidden & Not Recommended)
    if act == "hide" {
        let mut ids: Vec<serde_json::Value> = shared.borrow().web.prefs["hiddenByYou"].as_array().cloned().unwrap_or_default();
        if !ids.iter().any(|x| x.as_str() == Some(p.id.as_str())) { ids.push(serde_json::json!(p.id)); }
        let mut names = shared.borrow().web.prefs["hiddenByYouNames"].as_object().cloned().unwrap_or_default();
        names.insert(p.id.clone(), serde_json::json!(p.title));
        if !hide::on(&shared.borrow().web.prefs) { pref_put(shared, "hideOn", serde_json::json!(true)); }
        pref_put(shared, "hiddenByYou", serde_json::json!(ids));
        pref_put(shared, "hiddenByYouNames", serde_json::Value::Object(names));
        paint_sides(&mut shared.borrow_mut());
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("\u{201c}{}\u{201d} is hidden. Show it again from Settings \u{203a} Library \u{203a} Hidden & Not Recommended.", p.title).into()); }
        return;
    }
    if !xpl::is_own(&p.id) {
        return;
    }
    if act == "rename" {
        return name_ask(shared, Naming::Rename(p.id));
    }
    let text = format!("Delete \"{}\" from this PC?", p.title);
    ask(shared, "Delete playlist", &text, "Delete", move |shared| pl_delete(shared, p));
}

/// A playlist on this PC deleted (asked first), and its copies on the platforms
fn pl_delete(shared: &Shared, p: sources::Playlist) {
    let gone = shared.borrow().xpl.as_array().and_then(|l| l.iter().find(|x| x["id"].as_str() == Some(p.id.as_str())).cloned()).unwrap_or_default();
    for plat in mirror::PLATS {
        if gone["made"][plat].as_bool() == Some(true) {
            if let Some(mid) = gone["mirrors"][plat].as_str().map(String::from) {
                // (left hidden: a platform can still list one for a while after it's deleted; one that
                // couldn't be deleted is tried again on the next start)
                hidden_set(shared, &mirror::side_id(plat, &mid), true);
                if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { shared.borrow().rt.spawn(async move { mirror::remove_or_later(plat, &mid).await; }); }
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

// ---------- asked first: in the app's own look (not Windows' message box) ----------
thread_local! {
    /// what an OK does, while it's asked
    static ASKED: RefCell<Option<Box<dyn FnOnce(&Shared)>>> = const { RefCell::new(None) };
}

/// Asks (a title, what it'll do, the OK button's word); OK does `then`, Cancel nothing. The self-test
/// says yes at once.
fn ask(shared: &Shared, title: &str, text: &str, ok: &str, then: impl FnOnce(&Shared) + 'static) {
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() { return then(shared); }
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    ASKED.with(|a| *a.borrow_mut() = Some(Box::new(then)));
    ui.set_ask_title(title.into());
    ui.set_ask_text(text.into());
    ui.set_ask_ok(ok.into());
    ui.set_ask_open(true);
}

fn ask_answer_ui(shared: &Shared, ok: bool) {
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_ask_open(false); ui.set_ask_input_on(false); ui.set_ask_input_secret(false); }
    let then = ASKED.with(|a| a.borrow_mut().take());
    if ok { if let Some(f) = then { f(shared); } }
}

// ---------- Import from Streamlabs (slimport) ----------

/// The Alert Box link asked for: filled in from the clipboard if it holds one, hidden as it's typed (it's a secret)
fn sl_import_ask(shared: &Shared) {
    let clip = arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default();
    let now = if slimport::token(&clip).is_some() { clip.trim().to_string() } else { String::new() };
    ask_input(shared, "Import from Streamlabs", "On streamlabs.com, open Alert Box, click Click to show widget URL, then click Copy, and paste it here. It's kept secret: only Streamlabs sees it. Your alerts' pictures, sounds and messages are replaced with your Streamlabs ones.", "Import", &now, "Paste your Alert Box link here", Vec::new(), |s, link| sl_import(s, link));
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_ask_input_secret(true); }
}

/// The Alert Box's settings read, and its pictures and sounds fetched (into a folder of their own first)
fn sl_import(shared: &Shared, link: String) {
    let say = |t: &str| if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(t.into()); };
    let Some(token) = slimport::token(&link) else { return say("That isn't an Alert Box link. It looks like streamlabs.com/alert-box/v3/ and then letters and numbers.") };
    say("Importing your Streamlabs alerts...");
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        let got: Result<(Vec<slimport::One>, Vec<(&'static str, bool, std::path::PathBuf)>, usize), String> = async {
            let c = reqwest::Client::builder().user_agent("Mozilla/5.0 OmniPulse").timeout(std::time::Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
            let r = c.get(slimport::config_url(&token)).header("referer", format!("https://streamlabs.com/alert-box/v3/{token}")).send().await.map_err(|_| "Couldn't reach Streamlabs.".to_string())?;
            if !r.status().is_success() { return Err("Streamlabs didn't take that link. Copy it again from Alert Box.".into()); }
            let cfg: serde_json::Value = r.json().await.map_err(|_| "Streamlabs didn't take that link. Copy it again from Alert Box.".to_string())?;
            let list = slimport::read(&cfg["settings"]);
            if list.is_empty() { return Err("That Alert Box has no alerts to import.".into()); }
            let dir = std::env::temp_dir().join("omnipulse-streamlabs");
            let _ = std::fs::create_dir_all(&dir);
            let (mut files, mut skipped) = (Vec::new(), 0usize);
            for o in &list {
                for (image, url) in [(true, &o.image), (false, &o.sound)] {
                    let Some(url) = url else { continue };
                    // (a video as its picture: not one alerts take)
                    let Some(ext) = slimport::ext_of(url, image) else { skipped += 1; continue };
                    let Ok(r) = c.get(url).send().await else { skipped += 1; continue };
                    let Ok(bytes) = r.bytes().await else { skipped += 1; continue };
                    if bytes.len() > 20 * 1024 * 1024 { skipped += 1; continue; }
                    let path = dir.join(format!("{}-{}.{ext}", o.kind, if image { "picture" } else { "sound" }));
                    if std::fs::write(&path, &bytes).is_ok() { files.push((o.kind, image, path)); }
                }
            }
            Ok((list, files, skipped))
        }.await;
        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            match got {
                Ok((list, files, skipped)) => sl_import_apply(&sh, list, files, skipped),
                Err(e) => if let Some(ui) = sh.borrow().ui.upgrade() { ui.set_status(e.into()); },
            }
        }));
    });
}

/// What was imported, put in: each picture and sound as if chosen, each message, time and colours as if typed
fn sl_import_apply(shared: &Shared, list: Vec<slimport::One>, files: Vec<(&'static str, bool, std::path::PathBuf)>, skipped: usize) {
    let (mut pics, mut sounds) = (0, 0);
    for (kind, image, path) in &files {
        let Ok(file) = omnipulse_core::widget::alert_media_set(kind, if *image { "image" } else { "sound" }, &path.display().to_string()) else { continue };
        let key = if *image { "alertImages" } else { "alertSounds" };
        let mut all = shared.borrow().web.prefs[key].as_object().cloned().unwrap_or_default();
        all.insert(kind.to_string(), serde_json::json!({ "name": file, "at": now_ms() }));
        pref_put(shared, key, serde_json::Value::Object(all));
        if *image { pics += 1; } else {
            sounds += 1;
            // (measured, so it plays about as loud as the others)
            if let Ok(mut m) = omnipulse_core::loudness::alert_levels(kind) {
                let mut lv = shared.borrow().web.prefs["alertLevels"].as_object().cloned().unwrap_or_default();
                m["adj"] = serde_json::json!(0);
                lv.insert(kind.to_string(), m);
                pref_put(shared, "alertLevels", serde_json::Value::Object(lv));
            }
        }
        let _ = std::fs::remove_file(path);
    }
    // (its words, how long, its colours; a new picture uncropped; on or off as it was there)
    let mut edits = shared.borrow().web.prefs["alertEdit"].as_object().cloned().unwrap_or_default();
    let mut offs: Vec<serde_json::Value> = shared.borrow().web.prefs["alertOff"].as_array().cloned().unwrap_or_default();
    for o in &list {
        let mut e = edits.get(o.kind).and_then(|v| v.as_object()).cloned().unwrap_or_default();
        if let Some(m) = &o.main { e.insert("main".into(), serde_json::json!(m)); }
        if let Some(s) = o.secs { e.insert("secs".into(), serde_json::json!(s)); }
        if let Some((a, b)) = &o.colours { e.insert("color".into(), serde_json::json!(a)); e.insert("color2".into(), serde_json::json!(b)); }
        if files.iter().any(|f| f.0 == o.kind && f.1) { e.remove("crop"); }
        edits.insert(o.kind.into(), serde_json::Value::Object(e));
        offs.retain(|x| x != o.kind);
        if !o.on { offs.push(serde_json::json!(o.kind)); }
    }
    pref_put(shared, "alertEdit", serde_json::Value::Object(edits));
    pref_put(shared, "alertOff", serde_json::Value::Array(offs));
    push_out(shared, true);
    let names: Vec<&str> = list.iter().filter_map(|o| alerts::KINDS.iter().find(|k| k.0 == o.kind).map(|k| k.1)).collect();
    let note = format!("Imported from Streamlabs: {} ({} {}, {} {}){}.", names.join(", "), pics, if pics == 1 { "picture" } else { "pictures" }, sounds, if sounds == 1 { "sound" } else { "sounds" },
        if skipped > 0 { format!(". {skipped} couldn't come across (not a kind alerts take, or over 20 MB): choose those in Alerts and Their Order") } else { String::new() });
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
    pf_paint(shared);
}

/// Asks for a name (a box to type it in, filled with `now`, and ones there are to pick from); OK gives `then`
/// what's typed. The self-test takes what's filled in.
fn ask_input(shared: &Shared, title: &str, text: &str, ok: &str, now: &str, hint: &str, options: Vec<String>, then: impl FnOnce(&Shared, String) + 'static) {
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() { return then(shared, now.to_string()); }
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    ui.set_ask_input(now.into());
    ui.set_ask_input_hint(hint.into());
    ui.set_ask_input_options(ModelRc::from(Rc::new(VecModel::from(options.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()))));
    ui.set_ask_input_on(true);
    ask(shared, title, text, ok, move |s| {
        let typed = s.borrow().ui.upgrade().map(|u| u.get_ask_input().to_string()).unwrap_or_default();
        then(s, typed);
    });
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
                        Ok(()) => format!("Link copied. Anyone can see its songs on every platform and open it in OmniPulse.{}", if same { " (The same link as before, now up to date.)" } else { "" }),
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
    /// asking to join your Jam: each one waiting (id, name, their app's id), let in or declined from the
    /// Jam's window, as the web app's (no time limit)
    static JOINS: RefCell<Vec<(String, String, String)>> = const { RefCell::new(Vec::new()) };
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
                    host_keep(kind, &relay, Some(&made));
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

/// The room closed (the listeners told it's over, when `tell`: ended, so not picked up again)
fn room_end(tell: bool) {
    ROOM.with(|r| {
        if let Some(room) = r.borrow_mut().take() {
            if tell { let _ = room.tx.send(serde_json::json!({ "type": "end" }).to_string()); }
        }
    });
    if tell { host_keep("", "", None); }
    MEMBERS.with(|m| m.borrow_mut().clear());
    JOINS.with(|j| j.borrow_mut().clear());
}

/// One waiting to join your Jam: let in (they connect again by themselves) or declined
fn join_answer(shared: &Shared, i: usize, ok: bool) {
    let Some((id, name, _)) = JOINS.with(|j| { let mut j = j.borrow_mut(); (i < j.len()).then(|| j.remove(i)) }) else { return };
    ask_answer(&id, true, ok, false);
    jam_paint(shared);
    if ok { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{name} is joining your Jam.").into()); } }
}

// ---------- hosting through a restart (as the web app's) ----------
// What you're hosting is kept (state/native-hosting.json, and when last), so the app closing mid-session
// (an update) and opening again within a minute and a half picks it up where it was: everyone was told
// you'd be back (the relay waits that long)
const HOST_RESUME_SECS: u64 = 90;

fn hosting_file() -> Option<std::path::PathBuf> {
    omnipulse_core::data_dir().ok().map(|d| d.join("state").join("native-hosting.json"))
}

/// Kept (made: the room; None: forgotten), or (made Some, kind "") just its time refreshed
fn host_keep(kind: &str, relay: &str, made: Option<&serde_json::Value>) {
    let Some(p) = hosting_file() else { return };
    match made {
        None => { let _ = std::fs::remove_file(p); }
        Some(m) => {
            if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
            let v = serde_json::json!({ "kind": kind, "relay": relay, "made": m, "at": omnipulse_core::now_secs() });
            let _ = std::fs::write(p, v.to_string());
        }
    }
}

thread_local! {
    static HOST_FRESH_AT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// Still hosting: its time kept fresh (as states are pushed: every 30 s at most, well inside HOST_RESUME_SECS)
fn host_fresh() {
    let now = omnipulse_core::now_secs();
    if now.saturating_sub(HOST_FRESH_AT.with(|c| c.get())) < 30 { return; }
    HOST_FRESH_AT.with(|c| c.set(now));
    let Some(p) = hosting_file() else { return };
    let Some(mut v) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return };
    v["at"] = serde_json::json!(omnipulse_core::now_secs());
    let _ = std::fs::write(p, v.to_string());
}

/// How many are in the room, kept with it (so coming back after a restart says they're still there
/// only if anyone was)
fn host_members(n: usize) {
    let Some(p) = hosting_file() else { return };
    let Some(mut v) = std::fs::read_to_string(&p).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return };
    v["members"] = serde_json::json!(n);
    let _ = std::fs::write(p, v.to_string());
}

fn host_resume(shared: &Shared) {
    let Some(v) = hosting_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return };
    let fresh = omnipulse_core::now_secs().saturating_sub(v["at"].as_u64().unwrap_or(0)) <= HOST_RESUME_SECS;
    let kind: &'static str = match v["kind"].as_str() { Some("jam") => "jam", Some("listen") => "listen", _ => "" };
    if !fresh || kind.is_empty() || ROOM.with(|r| r.borrow().is_some()) || listening().is_some() { host_keep("", "", None); return; }
    let members = v["members"].as_u64().unwrap_or(0);
    room_open(shared, kind, Some(v["made"].clone()), move |s| {
        // (said only of a Jam, or of listeners who were there: a Listen Along room with nobody in it
        // opens again quietly)
        let note = match (kind, members > 0) {
            ("jam", true) => Some("Back in your Jam: everyone's still with you.".to_string()),
            ("jam", false) => Some("Your Jam is open again.".to_string()),
            (_, true) => Some(format!("Back in your Listen Along: {} still listening.", if members == 1 { "1 friend is".to_string() } else { format!("{members} friends are") })),
            _ => None,
        };
        if let (Some(n), Some(ui)) = (note, s.borrow().ui.upgrade()) { ui.set_status(n.into()); }
        push_out(s, true);
    });
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
    host_fresh();
}

/// What the room says
fn room_message(shared: &Shared, m: serde_json::Value) {
    match m["type"].as_str() {
        Some("open") => host_push(&shared.borrow()),
        Some("members") => {
            MEMBERS.with(|x| *x.borrow_mut() = m["list"].as_array().into_iter().flatten().filter_map(|n| n.as_str().map(String::from)).collect());
            host_members(MEMBERS.with(|x| x.borrow().len()));
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
            // (into your Jam: on the waiting list, the same person once, at their newest ask)
            if join {
                let cid = m["cid"].as_str().unwrap_or("").to_string();
                JOINS.with(|j| {
                    let mut j = j.borrow_mut();
                    match j.iter_mut().find(|x| (!cid.is_empty() && x.2 == cid) || x.0 == id) { Some(x) => x.0 = id.to_string(), None => j.push((id.to_string(), name.to_string(), cid)) }
                });
                jam_paint(shared);
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{name} wants to join your Jam: let them in from the Jam button.").into()); }
                return;
            }
            if ASK.with(|a| a.borrow().is_some()) || (!join && room_kind().as_deref() == Some("jam")) { ask_answer(id, join, false, true); return; }
            ASK.with(|a| *a.borrow_mut() = Some((id.to_string(), name.to_string(), std::time::Instant::now(), join)));
            jam_paint(shared);
        }
        Some("askGone") if JOINS.with(|j| j.borrow().iter().any(|x| m["id"].as_str() == Some(x.0.as_str()))) => {
            JOINS.with(|j| j.borrow_mut().retain(|x| m["id"].as_str() != Some(x.0.as_str())));
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
        ui.set_queue_room(if kind == "jam" { "In a Jam: you're seeing the host's queue".into() } else { "Listening along: you're seeing the host's queue".into() });
        ui.set_queue_room_host(false);
        return;
    }
    let jam = room_kind().as_deref() == Some("jam");
    ui.set_queue_room(if jam { format!("Your Jam · {} listening", members.len()).into() } else { "".into() });
    ui.set_queue_room_host(jam);
    ui.set_jam_on(jam);
    ui.set_jam_hidden(!jams_on(&shared.borrow()));
    let waiting = JOINS.with(|j| j.borrow().iter().map(|x| slint::SharedString::from(x.1.as_str())).collect::<Vec<_>>());
    let wait = if waiting.is_empty() { String::new() } else { format!(" \u{b7} {} waiting", waiting.len()) };
    ui.set_jam_label(if jam { if members.is_empty() { format!("Jam{wait}") } else { format!("Jam \u{b7} {}{wait}", members.len()) } } else { "Start a Jam".into() }.into());
    ui.set_jam_waiting(ModelRc::from(Rc::new(VecModel::from(waiting))));
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
/// Listening Along: Timing (the web app's listenNudge): this much behind the host (ahead, below 0), to
/// line up with their stream when that's heard too; -3 s to 8 s
static LISTEN_NUDGE: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);

fn nudge_of(_prefs: &serde_json::Value) -> i64 {
    // (no longer a setting: listeners play in step with the host, never put behind or ahead)
    0
}

fn listen_host_pos() -> u64 {
    let nudge = LISTEN_NUDGE.load(std::sync::atomic::Ordering::Relaxed);
    LISTEN.with(|l| {
        let l = l.borrow();
        let Some(st) = l.as_ref().and_then(|l| l.state.as_ref().map(|s| (s, l.at))) else { return 0 };
        let pos = st.0["pos"].as_f64().unwrap_or(0.0).max(0.0) as u64;
        let pos = if st.0["playing"].as_bool() == Some(true) { pos + st.1.elapsed().as_millis() as u64 } else { pos };
        (pos as i64 - nudge).max(0) as u64
    })
}

/// Whether Spotify songs play through Spotify here: signed in for playback with Premium (a free account,
/// signed in for its library, plays them from YouTube Music)
fn sp_plays_here() -> bool {
    omnipulse_core::spotify_engine::spe_signed_in() && omnipulse_core::spotify_engine::spe_premium() != Some(false)
}

/// Whether this app can play the host's track as it is (else its YouTube Music version)
fn listen_needs_version(t: &Track) -> bool {
    if listening().is_none() {
        return false;
    }
    let ytdlp = omnipulse_core::tools::ytdlp_path().is_some();
    let ok = match &t.src {
        track::Src::SoundCloud(_) => omnipulse_core::soundcloud::signed_in() || ytdlp,
        track::Src::Spotify(_) => omnipulse_core::spotify::spotify_signed_in() && sp_plays_here(),
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

/// How a track goes out into the next one: as Fade Between Tracks is set
fn fade_out_ms(a: &App) -> Option<u64> {
    fade_now(a)
}


/// A skip by hand (the buttons, media keys, the remote): listening, it goes to the Jam's host
/// The visualiser on or off, as Now Playing is: on only while it's shown, playing, a style picked (the
/// engine's tap and its timer stopped otherwise: nothing of it running for nothing)
/// (the self-test's made-up music: the visualiser on for it, without anything playing)
static VIZ_TEST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The Stream Widget's Equaliser design in use (its first output, or another): its levels wanted
fn eq_widget(p: &serde_json::Value) -> bool {
    integrations::set(p, "widget")
        && (p["widgetDesign"].as_str() == Some("equaliser") || integrations::outputs_of(p, "widget").iter().any(|o| o.get("widgetDesign").and_then(|d| d.as_str()) == Some("equaliser")))
}

thread_local! {
    /// (another app full screen, as last looked: looked at once a second at most)
    static FULL_OTHER: std::cell::Cell<(bool, Option<std::time::Instant>)> = const { std::cell::Cell::new((false, None)) };
}

/// A game or anything else full screen, in front (looked at once a second at most)
fn other_full_screen() -> bool {
    let (was, at) = FULL_OTHER.with(|f| f.get());
    if at.is_some_and(|t| t.elapsed() < std::time::Duration::from_secs(1)) { return was; }
    #[cfg(windows)]
    let now = desktop::dock::other_full_screen();
    #[cfg(not(windows))]
    let now = false;
    FULL_OTHER.with(|f| f.set((now, Some(std::time::Instant::now()))));
    now
}

thread_local! {
    /// the GPU scenes' renderer, while one's shown
    static GPU_VIS: RefCell<Option<gpuviz::Gpu>> = const { RefCell::new(None) };
}

thread_local! {
    /// the GPU visualiser in OBS: its own reading of the sound, and when it last read it
    static STREAM_VIS: RefCell<Option<(omnipulse_vis::audio::Analyser, std::time::Instant)>> = const { RefCell::new(None) };
}

/// The cover, for the GPU visualiser in OBS (/vis/art.png): up to 512 pixels, as a PNG, each new one counted
fn vis_art(ui: &AppWindow) {
    let Some(px) = ui.get_now_art_big().to_rgba8() else { return };
    let Some(img) = image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()) else { return };
    let img = if img.width() > 512 || img.height() > 512 { image::imageops::resize(&img, 512, 512, image::imageops::FilterType::Triangle) } else { img };
    let mut png = Vec::new();
    if image::DynamicImage::ImageRgba8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).is_err() { return; }
    if let Ok(mut v) = omnipulse_core::widget::VIS.lock() { v.art = png; v.art_n += 1; }
}

/// The cover, for the GPU scenes that use it: RGBA, top row first
fn gpu_art(ui: &AppWindow) -> Option<(Vec<u8>, u32, u32)> {
    let px = ui.get_now_art_big().to_rgba8()?;
    Some((px.as_bytes().to_vec(), px.width(), px.height()))
}

fn viz_update(shared: &Shared, ui: &AppWindow) {
    let (style, on_now, on_bar, on_widget, on_stream) = {
        let p = &shared.borrow().web.prefs;
        (p["nowViz"].as_str().unwrap_or("off").to_string(), p["vizNow"].as_bool() != Some(false), p["vizBar"].as_bool() == Some(true),
         p["taskbarWidget"].as_bool() == Some(true) && (p["widgetViz"].as_bool() == Some(true) || p["tbDesign"].as_str() == Some("viz")), eq_widget(p) || omnipulse_core::widget::vis_wanted())
    };
    // (a GPU scene: only with Hardware Acceleration on; picked without it, nothing)
    let gpu = shared.borrow().web.prefs["nativeGpu"].as_bool() == Some(true);
    // (Depths and Phosphor were taken out: whoever had one gets Smoke)
    let style = if style == "depths" || style == "phosphor" { "smoke".to_string() } else { style };
    let style = if gpuviz::is_scene(&style) && !gpu { "off".to_string() } else { style };
    let scene = gpuviz::is_scene(&style);
    if ui.get_viz_scene() != scene { ui.set_viz_scene(scene); }
    // (marked WIP: they're still being built)
    if scene { ui.set_viz_scene_name(format!("{} (WIP)", gpuviz::scenes().into_iter().find(|s| s.0 == style).map(|s| s.1).unwrap_or("")).into()); }
    if ui.get_viz_style() != style.as_str() { ui.set_viz_style(style.clone().into()); }
    // (Halo's blurred cover, behind it full screen: made if it isn't yet)
    if style == "halo" && ui.get_now_art_blur().size().width == 0 { art_blur(ui); }
    if ui.get_viz_now() != on_now { ui.set_viz_now(on_now); }
    if ui.get_viz_bar() != on_bar { ui.set_viz_bar(on_bar); }
    // (the GPU visualiser in OBS: whether something's playing, and how far behind viewers hear it (the
    // OmniPulse Offset's delay, when it's on))
    {
        let a = shared.borrow();
        let offset = if a.web.prefs["out2On"].as_bool() == Some(true) { a.web.prefs["out2Offset"].as_f64().unwrap_or(0.0).round().max(0.0) as i64 } else { 0 };
        // (and what it shows, as set on its page: the style, its detail, how often it's drawn, the Offset kept to)
        let set = &a.web.prefs["pluginSettings"][botpage::BOT];
        let pick = |k: &str, d: &str| set[k].as_str().filter(|s| !s.is_empty()).unwrap_or(d).to_string();
        let scene = Some(pick("visScene", "bars")).filter(|s| s != "depths" && s != "phosphor").unwrap_or_else(|| "smoke".into());
        // (a palette or your own two; else your theme's accents, never the album art's)
        let colours = set.as_object().and_then(botpage::vis_colours).unwrap_or_else(|| theme_accents(&a.web.prefs));
        let opts = serde_json::json!({ "scene": scene, "scale": pick("visScale", "1"), "fps": pick("visFps", "60"), "offset": set["visOffset"].as_bool() != Some(false),
            "colours": [colours.0, colours.1], "trail": pick("visTrail", "off") }).to_string();
        if let Ok(mut v) = omnipulse_core::widget::VIS.lock() { v.playing = a.loaded && !a.engine.is_paused(); v.offset = offset; if v.opts != opts { v.opts = opts; } }
    }
    let game_pause = shared.borrow().web.prefs["vizGamePause"].as_bool() != Some(false);
    if ui.get_viz_game_pause() != game_pause { ui.set_viz_game_pause(game_pause); }
    // (shown somewhere: full screen, Now Playing open, or the player's; and playing, in sight; not while a game's
    // full screen, unless that's switched off: the stream's levels go on either way)
    let gaming = game_pause && !ui.get_viz_full() && other_full_screen();
    let want = {
        let a = shared.borrow();
        let shown = ui.get_viz_full() || (on_now && a.page == "now") || on_bar;
        // (the taskbar widget's is in sight with the window minimized or in the tray)
        let in_sight = shown && ui.window().is_visible() && !ui.window().is_minimized();
        // (the taskbar widget's Visualiser design: its bars, whatever the app's style)
        let tb_viz = a.web.prefs["taskbarWidget"].as_bool() == Some(true) && a.web.prefs["tbDesign"].as_str() == Some("viz");
        let test = VIZ_TEST.load(std::sync::atomic::Ordering::Relaxed);
        (((style != "off" && (in_sight || on_widget) && !gaming) || on_stream || (tb_viz && !gaming)) && a.loaded && !a.engine.is_paused()) || (test && style != "off")
    };
    let running = VIZ.with(|v| v.borrow().is_some());
    if want == running { return; }
    let engine = shared.borrow().engine.clone();
    if want {
        engine.set_tap("viz", Some(viz::tap(engine.output_rate())));
        let model = Rc::new(VecModel::from(vec![0.0f32; viz::BANDS]));
        let tops = Rc::new(VecModel::from(vec![0.0f32; viz::BANDS]));
        let wave = Rc::new(VecModel::from(vec![0.0f32; viz::CURVE]));
        ui.set_viz_bands(ModelRc::from(model.clone()));
        ui.set_viz_peaks(ModelRc::from(tops.clone()));
        ui.set_viz_curve(ModelRc::from(wave.clone()));
        let ring = Rc::new(VecModel::from(vec![RingPiece::default(); viz::RING_PIECES]));
        ui.set_viz_ring(ModelRc::from(ring.clone()));
        let small = Rc::new(VecModel::from(vec![RingPiece::default(); viz::RING_PIECES]));
        ui.set_viz_ring_small(ModelRc::from(small.clone()));
        let t = slint::Timer::default();
        let m = model.clone();
        let w = ui.as_weak();
        // (30 a second; 15 in Performance Mode)
        let every = if shared.borrow().web.prefs["perfMode"].as_bool() == Some(true) { 66 } else { 33 };
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(every), move || {
            let b = viz::bands();
            for (i, v) in b.into_iter().enumerate() { m.set_row_data(i, v); }
            for (i, v) in viz::peaks(&b).into_iter().enumerate() { tops.set_row_data(i, v); }
            let cv = viz::curve(&b);
            for (i, v) in cv.into_iter().enumerate() { wave.set_row_data(i, v); }
            let l = viz::level(&b);
            if let Some(ui) = w.upgrade() {
                ui.set_viz_level(l);
                // (the Wave: one smooth line, and the faint area under it)
                if matches!(ui.get_viz_style().as_str(), "wave" | "solid") { let (l, a) = viz::wave_paths(&cv); let g = ui.global::<VizPaths>(); g.set_line(l.into()); g.set_area(a.into()); }
                // (a GPU scene: its next frame asked for, the newest shown; drawn smaller than shown, full screen)
                if ui.get_viz_scene() {
                    let style = ui.get_viz_style().to_string();
                    let (w, h) = if ui.get_viz_full() {
                        let s = ui.window().size();
                        let k = (1280.0 / s.width.max(1) as f32).min(720.0 / s.height.max(1) as f32).min(0.6);
                        ((s.width as f32 * k) as u32, (s.height as f32 * k) as u32)
                    } else { (480, 480) };
                    let (l, r, rate) = viz::stereo(gpuviz::SOUND);
                    let shown = GPU_VIS.with(|g| {
                        let mut g = g.borrow_mut();
                        let gv = g.get_or_insert_with(|| gpuviz::Gpu::start(rate));
                        if gv.new_scene(&style) { gv.set_art(gpu_art(&ui)); }
                        gv.tick(&style, w, h, &l, &r)
                    });
                    if let Some((px, w, h)) = shown { ui.set_viz_gpu(slint::Image::from_rgba8(slint::SharedPixelBuffer::clone_from_slice(&px, w, h))); }
                    // (no GPU for it, or one that wouldn't start: said once)
                    if let Some(e) = GPU_VIS.with(|g| g.borrow().as_ref().and_then(|g| g.failed())) {
                        thread_local! { static SAID: std::cell::Cell<bool> = const { std::cell::Cell::new(false) }; }
                        if !SAID.with(|s| s.replace(true)) { ui.set_status(format!("The GPU scene can't draw: {e}").into()); }
                    }
                }
                // (Halo: its bars, its particles, and its cover's pulse and shake)
                if ui.get_viz_style() == "halo" {
                    let h = viz::halo(&b);
                    let g = ui.global::<VizHalo>();
                    g.set_bars(h.bars.into());
                    g.set_dots(h.dots.into());
                    g.set_scale(h.scale);
                    g.set_dx(h.dx);
                    g.set_dy(h.dy);
                }
                if ui.get_viz_style() == "ring" {
                    for (i, (d, f)) in viz::ring_pieces(&b).into_iter().enumerate() { ring.set_row_data(i, RingPiece { d: d.into(), f }); }
                    // (the player's, round its cover, with In the Player on)
                    if ui.get_viz_bar() { for (i, (d, f)) in viz::ring_small(&b).into_iter().enumerate() { small.set_row_data(i, RingPiece { d: d.into(), f }); } }
                }
            }
            TBW.with(|t| if let Ok(t) = t.try_borrow() { if let Some((tw, _, _)) = t.as_ref() { tw.set_viz_level(l); } });
            // (the Stream Widget's Equaliser: its levels, for /levels.json)
            if on_stream { omnipulse_core::widget::set_levels(b.to_vec()); }
            // (the GPU visualiser in OBS: a frame of the sound, as the scenes read it, when it was heard)
            if omnipulse_core::widget::vis_wanted() {
                let (l, r, rate) = viz::stereo(gpuviz::SOUND);
                let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
                let f = STREAM_VIS.with(|s| {
                    let mut s = s.borrow_mut();
                    let (an, last) = s.get_or_insert_with(|| (omnipulse_vis::audio::Analyser::new(rate), std::time::Instant::now()));
                    let dt = last.elapsed().as_secs_f32();
                    *last = std::time::Instant::now();
                    omnipulse_vis::frame_json(an.analyse(&l, &r, dt), at)
                });
                if let Ok(mut v) = omnipulse_core::widget::VIS.lock() { v.frame = f; }
                if let Some(ui) = w.upgrade() { if omnipulse_core::widget::VIS.lock().map(|v| v.art_n == 0).unwrap_or(false) { vis_art(&ui); } }
            }
        });
        VIZ.with(|v| *v.borrow_mut() = Some((t, model)));
    } else {
        engine.set_tap("viz", None);
        viz::stop();
        STREAM_VIS.with(|s| *s.borrow_mut() = None);
        // (the GPU scene's thread let go, and its device with it)
        GPU_VIS.with(|g| *g.borrow_mut() = None);
        omnipulse_core::widget::set_levels(Vec::new());
        VIZ.with(|v| *v.borrow_mut() = None);
        ui.set_viz_bands(ModelRc::default());
        ui.set_viz_peaks(ModelRc::default());
        ui.set_viz_curve(ModelRc::default());
        ui.set_viz_ring(ModelRc::default());
        ui.set_viz_ring_small(ModelRc::default());
        ui.set_viz_level(0.0);
    }
}

/// A change of song by hand (a skip, a pick): no crossfade's colour pulse carried over to it (it ran
/// on the next cover shown, so a skip during or just after a crossfade flashed it as a transition)
fn no_pulse(shared: &Shared) {
    XFADE.with(|x| *x.borrow_mut() = None);
    XFADE_PULSE.with(|x| x.set(None));
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.global::<Theme>().set_pulsing(false); }
}

fn skip_press(shared: &Shared, d: i32) {
    no_pulse(shared);
    // (skipped before it counted as a play: noted against its artist, for Discover)
    if d > 0 && listening().is_none() {
        let a = shared.borrow();
        // (a song you like isn't held against its artist, as the web app's)
        if let Some(q) = a.queue.qi.and_then(|i| a.queue.items.get(i)).filter(|q| !a.library.iter().any(|x| x.key == q.t.key)) { discover::note_skip(&q.t, a.engine.position_ms(), a.engine.duration_ms()); }
    }
    let Some(kind) = listening() else {
        // (as the web app's: Next on the last track, with autoplay not adding any, goes round to the first)
        let round = { let a = shared.borrow(); d > 0 && a.queue.items.len() > 1 && a.queue.qi.is_some_and(|i| i + 1 >= a.queue.items.len()) && (!a.autoplay || a.repeat != "off") };
        if round { shared.borrow_mut().queue.qi = Some(0); return start(shared); }
        return step(shared, d);
    };
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

// Listening along when the app closed (an update, say) and opened again within 90 s: joined again, and
// said so (state/native-listening.json: the invite and when it was last heard from)
fn listening_file() -> Option<std::path::PathBuf> {
    omnipulse_core::data_dir().ok().map(|d| d.join("state").join("native-listening.json"))
}

thread_local! {
    static LISTEN_LINK: RefCell<String> = const { RefCell::new(String::new()) };
    static LISTEN_FRESH_AT: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
    /// joining again after a restart: the hello says "back", not "joined"
    static REJOINING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn listening_keep(on: bool) {
    let Some(p) = listening_file() else { return };
    if !on { let _ = std::fs::remove_file(p); return; }
    let now = omnipulse_core::now_secs();
    if now.saturating_sub(LISTEN_FRESH_AT.with(|c| c.get())) < 10 { return; }
    LISTEN_FRESH_AT.with(|c| c.set(now));
    let link = LISTEN_LINK.with(|l| l.borrow().clone());
    if link.is_empty() { return; }
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    let _ = std::fs::write(p, serde_json::json!({ "link": link, "at": now }).to_string());
}

fn listen_resume(shared: &Shared) {
    let Some(v) = listening_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return };
    let fresh = omnipulse_core::now_secs().saturating_sub(v["at"].as_u64().unwrap_or(0)) <= HOST_RESUME_SECS;
    let link = v["link"].as_str().unwrap_or("").to_string();
    if !fresh || link.is_empty() || listening().is_some() || ROOM.with(|r| r.borrow().is_some()) { listening_keep(false); return; }
    REJOINING.with(|r| r.set(true));
    listen_join(shared, &link);
}

fn listen_join(shared: &Shared, link: &str) {
    LISTEN_LINK.with(|l| *l.borrow_mut() = link.to_string());
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
            let back = REJOINING.with(|r| r.replace(false));
            LISTEN_FRESH_AT.with(|c| c.set(0));
            listening_keep(true);
            let note = match (kind == "jam", back) {
                (true, true) => format!("Back in {host}'s Jam."),
                (false, true) => format!("Back listening along with {host}."),
                (true, false) => format!("Joined {host}'s Jam. You can add songs and skip them too. Your own queue is set aside until you leave."),
                (false, false) => format!("Listening along with {host}. Your own queue is set aside until you leave."),
            };
            if let Some(ui) = shared.borrow().ui.upgrade() { if back { ui.set_status(note.into()); } else { say_with(&ui, &note, vec![("Leave", StatusAct::Leave)]); } }
        }
        Some("state") => { listening_keep(true); listen_follow(shared, m["state"].clone()) }
        // the host's app closed (an update, say): waited for; back, or gone for good (the relay says)
        Some("hostAway") => {
            let host = LISTEN.with(|l| l.borrow().as_ref().map(|l| l.host.clone())).unwrap_or_default();
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{host}'s app closed: waiting for them to come back...").into()); }
        }
        Some("hostBack") => {
            let host = LISTEN.with(|l| l.borrow().as_ref().map(|l| l.host.clone())).unwrap_or_default();
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{host} is back.").into()); }
        }
        Some("ended") => {
            let host = LISTEN.with(|l| l.borrow().as_ref().map(|l| l.host.clone())).unwrap_or_default();
            listen_leave(shared, true);
            let why = if m["why"].as_str() == Some("hostGone") { "didn't come back, so the session's over" } else { "ended the session" };
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{host} {why}. Back to your own queue.").into()); }
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
            a.resume_key = a.queue.current().map(|c| c.key.clone()).unwrap_or_default();
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
    // (left: not joined again next time)
    listening_keep(false);
    REJOINING.with(|r| r.set(false));
    STAND_IN.with(|x| x.borrow_mut().take());
    drop(l.tx);
    shared.borrow().engine.pause();
    {
        let mut a = shared.borrow_mut();
        a.queue = l.saved;
        a.loaded = false;
        paint_queue(&mut a);
    }
    apply_look(shared);
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
    /// the player bar's "..." lines' actions, in order
    static MORE_ACTS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// YouTube Music's helpers (two small tools it plays with, about 58 MB): how they are, as its settings page
/// says it (no tools named), what its button does ("" none) and whether they can be removed
pub(crate) fn helpers_state() -> (String, &'static str, bool) {
    let on = SHARED.with(|s| s.borrow().as_ref().map(|s| plat_on(&s.borrow().web.prefs, "youtube"))).unwrap_or(true);
    let pr = omnipulse_core::tools::progress();
    let mb = |b: u64| format!("{:.0}", b as f64 / 1_048_576.0);
    let used = omnipulse_core::tools::disk_use();
    let busy = pr["busy"].as_bool() == Some(true);
    let have = omnipulse_core::tools::ytdlp_path().is_some();
    let (note, setup) = if busy {
        let (done, total) = (pr["done"].as_u64().unwrap_or(0), pr["total"].as_u64().unwrap_or(0));
        (format!("Setting up{}...", if total > 0 { format!(": {} of {} MB", mb(done), mb(total)) } else { String::new() }), "")
    } else if pr["error"].as_str().is_some() {
        ("Couldn't set them up. Check your internet connection, then try again.".to_string(), "Try Again")
    } else if have {
        ("Ready. They find each song's audio on YouTube Music from official releases (checked against their checksums), and stay up to date.".to_string(), "")
    } else if !on {
        ("YouTube Music is off.".to_string(), "")
    } else if omnipulse_core::yt_stream::on_path_public("yt-dlp") {
        ("Ready, with the ones already on this PC.".to_string(), "")
    } else {
        ("Two small tools that YouTube Music needs (about 58 MB). They set themselves up.".to_string(), "Set Up")
    };
    let note = format!("{note}{}", if used > 0 && !busy { format!(" They take {} MB.", mb(used)) } else { String::new() });
    (note, setup, used > 0 && !busy)
}

/// YouTube Music's helpers: their state, on its settings page while it's open
fn helpers_paint(shared: &Shared) {
    if PF_EDIT.with(|e| e.borrow().as_ref().is_some_and(|(id, _)| id == "panel:youtube")) { pf_paint(shared); }
}

/// Set up yt-dlp and Deno if YouTube Music is on and they aren't there (as the web app does at start),
/// and let our yt-dlp update itself once a day
fn helpers_start(shared: &Shared) {
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() { return; }
    if plat_on(&shared.borrow().web.prefs, "youtube") && omnipulse_core::tools::ytdlp_path().is_none() && !omnipulse_core::yt_stream::on_path_public("yt-dlp") {
        omnipulse_core::tools::start_install();
    }
    shared.borrow().rt.spawn(async { let _ = omnipulse_core::tools::update_ytdlp_daily().await; });
}

fn helpers_act(shared: &Shared, act: &str) {
    if act == "setup" { omnipulse_core::tools::start_install(); }
    else {
        let note = match omnipulse_core::tools::uninstall() { Ok(b) => format!("YouTube Music's helpers are removed ({:.0} MB freed).", b as f64 / 1_048_576.0), Err(e) => format!("Couldn't remove them: {e}") };
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
    }
    helpers_paint(shared);
}

// ---------- the widgets' pop-out previews: in windows of OmniPulse's own ----------
thread_local! {
    /// each open preview: what it's of, its window, its web view (put in once the window's there), the timer keeping it sized
    static PREVIEWS: RefCell<Vec<(String, WebPreview, Option<wry::WebView>, slint::Timer)>> = const { RefCell::new(Vec::new()) };
}

/// A widget's preview in a window of its own (WebView2: its processes listed under OmniPulse in Task Manager),
/// the window named for it; one already open for it brought back, its page loaded again
fn web_preview(key: &str, title: &str, url: &str, w: u32, h: u32) -> Result<(), String> {
    // (WebView2's own folder: OmniPulse's data folder, not beside the app (Program Files can't be written))
    if let Ok(d) = omnipulse_core::data_dir() { unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", d.join("webview")); } }
    let open = PREVIEWS.with(|p| {
        let mut p = p.borrow_mut();
        p.retain(|(_, win, _, _)| win.window().is_visible());
        if let Some((_, win, view, _)) = p.iter().find(|x| x.0 == key) {
            if let Some(v) = view { let _ = v.load_url(url); }
            let _ = win.show();
            return true;
        }
        false
    });
    if open { return Ok(()); }
    let win = WebPreview::new().map_err(|e| e.to_string())?;
    win.set_name(title.into());
    win.window().set_size(slint::LogicalSize::new(w as f32, h as f32));
    // (a self-test's: off the screen, as its main window is)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() { win.window().set_position(slint::PhysicalPosition::new(-4000, 0)); }
    win.show().map_err(|e| e.to_string())?;
    // (every quarter second: the page put in once the window's there, and kept the window's size)
    let timer = slint::Timer::default();
    let (key2, url2) = (key.to_string(), url.to_string());
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        PREVIEWS.with(|p| {
            let mut p = p.borrow_mut();
            let Some(at) = p.iter().position(|x| x.0 == key2) else { return };
            // (closed: its web view and window let go)
            if !p[at].1.window().is_visible() { p[at].2.take(); p[at].3.stop(); return; }
            let (win, view) = (&p[at].1, &p[at].2);
            let size = win.window().size();
            let rect = wry::Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(size.width, size.height).into() };
            if let Some(v) = view { let _ = v.set_bounds(rect); return; }
            use slint::winit_030::WinitWindowAccessor;
            let made = win.window().with_winit_window(|w| wry::WebViewBuilder::new().with_url(&url2).with_autoplay(true).with_bounds(rect).build_as_child(w).ok()).flatten();
            p[at].2 = made;
        });
    });
    PREVIEWS.with(|p| p.borrow_mut().push((key.to_string(), win, None, timer)));
    Ok(())
}

// ---------- Apple Music, once (crate::apple_import) ----------
thread_local! {
    /// the sign-in window, its web view of Apple's page, and the timer watching it (while signing in)
    static APPLE: RefCell<Option<(AppleSignIn, Option<wry::WebView>, slint::Timer)>> = const { RefCell::new(None) };
    static APPLE_BUSY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn apple_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let when = shared.borrow().web.prefs["appleImported"].as_u64().unwrap_or(0);
    let on = if when > 0 { let d = ((when / 3_600_000) as i64 + local_offset_hours()).div_euclid(24); let (y, m, dd) = civil_date(d); format!(" Last imported {dd}/{m}/{y}.") } else { String::new() };
    ui.set_apple_note(format!("A one-time import. Your loved songs become likes on your other platforms, and your playlists become playlists on this PC. Nothing on Apple Music is changed, and your sign-in is forgotten afterwards.{on}").into());
    ui.set_apple_label(if APPLE_BUSY.with(|b| b.get()) { "Importing...".into() } else if when > 0 { "Import again".into() } else { "Import".into() });
}

/// Import: Apple's sign-in page in a window of its own; once its sign-in cookie is there, it's taken
/// (into memory only) and the window closes (five minutes to sign in), then the import itself
fn apple_start(shared: &Shared) {
    if APPLE_BUSY.with(|b| b.get()) { return; }
    APPLE_BUSY.with(|b| b.set(true));
    apple_paint(shared);
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    rt.spawn(async move {
        // (Apple's page as expected, before you're asked to sign in)
        let ok = omnipulse_core::apple::dev_token().await;
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            if let Err(e) = ok { return apple_fail(&sh, &format!("Couldn't reach Apple Music: {e}")); }
            if let Err(e) = apple_window(&sh) { return apple_fail(&sh, &e); }
            ui.set_status("Sign in to Apple Music in the window that opened. OmniPulse only reads your music and changes nothing there...".into());
            STATUS_HOLD.with(|h| *h.borrow_mut() = Some("Sign in to Apple Music in the window that opened. OmniPulse only reads your music and changes nothing there...".into()));
        }));
    });
}

fn apple_fail(shared: &Shared, why: &str) {
    APPLE.with(|a| { if let Some((w, _, _)) = a.borrow_mut().take() { let _ = w.hide(); } });
    omnipulse_core::apple::forget();
    APPLE_BUSY.with(|b| b.set(false));
    apple_paint(shared);
    STATUS_HOLD.with(|h| h.borrow_mut().take());
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(why.into()); }
}

fn apple_window(shared: &Shared) -> Result<(), String> {
    let w = AppleSignIn::new().map_err(|e| e.to_string())?;
    w.show().map_err(|e| e.to_string())?;
    // each second: the window's size kept to, and the sign-in looked for (five minutes)
    let timer = slint::Timer::default();
    let s = shared.clone();
    let tries = std::rc::Rc::new(std::cell::Cell::new(0u32));
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(500), move || {
        tries.set(tries.get() + 1);
        APPLE.with(|a| {
            let mut a = a.borrow_mut();
            let Some((w, v, _)) = a.as_mut() else { return };
            if v.is_some() { return; }
            use slint::winit_030::WinitWindowAccessor;
            let size = w.window().size();
            *v = w.window().with_winit_window(|win| wry::WebViewBuilder::new()
                .with_url("https://music.apple.com/us/home")
                .with_incognito(true)
                .with_bounds(wry::Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(size.width, size.height).into() })
                .build_as_child(win).ok()).flatten();
        });
        let got = APPLE.with(|a| {
            let a = a.borrow();
            let Some((w, v, _)) = a.as_ref() else { return Err("You closed the sign-in window.".to_string()) };
            if !w.window().is_visible() { return Err("You closed the sign-in window.".to_string()); }
            let sz = w.window().size();
            // (Apple's page put in once the window's there, a moment after it's shown)
            let Some(v) = v.as_ref() else { return Ok(None) };
            let _ = v.set_bounds(wry::Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(sz.width, sz.height).into() });
            Ok(v.cookies_for_url("https://music.apple.com").ok().and_then(|c| c.into_iter().find(|c| c.name() == "media-user-token" && !c.value().is_empty()).map(|c| c.value().to_string())))
        });
        match got {
            Err(e) => { let s = s.clone(); slint::Timer::single_shot(std::time::Duration::ZERO, move || apple_fail(&s, &e)); }
            Ok(Some(token)) => {
                omnipulse_core::apple::set_user_token(token);
                let s = s.clone();
                slint::Timer::single_shot(std::time::Duration::ZERO, move || { APPLE.with(|a| { if let Some((w, _, _)) = a.borrow_mut().take() { let _ = w.hide(); } }); apple_read(&s); });
            }
            Ok(None) if tries.get() >= 600 => { let s = s.clone(); slint::Timer::single_shot(std::time::Duration::ZERO, move || apple_fail(&s, "Signing in to Apple Music took too long.")); }
            Ok(None) => {}
        }
    });
    APPLE.with(|a| *a.borrow_mut() = Some((w, None, timer)));
    Ok(())
}

/// Signed in: your music read (and the sign-in forgotten straight away), each song looked for
fn apple_read(shared: &Shared) {
    let (rt, weak, wh) = {
        let a = shared.borrow();
        let p = &a.web.prefs;
        let wh = apple_import::Where { spotify: plat_on(p, "spotify") && omnipulse_core::spotify::spotify_signed_in(), soundcloud: plat_on(p, "soundcloud") && omnipulse_core::soundcloud::signed_in(), youtube: plat_on(p, "youtube") };
        (a.rt.clone(), a.ui.clone(), wh)
    };
    if let Some(ui) = weak.upgrade() { ui.set_status("Reading your Apple Music...".into()); }
    rt.spawn(async move {
        let lib = omnipulse_core::apple::library().await;
        omnipulse_core::apple::forget();
        let lib = match lib { Ok(l) => l, Err(e) => { let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { apple_fail(&sh, &format!("Couldn't import from Apple Music: {e}")); } })); return; } };
        let w2 = weak.clone();
        let found = apple_import::run(lib, wh, move |msg| { let _ = w2.upgrade_in_event_loop(move |ui| ui.set_status(msg.into())); }).await;
        // the likes (where they're found), as a like from the menu does
        let mut liked = 0usize;
        let mut missing = found.missing;
        let have: std::collections::HashSet<String> = SHARED_LIBRARY_KEYS.with(|k| k.borrow().clone());
        let mut newly = Vec::new();
        for t in &found.likes {
            if have.contains(&t.key) { continue; }
            match marks::set_like(t, true).await { Ok(()) => { liked += 1; newly.push(t.clone()); } Err(e) => missing.push(format!("{} - {} (couldn't like it: {e})", t.artist, t.title)) }
        }
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            {
                let mut a = sh.borrow_mut();
                for t in newly.into_iter().rev() { if !a.library.iter().any(|x| x.key == t.key) { a.library.insert(0, t); } }
                for (name, tracks) in &found.lists { xpl::create(&mut a.xpl, name, tracks); }
            }
            xpl_changed(&sh);
            pref_put(&sh, "appleImported", serde_json::json!(now_ms()));
            APPLE_BUSY.with(|b| b.set(false));
            apple_paint(&sh);
            STATUS_HOLD.with(|h| h.borrow_mut().take());
            let made = found.lists.len();
            let note = format!("Imported from Apple Music: {} liked, {} made{} Your Apple Music sign-in is forgotten.", plural(liked, "loved song", "loved songs"), plural(made, "playlist", "playlists"), if missing.is_empty() { ". Everything was found.".to_string() } else { format!(". {} found anywhere.", plural(missing.len(), "song wasn't", "songs weren't")) });
            if missing.is_empty() { ui.set_status(note.into()); } else { say_with(&ui, &note, vec![("Copy the missing", StatusAct::Copy(missing.join("\n")))]); }
        }));
    });
}

thread_local! {
    /// (the Library's keys, for the import to leave alone what's liked already)
    static SHARED_LIBRARY_KEYS: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
}

/// What a status's button does
#[derive(Clone)]
enum StatusAct { Update, Dismiss, Prefs(Track), Skip, Leave, Copy(String), Restart }

thread_local! {
    /// the status that has buttons (its text), and what each does
    static STATUS_ACTS: RefCell<(String, Vec<StatusAct>)> = const { RefCell::new((String::new(), Vec::new())) };
}

/// A status with buttons: it stays until it's dismissed, replaced, or one of them is clicked
fn say_with(ui: &AppWindow, text: &str, acts: Vec<(&str, StatusAct)>) {
    ui.set_status(text.into());
    STATUS_HOLD.with(|h| *h.borrow_mut() = Some(text.to_string()));
    ui.set_status_actions(ModelRc::from(Rc::new(VecModel::from(acts.iter().map(|a| slint::SharedString::from(a.0)).collect::<Vec<_>>()))));
    STATUS_ACTS.with(|s| *s.borrow_mut() = (text.to_string(), acts.into_iter().map(|a| a.1).collect()));
}

fn status_act(shared: &Shared, i: usize) {
    let Some(act) = STATUS_ACTS.with(|s| s.borrow().1.get(i).cloned()) else { return };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("".into()); }
    match act {
        StatusAct::Update => update_act(shared),
        StatusAct::Dismiss => {}
        StatusAct::Prefs(t) => tp_open(shared, &t),
        StatusAct::Skip => step(shared, 1),
        StatusAct::Leave => listen_leave(shared, false),
        StatusAct::Restart => {
            // (a fresh one started, this one closed: the session kept for it)
            save_session_now(&shared.borrow());
            if let Ok(exe) = std::env::current_exe() { let _ = std::process::Command::new(exe).env("OMNIPULSE_RESTARTING", "1").spawn(); }
            let _ = slint::quit_event_loop();
        }
        StatusAct::Copy(text) => { let _ = arboard::Clipboard::new().and_then(|mut c| c.set_text(text)); if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Copied.".into()); } }
    }
}

thread_local! {
    /// a newer release, once one's known
    static UPDATE: RefCell<Option<updater::Release>> = const { RefCell::new(None) };
}

/// Beta Updates: offered (its switch shown under Updates, and betas given with it on). Off for now: the
/// switch and all it does stay built, for when there are betas to test
const BETA_OFFERED: bool = false;

fn update_check(shared: &Shared, said: bool) {
    let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
    if let Some(ui) = weak.upgrade() { ui.set_update_note("Checking...".into()); }
    // (betas only while Beta Updates is offered: hidden for now, so a switch left on doesn't keep giving them)
    let beta = BETA_OFFERED && shared.borrow().web.prefs["betaUpdates"].as_bool().unwrap_or(false);
    rt.spawn(async move {
        let r = updater::check(beta).await;
        let _ = weak.upgrade_in_event_loop(move |ui| {
            match r {
                Ok(Some(rel)) => {
                    ui.set_update_note(format!("OmniPulse {} is out (this is {}). {}", rel.version, updater::VERSION, rel.notes).into());
                    ui.set_update_label("Update".into());
                    say_with(&ui, &format!("OmniPulse {} is out.", rel.version), vec![("Update", StatusAct::Update), ("Later", StatusAct::Dismiss)]);
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
    let Some(dest) = ask_path(rfd::FileDialog::new().set_title("Back up OmniPulse").set_file_name(format!("OmniPulse Backup {day}.zip")).add_filter("OmniPulse backup", &["zip"]), Ask::Save, { let s = shared.clone(); move || backup_make(&s) }) else { return };
    let msg = match omnipulse_core::backup::create(&dest.to_string_lossy(), &backup_web()) {
        Ok(n) => format!("Backed up: your settings and {n} {}.", if n == 1 { "file" } else { "files" }),
        Err(e) => format!("Couldn't back up: {e}"),
    };
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(msg.into()); }
}

fn backup_restore(shared: &Shared) {
    let Some(src) = ask_path(rfd::FileDialog::new().set_title("Restore an OmniPulse backup").add_filter("OmniPulse backup", &["zip"]), Ask::Open, { let s = shared.clone(); move || backup_restore(&s) }) else { return };
    ask(shared, "Restore this backup?", "Your settings, favourites, playlists, stats and alerts are replaced with the backup's, and OmniPulse starts again.", "Restore", move |shared| backup_restore_from(shared, src));
}

fn backup_restore_from(shared: &Shared, src: std::path::PathBuf) {
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
    let _ = safe;
    if !private {
        set_private(true);
        if let Some(ui) = shared.borrow().ui.upgrade() {
            ui.set_status("Streamer Mode is on. Account names, keys and addresses are hidden. Right-click it to play only music that's safe on stream.".into());
        }
    } else {
        set_private(false);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Streamer Mode off.".into()); }
    }
    // (who you're signed in as: hidden, or shown again; and Settings' switches, Streamer Mode's and Extended's,
    // in step with the title bar's button)
    accounts_paint(shared);
    paint_settings(&shared.borrow());
}

fn safe_set(shared: &Shared, on: bool) {
    {
        let mut a = shared.borrow_mut();
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("streamSafe".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("streamSafe", serde_json::json!(on)); }
        a.ap_failed = None;
        // (on: what's queued that it doesn't allow goes, after what's playing)
        if on {
            let prefs = a.web.prefs.clone();
            let cur = a.queue.qi.unwrap_or(0);
            let mut i = 0;
            a.queue.items.retain(|q| { let keep = i <= cur || streamsafe::allowed(&prefs, &q.t); i += 1; keep });
        }
        autoplay_for_safe(&mut a, on);
        paint_queue(&mut a);
        apply_filter(&mut a);
        paint_settings(&a);
        if let Some(ui) = a.ui.upgrade() {
            ui.set_stream_safe(on);
            let private = integrations::set(&a.web.prefs, "streamerMode");
            ui.set_status(if on { format!("Streamer Mode is on. Only music that's safe on stream shows and plays{}", if private { ", and private info is hidden." } else { "." }) } else { "Stream-Safe Music off.".to_string() }.into());
        }
    }
    // the Streamer Mode page takes the place of the pages it stands in for while it's on (and on one of
    // them, it opens); off, from it, back to the Library (as the web app's streamerPageSync)
    let page = shared.borrow().page.clone();
    STREAM_BUILT.with(|b| b.borrow_mut().clear());
    nav_paint(shared);
    paint_sides(&mut shared.borrow_mut());
    if on && matches!(page.as_str(), "discover" | "rediscover" | "library" | "favourites" | "sets" | "artists" | "playlist" | "artist") { open_streamer(shared); }
    if !on && (page == "streamer" || page == "streamcoll") { go(shared, Place::Page("library".into())); }
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
            // (stopped: the accent you picked again, not the last cover's)
            apply_look(shared);
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
    let Ok(mut src) = omnipulse_audio::Decoded::open_file(&path) else { return };
    // (trimmed: from Trim Start, over Trim End before its end)
    let ms = |v: &serde_json::Value| v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok())).map(|x| x.max(0.0).round() as u64).unwrap_or(0);
    let (tstart, tend) = (ms(&e["tstart"]), ms(&e["tend"]));
    if tstart > 0 { let _ = omnipulse_audio::Source::seek(&mut src, tstart); }
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
            let cut = tend > 0 && player.duration_ms() > 0 && player.position_ms() + tstart + tend >= player.duration_ms();
            if cut { player.pause(); }
            let ended = cut || events.as_ref().is_some_and(|rx| rx.try_iter().any(|ev| matches!(ev, omnipulse_audio::Event::Ended | omnipulse_audio::Event::Error(_))));
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

/// A sign-in's way back to this PC: one visit to http://localhost:<port><path> within five minutes, a page
/// saying it's done, and its query
fn await_redirect(port: u16, path: &str) -> Result<String, String> {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|e| format!("couldn't listen on port {port} for the sign-in: {e}"))?;
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(300);
    while std::time::Instant::now() < until {
        match listener.accept() {
            Ok((mut tcp, _)) => {
                let _ = tcp.set_nonblocking(false);
                let _ = tcp.set_read_timeout(Some(std::time::Duration::from_secs(5)));
                let mut buf = [0u8; 8192];
                let n = tcp.read(&mut buf).unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let target = req.lines().next().and_then(|l| l.split(' ').nth(1)).unwrap_or("").to_string();
                let (at, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
                if at != path {
                    let _ = tcp.write_all(b"HTTP/1.1 404 Not Found
Content-Length: 0
Connection: close

");
                    continue;
                }
                let page = "<!doctype html><meta charset=utf-8><title>OmniPulse</title><body style='font-family:sans-serif;background:#111;color:#eee;display:grid;place-items:center;height:90vh'><p>Done. You can close this tab and go back to OmniPulse.</p>";
                let _ = tcp.write_all(format!("HTTP/1.1 200 OK
Content-Type: text/html; charset=utf-8
Content-Length: {}
Connection: close

{page}", page.len()).as_bytes());
                return Ok(query.to_string());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(std::time::Duration::from_millis(100)),
            Err(e) => return Err(e.to_string()),
        }
    }
    Err("the sign-in wasn't finished in five minutes".into())
}

/// Words said by Windows' own speech, as a sound file (in the temp folder: the last few kept), in the voice
/// asked for (a name, or part of one; empty: Windows' own) at that speed
#[cfg(windows)]
fn speech_file(text: &str, voice: &str, rate: f64) -> Result<std::path::PathBuf, String> {
    use windows::Media::SpeechSynthesis::SpeechSynthesizer;
    use windows::Storage::Streams::DataReader;
    let _ = unsafe { windows::Win32::System::WinRT::RoInitialize(windows::Win32::System::WinRT::RO_INIT_MULTITHREADED) };
    let err = |e: windows::core::Error| format!("Windows' speech: {}", e.message());
    let synth = SpeechSynthesizer::new().map_err(err)?;
    if !voice.trim().is_empty() {
        let want = voice.trim().to_lowercase();
        if let Ok(all) = SpeechSynthesizer::AllVoices() {
            for v in all {
                if v.DisplayName().map(|n| n.to_string().to_lowercase().contains(&want)).unwrap_or(false) { synth.SetVoice(&v).map_err(err)?; break; }
            }
        }
    }
    if let Ok(o) = synth.Options() { let _ = o.SetSpeakingRate(rate.clamp(0.5, 3.0)); }
    let stream = synth.SynthesizeTextToStreamAsync(&windows::core::HSTRING::from(text)).map_err(err)?.join().map_err(err)?;
    let size = stream.Size().map_err(err)? as u32;
    let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).map_err(err)?).map_err(err)?;
    reader.LoadAsync(size).map_err(err)?.join().map_err(err)?;
    let mut buf = vec![0u8; size as usize];
    reader.ReadBytes(&mut buf).map_err(err)?;
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!("omnipulse-speech-{}.wav", N.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 8));
    std::fs::write(&path, buf).map_err(|e| format!("couldn't keep the speech: {e}"))?;
    Ok(path)
}
#[cfg(not(windows))]
fn speech_file(_text: &str, _voice: &str, _rate: f64) -> Result<std::path::PathBuf, String> { Err("speech is Windows' only, for now".into()) }

thread_local! {
    /// the sounds playing (a chatbot's Play a Sound): each kept going until it's over
    static SOUNDS: RefCell<Vec<(Rc<std::cell::Cell<bool>>, slint::Timer)>> = const { RefCell::new(Vec::new()) };
}

/// A sound file of yours played over the music, on the speakers OmniPulse plays to (at most three minutes)
fn play_sound(path: &str, vol: f32) -> Result<(), String> {
    let p = std::path::Path::new(path);
    let ext = p.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    if !matches!(ext.as_str(), "mp3" | "wav" | "ogg" | "flac" | "m4a" | "aac" | "opus") { return Err("that isn't a sound file (mp3, wav, ogg or flac)".into()); }
    if !p.is_file() { return Err(format!("there's no sound file at {path}")); }
    let src = omnipulse_audio::Decoded::open_file(p).map_err(|e| format!("couldn't play it: {e}"))?;
    let device = webstate::native_pref("nativeDevice").and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
    let player = Arc::new(omnipulse_audio::Engine::new(&device).map_err(|e| format!("couldn't play it: {e}"))?);
    player.load(Box::new(src), 0.0);
    player.set_volume(vol);
    player.play();
    let events = player.events();
    let done = Rc::new(std::cell::Cell::new(false));
    let d2 = done.clone();
    let began = std::time::Instant::now();
    let t = slint::Timer::default();
    t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        let _keep = &player;
        let over = began.elapsed() > std::time::Duration::from_secs(180) || events.as_ref().is_some_and(|rx| rx.try_iter().any(|ev| matches!(ev, omnipulse_audio::Event::Ended | omnipulse_audio::Event::Error(_))));
        if over { player.pause(); d2.set(true); }
    });
    // (the ones that are over let go of, this one kept)
    SOUNDS.with(|s| { let mut s = s.borrow_mut(); s.retain(|(d, _)| !d.get()); s.push((done, t)); });
    Ok(())
}

/// A plugin's OmniPulse.app call (its manifest asked for "app"): the app's work done on its own thread,
/// the answer sent back. A chatbot's requests go through the same limits and replies as the bot server's.
fn plugin_app(shared: &Shared, plugin: &str, ask: serde_json::Value, answer: Box<dyn FnOnce(serde_json::Value) + Send>) {
    let args = ask["args"].as_array().cloned().unwrap_or_default();
    let arg = |i: usize| args.get(i).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let once = std::sync::Arc::new(std::sync::Mutex::new(Some(answer)));
    let send = { let o = once.clone(); move |v: serde_json::Value| { if let Some(f) = o.lock().unwrap().take() { f(v); } } };
    match ask["op"].as_str().unwrap_or("") {
        // (a song request from chat: the platform it came from is noted with who asked)
        "request" => {
            let reply: ReqReply = std::sync::Arc::new(send);
            let from_bot = plugin == botpage::BOT;
            if !req_open(&shared.borrow().web.prefs, from_bot) { return reply(serde_json::json!({ "ok": false, "message": requests::msg(&shared.borrow().web.prefs, "closed", &[]) })); }
            song_request(shared, reply, arg(1), arg(0), from_bot);
        }
        "wrongSong" => {
            let is_mod = args.get(1).and_then(|v| v.as_bool()) == Some(true);
            wrong_song(shared, std::sync::Arc::new(send), &arg(0), is_mod, plugin == botpage::BOT);
        }
        "nowPlaying" => send(omnipulse_core::requests::now_answer()),
        // (a web address's answer, as text: the chatbot's Fetch a Web Address. OmniPulse's own chatbot only:
        // any other plugin reaches only its manifest's sites, by its own fetch)
        "fetchText" => {
            let url = arg(0);
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's chatbot can" })); }
            if !(url.starts_with("https://") || url.starts_with("http://")) { return send(serde_json::json!({ "error": "that isn't a web address" })); }
            let rt = shared.borrow().rt.clone();
            rt.spawn(async move {
                let got = async {
                    let r = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().map_err(|e| e.to_string())?.get(&url).send().await.map_err(|e| e.to_string())?;
                    let ok = r.status().is_success();
                    let text = r.text().await.map_err(|e| e.to_string())?;
                    if !ok { return Err(format!("the address answered: {}", text.chars().take(120).collect::<String>())); }
                    Ok::<String, String>(text.trim().chars().take(500).collect())
                }.await;
                send(match got { Ok(text) => serde_json::json!({ "ok": true, "text": text }), Err(e) => serde_json::json!({ "error": e }) });
            });
        }
        // (a sign-in coming back to this PC: the chatbot's Kick sign-in, its address http://localhost:<port><path>;
        // one visit, within five minutes, its query handed back)
        "awaitRedirect" => {
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's chatbot can" })); }
            let port = args.first().and_then(|v| v.as_u64()).unwrap_or(0) as u16;
            let path = arg(1);
            std::thread::spawn(move || send(match await_redirect(port, &path) { Ok(q) => serde_json::json!({ "ok": true, "query": q }), Err(e) => serde_json::json!({ "error": e }) }));
        }
        // (the Suite's Name That Tune: what's playing hidden from the Stream Widget while chat guesses it)
        "tuneMask" => {
            if plugin != botpage::BOT { return send(serde_json::json!({ "error": "only the Stream Suite can" })); }
            omnipulse_core::widget::tune_mask(args.first().and_then(|v| v.as_bool()).unwrap_or(false));
            send(serde_json::json!({ "ok": true }));
        }
        // (the Suite's Press Keys: as if typed, into whatever's in front)
        "pressKeys" => {
            if plugin != botpage::BOT { return send(serde_json::json!({ "error": "only the Stream Suite can" })); }
            send(match procwatch::press(&arg(0)) { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) });
        }
        // (the Suite's Open a File or Web Address: a program, a file or a web page, as double-clicking it)
        "openThing" => {
            if plugin != botpage::BOT { return send(serde_json::json!({ "error": "only the Stream Suite can" })); }
            send(match procwatch::open(&arg(0), &arg(1)) { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) });
        }
        // (the Suite's widgets, as they are now: passed on to OBS's pages)
        "suiteWidget" => {
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's Stream Suite can" })); }
            // ("Your Theme's" colours: the app theme's own two (App & Look), never the album art's; filled in
            // wherever the Suite sent none: its widgets and each creator)
            let mut v = args.first().cloned().unwrap_or(serde_json::Value::Null);
            let mine = theme_accents(&shared.borrow().web.prefs);
            let fill = |x: &mut serde_json::Value| if x.get("colors").map(|c| c.is_null()).unwrap_or(true) { x["colors"] = serde_json::json!({ "c1": mine.0, "c2": mine.1 }); };
            if v.is_object() { fill(&mut v); if let Some(list) = v.get_mut("creators").and_then(|c| c.as_array_mut()) { list.iter_mut().filter(|c| c.is_object()).for_each(fill); } }
            integrations::set_suite(v);
            push_out(shared, true);
            send(serde_json::json!({ "ok": true }));
        }
        // (JSON posted to a Discord webhook: the chatbot's Send to Discord; Discord's own addresses only)
        "postJson" => {
            let url = arg(0);
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's chatbot can" })); }
            let ok = ["https://discord.com/api/webhooks/", "https://discordapp.com/api/webhooks/", "https://ptb.discord.com/api/webhooks/", "https://canary.discord.com/api/webhooks/"].iter().any(|p| url.starts_with(p));
            if !ok { return send(serde_json::json!({ "error": "that isn't a Discord webhook address (https://discord.com/api/webhooks/...)" })); }
            let body = args.get(1).cloned().unwrap_or(serde_json::Value::Null);
            let rt = shared.borrow().rt.clone();
            rt.spawn(async move {
                let got = async {
                    let r = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).build().map_err(|e| e.to_string())?.post(&url).json(&body).send().await.map_err(|e| e.to_string())?;
                    if r.status().is_success() { Ok(()) } else { Err(format!("Discord said {}", r.status())) }
                }.await;
                send(match got { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) });
            });
        }
        // (words said out loud: Windows' own speech, made into a sound file, then played as a sound is)
        "speak" => {
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's chatbot can" })); }
            let (text, voice) = (arg(0), arg(1));
            let rate = args.get(2).and_then(|v| v.as_f64()).unwrap_or(1.0);
            let vol = args.get(3).and_then(|v| v.as_f64()).unwrap_or(80.0);
            std::thread::spawn(move || {
                let made = speech_file(&text, &voice, rate);
                let _ = slint::invoke_from_event_loop(move || send(match made.and_then(|p| play_sound(&p.to_string_lossy(), (vol / 100.0).clamp(0.0, 1.0) as f32)) { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) }));
            });
        }
        // (something shown on stream: the widget's, over its alerts; a picture or video of yours copied for it)
        "overlay" => {
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's chatbot can" })); }
            let mut fields = args.first().and_then(|v| v.as_object()).cloned().unwrap_or_default();
            for key in ["image", "video"] {
                let v = fields.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
                if v.is_empty() || v.starts_with("https://") || v.starts_with("http://") { continue; }
                match omnipulse_core::widget::overlay_media(&v, key == "video") { Ok(u) => { fields.insert(key.into(), serde_json::json!(u)); } Err(e) => return send(serde_json::json!({ "error": e })) }
            }
            send(match omnipulse_core::widget::overlay_push(&fields) { Ok(_) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) });
        }
        // (OBS's docks: the chat as the chatbot hears it, messages taken down, what happened for the activity feed)
        "dockChat" | "dockGone" | "dockActivity" => {
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's chatbot can" })); }
            let v = args.first().cloned().unwrap_or(serde_json::Value::Null);
            if !v.is_object() { return send(serde_json::json!({ "error": "give it an object" })); }
            let s = |k: &str| v[k].as_str().unwrap_or("").to_string();
            match ask["op"].as_str().unwrap_or("") { "dockChat" => omnipulse_core::docks::chat_push(v.clone()), "dockGone" => omnipulse_core::docks::chat_gone(&s("platform"), &s("id"), &s("login")), _ => omnipulse_core::docks::activity_push(v.clone()) }
            send(serde_json::json!({ "ok": true }));
        }
        // (a text file of yours, written: a chatbot's Write to a File, for an OBS Text source)
        "writeFile" => {
            if plugin != "chatbot" { return send(serde_json::json!({ "error": "only OmniPulse's chatbot can" })); }
            let (path, text) = (arg(0), arg(1));
            let append = args.get(2).and_then(|v| v.as_bool()).unwrap_or(false);
            let p = std::path::Path::new(&path);
            if !p.is_absolute() || !p.extension().map(|e| e.eq_ignore_ascii_case("txt")).unwrap_or(false) { return send(serde_json::json!({ "error": "give it a .txt file's whole place (C:\\...\\name.txt)" })); }
            let wrote = (|| -> std::io::Result<()> {
                if let Some(d) = p.parent() { std::fs::create_dir_all(d)?; }
                if append { use std::io::Write; let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?; writeln!(f, "{text}") } else { std::fs::write(p, text) }
            })();
            send(match wrote { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": format!("couldn't write {path}: {e}") }) });
        }
        // (a sound file of yours, over the music: a chatbot's Play a Sound)
        "sound" => {
            let vol = args.get(1).and_then(|v| v.as_f64()).unwrap_or(80.0);
            send(match play_sound(&arg(0), (vol / 100.0).clamp(0.0, 1.0) as f32) { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) });
        }
        // (the player: the next song, or play / pause)
        "control" => {
            match arg(0).as_str() { "next" => desktop_action(shared, "next"), "play" => desktop_action(shared, "play"), other => return send(serde_json::json!({ "error": format!("no {other}") })) }
            send(serde_json::json!({ "ok": true }));
        }
        // (an alert, as a bot's /alert: its type, who, how many, their message)
        "alert" => {
            let fields = args.first().and_then(|v| v.as_object()).cloned().unwrap_or_default();
            send(match omnipulse_core::widget::alert_push(&fields) { Ok(a) => serde_json::json!({ "ok": true, "alert": a }), Err(e) => serde_json::json!({ "error": e }) });
        }
        // (a sign-in's link, copied for a browser of your choosing: https only)
        "copyText" => {
            let text = arg(0);
            if !text.starts_with("https://") { return send(serde_json::json!({ "error": "only https addresses" })); }
            send(match share::copy(&text) { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) });
        }
        // (a sign-in page: https only, in the browser)
        "openUrl" => {
            let url = arg(0);
            if !url.starts_with("https://") { return send(serde_json::json!({ "error": "only https addresses" })); }
            // (the Stream Suite's: any https address, as you set its steps; other plugins only to the services
            // OmniPulse works with)
            if plugin == botpage::BOT { return send(match procwatch::open(&url, "") { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) }); }
            send(match omnipulse_core::open_url(url) { Ok(()) => serde_json::json!({ "ok": true }), Err(e) => serde_json::json!({ "error": e }) });
        }
        "say" => {
            let name = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == plugin).map(|p| p.manifest.name).unwrap_or_default();
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{name}: {}", arg(0).chars().take(300).collect::<String>()).into()); }
            send(serde_json::json!({ "ok": true }));
        }
        // (its own settings, kept: a sign-in's tokens; merged into what's kept, never running it again)
        "saveSettings" => {
            let values = args.first().and_then(|v| v.as_object()).cloned().unwrap_or_default();
            let had_actions = values.contains_key("actions");
            let all = {
                let mut a = shared.borrow_mut();
                let mut all = a.web.prefs["pluginSettings"].as_object().cloned().unwrap_or_default();
                let mut mine = all.get(plugin).and_then(|v| v.as_object()).cloned().unwrap_or_default();
                // (its page open: what it kept shown there at once, as saved, not as a change to save)
                PF_EDIT.with(|e| if let Some((id, v)) = e.borrow_mut().as_mut() {
                    if id == plugin { for (k, x) in &values { v.insert(k.clone(), x.clone()); } PF_ORIG.with(|o| { let mut o = o.borrow_mut(); for (k, x) in &values { o.insert(k.clone(), x.clone()); } }); }
                });
                for (k, v) in values { mine.insert(k, v); }
                all.insert(plugin.to_string(), serde_json::Value::Object(mine));
                let all = serde_json::Value::Object(all);
                if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pluginSettings".into(), all.clone()); }
                all
            };
            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pluginSettings", all); }
            send(serde_json::json!({ "ok": true }));
            if plugin == botpage::BOT { BOT_DRAWN.with(|d| *d.borrow_mut() = Default::default()); bot_paint(shared); }
            if plugin == botpage::BOT { emotes_refresh(shared, false); }
            if plugin == botpage::BOT && had_actions {
                let list = flows_load(shared);
                FLOW.with(|f| f.borrow_mut().0 = list);
                let open = shared.borrow().ui.upgrade().is_some_and(|ui| ui.get_flow_open());
                if open { flows_paint(shared); }
                hotkeys_update(shared);
            }
        }
        other => send(serde_json::json!({ "error": format!("the app has no {other}") })),
    }
}

fn outside_start(shared: &Shared) {
    let prefs = shared.borrow().web.prefs.clone();
    plugrun::set_app_handler(std::sync::Arc::new(|plugin: &str, ask: serde_json::Value, answer: Box<dyn FnOnce(serde_json::Value) + Send>| {
        let plugin = plugin.to_string();
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { plugin_app(&sh, &plugin, ask, answer); } }));
    }));
    omnipulse_core::widget::set_shown_handler(std::sync::Arc::new(|kind: &str| {
        let kind = kind.to_string();
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { alert_shown(&sh, &kind); } }));
    }));
    // (a dock's button: given to the chatbot, its answer or why not said back in the chat dock)
    omnipulse_core::docks::page_settings_set(prefs["dockSet"].clone());
    emotes_refresh(&shared, true);
    {
        let s = shared.clone();
        let t = slint::Timer::default();
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_secs(1800), move || emotes_refresh(&s, true));
        EMOTES_TIMER.with(|x| *x.borrow_mut() = Some(t));
    }
    omnipulse_core::docks::set_dock_handler(std::sync::Arc::new(|cmd: serde_json::Value| {
        let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            if cmd["do"] == "settings" {
                let all = docksets::from_page(&sh.borrow().web.prefs, cmd["page"].as_str().unwrap_or(""), &cmd["opt"]);
                omnipulse_core::docks::page_settings_set(all.clone());
                pref_put(&sh, "dockSet", all);
                return;
            }
            match plugrun::get(BOT) {
                Some(p) => {
                    let rt = sh.borrow().rt.clone();
                    rt.spawn(async move {
                        match p.call("dock", vec![cmd]).await {
                            Ok(v) => if let Some(m) = v.as_str().filter(|m| !m.is_empty()) { omnipulse_core::docks::notice(m) },
                            Err(e) => omnipulse_core::docks::notice(&format!("Couldn't: {e}")),
                        }
                    });
                }
                None => omnipulse_core::docks::notice("The chatbot isn't on. Turn it on in Settings > Streaming."),
            }
        }));
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
                        wrong_song(&sh, to_server(id), &user, ask["mod"].as_bool() == Some(true), false);
                    } else if ask["action"].as_str() == Some("chatbot") {
                        // (an action run by its web address: A Web Address, the chatbot's)
                        let reply = to_server(id);
                        let Some(p) = plugrun::get(BOT) else { return reply(serde_json::json!({ "ok": false, "message": "The chatbot isn't on." })) };
                        let rt = sh.borrow().rt.clone();
                        let data = serde_json::json!({ "name": ask["name"], "user": user, "input": ask["q"] });
                        rt.spawn(async move {
                            let r = p.call("appEvent", vec![serde_json::json!("web"), data]).await;
                            let msg = match r { Ok(v) => v.as_str().map(String::from).unwrap_or_else(|| "Done.".into()), Err(e) => e };
                            reply(serde_json::json!({ "ok": !msg.starts_with("There's no"), "message": msg }));
                        });
                    } else {
                        song_request(&sh, to_server(id), ask["q"].as_str().unwrap_or("").to_string(), user, false);
                    }
                }));
            }));
            let _ = omnipulse_core::requests::requests_start(prefs["reqPort"].as_u64().unwrap_or(7879) as u16, key.to_string(), integrations::set(&prefs, "botRemote"));
        }
    }
    push_out(shared, false);
}

/// A media key, the tray's menu or a taskbar button
/// The window closed (its X): OmniPulse quits, always
fn close_or_tray(shared: &Shared) {
    save_session_now(&shared.borrow());
    let _ = slint::quit_event_loop();
}

/// Minimize to Tray (Settings > App), else minimized to the taskbar as usual. (It was Close to Tray:
/// that one switched on is this one switched on.)
fn min_to_tray(prefs: &serde_json::Value) -> bool {
    prefs["minToTray"].as_bool().or(prefs["closeToTray"].as_bool()).unwrap_or(false)
}

/// The window minimized: to the tray (the music carries on; Open OmniPulse and Quit are in its menu),
/// with Minimize to Tray on
fn minimize(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if min_to_tray(&shared.borrow().web.prefs) {
        save_session_now(&shared.borrow());
        let _ = ui.window().hide();
    } else {
        ui.window().set_minimized(true);
    }
}

/// The devices menu's last line, with Chromecast on: a fresh look for cast devices
const CAST_SEARCH: &str = "Search for Cast devices";

thread_local! {
    /// (the Suite's Widgets or Docks page, open ("widgets", "docks"; "" for neither): what's opened from there
    /// goes back to it)
    static BOT_WIDGETS: RefCell<String> = const { RefCell::new(String::new()) };
    /// (BTTV, FFZ and 7TV's lists: fetched again every half hour; the accounts they were last fetched for)
    static EMOTES_TIMER: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    static EMOTES_FOR: RefCell<Option<(String, String)>> = const { RefCell::new(None) };
    /// (the volume's save, waiting for the slider to settle)
    static VOLUME_SAVE: RefCell<Option<slint::Timer>> = const { RefCell::new(None) };
    /// the chatbot's hotkeys, waiting to be registered (the tick has the Desktop that does it)
    static HOTKEYS_WANTED: RefCell<Option<Vec<String>>> = const { RefCell::new(None) };
}

/// One of the Suite's settings pages (a connection's, a feature's, a widget's): its name the page's title, its
/// crumbs through the Suite (and its Widgets or Docks, if it was opened from there), so Back goes where it came
/// from; none (""), the Suite's page, or its Widgets or Docks, as it was
fn bot_sheet_crumbs(shared: &Shared, id: &str) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let label = bot_manifest().map(|m| place_label(&m)).unwrap_or_else(|| "Suite".into());
    let which = BOT_WIDGETS.with(|w| w.borrow().clone());
    if id.is_empty() {
        if !which.is_empty() { return bot_widgets_open(shared, &which); }
        crumbs_set(&ui, vec![("Settings".into(), "root".into()), ("Streaming".into(), "cat:Streaming".into())]);
        ui.set_pf_head(label.into());
        return;
    }
    let mut path = vec![("Settings".to_string(), "root".to_string()), ("Streaming".into(), "cat:Streaming".into()), (label, format!("plugin:{}", botpage::BOT))];
    if !which.is_empty() { path.push((widgets_title(&which).into(), format!("bot{which}"))); }
    crumbs_set(&ui, path);
    ui.set_pf_head(botpage::sheet(id, &bot_current(shared)).0.into());
}

fn widgets_title(which: &str) -> &'static str { match which { "docks" => "Docks", "creators" => "Creators", _ => "Widgets" } }

/// The Suite's Widgets, or its Docks, as a page of their own: Settings > Streaming > Suite > Widgets
fn bot_widgets_open(shared: &Shared, which: &str) {
    BOT_WIDGETS.with(|w| *w.borrow_mut() = which.to_string());
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let label = bot_manifest().map(|m| place_label(&m)).unwrap_or_else(|| "Suite".into());
    crumbs_set(&ui, vec![("Settings".into(), "root".into()), ("Streaming".into(), "cat:Streaming".into()), (label, format!("plugin:{}", botpage::BOT))]);
    ui.set_pf_head(widgets_title(which).into());
    ui.set_bot_widgets(which.into());
}

/// BTTV's, FFZ's and 7TV's emotes, for the docks and the Multichat Overlay: fetched for your Twitch and Kick
/// accounts (the Suite's), again when they change (`always`: whether or not they have)
fn emotes_refresh(shared: &Shared, always: bool) {
    let (twitch, kick, rt) = {
        let a = shared.borrow();
        let m = &a.web.prefs["pluginSettings"][BOT];
        (m["userId"].as_str().unwrap_or("").to_string(), match &m["kickUserId"] { serde_json::Value::String(s) => s.clone(), serde_json::Value::Number(n) => n.to_string(), _ => String::new() }, a.rt.clone())
    };
    let now = (twitch.clone(), kick.clone());
    let same = EMOTES_FOR.with(|e| e.borrow().as_ref() == Some(&now));
    if same && !always { return; }
    EMOTES_FOR.with(|e| *e.borrow_mut() = Some(now));
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() { return; }
    rt.spawn(async move { omnipulse_core::emotes::refresh(&twitch, &kick).await; });
}

/// The chatbot's hotkeys from its actions (on, with it running), to be registered
fn hotkeys_update(shared: &Shared) {
    let prefs = &shared.borrow().web.prefs;
    let on = plugin_runs(prefs, BOT);
    let list: Vec<String> = if !on { Vec::new() } else {
        prefs["pluginSettings"][BOT]["actions"].as_array().into_iter().flatten()
            .filter(|a| a["on"].as_bool() != Some(false) && a["trigger"]["type"] == "hotkey")
            .filter_map(|a| a["trigger"]["keys"].as_str().map(|k| k.trim().to_string())).filter(|k| !k.is_empty()).collect()
    };
    HOTKEYS_WANTED.with(|h| *h.borrow_mut() = Some(list));
    // (and the programs its actions wait for: told to it as each starts)
    let progs: Vec<String> = if !on { Vec::new() } else {
        prefs["pluginSettings"][BOT]["actions"].as_array().into_iter().flatten()
            .filter(|a| a["on"].as_bool() != Some(false) && a["trigger"]["type"] == "process")
            .filter_map(|a| a["trigger"]["name"].as_str().map(|k| k.trim().to_string())).filter(|k| !k.is_empty()).collect()
    };
    let rt = shared.borrow().rt.clone();
    procwatch::want(progs, move |name, path| {
        if let Some(p) = plugrun::get(BOT) { rt.spawn(async move { let _ = p.call("appEvent", vec![serde_json::json!("process"), serde_json::json!({ "name": name, "path": path })]).await; }); }
    });
}

fn desktop_action(shared: &Shared, action: &str) {
    if let Some(keys) = action.strip_prefix("hotkey:") {
        if let Some(p) = plugrun::get(BOT) {
            let (keys, rt) = (keys.to_string(), shared.borrow().rt.clone());
            rt.spawn(async move { let _ = p.call("appEvent", vec![serde_json::json!("hotkey"), serde_json::json!({ "keys": keys })]).await; });
        }
        return;
    }
    match action {
        "play" => toggle(shared),
        "next" => skip_press(shared, 1),
        "prev" => skip_press(shared, -1),
        "like" => like_toggle(shared),
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
            save_session_now(&shared.borrow());
            let _ = slint::quit_event_loop();
        }
        _ => {}
    }
}

thread_local! {
    /// Settings > App > Taskbar Widget: its window (made the first time it's switched on), its handle,
    /// and whether it's showing
    static TBW: RefCell<Option<(TaskbarWidget, isize, bool)>> = const { RefCell::new(None) };
}

/// What's playing, for Windows' media controls
fn now_for_windows(a: &App) -> desktop::Now {
    let t = a.queue.current();
    desktop::Now {
        title: t.map(|t| t.title.clone()).unwrap_or_default(),
        artist: t.map(|t| t.artist.clone()).unwrap_or_default(),
        album: t.map(|t| t.album.clone()).unwrap_or_default(),
        art: t.map(|t| t.art.replace("-t200x200", "-t500x500")).unwrap_or_default(),
        playing: a.loaded && !a.engine.is_paused(),
        has_track: t.is_some(),
        pos_ms: a.engine.position_ms(),
        dur_ms: a.engine.duration_ms().max(t.map_or(0, |t| t.ms)),
    }
}

/// Windows' media controls and the taskbar widget, kept in step (each tick)
fn windows_now(shared: &Shared, ui: &AppWindow) {
    let (now, widget) = {
        let a = shared.borrow();
        let prefs = &a.web.prefs;
        (now_for_windows(&a), prefs["taskbarWidget"].as_bool() == Some(true) && (std::env::var_os("OMNIPULSE_SELFTEST").is_none() || std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("taskbar")))
    };
    desktop::media::update(&now);
    // a place asked for from them
    if let Some(ms) = desktop::take_seek() {
        let engine = shared.borrow().engine.clone();
        engine.seek(ms);
        let s2 = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(300), move || push_out(&s2, true));
    }
    taskbar_widget(shared, ui, widget);
}

/// The taskbar widget: made, shown, kept in its place and in step with the player bar; hidden while
/// something's full screen or the taskbar is (and with the switch off)
fn taskbar_widget(shared: &Shared, ui: &AppWindow, on: bool) {
    TBW.with(|w| {
        let mut w = w.borrow_mut();
        if !on {
            if let Some((tw, _, shown)) = w.as_mut() { if *shown { let _ = tw.hide(); *shown = false; } }
            return;
        }
        if w.is_none() {
            let Ok(tw) = TaskbarWidget::new() else { return };
            let s = shared.clone();
            tw.on_act(move |a| {
                let s = s.clone();
                // (after the click's done with: what it does may repaint the widget)
                slint::Timer::single_shot(std::time::Duration::ZERO, move || desktop_action(&s, &a));
            });
            *w = Some((tw, 0, false));
        }
        let Some((tw, hwnd, shown)) = w.as_mut() else { return };
        // the app's colours
        let (mt, wt) = (ui.global::<Theme>(), tw.global::<Theme>());
        // (see-through over a dark taskbar: light text, as its own; over a light one, the app's panel)
        let glass = desktop::dock::taskbar_dark();
        tw.set_glass(glass);
        if glass {
            wt.set_panel(slint::Color::from_argb_u8(0, 0, 0, 0)); wt.set_elevated(slint::Color::from_rgb_u8(0x2c, 0x2c, 0x2c));
            wt.set_text(slint::Color::from_rgb_u8(0xf3, 0xf3, 0xf3)); wt.set_muted(slint::Color::from_rgb_u8(0xb4, 0xb4, 0xb4));
        } else {
            wt.set_panel(mt.get_panel()); wt.set_elevated(mt.get_elevated()); wt.set_text(mt.get_text()); wt.set_muted(mt.get_muted());
        }
        wt.set_accent_own(mt.get_accent()); wt.set_accent_2_own(mt.get_accent_2()); wt.set_on_accent(mt.get_on_accent()); wt.set_motion(mt.get_motion());
        // what's playing, as the player bar has it
        tw.set_has_track(shared.borrow().queue.current().is_some());
        tw.set_song(ui.get_now_title()); tw.set_artist(ui.get_now_artist()); tw.set_plat(ui.get_now_plat());
        if tw.get_art() != ui.get_now_art() { tw.set_art(ui.get_now_art()); }
        tw.set_playing(ui.get_playing()); tw.set_liked(ui.get_now_liked()); tw.set_can_like(ui.get_can_like());
        tw.set_frac(ui.get_pos_frac()); tw.set_mixing(ui.get_mixing());
        tw.set_xf_on(ui.get_xf_on()); tw.set_xf_frac(ui.get_xf_frac());
        if ui.get_xf_on() && tw.get_xf_from_art() != ui.get_xf_from_art() { tw.set_xf_from_art(ui.get_xf_from_art()); }
        if ui.get_xf_on() && tw.get_xf_to_art() != ui.get_xf_to_art() { tw.set_xf_to_art(ui.get_xf_to_art()); }
        tw.set_show_progress(ui.get_widget_progress()); tw.set_big_art(ui.get_widget_big_art());
        let design = ui.get_tb_design();
        if tw.get_design() != design {
            tw.set_design(design.clone());
            // (a new width: placed again)
            if *shown { let _ = tw.hide(); *shown = false; }
        }
        let (ca, cb) = { let a = shared.borrow(); (look::Rgb::parse(&a.accent.0), look::Rgb::parse(&a.accent.1).or(look::Rgb::parse(&a.accent.0))) };
        if let Some(c) = ca { tw.set_art_a(c.slint()); }
        if let Some(c) = cb { tw.set_art_b(c.slint()); }
        // (its visualiser: the app's bands, as they're updated)
        let viz = if ui.get_widget_viz() || design == "viz" { ui.get_viz_style() } else { "off".into() };
        if tw.get_viz() != viz { tw.set_viz(viz); }
        if tw.get_viz_bands() != ui.get_viz_bands() { tw.set_viz_bands(ui.get_viz_bands()); tw.set_viz_peaks(ui.get_viz_peaks()); tw.set_viz_curve(ui.get_viz_curve()); }
        // where: by the clock, over the taskbar (put back over it each time, as it comes up over us when clicked)
        let scale = if *hwnd != 0 { tw.window().scale_factor() } else { ui.window().scale_factor() };
        // (as wide as its design needs)
        let wide = match design.as_str() { "compact" => 96.0, "bigtext" => 290.0, "ticker" => 320.0, _ => 340.0 };
        let spot = desktop::dock::spot((wide * scale) as f32 as i32, scale).filter(|_| !desktop::dock::busy());
        match spot {
            None => { if *shown { let _ = tw.hide(); *shown = false; } }
            Some((x, y, wd, ht)) => {
                tw.set_bar_h(ht as f32 / scale.max(0.5));
                if !*shown {
                    tw.window().set_size(slint::PhysicalSize::new(wd as u32, ht as u32));
                    tw.window().set_position(slint::PhysicalPosition::new(x, y));
                    let _ = tw.show();
                    *shown = true;
                }
                // (its handle: there once the window's been made, a moment after it's first shown)
                if *hwnd == 0 {
                    {
                        use slint::winit_030::winit::raw_window_handle::{HasWindowHandle, RawWindowHandle};
                        use slint::winit_030::WinitWindowAccessor;
                        *hwnd = tw.window().with_winit_window(|win| match win.window_handle().map(|h| h.as_raw()) {
                            Ok(RawWindowHandle::Win32(h)) => Some(h.hwnd.get()),
                            _ => None,
                        }).flatten().unwrap_or(0);
                        if *hwnd != 0 {
                            // (no taskbar button of its own: hidden and shown again with its new style)
                            desktop::dock::style(*hwnd, tw.get_glass());
                            let _ = tw.hide();
                            let _ = tw.show();
                        }
                    }
                }
                if *hwnd != 0 { desktop::dock::keep(*hwnd, (x, y, wd, ht)); }
            }
        }
    });
}

/// The window moved (HTCAPTION) or resized (an edge's hit-test code) by Windows, as from its own title bar
/// or frame. Not winit's drag_window: it marks a drag begun and clears that only when Windows' move ends,
/// so a press whose move never began (the button up before it, as when another window took the focus
/// mid-click: the lyrics widget's pop-out preview opening) left it marked, and every move and resize after
/// was ignored
fn os_drag(ui: &AppWindow, ht: usize) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::{HWND, LPARAM, POINT, WPARAM};
        use windows::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows::Win32::UI::WindowsAndMessaging::{GetCursorPos, PostMessageW, WM_NCLBUTTONDOWN};
        let Some(h) = window_hwnd(ui) else { return };
        unsafe {
            let mut p = POINT::default();
            if GetCursorPos(&mut p).is_err() { return; }
            let _ = ReleaseCapture();
            let lp = ((p.y as i16 as u16 as u32) << 16) | (p.x as i16 as u16 as u32);
            let _ = PostMessageW(Some(HWND(h as *mut _)), WM_NCLBUTTONDOWN, WPARAM(ht), LPARAM(lp as isize));
            // (Windows' move or resize takes the button's release for itself: the press let go of here, or
            // the title bar or edge stays held, its cursor kept and every click after going to it)
            let scale = ui.window().scale_factor().max(0.1);
            let at = ui.window().position();
            let pos = slint::LogicalPosition::new((p.x - at.x) as f32 / scale, (p.y - at.y) as f32 / scale);
            let weak = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: pos, button: slint::platform::PointerEventButton::Left });
                }
            });
        }
    }
    #[cfg(not(windows))]
    {
        use slint::winit_030::WinitWindowAccessor;
        if ht == 2 { ui.window().with_winit_window(|w| { let _ = w.drag_window(); }); }
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

thread_local! {
    /// a change of accent under way: from, to (the accent, the second, the text on it), and since when
    static ACCENT_FADE: RefCell<Option<([slint::Color; 3], [slint::Color; 3], std::time::Instant)>> = const { RefCell::new(None) };
    static ACCENT_TIMER: slint::Timer = slint::Timer::default();
}

/// The accents changed (picked, or a new cover's): faded from the ones shown over 0.9 s, as the web
/// app's (its CSS transition); at once with Animations off
fn accent_fade(ui: &AppWindow, c1: slint::Color, c2: slint::Color, on: slint::Color, motion: bool) {
    let t = ui.global::<Theme>();
    let to = [c1, c2, on];
    let now = [t.get_accent_own(), t.get_accent_2_own(), t.get_on_accent()];
    let target_now = ACCENT_FADE.with(|f| f.borrow().as_ref().map(|x| x.1));
    if target_now == Some(to) || (target_now.is_none() && now == to) { return; }
    if !motion {
        ACCENT_FADE.with(|f| f.borrow_mut().take());
        t.set_accent_own(c1); t.set_accent_2_own(c2); t.set_on_accent(on);
        aurora_bake(ui);
        return;
    }
    ACCENT_FADE.with(|f| *f.borrow_mut() = Some((now, to, std::time::Instant::now())));
    // (Aurora's background: its glows drawn as they fade, then painted again once they're done)
    t.set_aurora_baked(false);
    let weak = ui.as_weak();
    ACCENT_TIMER.with(|timer| timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(30), move || {
        let Some(ui) = weak.upgrade() else { return };
        let Some((from, to, at)) = ACCENT_FADE.with(|f| *f.borrow()) else { return ACCENT_TIMER.with(|t| t.stop()) };
        let x = (at.elapsed().as_secs_f32() / 0.9).min(1.0);
        // (ease, as CSS's)
        let k = x * x * (3.0 - 2.0 * x);
        let mix = |a: slint::Color, b: slint::Color| {
            let l = |p: u8, q: u8| (p as f32 + (q as f32 - p as f32) * k).round() as u8;
            slint::Color::from_argb_u8(l(a.alpha(), b.alpha()), l(a.red(), b.red()), l(a.green(), b.green()), l(a.blue(), b.blue()))
        };
        let t = ui.global::<Theme>();
        t.set_accent_own(mix(from[0], to[0]));
        t.set_accent_2_own(mix(from[1], to[1]));
        // (the text on the accent: dark or light, switched halfway)
        t.set_on_accent(if k < 0.5 { from[2] } else { to[2] });
        if x >= 1.0 { ACCENT_FADE.with(|f| f.borrow_mut().take()); ACCENT_TIMER.with(|t| t.stop()); aurora_bake(&ui); }
    }));
}

/// The Custom theme's headings' fonts: each one Windows has, as Slint names it, and as it's shown ("": the
/// app's own)
const THEME_FONTS: [(&str, &str); 11] = [
    ("", "OmniPulse's Own"), ("Segoe UI Variable Display", "Segoe UI Display"), ("Bahnschrift", "Bahnschrift"), ("Georgia", "Georgia"),
    ("Palatino Linotype", "Palatino"), ("Cambria", "Cambria"), ("Consolas", "Consolas"), ("Cascadia Code", "Cascadia Code"),
    ("Trebuchet MS", "Trebuchet"), ("Segoe Print", "Segoe Print"), ("Impact", "Impact"),
];

/// Aurora's background painted once (Theme.aurora-bake): its colour, and its two glows as the page draws them
/// (each a circle fading from its colour to nothing at 70% of the way to its box's corner), at the window's
/// logical size; again when the colours or the size change
fn aurora_bake(ui: &AppWindow) {
    let t = ui.global::<Theme>();
    if !t.get_aurora() { if t.get_aurora_baked() { t.set_aurora_baked(false); t.set_aurora_bake(slint::Image::default()); } return; }
    let size = ui.window().size().to_logical(ui.window().scale_factor());
    let (w, h) = (size.width.round().max(1.0) as u32, size.height.round().max(1.0) as u32);
    let key = (w, h, t.get_bg(), t.get_accent_own(), t.get_accent_2_own());
    thread_local! { static LAST: RefCell<Option<(u32, u32, slint::Color, slint::Color, slint::Color)>> = const { RefCell::new(None) }; }
    if LAST.with(|l| *l.borrow() == Some(key)) && t.get_aurora_baked() { return; }
    LAST.with(|l| *l.borrow_mut() = Some(key));
    let (bg, a, b) = (key.2, key.3, key.4);
    let (fw, fh) = (w as f32, h as f32);
    // (each glow: its box's centre and half-diagonal, its colour, how strong it is at its middle)
    let glows = [((0.25 * fw, 0.15 * fh), 0.5 * (0.81 * fw * fw + 0.81 * fh * fh).sqrt(), a, 0.22f32), ((0.8 * fw, 0.75 * fh), 0.5 * (0.81 * fw * fw + 0.81 * fh * fh).sqrt(), b, 0.2f32)];
    let mut px = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(w, h);
    let buf = px.make_mut_slice();
    for y in 0..h {
        for x in 0..w {
            let (mut r, mut g, mut bl) = (bg.red() as f32, bg.green() as f32, bg.blue() as f32);
            for ((cx, cy), rad, c, k) in glows {
                let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt() / rad;
                let al = if d >= 0.7 { 0.0 } else { k * (1.0 - d / 0.7) };
                if al > 0.0 { r += (c.red() as f32 - r) * al; g += (c.green() as f32 - g) * al; bl += (c.blue() as f32 - bl) * al; }
            }
            buf[(y * w + x) as usize] = slint::Rgb8Pixel { r: r as u8, g: g as u8, b: bl as u8 };
        }
    }
    t.set_aurora_bake(slint::Image::from_rgb8(px));
    t.set_aurora_baked(true);
}

/// The accents as your theme has them, or as you picked them: never the album art's
fn theme_rgb(prefs: &serde_json::Value) -> (look::Rgb, look::Rgb) {
    if let Some(t) = look::preset(prefs).filter(|_| prefs["themeOwnAccents"].as_bool() != Some(false)) { return t.accents(); }
    let mine = look::Rgb::parse(prefs["accent"].as_str().unwrap_or(ACCENTS[0])).unwrap_or(look::Rgb(0x8e, 0x9b, 0xff));
    (mine, prefs["accent2"].as_str().and_then(look::Rgb::parse).unwrap_or(mine))
}

fn apply_look(shared: &Shared) {
    let a = shared.borrow();
    let Some(ui) = a.ui.upgrade() else { return };
    let prefs = &a.web.prefs;
    let (step, hue, strength) = look::from_prefs(prefs);
    // a theme of its own (Neon, unless one's picked), else Custom: the step and tint
    let preset = look::preset(prefs);
    let p = preset.map(|t| t.palette()).unwrap_or_else(|| look::palette(step, hue, strength));
    let t = ui.global::<Theme>();
    // (a theme's own headings' font; Custom's, the one you picked)
    let own = prefs["themeFont"].as_str().unwrap_or("");
    let font = preset.map(|t| t.font).unwrap_or_else(|| THEME_FONTS.iter().find(|f| f.0 == own).map_or("", |f| f.0));
    t.set_head_font(font.into());
    ui.set_theme_font_at(THEME_FONTS.iter().position(|f| f.0 == own).unwrap_or(0) as i32);
    // (Performance Mode: no glow or aurora behind things, no motion, below)
    let perf = prefs["perfMode"].as_bool() == Some(true);
    ui.set_perf_on(perf);
    t.set_glow(!perf && preset.is_some_and(|t| t.glow));
    t.set_aurora(!perf && preset.is_some_and(|t| t.aurora));
    t.set_round_art(preset.is_some_and(|t| t.round_art));
    // Settings > App > Rounded Corners (the web app's setting too)
    let rounded = prefs["appRounded"].as_bool() != Some(false);
    t.set_panel_radius(if rounded { 10.0 } else { 0.0 });
    // (and the player's, and the cards': square everywhere with it off)
    t.set_radius(if rounded { 10.0 } else { 0.0 });
    t.set_card_radius(if rounded { 18.0 } else { 0.0 });
    ui.set_rounded(rounded);
    // Settings > Look > List Density (the web app's: rows 4 px apart, covers 32 px, at Compact)
    let compact = prefs["density"].as_str() == Some("compact");
    t.set_row_h(if compact { 40.0 } else { 60.0 });
    t.set_row_art(if compact { 32.0 } else { 44.0 });
    ui.set_compact_rows(compact);
    // Settings > App: Animations, Close to Tray
    let anim = prefs["animOn"].as_bool() != Some(false);
    let motion = anim && !perf;
    t.set_motion(motion);
    ui.set_anim_on(anim);
    ui.set_close_to_tray(min_to_tray(prefs));
    ui.set_taskbar_widget(prefs["taskbarWidget"].as_bool() == Some(true));
    ui.set_widget_progress(prefs["widgetProgress"].as_bool() != Some(false));
    ui.set_widget_viz(prefs["widgetViz"].as_bool() == Some(true));
    ui.set_widget_big_art(prefs["widgetBigArt"].as_bool() != Some(false));
    ui.set_tb_design(prefs["tbDesign"].as_str().unwrap_or("standard").into());
    ui.set_gpu_on(prefs["nativeGpu"].as_bool() == Some(true));
    LISTEN_NUDGE.store(nudge_of(prefs), std::sync::atomic::Ordering::Relaxed);
    let nudge = nudge_of(prefs).to_string();
    if ui.get_listen_nudge().trim() != nudge && !(ui.get_listen_nudge().trim().is_empty() && nudge == "0") { ui.set_listen_nudge(nudge.into()); }
    let name = prefs["appName"].as_str().unwrap_or("").trim().to_string();
    ui.set_title_prompt(name.is_empty() && plat_on(prefs, "soundcloud") && !omnipulse_core::soundcloud::signed_in());
    if ui.get_app_name().trim() != name { ui.set_app_name(name.into()); }
    ui.set_side_collapsed(prefs["sideCollapsed"].as_bool() == Some(true));
    t.set_bg(p.bg.slint());
    // (Windows' own frame and caption in the theme's background: what shows for a moment as the window's
    // resized, before it's drawn again, a white strip otherwise)
    #[cfg(windows)]
    if let Some(h) = window_hwnd(&ui) {
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR, DWMWA_CAPTION_COLOR, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_DONOTROUND, DWMWCP_ROUND, DWM_WINDOW_CORNER_PREFERENCE};
        let colorref: u32 = p.bg.0 as u32 | ((p.bg.1 as u32) << 8) | ((p.bg.2 as u32) << 16);
        // (the window's own corners: round with Rounded Corners on, as Windows 11 rounds its windows; square off)
        let corners: DWM_WINDOW_CORNER_PREFERENCE = if prefs["appRounded"].as_bool() != Some(false) { DWMWCP_ROUND } else { DWMWCP_DONOTROUND };
        unsafe {
            let hw = windows::Win32::Foundation::HWND(h as *mut _);
            let _ = DwmSetWindowAttribute(hw, DWMWA_CAPTION_COLOR, &colorref as *const u32 as _, 4);
            let _ = DwmSetWindowAttribute(hw, DWMWA_BORDER_COLOR, &colorref as *const u32 as _, 4);
            let _ = DwmSetWindowAttribute(hw, DWMWA_WINDOW_CORNER_PREFERENCE, &corners as *const DWM_WINDOW_CORNER_PREFERENCE as _, std::mem::size_of::<DWM_WINDOW_CORNER_PREFERENCE>() as u32);
        }
    }
    // (Aurora: its panels see-through, over the glow behind them)
    // (only with Hardware Acceleration on: the software renderer's see-through panels showed seams and square
    // corners, and scrolled slowly)
    let gpu = prefs["nativeGpu"].as_bool() == Some(true);
    let see = |c: look::Rgb, a: u8| if gpu && preset.is_some_and(|t| t.aurora) { slint::Color::from_argb_u8(a, c.0, c.1, c.2) } else { c.slint() };
    t.set_panel(see(p.panel, 190));
    // (the accents' blends go into the theme without its tint; a theme of its own's, into its panel)
    t.set_plain_panel(if preset.is_some() { p.panel.slint() } else { look::palette(step, hue, 0).panel.slint() });
    t.set_elevated(see(p.elevated, 215));
    t.set_menu(p.elevated.slint());
    t.set_line(p.line.slint());
    t.set_text(p.text.slint());
    t.set_muted(p.muted.slint());
    // the accent: the album art's while something plays (if asked for), or the one picked
    let art = integrations::set(prefs, "artAccent") && a.loaded && !ART_PAUSED.with(|p| p.get());
    let (c1, c2) = if art {
        let first = look::Rgb::parse(&a.accent.0);
        let c1 = first.map(|c| look::tune_art(c, p.light));
        // (one colour on the cover: a shade of it, as the web app's)
        let c2 = look::Rgb::parse(&a.accent.1).map(|c| look::tune_art(c, p.light)).or(first.map(|c| look::tune_art_shade(c, p.light)));
        (c1, c2)
    } else if let Some(t) = preset.filter(|_| prefs["themeOwnAccents"].as_bool() != Some(false)) {
        // (a theme of its own: its accents, unless you've turned them off for yours)
        let (a, b) = t.accents();
        (Some(a), Some(b))
    } else {
        let mine = look::Rgb::parse(prefs["accent"].as_str().unwrap_or(ACCENTS[0]));
        (mine, prefs["accent2"].as_str().and_then(look::Rgb::parse).or(mine))
    };
    let c1 = c1.unwrap_or(look::Rgb(0x8e, 0x9b, 0xff));
    // (the docks in OBS follow it: its colours, font and corners; the theme's accent or yours, never the album art's)
    omnipulse_core::docks::app_theme_set(serde_json::json!({ "bg": p.bg.hex(), "panel": p.panel.hex(), "line": p.line.hex(), "text": p.text.hex(), "muted": p.muted.hex(),
        "accent": theme_rgb(prefs).0.hex(), "light": p.light, "font": font, "rounded": rounded }));
    accent_fade(&ui, c1.slint(), c2.unwrap_or(c1).slint(), look::on_accent(c1).slint(), motion);
    // (the Suite page's tiles: each its own colour, from the theme's)
    let tints: Vec<slint::Color> = look::tints(c1, p.light).into_iter().map(|c| c.slint()).collect();
    t.set_tints(ModelRc::from(Rc::new(VecModel::from(tints))));
    ui.set_light_theme(p.light);
    // and Settings > Look shows where things are
    ui.set_theme_step(step as f32);
    ui.set_tint_hue(hue);
    ui.set_tint_strength(strength as f32);
    ui.set_theme_step_name(look::STEPS[step].0.into());
    ui.set_tint_strength_name(look::STRENGTHS[strength].into());
    // (the tint's hues: a picture a pixel a degree)
    if ui.get_tint_strip().size().width == 0 {
        let mut px = slint::SharedPixelBuffer::<slint::Rgb8Pixel>::new(360, 1);
        for (h, p) in px.make_mut_slice().iter_mut().enumerate() { let c = look::hsl(h as f32, 0.7, 58.0); *p = slint::Rgb8Pixel { r: c.0, g: c.1, b: c.2 }; }
        ui.set_tint_strip(slint::Image::from_rgb8(px));
    }
    ui.set_tint_preview(if strength > 0 { look::hsl(hue, 0.7, if p.light { 55.0 } else { 60.0 }).slint() } else { slint::Color::from_argb_u8(0, 0, 0, 0) });
    ui.set_accent_list(ModelRc::from(Rc::new(VecModel::from(ACCENTS.iter().filter_map(|c| look::Rgb::parse(c)).map(|c| c.slint()).collect::<Vec<_>>()))));
    // the themes to pick, three to a row: its own ones, then Custom (as the step and tint are set now)
    let custom = look::palette(step, hue, strength);
    let ca = look::Rgb::parse(prefs["accent"].as_str().unwrap_or(ACCENTS[0])).unwrap_or(look::Rgb(0x8e, 0x9b, 0xff));
    let mut opts: Vec<ThemeOpt> = look::PRESETS.iter().map(|t| { let (a, b) = t.accents(); let q = t.palette(); ThemeOpt { id: t.id.into(), name: t.name.into(), bg: q.bg.slint(), panel: q.panel.slint(), text: q.text.slint(), a: a.slint(), b: b.slint() } }).collect();
    opts.push(ThemeOpt { id: "custom".into(), name: "Custom".into(), bg: custom.bg.slint(), panel: custom.panel.slint(), text: custom.text.slint(), a: ca.slint(), b: prefs["accent2"].as_str().and_then(look::Rgb::parse).unwrap_or(ca).slint() });
    let rows: Vec<ThemeRow> = opts.chunks(3).map(|c| ThemeRow { items: ModelRc::from(Rc::new(VecModel::from(c.to_vec()))) }).collect();
    ui.set_theme_rows(ModelRc::from(Rc::new(VecModel::from(rows))));
    ui.set_theme_name(preset.map(|t| t.id).unwrap_or("custom").into());
    ui.set_theme_own_accents(prefs["themeOwnAccents"].as_bool() != Some(false));
    // (your own colours, not among the ones offered: a swatch of their own)
    let mine = |k: &str| prefs[k].as_str().map(|c| c.to_lowercase()).filter(|c| !ACCENTS.contains(&c.as_str())).and_then(|c| look::Rgb::parse(&c));
    ui.set_accent_mine_on(mine("accent").is_some());
    ui.set_accent_mine(mine("accent").unwrap_or(look::Rgb(0, 0, 0)).slint());
    ui.set_accent2_mine_on(mine("accent2").is_some());
    ui.set_accent2_mine(mine("accent2").unwrap_or(look::Rgb(0, 0, 0)).slint());
    let picked = prefs["accent"].as_str().unwrap_or(ACCENTS[0]).to_lowercase();
    ui.set_accent_on(ACCENTS.iter().position(|c| *c == picked).map_or(-1, |i| i as i32));
    let picked2 = prefs["accent2"].as_str().unwrap_or("").to_lowercase();
    ui.set_accent2_on(ACCENTS.iter().position(|c| *c == picked2).map_or(-1, |i| i as i32));
    ui.set_art_accent(integrations::set(prefs, "artAccent"));
    ui.set_tab_right(prefs["tabAlign"].as_str() == Some("right"));
    ui.set_player_style(prefs["playerStyle"].as_str().filter(|s| matches!(*s, "bar" | "float" | "minimal" | "turntable" | "island" | "sidebar" | "decks" | "waveform" | "strip")).unwrap_or("tab").into());
    ui.set_beta_updates(prefs["betaUpdates"].as_bool().unwrap_or(false));
    ui.set_title_colour(prefs["titleColour"].as_str().filter(|s| matches!(*s, "subtle" | "plain")).unwrap_or("match").into());
    // (Aurora's background, painted again for its colours)
    aurora_bake(&ui);
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
    omnipulse_core::crash_log_install();
    omnipulse_core::diag::install();
    // one OmniPulse at a time: a second one hands its link (or "show") to the first, and closes
    let link_arg = std::env::args().find(|x| x.starts_with("omnipulse://"));
    if std::env::var_os("OMNIPULSE_RESTARTING").is_some() { instance::wait_for_the_last(); }
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
    // (a self-test leaves your Spotify alone: no session of its own, but for the Spotify test)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() && std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() != Some("spotify") && std::env::var_os("OMNIPULSE_SELFTEST_SPOTIFY").is_none() {
        std::env::set_var("OMNIPULSE_NO_SPOTIFY", "1");
    }
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
    // Slint's software renderer to start with: measured at 38 MB and ~3% of a core while playing,
    // against ~100 MB with the GPU one (most of it the graphics driver). SLINT_BACKEND picks another.
    // (Settings > App > Hardware Acceleration: the GPU one, from the next start)
    if std::env::var_os("SLINT_BACKEND").is_none() {
        let gpu = webstate::native_pref("nativeGpu").and_then(|v| v.as_bool()) == Some(true);
        let _ = slint::BackendSelector::new().backend_name("winit".into()).renderer_name(if gpu { "femtovg" } else { "software" }.into()).select();
    }
    let ui = AppWindow::new()?;
    let engine = Arc::new(Engine::new("").map_err(slint::PlatformError::Other)?);
    let events = engine.events().expect("events");
    // (the volume you left it at; 80% the first time)
    let start_vol = webstate::native_pref("nativeVolume").and_then(|v| v.as_f64()).filter(|v| v.is_finite()).map_or(80.0, |v| v.clamp(0.0, 100.0));
    engine.set_volume((start_vol / 100.0) as f32);
    ui.set_volume(start_vol as f32);
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
        autoplay: true, ap_busy: false, ap_then_play: false, ap_failed: None, resume_ms: None, resume_key: String::new(), loaded: false, accent: ("#8e9bff".into(), "#8e9bff".into()), started_secs: 0, token: 0,
        lyrics: Vec::new(), heard: (String::new(), 0.0, false), last_pos: 0, xpl: xpl::load(), naming: None,
        loud: HashMap::new(), eq_model: Rc::new(VecModel::from(vec![0.0f32; 10])), add_map: Vec::new(), queue_spaced: false, analysis: HashMap::new(), cur_loud: None,
    }));
    // (before anything is listed: Go+ tracks only while YouTube Music can play them)
    go_plus_set(&shared.borrow().web.prefs);

    SHARED.with(|s| *s.borrow_mut() = Some(shared.clone()));
    // the plugins that are on, each started on its own thread
    // the plugins that come with OmniPulse (beside it, in plugins/): installed, or brought up to date
    // (not from the self-test: a test build never replaces your installed ones)
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("plugins"))).filter(|d| d.is_dir() && std::env::var_os("OMNIPULSE_SELFTEST").is_none()) {
        omnipulse_core::plugins::install_bundled(&dir);
    }
    plugins_start(&shared);
    // OmniPulse Offset, if it's on (not in the self-test: it would play)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { offset_apply(&shared); }
    ui.set_page_sub("Loading your Library...".into());
    restore_session(&shared);
    apply_look(&shared);
    // Match Loudness for the Spotify songs Spotify plays: its own levelling, at the same target
    { let (on, target) = sound::spotify_norm(&shared.borrow().web.prefs); omnipulse_core::spotify_engine::spe_set_normalisation(on, target); }
    let s = shared.clone();
    // (the Custom theme's headings' font: the choices, and one picked)
    ui.set_theme_fonts(ModelRc::from(Rc::new(VecModel::from(THEME_FONTS.iter().map(|f| slint::SharedString::from(f.1)).collect::<Vec<_>>()))));
    let s2 = shared.clone();
    ui.on_theme_font_picked(move |k| {
        if let Some(f) = THEME_FONTS.get(k.max(0) as usize) { set_look_pref(&s2, "themeFont", serde_json::json!(f.0)); }
    });
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
    // a theme picked (Custom: the step and tint); its own colours, or yours
    ui.on_theme_picked(move |id| set_look_pref(&s, "themeName", serde_json::json!(id.as_str())));
    let s = shared.clone();
    ui.on_theme_own_accents_changed(move |on| set_look_pref(&s, "themeOwnAccents", serde_json::json!(on)));
    let s = shared.clone();
    // your own accent colour: Windows' colour picker, from the one there now
    ui.on_accent_custom(move |which| {
        let key = if which == 2 { "accent2" } else { "accent" };
        let now = s.borrow().web.prefs[key].as_str().or(s.borrow().web.prefs["accent"].as_str()).unwrap_or(ACCENTS[0]).to_string();
        desktop::pick_color_then(&now, move |c| SHARED.with(|s| if let Some(sh) = s.borrow().clone() { set_look_pref(&sh, key, serde_json::json!(c.to_lowercase())); }));
    });
    let s = shared.clone();
    // the second accent (null: one colour, as the web app's setAccent2)
    ui.on_accent2_picked(move |i| {
        let v = ACCENTS.get(i.max(0) as usize).filter(|_| i >= 0).map_or(serde_json::Value::Null, |c| serde_json::json!(c));
        set_look_pref(&s, "accent2", v);
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
        let s = s.clone();
        ask_then(rfd::FileDialog::new().set_title("Add a music folder"), Ask::Folder, move |dir| {
            let mut folders = omnipulse_core::local::local_folders();
            folders.push(dir.to_string_lossy().into_owned());
            folders_set(&s, folders);
        });
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
    // (the EQ's box resized: its curve made again to fit)
    ui.on_eq_shape(move |a| {
        if (EQ_ASPECT.with(|x| x.get()) - a).abs() < 0.001 || a <= 0.0 { return; }
        EQ_ASPECT.with(|x| x.set(a));
        let (line, fill) = eq_curve(&sound::eq_bands(&s.borrow().web.prefs));
        if let Some(ui) = s.borrow().ui.upgrade() { ui.set_eq_curve(line.into()); ui.set_eq_curve_fill(fill.into()); }
    });
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
    // (the first-run setup: a new install, as soon as the window's up)
    {
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_millis(600), move || {
            let wanted = setup_wanted(&s.borrow());
            if wanted { setup_show(&s, 0); }
            // (one set up already: never shown it, on updating to a version with it)
            else if s.borrow().web.prefs["firstRunDone"].is_null() && std::env::var_os("OMNIPULSE_SELFTEST").is_none() { pref_put(&s, "firstRunDone", serde_json::json!(true)); }
        });
    }
    // a newer version: asked about a little after starting (not in the self-test)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_none() {
        // (and the Stream Deck plugin, if it's older than the one that came with the app: once per version)
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(12), move || {
            let offered = s.borrow().web.prefs["sdOffered"].as_str().unwrap_or("").to_string();
            if let Some(v) = streamdeck::update_on_start(&offered) {
                pref_put(&s, "sdOffered", serde_json::json!(v));
                if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status("The Stream Deck plugin has an update. Confirm it in Stream Deck.".into()); }
            }
        });
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(20), move || update_check(&s, false));
        // (and the Seamless Mix ratings made since the last launch, sent once: with Send My Ratings on)
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(30), move || {
            if !set_row_on(&s.borrow().web.prefs, "shareRatings", true) { return; }
            if let Ok(d) = omnipulse_core::data_dir() { mix_ratings_send(&s, d.join("state")); }
        });
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
    ui.on_follow_click(move || follow_click(&s));
    let s = shared.clone();
    ui.on_follow_more_click(move || follow_more_click(&s));
    let s = shared.clone();
    ui.on_side_collapse(move |on| {
        let v = serde_json::json!(on);
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("sideCollapsed".into(), v.clone()); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("sideCollapsed", v); }
    });
    let s = shared.clone();
    ui.on_nav_hide(move |id| {
        let key = match id.as_str() { "navFavourites" => "favPage", "navSets" => "setsOn", _ => "statsShown" };
        // (as switching it off in Settings)
        if set_row_on(&s.borrow().web.prefs, key, true) { set_toggle(&s, key); }
    });
    // (saved as it's typed; the box keeps what's typed, the title bar and taskbar follow it)
    let s = shared.clone();
    ui.on_app_name_changed(move |name| {
        let v = serde_json::json!(name.trim());
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("appName".into(), v.clone()); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("appName", v); }
    });
    let s = shared.clone();
    ui.on_listen_nudge_changed(move |text| {
        let n = text.trim().parse::<f64>().map(|n| n.round() as i64).unwrap_or(0).clamp(-3000, 8000);
        LISTEN_NUDGE.store(n, std::sync::atomic::Ordering::Relaxed);
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("listenNudge".into(), serde_json::json!(n)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("listenNudge", serde_json::json!(n)); }
    });
    let s = shared.clone();
    ui.on_sets_liked_changed(move |on| {
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("setsLiked".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("setsLiked", serde_json::json!(on)); }
        open_page(&s, "sets");
    });
    // Seamless Mix's button (the same as its switch)
    // (the title bar's Seamless Mix: Smart Transitions, which replaced the first Seamless Mix)
    ui.set_mix_on(smart_on(&shared.borrow().web.prefs));
    ui.set_mix_available(true);
    let s = shared.clone();
    ui.on_mix_toggle(move || set_toggle(&s, "smartOn"));
    let s = shared.clone();
    ui.on_setup_go(move |how| setup_go(&s, &how));
    let s = shared.clone();
    ui.on_tb_design_picked(move |d| set_look_pref(&s, "tbDesign", serde_json::json!(d.as_str())));
    // (Cover Strip: each cover, blurred, as it's shown)
    let w = ui.as_weak();
    // (the blurred cover: Cover Strip's, and Halo's behind it full screen)
    ui.on_now_art_shown(move || if let Some(ui) = w.upgrade() {
        if ui.get_player_style() == "strip" || ui.get_viz_style() == "halo" { art_blur(&ui); }
        // (a GPU scene's cover: the new one)
        // (only once there's a picture: none yet, the old one stays rather than going black)
        if ui.get_viz_scene() { if let Some(art) = gpu_art(&ui) { GPU_VIS.with(|g| if let Some(g) = g.borrow().as_ref() { g.set_art(Some(art)); }); } }
        // (and the one in OBS)
        if omnipulse_core::widget::vis_wanted() { vis_art(&ui); }
    });
    let s = shared.clone();
    // (Beta Updates: kept, and checked again on that channel at once)
    ui.on_beta_updates_set(move |on| {
        set_look_pref(&s, "betaUpdates", serde_json::json!(on));
        if let Some(ui) = s.borrow().ui.upgrade() { ui.set_beta_updates(on); }
        update_check(&s, false);
    });
    let s = shared.clone();
    ui.on_mix_rate(move |good| if let Err(e) = mix_feedback(&s, good) { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(e.into()); } });
    // (Now Playing: the lyrics or the description, as last picked)
    ui.set_now_pane(if shared.borrow().web.prefs["nowPane"].as_str() == Some("desc") { "desc".into() } else { "lyrics".into() });
    let s = shared.clone();
    ui.on_now_pane_picked(move |p| {
        let v = serde_json::json!(p.as_str());
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("nowPane".into(), v.clone()); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("nowPane", v); }
    });
    // (the visualiser's style picked in App & Look, and where it's shown; full screen isn't kept)
    let s = shared.clone();
    ui.on_viz_pick(move |style| {
        let v = serde_json::json!(style.as_str());
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("nowViz".into(), v.clone()); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("nowViz", v); }
        if let Some(ui) = s.borrow().ui.upgrade() { viz_update(&s, &ui); if style == "halo" { art_blur(&ui); } }
    });
    // (full screen: the song's details shown or hidden, kept)
    ui.set_viz_full_info(shared.borrow().web.prefs["vizFullInfo"].as_bool() != Some(false));
    let s = shared.clone();
    ui.on_viz_info(move |on| {
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("vizFullInfo".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("vizFullInfo", serde_json::json!(on)); }
    });
    // (its colours: yours, or a palette of its own)
    ui.global::<VizLook>().set_palette(shared.borrow().web.prefs["vizPalette"].as_str().unwrap_or("theme").into());
    let s = shared.clone();
    ui.on_viz_palette(move |pal| {
        let v = serde_json::json!(pal.as_str());
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("vizPalette".into(), v.clone()); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("vizPalette", v); }
        if let Some(ui) = s.borrow().ui.upgrade() { ui.global::<VizLook>().set_palette(pal); }
    });
    let s = shared.clone();
    ui.on_viz_place(move |place, on| {
        let key = match place.as_str() { "now" => Some("vizNow"), "bar" => Some("vizBar"), "game" => Some("vizGamePause"), _ => None };
        if let Some(k) = key {
            if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert(k.into(), serde_json::json!(on)); }
            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref(k, serde_json::json!(on)); }
        }
        if let Some(ui) = s.borrow().ui.upgrade() { viz_update(&s, &ui); }
    });
    // (Now Playing's visualiser: on to the next style, kept)
    let s = shared.clone();
    ui.on_viz_cycle(move || {
        let now = s.borrow().web.prefs["nowViz"].as_str().unwrap_or("off").to_string();
        let gpu = s.borrow().web.prefs["nativeGpu"].as_bool() == Some(true);
        let next = match now.as_str() {
            "halo" if gpu => "smoke", "smoke" => "swarm", "swarm" => "ink", "ink" => "off", "off" => "bars", "bars" => "mirror", "mirror" => "peaks", "peaks" => "blocks", "blocks" => "dots", "dots" => "wave", "wave" => "solid", "solid" => "centre", "centre" => "glow", "glow" => "ring", "ring" => "halo", _ => "off" };
        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("nowViz".into(), serde_json::json!(next)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("nowViz", serde_json::json!(next)); }
        if let Some(ui) = s.borrow().ui.upgrade() { viz_update(&s, &ui); }
    });
    // (right-clicked in the title bar: Seamless Mix's own page; a platform's settings, where they are)
    let s = shared.clone();
    ui.on_title_settings(move |what| {
        let Some(ui) = s.borrow().ui.upgrade() else { return };
        go(&s, Place::Page("settings".into()));
        // (whatever page of Settings was open, let go first)
        PF_EDIT.with(|e| e.borrow_mut().take());
        ui.set_pf_open(false);
        ui.set_set_query("".into());
        match what.as_str() {
            "smart" => panel_open(&s, "smart"),
            "local" => ui.set_set_cat("Library".into()),
            "youtube" => { ui.set_set_cat("Integrations".into()); panel_open(&s, "youtube"); }
            p if p.starts_with("plugin:") => { ui.set_set_cat("Plugins".into()); pf_open(&s, &p[7..]); }
            p if p.starts_with("plugin") => ui.set_set_cat("Plugins".into()),
            _ => ui.set_set_cat("Integrations".into()),
        }
    });
    let s = shared.clone();
    ui.on_close_to_tray_changed(move |on| set_look_pref(&s, "minToTray", serde_json::json!(on)));
    let s = shared.clone();
    ui.on_app_pref(move |key, on| {
        set_look_pref(&s, &key, serde_json::json!(on));
        if key == "nativeGpu" { if let Some(ui) = s.borrow().ui.upgrade() { say_with(&ui, "Hardware Acceleration changes the next time OmniPulse starts.", vec![("Restart Now", StatusAct::Restart), ("Later", StatusAct::Dismiss)]); } }
        if let Some(ui) = s.borrow().ui.upgrade() { windows_now(&s, &ui); }
    });
    let s = shared.clone();
    ui.on_anim_changed(move |on| set_look_pref(&s, "animOn", serde_json::json!(on)));
    let s = shared.clone();
    // (and the visualiser started again, at its new rate)
    ui.on_perf_changed(move |on| {
        set_look_pref(&s, "perfMode", serde_json::json!(on));
        let was = VIZ.with(|v| v.borrow_mut().take()).is_some();
        if was { let e = s.borrow().engine.clone(); e.set_tap("viz", None); }
        let ui = s.borrow().ui.upgrade();
        if let Some(ui) = ui { viz_update(&s, &ui); }
    });
    nav_paint(&shared);
    let s = shared.clone();
    ui.on_density_changed(move |c| set_look_pref(&s, "density", serde_json::json!(if c { "compact" } else { "comfortable" })));
    let s = shared.clone();
    ui.on_player_style_picked(move |style| {
        set_look_pref(&s, "playerStyle", serde_json::json!(style.as_str()));
        if style == "strip" { if let Some(ui) = s.borrow().ui.upgrade() { art_blur(&ui); } }
    });
    let s = shared.clone();
    ui.on_title_colour_picked(move |c| set_look_pref(&s, "titleColour", serde_json::json!(c.as_str())));
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
    // (Streamer Mode's right-click: only music that's safe on stream, or everything again)
    ui.on_streamer_music(move || { let on = streamsafe::on(&s.borrow().web.prefs); safe_set(&s, !on); });
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
        // (and copies of deleted playlists that couldn't be deleted then)
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { shared.borrow().rt.spawn(mirror::remove_left()); }
        // Spotify playback took the app down last time (it can, for some accounts): signed out of it, and said
        if omnipulse_core::spotify_engine::spe_crashed_last_time() {
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status("Spotify playback closed the app last time, so it's been signed out of it. A free Spotify account needs your own free app keys: Settings > Accounts.".into()); }
        }
        // hosting when the app closed (an update) and opened again soon after: picked up again
        let s = shared.clone();
        slint::Timer::single_shot(std::time::Duration::from_secs(2), move || { host_resume(&s); listen_resume(&s); });
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
        if page == "mix" { return mix_refresh(&s); }
        if page == "artists" { s.borrow_mut().artists = None; }
        if page == "discover" { return open_discover_fresh(&s); }
        open_page(&s, &page);
    });
    let s = shared.clone();
    ui.on_lyrics_pick(move |i| lyrics_pick(&s, i as usize));
    let s = shared.clone();
    ui.on_account_keys(move |i| { KS_SIGNED_IN.with(|k| k.set(true)); ks_open(&s, "spotify", i as usize); });
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
        // (a credentials file's text, or several keys, pasted in: each to its own box, as the web app's)
        let plat = KS.with(|k| k.borrow().as_ref().map(|x| x.0.clone())).unwrap_or_default();
        let found = keysetup::keys_from(&plat, &v);
        if !found.is_empty() && !(found.len() == 1 && found[0] == (i as usize, v.trim().to_string())) {
            ks_take(&s, found);
            return;
        }
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
    // (a file with the keys in it, such as Google's credentials file: read for them)
    let s = shared.clone();
    ui.on_ks_file(move || {
        let Some(plat) = KS.with(|k| k.borrow().as_ref().map(|x| x.0.clone())) else { return };
        let s = s.clone();
        ask_then(rfd::FileDialog::new().set_title("A file with your keys in it"), Ask::Open, move |path| {
            let text = std::fs::metadata(&path).ok().filter(|m| m.len() < 65536).and_then(|_| std::fs::read_to_string(&path).ok()).unwrap_or_default();
            let found = keysetup::keys_from(&plat, &text);
            if found.is_empty() {
                let name = keysetup::setup(&plat).map(|x| x.name).unwrap_or("it");
                if let Some(ui) = s.borrow().ui.upgrade() { ui.set_ks_note(format!("That file has no keys for {name} in it.").into()); }
                return;
            }
            ks_take(&s, found);
        });
    });
    let s = shared.clone();
    ui.on_account_settings(move |p| {
        // (Bugs and Ideas: its page, with the kind picked as clicked)
        if let Some(kind) = p.strip_prefix("report:") {
            if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("reportKind".into(), serde_json::json!(kind)); }
            return panel_open(&s, "report");
        }
        panel_open(&s, &p)
    });
    flows_wire(&ui, &shared);
    let s = shared.clone();
    ui.on_set_more(move |key| {
        // (from a Settings page itself, not the Suite's Widgets and Docks: its crumbs as usual)
        BOT_WIDGETS.with(|w| w.borrow_mut().clear());
        if let Some(id) = key.strip_prefix("plugOn_").or_else(|| key.strip_prefix("plugRun_")) {
            // (its page where it lives: Settings > Streaming for the chatbot, so Back goes there)
            if let Some(sec) = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id).map(|p| p.manifest.section).filter(|x| !x.is_empty()) {
                if let Some(ui) = s.borrow().ui.upgrade() { ui.set_set_cat(sec.into()); }
            }
            return pf_open(&s, id);
        }
        if let Some(name) = panels::of_switch(&key) { panel_open(&s, name); }
    });
    let s = shared.clone();
    ui.on_pf_copy(move |i| {
        let v = PANEL_FIELDS.with(|p| p.borrow().get(i as usize).and_then(|f| f["copy"].as_str().map(String::from)));
        if let Some(v) = v {
            let note = match share::copy(&v) { Ok(()) => "Copied.".to_string(), Err(e) => e };
            if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(note.into()); }
        }
    });
    let s = shared.clone();
    ui.on_pf_set(move |i, v| { pf_set(&s, i as usize, &v); pf_mark(&s); pf_paint_changed(&s); });
    let s = shared.clone();
    ui.on_pf_save(move || pf_save(&s));
    let s = shared.clone();
    ui.on_pf_leave(move |to| pf_leave(&s, &to));
    let s = shared.clone();
    ui.on_leave_choice(move |c| leave_choice(&s, &c));
    let s = shared.clone();
    ui.on_bot_set(move |k, v| bot_set(&s, &k, &v));
    let s = shared.clone();
    ui.on_bot_act(move |a| bot_act(&s, &a));
    let s = shared.clone();
    ui.on_bot_try(move |t| bot_try(&s, &t));
    let s = shared.clone();
    ui.on_copy_text(move |t| { if share::copy(&t).is_ok() { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status("Copied.".into()); } } });
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
            // cast devices on the network (found in the background: the menu has more next time it opens),
            // with Chromecast switched on (Settings > Integrations, the web app's castOn)
            let cast_on = SHARED.with(|s| s.borrow().as_ref().is_some_and(|sh| sh.borrow().web.prefs["castOn"].as_bool() == Some(true)));
            if cast_on { let _ = omnipulse_core::cast::discover_start(); }
            let cast = if cast_on { omnipulse_core::cast::devices() } else { Vec::new() };
            let on = casting();
            if let Some(i) = on.as_ref().and_then(|n| cast.iter().position(|d| d.name == *n)) { ui.set_device_on((list.len() + i) as i32); }
            // (where OmniPulse Offset plays, if it is: marked too)
            let off = if omnipulse_core::omni_input::running() { omnipulse_core::omni_input::device() } else { String::new() };
            ui.set_device_offset(if off.is_empty() { -1 } else { list.iter().position(|d| *d == off || d.starts_with(&off) || off.starts_with(d.as_str())).map_or(-1, |i| i as i32) });
            // (Settings > Sound's Output Device: Windows' own, as the web app lists them)
            ui.set_outputs(ModelRc::from(Rc::new(VecModel::from(list.iter().map(|d| slint::SharedString::from(d.as_str())).collect::<Vec<_>>()))));
            ui.set_output_at(if (ui.get_device_on() as usize) < list.len() { ui.get_device_on() } else { -1 });
            // (cast devices after Windows', under a heading of their own)
            ui.set_devices_cast_from(if cast.is_empty() { -1 } else { list.len() as i32 });
            list.extend(cast.iter().map(|d| format!("Cast to {}", d.name)));
            // (and a fresh look for them, as the web app's Search for Cast devices)
            if cast_on { list.push(CAST_SEARCH.to_string()); }
            CAST_LIST.with(|c| *c.borrow_mut() = cast);
            ui.set_devices(ModelRc::from(Rc::new(VecModel::from(list.into_iter().map(slint::SharedString::from).collect::<Vec<_>>()))));
        });
    }
    {
        let s = shared.clone();
        ui.on_cast_toggle(move || {
            set_toggle(&s, "castOn");
            if let Some(ui) = s.borrow().ui.upgrade() {
                ui.set_cast_on(s.borrow().web.prefs["castOn"].as_bool() == Some(true));
                ui.invoke_devices_want();
                // (the devices found a moment later: the list again then)
                let w = ui.as_weak();
                slint::Timer::single_shot(std::time::Duration::from_millis(2500), move || { if let Some(ui) = w.upgrade() { ui.invoke_devices_want(); } });
            }
        });
        ui.set_cast_on(shared.borrow().web.prefs["castOn"].as_bool() == Some(true));
    }
    let s = shared.clone();
    ui.on_device_pick(move |i| {
        let Some(ui) = s.borrow().ui.upgrade() else { return };
        let Some(name) = ui.get_devices().row_data(i as usize).map(|d| d.to_string()) else { return };
        if name == CAST_SEARCH {
            let _ = omnipulse_core::cast::discover_start();
            ui.set_status("Searching for Cast devices...".into());
            let weak = ui.as_weak();
            slint::Timer::single_shot(std::time::Duration::from_millis(2500), move || {
                let Some(ui) = weak.upgrade() else { return };
                ui.invoke_devices_want();
                let n = omnipulse_core::cast::devices().len();
                ui.set_status(if n == 0 { "No Cast devices found on your network.".into() } else { format!("Found {}. They're in the menu.", plural(n, "Cast device", "Cast devices")).into() });
            });
            return;
        }
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
    // (its cover: others looked for, one picked, or an image's address pasted)
    let s = shared.clone();
    ui.on_tp_cover_find(move || tp_cover_find(&s));
    let s = shared.clone();
    ui.on_tp_cover_pick(move |i| tp_cover_pick(&s, i));
    let s = shared.clone();
    ui.on_tp_cover_url(move |u| tp_cover_url(&s, &u));
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
    ui.on_tp_drag({ let s = shared.clone(); move |f, w| tp_drag(&s, f, w) });
    ui.on_tp_preview({ let s = shared.clone(); move |w| tp_preview(&s, w) });
    ui.on_tp_whole({ let s = shared.clone(); move || { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_tp_start("".into()); ui.set_tp_end("".into()); } tp_bar(&s); } });
    ui.on_tp_search({ let s = shared.clone(); move |q| {
        // (a search for other versions, as the web app's: the closest few for what's typed)
        let q = q.trim().to_string();
        if q.is_empty() || yt_link_id(&q).is_some() { return; }
        let Some(t) = TP_TRACK.with(|x| x.borrow().clone()) else { return };
        let (rt, weak) = { let a = s.borrow(); (a.rt.clone(), a.ui.clone()) };
        if let Some(ui) = weak.upgrade() { ui.set_status("Searching...".into()); }
        rt.spawn(async move {
            let mut found = omnipulse_core::youtube::search(&q).await.unwrap_or_default();
            found.truncate(8);
            let _ = weak.upgrade_in_event_loop(move |ui| {
                if found.is_empty() { ui.set_status("Nothing found on YouTube Music.".into()); return; }
                ui.set_status("".into());
                TP_VERSIONS.with(|v| *v.borrow_mut() = found.clone());
                let names: Vec<slint::SharedString> = found.iter().map(|c| format!("{} · {} · {}", c.title, c.artists, fmt_ms(c.duration_ms)).into()).collect();
                ui.set_tp_versions(ModelRc::from(Rc::new(VecModel::from(names))));
                ui.set_tp_version_on(-1);
                let _ = &t;
            });
        });
    } });
    let s = shared.clone();
    ui.on_tp_now(move |which| {
        let pos = s.borrow().engine.position_ms();
        if let Some(ui) = s.borrow().ui.upgrade() { if which == 0 { ui.set_tp_start(fmt_time(pos).into()); } else { ui.set_tp_end(fmt_time(pos).into()); } }
        tp_bar(&s);
    });
    // the track's menu
    let s = shared.clone();
    // the player bar's "..." (the web app's track menu for what's playing): each line, and what it does
    ui.on_more_want(move || {
        let a = s.borrow();
        let Some(ui) = a.ui.upgrade() else { return };
        let mut items: Vec<(String, String)> = Vec::new();
        if let Some(t) = a.queue.current().cloned() {
            let m = marks_of(&a);
            let r = row_menu(&a, &m, &t, t.row(true));
            items.push(("Play Next".into(), "next".into()));
            items.push(("Add to Queue".into(), "add".into()));
            items.push(("Add to Playlist...".into(), "playlist".into()));
            if r.can_like { items.push((if r.liked { "Remove from Likes" } else { "Add to Likes" }.into(), "like".into())); }
            items.push((if r.fav { "Remove from Favourites" } else { "Add to Favourites" }.into(), "fav".into()));
            let names: Vec<String> = t.artist.split([',', '&']).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect();
            if names.len() > 1 { for (k, n) in names.iter().enumerate() { items.push((format!("Go to {n}"), format!("artist:{k}"))); } } else { items.push(("Go to Artist".into(), "artist:0".into())); }
            if !r.uploader.is_empty() { items.push((format!("Go to {} (Uploader)", r.uploader), "uploader".into())); }
            items.push(("Search for This Song".into(), "search".into()));
            items.push(("Don't Recommend This Track".into(), "hide:track".into()));
            if !t.album.is_empty() { items.push(("Don't Recommend This Album".into(), "hide:album".into())); }
            if let Some(n) = names.first() { items.push((format!("Don't Recommend {n}"), "hide:artist".into())); }
            if !r.cleared.is_empty() { items.push((r.cleared.to_string(), "cleared".into())); }
            if !r.open_label.is_empty() { items.push((r.open_label.to_string(), "open".into())); }
            for (plat, _) in share::links(&t) { items.push((format!("Copy {} Link", share::name(plat)), format!("link:{plat}"))); }
            items.push(("Track Preferences...".into(), "prefs".into()));
        }
        MORE_ACTS.with(|m| *m.borrow_mut() = items.iter().map(|x| x.1.clone()).collect());
        ui.set_more_items(ModelRc::from(Rc::new(VecModel::from(items.into_iter().map(|x| slint::SharedString::from(x.0)).collect::<Vec<_>>()))));
    });
    let s = shared.clone();
    ui.on_more_pick(move |i| {
        let Some(act) = MORE_ACTS.with(|m| m.borrow().get(i as usize).cloned()) else { return };
        let Some(t) = s.borrow().queue.current().cloned() else { return };
        if act == "playlist" { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_pl_menu_asks(ui.get_pl_menu_asks() + 1); } return; }
        if let Some(plat) = act.strip_prefix("link:") { return copy_track_link(&s, &t, plat); }
        if let Some(what) = act.strip_prefix("hide:") { return hide_track(&s, &t, what); }
        match act.as_str() {
            "next" | "add" => {
                let mut a = s.borrow_mut();
                if act == "next" { a.queue.play_next(t.clone()); } else { a.queue.add(t.clone()); }
                paint_queue(&mut a);
                save_session(&a);
                if let Some(ui) = a.ui.upgrade() { ui.set_status(format!("{} \"{}\".", if act == "next" { "Playing next:" } else { "Added to the queue:" }, t.title).into()); }
            }
            "search" => {
                let q = format!("{} {}", t.artist.split(", ").next().unwrap_or(""), t.title);
                if let Some(ui) = s.borrow().ui.upgrade() { ui.set_search_text(q.clone().into()); }
                go(&s, Place::Search(q));
            }
            other => { track_act(&s, &t, other); }
        }
    });
    let s = shared.clone();
    ui.on_artist(move |i| go(&s, Place::Artist(i as usize)));
    // drag and drop: carried as plain text, read and acted on here
    {
        let w = ui.global::<Wid>();
        let s = shared.clone();
        w.on_on(move |id, _| {
            let cur = bot_current(&s);
            // (the hype meter's own switch from before (hypeOn) counts too: its tile's is the one now)
            if id == "hype" && cur.get("hypeOn").and_then(|v| v.as_bool()) == Some(false) { return false; }
            !cur.get("widgetsOff").and_then(|v| v.as_str()).unwrap_or("").split('|').any(|x| x.trim() == id.as_str())
        });
        let d = ui.global::<Dnd>();
        d.on_payload(|what| slint::DataTransfer::from(what));
        let s = shared.clone();
        d.on_accepts(move |target, data| {
            // (a page of the sidebar, onto another)
            if let Some(over) = target.strip_prefix("nav:") { return data.plain_text().ok().and_then(|t| t.strip_prefix("n:").map(|f| f != over)).unwrap_or(false); }
            // (a playlist to a gap: anywhere but just before or after itself)
            if let Some(to) = target.strip_prefix("sidegap:").and_then(|x| x.parse::<usize>().ok()) {
                return data.plain_text().ok().and_then(|t| t.strip_prefix("p:").and_then(|x| x.parse::<usize>().ok())).is_some_and(|from| to != from && to != from + 1);
            }
            let a = s.borrow(); dragged(&a, &data).is_some_and(|w| drop_ok(&a, &target, &w))
        });
        let s = shared.clone();
        d.on_dropped(move |target, data| {
            if let Some(over) = target.strip_prefix("nav:") {
                if let Some(from) = data.plain_text().ok().and_then(|t| t.strip_prefix("n:").map(String::from)) { nav_move(&s, &from, over); }
                return;
            }
            if let Some(to) = target.strip_prefix("sidegap:").and_then(|x| x.parse::<usize>().ok()) {
                if let Some(from) = data.plain_text().ok().and_then(|t| t.strip_prefix("p:").and_then(|x| x.parse::<usize>().ok())) { side_move(&s, from, to); }
                return;
            }
            let what = { let a = s.borrow(); dragged(&a, &data).filter(|w| drop_ok(&a, &target, w)) };
            if let (Some(w), Some(j)) = (what.clone(), target.strip_prefix("qrow:").and_then(|x| x.parse::<usize>().ok())) {
                let from = data.plain_text().ok().and_then(|t| t.strip_prefix("q:").and_then(|x| x.parse::<usize>().ok()));
                return drop_queue_row(&s, j, from, w);
            }
            if let Some(w) = what { drop_do(&s, &target, w); }
        });
    }
    let s = shared.clone();
    ui.on_side(move |i| {
        let at = s.borrow().side_map.get(i as usize).copied();
        let id = at.and_then(|at| s.borrow().playlists.get(at).map(|p| p.id.clone()));
        if let Some(id) = id { go(&s, Place::Playlist(id)); }
    });
    let s = shared.clone();
    ui.on_crumb_go(move |i| crumb_go(&s, i.max(0) as usize));
    ui.on_follow_plat({ let s = shared.clone(); move |p| follow_plat(&s, &p) });
    ui.on_album_act({ let s = shared.clone(); move |i, act| {
        let Some(f) = ALBUMS.with(|a| a.borrow().get(i.max(0) as usize).cloned()) else { return };
        let (title, by, _, _) = f.tile();
        if act == "album-hide" {
            // (as the track menu's Don't Recommend This Album: by its title and artist)
            let t = Track { album: title.clone(), artist: by.clone(), ..Default::default() };
            return hide_track(&s, &t, "album");
        }
        // Play: its tracks, from the first
        if let sources::Found::Album { plat: "local", id, .. } = &f {
            let list: Vec<Track> = s.borrow().library.iter().filter(|t| t.plat() == "local" && t.album.trim() == id).cloned().collect();
            if !list.is_empty() { play_list(&s, list, 0); }
            return;
        }
        let (rt, weak) = { let a = s.borrow(); (a.rt.clone(), a.ui.clone()) };
        rt.spawn(async move {
            let list = sources::found_tracks(&f).await;
            let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|x| { if let Some(sh) = x.borrow().clone() { if !list.is_empty() { play_list(&sh, list, 0); } } }));
        });
    } });
    ui.on_tile_act({ let s = shared.clone(); move |sh, k, act| {
        // (a Discover shelf's song; another page's shelves are songs only)
        let t = if s.borrow().page == "discover" {
            DISCOVER.with(|d| d.borrow().get(sh as usize).and_then(|x| x.items.get(k as usize).cloned())).and_then(|i| if let discover::Item::Track(t) = i { Some(t) } else { None })
        } else { s.borrow().shelves.get(sh as usize).and_then(|l| l.get(k as usize).cloned()) };
        let Some(t) = t else { return };
        menu_act(&s, t, &act);
    } });
    ui.on_helpers_act({ let s = shared.clone(); move |a| helpers_act(&s, &a) });
    ui.on_apple_import({ let s = shared.clone(); move || {
        let keys = s.borrow().library.iter().map(|t| t.key.clone()).collect();
        SHARED_LIBRARY_KEYS.with(|k| *k.borrow_mut() = keys);
        apple_start(&s);
    } });
    ui.on_picks_copy({ let s = shared.clone(); move || {
        let a = s.borrow();
        let list: Vec<Track> = a.page_tracks.iter().filter(|t| a.picked.contains(&t.key)).cloned().collect();
        if list.is_empty() { return; }
        // (as text too, for anywhere else: "Artist - Title  link", a line each, as the web app's)
        let text = list.iter().map(|t| format!("{}{}{}", if t.artist.is_empty() { String::new() } else { format!("{} - ", t.artist) }, t.title, if t.web_link().is_empty() { String::new() } else { format!("  {}", t.web_link()) })).collect::<Vec<_>>().join("\n");
        let _ = share::copy(&text);
        if let Some(ui) = a.ui.upgrade() { ui.set_status(format!("Copied {}. Ctrl+V pastes them into a playlist that's open, or the queue.", plural(list.len(), "track", "tracks")).into()); }
        PICK_CLIP.with(|c| *c.borrow_mut() = list);
    } });
    ui.on_picks_paste({ let s = shared.clone(); move || {
        let list = PICK_CLIP.with(|c| c.borrow().clone());
        if list.is_empty() { return; }
        let here = s.borrow().here.clone();
        match here {
            // (into the playlist that's open: yours, or one on this PC; the same song twice asked about)
            Some(Place::Playlist(id)) if xpl::is_own(&id) => pl_add_many(&s, &id, list),
            Some(Place::Playlist(_)) => { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status("This playlist isn't yours to change. Paste into one of yours, or one on this PC.".into()); } }
            // (anywhere else: the queue, after what you've queued)
            _ => {
                let n = list.len();
                let mut a = s.borrow_mut();
                for t in list { a.queue.add(t); }
                paint_queue(&mut a);
                save_session(&a);
                if let Some(ui) = a.ui.upgrade() { ui.set_status(format!("Added {} to the queue.", plural(n, "track", "tracks")).into()); }
            }
        }
    } });
    ui.on_jam_admit({ let s = shared.clone(); move |i, ok| join_answer(&s, i.max(0) as usize, ok) });
    ui.on_ask_answer({ let s = shared.clone(); move |ok| ask_answer_ui(&s, ok) });
    ui.on_now_act({ let s = shared.clone(); move |act| { let t = s.borrow().queue.current().cloned(); if let Some(t) = t { track_act(&s, &t, &act); } } });
    ui.on_title_rename({ let s = shared.clone(); move || name_ask(&s, Naming::AppName) });
    ui.on_title_signin({ let s = shared.clone(); move || { let i = ACCOUNTS.iter().position(|a| a.0 == "soundcloud").unwrap_or(0); account_act(&s, i); } });
    ui.on_page_pl({ let s = shared.clone(); move |act| {
        let side = { let a = s.borrow(); match &a.here { Some(Place::Playlist(id)) => a.side_map.iter().position(|&k| a.playlists.get(k).is_some_and(|p| &p.id == id)), _ => None } };
        if let Some(i) = side { side_action(&s, i, &act); }
    } });
    ui.on_artist_act({ let s = shared.clone(); move |i, act| artist_act(&s, i.max(0) as usize, &act) });
    ui.on_artists_batch_go({ let s = shared.clone(); move |k| artists_batch(&s, k.max(0) as usize) });
    ui.on_now_menu({ let w = ui.as_weak(); move || { if let Some(ui) = w.upgrade() { ui.set_now_menu_asks(ui.get_now_menu_asks() + 1); } } });
    ui.on_queue_clear({ let s = shared.clone(); move |req| {
        let mut a = s.borrow_mut();
        let note = if req { let n = a.queue.clear_requests(); format!("{} removed.", plural(n, "waiting request", "waiting requests")) } else { a.queue.clear(); "The queue is cleared.".to_string() };
        paint_queue(&mut a);
        save_session(&a);
        if let Some(ui) = a.ui.upgrade() { ui.set_status(note.into()); }
    } });
    ui.on_status_act({ let s = shared.clone(); move |i| status_act(&s, i.max(0) as usize) });
    ui.on_status_dismiss({ let w = ui.as_weak(); move || { if let Some(ui) = w.upgrade() { ui.set_status("".into()); STATUS_HOLD.with(|h| h.borrow_mut().take()); } } });
    ui.on_la_cycle({ let weak = ui.as_weak(); move |i| { LA.with(|l| { if let Some((_, _, c)) = l.borrow_mut().as_mut() { if let Some(x) = c.get_mut(i.max(0) as usize) { x.choice = x.choice.next(); } } }); if let Some(ui) = weak.upgrade() { la_paint(&ui); } } });
    ui.on_la_all({ let weak = ui.as_weak(); move |k| { LA.with(|l| { if let Some((_, _, c)) = l.borrow_mut().as_mut() { for x in c.iter_mut() { x.choice = match k { 0 => lookalike::Choice::Both, 1 => lookalike::Choice::Old, _ => lookalike::Choice::New }; } } }); if let Some(ui) = weak.upgrade() { la_paint(&ui); } } });
    ui.on_la_done({ let s = shared.clone(); move |ok| la_done(&s, ok) });
    ui.on_open_web(|url| { let _ = omnipulse_core::open_url(url.to_string()); });
    let s = shared.clone();
    ui.on_settings_close(move || settings_close(&s));
    let s = shared.clone();
    ui.on_set_hit(move |i| set_hit(&s, i.max(0) as usize));
    let s = shared.clone();
    ui.on_search(move |q| { if !q.trim().is_empty() { go(&s, Place::Search(q.trim().into())); } });
    let s = shared.clone();
    ui.on_search_edited(move |q| {
        let mut a = s.borrow_mut();
        if a.page == "settings" { drop(a); settings_search(&s, &q); return; }
        if a.page == "search" { return; }   // (there, Enter searches)
        a.page_query = q.trim().to_string();
        let before = a.page_all.len();
        apply_filter(&mut a);
        let shown = a.page_tracks.len();
        drop(a);
        // (the page's shelves too: what doesn't match leaves them, a shelf with nothing left goes)
        let tiles = page_find_shelves(&s, &q);
        if let Some(ui) = s.borrow().ui.upgrade() {
            let want = !q.trim().is_empty();
            let (n, of) = if ui.get_page_kind() == "tracks" { (shown, before) } else { tiles };
            ui.set_page_find_note(if !want { String::new() } else if n == 0 { "Nothing on this page matches. Press Enter to search everywhere.".to_string() } else { format!("{n} of {of} on this page. Press Enter to search everywhere.") }.into());
        }
    });
    ui.set_hide_likes(shared.borrow().web.prefs["hideLikes"].as_bool() == Some(true));
    let s = shared.clone();
    ui.on_hide_likes_changed(move |on| {
        let mut a = s.borrow_mut();
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("hideLikes".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("hideLikes", serde_json::json!(on)); }
        apply_filter(&mut a);
    });
    let s = shared.clone();
    // (remembered, as the web app's favOnly)
    { let on = shared.borrow().web.prefs["favOnly"].as_bool() == Some(true); shared.borrow_mut().fav_only = on; ui.set_fav_only(on); }
    ui.on_fav_only_changed(move |on| {
        let mut a = s.borrow_mut();
        a.fav_only = on;
        if let Some(o) = a.web.prefs.as_object_mut() { o.insert("favOnly".into(), serde_json::json!(on)); }
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("favOnly", serde_json::json!(on)); }
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
    ui.on_stats_clear({ let s = shared.clone(); move || stats_clear(&s) });
    ui.on_stats_open({ let s = shared.clone(); move |w| { STATS_DETAIL.with(|d| *d.borrow_mut() = w.to_string()); let p = s.borrow().chip; open_stats(&s, p); } });
    ui.on_stats_play_these({ let s = shared.clone(); move || { let l = STATS_TRACKS.with(|t| t.borrow().clone()); if !l.is_empty() { play_list(&s, l, 0); } } });
    ui.on_stats_song({ let s = shared.clone(); move |i| {
        let l = STATS_TOP.with(|t| t.borrow().0.clone());
        if (i as usize) < l.len() { play_list(&s, l, i as usize); }
    } });
    ui.on_stats_artist({ let s = shared.clone(); move |i| {
        if let Some(n) = STATS_TOP.with(|t| t.borrow().1.get(i.max(0) as usize).cloned()) { go(&s, Place::ArtistNamed(n)); }
    } });
    ui.on_stats_act({ let s = shared.clone(); move |which, k, act| {
        let t = if which == 1 { STATS_TOP.with(|t| t.borrow().0.get(k.max(0) as usize).cloned()) } else {
            match STATS_ROWS.with(|r| r.borrow().get(k.max(0) as usize).map(|x| x.0.clone())) {
                Some(StatsRow::Song(i)) => STATS_TRACKS.with(|t| t.borrow().get(i).cloned()),
                _ => None,
            }
        };
        let Some(t) = t else { return };
        if act == "play" { return play_list(&s, vec![t], 0); }
        menu_act(&s, t, &act);
    } });
    ui.on_stats_row({ let s = shared.clone(); move |k| {
        match STATS_ROWS.with(|r| r.borrow().get(k.max(0) as usize).map(|x| x.0.clone())) {
            Some(StatsRow::Artist(name)) => go(&s, Place::ArtistNamed(name)),
            Some(StatsRow::Song(i)) if i != usize::MAX => { let l = STATS_TRACKS.with(|t| t.borrow().clone()); if i < l.len() { play_list(&s, l, i); } }
            _ => {}
        }
    } });
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
        if page == "rediscover" && i == 3 {
            return page_albums_grid(&s);
        }
        if page == "artist" && i == 2 {
            return artist_albums(&s);
        }
        // (Search: each category on its own; the tracks for Tracks and Sets & Podcasts)
        if page == "search" {
            s.borrow_mut().chip = i as usize;
            ui.set_page_kind(if i == 1 || i == 5 { "tracks" } else { "search" }.into());
            apply_filter(&mut s.borrow_mut());
            search_paint(&s);
            return;
        }
        if ui.get_page_kind() == "albumgrid" { ui.set_page_kind("tracks".into()); }
        let mut a = s.borrow_mut();
        a.chip = i as usize;
        // (kept for this page, as the web app keeps it)
        if let Some(k) = FILTER_KEYS.get(i as usize) {
            let mut all = if a.web.prefs["pageFilters"].is_object() { a.web.prefs["pageFilters"].clone() } else { serde_json::json!({}) };
            all[filter_view(&page)] = serde_json::json!(k);
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert("pageFilters".into(), all.clone()); }
            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("pageFilters", all); }
        }
        apply_filter(&mut a);
    });
    let s = shared.clone();
    ui.on_album_tile(move |i| album_open(&s, i as usize));
    let s = shared.clone();
    ui.on_rows_in_view(move |first, count| {
        ROW_ART.with(|c| c.borrow_mut().view = (first.max(0) as usize, count.max(0) as usize));
        rows_art(&s);
    });
    // an album's page: Play, and Like Album
    let s = shared.clone();
    ui.on_album_head_act(move |a| match a.as_str() {
        "play" => { if let Some(ui) = s.borrow().ui.upgrade() { if ui.get_rows().row_count() > 0 { ui.invoke_play_row(0); } } }
        "like" => album_like(&s),
        _ => {}
    });
    let s = shared.clone();
    ui.on_play_row(move |i| {
        let list = s.borrow().page_tracks.clone();
        play_list(&s, list, i as usize);
    });
    let s = shared.clone();
    ui.on_tile(move |sh, k| {
        if s.borrow().page == "search" { return search_tile(&s, sh as usize, k as usize); }
        if s.borrow().page == "streamer" { return open_stream_coll(&s, sh as usize, k as usize); }
        if s.borrow().page == "discover" { return discover_tile(&s, sh as usize, k as usize); }
        let list = s.borrow().shelves.get(sh as usize).cloned().unwrap_or_default();
        play_list(&s, list, k as usize);
    });
    let s = shared.clone();
    ui.on_taste_pick(move || taste_open(&s, 1));
    // Discover's language: the next one, and Discover made again in it
    let s = shared.clone();
    // (the GPU visualiser's scenes, for OBS's /vis: as WebGL2 runs them)
    if let Ok(mut v) = omnipulse_core::widget::VIS.lock() { v.scenes = omnipulse_vis::webgl_json(); }
    ui.set_lang_options(ModelRc::from(Rc::new(VecModel::from(lang::CHOICES.iter().map(|(_, l)| slint::SharedString::from(*l)).collect::<Vec<_>>()))));
    ui.on_lang_pick(move |k| {
        let next = {
            let mut a = s.borrow_mut();
            let next = lang::CHOICES[(k.max(0) as usize).min(lang::CHOICES.len() - 1)].0;
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert("recLang".into(), serde_json::json!(next)); }
            next
        };
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("recLang", serde_json::json!(next)); }
        open_discover(&s);
    });
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
                    ui.set_status(format!("Feeling lucky: new music like {who} that isn't in your likes yet.").into());
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
            let all: Vec<Track> = a.page_tracks.iter().filter(|x| a.picked.contains(&x.key) && streamsafe::allowed(&a.web.prefs, x)).cloned().collect();
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
        // (the album shown above it: opened, or played)
        if act == "album" || act == "albumplay" {
            drop(a);
            let k = s.borrow().rows.row_data(i as usize).map(|r| r.alb_i).unwrap_or(-1);
            let Some(f) = PAGE_ALBUMS.with(|p| p.borrow().1.get(k.max(0) as usize).cloned()) else { return };
            if act == "album" { ALBUMS.with(|x| *x.borrow_mut() = vec![f]); return album_open(&s, 0); }
            let (rt, weak) = { let a = s.borrow(); (a.rt.clone(), a.ui.clone()) };
            let local: Vec<Track> = match &f { sources::Found::Album { plat: "local", id, .. } => s.borrow().library.iter().filter(|t| t.plat() == "local" && t.album.trim() == id).cloned().collect(), _ => Vec::new() };
            rt.spawn(async move {
                let list = if local.is_empty() { sources::found_tracks(&f).await } else { local };
                let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { if !list.is_empty() { play_list(&sh, list, 0); } } }));
            });
            return;
        }
        // (its versions opened beneath it, or closed; More versions: a search everywhere for the song)
        if act == "versions" {
            VER_OPEN.with(|o| { let mut o = o.borrow_mut(); if !o.remove(&t.key) { o.insert(t.key.clone()); } });
            apply_filter(&mut a);
            return;
        }
        if act == "moreversions" {
            drop(a);
            let (who, _) = versions::guess_names(&t);
            let q = format!("{} {}", if who.is_empty() { t.artist.clone() } else { who }, vgroup::base_title(&t, ""));
            return go(&s, Place::Search(q.trim().into()));
        }
        if act == "play" {
            let list = a.page_tracks.clone();
            drop(a);
            return play_list(&s, list, i as usize);
        }
        if matches!(act.as_str(), "like" | "fav" | "uploader" | "open" | "prefs" | "cleared") || act.starts_with("artist:") || act.starts_with("artistat:") {
            drop(a);
            track_act(&s, &t, &act);
            return;
        }
        if let Some(what) = act.strip_prefix("hide:") {
            drop(a);
            return hide_track(&s, &t, what);
        }
        if let Some(plat) = act.strip_prefix("link:") {
            drop(a);
            return copy_track_link(&s, &t, plat);
        }
        drop(a);
        queue_add(&s, t, act == "next");
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
        if let Some(t) = a.queue.items.get(at).map(|q| q.t.clone()) {
            if act.starts_with("pl") { drop(a); return pl_row_action(&s, &act, t); }
            if let Some(what) = act.strip_prefix("hide:") { drop(a); return hide_track(&s, &t, what); }
            if !matches!(act.as_str(), "next" | "remove" | "shuffle") { drop(a); menu_act(&s, t, &act); return; }
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
            a.queue.name = a.ui.upgrade().map(|u| u.get_page_title().to_string()).unwrap_or_default();
            a.queue_spaced = spaced;
            a.ap_failed = None;
        }
        start(&s);
    });
    let s = shared.clone();
    ui.on_autoplay_changed(move |on| {
        let mut a = s.borrow_mut();
        a.autoplay = on;
        // (Stream-Safe Music on: the stream's own, the web app's apStream)
        if streamsafe::on(&a.web.prefs) {
            if let Some(o) = a.web.prefs.as_object_mut() { o.insert("apStream".into(), serde_json::json!(on)); }
            if std::env::var_os("OMNIPULSE_SELFTEST").is_none() { webstate::save_pref("apStream", serde_json::json!(on)); }
        }
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
    ui.on_volume_changed({ let engine = engine.clone(); move |v| if casting().is_some() { omnipulse_core::cast::set_volume(v as f64) } else {
        engine.set_volume(v);
        // (kept for next time, once the slider's let go of: it opened at 80% every time before)
        if std::env::var_os("OMNIPULSE_SELFTEST").is_none() {
            let t = slint::Timer::default();
            t.start(slint::TimerMode::SingleShot, std::time::Duration::from_millis(800), move || webstate::save_pref("nativeVolume", serde_json::json!((v * 100.0).round())));
            VOLUME_SAVE.with(|x| *x.borrow_mut() = Some(t));
        }
    } });

    // the window: dragged by the title bar, its own buttons
    ui.on_drag({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() { os_drag(&ui, 2); }   // (HTCAPTION)
        }
    });
    // resized from an edge or corner (Windows does the dragging, as it does the title bar's)
    // (being resized: the costly glows rest and motion stops, so each step draws quickly; back when it's let go)
    {
        let weak = ui.as_weak();
        let motion = std::rc::Rc::new(std::cell::Cell::new(true));
        desktop::taskbar::ON_SIZING.with(|f| *f.borrow_mut() = Some(Box::new(move |on| {
            let Some(ui) = weak.upgrade() else { return };
            let t = ui.global::<Theme>();
            if on { motion.set(t.get_motion()); t.set_motion(false); } else { t.set_motion(motion.get()); }
            t.set_resizing(on);
            if !on { aurora_bake(&ui); }
        })));
    }
    ui.on_resize({
        let weak = ui.as_weak();
        move |d| {
            // (HTTOP, HTBOTTOM, HTLEFT, HTRIGHT, HTTOPLEFT, HTTOPRIGHT, HTBOTTOMLEFT, HTBOTTOMRIGHT)
            let ht = match d { 0 => 12, 1 => 15, 2 => 10, 3 => 11, 4 => 13, 5 => 14, 6 => 16, _ => 17 };
            if let Some(ui) = weak.upgrade() { os_drag(&ui, ht); }
        }
    });
    // (maximized or not, however it got that way: its button, a snap, Win+Up or Down, the title bar dragged
    // off the top. Each changes its size, so it's looked at then; the edges only there to resize it when it isn't)
    ui.on_window_resized({
        let weak = ui.as_weak();
        move || {
            if let Some(ui) = weak.upgrade() {
                let m = ui.window().is_maximized();
                if ui.get_is_max() != m { ui.set_is_max(m); }
                // (a maximize, a snap, F11: Aurora's background painted for the new size, a moment after; while
                // it's dragged, that waits till it's let go)
                if !ui.global::<Theme>().get_resizing() {
                    let w2 = ui.as_weak();
                    slint::Timer::single_shot(std::time::Duration::from_millis(120), move || if let Some(ui) = w2.upgrade() { aurora_bake(&ui); });
                }
            }
        }
    });
    { let s = shared.clone(); ui.on_minimize(move || minimize(&s)); }
    ui.on_full_screen_toggle({
        let weak = ui.as_weak();
        move || if let Some(ui) = weak.upgrade() { let w = ui.window(); w.set_fullscreen(!w.is_fullscreen()); }
    });
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
    ui.on_close_window(move || close_or_tray(&s));
    let s = shared.clone();
    ui.window().on_close_requested(move || { close_or_tray(&s); slint::CloseRequestResponse::KeepWindowShown });

    // the media keys and the tray (made here, on the window's thread, which has the message loop they need)
    let desk = desktop::Desktop::start();
    let timer = slint::Timer::default();
    let s = shared.clone();
    let mut n = 0u64;
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
        n += 1;
        tick(&s, &events, n);
        for a in desk.poll() {
            desktop_action(&s, &a);
        }
        // (the tray icon's hover tip: the memory in use, every few seconds)
        if n % 12 == 1 { desk.show_memory(); }
        // (the chatbot's hotkeys, as its actions have them now)
        if let Some(list) = HOTKEYS_WANTED.with(|h| h.borrow_mut().take()) {
            let failed = desk.set_hotkeys(&list);
            if !failed.is_empty() { if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(format!("Couldn't set up the hotkey {}. It may be written wrong, or another app already uses it.", failed.join(", ")).into()); } }
        }
    });
    // the taskbar buttons, once the window is up
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(500), move || {
        if let Some(ui) = weak.upgrade() {
            if let Some(h) = window_hwnd(&ui) {
                desktop::taskbar::attach(h);
                // Windows' media controls (always)
                desktop::media::enable(h, true);
            }
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
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("rchange") { vec![(9, "play"), (11, "rate"), (12, "skip"), (16, "rate"), (17, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("mixfb") { vec![(9, "play"), (10, "rate"), (11, "mixprep"), (70, "mixseek"), (80, "mixcheck"), (84, "mixcheck"), (88, "mixcheck"), (92, "mixcheck"), (96, "mixcheck"), (100, "mixcheck"), (104, "mixcheck"), (108, "mixcheck"), (112, "rate"), (114, "mixcheck"), (118, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("replies") { vec![(9, "play"), (10, "openreplies"), (12, "soundcat"), (13, "openmix"), (15, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("genre") { vec![(9, "play"), (10, "searchhouse"), (30, "searchartists"), (34, "discover"), (60, "findtiles"), (62, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("smartblend") { vec![(8, "smartpair"), (70, "smartseek"), (72, "smartcheck"), (76, "smartcheck"), (80, "smartcheck"), (84, "smartcheck"), (88, "smartcheck"), (92, "smartcheck"), (96, "smartcheck"), (100, "smartcheck"), (106, "smartcheck"), (108, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("smart") { vec![(9, "play"), (10, "smartprep"), (12, "smartpage"), (70, "smartseek"), (72, "smartcheck"), (76, "smartcheck"), (80, "smartcheck"), (84, "smartcheck"), (88, "smartcheck"), (92, "smartcheck"), (100, "smartcheck"), (102, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("accent") { vec![(9, "play"), (12, "accentfade"), (15, "askshow"), (17, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("scpl") { vec![(9, "play"), (14, "scplaylists"), (20, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("np") { vec![(9, "play"), (14, "now"), (18, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("versions") { vec![(9, "play"), (10, "artistnamed"), (30, "vopen"), (31, "listdown"), (32, "artistalbums"), (45, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("rhythm") { vec![(9, "play"), (100, "rhythm"), (101, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=dnd: a Library track dragged onto the queue; what the sidebar would take,
            // only asked, nothing dropped on your playlists)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("dnd") { vec![(9, "play"), (12, "library"), (13, "qopen"), (14, "dragq"), (16, "dragasks"), (17, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=panels: the settings panels, opened (nothing saved))
            // (OMNIPULSE_SELFTEST_PLAN=spotify: a Spotify song played through Spotify (Seamless Mix off for it), and whether its sound comes)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("spotify") { vec![(8, "spplay"), (14, "spcheck"), (20, "spcheck"), (21, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=rclick: a right-click on the sidebar's first playlist and on a Library row, and what shows)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("rclick") { vec![(12, "rclick:214:269"), (14, "esc"), (15, "rclick:1000:630"), (17, "esc"), (18, "rclick:80:322"), (20, "esc"), (21, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("rpick") { vec![(12, "rclick:500:384"), (13, "lclick:560:126"), (15, "lclick:560:30"), (16, "esc"), (17, "rclick:80:322"), (18, "lclick:127:344"), (20, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=tips: the pointer rested on the player bar's "..." and the shuffle button: their tooltips)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("tips") { vec![(9, "play"), (11, "hover:1095:711"), (13, "tipsay"), (13, "hover:571:711"), (15, "tipsay"), (16, "hover:600:300"), (17, "tipsay"), (18, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=apple: Apple Music's sign-in window opened off-screen, Apple's page looked at, closed; nobody signs in)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("apple") { vec![(6, "appleopen"), (16, "applecheck"), (17, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=tp: Track preferences for what plays, its bar dragged)
            // (OMNIPULSE_SELFTEST_PLAN=viz: Now Playing's visualiser, Bars then Glow, the sound at nothing)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("viz") { vec![(8, "vizprep"), (9, "library"), (11, "play"), (13, "now"), (17, "viz:blocks"), (19, "viz:wave"), (21, "viz:centre"), (23, "vizfull"), (25, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=alertwave: the first alert with a sound, its waveform and trims, in a tall window)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("alertwave") { vec![(9, "size:1180:1700"), (10, "settings"), (11, "alertsnd"), (13, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=normal: as it's used: a song playing (the sound at nothing), Now Playing,
            // Discover, the Library again; memory measured from outside)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("normal") { vec![(8, "mute"), (9, "library"), (11, "play"), (14, "now"), (30, "discover"), (55, "library"), (95, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=idle: the Library, then Discover, sitting there, for memory measured from outside)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("idle") { vec![(9, "library"), (40, "discover"), (75, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=applook: App & Look, nothing played)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("sayacts") { vec![(6, "sayacts"), (8, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("albumpage") { vec![(8, "search"), (28, "openalbum"), (38, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("round") { vec![(9, "settings"), (10, "cat_Streaming"), (12, "titleset:youtube"), (14, "prefset:reqBot=moobot"), (15, "panel:requests"), (17, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("flows") { vec![(9, "settings"), (10, "cat_Streaming"), (11, "flows"), (12, "flownew:5"), (14, "flownew:7"), (16, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("flowgames") { vec![(9, "settings"), (10, "cat_Streaming"), (11, "flows"), (12, "flownew:24"), (14, "flownew:11"), (16, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("restored") {
                let dir = std::env::var("OMNIPULSE_SELFTEST_BOTS").unwrap_or_default();
                let (sb, miu): (&'static str, &'static str) = (Box::leak(format!("restore:{dir}/streamerbot-backup.zip").into_boxed_str()), Box::leak(format!("restore:{dir}/mixitup.miubackup").into_boxed_str()));
                vec![(9, "settings"), (10, "cat_Streaming"), (11, "flows"), (12, sb), (13, "flowpick:3"), (15, miu), (16, "flowpick:30"), (18, "end")]
            }
            // (OMNIPULSE_SELFTEST_PLAN=steps: the steps OMNIPULSE_SELFTEST_STEPS gives, "secs=step;secs=step", any of the steps below)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("steps") {
                std::env::var("OMNIPULSE_SELFTEST_STEPS").unwrap_or_default().split(';').filter_map(|x| x.split_once('=')).filter_map(|(t, w)| Some((t.trim().parse::<u64>().ok()?, &*Box::leak(w.trim().to_string().into_boxed_str())))).collect()
            }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("botset") { vec![(9, "settings"), (10, "titleset:plugin:chatbot"), (12, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("ring") { vec![(8, "vizprep"), (9, "library"), (11, "play"), (13, "now"), (15, "viz:ring"), (19, "sayacts"), (22, "vizfull"), (25, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("applook") { vec![(9, "settings"), (10, "cat_App & Look"), (12, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=plugbar: a plugin's icon put in the title bar, then right-clicked for its settings)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("plugbar") { vec![(9, "plugbar"), (11, "titleset:plugin:internet-archive"), (13, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=loadview: the pages as they load, their platforms ticked off)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("loadview") { vec![(1, "library"), (2, "rediscover"), (4, "discover"), (5, "loadshot1"), (6, "loadshot2"), (7, "stats"), (8, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=titleset: the title bar's icons right-clicked: Seamless Mix's settings, then SoundCloud's)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("titleset") { vec![(9, "titleset:smart"), (11, "titleset:soundcloud"), (13, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=tallpanels: the Stream Widget's and the alerts' pages in a window tall enough for all of them)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("tallpanels") { vec![(9, "size:1180:2600"), (10, "settings"), (11, "cat_Streaming"), (12, "p_widget"), (14, "p_alerts"), (16, "p_alertlist"), (18, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=side: the sidebar collapsed by hand, and Settings left for Discover)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("side") { vec![(9, "library"), (11, "sidefold"), (13, "sideopen"), (14, "settings"), (16, "discover"), (18, "back"), (20, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("setfit") { vec![(9, "play"), (10, "queue"), (11, "size:1150:530"), (12, "nav:settings"), (13, "cat:Sound"), (14, "scroll:4000"), (16, "cat:Integrations"), (17, "scroll:-4000"), (19, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=pagespeed: the Library, Discover and Rediscover opened twice each, timed with OMNIPULSE_PAGE_TIMING)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("pagespeed") { vec![(8, "nav:library"), (11, "nav:discover"), (40, "nav:rediscover"), (43, "nav:library"), (46, "nav:discover"), (49, "nav:rediscover"), (52, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("acc2") { vec![(9, "play"), (10, "accent2"), (12, "nav:library"), (14, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("dragasks") { vec![(9, "play"), (12, "dragasks"), (13, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("loudsteps") { vec![(9, "play"), (16, "loud:normal"), (17, "loud:loud"), (18, "loud:normal"), (19, "loud:quiet"), (20, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("titleedit") { vec![(9, "play"), (10, "titleedit"), (12, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("panels2") { vec![(9, "play"), (10, "nav:settings"), (11, "cat:Streaming"), (12, "panel:widget"), (14, "scroll:600"), (16, "panel:alerts"), (18, "scroll:900"), (20, "panel:limits"), (22, "panel:offset"), (24, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("sound2") { vec![(9, "play"), (10, "nav:settings"), (11, "cat:Sound"), (13, "scroll:300"), (15, "cat:App & Look"), (16, "scroll:200"), (18, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("mixpage") { vec![(9, "play"), (10, "nav:settings"), (11, "cat:Sound"), (13, "scroll:300"), (14, "panel:mix"), (16, "scroll:900"), (18, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("statclick") { vec![(9, "play"), (10, "nav:stats"), (13, "statsong"), (15, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("setclear") { vec![(9, "play"), (10, "nav:settings"), (12, "setq:fade"), (13, "setq:"), (15, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("artistsym") { vec![(9, "play"), (12, "nav:artists"), (18, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("square") { vec![(9, "play"), (10, "unround"), (12, "nav:settings"), (13, "cat:Streaming"), (14, "panel:widget"), (16, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("qset") { vec![(9, "play"), (11, "queue"), (14, "nav:settings"), (15, "cat:App & Look"), (16, "scroll:420"), (18, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("nowslim") { vec![(9, "play"), (11, "queue"), (12, "now"), (15, "size:860:720"), (17, "size:1100:720"), (19, "size:1500:860"), (21, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("smartmem") { vec![(9, "smartprep"), (10, "play"), (12, "mem"), (37, "mem"), (38, "nearend"), (62, "mem"), (87, "mem"), (88, "nearend"), (112, "mem"), (137, "mem"), (138, "nearend"), (162, "mem"), (187, "mem"), (188, "nearend"), (212, "mem"), (237, "mem"), (238, "nearend"), (262, "mem"), (287, "mem"), (288, "nearend"), (312, "mem"), (337, "mem"), (338, "nearend"), (362, "mem"), (387, "mem"), (388, "nearend"), (412, "mem"), (413, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("lyrprev") { vec![(8, "playtitle"), (9, "widgeton"), (10, "openpanel:lyricsw"), (12, "lyrdraft"), (150, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("lyricsw") { vec![(8, "playtitle"), (9, "widgeton"), (20, "lyrshow"), (40, "lyrshow"), (90, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("gamealert") { vec![(8, "play"), (9, "widgeton"), (14, "gametest"), (21, "gametest"), (28, "gametest"), (35, "gametest"), (42, "gametest"), (49, "gametest"), (56, "gametest"), (63, "gametest"), (70, "gametest"), (77, "gametest"), (84, "gametest"), (91, "gametest"), (98, "gametest"), (105, "gametest"), (112, "gametest"), (119, "gametest"), (121, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("scplay") { vec![(9, "play"), (18, "spcheck3"), (22, "spcheck3"), (23, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("goplus") { vec![(9, "goplus"), (10, "gpfind"), (20, "spcheck3"), (40, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("spsmart") { vec![(9, "smartprep"), (10, "spplay3"), (24, "spcheck3"), (30, "spcheck3"), (31, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("dropclick") {
                // (OMNIPULSE_SELFTEST_CLICKS="panel;x,y;x,y": the panel opened, then clicks where a pointer would)
                let spec = std::env::var("OMNIPULSE_SELFTEST_CLICKS").unwrap_or_else(|_| "requests;1028,196;1028,262".into());
                let mut v: Vec<(u64, &'static str)> = vec![(9, Box::leak(format!("openpanel:{}", spec.split(';').next().unwrap_or("requests")).into_boxed_str()))];
                for (k, c) in spec.split(';').skip(1).enumerate() { v.push((11 + 2 * k as u64, Box::leak(format!("click:{c}").into_boxed_str()))); }
                v.push((11 + 2 * spec.split(';').count() as u64, "pfshow"));
                v.push((13 + 2 * spec.split(';').count() as u64, "end"));
                v
            }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("selects") { vec![(9, "selects:requests"), (11, "selects:alerts"), (13, "selects:widget"), (15, "selects:replies"), (17, "selects:discord"), (19, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("alerts") { vec![(9, "openwidget"), (11, "alerton"), (12, "alertshow"), (13, "alerton"), (14, "alertshow"), (15, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("qorder") { vec![(9, "rediscover"), (14, "play"), (18, "qshow"), (19, "qadd"), (20, "qshow"), (22, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("beat1") { vec![(9, "play"), (10, "smartprep"), (12, "tprefs"), (13, "beatearlier"), (14, "tpsave"), (15, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("tp") { vec![(9, "play"), (12, "tprefs"), (13, "tpdrag"), (14, "nowmenu"), (15, "now"), (18, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=la: the look-alikes dialog, with two made up (nothing added))
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("la") { vec![(8, "lashow"), (9, "lacycle"), (10, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=stats: the Stats page, its facts said)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("stats") { vec![(6, "stats"), (8, "statsfacts"), (9, "sd_time"), (11, "sd_artists"), (13, "sd_songs"), (15, "sd_when"), (16, "sd_where"), (17, "sd_streak"), (18, "sd_"), (19, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=setsearch: Settings searched, a panel's path clicked back along, and closed)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("setsearch") { vec![(9, "library"), (10, "settings"), (11, "ss_fade"), (12, "ss_mix tempo"), (13, "sshit"), (14, "p_alert:raid"), (15, "crumb2"), (16, "p_safe"), (17, "setclose"), (18, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("panels") { vec![(9, "settings"), (10, "cat_Integrations"), (11, "cat_Plugins"), (12, "p_requests"), (12, "p_widget"), (14, "p_alerts"), (16, "p_plugins"), (17, "plugread"), (20, "p_alertlist"), (21, "p_alert:raid"), (22, "pfclose"), (23, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=taskbar: the taskbar widget switched on (shown for a few seconds), a
            // picture of it, then off)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("taskbar") { vec![(9, "play"), (10, "tbon"), (15, "tbshot"), (16, "tbsmall"), (18, "tbshot2"), (19, "tboff"), (21, "end")] }
            // (OMNIPULSE_SELFTEST_PLAN=look: List Density at Compact, and Search with Hide likes)
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("look") { vec![(9, "play"), (12, "library"), (13, "compact"), (14, "settings"), (15, "comfy"), (16, "search"), (19, "hidelikes"), (21, "hidelikesoff"), (22, "discover"), (30, "langnext"), (40, "end")] }
            else if only_discover { vec![(9, "play"), (11, "discover"), (30, "disc"), (45, "discmix"), (12, "import"), (46, "taste"), (47, "taste2"), (48, "taste3"), (70, "tall"), (70, "tasted"), (71, "short"), (71, "yourmix"), (90, "mixplats"), (91, "marquee"), (92, "marqueeoff"), (93, "end")] } else { full.to_vec() };
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), move || {
            let Some(&(at, what)) = plan.get(step_no) else { return };
            if started.elapsed().as_secs() < at { return; }
            let ui = s.borrow().ui.upgrade().unwrap();
            // a picture of what the last step opened
            if step_no > 0 {
                let name = plan[step_no - 1].1;
                if let Ok(shot) = ui.window().take_snapshot() {
                    let _ = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec()).map(|i| i.save(format!("{dir}/{}.png", name.replace(':', "_"))));
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
                "tbon" => ui.invoke_app_pref("taskbarWidget".into(), true),
                "tbshot" => TBW.with(|w| {
                    if let Some((tw, h, shown)) = w.borrow().as_ref() {
                        println!("selftest: taskbar widget shown {shown}, hwnd {h}, size {:?}, at {:?}, \"{}\" playing {}", tw.window().size(), tw.window().position(), tw.get_song(), tw.get_playing());
                        if let Ok(shot) = tw.window().take_snapshot() {
                            let _ = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec()).map(|i| i.save(format!("{dir}/taskbar-widget.png")));
                        }
                    } else { println!("selftest: no taskbar widget"); }
                }),
                "tbsmall" => { ui.invoke_app_pref("widgetBigArt".into(), false); ui.invoke_app_pref("widgetProgress".into(), false); }
                "tbshot2" => TBW.with(|w| { if let Some((tw, _, _)) = w.borrow().as_ref() { if let Ok(shot) = tw.window().take_snapshot() { let _ = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec()).map(|i| i.save(format!("{dir}/taskbar-widget-small.png"))); } } }),
                "tboff" => { ui.invoke_app_pref("taskbarWidget".into(), false); println!("selftest: taskbar widget after off: shown {}", TBW.with(|w| w.borrow().as_ref().is_some_and(|x| x.2))); }
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
                "openalbum" => {
                    let first = SEARCH.with(|x| x.borrow().2.first().cloned());
                    println!("selftest: search albums {}", SEARCH.with(|x| x.borrow().2.len()));
                    if let Some(f) = first { FOUND.with(|x| *x.borrow_mut() = vec![vec![f]]); open_found(&s, 0, 0); }
                }
                "compact" => ui.invoke_density_changed(true),
                "sidefold" => ui.set_side_collapsed(true),
                "vizprep" => {
                    ui.set_volume(0.0);
                    ui.invoke_volume_changed(0.0);
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("nowViz".into(), serde_json::json!("peaks")); o.insert("vizBar".into(), serde_json::json!(true)); }
                }
                "vizfull" => ui.set_viz_full(true),
                "mute" => { ui.set_volume(0.0); ui.invoke_volume_changed(0.0); }
                "sayacts" => say_with(&ui, "Hardware Acceleration changes the next time OmniPulse starts.", vec![("Restart Now", StatusAct::Dismiss), ("Later", StatusAct::Dismiss)]),
                w if w.starts_with("viz:") => { if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("nowViz".into(), serde_json::json!(&w[4..])); } }
                "vizlook" => { ui.set_viz_full(false); ui.invoke_nav("settings".into()); }
                "vizglow" => {
                    println!("selftest: viz bands {:?}", ui.get_viz_bands().iter().map(|v| (v * 100.0).round() / 100.0).collect::<Vec<_>>());
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("nowViz".into(), serde_json::json!("glow")); }
                }
                "plugbar" => {
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("pluginSettings".into(), serde_json::json!({ "internet-archive": { "_titleBar": true } })); }
                    paint_plats(&s.borrow());
                    println!("selftest: plugbar: {:?}", ui.get_plats().iter().map(|p| (p.id.to_string(), p.on)).collect::<Vec<_>>());
                }
                w if w.starts_with("titleset:") => ui.invoke_title_settings(w[9..].into()),
                "sideopen" => ui.set_side_collapsed(false),
                "back" => { ui.invoke_back(); println!("selftest: back: page {}", s.borrow().page); }
                w if w.starts_with("cat_") => ui.set_set_cat(w[4..].into()),
                w if w.starts_with("ss_") => { ui.set_search_text(w[3..].into()); ui.invoke_search_edited(w[3..].into()); println!("selftest: settings search \"{}\": {}", &w[3..], ui.get_set_hits().iter().take(6).map(|h| format!("{} ({})", h.label, h.path)).collect::<Vec<_>>().join("; ")); }
                "spplay" => {
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("smartOn".into(), serde_json::json!(false)); }
                    let t = s.borrow().library.iter().find(|t| matches!(t.src, track::Src::Spotify(_))).cloned();
                    match t { Some(t) => { println!("selftest: spotify: playing \"{}\"", t.title); play_list(&s, vec![t], 0); } None => println!("selftest: spotify: no Spotify song in the Library") }
                }
                "spcheck" => { let a = s.borrow(); println!("selftest: spotify: now \"{}\" at {} ms, loaded {}, paused {}, status \"{}\"", a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.engine.position_ms(), a.loaded, a.engine.is_paused(), ui.get_status()); }
                "yourmix" => { let at = ui.get_shelves().iter().position(|sh| sh.title == "Made for you"); println!("selftest: opening Made for you {at:?}"); if let Some(i) = at { ui.invoke_tile(i as i32, 0); } }
                "mixplats" => { let a = s.borrow(); let mut n: std::collections::BTreeMap<&str, usize> = Default::default(); for t in &a.page_tracks { *n.entry(t.plat()).or_default() += 1; } println!("selftest: mix \"{}\": {} tracks by platform {:?}", ui.get_page_title(), a.page_tracks.len(), n); }
                "tall" => ui.window().set_size(slint::LogicalSize::new(1180.0, 2600.0)),
                "short" => ui.window().set_size(slint::LogicalSize::new(1180.0, 760.0)),
                "lashow" => {
                    let lib: Vec<Track> = s.borrow().library.iter().take(4).cloned().collect();
                    if lib.len() >= 4 {
                        let c = vec![lookalike::Conflict { new: lib[0].clone(), twin: lib[1].clone(), choice: lookalike::Choice::Both }, lookalike::Conflict { new: lib[2].clone(), twin: lib[3].clone(), choice: lookalike::Choice::Both }];
                        LA.with(|l| *l.borrow_mut() = Some(("none".into(), Vec::new(), c)));
                        la_paint(&ui);
                        ui.set_la_open(true);
                    }
                }
                "tpdrag" => { ui.invoke_tp_drag(0.2, 0); ui.invoke_tp_drag(0.8, 1); println!("selftest: trim bar {:.2}-{:.2}, start \"{}\" end \"{}\"", ui.get_tp_from(), ui.get_tp_to(), ui.get_tp_start(), ui.get_tp_end()); }
                "nowmenu" => { ui.set_tp_open(false); ui.invoke_more_want(); println!("selftest: player menu: {:?}", ui.get_more_items().iter().map(|x| x.to_string()).collect::<Vec<_>>()); }
                "lacycle" => { ui.invoke_la_cycle(1); println!("selftest: look-alikes: {:?}", ui.get_la_rows().iter().map(|r| r.choice.to_string()).collect::<Vec<_>>()); ui.invoke_la_done(false); }
                w if w.starts_with("sd_") => { ui.invoke_stats_open(w[3..].into()); println!("selftest: stats detail \"{}\": \"{}\", {} blocks", &w[3..], ui.get_st_detail_head(), ui.get_st_blocks().row_count()); }
                "queue" => { ui.set_queue_open(true); }
                w if w.starts_with("nav:") => { ui.invoke_nav(w[4..].into()); }
                "accent2" => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { if let Some(o) = sh.borrow_mut().web.prefs.as_object_mut() { o.insert("artAccent".into(), serde_json::json!(false)); o.insert("accent".into(), serde_json::json!("#4cc9f0")); o.insert("accent2".into(), serde_json::json!("#fa586a")); } apply_look(&sh); } }
                w if w.starts_with("loud:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { loud_pick(&sh, &w[5..]); } }
                "titleedit" => { ui.set_title_editing(true); }
                "statsong" => { let before = s.borrow().queue.current().map(|t| t.title.clone()); ui.invoke_stats_song(0); println!("selftest: stats top song 0: {:?} -> {:?}, top {:?}", before, s.borrow().queue.current().map(|t| t.title.clone()), STATS_TOP.with(|t| t.borrow().0.iter().map(|x| x.title.clone()).collect::<Vec<_>>())); }
                w if w.starts_with("setq:") => { ui.set_search_text(w[5..].into()); ui.invoke_search_edited(w[5..].into()); println!("selftest: settings search \"{}\": query \"{}\", cat \"{}\", {} hits", &w[5..], ui.get_set_query(), ui.get_set_cat(), ui.get_set_hits().row_count()); }
                "unround" => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { pref_put(&sh, "appRounded", serde_json::json!(false)); apply_look(&sh); } }
                // (the chatbot's Actions page, and a new action on it from a template)
                "flows" => flows_open(&s),
                w if w.starts_with("flownew:") => ui.invoke_flow_new(w[8..].parse().unwrap_or(0)),
                // (a backup restored, as Restore would: restore:<its path>; a run like this keeps nothing)
                w if w.starts_with("restore:") => { flows_restored(&s, botimport::restore_file(std::path::Path::new(&w[8..]))); println!("selftest: {}", ui.get_status()); }
                w if w.starts_with("flowpick:") => ui.invoke_flow_pick(w[9..].parse().unwrap_or(0)),
                // (a setting, for this run only: prefset:key=value)
                w if w.starts_with("prefset:") => {
                    if let Some((k, v)) = w[8..].split_once('=') { if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert(k.into(), serde_json::from_str(v).unwrap_or_else(|_| serde_json::json!(v))); } }
                }
                // (the look drawn again, after a prefset: a theme or a player style tried)
                "look" => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { apply_look(&sh); sound_paint(&sh.borrow()); } }
                // (the visualiser's music, made up: its tap fed a beat, nothing played; viz on wherever it's shown)
                "vizfeed" => {
                    if let Some(sh) = SHARED.with(|x| x.borrow().clone()) {
                        let _ = &sh;
                        VIZ_TEST.store(true, std::sync::atomic::Ordering::Relaxed);
                        let started = std::time::Instant::now();
                        let t = slint::Timer::default();
                        t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(33), move || viz::test_feed(started.elapsed().as_secs_f32()));
                        std::mem::forget(t);
                    }
                }
                w if w.starts_with("setup:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { setup_show(&sh, w[6..].parse().unwrap_or(0)); } }
                // (a widget's pop-out preview, in its own window: "wpreview:widget")
                w if w.starts_with("wpreview:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { widget_preview_open(&sh, &w[9..]); } }
                w if w.starts_with("panel:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { panel_open(&sh, &w[6..]); } }
                // (a plugin's own page, like the Suite's)
                w if w.starts_with("plug:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { pf_open(&sh, &w[5..]); } }
                w if w.starts_with("botact:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { bot_act(&sh, &w[7..]); } }
                w if w.starts_with("cat:") => { ui.set_set_cat(w[4..].into()); }
                // (Settings' search typed, and its n-th result picked)
                w if w.starts_with("setsearch:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { settings_search(&sh, &w[10..]); } }
                w if w.starts_with("sethit:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { set_hit(&sh, w[7..].parse().unwrap_or(0)); } }
                w if w.starts_with("scroll:") => { ui.window().dispatch_event(slint::platform::WindowEvent::PointerScrolled { position: slint::LogicalPosition::new(560.0, 400.0), delta_x: 0.0, delta_y: -(w[7..].parse::<f32>().unwrap_or(0.0)) }); }
                w if w.starts_with("size:") => {
                    let v: Vec<f32> = w[5..].split(':').filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 2 { ui.window().set_size(slint::LogicalSize::new(v[0], v[1])); }
                }
                w if w.starts_with("hover:") => {
                    let v: Vec<f32> = w[6..].split(':').filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 2 { ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: slint::LogicalPosition::new(v[0], v[1]) }); }
                }
                w if w.starts_with("lclick:") => {
                    let v: Vec<f32> = w[7..].split(':').filter_map(|x| x.parse().ok()).collect();
                    if v.len() == 2 {
                        let at = slint::LogicalPosition::new(v[0], v[1]);
                        ui.window().dispatch_event(slint::platform::WindowEvent::PointerMoved { position: at });
                        ui.window().dispatch_event(slint::platform::WindowEvent::PointerPressed { position: at, button: slint::platform::PointerEventButton::Left });
                        ui.window().dispatch_event(slint::platform::WindowEvent::PointerReleased { position: at, button: slint::platform::PointerEventButton::Left });
                    }
                    println!("selftest: page now {}", ui.get_page());
                }
                w if w.starts_with("rclick:") => {
                    use slint::platform::{PointerEventButton, WindowEvent};
                    let v: Vec<f32> = w[7..].split(':').filter_map(|x| x.parse().ok()).collect();
                    let at = slint::LogicalPosition::new(v[0], v[1]);
                    ui.window().dispatch_event(WindowEvent::PointerMoved { position: at });
                    ui.window().dispatch_event(WindowEvent::PointerPressed { position: at, button: PointerEventButton::Right });
                    ui.window().dispatch_event(WindowEvent::PointerReleased { position: at, button: PointerEventButton::Right });
                }
                "esc" => { ui.window().dispatch_event(slint::platform::WindowEvent::KeyPressed { text: slint::platform::Key::Escape.into() }); ui.window().dispatch_event(slint::platform::WindowEvent::KeyReleased { text: slint::platform::Key::Escape.into() }); }
                "tipsay" => { let t = ui.global::<Tips>(); println!("selftest: tooltip on {}: \"{}\"", t.get_on(), t.get_text()); }
                "appleopen" => { let r = apple_window(&s); APPLE.with(|a| if let Some((w, _, _)) = a.borrow().as_ref() { w.window().set_position(slint::PhysicalPosition::new(-4000, 0)); }); println!("selftest: apple window: {:?}", r.err()); }
                "applecheck" => {
                    APPLE.with(|a| if let Some((_, Some(v), _)) = a.borrow().as_ref() { println!("selftest: apple page: {:?}, cookies {}", v.url().ok(), v.cookies_for_url("https://music.apple.com").map(|c| c.len()).unwrap_or(0)); } else { println!("selftest: apple page: none"); });
                    APPLE.with(|a| { if let Some((w, _, _)) = a.borrow_mut().take() { let _ = w.hide(); } });
                }
                "statsfacts" => println!("selftest: stats: {} | streak {} {} | facts {:?}", ui.get_st_big(), ui.get_st_streak(), ui.get_st_streak_sub(), ui.get_st_fact_heads().iter().zip(ui.get_st_facts().iter()).map(|(h, f)| format!("{h}: {f}")).collect::<Vec<_>>()),
                "sshit" => { ui.invoke_set_hit(0); println!("selftest: after a hit: cat \"{}\", query \"{}\"", ui.get_set_cat(), ui.get_set_query()); }
                "crumb2" => { println!("selftest: path {:?}", ui.get_pf_crumbs().iter().collect::<Vec<_>>()); ui.invoke_crumb_go(2); println!("selftest: after its 3rd part: open {}, head \"{}\"", ui.get_pf_open(), ui.get_pf_head()); }
                "setclose" => { ui.invoke_settings_close(); println!("selftest: settings closed: page \"{}\", panel {}, cat \"{}\"", ui.get_page(), ui.get_pf_open(), ui.get_set_cat()); }
                "alertsnd" => {
                    // (none yet: a made-up one for Follows, its outline a swell with silence at its start; not kept)
                    if s.borrow().web.prefs["alertSounds"].as_object().is_none_or(|m| m.is_empty()) {
                        let env: Vec<f64> = (0..160).map(|i| if i < 22 { 0.01 } else { ((i as f64 * 0.37).sin().abs() * 0.6 + 0.2) * (1.0 - (i as f64 - 22.0) / 160.0) }).collect();
                        if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() {
                            o.insert("alertSounds".into(), serde_json::json!({ "follow": { "name": "ding.mp3", "at": 1 } }));
                            o.insert("alertLevels".into(), serde_json::json!({ "follow": { "ms": 2400, "auto": -2.5, "env": env } }));
                            o.insert("alertEdit".into(), serde_json::json!({ "follow": { "tstart": 330, "tend": 200 } }));
                        }
                    }
                    let k = s.borrow().web.prefs["alertSounds"].as_object().and_then(|m| m.keys().next().cloned());
                    println!("selftest: alertsnd: {k:?}, levels {}", k.as_ref().map(|k| s.borrow().web.prefs["alertLevels"][k]["env"].as_array().map_or(0, |e| e.len())).unwrap_or(0));
                    if let Some(k) = k { panel_open(&s, &format!("alert:{k}")); }
                }
                w if w.starts_with("p_") => { panel_open(&s, &w[2..]); println!("selftest: panel {} has {} fields", &w[2..], ui.get_pf_fields().row_count()); }
                "plugread" => { panel_put(&s, "plugSource", serde_json::json!(std::env::current_dir().unwrap_or_default().join("plugins").join("internet-archive").join("manifest.json").display().to_string())); panel_press(&s, "plugCheck"); }
                "pfclose" => { PF_EDIT.with(|e| e.borrow_mut().take()); ui.set_pf_open(false); }
                "qopen" => ui.set_queue_open(true),
                "dragq" => {
                    use slint::platform::{PointerEventButton, WindowEvent};
                    let n = s.borrow().queue.items.len();
                    let w = ui.window();
                    let at = |x: f32, y: f32| slint::LogicalPosition::new(x, y);
                    w.dispatch_event(WindowEvent::PointerPressed { position: at(560.0, 392.0), button: PointerEventButton::Left });
                    for k in 1..=12 { w.dispatch_event(WindowEvent::PointerMoved { position: at(560.0 + k as f32 * 40.0, 392.0 + k as f32 * 2.0) }); }
                    w.dispatch_event(WindowEvent::PointerReleased { position: at(1040.0, 416.0), button: PointerEventButton::Left });
                    println!("selftest: drag onto the queue: {n} -> {} (\"{}\")", s.borrow().queue.items.len(), ui.get_status());
                }
                // (a widget page's setting changed, as a click there does, and how long it and the page's redraw took)
                w if w.starts_with("botset:") => {
                    if let Some((k, v)) = w[7..].split_once('=') {
                        let t0 = std::time::Instant::now();
                        bot_set(&s, k, v);
                        let set_ms = t0.elapsed().as_secs_f64() * 1000.0;
                        let t1 = std::time::Instant::now();
                        bot_paint(&s);
                        println!("selftest: timing {} {k}: set {set_ms:.1} ms, redraw {:.1} ms", BOT_SHEET.with(|b| b.borrow().clone()), t1.elapsed().as_secs_f64() * 1000.0);
                    }
                }
                "dragasks" => {
                    let d = ui.global::<Dnd>();
                    let ask = |t: &str, what: &str| d.invoke_accepts(t.into(), slint::DataTransfer::from(slint::SharedString::from(what)));
                    let sides = s.borrow().side_map.len();
                    let own: Vec<bool> = (0..sides.min(6)).map(|i| ask(&format!("side:{i}"), "t:0")).collect();
                    println!("selftest: drag asks: track on sides 0-5 {own:?}; playlist 0 on itself {}, on side 1 {}; queue {}, playlist on queue {}", ask("side:0", "p:0"), ask("side:1", "p:0"), ask("queue", "t:0"), ask("queue", "p:0"));
                    // (the sidebar's order: a playlist before another, not where it already is; moved, pinned)
                    println!("selftest: drag asks: playlist 0 to gap 0 {}, gap 1 {}, gap 2 {}; track to gap 2 {}", ask("sidegap:0", "p:0"), ask("sidegap:1", "p:0"), ask("sidegap:2", "p:0"), ask("sidegap:2", "t:0"));
                    let names = |s: &Shared| { let a = s.borrow(); a.side_map.iter().take(4).map(|&i| a.playlists[i].title.clone()).collect::<Vec<_>>() };
                    let before = names(&s);
                    d.invoke_dropped("sidegap:2".into(), slint::DataTransfer::from(slint::SharedString::from("p:0")));
                    let moved = names(&s);
                    side_action(&s, 2, "pin");
                    println!("selftest: sidebar order {before:?} -> moved {moved:?} -> third pinned {:?}", names(&s));
                }
                "langnext" => { ui.invoke_lang_pick(ui.get_lang_at() + 1); println!("selftest: Discover language now \"{}\"", ui.get_lang_label()); }
                "comfy" => { ui.invoke_density_changed(false); ui.invoke_nav("library".into()); }
                "hidelikes" => { let n = ui.get_rows().row_count(); ui.invoke_hide_likes_changed(true); println!("selftest: search {n} rows, with Hide likes {}", ui.get_rows().row_count()); }
                "hidelikesoff" => { ui.invoke_hide_likes_changed(false); println!("selftest: Hide likes off: {} rows", ui.get_rows().row_count()); }
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
                "artistnamed" => open_artist_named(&s, "deadmau5"),
                "artistalbums" => {
                    let rows = s.borrow().rows.clone();
                    let albs: Vec<String> = (0..rows.row_count()).filter_map(|k| rows.row_data(k)).filter(|r| r.alb != "").map(|r| format!("{} ({})", r.alb, r.alb_plat)).collect();
                    println!("selftest: albums among the rows: {albs:?}");
                    ui.invoke_chip(2);
                }
                "listdown" => ui.set_list_y(330.0),
                "scplaylists" => {
                    {
                        let a = s.borrow();
                        let hidden: Vec<String> = a.web.prefs["hiddenPlaylists"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
                        let by: Vec<String> = a.web.prefs["hiddenByYou"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
                        for pl in a.playlists.iter().filter(|p| p.plat == "soundcloud") { println!("selftest: sc playlist {:?} {} hidden {} byyou {} shown {}", pl.title, pl.id, hidden.contains(&pl.id), by.contains(&pl.id), a.side_map.iter().any(|&i| a.playlists[i].id == pl.id)); }
                        println!("selftest: scOn {:?}", a.web.prefs["scOn"]);
                    }
                    let rt = s.borrow().rt.clone();
                    rt.spawn(async move {
                        for path in ["/me/playlists?limit=200&linked_partitioning=true&show_tracks=false", "/me/playlists?limit=50&show_tracks=false", "/me/likes/playlists?limit=50"] {
                            match omnipulse_core::soundcloud::api_get(path.into()).await {
                                Ok(v) => { let n = if v.is_array() { v.as_array().map(|a| a.len()).unwrap_or(0) } else { v["collection"].as_array().map(|a| a.len()).unwrap_or(0) }; println!("selftest: sc {path}: ok, {n} items, keys {:?}", v.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>())); }
                                Err(e) => println!("selftest: sc {path}: error {e}"),
                            }
                        }
                    });
                }
                "askshow" => { ui.set_ask_title("Delete playlist".into()); ui.set_ask_text("Delete \"Balls\" from this PC?".into()); ui.set_ask_ok("Delete".into()); ui.set_ask_open(true); }
                "accentfade" => {
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("artAccent".into(), serde_json::json!(false)); o.insert("accent".into(), serde_json::json!("#ff0000")); }
                    apply_look(&s);
                    for ms in [0u64, 300, 600, 1200] {
                        let w = ui.as_weak();
                        slint::Timer::single_shot(std::time::Duration::from_millis(ms), move || { if let Some(ui) = w.upgrade() { println!("selftest: accent at {ms} ms: {:?}", ui.global::<Theme>().get_accent_own()); } });
                    }
                }
                "findtiles" => { ui.set_search_text("mix".into()); ui.invoke_search_edited("mix".into()); println!("selftest: find on discover: \"{}\"", ui.get_page_find_note()); }
                "searchhouse" => go(&s, Place::Search("house".into())),
                "searchartists" => { ui.invoke_chip(2); println!("selftest: search artists: {} shelves", ui.get_shelves().row_count()); }
                "soundcat" => { ui.invoke_nav("settings".into()); ui.set_set_cat("Sound".into()); }
                "openmix" => panel_open(&s, "mix"),
                "openreplies" => { ui.invoke_nav("settings".into()); ui.set_set_cat("Streaming".into()); panel_open(&s, "replies"); }
                // (the first song with versions: them opened beneath it)
                "vopen" => {
                    let rows = s.borrow().rows.clone();
                    let at = (0..rows.row_count()).find(|&k| rows.row_data(k).is_some_and(|r| r.versions != ""));
                    println!("selftest: versions: {} rows, the first with versions at {at:?}", rows.row_count());
                    if let Some(k) = at { ui.invoke_row_action(k as i32, "versions".into()); }
                }
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
                    // (Seamless Mix on for the test, whatever it is for you: not kept)
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("smartOn".into(), serde_json::json!(true)); if std::env::var_os("OMNIPULSE_SELFTEST_ART").is_some() { o.insert("artAccent".into(), serde_json::json!(true)); } }
                    let next: Vec<Track> = { let a = s.borrow(); let i = a.queue.qi.unwrap_or(0); a.queue.items.iter().skip(i + 1).take(6).map(|q| q.t.clone()).collect() };
                    for t in next { rhythm_ahead(&s, t.clone()); loud_ahead(&s, t); }
                    println!("selftest: mix: measuring the next 6 ahead");
                }
                "mixseek" => {
                    let (cur, qi) = { let a = s.borrow(); (a.queue.current().cloned(), a.queue.qi.unwrap_or(0)) };
                    let Some(cur) = cur else { return };
                    let Some(a) = omnipulse_core::rhythm::known(&cur.key) else { println!("selftest: mix: the one playing isn't measured"); return };
                    let dur = s.borrow().engine.duration_ms();
                    let cands: Vec<(usize, Track)> = { let q = &s.borrow().queue; q.items.iter().enumerate().skip(qi + 1).take(6).map(|(i, x)| (i, x.t.clone())).collect() };
                    // (both measured whole: a blend as Seamless Mix plans it)
                    let fa = rhythm_full(&cur.key);
                    println!("selftest: mix: the one playing measured whole: {}", fa.is_some());
                    let found = cands.into_iter().find_map(|(i, t)| {
                        let (fa, fb) = (fa.as_ref()?, rhythm_full(&t.key)?);
                        let p = smart::blend(&smart::Side { dur_ms: dur, rhythm: Some(fa), ..Default::default() }, &smart::Side { dur_ms: t.ms, rhythm: Some(&fb), ..Default::default() }, 0)?;
                        Some((i, t, fb, p))
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
                "skip" => { skip_press(&s, 1); }
                "rate" => { println!("selftest: rate: {:?}", mix_feedback(&s, false)); }
                // Smart Transitions: on for the test (not kept), the next ones measured ahead
                "smartprep" => {
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("smartOn".into(), serde_json::json!(true)); }
                    let next: Vec<Track> = { let a = s.borrow(); let i = a.queue.qi.unwrap_or(0); a.queue.items.iter().skip(i + 1).take(2).map(|q| q.t.clone()).collect() };
                    let cur = s.borrow().queue.current().cloned();
                    for t in cur.into_iter().chain(next) { rhythm_ahead(&s, t.clone()); loud_ahead(&s, t); }
                    println!("selftest: smart: on, measuring the one playing and the next ahead");
                }
                // (a pair that blends on the beat: OMNIPULSE_SELFTEST_PAIR="A,B", SoundCloud ids)
                "smartpair" => {
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("smartOn".into(), serde_json::json!(true)); }
                    let ids: Vec<String> = std::env::var("OMNIPULSE_SELFTEST_PAIR").unwrap_or_else(|_| "1233870340,315594546".into()).split(',').map(|x| x.trim().to_string()).collect();
                    let (rt, weak) = { let a = s.borrow(); (a.rt.clone(), a.ui.clone()) };
                    rt.spawn(async move {
                        let mut list = Vec::new();
                        for id in ids { if let Some(t) = omnipulse_core::soundcloud::api_get(format!("/tracks/{id}")).await.ok().and_then(|v| Track::soundcloud(&v)) { list.push(t); } }
                        let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| {
                            let Some(s) = s.borrow().clone() else { return };
                            println!("selftest: smart: playing {:?}", list.iter().map(|t| t.title.clone()).collect::<Vec<_>>());
                            for t in &list { rhythm_ahead(&s, t.clone()); loud_ahead(&s, t.clone()); }
                            play_list(&s, list, 0);
                        }));
                    });
                }
                // Track Preferences' Beat 1: a beat earlier, then saved
                // the widget and its alerts on (for the test), its server started
                "widgeton" => {
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() {
                        o.insert("widget".into(), serde_json::json!(true)); o.insert("alertsOn".into(), serde_json::json!(true));
                        // (OMNIPULSE_SELFTEST_TRANS: the alerts' transition, for the test)
                        if let Ok(t) = std::env::var("OMNIPULSE_SELFTEST_TRANS") { o.insert("alertTrans".into(), serde_json::json!(t)); }
                        if std::env::var_os("OMNIPULSE_SELFTEST_LYRICS").is_some() { o.insert("lyricsWidget".into(), serde_json::json!(true)); }
                    }
                    let r = integrations::widget_start(&s.borrow().web.prefs);
                    outside_start(&s);
                    push_out(&s, true);
                    println!("selftest: widget on: {r:?}");
                }
                // a Library song by its title (OMNIPULSE_SELFTEST_TITLE), played
                "playtitle" => {
                    // ("title|artist|platform": its artist and where it's from too, if given)
                    let want = std::env::var("OMNIPULSE_SELFTEST_TITLE").unwrap_or_default().to_lowercase();
                    let mut w = want.split('|');
                    let (title, artist, plat) = (w.next().unwrap_or("").to_string(), w.next().unwrap_or("").to_string(), w.next().unwrap_or("").to_string());
                    // (and a part of its key, to pick one listing of several)
                    let key = w.next().unwrap_or("").to_string();
                    let list: Vec<Track> = s.borrow().library.iter().filter(|t| t.title.to_lowercase().contains(&title) && t.artist.to_lowercase().contains(&artist) && (plat.is_empty() || t.plat() == plat) && t.key.to_lowercase().contains(&key)).take(1).cloned().collect();
                    println!("selftest: playing {:?}", list.iter().map(|t| t.title.clone()).collect::<Vec<_>>());
                    play_list(&s, list, 0);
                }
                // the Lyrics Widget's page with some changes on it, not saved (for its preview)
                "lyrdraft" => {
                    PF_EDIT.with(|e| if let Some((_, v)) = e.borrow_mut().as_mut() {
                        for (k, x) in [("lyricsOutline", serde_json::json!(true)), ("lyricsOutlineSize", serde_json::json!(5)), ("lyricsTextGrad", serde_json::json!(true)), ("lyricsColor2", serde_json::json!("#ffd166")),
                            ("lyricsOwnHi", serde_json::json!(true)), ("lyricsHi", serde_json::json!("#ff4dd2")), ("lyricsHiGrad", serde_json::json!(true)), ("lyricsHi2", serde_json::json!("#4cc9f0")),
                            ("lyricsBox", serde_json::json!(true)), ("lyricsBoxGrad", serde_json::json!(true)), ("lyricsBoxOpacity", serde_json::json!(0.8)), ("lyricsFont", serde_json::json!("bangers")), ("lyricsTransform", serde_json::json!("uppercase"))] { v.insert(k.into(), x); }
                    });
                    println!("selftest: lyrics draft set, saved outline {}", s.borrow().web.prefs["lyricsOutline"]);
                }
                "lyrshow" => { let a = s.borrow(); println!("selftest: lyrics {} lines (timed {}), at {} ms", a.lyrics.len(), a.lyrics.iter().filter(|l| l.ms > 0).count(), a.engine.position_ms()); }
                // Alerts > Game Changes > Test (OMNIPULSE_SELFTEST_ALERT: another kind)
                "gametest" => { let k = std::env::var("OMNIPULSE_SELFTEST_ALERT").unwrap_or_else(|_| "game".into()); panel_press(&s, &format!("alTest:{k}")); println!("selftest: tested {k}"); }
                // the app's memory and CPU (Windows: working set, private bytes, CPU seconds so far)
                "mem" => {
                    #[repr(C)] #[derive(Default)] struct Pmc { cb: u32, faults: u32, peak_ws: usize, ws: usize, qpp: usize, qp: usize, qnpp: usize, qnp: usize, pagefile: usize, peak_pagefile: usize, private: usize }
                    #[repr(C)] #[derive(Default, Clone, Copy)] struct Ft { lo: u32, hi: u32 }
                    #[link(name = "kernel32")] extern "system" { fn GetCurrentProcess() -> isize; fn K32GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32; fn GetProcessTimes(h: isize, c: *mut Ft, e: *mut Ft, k: *mut Ft, u: *mut Ft) -> i32; }
                    let mut p = Pmc { cb: std::mem::size_of::<Pmc>() as u32, ..Default::default() };
                    let (mut c, mut e, mut k, mut u) = (Ft::default(), Ft::default(), Ft::default(), Ft::default());
                    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb); GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u); }
                    let t = |f: Ft| ((f.hi as u64) << 32 | f.lo as u64) as f64 / 1e7;
                    let a = s.borrow();
                    println!("selftest: mem: {:.0} MB working set, {:.0} MB private, {:.1} s CPU; now \"{}\" at {} ms, mixing {}, analysis {} songs, rhythm {} busy", p.ws as f64 / 1e6, p.private as f64 / 1e6, t(k) + t(u), a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.engine.position_ms(), a.engine.mixing(), a.analysis.len(), RHYTHM_FULL_BUSY.lock().map(|b| b.len()).unwrap_or(0));
                }
                // to 25 s before the end of what's playing (a transition soon)
                "nearend" => { let a = s.borrow(); let d = a.engine.duration_ms(); if d > 30_000 { a.engine.seek(d - 25_000); } }
                // the Library's Go+ tracks (20 of them): whether each has a YouTube Music version to play from
                "gpfind" => {
                    let list: Vec<Track> = s.borrow().library.iter().filter(|t| t.go_plus).take(20).cloned().collect();
                    let rt = s.borrow().rt.clone();
                    rt.spawn(async move {
                        let mut miss = Vec::new();
                        for t in &list { if versions::find(t).await.is_none() { miss.push(format!("{} - {}", t.artist, t.title)); } }
                        println!("selftest: go+: {} of {} have no YouTube Music version: {miss:?}", miss.len(), list.len());
                    });
                }
                // a Spotify song from the Library played, two after it (with Smart Transitions on: from YouTube Music)
                "spplay3" => {
                    let list: Vec<Track> = s.borrow().library.iter().filter(|t| matches!(t.src, track::Src::Spotify(_))).take(3).cloned().collect();
                    println!("selftest: spotify: playing {:?}", list.iter().map(|t| t.title.clone()).collect::<Vec<_>>());
                    play_list(&s, list, 0);
                }
                "spcheck3" => {
                    let a = s.borrow();
                    println!("selftest: spotify: now \"{}\" at {} ms, paused {}, from its YouTube Music version {}", a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.engine.position_ms(), a.engine.is_paused(), MIX_OUT_OK.with(|c| c.get()));
                }
                w if w.starts_with("openpanel:") => { ui.invoke_nav("settings".into()); panel_open(&s, &w[10..]); }
                // a click where a pointer would (pressed and let go), in the window's own coordinates
                w if w.starts_with("click:") => {
                    let xy: Vec<f32> = w[6..].split(',').filter_map(|x| x.trim().parse().ok()).collect();
                    if xy.len() == 2 {
                        use slint::platform::{PointerEventButton, WindowEvent};
                        let position = slint::LogicalPosition::new(xy[0], xy[1]);
                        let win = ui.window();
                        win.dispatch_event(WindowEvent::PointerMoved { position });
                        win.dispatch_event(WindowEvent::PointerPressed { position, button: PointerEventButton::Left });
                        win.dispatch_event(WindowEvent::PointerReleased { position, button: PointerEventButton::Left });
                        println!("selftest: clicked {xy:?}");
                    }
                }
                // (how often the window's been drawn: frames since the last "fps", and a second's worth)
                "fps" => {
                    thread_local! { static COUNT: std::cell::RefCell<Option<(std::rc::Rc<std::cell::Cell<u64>>, std::time::Instant)>> = const { std::cell::RefCell::new(None) }; }
                    COUNT.with(|c| {
                        let mut c = c.borrow_mut();
                        match c.as_mut() {
                            None => {
                                let n = std::rc::Rc::new(std::cell::Cell::new(0u64));
                                let n2 = n.clone();
                                let _ = ui.window().set_rendering_notifier(move |state, _| { if matches!(state, slint::RenderingState::BeforeRendering) { n2.set(n2.get() + 1); } });
                                *c = Some((n, std::time::Instant::now()));
                                println!("selftest: fps counting");
                            }
                            Some((n, since)) => {
                                let secs = since.elapsed().as_secs_f64().max(0.001);
                                println!("selftest: fps {:.1} ({} frames in {secs:.1} s)", n.get() as f64 / secs, n.get());
                                n.set(0);
                                *since = std::time::Instant::now();
                            }
                        }
                    });
                }
                // (the settings changed on the open page and not saved yet: which, and to what)
                "pfedits" => {
                    let changed: Vec<String> = PF_EDIT.with(|e| e.borrow().as_ref().map(|(_, v)| v.iter().filter(|(k, _)| pf_changed(k)).map(|(k, x)| format!("{k}={x}")).collect()).unwrap_or_default());
                    println!("selftest: unsaved {changed:?}");
                }
                "pfshow" => {
                    let f = ui.get_pf_fields();
                    let v: Vec<String> = (0..f.row_count()).filter_map(|i| f.row_data(i)).filter(|x| x.kind == "select").map(|x| format!("{}={}", x.key, x.at)).collect();
                    println!("selftest: dropdowns now {v:?}");
                }
                // a page's every dropdown: its second option picked (as a click does), and whether it shows it
                w if w.starts_with("selects:") => {
                    let name = &w[8..];
                    panel_open(&s, name);
                    let n = ui.get_pf_fields().row_count();
                    for i in 0..n {
                        let Some(f) = ui.get_pf_fields().row_data(i) else { continue };
                        if f.kind != "select" { continue; }
                        let want = if f.at == 1 { 0 } else { 1 };
                        ui.invoke_pf_set(i as i32, format!("\u{2}{want}").into());
                        let after = ui.get_pf_fields().row_data(i);
                        let ok = after.as_ref().is_some_and(|x| x.key == f.key && x.at == want);
                        println!("selftest: selects: {name}.{} ({} options) {} -> {} {}", f.key, f.options.row_count(), f.at, after.map_or(-1, |x| x.at), if ok { "ok" } else { "STUCK" });
                    }
                }
                // Settings > Streaming > Stream Widget, and its Alerts switch pressed (as a click does)
                "openwidget" => { ui.invoke_nav("settings".into()); ui.set_set_cat("Streaming".into()); panel_open(&s, "widget"); }
                "alerton" => {
                    let f = ui.get_pf_fields();
                    if let Some(i) = (0..f.row_count()).find(|&i| f.row_data(i).is_some_and(|x| x.key == "alertsOn")) {
                        let now = f.row_data(i).unwrap().value == "true";
                        ui.invoke_pf_set(i as i32, if now { "false" } else { "true" }.into());
                    }
                }
                "alertshow" => {
                    let f = ui.get_pf_fields();
                    let shown = (0..f.row_count()).find_map(|i| f.row_data(i).filter(|x| x.key == "alertsOn").map(|x| x.value.to_string()));
                    println!("selftest: alerts: switch shows {shown:?}, setting {}", set_row_on(&s.borrow().web.prefs, "alertsOn", false));
                }
                // the queue as the panel shows it: each row's heading and title
                "qshow" => {
                    let a = s.borrow();
                    let rows: Vec<String> = a.qrows.iter().take(8).map(|r| format!("[{}] {}", r.head, r.title)).collect();
                    println!("selftest: queue (page {}, {} items, playing {:?}): {rows:?}", a.page, a.queue.items.len(), a.queue.qi);
                }
                // a song from Library added to the queue (as its menu's Add to Queue)
                "qadd" => {
                    let t = s.borrow().library.iter().rev().find(|t| !s.borrow().queue.items.iter().any(|q| q.t.key == t.key)).cloned();
                    if let Some(t) = t { println!("selftest: adding \"{}\"", t.title); queue_add(&s, t, false); }
                }
                "beatearlier" => { ui.set_tp_beat(ui.get_tp_beat() - 1); ui.set_tp_phrase(ui.get_tp_phrase() - 1); println!("selftest: beat 1 shown {}, at {}, phrases {}", ui.get_tp_beat_shown(), ui.get_tp_beat(), ui.get_tp_phrase()); }
                "tpsave" => {
                    ui.invoke_tp_save();
                    let key = s.borrow().queue.current().map(|t| t.key.clone()).unwrap_or_default();
                    println!("selftest: beat 1 saved: {} (for {key}), prefs {}; phrases {}", beat_shift(&s.borrow().web.prefs, &key), s.borrow().web.prefs["beatShifts"], phrase_shift(&s.borrow().web.prefs, &key));
                }
                "smartpage" => {
                    panel_open(&s, "smart");
                    let vals = panels::values("smart", &s.borrow().web.prefs);
                    let f: Vec<String> = panels::fields("smart", &vals, &s.borrow().web.prefs, "").iter().map(|f| format!("{} ({})", f["label"].as_str().unwrap_or("").chars().take(40).collect::<String>(), f["type"].as_str().unwrap_or(""))).collect();
                    println!("selftest: smart page \"{}\": {f:?}; its row in Settings: {}", panels::title("smart"), SET_ROWS.iter().any(|r| r.3 == "smartOn"));
                }
                // (to just before the transition as planned)
                "smartseek" => {
                    let dur = s.borrow().engine.duration_ms();
                    let Some((a, b)) = smart_pair(&s.borrow(), dur) else { println!("selftest: smart: nothing next"); return };
                    let f = smart::fade(&a.side(), &b.side(), smart_max_ms(&s.borrow().web.prefs), SMART_FIXED_MS);
                    let pos = s.borrow().engine.position_ms();
                    let blend = smart_decide(&s, &a, &b, pos, false);
                    let why = SMART_PAIR.with(|x| x.borrow().as_ref().map(|x| x.3.clone()));
                    println!("selftest: smart: \"{}\" ({:?} BPM, whole {}) into \"{}\" ({:?} BPM, whole {}); fade {f:?}; blend {:?}; decided {why:?}", a.t.title, a.rhythm.as_ref().map(|m| m.rhythm.bpm), a.rhythm.is_some(), b.t.title, b.rhythm.as_ref().map(|m| m.rhythm.bpm), b.rhythm.is_some(), blend.as_ref().map(|p| (p.at_ms, p.ms, p.rate)));
                    let to = blend.map(|p| p.at_ms).unwrap_or(f.out_from_ms).saturating_sub(9000);
                    s.borrow().engine.seek(to);
                    println!("selftest: smart: sought to {to} ms of {dur}");
                }
                "smartcheck" => {
                    let a = s.borrow();
                    let phase = MIX.with(|m| match &*m.borrow() { None => "none".to_string(), Some(MixPhase::Opening(_)) => "opening".into(), Some(MixPhase::Waiting { .. }) => "waiting".into(), Some(MixPhase::Mixing { tempo, .. }) => format!("mixing at {:.4}x", tempo.get()) });
                    println!("selftest: smart: mix {phase}, crossfading {} mixing {}, now \"{}\" at {} ms, why {:?}", a.engine.crossfading(), a.engine.mixing(), a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.engine.position_ms(), MIX_WHY.with(|m| m.borrow().as_ref().map(|x| x.2.clone())));
                }
                "mixcheck" => {
                    let a = s.borrow();
                    let phase = MIX.with(|m| match &*m.borrow() { None => "none".to_string(), Some(MixPhase::Opening(_)) => "opening".into(), Some(MixPhase::Waiting { .. }) => "waiting".into(), Some(MixPhase::Mixing { tempo, .. }) => format!("mixing at {:.4}x", tempo.get()) });
                    let ear = "";
                    println!("selftest: mix: {phase}, engine crossfading {} mixing {}, now \"{}\" at {} ms; {ear}", a.engine.crossfading(), a.engine.mixing(), a.queue.current().map(|t| t.title.clone()).unwrap_or_default(), a.engine.position_ms());
                    let lit: Vec<String> = a.qrows.iter().filter(|r| r.coming).map(|r| r.title.to_string()).collect();
                    let playing: Vec<String> = a.qrows.iter().filter(|r| r.playing).map(|r| r.title.to_string()).collect();
                    println!("selftest: mix shown: \"{}\" at {} / {}, queue playing {playing:?}, lit {lit:?}, accent pulsing {}", ui.get_now_title(), ui.get_pos_text(), ui.get_dur_text(), ui.global::<Theme>().get_pulsing());
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

    // (run until quit, not until the window hides: with Close to Tray the app stays in the tray)
    ui.show()?;
    // (a self-test's window off the screen: its pictures are taken all the same, and it's never in your way)
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() && std::env::var_os("OMNIPULSE_SELFTEST_ONSCREEN").is_none() {
        ui.window().set_position(slint::PhysicalPosition::new(-4000, 0));
    }
    slint::run_event_loop_until_quit()?;
    let _ = ui.hide();
    // (the session, if a write was still waiting)
    queue::flush_session();
    // the window closed: OmniPulse ends, the music with it. (Not waiting on the background work: a plugin's
    // connection or a long fetch kept the process, and the music, going after the window had gone)
    rt.shutdown_timeout(std::time::Duration::from_millis(800));
    std::process::exit(0)
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

#[cfg(all(test, windows))]
mod speech_tests {
    // (Windows' speech: words made into a sound file, never played here)
    #[test]
    fn speech_makes_a_sound_file() {
        let p = super::speech_file("Thanks for the follow", "", 1.2).unwrap();
        let b = std::fs::read(&p).unwrap();
        assert!(b.len() > 10_000 && &b[..4] == b"RIFF" && &b[8..12] == b"WAVE", "{} bytes", b.len());
    }
}
