// OmniPulse, native (Phase 2 of the rewrite): one window, one process. The Library (your SoundCloud
// and Spotify likes, and your music files) in a list that holds thousands, played through
// omnipulse-audio, with a basic queue. Built beside the web app, which keeps working until this
// one does everything.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use omnipulse_audio::{Decoded, Engine, Event, HlsSource, PushFeed, PushSource, Source};
use omnipulse_core as core;
use serde_json::Value;
use slint::{ComponentHandle, Model, ModelRc, SharedPixelBuffer, Rgba8Pixel, VecModel};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

slint::include_modules!();

#[derive(Clone, Debug)]
enum Src {
    SoundCloud(u64),
    Spotify(String),
    Local(String),
}

#[derive(Clone, Debug)]
struct Track {
    src: Src,
    title: String,
    artist: String,
    album: String,
    ms: u64,
    art: String,
}

impl Track {
    fn plat(&self) -> &'static str {
        match self.src {
            Src::SoundCloud(_) => "soundcloud",
            Src::Spotify(_) => "spotify",
            Src::Local(_) => "local",
        }
    }
    fn row(&self, playing: bool) -> Row {
        Row { title: self.title.clone().into(), artist: self.artist.clone().into(), album: self.album.clone().into(), dur: fmt_ms(self.ms).into(), plat: self.plat().into(), playing }
    }
}

fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

// ---------- the Library ----------
fn sc_track(v: &Value) -> Option<Track> {
    let t = if v.get("track").is_some() { &v["track"] } else { v };
    let id = t["id"].as_u64()?;
    if t["kind"].as_str().is_some_and(|k| k != "track") {
        return None;
    }
    Some(Track {
        src: Src::SoundCloud(id),
        title: t["title"].as_str().unwrap_or("Untitled").to_string(),
        artist: t["user"]["username"].as_str().unwrap_or("").to_string(),
        album: String::new(),
        ms: t["full_duration"].as_u64().or(t["duration"].as_u64()).unwrap_or(0),
        art: t["artwork_url"].as_str().or(t["user"]["avatar_url"].as_str()).unwrap_or("").replace("-large", "-t200x200"),
    })
}

async fn soundcloud_likes() -> Vec<Track> {
    let mut out = Vec::new();
    if !core::soundcloud::signed_in() {
        return out;
    }
    let mut path = "/me/likes/tracks?limit=200&linked_partitioning=true&access=playable".to_string();
    for _ in 0..50 {
        let Ok(v) = core::soundcloud::api_get(path.clone()).await else { break };
        let items = v["collection"].as_array().cloned().or_else(|| v.as_array().cloned()).unwrap_or_default();
        out.extend(items.iter().filter_map(sc_track));
        match v["next_href"].as_str().and_then(|h| h.strip_prefix("https://api.soundcloud.com")) {
            Some(next) if !items.is_empty() => path = next.to_string(),
            _ => break,
        }
    }
    out
}

async fn spotify_likes(max: usize) -> Vec<Track> {
    let mut out = Vec::new();
    if !core::spotify::spotify_signed_in() {
        return out;
    }
    let mut offset = 0;
    while out.len() < max {
        let Ok(v) = core::spotify::spotify_api("GET".into(), format!("/me/tracks?limit=50&offset={offset}&market=from_token"), None).await else { break };
        let items = v["items"].as_array().cloned().unwrap_or_default();
        for it in &items {
            let t = &it["track"];
            let Some(uri) = t["uri"].as_str() else { continue };
            if t["is_playable"].as_bool() == Some(false) {
                continue;
            }
            out.push(Track {
                src: Src::Spotify(uri.to_string()),
                title: t["name"].as_str().unwrap_or("Untitled").to_string(),
                artist: t["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
                album: t["album"]["name"].as_str().unwrap_or("").to_string(),
                ms: t["duration_ms"].as_u64().unwrap_or(0),
                art: t["album"]["images"].as_array().and_then(|i| i.get(1).or(i.first())).and_then(|i| i["url"].as_str()).unwrap_or("").to_string(),
            });
        }
        if items.len() < 50 {
            break;
        }
        offset += 50;
    }
    out
}

fn local_files() -> Vec<Track> {
    core::local::local_load_index();
    core::local::LOCAL.lock().map(|g| g.iter().map(|e| Track {
        src: Src::Local(e.path.clone()),
        title: if e.title.is_empty() { std::path::Path::new(&e.path).file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled").to_string() } else { e.title.clone() },
        artist: e.artist.clone(),
        album: e.album.clone(),
        ms: e.duration,
        art: String::new(),
    }).collect()).unwrap_or_default()
}

// ---------- Spotify: librespot's sound into the engine ----------
static SP_FEED: Mutex<Option<(PushFeed, u32)>> = Mutex::new(None);

struct FeedOut;
impl core::spotify_engine::PcmOut for FeedOut {
    fn send(&self, gen: u32, samples: &[f32]) -> bool {
        SP_FEED.lock().unwrap().as_ref().is_some_and(|(f, _)| f.send(gen, samples))
    }
}

fn spotify_attach() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        core::spotify_engine::spe_attach(Arc::new(FeedOut), Arc::new(|ev: Value| {
            if ev["type"] == "end" {
                if let Some((f, gen)) = SP_FEED.lock().unwrap().as_ref() {
                    f.end(*gen);
                }
            }
        }));
    });
}

// ---------- making a track playable ----------
async fn open_source(t: &Track, rt: tokio::runtime::Handle) -> Result<Box<dyn Source>, String> {
    match &t.src {
        Src::Local(path) => {
            let p = path.clone();
            tokio::task::spawn_blocking(move || Decoded::open_file(std::path::Path::new(&p)).map(|d| Box::new(d) as Box<dyn Source>)).await.map_err(|e| e.to_string())?
        }
        Src::SoundCloud(id) => {
            let v = core::soundcloud::stream_url(id.to_string()).await?;
            let cands = v["candidates"].as_array().cloned().unwrap_or_default();
            let mut why = String::from("SoundCloud sent no stream for it");
            for c in cands {
                let name = c["name"].as_str().unwrap_or("").to_string();
                if name.contains("preview") || name.contains("opus") {
                    continue;
                }
                let mut url = c["url"].as_str().unwrap_or("").to_string();
                if url.starts_with("https://api.soundcloud.com") {
                    url = core::soundcloud::resolve_api_url(&url).await.unwrap_or(url);
                }
                let hls = name.starts_with("hls");
                let r = tokio::task::spawn_blocking(move || -> Result<Box<dyn Source>, String> {
                    if hls { Ok(Box::new(HlsSource::open(&url)?)) } else { Ok(Box::new(omnipulse_audio::open_url(&url, "mp3")?)) }
                }).await.map_err(|e| e.to_string())?;
                match r {
                    Ok(s) => return Ok(s),
                    Err(e) => why = e,
                }
            }
            Err(why)
        }
        Src::Spotify(uri) => {
            spotify_attach();
            let gen = core::spotify_engine::spe_play(uri.clone(), 0).await?;
            let (src, feed) = PushSource::new(
                44100,
                Some(t.ms),
                Box::new(move |ms| {
                    let g = rt.block_on(core::spotify_engine::spe_seek(ms as u32))?;
                    if let Some(x) = SP_FEED.lock().unwrap().as_mut() { x.1 = g; }
                    Ok(g)
                }),
                Box::new(core::spotify_engine::spe_played),
            );
            *SP_FEED.lock().unwrap() = Some((feed, gen));
            Ok(Box::new(src))
        }
    }
}

// The cover, decoded off the UI thread
async fn fetch_art(url: String) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    if url.is_empty() {
        return None;
    }
    let bytes = reqwest::get(&url).await.ok()?.bytes().await.ok()?;
    let img = image::load_from_memory(&bytes).ok()?.thumbnail(128, 128).to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(img.as_raw(), img.width(), img.height()))
}

// ---------- the app ----------
struct State {
    library: Vec<Track>,
    queue: Vec<Track>,
    qi: Option<usize>,
    token: u64,
}

fn main() -> Result<(), slint::PlatformError> {
    let rt = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build().expect("runtime");
    // Slint's software renderer to start with: measured at 38 MB and ~3% of a core while playing, against
    // ~100 MB with the GPU one (most of it the graphics driver). SLINT_BACKEND picks another.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        let _ = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("software".into()).select();
    }
    let ui = AppWindow::new()?;
    let engine = Arc::new(Engine::new("").map_err(slint::PlatformError::Other)?);
    let events = engine.events().expect("events");
    engine.set_volume(0.8);
    let state = Rc::new(std::cell::RefCell::new(State { library: Vec::new(), queue: Vec::new(), qi: None, token: 0 }));
    let rows: Rc<VecModel<Row>> = Rc::new(VecModel::default());
    let qrows: Rc<VecModel<Row>> = Rc::new(VecModel::default());
    ui.set_rows(ModelRc::from(rows.clone()));
    ui.set_queue_rows(ModelRc::from(qrows.clone()));
    ui.set_page_sub("Loading your Library...".into());

    // the Library, loaded in the background
    {
        let weak = ui.as_weak();
        rt.spawn(async move {
            let t0 = std::time::Instant::now();
            let (sc, sp) = tokio::join!(soundcloud_likes(), spotify_likes(500));
            let local = local_files();
            let mut all: Vec<Track> = Vec::new();
            all.extend(sc);
            all.extend(sp);
            all.extend(local);
            let note = format!("{} tracks from your likes and files, loaded in {:.1} s.", all.len(), t0.elapsed().as_secs_f32());
            let _ = weak.upgrade_in_event_loop(move |ui| {
                LIBRARY.with(|l| *l.borrow_mut() = all);
                ui.set_page_sub(note.into());
                ui.invoke_nav("library".into());
            });
        });
    }

    // what happens when something's played: the engine gets its source, the bar its track
    let start: Rc<dyn Fn(&AppWindow)> = {
        let state = state.clone();
        let engine = engine.clone();
        let qrows = qrows.clone();
        let rows = rows.clone();
        let rt_handle = rt.handle().clone();
        Rc::new(move |ui: &AppWindow| {
            let (t, token) = {
                let mut s = state.borrow_mut();
                let Some(i) = s.qi else { return };
                let Some(t) = s.queue.get(i).cloned() else { return };
                s.token += 1;
                (t, s.token)
            };
            ui.set_now_title(t.title.clone().into());
            ui.set_now_artist(t.artist.clone().into());
            ui.set_now_plat(t.plat().into());
            ui.set_now_art(slint::Image::default());
            ui.set_status(format!("Loading \"{}\"...", t.title).into());
            // what's playing, marked in the lists
            {
                let s = state.borrow();
                qrows.set_vec(s.queue.iter().enumerate().map(|(k, x)| x.row(Some(k) == s.qi)).collect::<Vec<_>>());
            }
            LIBRARY.with(|l| {
                let l = l.borrow();
                for k in 0..rows.row_count() {
                    if let (Some(mut r), Some(x)) = (rows.row_data(k), l.get(k)) {
                        let on = x.title == t.title && x.artist == t.artist;
                        if r.playing != on { r.playing = on; rows.set_row_data(k, r); }
                    }
                }
            });
            let weak = ui.as_weak();
            let engine = engine.clone();
            let h = rt_handle.clone();
            let art = t.art.clone();
            rt_handle.spawn(async move {
                let src = open_source(&t, h.clone()).await;
                let pixels = fetch_art(art).await;
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    if TOKEN.with(|c| c.get()) != token { return; }
                    match src {
                        Ok(src) => {
                            engine.load(src, 0.0);
                            engine.play();
                            ui.set_playing(true);
                            ui.set_status("".into());
                        }
                        Err(e) => ui.set_status(format!("Couldn't play \"{}\": {e}", t.title).into()),
                    }
                    if let Some(p) = pixels { ui.set_now_art(slint::Image::from_rgba8(p)); }
                });
            });
            TOKEN.with(|c| c.set(token));
        })
    };

    ui.on_nav({
        let weak = ui.as_weak();
        let rows = rows.clone();
        move |page| {
            let ui = weak.upgrade().unwrap();
            ui.set_page(page.clone());
            if page == "library" {
                ui.set_page_title("Library".into());
                LIBRARY.with(|l| rows.set_vec(l.borrow().iter().map(|t| t.row(false)).collect::<Vec<_>>()));
            } else {
                ui.set_page_title("Discover".into());
            }
        }
    });

    ui.on_play_row({
        let state = state.clone();
        let start = start.clone();
        let weak = ui.as_weak();
        move |i| {
            let i = i as usize;
            {
                let mut s = state.borrow_mut();
                s.library = LIBRARY.with(|l| l.borrow().clone());
                // the list from here on is the queue (a long Library: the next 500)
                s.queue = s.library.iter().skip(i).take(500).cloned().collect();
                s.qi = Some(0);
            }
            start(&weak.upgrade().unwrap());
        }
    });

    ui.on_play_queue({
        let state = state.clone();
        let start = start.clone();
        let weak = ui.as_weak();
        move |i| {
            state.borrow_mut().qi = Some(i as usize);
            start(&weak.upgrade().unwrap());
        }
    });

    let step: Rc<Step> = {
        let state = state.clone();
        let start = start.clone();
        let engine = engine.clone();
        Rc::new(move |ui: &AppWindow, d: i32| {
            // back: the start of this track first, if it's been playing a few seconds
            if d < 0 && engine.position_ms() > 3000 {
                engine.seek(0);
                return;
            }
            let next = {
                let s = state.borrow();
                s.qi.map(|i| i as i32 + d).filter(|&n| n >= 0 && (n as usize) < s.queue.len())
            };
            if let Some(n) = next {
                state.borrow_mut().qi = Some(n as usize);
                start(ui);
            } else if d > 0 {
                engine.stop();
                ui.set_playing(false);
            }
        })
    };

    ui.on_next({ let step = step.clone(); let weak = ui.as_weak(); move || step(&weak.upgrade().unwrap(), 1) });
    ui.on_prev({ let step = step.clone(); let weak = ui.as_weak(); move || step(&weak.upgrade().unwrap(), -1) });
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

    // position and the end of a track, four times a second
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(250), {
        let weak = ui.as_weak();
        let engine = engine.clone();
        let step = step.clone();
        move || {
            let ui = weak.upgrade().unwrap();
            let (pos, dur) = (engine.position_ms(), engine.duration_ms());
            ui.set_pos_text(fmt_ms(pos).into());
            ui.set_dur_text(fmt_ms(dur).into());
            ui.set_pos_frac(if dur > 0 { (pos as f32 / dur as f32).min(1.0) } else { 0.0 });
            while let Ok(ev) = events.try_recv() {
                match ev {
                    Event::Ended => step(&ui, 1),
                    Event::Error(e) => ui.set_status(e.into()),
                }
            }
        }
    });

    // OMNIPULSE_SELFTEST=<png>: load, play the first track silently, save a picture of the window, quit
    let _selftest = std::env::var("OMNIPULSE_SELFTEST").ok().map(|png| {
        engine.set_volume(0.0);
        ui.set_volume(0.0);
        let t = slint::Timer::default();
        let weak = ui.as_weak();
        let start_at = std::time::Instant::now();
        let mut played = false;
        t.start(slint::TimerMode::Repeated, std::time::Duration::from_millis(500), move || {
            let ui = weak.upgrade().unwrap();
            if !played && ui.get_rows().row_count() > 0 {
                played = true;
                ui.invoke_play_row(0);
            }
            if start_at.elapsed().as_secs() >= 22 {
                if let Ok(shot) = ui.window().take_snapshot() {
                    let _ = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec()).map(|i| i.save(&png));
                }
                println!("selftest: {} rows, playing {}, {} / {}, status \"{}\"", ui.get_rows().row_count(), ui.get_now_title(), ui.get_pos_text(), ui.get_dur_text(), ui.get_status());
                let _ = slint::quit_event_loop();
            }
        });
        t
    });

    ui.run()?;
    drop(rt);
    Ok(())
}

/// Moves through the queue: +1 next, -1 back
type Step = dyn Fn(&AppWindow, i32);

thread_local! {
    static LIBRARY: std::cell::RefCell<Vec<Track>> = const { std::cell::RefCell::new(Vec::new()) };
    static TOKEN: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}
