// Now Playing's GPU scenes (omnipulse-vis: Smoke, Swarm, Ink, Phosphor), with Hardware Acceleration on:
// drawn on a thread of their own (its GPU device its own), a frame asked for each tick and shown when it's done,
// so the app never waits on the GPU. The sound's read here, as the scenes want it: left and right, its hits,
// its builds and drops.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Instant;

use omnipulse_vis::audio::{Analyser, Audio, WINDOW};

/// The scenes, as the styles they're picked as: their ids and names
pub fn scenes() -> Vec<(&'static str, &'static str)> { omnipulse_vis::scenes::all().iter().map(|s| (s.id, s.name)).collect() }

pub fn is_scene(style: &str) -> bool { omnipulse_vis::scenes::by_id(style).is_some() }

enum Ask {
    Frame { scene: String, w: u32, h: u32, t: f32, audio: Audio },
    Art(Option<(Vec<u8>, u32, u32)>),
}

/// A frame drawn: its pixels (RGBA, top row first) and its size
pub type Frame = (Vec<u8>, u32, u32);

pub struct Gpu {
    ask: mpsc::Sender<Ask>,
    done: Arc<Mutex<Option<Frame>>>,
    busy: Arc<AtomicBool>,
    /// (why it can't draw: no GPU, or one that wouldn't start)
    failed: Arc<Mutex<Option<String>>>,
    analyser: Analyser,
    start: Instant,
    last: Instant,
    /// (the scene it was last asked for: a new one is given the cover again, and starts from it)
    scene: String,
}

impl Gpu {
    pub fn start(rate: u32) -> Gpu {
        let (ask, asked) = mpsc::channel::<Ask>();
        let done = Arc::new(Mutex::new(None));
        let busy = Arc::new(AtomicBool::new(false));
        let failed = Arc::new(Mutex::new(None));
        let (d, b, f) = (done.clone(), busy.clone(), failed.clone());
        std::thread::Builder::new().name("vis".into()).spawn(move || {
            let mut r = match omnipulse_vis::Renderer::new() {
                Ok(r) => r,
                Err(e) => { *f.lock().unwrap() = Some(e); b.store(false, Ordering::Relaxed); return; }
            };
            // (asked for till the app lets it go: its sender dropped)
            while let Ok(a) = asked.recv() {
                match a {
                    Ask::Art(art) => r.set_art(art.as_ref().map(|(px, w, h)| (px.as_slice(), *w, *h))),
                    Ask::Frame { scene, w, h, t, audio } => {
                        if r.scene() != Some(scene.as_str()) {
                            if let Some(s) = omnipulse_vis::scenes::by_id(&scene) {
                                if let Err(e) = r.set_scene(s) { *f.lock().unwrap() = Some(e); }
                            }
                        }
                        if let Ok(px) = r.render(w, h, t, &audio) { *d.lock().unwrap() = Some((px, w.clamp(16, 4096), h.clamp(16, 4096))); }
                        b.store(false, Ordering::Relaxed);
                    }
                }
            }
        }).ok();
        Gpu { ask, done, busy, failed, analyser: Analyser::new(rate), start: Instant::now(), last: Instant::now(), scene: String::new() }
    }

    /// Whether `scene` is new since the last tick (then it's given the cover again)
    pub fn new_scene(&mut self, scene: &str) -> bool { if self.scene == scene { false } else { self.scene = scene.into(); true } }

    /// Why it can't draw, if it can't
    pub fn failed(&self) -> Option<String> { self.failed.lock().unwrap().clone() }

    /// The cover, for the scenes that use it (Smoke: the cover's smoke)
    pub fn set_art(&self, art: Option<(Vec<u8>, u32, u32)>) { let _ = self.ask.send(Ask::Art(art)); }

    /// A tick: the sound read (its last stretch, left and right), the next frame asked for (unless the last's
    /// still being drawn), and the newest frame done, if there's one since the last tick
    pub fn tick(&mut self, scene: &str, w: u32, h: u32, left: &[f32], right: &[f32]) -> Option<Frame> {
        let dt = self.last.elapsed().as_secs_f32();
        self.last = Instant::now();
        let audio = self.analyser.analyse(left, right, dt).clone();
        if !self.busy.swap(true, Ordering::Relaxed) {
            let t = self.start.elapsed().as_secs_f32();
            if self.ask.send(Ask::Frame { scene: scene.into(), w, h, t, audio }).is_err() { self.busy.store(false, Ordering::Relaxed); }
        }
        self.done.lock().unwrap().take()
    }
}

/// (how much of the sound each look takes)
pub const SOUND: usize = WINDOW;
