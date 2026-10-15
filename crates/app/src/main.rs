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
mod lang;
mod panels;
mod hide;
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
    let chip = app.chip;
    let prefs = app.web.prefs.clone();
    let favs: std::collections::HashSet<&str> = app.web.favourites.iter().map(|t| t.key.as_str()).collect();
    let fav_only = app.fav_only;
    // (as the web app's js/pagefind.js: word by word, in the title, artist or album)
    let want = sources::words(&app.page_query);
    let found = |t: &Track| want.is_empty() || { let have = sources::words(&format!("{} {} {}", t.title, t.artist, t.album)); want.iter().all(|w| have.iter().any(|h| h.starts_with(w.as_str()))) };
    // (Search, with Hide likes: what you've liked left out)
    let mine: std::collections::HashSet<&str> = if app.page == "search" && prefs["hideLikes"].as_bool() == Some(true) { app.library.iter().map(|t| t.key.as_str()).collect() } else { Default::default() };
    let shown = |t: &&Track| plat_on(&prefs, t.plat()) && (!fav_only || favs.contains(t.key.as_str())) && !mine.contains(t.key.as_str()) && !hide::no_version(&prefs, t) && streamsafe::allowed(&prefs, t) && found(t);
    let mut filtered: Vec<Track> = match (app.page.as_str(), chip) {
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 1) => app.page_all.iter().filter(shown).filter(|t| !t.is_mix()).cloned().collect(),
        ("library" | "rediscover" | "favourites" | "search" | "playlist", 2) => app.page_all.iter().filter(shown).filter(|t| t.is_mix()).cloned().collect(),
        _ => app.page_all.iter().filter(shown).cloned().collect(),
    };
    if app.page == "library" {
        lib_sort(&prefs, &mut filtered);
    }
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
    app.rows.set_vec(filtered.iter().map(|t| Row { versions: ver_label.get(&t.key).cloned().unwrap_or_default().into(), version: ver_rows.contains(&t.key), ..row_menu(app, &marks, t, Row { picked: app.picked.contains(&t.key), ..t.row(t.key == playing) }) }).collect::<Vec<_>>());
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
    // (the chip last picked on this page)
    let kept = if chips { FILTER_KEYS.iter().position(|k| app.web.prefs["pageFilters"][filter_view(page)].as_str() == Some(k)).unwrap_or(0) } else { 0 };
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
            let (mut pool, lp) = { let a = shared.borrow(); (a.library.clone(), a.web.last_played.clone()) };
            let cut = now_ms().saturating_sub(MONTH_MS);
            pool.retain(|t| lp.get(&t.key).copied().unwrap_or(0) < cut);
            { let prefs = shared.borrow().web.prefs.clone(); pool.retain(|t| hide::rec_ok(&prefs, t)); }
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
        // (the ones you hid, with Hidden & Not Recommended on)
        .filter(|&i| !hide::on(&prefs) || !prefs["hiddenByYou"].as_array().is_some_and(|l| l.iter().any(|x| x.as_str() == Some(a.playlists[i].id.as_str()))))
        .collect();
    let sides: Vec<Side> = a.side_map.iter().map(|&i| Side { title: a.playlists[i].title.clone().into(), plat: if xpl::is_own(&a.playlists[i].id) { "xpl".into() } else { a.playlists[i].plat.into() }, own: xpl::is_own(&a.playlists[i].id), share: a.playlists[i].plat != "local" || xpl::is_own(&a.playlists[i].id), sc: share::platform_id(&a.playlists[i], &a.xpl, "soundcloud").is_some(), sp: share::platform_id(&a.playlists[i], &a.xpl, "spotify").is_some(), collab: collab_of(&a.xpl, &a.playlists[i].id).is_some(), collab_owner: collab_of(&a.xpl, &a.playlists[i].id).is_some_and(|c| c["owner"].as_bool() == Some(true)), merged: a.xpl.as_array().is_some_and(|l| l.iter().any(|x| x["id"].as_str() == Some(a.playlists[i].id.as_str()) && x["merged"].as_array().is_some_and(|m| !m.is_empty()))) }).collect();
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
            let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().map(|(_, l)| Rc::new(VecModel::from(l.iter().map(|f| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into(), genre: false, c1: Default::default(), c2: Default::default(), track: false, liked: false, row: Default::default() } }).collect::<Vec<_>>()))).collect();
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
            let model = Rc::new(VecModel::from(list.iter().map(|f| { let (t, sub, _, plat) = f.tile(); Tile { title: t.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into(), genre: false, c1: Default::default(), c2: Default::default(), track: false, liked: false, row: Default::default() } }).collect::<Vec<_>>()));
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
    let everywhere = shared.borrow().web.prefs["followEverywhere"].as_bool() != Some(false) && std::env::var_os("OMNIPULSE_SELFTEST").is_none();
    rt.spawn(async move {
        let mut list = sources::followed_artists().await;
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
    let Some(a) = at.and_then(|i| shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned())) else {
        // (by name: Follow, where they were found)
        let found = NAMED.with(|n| n.borrow().clone()).filter(|n| !n.plats().is_empty());
        ui.set_follow_label(if found.is_some() { "Follow".into() } else { "".into() });
        ui.set_follow_on(false);
        if let Some(n) = &found {
            if n.sc.is_some() { ui.set_follow_sc("Follow on SoundCloud".into()); }
            if n.sp.is_some() { ui.set_follow_sp("Follow on Spotify".into()); }
        }
        ui.set_follow_more("".into());
        return;
    };
    ui.set_follow_sc(if a.sc.is_some() { "Unfollow on SoundCloud".into() } else if omnipulse_core::soundcloud::signed_in() { "Follow on SoundCloud".into() } else { "".into() });
    ui.set_follow_sp(if a.sp.is_some() { "Unfollow on Spotify".into() } else if omnipulse_core::spotify::spotify_signed_in() { "Follow on Spotify".into() } else { "".into() });
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
        else if a.sp.is_none() && !a.plats().is_empty() && omnipulse_core::spotify::spotify_signed_in() { "spotify" } else { "" };
    ui.set_follow_more(more.into());
}

/// The artist's entry in the Artists list changed
fn artist_set(shared: &Shared, i: usize, f: impl FnOnce(&mut sources::Artist)) {
    if let Some(a) = shared.borrow_mut().artists.as_mut().and_then(|l| l.get_mut(i)) { f(a); }
    follow_paint(shared);
}

/// Follow on an artist's page opened by name: on the platforms they were found on (only `only`, if given),
/// then they're in your Artists
fn follow_named(shared: &Shared, only: Option<&'static str>) {
    let Some(n) = NAMED.with(|x| x.borrow().clone()) else { return };
    let plats: Vec<(&'static str, String)> = [n.sc.map(|x| ("soundcloud", x.to_string())), n.sp.clone().map(|x| ("spotify", x))].into_iter().flatten().filter(|(p, _)| only.is_none_or(|o| o == *p)).collect();
    let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
    rt.spawn(async move {
        let mut done: Vec<(&'static str, String)> = Vec::new();
        for (p, id) in &plats { if sources::set_follow(p, id, true).await.is_ok() { done.push((*p, id.clone())); } }
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            if done.is_empty() { ui.set_status(format!("Couldn't follow {}.", n.name).into()); return; }
            let a = sources::Artist { name: n.name.clone(), img: n.img.clone(), sc: done.iter().find(|d| d.0 == "soundcloud").and_then(|d| d.1.parse().ok()), sp: done.iter().find(|d| d.0 == "spotify").map(|d| d.1.clone()) };
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
    let plat: &'static str = if plat == "spotify" { "spotify" } else { "soundcloud" };
    let at = FOLLOW.with(|f| f.borrow().0);
    let Some(i) = at else { return follow_named(shared, Some(plat)) };
    let Some(a) = shared.borrow().artists.as_ref().and_then(|l| l.get(i).cloned()) else { return };
    let have = if plat == "soundcloud" { a.sc.map(|x| x.to_string()) } else { a.sp.clone() };
    match have {
        // (followed there: unfollowed there only)
        Some(id) => {
            let (rt, weak) = { let x = shared.borrow(); (x.rt.clone(), x.ui.clone()) };
            rt.spawn(async move {
                let r = sources::set_follow(plat, &id, false).await;
                let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                    let Some(sh) = s.borrow().clone() else { return };
                    match r {
                        Ok(()) => { artist_set(&sh, i, |x| if plat == "soundcloud" { x.sc = None; } else { x.sp = None; }); ui.set_status(format!("Unfollowed {} on {}.", a.name, plat_name(plat)).into()); }
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
                artist_set(&sh, i, |x| for (p, id) in &back { if *p == "soundcloud" { x.sc = id.parse().ok(); } else { x.sp = Some(id.clone()); } });
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
    let plats: Vec<(&'static str, String)> = [a.sc.map(|n| ("soundcloud", n.to_string())), a.sp.clone().map(|id| ("spotify", id))].into_iter().flatten().collect();
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
            artist_set(&sh, i, |x| for (p, _) in &done { if *p == "soundcloud" { x.sc = None; } else { x.sp = None; } });
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
        let plat: &'static str = if plat == "spotify" { "spotify" } else { "soundcloud" };
        if let Some(ui) = weak.upgrade() { ui.set_status(format!("Looking for {} on {}...", a.name, plat_name(plat)).into()); }
        rt.spawn(async move {
            let r = match sources::find_artist_on(plat, &a.name).await {
                None => Err(format!("{} isn't on {} (by that name).", a.name, plat_name(plat))),
                Some(id) => sources::set_follow(plat, &id, true).await.map(|_| id).map_err(|e| format!("Couldn't follow on {}: {e}", plat_name(plat))),
            };
            let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                let Some(sh) = s.borrow().clone() else { return };
                match r {
                    Ok(id) => { artist_set(&sh, i, |x| if plat == "soundcloud" { x.sc = id.parse().ok(); } else { x.sp = Some(id.clone()); }); ui.set_status(format!("Following {} on {}.", a.name, plat_name(plat)).into()); }
                    Err(e) => ui.set_status(e.into()),
                }
            }));
        });
        return;
    }
    // Unfollow Everywhere: asked first (as the web app's)
    let on: Vec<&str> = a.plats().into_iter().map(plat_name).collect();
    let sure = rfd::MessageDialog::new().set_title("Unfollow").set_description(format!("Unfollow {} on {}?", a.name, on.join(" and "))).set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
    if !sure { return; }
    let plats: Vec<(&'static str, String)> = [a.sc.map(|n| ("soundcloud", n.to_string())), a.sp.clone().map(|id| ("spotify", id))].into_iter().flatten().collect();
    rt.spawn(async move {
        let mut done = Vec::new();
        for (p, id) in &plats { if sources::set_follow(p, id, false).await.is_ok() { done.push(*p); } }
        let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
            let Some(sh) = s.borrow().clone() else { return };
            artist_set(&sh, i, |x| for p in &done { if *p == "soundcloud" { x.sc = None; } else { x.sp = None; } });
            ui.set_status(format!("Unfollowed {}.", a.name).into());
        }));
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
                    let _ = weak.upgrade_in_event_loop(move |_| SHARED.with(|s| { if let Some(sh) = s.borrow().clone() { artist_set(&sh, i, |x| if to == "soundcloud" { x.sc = id.parse().ok(); } else { x.sp = Some(id.clone()); }); } }));
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
    let plat = if a.sc.is_none() { "soundcloud" } else { "spotify" };
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
                    artist_set(&sh, i, |x| if plat == "soundcloud" { x.sc = id.parse().ok(); } else { x.sp = Some(id.clone()); });
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
    load_into(shared, "artist", a.name.clone(), async move {
        let (t, dz, bio) = tokio::join!(sources::artist_tracks(&a), artist_top(&a.name, cat), artist_bio(&a.name));
        let t = sources::once_each(sources::interleave(vec![t, dz]));
        let sub = format!("{bio}You follow them on {on}. {}.", plural(t.len(), "track", "tracks"));
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
    let (cat, sc_on, sp_on) = { let p = &shared.borrow().web.prefs; (catalog_on(p), plat_on(p, "soundcloud"), plat_on(p, "spotify")) };
    let name = name.to_string();
    load_into(shared, "artist", name.clone(), async move {
        let (sc, sp) = tokio::join!(
            async { if sc_on { sources::find_artist_on("soundcloud", &name).await.and_then(|x| x.parse().ok()) } else { None } },
            async { if sp_on { sources::find_artist_on("spotify", &name).await } else { None } },
        );
        let a = sources::Artist { name: name.clone(), img: String::new(), sc, sp };
        {
            let found = a.clone();
            let _ = slint::invoke_from_event_loop(move || SHARED.with(|s| { NAMED.with(|n| *n.borrow_mut() = Some(found)); if let Some(sh) = s.borrow().clone() { follow_paint(&sh); } }));
        }
        let on = a.plats().into_iter().map(|p| if p == "spotify" { "Spotify" } else { "SoundCloud" }).collect::<Vec<_>>().join(" and ");
        let (t, dz, bio) = tokio::join!(sources::artist_tracks(&a), artist_top(&name, cat), artist_bio(&name));
        let t = sources::once_each(sources::interleave(vec![mine.clone(), t, dz]));
        let liked = if mine.is_empty() { String::new() } else { format!("{} you like first. ", plural(mine.len(), "song", "songs")) };
        let sub = format!("{bio}{liked}{}{}.", if on.is_empty() { String::new() } else { format!("From {on} and more. ") }, plural(t.len(), "track", "tracks"));
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
                    Tile { title: title.into(), sub: sub.into(), art: slint::Image::default(), plat: plat.into(), genre, c1, c2, track: is_track, liked, row }
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
        discover::Item::Mix(m) => {
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
        ui.set_tp_open(true);
    }
    tp_bar(shared);
    tp_versions(shared, t);
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
    let models: Vec<Rc<VecModel<Tile>>> = shelves.iter().map(|(_, l)| Rc::new(VecModel::from(l.iter().map(|(n, c)| Tile { title: n.clone().into(), sub: sub(c).into(), art: slint::Image::default(), plat: "".into(), genre: false, c1: Default::default(), c2: Default::default(), track: false, liked: false, row: Default::default() }).collect::<Vec<_>>()))).collect();
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
            let pname = |p: &str| match p { "spotify" => "Spotify", "youtube" => "YouTube Music", "local" => "Your files", "soundcloud" => "SoundCloud", other => other }.to_string();
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
    let pname = |p: &str| match p { "spotify" => "Spotify", "youtube" => "YouTube Music", "local" => "Your files", _ => "SoundCloud" };
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
/// A song's menu's choice, wherever it was right-clicked (a tile, Stats, the queue; a page's rows have
/// their own, for what's picked): to the queue, a playlist, not recommended, a link, and the rest
fn menu_act(shared: &Shared, t: Track, act: &str) {
    if act == "next" || act == "add" {
        let empty = shared.borrow().queue.current().is_none();
        { let mut a = shared.borrow_mut(); if act == "next" { a.queue.play_next(t.clone()); } else { a.queue.add(t.clone()); } paint_queue(&mut a); save_session(&a); }
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("{} \"{}\".", if act == "next" { "Playing next:" } else { "Added to the queue:" }, t.title).into()); }
        if empty { start(shared); }
        return;
    }
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
        a if a.starts_with("artist:") => {
            let k: usize = a[7..].parse().unwrap_or(0);
            let name = t.artist.split([',', '&']).map(|x| x.trim()).filter(|x| !x.is_empty()).nth(k).unwrap_or(&t.artist).to_string();
            go(shared, Place::ArtistNamed(name));
        }
        _ => return false,
    }
    true
}

fn paint_queue(a: &mut App) {
    // (a mix under way: the queue as it was, the one going out playing, the one coming in lit)
    let real = a.queue.qi;
    let shown = mix_show_qi().filter(|&p| p < a.queue.items.len());
    if shown.is_some() { a.queue.qi = shown; }
    let coming = shown.and(real);
    paint_queue_rows(a, coming);
    a.queue.qi = real;
}

fn paint_queue_rows(a: &mut App, coming: Option<usize>) {
    let panel = a.queue.panel();
    let marks = marks_of(a);
    let map: Vec<usize> = panel.iter().map(|p| p.0).collect();
    let rows: Vec<Row> = panel
        .iter()
        .map(|(i, q, head)| {
            let mut r = row_menu(a, &marks, &q.t, q.t.row(Some(*i) == a.queue.qi));
            r.head = (*head).into();
            r.auto = q.auto;
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
        "Off: playback stops at the end of your queue.".into()
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

fn save_session(a: &App) {
    let pos = a.resume_ms.unwrap_or_else(|| a.engine.position_ms());
    let mut v = a.queue.to_session(pos);
    // (your usual autoplay: the stream's is kept as apStream)
    v["autoplay"] = serde_json::json!(AP_USUAL.with(|u| u.get()).unwrap_or(a.autoplay));
    queue::save_session(&v);
}

// ---------- playing ----------
fn start(shared: &Shared) {
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
    let mix_on_now = integrations::on(&shared.borrow().web.prefs, "mixOn");
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
        } else if t.go_plus || mb {
            match versions::find(&t).await {
                Some(v) => (player::open_source(&Track::youtube(&v.yt), h.clone()).await.map(Some), Some(v)),
                None if matches!(t.src, track::Src::Spotify(_)) && sp_plays_here() => (player::open_source(&t, h.clone()).await.map(Some), None),
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
                        let db = sound::loud_db(&shared.borrow().web.prefs, &t, known, version.is_some());
                        // the track before ending with Fade Between Tracks on: both heard at once, this one coming in
                        let resumed = resume > 0 || trim_of(&shared.borrow().web.prefs, &t.key).0 > 0;
                        match (src, XFADE.with(|x| x.borrow_mut().take())) {
                            // (mixed in: playing already)
                            (None, _) => {}
                            (Some(src), Some((ms, at))) if !resumed && at.elapsed().as_secs() < 8 => {
                                engine.fade(None, 1.0, 200);
                                engine.crossfade(src, db, ms);
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
    ui.set_lyrics_note(if lyrics.is_empty() { "No lyrics found for this one.".into() } else if lyrics.iter().all(|l| l.ms == 0) { "These lyrics aren't timed.".into() } else { "".into() });
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

/// The next track's loudness measured ahead (seamless mixing on): a mix brings it in at its own match,
/// not a guess that moves once it's measured. Of what the mix plays (its YouTube Music version, if it
/// plays as that); kept as a measurement as it plays is.
fn loud_ahead(shared: &Shared, t: Track) {
    if sound::loud_step(&shared.borrow().web.prefs) == "off" || t.is_mix() { return; }
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
    /// (the one playing's tempo, when the mix slows it into the next one's)
    /// (its key shift, if it has one: held through the mix, then bent back to its own key)
    Waiting { key: String, plan: mix::Plan, tempo: omnipulse_audio::Tempo, version: Option<versions::Version>, out: Option<omnipulse_audio::Tempo>, pitch: Option<omnipulse_audio::Tempo> },
    /// coming in (and then easing back to its own tempo); the one going out slowed as it goes, if it is
    Mixing { plan: mix::Plan, tempo: omnipulse_audio::Tempo, eased_from: Option<std::time::Instant>, ease_ms: u64, out: Option<omnipulse_audio::Tempo>, began: std::time::Instant, pitch: Option<omnipulse_audio::Tempo> },
}

thread_local! {
    /// Wordplay's timed lyrics, by track (empty: none that fit), and those being found
    static MIX_WORDS: RefCell<HashMap<String, Vec<(u64, String)>>> = RefCell::new(HashMap::new());
    static MIX_WORDS_BUSY: RefCell<std::collections::HashSet<String>> = RefCell::new(std::collections::HashSet::new());
    static MIX: RefCell<Option<MixPhase>> = const { RefCell::new(None) };
    /// a mix's key shift bending back to the track's own key once the mix is over: (its pitch, where it
    /// was, since when, over how long)
    static PITCH_BACK: RefCell<Option<(omnipulse_audio::Tempo, f64, std::time::Instant, u64)>> = const { RefCell::new(None) };
    /// the transition of its own used last (Wordplay, Drop Swap, Melody Play), so the next is another
    static MIX_STYLE_LAST: std::cell::Cell<Option<&'static str>> = const { std::cell::Cell::new(None) };
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

fn mix_kind(p: &mix::Plan) -> &'static str {
    if let Some(s) = p.style { s } else if p.tape_stop { "tape stop" } else if p.out_loop.is_some() || p.in_loop.is_some() { "looped" } else if p.beatmatched { "on the beat" } else { "blend" }
}

fn audit_began(shared: &Shared, plan: &mix::Plan, what: String) {
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
            // (the one going out slowed into the next one's tempo as far through as the mix is, and the next
            // one with it, from the other's tempo to its own: at its own by the end, nothing to ease back)
            if let Some(o) = out.clone() {
                let x = began.elapsed().as_millis() as f64 / plan.ramp_ms.max(1) as f64;
                o.set(mix::out_ramp(plan.out_tempo, x));
                tempo.set(mix::in_ramp(plan.out_tempo, x));
                if engine.crossfading() { return true; }
                tempo.set(1.0);
                pitch_back_start(pitch, *ease_ms);
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
            tempo.set(mix::ease(plan.rate, t, *ease_ms));
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
    let (on, listening_now, next, cur) = {
        let a = shared.borrow();
        let next = if a.repeat == "one" { None } else { a.queue.qi.and_then(|i| a.queue.items.get(i + 1)).map(|q| q.t.clone()) };
        (integrations::on(&a.web.prefs, "mixOn"), listening().is_some(), next, a.queue.current().cloned())
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
    if !on || listening_now || !MIX_OUT_OK.with(|c| c.get()) || dur == 0 || next.is_mix() { why(&format!("on {on}, listening {listening_now}, out ok {}, dur {dur}, next is a set {}", MIX_OUT_OK.with(|c| c.get()), next.is_mix())); return; }
    // (Stream-Safe Music: never mixed into one it doesn't allow; the usual way skips it)
    if !streamsafe::allowed(&shared.borrow().web.prefs, &next) { why("Stream-Safe Music doesn't allow the next one"); return; }
    // (where its sound ends: measured, if it's before the file's end)
    let audible = shared.borrow().cur_loud.as_ref().and_then(|k| shared.borrow().analysis.get(k).and_then(|x| x["audible_ms"].as_u64())).filter(|&x| x > 0 && x < dur).unwrap_or(dur);
    let planning = std::time::Instant::now();
    // Wordplay (Settings > Sound): both songs' timed lyrics, for the recordings as measured
    // Settings > Sound > Seamless Mix: its styles, its blend, and (testing) one style forced
    let force = shared.borrow().web.prefs["mixForce"].as_str().unwrap_or("auto").to_string();
    let allows = |style: &str| force == "auto" || force == style;
    {
        let p = &shared.borrow().web.prefs;
        use std::sync::atomic::Ordering::Relaxed;
        mix::RUN_BARS.store(p["mixBars"].as_u64().unwrap_or(0) as usize, Relaxed);
        mix::KEYS_OK.store(set_row_on(p, "mixKeys", true), Relaxed);
        mix::LOOPS_OK.store(set_row_on(p, "mixLoops", true), Relaxed);
        mix::FORCE_TAPE.store(force == "tape", Relaxed);
        mix::BEAT_ONLY.store(force == "beat", Relaxed);
    }
    let words_on = set_row_on(&shared.borrow().web.prefs, "mixWords", true) && allows("wordplay");
    let wordplay = if words_on { rhythm_full(&cur.key).zip(rhythm_full(&next.key)) } else { None }.and_then(|(a, b)| {
        let lines = |t: &Track, m: &omnipulse_core::rhythm::Measured| -> Option<Vec<(u64, String)>> {
            if let Some(l) = MIX_WORDS.with(|w| w.borrow().get(&t.key).cloned()) { return Some(l); }
            if MIX_WORDS_BUSY.with(|b| !b.borrow_mut().insert(t.key.clone())) { return None; }
            let (t, secs, rt) = (t.clone(), (m.to_ms.saturating_sub(m.from_ms)) / 1000, shared.borrow().rt.clone());
            let weak = shared.borrow().ui.clone();
            rt.spawn(async move {
                let l = player::mix_lyrics(&t, secs).await;
                let _ = weak.upgrade_in_event_loop(move |_| {
                    MIX_WORDS.with(|w| w.borrow_mut().insert(t.key.clone(), l));
                    MIX_WORDS_BUSY.with(|b| b.borrow_mut().remove(&t.key));
                });
            });
            None
        };
        let (la, lb) = (lines(&cur, &a), lines(&next, &b));
        mix::plan_wordplay(&a, &la?, audible, pos, &b, &lb?)
    });
    // Tape Stops (Settings > Sound): off, none planned
    mix::TAPE_OK.store(set_row_on(&shared.borrow().web.prefs, "mixTape", true), std::sync::atomic::Ordering::Relaxed);
    // Drop Swap and Melody Play (Settings > Sound); of the ones there are, not the one used last time
    let (drop_on, melody_on) = { let p = &shared.borrow().web.prefs; (set_row_on(p, "mixDrop", true) && allows("drop"), set_row_on(p, "mixMelody", true) && allows("melody")) };
    let both = rhythm_full(&cur.key).zip(rhythm_full(&next.key));
    let special = {
        let mut all: Vec<mix::Plan> = wordplay.into_iter().collect();
        if let Some((a, b)) = &both {
            if drop_on { all.extend(mix::plan_drop_swap(a, audible, pos, b)); }
            if melody_on { all.extend(mix::plan_melody(a, audible, pos, b)); }
        }
        let last = MIX_STYLE_LAST.with(|l| l.get());
        let i = all.iter().position(|p| p.style != last).unwrap_or(0);
        (!all.is_empty()).then(|| all.swap_remove(i))
    };
    // both measured whole: every beat tracked, so no listening again
    // (a style of its own forced, for testing: that one, or no mix at all, never another)
    let special_forced = matches!(force.as_str(), "wordplay" | "drop" | "melody");
    if special_forced && special.is_none() { why(&format!("{force} forced, and this pair can't be mixed that way")); return; }
    let whole = special.or_else(|| both.and_then(|(a, b)| mix::plan_structure(&a, audible, pos, &b)));
    let (plan, local) = match whole {
        Some(p) => (p, Some(())),
        None => {
            let (Some(a), Some(b)) = (omnipulse_core::rhythm::known(&cur.key), omnipulse_core::rhythm::known(&next.key)) else { why("not both measured"); return };
            // (the playing one's beat as measured again near its end, once it has been)
            let local = MIX_LOCAL.with(|l| l.borrow().as_ref().filter(|x| x.0 == cur.key).map(|x| x.1.clone()));
            let a = local.clone().unwrap_or(a);
            if force == "tape" { why("a tape stop forced: both songs need measuring whole first"); return; }
            let Some(plan) = mix::plan(&a, audible, pos, &b, MIX_BARS) else { why("no plan from the grid"); return };
            (plan, local.map(|_| ()))
        }
    };
    MIX_AUDIT.with(|a| { let mut a = a.borrow_mut(); a.plan_ns += planning.elapsed().as_nanos() as u64; a.plans += 1; });
    // listened to again first (measured only in part), then got ready about seven seconds ahead of its moment
    if local.is_none() {
        mix_ear(shared, &cur.key, pos, plan.at_ms);
        // (too late to listen again: as it was measured)
        if pos + MIX_EAR_MS + 6000 < plan.at_ms || MIX_EAR.with(|e| e.borrow().is_some()) { why(&format!("listening again first (plan at {})", plan.at_ms)); return; }
    }
    if pos + 7000 < plan.at_ms { why(&format!("waiting for {} ({})", plan.at_ms, mix_kind(&plan))); return; }
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
                    let out = (plan.out_tempo != 1.0).then(|| omnipulse_audio::Tempo::new(1.0));
                    if let Some(o) = &out { shared.borrow().engine.stretch_playing(o.clone()); }
                    shared.borrow().engine.mix_at(src, db, plan.at_ms, plan.ms, Some(mix::handoff(&plan, set_row_on(&prefs, "mixBands", false))));
                    MIX.with(|m| *m.borrow_mut() = Some(MixPhase::Waiting { key: next.key.clone(), plan, tempo, version, out, pitch }));
                }
                // (it couldn't be got ready: the usual way, then)
                Err(e) => { omnipulse_core::diag::note(&format!("Seamless Mix: couldn't get the next one ready: {e}")); MIX.with(|m| *m.borrow_mut() = None) }
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
            // (each bar's loudness, dB, rounded: where its intro, drops and outro are)
            "barDb": m.rhythm.bar_db.iter().map(|x| (*x as f64 * 10.0).round() / 10.0).collect::<Vec<_>>(),
        })),
    })
}

fn mix_record(a: &App, plan: &mix::Plan, from: Option<&Track>, to: Option<&Track>) -> serde_json::Value {
    let p = &a.web.prefs;
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0);
    serde_json::json!({
        "id": now,
        "kind": mix_kind(plan),
        "plan": plan,
        "from": from.map(mix_side),
        "to": to.map(mix_side),
        "settings": {
            "mixWords": set_row_on(p, "mixWords", true), "mixDrop": set_row_on(p, "mixDrop", true),
            "mixMelody": set_row_on(p, "mixMelody", true), "mixBands": set_row_on(p, "mixBands", false), "mixTape": set_row_on(p, "mixTape", true),
        },
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
                    "kind": "unmixed",
                    // ("ended": played to its end and on to the next unmixed; "picked": a skip or a click, by hand)
                    "how": how,
                    "whyNotMixed": c.why,
                    "mixOn": integrations::on(p, "mixOn"),
                    "plan": null,
                    "from": c.from.as_ref().map(mix_side),
                    "to": mix_side(&c.to),
                    "settings": { "mixWords": set_row_on(p, "mixWords", true), "mixDrop": set_row_on(p, "mixDrop", true), "mixMelody": set_row_on(p, "mixMelody", true), "mixBands": set_row_on(p, "mixBands", false), "mixTape": set_row_on(p, "mixTape", true) },
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
    let n = std::fs::read_to_string(dir.join("mix-feedback.jsonl")).map(|s| s.lines().count()).unwrap_or(0);
    let note = format!("Transition rated {}: {what} ({}). {n} ratings so far.", if good { "good" } else { "bad" }, rec["kind"].as_str().unwrap_or(""));
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.clone().into()); }
    Ok(note)
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
    let Some(n) = at else { omnipulse_core::diag::note("Seamless Mix: its moment came, but the next one wasn't next any more"); return };
    // (each beat of the one coming in, at its own tempo, for easing back over sixteen of them)
    let ease_ms = omnipulse_core::rhythm::known(&key).map(|m| (16.0 * m.rhythm.period_ms) as u64).unwrap_or(8000);
    audit_began(shared, &plan, format!("{} into {}", shared.borrow().queue.current().map(|t| t.title.clone()).unwrap_or_default(), shared.borrow().queue.items.get(n).map(|q| q.t.title.clone()).unwrap_or_default()));
    if plan.style.is_some() { MIX_STYLE_LAST.with(|l| l.set(plan.style)); }
    // (what it was, for rating it from the Stream Deck)
    let rec = {
        let a = shared.borrow();
        mix_record(&a, &plan, a.queue.current(), a.queue.items.get(n).map(|q| &q.t))
    };
    LAST_MIX.with(|l| *l.borrow_mut() = Some((rec, std::time::Instant::now())));
    if let Some(ui) = shared.borrow().ui.upgrade() {
        match (plan.style, plan.word.clone()) {
            (_, Some(w)) => ui.set_status(format!("Wordplay: \u{201c}{w}\u{201d}").into()),
            (Some("drop swap"), _) => ui.set_status("Drop swap.".into()),
            (Some("melody play"), _) => ui.set_status("Melody play.".into()),
            _ => {}
        }
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
    if integrations::on(&shared.borrow().web.prefs, "mixOn") { rhythm_ahead(shared, next.clone()); loud_ahead(shared, next.clone()); }
    let yt_id = |t: &Track| if let track::Src::YouTube(id) = &t.src { Some(id.clone()) } else { None };
    // (a Spotify song from YouTube Music: with a free account, or Seamless Mix on)
    let sp_yt = matches!(next.src, track::Src::Spotify(_)) && (!sp_plays_here() || integrations::on(&shared.borrow().web.prefs, "mixOn"));
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
    }
    { let mut a = shared.borrow_mut(); apply_filter(&mut a); paint_queue(&mut a); }
    // a requested song: the widget cheers whoever asked for it, for a few seconds
    let by = if playing { shared.borrow().queue.qi.and_then(|i| shared.borrow().queue.items.get(i).map(|q| q.by.clone())).unwrap_or_default() } else { String::new() };
    if on && !by.is_empty() {
        integrations::like_banter(by.trim_end_matches(" (Jam)"));
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
            // (stopped: the accent you picked again, not the last cover's)
            apply_look(shared);
            if let Some(ui) = shared.borrow().ui.upgrade() {
                ui.set_playing(false);
            }
            push_out(shared, true);
        }
        // back from the first: it starts again
        None if d < 0 => engine.seek(0),
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
            move |t: &Track| (sets || !t.is_mix()) && !(avoid && lp.get(&t.key).is_some_and(|&ts| now.saturating_sub(ts) < within)) && streamsafe::allowed(prefs, t) && hide::rec_ok(prefs, t) && !hide::no_version(prefs, t)
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
    // a mix playing: the title bar's button and the progress bar show it
    let mixing = shared.borrow().engine.mixing();
    if ui.get_mixing() != mixing { ui.set_mixing(mixing); }
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
    let near_end = {
        let mut a = shared.borrow_mut();
        if let Some(i) = a.lyrics.iter().rposition(|l| l.ms > 0 && l.ms <= pos + 200).filter(|_| mix_show_pos().is_none()) {
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
                        ui.set_status(format!("\"{}\" plays that version from now on.", t.title).into());
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
        "mix-good" | "mix-bad" => if let Err(e) = mix_feedback(shared, action == "mix-good") { note = Some(e) },
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
const SET_ROWS: [(&str, &str, &str, &str, bool); 28] = [
    ("Library", "Sets & Podcasts in the Queue", "Lists and autoplay include anything over 15 minutes. Off: only when you pick one.", "queueSets", false),
    ("Library", "Autoplay", "Plays similar tracks when the queue runs out.", "autoplay", true),
    ("Library", "Hidden & Not Recommended", "Right-click a track to stop it, its album or its artist being recommended. Off: nothing is left out; the lists are kept.", "hideOn", true),
    ("Library", "Avoid Recent Plays", "Skips songs you've heard lately in what the app picks for you. What you choose yourself still plays.", "avoidRecent", true),
    ("Sound", "Seamless Mix", "Songs with a beat blend into the next on it, as a DJ mixes: at its tempo, on its phrase, the next one sweeping in from the top as the one playing gives way below it, down to the drop. While it's on, Spotify songs play from their YouTube Music version (their Spotify details kept), so they mix too. What it uses on your PC: your internet connection and CPU to download and measure each next song ahead of time, and a little more CPU and memory while a mix plays or a song plays at a changed tempo.", "mixOn", true),
    ("Library", "Skip non-music parts", "What plays from YouTube skips the parts of music videos that aren't music (a skit before the song, an interview after it), as marked on SponsorBlock.", "sbOn", true),
    ("Library", "Favourites Page", "Your starred tracks on a page of their own, in the sidebar. The stars stay either way.", "favPage", true),
    ("Library", "Sets & Podcasts Page", "A page for anything over 15 minutes, in the sidebar. Right-click it there to hide it.", "setsOn", true),
    ("Library", "Stats", "Your listening, in the sidebar. Right-click it there to hide it.", "statsShown", true),
    ("Integrations", "SoundCloud", "Its likes, playlists and search.", "scOn", true),
    ("Integrations", "Spotify", "Its likes, playlists and search.", "spOn", true),
    ("Integrations", "YouTube Music", "Search, radio, and Go+ tracks in full.", "ytOn", true),
    ("Integrations", "Deezer and Apple Music catalogues", "Open music catalogues, no account needed: search, Discover and artist pages find their songs, and they play from YouTube Music.", "catOn", true),
    ("Integrations", "MusicBrainz", "An open music catalogue: search finds its songs too, and they play from YouTube Music.", "mbOn", true),
    ("Integrations", "Listen Along & Jams", "Listen together with friends, from your Discord status or a Jam invite. Off: no Jam button, and invites and requests are turned away (song requests aren't affected).", "jamsOn", true),
    ("Integrations", "Last.fm Scrobbling", "What you play goes to your Last.fm profile.", "scrobble", true),
    ("Integrations", "Like Other Tracks There Too", "With your YouTube account connected: liking a SoundCloud or Spotify track likes the same song on YouTube.", "ytMirror", false),
    ("Integrations", "Follow Everywhere", "Following an artist on one platform follows them on the others.", "followEverywhere", true),
    ("Integrations", "Chromecast", "Speakers and TVs on your network, to play to: they show with Windows' devices, where the sound plays.", "castOn", false),
    ("Streaming", "Streamer Mode", "Hides account names, keys, addresses and plugin settings. The title bar's button does this too.", "streamerMode", false),
    ("Streaming", "Extended", "Also leaves out music that isn't safe on stream: only open licences, and artists and labels cleared for streaming.", "streamSafe", false),
    ("Integrations", "Discord Status", "Shows what you're playing on your Discord profile.", "presence", false),
    ("Streaming", "Stream Widget", "A now-playing card for OBS, with alerts.", "widget", false),
    ("Streaming", "Alerts", "On the widget: follows, subs, bits, raids and more, from your bot.", "alertsOn", false),
    ("Streaming", "Song Requests", "Chat asks for songs through your bot.", "req", false),
    ("Streaming", "Limit Requests", "The most waiting, and the most per viewer. Off: chat can request as many songs as it likes.", "reqLimits", true),
    ("Streaming", "OmniPulse Offset", "Your music a second time, as its own app (\"OmniPulse Offset\"), to send to a stream mix only, with an offset and a volume of its own.", "out2On", false),
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
            .filter(|(_, _, _, key, _)| !matches!(*key, "scOn" | "spOn" | "ytOn" | "scrobble"))
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
        // the plugins installed (the web app adds and removes them; each one switches on and off here)
        if sec == "Plugins" && !streamer {
            for p in omnipulse_core::plugins::list() {
                let m = &p.manifest;
                let state = if !plugin_on(&a.web.prefs, &m.id) { "Off.".to_string() } else if plugrun::get(&m.id).is_some() { "Running.".to_string() } else { PLUG_ERR.with(|e| e.borrow().get(&m.id).cloned()).map(|e| format!("Couldn't start: {e}")).unwrap_or_else(|| "Starting...".into()) };
                let note = format!("{}{}{}", if m.description.is_empty() { String::new() } else { format!("{} ", m.description) }, if p.bundled { "Comes with OmniPulse. " } else { "" }, state);
                let more = m.settings.as_array().is_some_and(|l| !l.is_empty());
                list.push(SetRow { head: Default::default(), label: m.name.clone().into(), note: note.into(), on: plugin_on(&a.web.prefs, &m.id), key: format!("plugOn_{}", m.id).into(), sub: false, more, button: Default::default() });
            }
            // (the plugins: adding one, and taking out the ones you've added; it stays at the top)
            list.insert(0, SetRow { head: Default::default(), label: "Add a Plugin".into(), note: "Music sources made by others. Each is walled off and only reaches the sites it lists.".into(), on: false, key: "plugAdd".into(), sub: false, more: false, button: "Add".into() });
        }
        ModelRc::from(Rc::new(VecModel::from(list)))
    };
    ui.set_set_rows_integrations(rows("Integrations"));
    ui.set_set_rows_library(rows("Library"));
    ui.set_set_rows_sound(rows("Sound"));
    ui.set_set_rows_plugins(rows("Plugins"));
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
    PF_CRUMBS.with(|c| *c.borrow_mut() = path.into_iter().map(|(_, t)| t).collect());
}

/// A part of the panel's path clicked: back to it
fn crumb_go(shared: &Shared, i: usize) {
    let Some(to) = PF_CRUMBS.with(|c| c.borrow().get(i).cloned()) else { return };
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    if let Some(n) = to.strip_prefix("panel:") { panel_open(shared, n); return; }
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
const SET_HAND: [(&str, &str, &str); 22] = [
    ("", "Updates", "Checks for a newer version a little after it starts, and installs it."),
    ("App & Look", "Theme", "Light to dark."),
    ("App & Look", "Tint", "A colour through the theme."),
    ("App & Look", "Tint Strength", "How much of the tint."),
    ("App & Look", "Accent", "The colour of buttons, the progress bar and what's playing."),
    ("App & Look", "Colours from Album Art", "Both colours come from the cover that's playing."),
    ("App & Look", "Player Style", "Tab, Classic Bar, Floating or Minimal."),
    ("App & Look", "Track Info Tab", "Which end of the player bar the tab sits at."),
    ("App & Look", "List Density", "How tightly tracks are listed. Compact fits more on screen."),
    ("App & Look", "Display Name", "Shown in the title bar and taskbar."),
    ("App & Look", "Minimize to Tray", "Minimizing hides the window to the tray; music keeps playing. Open it again, or quit, from the tray's menu."),
    ("App & Look", "Taskbar Widget", "What's playing, always on show in the taskbar by the clock, with its controls."),
    ("App & Look", "Hardware Acceleration", "Draws the app with your graphics card. Takes effect the next time OmniPulse starts."),
    ("App & Look", "Animations", "Fades and motion."),
    ("App & Look", "Rounded Corners", "On the sidebar, the page and the queue. Off: square."),
    ("App & Look", "Back Up & Restore", "Your settings, keys, favourites, playlists, stats and alerts, in one .zip."),
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
    for (cat, label, note, key, _) in SET_ROWS {
        let cat = if matches!(key, "scOn" | "spOn" | "ytOn" | "scrobble") { "Integrations" } else { cat };
        // (Alerts: found in the Stream Widget's page)
        if key == "alertsOn" {
            out.push(SetFound { path: format!("{} \u{203a} {}", path(cat), panels::title("widget")), label: label.into(), note: note.into(), cat: cat.into(), panel: "widget".into(), key: key.into(), toggle: true });
            continue;
        }
        out.push(SetFound { path: path(cat), label: label.into(), note: note.into(), cat: cat.into(), panel: String::new(), key: key.into(), toggle: true });
    }
    let ip = omnipulse_core::lan_ip().unwrap_or_else(|| "THIS-PC".into());
    for (name, cat) in [("requests", "Streaming"), ("replies", "Streaming"), ("limits", "Streaming"), ("mix", "Sound"), ("discord", "Integrations"), ("together", "Integrations"), ("lyrics", "Integrations"), ("widget", "Streaming"), ("alerts", "Streaming"), ("alertlist", "Streaming"), ("remote", "Streaming"), ("offset", "Streaming"), ("safe", "Streaming"), ("recent", "Library"), ("hidden", "Library"), ("plugins", "Plugins")] {
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
}

fn panel_open(shared: &Shared, name: &str) {
    panel_keys(shared);
    if name == "plugins" { panels::PLUG_PENDING.with(|p| p.borrow_mut().take()); }
    let vals = panels::values(name, &shared.borrow().web.prefs);
    PF_EDIT.with(|e| *e.borrow_mut() = Some((format!("panel:{name}"), vals)));
    pf_paint(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        let head = match name.strip_prefix("alert:") { Some(k) => alerts::name(&shared.borrow().web.prefs, k), None => panels::title(name) };
        let cat = match name {
            "requests" | "replies" | "limits" | "widget" | "remote" | "offset" | "alerts" | "alertlist" | "safe" => "Streaming",
            n if n.starts_with("alert:") => "Streaming",
            "plugins" => "Plugins",
            "mix" => "Sound",
            "discord" | "together" | "lyrics" => "Integrations",
            _ => "Library",
        };
        let mut path = vec![("Settings".to_string(), "root".to_string()), (cat.to_string(), format!("cat:{cat}"))];
        match name {
            "alerts" => path.push((panels::title("widget"), "panel:widget".into())),
            "alertlist" => path.push((panels::title("alerts"), "panel:alerts".into())),
            "replies" => path.push((panels::title("requests"), "panel:requests".into())),
            n if n.starts_with("alert:") => {
                path.push((panels::title("alerts"), "panel:alerts".into()));
                path.push((panels::title("alertlist"), "panel:alertlist".into()));
            }
            _ => {}
        }
        crumbs_set(&ui, path);
        ui.set_pf_head(head.into());
        ui.set_pf_open(true);
    }
}

fn panel_paint(shared: &Shared, name: &str, vals: &serde_json::Map<String, serde_json::Value>) {
    let fields = panels::fields(name, vals, &shared.borrow().web.prefs, &omnipulse_core::lan_ip().unwrap_or_else(|| "THIS-PC".into()));
    let text = |v: &serde_json::Value| match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Null => String::new(), x => x.to_string() };
    let rows: Vec<PlugField> = fields.iter().map(|f| {
        let key = f["key"].as_str().unwrap_or("").to_string();
        let kind = f["type"].as_str().unwrap_or("text").to_string();
        let now = vals.get(&key).cloned().unwrap_or_default();
        let labels = || -> Vec<slint::SharedString> { f["labels"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(slint::SharedString::from)).collect() };
        let options: Vec<slint::SharedString> = match kind.as_str() {
            "row" | "select" | "stepper" => labels(),
            "range" | "rangenum" => vec![f["min"].to_string().into(), f["max"].to_string().into(), f["step"].to_string().into(), f["unit"].as_str().unwrap_or("").into()],
            "togglecfg" => vec!["Configure".into()],
            _ => Vec::new(),
        };
        // (where it is among its options; a colour: its value as 0xRRGGBB, -1 if it isn't one)
        let at: i32 = match kind.as_str() {
            "select" | "stepper" => f["options"].as_array().and_then(|o| o.iter().position(|o| panels::same(o, &now))).unwrap_or(0) as i32,
            "color" => now.as_str().and_then(|c| c.strip_prefix('#')).filter(|c| c.len() == 6).and_then(|c| i32::from_str_radix(c, 16).ok()).unwrap_or(-1),
            _ => 0,
        };
        let value = match kind.as_str() {
            "row" | "togglecfg" => if now.as_bool() == Some(true) { "true".into() } else { "false".into() },
            "select" | "stepper" => f["labels"][at.max(0) as usize].as_str().unwrap_or("").to_string(),
            "range" | "rangenum" => now.as_f64().unwrap_or(f["min"].as_f64().unwrap_or(0.0)).to_string(),
            "toggle" => if now.as_bool() == Some(true) { "true".into() } else { "false".into() },
            "info" | "button" => f["value"].as_str().unwrap_or("").to_string(),
            _ => text(&now),
        };
        PlugField { label: f["label"].as_str().unwrap_or("").into(), note: f["note"].as_str().unwrap_or("").into(), value: value.into(), kind: kind.into(), options: ModelRc::from(Rc::new(VecModel::from(options))), key: key.into(), at }
    }).collect();
    PANEL_FIELDS.with(|p| *p.borrow_mut() = fields);
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_fields(ModelRc::from(Rc::new(VecModel::from(rows)))); }
}

/// A field changed (a choice: on to its next; a button: pressed, at once)
fn panel_set(shared: &Shared, name: &str, mut vals: serde_json::Map<String, serde_json::Value>, i: usize, v: &str) {
    let Some(f) = PANEL_FIELDS.with(|p| p.borrow().get(i).cloned()) else { return };
    let key = f["key"].as_str().unwrap_or("").to_string();
    let redraw = match f["type"].as_str().unwrap_or("") {
        "toggle" => { vals.insert(key, serde_json::json!(v == "true")); true }
        "select" | "stepper" => {
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
                match desktop::pick_color(&now) { Some(c) => { vals.insert(key, serde_json::json!(c)); } None => return }
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
fn panel_press(shared: &Shared, key: &str) {
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
            "A new key: the addresses have it.".to_string()
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
            if let Some((_, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) {
                vals.insert("reqMax".into(), serde_json::json!(10));
                vals.insert("reqPerUser".into(), serde_json::json!(2));
                PF_EDIT.with(|e| *e.borrow_mut() = Some(("panel:limits".into(), vals)));
            }
            "Back to 10 waiting, 2 per viewer (Save keeps it).".to_string()
        }
        "alertsSwitch" => {
            set_toggle(shared, "alertsOn");
            return;
        }
        // (Chat Replies: from Song Requests, its page saved first)
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
            let (title, exts): (&str, &[&str]) = if image { ("Choose a picture or GIF for this alert", &["gif", "png", "webp", "jpg", "jpeg"]) } else { ("Choose a sound for this alert", &["mp3", "wav", "ogg", "m4a", "aac", "flac"]) };
            let Some(path) = rfd::FileDialog::new().set_title(title).add_filter(if image { "Pictures" } else { "Sounds" }, exts).pick_file() else { return };
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
                let Some(path) = rfd::FileDialog::new().set_title("Choose a picture or GIF for this alert's collage").add_filter("Pictures", &["gif", "png", "webp", "jpg", "jpeg"]).pick_file() else { return };
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
            if image { "Its icon shows again.".to_string() } else { "Its sound is gone.".to_string() }
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
            let Some(path) = rfd::FileDialog::new().set_title("A plugin's manifest").add_filter("Plugin manifest", &["json"]).pick_file() else { return };
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
            let sure = std::env::var_os("OMNIPULSE_SELFTEST").is_some()
                || rfd::MessageDialog::new().set_title("Remove plugin").set_description(format!("Remove {name}? Its songs leave your search and Library.")).set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
            if !sure { return; }
            plugrun::stop(id);
            let r = omnipulse_core::plugins::remove(id);
            paint_settings(&shared.borrow());
            match r { Ok(()) => format!("Removed {name}."), Err(e) => format!("Couldn't remove it: {e}") }
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
fn panel_save(shared: &Shared, name: &str, vals: serde_json::Map<String, serde_json::Value>) {
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
    panel_keys(shared);
    match name {
        "requests" => { omnipulse_core::requests::requests_stop(); outside_start(shared); }
        "widget" | "alerts" => { omnipulse_core::widget::widget_stop(); outside_start(shared); push_out(shared, true); }
        "remote" => { omnipulse_core::remote::remote_stop(); outside_start(shared); }
        "offset" => offset_apply(shared),
        _ => {}
    }
    PF_EDIT.with(|e| e.borrow_mut().take());
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_open(false); ui.set_status(format!("{} settings saved.", panels::title(name)).into()); }
    paint_settings(&shared.borrow());
}

/// A plugin's settings dialog: its manifest's fields, as they're set (prefs.pluginSettings, as the web app's)
fn pf_open(shared: &Shared, id: &str) {
    let Some(p) = omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id) else { return };
    let now = plugrun::settings_of(&p.manifest, &shared.borrow().web.prefs);
    let vals = now.as_object().cloned().unwrap_or_default();
    PF_EDIT.with(|e| *e.borrow_mut() = Some((id.to_string(), vals)));
    pf_paint(shared);
    if let Some(ui) = shared.borrow().ui.upgrade() {
        crumbs_set(&ui, vec![("Settings".into(), "root".into()), ("Plugins".into(), "cat:Plugins".into())]);
        ui.set_pf_head(p.manifest.name.clone().into());
        ui.set_pf_open(true);
    }
}

fn pf_fields(id: &str) -> Vec<serde_json::Value> {
    omnipulse_core::plugins::list().into_iter().find(|p| p.manifest.id == id).and_then(|p| p.manifest.settings.as_array().cloned()).unwrap_or_default()
}

fn pf_paint(shared: &Shared) {
    let Some((id, vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    if let Some(name) = id.strip_prefix("panel:") { return panel_paint(shared, name, &vals); }
    let text = |v: &serde_json::Value| match v { serde_json::Value::String(s) => s.clone(), serde_json::Value::Null => String::new(), x => x.to_string() };
    let rows: Vec<PlugField> = pf_fields(&id).iter().filter_map(|f| {
        let key = f["key"].as_str()?.to_string();
        let kind = f["type"].as_str().unwrap_or("text").to_string();
        let options: Vec<slint::SharedString> = f["options"].as_array().into_iter().flatten().map(|o| text(o).into()).collect();
        let at = f["options"].as_array().and_then(|o| o.iter().position(|o| Some(o) == vals.get(&key))).unwrap_or(0) as i32;
        Some(PlugField { label: f["label"].as_str().unwrap_or(&key).into(), note: f["note"].as_str().unwrap_or("").into(), value: vals.get(&key).map(text).unwrap_or_default().into(), kind: kind.into(), options: ModelRc::from(Rc::new(VecModel::from(options))), key: key.into(), at })
    }).collect();
    if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_pf_fields(ModelRc::from(Rc::new(VecModel::from(rows)))); }
}

/// One setting changed in the dialog (a select's "next": on to its next option)
fn pf_set(shared: &Shared, i: usize, v: &str) {
    let Some((id, mut vals)) = PF_EDIT.with(|e| e.borrow().clone()) else { return };
    if let Some(name) = id.strip_prefix("panel:") { return panel_set(shared, name, vals, i, v); }
    let Some(f) = pf_fields(&id).get(i).cloned() else { return };
    let Some(key) = f["key"].as_str().map(String::from) else { return };
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

// ---------- the sidebar's pages: your order, and the ones you've hidden ----------
const NAV_IDS: [&str; 7] = ["navDiscover", "navRediscover", "navLikes", "navFavourites", "navSets", "navArtists", "navStats"];

fn nav_ids(prefs: &serde_json::Value) -> Vec<String> {
    let order: Vec<String> = prefs["navOrder"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    let rank = |id: &str| order.iter().position(|o| o == id).unwrap_or(order.len() + NAV_IDS.iter().position(|n| *n == id).unwrap_or(0));
    let mut ids: Vec<String> = NAV_IDS.iter().map(|s| s.to_string()).collect();
    ids.sort_by_key(|id| rank(id));
    let off = |key: &str| prefs[key].as_bool() == Some(false);
    ids.retain(|id| !(id == "navFavourites" && off("favPage")) && !(id == "navSets" && off("setsOn")) && !(id == "navStats" && off("statsShown")));
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
            // (Seamless Mix: Start a Jam greyed out with it on, and back with it off)
            if key == "mixOn" { jam_paint(shared); }
            // the servers stop, and start again with what's set now
            match key {
                "widget" | "alertsOn" => omnipulse_core::widget::widget_stop(),
                "req" => omnipulse_core::requests::requests_stop(),
                "remoteOn" => omnipulse_core::remote::remote_stop(),
                "presence" if !on => integrations::presence(&shared.borrow().rt, &shared.borrow().web.prefs, None, 0, false, None),
                _ => {}
            }
            if matches!(key, "widget" | "req" | "remoteOn" | "alertsOn") && (on || key == "alertsOn") { panel_keys(shared); outside_start(shared); }
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
            if key == "mixOn" { if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_mix_on(on); } }
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

const SP_FREE: &str = "That's a free Spotify account. Spotify only lets Premium accounts sign in straight away; a free one signs in with the Spotify app of someone with Premium who adds you to it: the steps are open.";

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
        let status = if busy {
            "Finish signing in in your browser.".to_string()
        } else if signed {
            match (key, who) {
                ("spotify", Some(w)) if omnipulse_core::spotify_engine::spe_signed_in() && omnipulse_core::spotify_engine::spe_premium() == Some(false) => format!("Signed in as {w} · Free: your likes and playlists are here; Spotify songs play from YouTube Music."),
                ("spotify", Some(w)) if omnipulse_core::spotify_engine::spe_signed_in() => format!("Signed in as {w} · Premium: Spotify plays in full here."),
                ("spotify", Some(w)) => format!("Signed in as {w} · Free: Spotify tracks play from YouTube Music."),
                (_, Some(w)) => format!("Signed in as {w}."),
                ("spotify", None) if private && omnipulse_core::spotify_engine::spe_premium() == Some(false) => "Signed in (your name's hidden in Streamer Mode) · Free: Spotify songs play from YouTube Music.".to_string(),
                ("spotify", None) if private => "Signed in (your name's hidden in Streamer Mode) · Premium: Spotify plays in full here.".to_string(),
                _ if private => "Signed in (your name's hidden in Streamer Mode).".to_string(),
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
        Account { name: name.into(), status: status.into(), signed, busy, keys: keys.into(), switch: sw.into(), on, manage: if signed { manage.into() } else { "".into() } }
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

const SP_OWN_KEYS: &str = "Optional. Spotify limits requests per app, and its own sign-in (the one you used) shares one allowance with everyone signed in that way, so when it's busy, OmniPulse has to wait. Keys of your own give you an allowance of your own: your requests go through them, with Spotify's sign-in kept as a backup. Spotify needs an app's owner to have Premium; with a free account, use the Client ID of someone with Premium who's added you.";

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
            if KS_SIGNED_IN.with(|k| k.replace(false)) {
                // signed in already: with your own app too, beside it
                let (rt, weak) = { let a = shared.borrow(); (a.rt.clone(), a.ui.clone()) };
                rt.spawn(async move {
                    let r = omnipulse_core::spotify::spotify_login().await;
                    let _ = weak.upgrade_in_event_loop(move |ui| SHARED.with(|s| {
                        ui.set_status(match &r { Ok(()) => "Signed in with your own Spotify keys: your requests go through them now.".to_string(), Err(e) => format!("Couldn't sign in with your keys: {e}") }.into());
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
}

/// Each tick while playing: long silences jumped over, the fade out at the end. True if it moved on.
fn fade_and_silences(shared: &Shared, pos: u64, dur: u64) -> bool {
    let (an, key, fade, skip, engine) = {
        let a = shared.borrow();
        let Some(t) = a.queue.current() else { return false };
        let an = a.cur_loud.as_ref().and_then(|k| a.analysis.get(k)).cloned();
        (an, t.key.clone(), fade_out_ms(&a), sound::skip_silence_ms(&a.web.prefs), a.engine.clone())
    };
    let Some(an) = an else {
        // (not measured, as Spotify's songs aren't: its level watched as it plays, and on to the next
        // after that long in silence, as the web app's)
        if skip == 0 || listening().is_some() || MIX.with(|m| m.borrow().is_some()) { QUIET_SINCE.with(|q| q.set(None)); return false; }
        let quiet = engine.level().0 < -60.0 && pos > 3000;
        if !quiet { QUIET_SINCE.with(|q| q.set(None)); return false; }
        let since = QUIET_SINCE.with(|q| { if q.get().is_none() { q.set(Some(std::time::Instant::now())); } q.get().unwrap() });
        if since.elapsed().as_millis() as u64 >= skip {
            QUIET_SINCE.with(|q| q.set(None));
            if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(format!("Moved on after {} s of silence.", skip / 1000).into()); }
            step(shared, 1);
            return true;
        }
        return false;
    };
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
            let tracks: Vec<Track> = share["tracks"].as_array().into_iter().flatten().map(collab::with_who(&share)).filter_map(|s| Track::from_snapshot(&s)).filter(|t| t.plat() != "local").collect();
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
            let item = a.queue.items.remove(at);
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
                if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(if kind == "jam" { format!("Sent {} to the Jam.", plural(n, "track", "tracks")) } else { "You're listening along: only a Jam takes songs from listeners.".to_string() }.into()); }
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
    let sure = std::env::var_os("OMNIPULSE_SELFTEST").is_some()
        || rfd::MessageDialog::new().set_title("Delete playlist").set_description(format!("Delete \"{}\" from this PC?", p.title)).set_buttons(rfd::MessageButtons::OkCancel).show() == rfd::MessageDialogResult::Ok;
    if !sure {
        return;
    }
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

/// Still hosting: its time kept fresh (as states are pushed: every 10 s at most)
fn host_fresh() {
    let now = omnipulse_core::now_secs();
    if now.saturating_sub(HOST_FRESH_AT.with(|c| c.get())) < 10 { return; }
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

const JAM_OFF: &str = "Jams are off while Seamless Mix is on (Settings > Sound): a Jam plays the host's queue as it is, unmixed.";

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
    // (Seamless Mix on: no Jam to start; one already going carries on)
    ui.set_jam_off(!jam && integrations::on(&shared.borrow().web.prefs, "mixOn"));
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
    // (asked to start one, with Seamless Mix on: declined, saying why)
    if ASK.with(|a| a.borrow().as_ref().is_some_and(|x| !x.3)) && integrations::on(&shared.borrow().web.prefs, "mixOn") {
        if let Some((id, _, _, join)) = ASK.with(|a| a.borrow_mut().take()) { ask_answer(&id, join, false, false); }
        jam_paint(shared);
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(JAM_OFF.into()); }
        return;
    }
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
    // (skipped before it counted as a play: noted against its artist, for Discover)
    if d > 0 && listening().is_none() {
        let a = shared.borrow();
        if let Some(q) = a.queue.qi.and_then(|i| a.queue.items.get(i)) { discover::note_skip(&q.t, a.engine.position_ms(), a.engine.duration_ms()); }
    }
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
    // (Seamless Mix on: no joining a Jam; Listen Along's still fine, as mixing waits while listening)
    if kind == "jam" && integrations::on(&shared.borrow().web.prefs, "mixOn") {
        if let Some(ui) = weak.upgrade() { ui.set_status(JAM_OFF.into()); }
        return;
    }
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

/// YouTube Music's helpers (yt-dlp and Deno, about 58 MB): how they are, in Settings > Integrations
fn helpers_paint(shared: &Shared) {
    let Some(ui) = shared.borrow().ui.upgrade() else { return };
    let on = plat_on(&shared.borrow().web.prefs, "youtube");
    let pr = omnipulse_core::tools::progress();
    let mb = |b: u64| format!("{:.0}", b as f64 / 1_048_576.0);
    let used = omnipulse_core::tools::disk_use();
    let have = omnipulse_core::tools::ytdlp_path().is_some();
    let (note, setup) = if pr["busy"].as_bool() == Some(true) {
        let (done, total) = (pr["done"].as_u64().unwrap_or(0), pr["total"].as_u64().unwrap_or(0));
        (format!("Setting up: {}{}...", pr["step"].as_str().unwrap_or("").to_lowercase(), if total > 0 { format!(", {} of {} MB", mb(done), mb(total)) } else { String::new() }), "")
    } else if let Some(e) = pr["error"].as_str() {
        (format!("Couldn't set up: {e}"), "Try Again")
    } else if have {
        ("Ready: yt-dlp finds each song's audio and Deno runs YouTube's player code for it, from their official releases (checked against their checksums), kept up to date.".to_string(), "")
    } else if !on {
        ("YouTube Music is off.".to_string(), "")
    } else if omnipulse_core::yt_stream::on_path_public("yt-dlp") {
        ("Ready, with your own yt-dlp.".to_string(), "")
    } else {
        ("Two small tools YouTube Music plays with (yt-dlp and Deno, about 58 MB): set up by themselves.".to_string(), "Set Up")
    };
    ui.set_helpers_note(format!("{note}{}", if used > 0 && !pr["busy"].as_bool().unwrap_or(false) { format!(" They take {} MB.", mb(used)) } else { String::new() }).into());
    ui.set_helpers_setup(setup.into());
    ui.set_helpers_remove(used > 0 && pr["busy"].as_bool() != Some(true));
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
        let note = match omnipulse_core::tools::uninstall() { Ok(b) => format!("yt-dlp and Deno are removed ({:.0} MB freed).", b as f64 / 1_048_576.0), Err(e) => format!("Couldn't remove them: {e}") };
        if let Some(ui) = shared.borrow().ui.upgrade() { ui.set_status(note.into()); }
    }
    helpers_paint(shared);
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
    ui.set_apple_note(format!("Once: your loved songs become likes on your other platforms, and your playlists become playlists on this PC. Nothing on Apple Music is changed, and your sign-in is forgotten after.{on}").into());
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
            ui.set_status("Sign in to Apple Music in the window that opened (only to read your music: nothing there is changed)...".into());
            STATUS_HOLD.with(|h| *h.borrow_mut() = Some("Sign in to Apple Music in the window that opened (only to read your music: nothing there is changed)...".into()));
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
            save_session(&shared.borrow());
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
    // (who you're signed in as: hidden, or shown again)
    accounts_paint(shared);
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
            let _ = omnipulse_core::requests::requests_start(prefs["reqPort"].as_u64().unwrap_or(7879) as u16, key.to_string(), integrations::set(&prefs, "botRemote"));
        }
    }
    push_out(shared, false);
}

/// A media key, the tray's menu or a taskbar button
/// The window closed (its X): OmniPulse quits, always
fn close_or_tray(shared: &Shared) {
    save_session(&shared.borrow());
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
        save_session(&shared.borrow());
        let _ = ui.window().hide();
    } else {
        ui.window().set_minimized(true);
    }
}

/// The devices menu's last line, with Chromecast on: a fresh look for cast devices
const CAST_SEARCH: &str = "Search for Cast devices";

fn desktop_action(shared: &Shared, action: &str) {
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
            save_session(&shared.borrow());
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
        tw.set_show_progress(ui.get_widget_progress()); tw.set_big_art(ui.get_widget_big_art());
        // where: by the clock, over the taskbar (put back over it each time, as it comes up over us when clicked)
        let scale = if *hwnd != 0 { tw.window().scale_factor() } else { ui.window().scale_factor() };
        let spot = desktop::dock::spot((340.0 * scale) as i32, scale).filter(|_| !desktop::dock::busy());
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
    let motion = prefs["animOn"].as_bool() != Some(false);
    t.set_motion(motion);
    ui.set_anim_on(motion);
    ui.set_close_to_tray(min_to_tray(prefs));
    ui.set_taskbar_widget(prefs["taskbarWidget"].as_bool() == Some(true));
    ui.set_widget_progress(prefs["widgetProgress"].as_bool() != Some(false));
    ui.set_widget_big_art(prefs["widgetBigArt"].as_bool() != Some(false));
    ui.set_gpu_on(prefs["nativeGpu"].as_bool() == Some(true));
    LISTEN_NUDGE.store(nudge_of(prefs), std::sync::atomic::Ordering::Relaxed);
    let nudge = nudge_of(prefs).to_string();
    if ui.get_listen_nudge().trim() != nudge && !(ui.get_listen_nudge().trim().is_empty() && nudge == "0") { ui.set_listen_nudge(nudge.into()); }
    let name = prefs["appName"].as_str().unwrap_or("").trim().to_string();
    ui.set_title_prompt(name.is_empty() && plat_on(prefs, "soundcloud") && !omnipulse_core::soundcloud::signed_in());
    if ui.get_app_name().trim() != name { ui.set_app_name(name.into()); }
    t.set_bg(p.bg.slint());
    t.set_panel(p.panel.slint());
    t.set_elevated(p.elevated.slint());
    t.set_line(p.line.slint());
    t.set_text(p.text.slint());
    t.set_muted(p.muted.slint());
    // the accent: the album art's while something plays (if asked for), or the one picked
    let art = integrations::set(prefs, "artAccent") && a.loaded;
    let (c1, c2) = if art {
        let first = look::Rgb::parse(&a.accent.0);
        let c1 = first.map(|c| look::tune_art(c, p.light));
        // (one colour on the cover: a shade of it, as the web app's)
        let c2 = look::Rgb::parse(&a.accent.1).map(|c| look::tune_art(c, p.light)).or(first.map(|c| look::tune_art_shade(c, p.light)));
        (c1, c2)
    } else {
        let mine = look::Rgb::parse(prefs["accent"].as_str().unwrap_or(ACCENTS[0]));
        (mine, prefs["accent2"].as_str().and_then(look::Rgb::parse).or(mine))
    };
    let c1 = c1.unwrap_or(look::Rgb(0x8e, 0x9b, 0xff));
    t.set_accent_own(c1.slint());
    t.set_accent_2_own(c2.unwrap_or(c1).slint());
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
    let picked2 = prefs["accent2"].as_str().unwrap_or("").to_lowercase();
    ui.set_accent2_on(ACCENTS.iter().position(|c| *c == picked2).map_or(-1, |i| i as i32));
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
    // Match Loudness for the Spotify songs Spotify plays: its own levelling, at the same target
    { let (on, target) = sound::spotify_norm(&shared.borrow().web.prefs); omnipulse_core::spotify_engine::spe_set_normalisation(on, target); }
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
        } else if integrations::on(&s.borrow().web.prefs, "mixOn") {
            if let Some(ui) = s.borrow().ui.upgrade() { ui.set_status(JAM_OFF.into()); }
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
    ui.on_follow_click(move || follow_click(&s));
    let s = shared.clone();
    ui.on_follow_more_click(move || follow_more_click(&s));
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
    ui.set_mix_on(shared.borrow().web.prefs["mixOn"].as_bool() != Some(false));
    let s = shared.clone();
    ui.on_mix_toggle(move || set_toggle(&s, "mixOn"));
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
    nav_paint(&shared);
    let s = shared.clone();
    ui.on_density_changed(move |c| set_look_pref(&s, "density", serde_json::json!(if c { "compact" } else { "comfortable" })));
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
        let Some(path) = rfd::FileDialog::new().set_title("A file with your keys in it").pick_file() else { return };
        let text = std::fs::metadata(&path).ok().filter(|m| m.len() < 65536).and_then(|_| std::fs::read_to_string(&path).ok()).unwrap_or_default();
        let found = keysetup::keys_from(&plat, &text);
        if found.is_empty() {
            let name = keysetup::setup(&plat).map(|x| x.name).unwrap_or("it");
            if let Some(ui) = s.borrow().ui.upgrade() { ui.set_ks_note(format!("That file has no keys for {name} in it.").into()); }
            return;
        }
        ks_take(&s, found);
    });
    let s = shared.clone();
    ui.on_set_more(move |key| {
        if let Some(id) = key.strip_prefix("plugOn_") { return pf_open(&s, id); }
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
        let d = ui.global::<Dnd>();
        d.on_payload(|what| slint::DataTransfer::from(what));
        let s = shared.clone();
        d.on_accepts(move |target, data| {
            // (a page of the sidebar, onto another)
            if let Some(over) = target.strip_prefix("nav:") { return data.plain_text().ok().and_then(|t| t.strip_prefix("n:").map(|f| f != over)).unwrap_or(false); }
            let a = s.borrow(); dragged(&a, &data).is_some_and(|w| drop_ok(&a, &target, &w))
        });
        let s = shared.clone();
        d.on_dropped(move |target, data| {
            if let Some(over) = target.strip_prefix("nav:") {
                if let Some(from) = data.plain_text().ok().and_then(|t| t.strip_prefix("n:").map(String::from)) { nav_move(&s, &from, over); }
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
        apply_filter(&mut a);
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
    // Discover's language: the next one, and Discover made again in it
    let s = shared.clone();
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
        if matches!(act.as_str(), "like" | "fav" | "uploader" | "open" | "prefs" | "cleared") || act.starts_with("artist:") {
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
    // resized from an edge or corner (Windows does the dragging, as it does the title bar's)
    ui.on_resize({
        let weak = ui.as_weak();
        move |d| {
            use slint::winit_030::{winit::window::ResizeDirection as R, WinitWindowAccessor};
            let dir = match d { 0 => R::North, 1 => R::South, 2 => R::West, 3 => R::East, 4 => R::NorthWest, 5 => R::NorthEast, 6 => R::SouthWest, _ => R::SouthEast };
            if let Some(ui) = weak.upgrade() { ui.window().with_winit_window(|w| { let _ = w.drag_resize_window(dir); }); }
        }
    });
    { let s = shared.clone(); ui.on_minimize(move || minimize(&s)); }
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
            desktop_action(&s, a);
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
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("replies") { vec![(9, "play"), (10, "openreplies"), (12, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("np") { vec![(9, "play"), (14, "now"), (18, "end")] }
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("versions") { vec![(9, "play"), (10, "artistnamed"), (40, "vopen"), (41, "end")] }
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
            else if std::env::var("OMNIPULSE_SELFTEST_PLAN").ok().as_deref() == Some("setfit") { vec![(9, "play"), (10, "queue"), (11, "size:1150:530"), (12, "nav:settings"), (13, "cat:Sound"), (14, "scroll:4000"), (16, "cat:Integrations"), (17, "scroll:-4000"), (19, "end")] }
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
                "compact" => ui.invoke_density_changed(true),
                w if w.starts_with("cat_") => ui.set_set_cat(w[4..].into()),
                w if w.starts_with("ss_") => { ui.set_search_text(w[3..].into()); ui.invoke_search_edited(w[3..].into()); println!("selftest: settings search \"{}\": {}", &w[3..], ui.get_set_hits().iter().take(6).map(|h| format!("{} ({})", h.label, h.path)).collect::<Vec<_>>().join("; ")); }
                "spplay" => {
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("mixOn".into(), serde_json::json!(false)); }
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
                w if w.starts_with("panel:") => { if let Some(sh) = SHARED.with(|x| x.borrow().clone()) { panel_open(&sh, &w[6..]); } }
                w if w.starts_with("cat:") => { ui.set_set_cat(w[4..].into()); }
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
                "dragasks" => {
                    let d = ui.global::<Dnd>();
                    let ask = |t: &str, what: &str| d.invoke_accepts(t.into(), slint::DataTransfer::from(slint::SharedString::from(what)));
                    let sides = s.borrow().side_map.len();
                    let own: Vec<bool> = (0..sides.min(6)).map(|i| ask(&format!("side:{i}"), "t:0")).collect();
                    println!("selftest: drag asks: track on sides 0-5 {own:?}; playlist 0 on itself {}, on side 1 {}; queue {}, playlist on queue {}", ask("side:0", "p:0"), ask("side:1", "p:0"), ask("queue", "t:0"), ask("queue", "p:0"));
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
                    if let Some(o) = s.borrow_mut().web.prefs.as_object_mut() { o.insert("mixOn".into(), serde_json::json!(true)); if std::env::var_os("OMNIPULSE_SELFTEST_ART").is_some() { o.insert("artAccent".into(), serde_json::json!(true)); } }
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
                "skip" => { skip_press(&s, 1); }
                "rate" => { println!("selftest: rate: {:?}", mix_feedback(&s, false)); }
                "mixcheck" => {
                    let a = s.borrow();
                    let phase = MIX.with(|m| match &*m.borrow() { None => "none".to_string(), Some(MixPhase::Opening(_)) => "opening".into(), Some(MixPhase::Waiting { .. }) => "waiting".into(), Some(MixPhase::Mixing { tempo, .. }) => format!("mixing at {:.4}x", tempo.get()) });
                    let ear = MIX_EAR.with(|e| e.borrow().as_ref().map(|x| format!("listening again from {} ms", x.1))).or_else(|| MIX_LOCAL.with(|l| l.borrow().as_ref().map(|x| format!("measured again: {} BPM, downbeat {}, from {} ms", x.1.rhythm.bpm, x.1.rhythm.downbeat, x.1.from_ms)))).unwrap_or_default();
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
    slint::run_event_loop_until_quit()?;
    let _ = ui.hide();
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
