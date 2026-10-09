// The player: a decode thread that reads the source, converts it to the output's rate and runs the
// chain (js/audio.js's, see dsp.rs), and an output stream (cpal: WASAPI on Windows) that plays it.
// About a third of a second is kept ready between them. Volume, mute and pause act in the output
// itself, so they're heard at once.

use crate::dsp::{level, Chain, Smoothed};
use crate::source::Source;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

const AHEAD_S: f32 = 0.35;

#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// the track played to its end
    Ended,
    Error(String),
}

/// Turns a source's sound into the chain's: its rate converted, then match / EQ / limiter
pub struct Pipeline {
    src: Box<dyn Source>,
    resampler: Option<Resampler>,
    pub chain: Chain,
    raw: Vec<f32>,
    pub level: (f32, f32),
    done: bool,
}

impl Pipeline {
    pub fn new(src: Box<dyn Source>, out_rate: u32) -> Self {
        let resampler = (src.rate() != out_rate).then(|| Resampler::new(src.rate(), out_rate));
        Self { src, resampler, chain: Chain::new(out_rate as f32), raw: Vec::new(), level: (-180.0, -180.0), done: false }
    }

    /// The next piece, appended to `out`; false once there's no more
    pub fn fill(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if self.done {
            return Ok(false);
        }
        self.raw.clear();
        let more = self.src.read(&mut self.raw)?;
        if !self.raw.is_empty() {
            self.level = level(&self.raw);
        }
        let start = out.len();
        match self.resampler.as_mut() {
            Some(r) => r.push(&self.raw, !more, out),
            None => out.extend_from_slice(&self.raw),
        }
        self.chain.process(&mut out[start..]);
        if !more {
            self.done = true;
        }
        Ok(more || out.len() > start)
    }

    pub fn seek(&mut self, ms: u64) -> Result<u64, String> {
        let at = self.src.seek(ms)?;
        if let Some(r) = self.resampler.as_mut() {
            r.reset();
        }
        self.chain.reset();
        self.done = false;
        Ok(at)
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.src.duration_ms()
    }
}

/// rubato's FFT resampler, fed any number of frames at a time
pub struct Resampler {
    inner: rubato::FftFixedIn<f32>,
    pending: [Vec<f32>; 2],
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let inner = rubato::FftFixedIn::<f32>::new(from as usize, to as usize, 1024, 2, 2).expect("resampler");
        Self { inner, pending: [Vec::new(), Vec::new()] }
    }

    fn push(&mut self, inter: &[f32], last: bool, out: &mut Vec<f32>) {
        use rubato::Resampler as _;
        for f in inter.as_chunks::<2>().0 {
            self.pending[0].push(f[0]);
            self.pending[1].push(f[1]);
        }
        loop {
            let need = self.inner.input_frames_next();
            if self.pending[0].len() < need {
                break;
            }
            let chunk: [Vec<f32>; 2] = [self.pending[0].drain(..need).collect(), self.pending[1].drain(..need).collect()];
            if let Ok(res) = self.inner.process(&chunk, None) {
                interleave(&res, out);
            }
        }
        if last && !self.pending[0].is_empty() {
            let chunk = [std::mem::take(&mut self.pending[0]), std::mem::take(&mut self.pending[1])];
            if let Ok(res) = self.inner.process_partial(Some(&chunk), None) {
                interleave(&res, out);
            }
        }
    }

    fn reset(&mut self) {
        use rubato::Resampler as _;
        self.inner.reset();
        self.pending = [Vec::new(), Vec::new()];
    }
}

fn interleave(ch: &[Vec<f32>], out: &mut Vec<f32>) {
    let n = ch[0].len().min(ch[1].len());
    out.reserve(n * 2);
    for (l, r) in ch[0][..n].iter().zip(&ch[1][..n]) {
        out.push(*l);
        out.push(*r);
    }
}

// ---------- the live player ----------
enum Cmd {
    Load(Box<dyn Source>, f32),
    Seek(u64),
    Stop,
    Eq(Vec<f32>),
    Match(f32, bool),
    Rate(u32),
}

struct Shared {
    queue: Mutex<VecDeque<f32>>,
    volume: AtomicU32,
    muted: AtomicBool,
    paused: AtomicBool,
    loaded: AtomicBool,
    /// frames played since the track's base position
    played: AtomicU64,
    base_ms: AtomicU64,
    duration_ms: AtomicU64,
    rate: AtomicU32,
    level: Mutex<(f32, f32)>,
    source_done: AtomicBool,
    /// Fade Between Tracks: where the fade is going, how far it moves each frame, and where to start
    /// it from first (NaN: from where it is)
    fade_target: AtomicU32,
    fade_step: AtomicU32,
    fade_from: AtomicU32,
}

pub struct Engine {
    shared: Arc<Shared>,
    cmd: Sender<Cmd>,
    events: Mutex<Option<Receiver<Event>>>,
    ev_tx: Sender<Event>,
    output_alive: Mutex<Option<Arc<AtomicBool>>>,
    device: Mutex<String>,
}

fn f32_load(a: &AtomicU32) -> f32 {
    f32::from_bits(a.load(Ordering::Relaxed))
}

impl Engine {
    /// Starts the engine on a device (by name; "" for the default)
    pub fn new(device: &str) -> Result<Self, String> {
        let shared = Arc::new(Shared {
            queue: Mutex::new(VecDeque::new()),
            volume: AtomicU32::new(1.0f32.to_bits()),
            muted: AtomicBool::new(false),
            paused: AtomicBool::new(true),
            loaded: AtomicBool::new(false),
            played: AtomicU64::new(0),
            base_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            rate: AtomicU32::new(48000),
            level: Mutex::new((-180.0, -180.0)),
            fade_target: AtomicU32::new(1.0f32.to_bits()),
            fade_step: AtomicU32::new(1.0f32.to_bits()),
            fade_from: AtomicU32::new(f32::NAN.to_bits()),
            source_done: AtomicBool::new(false),
        });
        let (cmd, cmd_rx) = mpsc::channel();
        let (ev_tx, ev_rx) = mpsc::channel();
        let e = Self { shared, cmd, events: Mutex::new(Some(ev_rx)), ev_tx, output_alive: Mutex::new(None), device: Mutex::new(String::new()) };
        e.open_output(device)?;
        {
            let shared = e.shared.clone();
            let ev = e.ev_tx.clone();
            std::thread::Builder::new().name("op-decode".into()).spawn(move || decode_loop(shared, cmd_rx, ev)).map_err(|e| e.to_string())?;
        }
        Ok(e)
    }

    /// The events (once): Ended, Error
    pub fn events(&self) -> Option<Receiver<Event>> {
        self.events.lock().unwrap().take()
    }

    /// Output devices by name
    pub fn devices() -> Vec<String> {
        cpal::default_host().output_devices().map(|d| d.filter_map(|x| x.name().ok()).collect()).unwrap_or_default()
    }

    /// Moves playback to another device (by name; "" for the default)
    pub fn set_device(&self, name: &str) -> Result<(), String> {
        let was = self.position_ms();
        self.open_output(name)?;
        if self.shared.loaded.load(Ordering::SeqCst) {
            self.seek(was);
        }
        Ok(())
    }

    fn open_output(&self, name: &str) -> Result<(), String> {
        if let Some(a) = self.output_alive.lock().unwrap().take() {
            a.store(false, Ordering::SeqCst);
        }
        let alive = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel::<Result<u32, String>>();
        let shared = self.shared.clone();
        let ev = self.ev_tx.clone();
        let flag = alive.clone();
        let dev_name = name.to_string();
        // a cpal stream stays on the thread that made it
        std::thread::Builder::new()
            .name("op-output".into())
            .spawn(move || {
                let made = build_output(&dev_name, shared, ev);
                match made {
                    Ok((stream, rate)) => {
                        let _ = tx.send(Ok(rate));
                        while flag.load(Ordering::SeqCst) {
                            std::thread::park_timeout(Duration::from_millis(200));
                        }
                        drop(stream);
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        let rate = rx.recv_timeout(Duration::from_secs(10)).map_err(|_| "The output device didn't answer.".to_string())??;
        *self.output_alive.lock().unwrap() = Some(alive);
        *self.device.lock().unwrap() = name.to_string();
        if self.shared.rate.swap(rate, Ordering::SeqCst) != rate {
            let _ = self.cmd.send(Cmd::Rate(rate));
        }
        Ok(())
    }

    /// Plays a source from its start (paused stays paused), at a loudness gain in dB
    pub fn load(&self, src: Box<dyn Source>, match_db: f32) {
        let _ = self.cmd.send(Cmd::Load(src, match_db));
    }
    pub fn play(&self) {
        self.shared.paused.store(false, Ordering::SeqCst);
    }
    pub fn pause(&self) {
        self.shared.paused.store(true, Ordering::SeqCst);
    }
    pub fn is_paused(&self) -> bool {
        self.shared.paused.load(Ordering::SeqCst)
    }
    pub fn stop(&self) {
        let _ = self.cmd.send(Cmd::Stop);
    }
    pub fn seek(&self, ms: u64) {
        let _ = self.cmd.send(Cmd::Seek(ms));
    }
    /// 0..1
    pub fn set_volume(&self, v: f32) {
        self.shared.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }
    pub fn set_muted(&self, m: bool) {
        self.shared.muted.store(m, Ordering::Relaxed);
    }
    /// Ten band gains in dB (all 0 when the equalizer is off)
    pub fn set_eq(&self, gains: &[f32]) {
        let _ = self.cmd.send(Cmd::Eq(gains.to_vec()));
    }
    /// A track's loudness gain: at once (a track starting) or eased in (a measurement arriving)
    /// A fade to `to` (0 silent, 1 full) over `ms` (0: at once), from `from` if given, or from
    /// where it is. It's on top of the volume.
    pub fn fade(&self, from: Option<f32>, to: f32, ms: u64) {
        let rate = self.shared.rate.load(Ordering::Relaxed).max(1) as f32;
        let step = if ms == 0 { 1.0 } else { 1000.0 / (ms as f32 * rate) };
        self.shared.fade_step.store(step.to_bits(), Ordering::Relaxed);
        self.shared.fade_target.store(to.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        if let Some(f) = from {
            self.shared.fade_from.store(f.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
        }
    }
    pub fn set_match_db(&self, db: f32, at_once: bool) {
        let _ = self.cmd.send(Cmd::Match(db, at_once));
    }
    pub fn position_ms(&self) -> u64 {
        let s = &self.shared;
        let rate = s.rate.load(Ordering::Relaxed).max(1) as u64;
        s.base_ms.load(Ordering::Relaxed) + s.played.load(Ordering::Relaxed) * 1000 / rate
    }
    pub fn duration_ms(&self) -> u64 {
        self.shared.duration_ms.load(Ordering::Relaxed)
    }
    /// How loud the source is right now, before any gain: (rms, peak) in dBFS
    pub fn level(&self) -> (f32, f32) {
        *self.shared.level.lock().unwrap()
    }
    pub fn output_rate(&self) -> u32 {
        self.shared.rate.load(Ordering::Relaxed)
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(a) = self.output_alive.lock().unwrap().take() {
            a.store(false, Ordering::SeqCst);
        }
    }
}

fn build_output(name: &str, shared: Arc<Shared>, ev: Sender<Event>) -> Result<(cpal::Stream, u32), String> {
    let host = cpal::default_host();
    let dev = if name.is_empty() {
        host.default_output_device().ok_or("There's no output device.")?
    } else {
        host.output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().map(|n| n == name).unwrap_or(false))
            .ok_or_else(|| format!("{name} isn't connected."))?
    };
    let sup = dev.default_output_config().map_err(|e| e.to_string())?;
    let fmt = sup.sample_format();
    let cfg: cpal::StreamConfig = sup.into();
    let (ch, rate) = (cfg.channels as usize, cfg.sample_rate.0);
    let mut vol = Smoothed::new(f32_load(&shared.volume), 0.02, rate as f32);
    let mut fade = 1.0f32;
    let mut fill = move |out: &mut [f32]| {
        let target = if shared.muted.load(Ordering::Relaxed) { 0.0 } else { f32_load(&shared.volume) };
        vol.set(target);
        let from = f32::from_bits(shared.fade_from.swap(f32::NAN.to_bits(), Ordering::Relaxed));
        if !from.is_nan() { fade = from; }
        let (fade_to, fade_step) = (f32_load(&shared.fade_target), f32_load(&shared.fade_step));
        if shared.paused.load(Ordering::Relaxed) {
            out.iter_mut().for_each(|s| *s = 0.0);
            return;
        }
        let mut q = shared.queue.lock().unwrap();
        let mut frames = 0u64;
        for fr in out.chunks_mut(ch.max(1)) {
            let (l, r) = match (q.pop_front(), q.pop_front()) {
                (Some(l), Some(r)) => {
                    frames += 1;
                    (l, r)
                }
                _ => (0.0, 0.0),
            };
            if fade < fade_to { fade = (fade + fade_step).min(fade_to); } else if fade > fade_to { fade = (fade - fade_step).max(fade_to); }
            let g = vol.step() * fade;
            match fr.len() {
                1 => fr[0] = (l + r) * 0.5 * g,
                _ => {
                    fr[0] = l * g;
                    fr[1] = r * g;
                    fr[2..].iter_mut().for_each(|s| *s = 0.0);
                }
            }
        }
        drop(q);
        shared.played.fetch_add(frames, Ordering::Relaxed);
    };
    let err = move |e: cpal::StreamError| {
        let _ = ev.send(Event::Error(format!("The output device stopped: {e}")));
    };
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_output_stream(&cfg, move |d: &mut [f32], _| fill(d), err, None),
        cpal::SampleFormat::I16 => {
            let mut tmp = Vec::new();
            dev.build_output_stream(&cfg, move |d: &mut [i16], _| {
                tmp.resize(d.len(), 0.0);
                fill(&mut tmp);
                d.iter_mut().zip(&tmp).for_each(|(o, s)| *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16);
            }, err, None)
        }
        other => return Err(format!("The device uses a sample format ({other}) this can't play.")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, rate))
}

fn decode_loop(shared: Arc<Shared>, cmds: Receiver<Cmd>, ev: Sender<Event>) {
    let mut pipe: Option<Pipeline> = None;
    let mut eq = vec![0.0f32; 10];
    let mut ended_sent = true;
    let mut scratch = Vec::with_capacity(8192);
    loop {
        // commands first; wait for one when there's nothing to do
        let idle = pipe.is_none() || shared.source_done.load(Ordering::SeqCst);
        let got = if idle { cmds.recv_timeout(Duration::from_millis(50)).ok() } else { cmds.try_recv().ok() };
        if let Some(c) = got {
            match c {
                Cmd::Load(src, db) => {
                    let rate = shared.rate.load(Ordering::SeqCst);
                    let mut p = Pipeline::new(src, rate);
                    p.chain.eq.set(&eq);
                    p.chain.set_match_db(db, true);
                    shared.queue.lock().unwrap().clear();
                    shared.played.store(0, Ordering::SeqCst);
                    shared.base_ms.store(0, Ordering::SeqCst);
                    shared.duration_ms.store(p.duration_ms().unwrap_or(0), Ordering::SeqCst);
                    shared.source_done.store(false, Ordering::SeqCst);
                    shared.loaded.store(true, Ordering::SeqCst);
                    pipe = Some(p);
                    ended_sent = false;
                }
                Cmd::Seek(ms) => {
                    if let Some(p) = pipe.as_mut() {
                        match p.seek(ms) {
                            Ok(at) => {
                                shared.queue.lock().unwrap().clear();
                                shared.played.store(0, Ordering::SeqCst);
                                shared.base_ms.store(at, Ordering::SeqCst);
                                shared.source_done.store(false, Ordering::SeqCst);
                                ended_sent = false;
                            }
                            Err(e) => {
                                let _ = ev.send(Event::Error(e));
                            }
                        }
                    }
                }
                Cmd::Stop => {
                    pipe = None;
                    shared.queue.lock().unwrap().clear();
                    shared.loaded.store(false, Ordering::SeqCst);
                    shared.source_done.store(true, Ordering::SeqCst);
                    ended_sent = true;
                }
                Cmd::Eq(g) => {
                    eq = g;
                    if let Some(p) = pipe.as_mut() {
                        p.chain.eq.set(&eq);
                    }
                }
                Cmd::Match(db, at_once) => {
                    if let Some(p) = pipe.as_mut() {
                        p.chain.set_match_db(db, at_once);
                    }
                }
                Cmd::Rate(rate) => {
                    // a device at another rate: rebuild the conversion and chain where playback was
                    if let Some(old) = pipe.take() {
                        let was = shared.base_ms.load(Ordering::SeqCst);
                        let mut p = Pipeline::new(old.src, rate);
                        p.chain.eq.set(&eq);
                        p.chain.matched = old.chain.matched;
                        if p.seek(was).is_ok() {
                            shared.queue.lock().unwrap().clear();
                            shared.played.store(0, Ordering::SeqCst);
                        }
                        pipe = Some(p);
                    }
                }
            }
            continue;
        }
        let Some(p) = pipe.as_mut() else { continue };
        if shared.source_done.load(Ordering::SeqCst) {
            if !ended_sent && shared.queue.lock().unwrap().is_empty() {
                ended_sent = true;
                let _ = ev.send(Event::Ended);
            }
            continue;
        }
        let rate = shared.rate.load(Ordering::Relaxed) as f32;
        let ahead = (rate * AHEAD_S) as usize * 2;
        if shared.queue.lock().unwrap().len() >= ahead {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        scratch.clear();
        match p.fill(&mut scratch) {
            Ok(more) => {
                *shared.level.lock().unwrap() = p.level;
                shared.queue.lock().unwrap().extend(scratch.iter().copied());
                if !more {
                    shared.source_done.store(true, Ordering::SeqCst);
                }
            }
            Err(e) => {
                shared.source_done.store(true, Ordering::SeqCst);
                let _ = ev.send(Event::Error(e));
            }
        }
    }
}

/// Renders a source through the chain without a device (for tests and the test player's --wav)
pub fn render(src: Box<dyn Source>, out_rate: u32, eq: &[f32], match_db: f32, max_secs: f32) -> Result<Vec<f32>, String> {
    let mut p = Pipeline::new(src, out_rate);
    p.chain.eq.set(eq);
    p.chain.set_match_db(match_db, true);
    let mut out = Vec::new();
    let limit = (max_secs * out_rate as f32) as usize * 2;
    while out.len() < limit {
        if !p.fill(&mut out)? {
            break;
        }
    }
    out.truncate(limit);
    Ok(out)
}
