// The player: a decode thread that reads the source, converts it to the output's rate and runs the
// chain (js/audio.js's, see dsp.rs), and an output stream (cpal: WASAPI on Windows) that plays it.
// A crossfade plays two sources at once: the next one comes in as the one before goes out (equal
// power), each through its own chain, and then it's the one playing. A mix is a crossfade that starts
// at an exact point of the one playing (a downbeat), handed over a band at a time (a Handoff: its highs,
// its mids, its bass each at their own moment, as a DJ does on an EQ; the two kicks never thump
// together), the one going out echoed as it leaves, for beat-matched transitions.
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
    /// a mix's moment came: the next track is coming in (it's the one playing from here)
    Mixed,
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
    /// (as it plays as it is) the last moment of it read, and where its reading is (from where it
    /// started or was put: frames on from `base_ms`), for a stretch to take over from (Pipeline::stretch)
    recent: VecDeque<f32>,
    base_ms: u64,
    read_frames: u64,
    /// sound to give before reading on (a stretch's first, crossed over from it as it was)
    carry: Vec<f32>,
}

/// How much of the track playing is kept as it plays, for a stretch taking over to start from (its
/// deck warmed up on it, not heard), and how long it crosses over from it as it was (no click)
const RECENT_S: f32 = 0.5;
const TAKEOVER_MS: u64 = 30;

/// A source with a moment of it already read put back in front (for a stretch taking over)
struct Replay {
    head: Vec<f32>,
    inner: Box<dyn Source>,
}

impl Source for Replay {
    fn rate(&self) -> u32 { self.inner.rate() }
    fn duration_ms(&self) -> Option<u64> { self.inner.duration_ms() }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if !self.head.is_empty() {
            out.append(&mut self.head);
            return Ok(true);
        }
        self.inner.read(out)
    }
    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        self.head.clear();
        self.inner.seek(ms)
    }
}

impl Pipeline {
    pub fn new(src: Box<dyn Source>, out_rate: u32) -> Self {
        let resampler = (src.rate() != out_rate).then(|| Resampler::new(src.rate(), out_rate));
        Self { src, resampler, chain: Chain::new(out_rate as f32), raw: Vec::new(), level: (-180.0, -180.0), done: false, recent: VecDeque::new(), base_ms: 0, read_frames: 0, carry: Vec::new() }
    }

    /// From here on, at a tempo (its pitch kept: crate::stretch), the track playing as it was until now:
    /// its deck warmed up on the last moment of it (not heard), lined up to the frame on where it is,
    /// and crossed over to from it as it was over 30 ms, so the change isn't heard. Once only.
    pub fn stretch(&mut self, tempo: crate::stretch::Tempo) -> Result<(), String> {
        if self.src.position_ms().is_some() || self.done { return Ok(()); }
        let rate = self.src.rate();
        // a moment more of it as it is, to cross over from
        let want = (TAKEOVER_MS * rate as u64 / 1000) as usize * 2;
        let mut ahead = std::mem::take(&mut self.carry);
        while ahead.len() < want {
            if !self.src.read(&mut ahead)? { break; }
        }
        let recent: Vec<f32> = self.recent.drain(..).collect();
        let back = recent.len() / 2;
        let now_ms = self.base_ms + self.read_frames * 1000 / rate.max(1) as u64;
        let start_ms = now_ms.saturating_sub(back as u64 * 1000 / rate.max(1) as u64);
        let mut head = recent;
        head.extend_from_slice(&ahead);
        let old = std::mem::replace(&mut self.src, Box::new(crate::source::Tone::new(rate, 0.0, 0.0, 0.0)));
        let mut st = match crate::stretch::Stretched::starting(Box::new(Replay { head, inner: old }), tempo, start_ms) {
            Ok(st) => st,
            Err(e) => return Err(e),
        };
        // warmed up: what it gives before where it is (its own delay, then the moment kept) let go, to the frame
        let mut out = Vec::new();
        let mut block = Vec::new();
        while out.len() < ahead.len().max(2) {
            let p0 = st.source_frames();
            block.clear();
            if !st.read(&mut block)? { break; }
            let (p1, n) = (st.source_frames(), block.len() / 2);
            if n == 0 || p1 < back as f64 { continue; }
            let step = (p1 - p0) / n as f64;
            let skip = if step > 0.0 { (((back as f64 - p0) / step).ceil().max(0.0) as usize).min(n) } else { 0 };
            out.extend_from_slice(&block[skip * 2..]);
        }
        // crossed over to it from it as it was (the same moment)
        let m = (ahead.len() / 2).min(out.len() / 2);
        for i in 0..m {
            let w = i as f32 / m.max(1) as f32;
            for c in 0..2 { out[i * 2 + c] = ahead[i * 2 + c] * (1.0 - w) + out[i * 2 + c] * w; }
        }
        self.carry = out;
        self.src = Box::new(st);
        Ok(())
    }

    /// The next piece, appended to `out`; false once there's no more
    pub fn fill(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if self.done {
            return Ok(false);
        }
        self.raw.clear();
        let more = if self.carry.is_empty() { self.src.read(&mut self.raw)? } else { self.raw = std::mem::take(&mut self.carry); true };
        if !self.raw.is_empty() {
            self.level = level(&self.raw);
        }
        // (as it is: its last moment kept, and how far it's read, for a stretch to take over)
        if self.src.position_ms().is_none() {
            self.read_frames += self.raw.len() as u64 / 2;
            self.recent.extend(self.raw.iter().copied());
            let cap = (RECENT_S * self.src.rate() as f32) as usize * 2;
            if self.recent.len() > cap { let over = self.recent.len() - cap; self.recent.drain(..over); }
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
        (self.base_ms, self.read_frames) = (at, 0);
        self.recent.clear();
        self.carry.clear();
        Ok(at)
    }

    pub fn duration_ms(&self) -> Option<u64> {
        self.src.duration_ms()
    }

    /// Where its source says it is in its own time (a stretched one's), if it says
    pub fn src_position_ms(&self) -> Option<u64> {
        self.src.position_ms()
    }
}

/// rubato's FFT resampler, fed any number of frames at a time
pub struct Resampler {
    inner: rubato::FftFixedIn<f32>,
    pending: [Vec<f32>; 2],
    /// its own delay (frames of output: up to 80 ms between rates with little in common, as a key
    /// shift's), still to be let go: what it gives is where its input was, to the frame
    skip: usize,
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        let inner = rubato::FftFixedIn::<f32>::new(from as usize, to as usize, 1024, 2, 2).expect("resampler");
        let skip = { use rubato::Resampler as _; inner.output_delay() };
        Self { inner, pending: [Vec::new(), Vec::new()], skip }
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
                self.give(&res, out);
            }
        }
        // (the end: as much again as its delay pushed through, so what's inside it comes out too)
        if last && self.skip == 0 && !self.pending[0].is_empty() {
            use rubato::Resampler as _;
            let tail = self.inner.output_delay() * self.inner.input_frames_next() / self.inner.output_frames_next().max(1) + 1;
            for ch in &mut self.pending { ch.extend(std::iter::repeat_n(0.0, tail)); }
            loop {
                let need = self.inner.input_frames_next();
                if self.pending[0].len() < need { break; }
                let chunk: [Vec<f32>; 2] = [self.pending[0].drain(..need).collect(), self.pending[1].drain(..need).collect()];
                if let Ok(res) = self.inner.process(&chunk, None) { self.give(&res, out); }
            }
        }
        if last && !self.pending[0].is_empty() {
            let chunk = [std::mem::take(&mut self.pending[0]), std::mem::take(&mut self.pending[1])];
            if let Ok(res) = self.inner.process_partial(Some(&chunk), None) {
                self.give(&res, out);
            }
        }
    }

    /// What it made, less what's left of its own delay
    fn give(&mut self, res: &[Vec<f32>], out: &mut Vec<f32>) {
        let n = res[0].len().min(res[1].len());
        let drop = self.skip.min(n);
        self.skip -= drop;
        if drop == 0 { return interleave(res, out); }
        interleave(&[res[0][drop..n].to_vec(), res[1][drop..n].to_vec()], out);
    }

    fn reset(&mut self) {
        use rubato::Resampler as _;
        self.inner.reset();
        self.pending = [Vec::new(), Vec::new()];
        self.skip = self.inner.output_delay();
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
    /// the next source, coming in over this many ms as the one playing goes out
    Crossfade(Box<dyn Source>, f32, u64, Option<Handoff>),
    /// a mix: the next source (at the place it starts from), coming in when the one playing reaches
    /// `at_ms` (sample-exact), over `ms`, handed over as `handoff` says (None: a plain crossfade)
    MixAt { src: Box<dyn Source>, db: f32, at_ms: u64, ms: u64, handoff: Option<Handoff> },
    /// the track playing at a tempo from here on (its pitch kept), as the handle says
    StretchPlaying(crate::stretch::Tempo),
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
    /// frames of those that were the track before's (a crossfade starts behind what's queued)
    skew: AtomicU64,
    /// a crossfade under way
    fading_in: AtomicBool,
    /// a mix waiting for its moment
    mix_waiting: AtomicBool,
    /// frames given to the output since the base position (for where a stretched source is)
    produced: AtomicU64,
    /// a stretched source's place in its own time, less where the frames given say it is (ms)
    stretch_off: std::sync::atomic::AtomicI64,
    base_ms: AtomicU64,
    duration_ms: AtomicU64,
    rate: AtomicU32,
    level: Mutex<(f32, f32)>,
    source_done: AtomicBool,
    /// frames the output wanted mid-track and had none for (a gap heard), since last asked
    dry: AtomicU64,
    /// Fade Between Tracks: where the fade is going, how far it moves each frame, and where to start
    /// it from first (NaN: from where it is)
    fade_target: AtomicU32,
    fade_step: AtomicU32,
    fade_from: AtomicU32,
    /// the sound before the volume (stereo, interleaved), for other outputs (OmniPulse Offset, casting), by name
    tap: Mutex<Vec<(String, Tap)>>,
    /// the main output held back by this many frames (a second output that has to be earlier)
    main_delay: AtomicU32,
    /// how hard the engine works (an audit of Seamless Mix's cost): for playing as it is, through the
    /// stretch (a mix's tempo, and after one), and mixing two at once: (ns of work, frames of sound made)
    perf: Mutex<[(u64, u64); 3]>,
}

/// What the engine's work cost, by what it was doing (Engine::perf)
#[derive(Clone, Copy, Debug, Default)]
pub struct Perf {
    /// (ns of work, frames of sound made): playing a source as it is, through the stretch, mixing
    pub plain: (u64, u64),
    pub stretched: (u64, u64),
    pub mixing: (u64, u64),
}

impl Perf {
    /// The share of one core it took (0..1) for a (work, frames) pair at `rate`
    pub fn load((ns, frames): (u64, u64), rate: u32) -> Option<f64> {
        (frames > 0).then(|| ns as f64 / 1e9 / (frames as f64 / rate.max(1) as f64))
    }
}

fn perf_add(shared: &Shared, what: usize, since: std::time::Instant, frames: u64) {
    if let Ok(mut p) = shared.perf.lock() {
        p[what].0 += since.elapsed().as_nanos() as u64;
        p[what].1 += frames;
    }
}

/// Gets the sound as it plays (stereo, interleaved, before the volume), at the output's rate
pub type Tap = Box<dyn FnMut(&[f32]) + Send>;

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
            skew: AtomicU64::new(0),
            fading_in: AtomicBool::new(false),
            mix_waiting: AtomicBool::new(false),
            produced: AtomicU64::new(0),
            stretch_off: std::sync::atomic::AtomicI64::new(0),
            base_ms: AtomicU64::new(0),
            duration_ms: AtomicU64::new(0),
            rate: AtomicU32::new(48000),
            level: Mutex::new((-180.0, -180.0)),
            fade_target: AtomicU32::new(1.0f32.to_bits()),
            fade_step: AtomicU32::new(1.0f32.to_bits()),
            fade_from: AtomicU32::new(f32::NAN.to_bits()),
            source_done: AtomicBool::new(false),
            dry: AtomicU64::new(0),
            tap: Mutex::new(Vec::new()),
            main_delay: AtomicU32::new(0),
            perf: Mutex::new([(0, 0); 3]),
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
    /// Plays a source from its start, coming in over `ms` as the one playing goes out (a true crossfade:
    /// both are heard at once); from then on it's the one playing (its position, length and end)
    pub fn crossfade(&self, src: Box<dyn Source>, match_db: f32, ms: u64) {
        let _ = self.cmd.send(Cmd::Crossfade(src, match_db, ms, None));
    }
    /// The same, handed over as `handoff` has it (ms from the crossfade's start)
    pub fn crossfade_with(&self, src: Box<dyn Source>, match_db: f32, ms: u64, handoff: Handoff) {
        let _ = self.cmd.send(Cmd::Crossfade(src, match_db, ms, Some(handoff)));
    }
    /// A crossfade is under way
    pub fn crossfading(&self) -> bool {
        self.shared.fading_in.load(Ordering::SeqCst)
    }
    /// A mix: `src` (already where it starts from: its first downbeat, say) comes in when the one
    /// playing reaches `at_ms` (exactly), over `ms`, handed over as `handoff` says
    pub fn mix_at(&self, src: Box<dyn Source>, match_db: f32, at_ms: u64, ms: u64, handoff: Option<Handoff>) {
        let _ = self.cmd.send(Cmd::MixAt { src, db: match_db, at_ms, ms, handoff });
    }
    /// The track playing at `tempo` from here on, its pitch kept (a mix slowing it into the next one's
    /// tempo): it goes on as it was (start the handle at 1.0), then follows the handle
    pub fn stretch_playing(&self, tempo: crate::stretch::Tempo) {
        let _ = self.cmd.send(Cmd::StretchPlaying(tempo));
    }
    /// A mix is waiting for its moment, or under way
    /// What the engine's work has cost so far (since it started), by what it was doing
    pub fn perf(&self) -> Perf {
        let p = self.shared.perf.lock().map(|p| *p).unwrap_or_default();
        Perf { plain: p[0], stretched: p[1], mixing: p[2] }
    }
    /// The output's rate (frames a second)
    pub fn rate(&self) -> u32 {
        self.shared.rate.load(Ordering::Relaxed)
    }
    pub fn mixing(&self) -> bool {
        self.shared.mix_waiting.load(Ordering::SeqCst) || self.shared.fading_in.load(Ordering::SeqCst)
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
        let at = s.base_ms.load(Ordering::Relaxed) + s.played.load(Ordering::Relaxed).saturating_sub(s.skew.load(Ordering::Relaxed)) * 1000 / rate;
        // (a stretched source: where it is in its own time)
        (at as i64 + s.stretch_off.load(Ordering::Relaxed)).max(0) as u64
    }
    pub fn duration_ms(&self) -> u64 {
        self.shared.duration_ms.load(Ordering::Relaxed)
    }
    /// How much went unheard (ms of silence where the track should have been) since last asked
    pub fn take_gap_ms(&self) -> u64 {
        let rate = self.shared.rate.load(Ordering::Relaxed).max(1) as u64;
        self.shared.dry.swap(0, Ordering::Relaxed) * 1000 / rate
    }
    /// How loud the source is right now, before any gain: (rms, peak) in dBFS
    pub fn level(&self) -> (f32, f32) {
        *self.shared.level.lock().unwrap()
    }
    /// Another output's feed, by name (None: that one stops)
    pub fn set_tap(&self, name: &str, tap: Option<Tap>) {
        let mut taps = self.shared.tap.lock().unwrap();
        taps.retain(|(n, _)| n != name);
        if let Some(t) = tap {
            taps.push((name.to_string(), t));
        }
    }

    /// Holds the main output back (for a second output that has to be heard earlier)
    pub fn set_main_delay_ms(&self, ms: u32) {
        let frames = (ms as u64 * self.shared.rate.load(Ordering::Relaxed) as u64 / 1000) as u32;
        self.shared.main_delay.store(frames, Ordering::Relaxed);
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
    // the main output's delay line (empty unless it's held back), and the tap's piece
    let mut held: VecDeque<(f32, f32)> = VecDeque::new();
    let mut tapped: Vec<f32> = Vec::new();
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
        let delay = shared.main_delay.load(Ordering::Relaxed) as usize;
        let tapping = !shared.tap.lock().unwrap().is_empty();
        tapped.clear();
        for fr in out.chunks_mut(ch.max(1)) {
            let (l, r) = match (q.pop_front(), q.pop_front()) {
                (Some(l), Some(r)) => {
                    frames += 1;
                    (l, r)
                }
                _ => (0.0, 0.0),
            };
            if fade < fade_to { fade = (fade + fade_step).min(fade_to); } else if fade > fade_to { fade = (fade - fade_step).max(fade_to); }
            if tapping { tapped.push(l * fade); tapped.push(r * fade); }
            // held back: what's heard now is what came `delay` frames ago
            let (l, r) = if delay == 0 && held.is_empty() {
                (l, r)
            } else {
                held.push_back((l, r));
                while held.len() > delay + 1 { held.pop_front(); }
                if held.len() <= delay { (0.0, 0.0) } else { held.pop_front().unwrap_or((0.0, 0.0)) }
            };
            let g = vol.step();
            let (l, r) = (l * fade, r * fade);
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
        // (a gap: wanted and none there, with a track still going)
        let wanted = (out.len() / ch.max(1)) as u64;
        if frames < wanted && shared.loaded.load(Ordering::Relaxed) && !shared.source_done.load(Ordering::Relaxed) {
            shared.dry.fetch_add(wanted - frames, Ordering::Relaxed);
        }
        shared.played.fetch_add(frames, Ordering::Relaxed);
        if tapping {
            for (_, t) in shared.tap.lock().unwrap().iter_mut() { t(&tapped); }
        }
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

/// Two sources at once: the one going out, the one coming in, and how far the fade is (in frames)
struct Fading {
    next: Pipeline,
    total: u64,
    done: u64,
    out_buf: VecDeque<f32>,
    in_buf: VecDeque<f32>,
    out_ended: bool,
    in_ended: bool,
    /// a mix's handover, in frames (None: a plain crossfade); each side's bands apart, and the echo
    handoff: Option<Handoff>,
    bands: [Bands; 2],
    echo: Echo,
    /// each side's swept filter (going out, coming in: two Butterworths, a Linkwitz-Riley side), and
    /// the rate (to tune them)
    sweeps: [[Butter; 2]; 2],
    rate: u32,
    /// a tape stop under way: where it is in what's read of the one going out (frames, fractional)
    brake_at: f64,
    /// a tape start under way: where it is in what's read of the one coming in, and how far behind it
    /// has fallen (frames: it reads less than it plays while it spins up; its position put right after)
    spin_at: f64,
    pub spin_lag: f64,
    /// each side's loop as it's heard the first time (frames, interleaved), for playing it again
    loops: [Vec<f32>; 2],
    /// each side's centre cut (its voice handed over)
    centres: [Centre; 2],
}

/// One band's handover in a mix: the one coming in up from `in_at` over `in_len`, the one going out
/// down from `out_at` over `out_len` (equal power; ms from the mix's start, or frames in the engine)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ramp {
    pub in_at: u64,
    pub in_len: u64,
    pub out_at: u64,
    pub out_len: u64,
}

impl Ramp {
    /// both at once: a swap (over `len`) at `at`
    pub fn swap(at: u64, len: u64) -> Self { Self { in_at: at, in_len: len, out_at: at, out_len: len } }
    fn gains(&self, f: u64) -> (f32, f32) {
        let q = |at: u64, len: u64| if f < at { 0.0 } else { ((f - at) as f32 / len.max(1) as f32).min(1.0) };
        (((q(self.out_at, self.out_len)) * std::f32::consts::FRAC_PI_2).cos(), (q(self.in_at, self.in_len) * std::f32::consts::FRAC_PI_2).sin())
    }
    fn frames(&self, rate: u64) -> Self {
        let f = |ms: u64| ms * rate / 1000;
        Self { in_at: f(self.in_at), in_len: f(self.in_len), out_at: f(self.out_at), out_len: f(self.out_len) }
    }
}

/// A filter (high-pass or low-pass; 24 dB an octave, a Linkwitz-Riley's side) swept through its
/// `points` (a cutoff at each moment: ms from the mix's start, or frames in the engine), exponentially
/// between them (as the ear hears it); the first one's before them, the last one's after
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sweep {
    pub points: [(u64, f32); 5],
    pub high: bool,
}

impl Sweep {
    fn hz_at(&self, f: u64) -> f32 {
        let pts = &self.points;
        if f <= pts[0].0 { return pts[0].1; }
        for w in pts.windows(2) {
            let ((a, ha), (b, hb)) = (w[0], w[1]);
            if f <= b {
                let q = if b > a { (f - a) as f32 / (b - a) as f32 } else { 1.0 };
                return ha * (hb / ha).powf(q);
            }
        }
        pts[4].1
    }
    fn frames(&self, rate: u64) -> Self { Self { points: self.points.map(|(t, hz)| (t * rate / 1000, hz)), high: self.high } }
}

/// A wipe's split: from up here (the very top), easing down (to WIPE_HALF_HZ halfway) to above the
/// voices' presence (by the beat before the swap: the one playing still bright, its voice clear),
/// then on down to the bass (where the kicks are) by the swap
const WIPE_TOP_HZ: f32 = 16_000.0;
const WIPE_HALF_HZ: f32 = 10_000.0;
const WIPE_VOICE_HZ: f32 = 5_000.0;
const WIPE_BASS_HZ: f32 = 160.0;

/// How a mix hands over, a band at a time: its bass (below ~150 Hz), its mids (to ~2.5 kHz: the
/// vocals, mostly) and its highs (the hats, the air), and the one going out echoed as it leaves
/// (`echo`: sent from, for, its delay; all ms from the mix's start); each side through a filter
/// swept as it goes (`out_sweep`) or comes in (`in_sweep`), if it is
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Handoff {
    pub low: Ramp,
    pub mid: Ramp,
    pub high: Ramp,
    pub echo: Option<(u64, u64, u64)>,
    pub out_sweep: Option<Sweep>,
    pub in_sweep: Option<Sweep>,
    /// a tape stop: the one going out slowed to a halt (speed and pitch together, as a turntable's
    /// motor cut: no keylock) from `.0` over `.1` (ms from the mix's start, or frames), silent after
    pub brake: Option<(u64, u64)>,
    /// a tape start: the one coming in silent until `.0`, then spun up from a halt to its speed over
    /// `.1` (speed and pitch together, as a turntable's motor switched on)
    pub spin: Option<(u64, u64)>,
    /// a loop of the one going out, or of the one coming in (as a DJ's: a beat or a few bars of it
    /// played again, on the beat, so nothing of it need be cut short to fit the mix)
    pub out_loop: Option<Loop>,
    pub in_loop: Option<Loop>,
    /// a roll of the one going out (as Traktor's Beatmasher): its bar from the mix's start heard, then
    /// that bar's first half twice, its first quarter four times... a bar each, a build
    pub roll: Option<Roll>,
    /// the voice handed over (a centre cut, as a DJ app's vocal/instrumental switch): the one going out's
    /// taken away from `.0` over `.1`; the one coming in's held out until `.0`, then brought up over
    /// `.1` (ms from the mix's start, or frames). The two voices never together.
    pub out_voice: Option<(u64, u64)>,
    pub in_voice: Option<(u64, u64)>,
}

/// A roll: `bar` (ms, or frames) from the mix's start played, then over each of the next `bars - 1`
/// bars, the first 1/2, 1/4, 1/8... of it again and again (a bar's worth each time)
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Roll {
    pub bar: u64,
    pub bars: u32,
}

impl Roll {
    fn frames(&self, rate: u64) -> Self { Self { bar: (self.bar * rate / 1000).max(8), bars: self.bars } }
    /// where frame `f` of the mix is: None, heard as it is (the first bar, being kept); else where in the
    /// kept bar it's playing from, and that piece's length (for its seams)
    fn at(&self, f: u64) -> Option<(u64, u64)> {
        let k = f / self.bar;
        if k == 0 || k >= self.bars as u64 { return None; }
        let len = (self.bar >> k.min(6)).max(1);
        Some(((f % self.bar) % len, len))
    }
}

/// A loop in a mix: `len` of a song from `from` (ms from the mix's start, or frames) played, then
/// played again `reps` more times; then it goes on from where the loop ended
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Loop {
    pub from: u64,
    pub len: u64,
    pub reps: u32,
}

impl Loop {
    fn frames(&self, rate: u64) -> Self { Self { from: self.from * rate / 1000, len: (self.len * rate / 1000).max(1), reps: self.reps } }
    /// where frame `f` of the mix is in it: (pass, where in the pass), or None outside it
    fn at(&self, f: u64) -> Option<(u64, u64)> {
        (f >= self.from && f < self.from + self.len * (1 + self.reps as u64)).then(|| ((f - self.from) / self.len, (f - self.from) % self.len))
    }
    /// (its seams smoothed: a millisecond and a half either side, no click where it starts over)
    fn seam(&self, k: u64) -> f32 {
        const W: f32 = 64.0;
        ((k + 1) as f32 / W).min((self.len - k) as f32 / W).min(1.0)
    }
}


/// The centre's voice range (the vocals, mostly: what's in both channels alike, 200 Hz to 6 kHz) turned
/// down, the rest as it is: the kick and bass (centred too, but below it), the hats and air (above it),
/// and everything panned (the sides) kept. Both the centre and the sides go through the same three-way
/// split, so at full it's the signal (its phase turned, as the band handover's is).
#[derive(Clone, Copy)]
struct Centre {
    mid: [Crossover; 3],
    side: [Crossover; 3],
}

const VOICE_LOW_HZ: f32 = 200.0;
const VOICE_HIGH_HZ: f32 = 6000.0;

impl Centre {
    fn new(rate: u32) -> Self {
        let three = [Crossover::new(rate, VOICE_LOW_HZ), Crossover::new(rate, VOICE_HIGH_HZ), Crossover::new(rate, VOICE_HIGH_HZ)];
        Self { mid: three, side: three }
    }
    /// One band split three ways (its bass through the upper crossover too, for its phase)
    fn three(c: &mut [Crossover; 3], x: f32) -> (f32, f32, f32) {
        let (l, rest) = c[0].split(0, x);
        let (m, h) = c[1].split(0, rest);
        let (a, b) = c[2].split(0, l);
        (a + b, m, h)
    }
    /// A frame with its centre's voice at `g` (1: as it is; 0: gone)
    fn run(&mut self, l: f32, r: f32, g: f32) -> (f32, f32) {
        let (m, s) = ((l + r) * 0.5, (l - r) * 0.5);
        let (ml, mm, mh) = Self::three(&mut self.mid, m);
        let (sl, sm, sh) = Self::three(&mut self.side, s);
        let (m, s) = (ml + mm * g + mh, sl + sm + sh);
        (m + s, m - s)
    }
}

impl Handoff {
    /// The same, its voices handed over too: the one going out's taken away over the `beat_ms * 2` before
    /// `at`, the one coming in's brought up over the two after it
    pub fn with_voices(mut self, at: u64, beat_ms: f64) -> Self {
        let two = (beat_ms * 2.0).round() as u64;
        self.out_voice = Some((at.saturating_sub(two), two));
        self.in_voice = Some((at, two));
        self
    }

    /// A plain crossfade over `ms` (equal power, all of it at once), the voices handed over halfway: the
    /// one going out's gone by then, the one coming in's up from then, so they're never sung together
    pub fn voiced_crossfade(ms: u64) -> Self {
        let all = Ramp::swap(0, ms);
        let turn = (ms / 5).clamp(300, 1500);
        Self { low: all, mid: all, high: all, echo: None, out_sweep: None, in_sweep: None, brake: None, spin: None, out_loop: None, in_loop: None, roll: None, out_voice: Some(((ms / 2).saturating_sub(turn), turn)), in_voice: Some((ms / 2, turn)) }
    }

    /// The bass alone swapped at `at` (over a quarter second), the rest crossfaded over all `ms`
    pub fn bass_swap(ms: u64, at: u64) -> Self {
        let all = Ramp::swap(0, ms);
        Self { low: Ramp::swap(at, 250), mid: all, high: all, echo: None, out_sweep: None, in_sweep: None, brake: None, spin: None, out_loop: None, in_loop: None, roll: None, out_voice: None, in_voice: None }
    }

    /// Staged over `ms` (of beats `beat_ms` long), swapped at `swap` (a downbeat): the highs cross over
    /// (equal power: never two sets of hats at full) from 4 bars before the swap to 2 bars after it; at
    /// the swap, the mids and bass come in as the other's go (the mids over the two beats leading to it,
    /// the bass at once on it: one voice then the other, the kicks never together), the one going out's
    /// mids echoed as they go
    pub fn staged(ms: u64, beat_ms: f64, swap: u64) -> Self {
        let beat = beat_ms.round() as u64;
        let highs_from = swap.saturating_sub(16 * beat);
        let highs_to = (swap + 8 * beat).min(ms);
        Self {
            high: Ramp::swap(highs_from, highs_to - highs_from),
            mid: Ramp::swap(swap.saturating_sub(2 * beat), 2 * beat),
            low: Ramp::swap(swap.saturating_sub(60), 120),
            // (a dotted eighth's echo: three quarters of a beat; sent for the two beats before the swap and the one after)
            echo: Some((swap.saturating_sub(2 * beat), 3 * beat, (beat_ms * 0.75).round() as u64)),
            out_sweep: None,
            in_sweep: None,
            brake: None,
            spin: None,
            out_loop: None,
            in_loop: None,
            roll: None,
            out_voice: None,
            in_voice: None,
        }
    }

    /// A wipe, swapped at `swap` (a downbeat; of beats `beat_ms` long): one split sweeps down through
    /// the sound, the one going out heard below it (a low-pass) and the one coming in above it (a
    /// high-pass), so no part of the sound is ever both's. Up to the swap's last beat it only goes as far
    /// as where the voices start (the one coming in's air and hats, over the other); the voices are the
    /// one going out's until then. Over that last beat it goes down to the bass, and on the swap the one
    /// coming in's voice, bass and kick are all in (its high-pass gliding open) and the one going out is
    /// gone: the voices and the kicks never together. The two sides add up to all of it, so it's as loud
    /// all through.
    pub fn wipe(beat_ms: f64, swap: u64) -> Self {
        let beat = (beat_ms.round() as u64).min(swap);
        let glide = 120;
        let side = Ramp { in_at: 0, in_len: beat, out_at: swap, out_len: glide };
        let down = [(0, WIPE_TOP_HZ), ((swap - beat) / 2, WIPE_HALF_HZ), (swap - beat, WIPE_VOICE_HZ), (swap, WIPE_BASS_HZ)];
        Self {
            low: side,
            mid: side,
            high: side,
            echo: None,
            out_sweep: Some(Sweep { points: [down[0], down[1], down[2], down[3], down[3]], high: false }),
            in_sweep: Some(Sweep { points: [down[0], down[1], down[2], down[3], (swap + glide, 10.0)], high: true }),
            brake: None,
            spin: None,
            out_loop: None,
            in_loop: None,
            roll: None,
            out_voice: None,
            in_voice: None,
        }
    }

    /// A tape stop over `ms`: the one going out slows to a halt (its speed falling evenly to nothing, its
    /// pitch with it), its last moment faded; the one coming in starts as it stops (at once, but for a
    /// few ms against a click). For tempos too far apart to blend on the beat: one stops, the other goes.
    pub fn tape_stop(ms: u64) -> Self {
        let side = Ramp { in_at: ms, in_len: 25, out_at: ms.saturating_sub(60), out_len: 60 };
        Self { low: side, mid: side, high: side, echo: None, out_sweep: None, in_sweep: None, brake: Some((0, ms)), spin: None, out_loop: None, in_loop: None, roll: None, out_voice: None, in_voice: None }
    }

    /// A cut on the beat at `swap`: the one going out to the moment, the one coming in from it (a
    /// few ms each way against a click)
    pub fn cut(swap: u64) -> Self {
        let side = Ramp { in_at: swap, in_len: 8, out_at: swap.saturating_sub(12), out_len: 12 };
        Self { low: side, mid: side, high: side, echo: None, out_sweep: None, in_sweep: None, brake: None, spin: None, out_loop: None, in_loop: None, roll: None, out_voice: None, in_voice: None }
    }

    /// Wordplay: the one going out's word (a beat from the mix's start) looped `reps` more times, then
    /// cut on the beat at `swap` as the next one comes in singing the same word
    pub fn word_cut(swap: u64, beat: u64, reps: u32) -> Self {
        let side = Ramp { in_at: swap, in_len: 8, out_at: swap.saturating_sub(12), out_len: 12 };
        Self { low: side, mid: side, high: side, echo: None, out_sweep: None, in_sweep: None, brake: None, spin: None, out_loop: Some(Loop { from: 0, len: beat, reps }), in_loop: None, roll: None, out_voice: None, in_voice: None }
    }

    /// A roll out (as Traktor's Beatmasher): the one going out's bar from the mix's start played, then
    /// rolled over `bars - 1` more bars (its first half twice, its first quarter four times...), a
    /// high-pass rising through it, the build; on the roll's end the one coming in lands at once on its
    /// downbeat (none of it read before), the one going out cut. Each at its own tempo.
    pub fn roll(bar_ms: u64, bars: u32) -> Self {
        let end = bar_ms * bars as u64;
        let side = Ramp { in_at: end, in_len: 8, out_at: end.saturating_sub(12), out_len: 12 };
        let rise = [(0, 20.0), (bar_ms, 20.0), (bar_ms * 2, 120.0), (end.saturating_sub(bar_ms / 2), 900.0), (end, 1400.0)];
        Self { low: side, mid: side, high: side, echo: None, out_sweep: Some(Sweep { points: rise, high: true }), in_sweep: None, brake: None, spin: Some((end, 1)), out_loop: None, in_loop: None, roll: Some(Roll { bar: bar_ms, bars }), out_voice: None, in_voice: None }
    }

    /// A tape stop over `stop`, with a tape start over `start` in its last part: the one going out slows to
    /// a halt while, under its end, the one coming in spins up from one to its speed, arriving at it as
    /// the other stops (no gap between them). For songs too far apart (in tempo or in key) to blend.
    pub fn tape(stop: u64, start: u64) -> Self {
        let start = start.clamp(1, stop.max(1));
        let from = stop - start;
        let side = Ramp { in_at: from, in_len: 10, out_at: stop.saturating_sub(60), out_len: 60 };
        Self { low: side, mid: side, high: side, echo: None, out_sweep: None, in_sweep: None, brake: Some((0, stop)), spin: Some((from, start)), out_loop: None, in_loop: None, roll: None, out_voice: None, in_voice: None }
    }

    fn frames(&self, rate: u64) -> Self {
        let f = |ms: u64| ms * rate / 1000;
        Self {
            low: self.low.frames(rate), mid: self.mid.frames(rate), high: self.high.frames(rate), echo: self.echo.map(|(a, b, c)| (f(a), f(b), f(c.max(1)))),
            out_sweep: self.out_sweep.map(|w| w.frames(rate)), in_sweep: self.in_sweep.map(|w| w.frames(rate)),
            brake: self.brake.map(|(a, b)| (f(a), f(b))),
            spin: self.spin.map(|(a, b)| (f(a), f(b))),
            out_loop: self.out_loop.map(|l| l.frames(rate)),
            in_loop: self.in_loop.map(|l| l.frames(rate)),
            roll: self.roll.map(|r| r.frames(rate)),
            out_voice: self.out_voice.map(|(a, b)| (f(a), f(b).max(1))),
            in_voice: self.in_voice.map(|(a, b)| (f(a), f(b).max(1))),
        }
    }
}

/// A second-order Butterworth low- or high-pass (the Audio EQ Cookbook, Q = 1/sqrt 2), stereo
#[derive(Clone, Copy)]
struct Butter {
    b: [f32; 3],
    a: [f32; 2],
    z: [[f32; 4]; 2],
}

impl Butter {
    fn new(rate: u32, hz: f32, high: bool) -> Self {
        let mut f = Self { b: [1.0, 0.0, 0.0], a: [0.0, 0.0], z: [[0.0; 4]; 2] };
        f.tune(rate, hz, high, std::f32::consts::FRAC_1_SQRT_2);
        f
    }
    /// Its cutoff (and resonance) moved, what it's playing kept (for a sweep)
    fn tune(&mut self, rate: u32, hz: f32, high: bool, q: f32) {
        let w0 = 2.0 * std::f32::consts::PI * hz.clamp(10.0, rate as f32 * 0.45) / rate as f32;
        let (s, c) = w0.sin_cos();
        let alpha = s / (2.0 * q);
        let a0 = 1.0 + alpha;
        let b = if high { [(1.0 + c) / 2.0, -(1.0 + c), (1.0 + c) / 2.0] } else { [(1.0 - c) / 2.0, 1.0 - c, (1.0 - c) / 2.0] };
        self.b = b.map(|x| x / a0);
        self.a = [-2.0 * c / a0, (1.0 - alpha) / a0];
    }
    fn run(&mut self, ch: usize, x: f32) -> f32 {
        let [x1, x2, y1, y2] = self.z[ch];
        let y = self.b[0] * x + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
        self.z[ch] = [x, x1, y, y1];
        y
    }
}

/// A Linkwitz-Riley crossover (24 dB an octave: two Butterworths each way); its two sides add up to
/// the signal, its phase turned (an all-pass)
#[derive(Clone, Copy)]
struct Crossover {
    lo: [Butter; 2],
    hi: [Butter; 2],
}

impl Crossover {
    fn new(rate: u32, hz: f32) -> Self { Self { lo: [Butter::new(rate, hz, false); 2], hi: [Butter::new(rate, hz, true); 2] } }
    fn split(&mut self, ch: usize, x: f32) -> (f32, f32) {
        let l = self.lo[0].run(ch, x);
        let l = self.lo[1].run(ch, l);
        let h = self.hi[0].run(ch, x);
        let h = self.hi[1].run(ch, h);
        (l, h)
    }
}

/// A signal in three bands: its bass (below 150 Hz), its mids (to 2.5 kHz) and its highs, steeply
/// apart; together, the signal (the bass turned in phase as the other two are, so they still add up)
#[derive(Clone, Copy)]
struct Bands {
    low: Crossover,
    high: Crossover,
    /// (the bass through the 2.5 kHz crossover too: the same phase as the rest)
    low_phase: Crossover,
}

impl Bands {
    fn new(rate: u32) -> Self { Self { low: Crossover::new(rate, 150.0), high: Crossover::new(rate, 2500.0), low_phase: Crossover::new(rate, 2500.0) } }
    fn split(&mut self, ch: usize, x: f32) -> (f32, f32, f32) {
        let (l, rest) = self.low.split(ch, x);
        let (m, h) = self.high.split(ch, rest);
        let (a, b) = self.low_phase.split(ch, l);
        (a + b, m, h)
    }
}

/// The echo on the one going out: a stereo delay line, fed back
struct Echo {
    line: Vec<f32>,
    at: usize,
}

const ECHO_FEEDBACK: f32 = 0.35;
const ECHO_WET: f32 = 0.3;

impl Echo {
    /// What it gives back now, for this frame of what's sent to it (`delay` frames long)
    fn tick(&mut self, delay: u64, send: (f32, f32)) -> (f32, f32) {
        let n = delay as usize * 2;
        if self.line.len() != n { self.line = vec![0.0; n]; self.at = 0; }
        let (wl, wr) = (self.line[self.at], self.line[self.at + 1]);
        self.line[self.at] = send.0 + wl * ECHO_FEEDBACK;
        self.line[self.at + 1] = send.1 + wr * ECHO_FEEDBACK;
        self.at = (self.at + 2) % n;
        (wl, wr)
    }
}

impl Fading {
    /// (`handoff` in ms: it's kept in frames)
    fn new(next: Pipeline, total: u64, handoff: Option<Handoff>, rate: u32) -> Self {
        Self {
            next, total: total.max(1), done: 0, out_buf: VecDeque::new(), in_buf: VecDeque::new(), out_ended: false, in_ended: false,
            handoff: handoff.map(|h| h.frames(rate as u64)), bands: [Bands::new(rate); 2], echo: Echo { line: Vec::new(), at: 0 },
            sweeps: [[Butter::new(rate, 20.0, true); 2]; 2], rate, brake_at: 0.0, spin_at: 0.0, spin_lag: 0.0, loops: [Vec::new(), Vec::new()], centres: [Centre::new(rate); 2],
        }
    }
}

const XFADE_CHUNK: usize = 4096 * 2;

impl Fading {
    /// The next piece of both, mixed (equal power); true once the fade is over
    fn mix(&mut self, out: &mut Pipeline, into: &mut Vec<f32>, scratch: &mut Vec<f32>) -> Result<bool, String> {
        while !self.out_ended && self.out_buf.len() < XFADE_CHUNK {
            scratch.clear();
            if !out.fill(scratch)? { self.out_ended = true; }
            self.out_buf.extend(scratch.iter().copied());
        }
        while !self.in_ended && self.in_buf.len() < XFADE_CHUNK {
            scratch.clear();
            if !self.next.fill(scratch)? { self.in_ended = true; }
            self.in_buf.extend(scratch.iter().copied());
        }
        let frames = (self.in_buf.len() / 2).min(if self.out_ended { usize::MAX } else { self.out_buf.len() / 2 }).min(XFADE_CHUNK / 2);
        for _ in 0..frames {
            let p = (self.done as f32 / self.total.max(1) as f32).min(1.0);
            let (go, gi) = ((p * std::f32::consts::FRAC_PI_2).cos(), (p * std::f32::consts::FRAC_PI_2).sin());
            let braking = self.handoff.and_then(|h| h.brake).filter(|(from, _)| self.done >= *from);
            let out_loop = self.handoff.and_then(|h| h.out_loop).and_then(|l| l.at(self.done).map(|at| (l, at)));
            let roll = self.handoff.and_then(|h| h.roll);
            let rolled = roll.and_then(|r| r.at(self.done));
            let (ol, or) = match braking {
                _ if rolled.is_some() => {
                    let (k, len) = rolled.unwrap();
                    let g = Loop { from: 0, len, reps: 1 }.seam(k);
                    let i = k as usize * 2;
                    (self.loops[0].get(i).copied().unwrap_or(0.0) * g, self.loops[0].get(i + 1).copied().unwrap_or(0.0) * g)
                }
                // (its first bar: heard, and kept)
                _ if roll.is_some_and(|r| self.done < r.bar) => {
                    if self.done == 0 { self.loops[0].clear(); }
                    let v = (self.out_buf.pop_front().unwrap_or(0.0), self.out_buf.pop_front().unwrap_or(0.0));
                    self.loops[0].push(v.0);
                    self.loops[0].push(v.1);
                    v
                }
                // (rolled to its end: nothing more of it)
                _ if roll.is_some() => (0.0, 0.0),
                // looped: played again (none of it read meanwhile), or heard the first time and kept
                _ if out_loop.is_some_and(|(_, (pass, _))| pass > 0) => {
                    let (l, (_, k)) = out_loop.unwrap();
                    let g = l.seam(k);
                    let i = k as usize * 2;
                    (self.loops[0].get(i).copied().unwrap_or(0.0) * g, self.loops[0].get(i + 1).copied().unwrap_or(0.0) * g)
                }
                _ if out_loop.is_some() => {
                    let (l, (_, k)) = out_loop.unwrap();
                    if k == 0 { self.loops[0].clear(); }
                    let v = (self.out_buf.pop_front().unwrap_or(0.0), self.out_buf.pop_front().unwrap_or(0.0));
                    self.loops[0].push(v.0);
                    self.loops[0].push(v.1);
                    let g = if l.reps > 0 { l.seam(k).max(if k < 64 { 1.0 } else { 0.0 }) } else { 1.0 };
                    (v.0 * g, v.1 * g)
                }
                None => (self.out_buf.pop_front().unwrap_or(0.0), self.out_buf.pop_front().unwrap_or(0.0)),
                // stopped: nothing more of it
                Some((from, len)) if self.done >= from + len => (0.0, 0.0),
                // slowing: read at a falling speed (between its frames, in a line), as a turntable spinning down
                Some((from, len)) => {
                    let speed = 1.0 - (self.done - from) as f64 / len.max(1) as f64;
                    let i = self.brake_at.floor() as usize;
                    let w = (self.brake_at - i as f64) as f32;
                    let at = |k: usize, c: usize| self.out_buf.get(k * 2 + c).copied().unwrap_or(0.0);
                    let v = (at(i, 0) + (at(i + 1, 0) - at(i, 0)) * w, at(i, 1) + (at(i + 1, 1) - at(i, 1)) * w);
                    self.brake_at += speed;
                    let gone = (self.brake_at.floor() as usize).min(self.out_buf.len() / 2);
                    self.out_buf.drain(..gone * 2);
                    self.brake_at -= gone as f64;
                    v
                }
            };
            let spinning = self.handoff.and_then(|h| h.spin).filter(|(from, len)| self.done < *from + *len);
            let in_loop = self.handoff.and_then(|h| h.in_loop).and_then(|l| l.at(self.done).map(|at| (l, at)));
            let (il, ir) = match spinning {
                _ if in_loop.is_some_and(|(_, (pass, _))| pass > 0) => {
                    let (l, (_, k)) = in_loop.unwrap();
                    let g = l.seam(k);
                    let i = k as usize * 2;
                    (self.loops[1].get(i).copied().unwrap_or(0.0) * g, self.loops[1].get(i + 1).copied().unwrap_or(0.0) * g)
                }
                _ if in_loop.is_some() => {
                    let (l, (_, k)) = in_loop.unwrap();
                    if k == 0 { self.loops[1].clear(); }
                    let v = (self.in_buf.pop_front().unwrap_or(0.0), self.in_buf.pop_front().unwrap_or(0.0));
                    self.loops[1].push(v.0);
                    self.loops[1].push(v.1);
                    let g = if l.reps > 0 { l.seam(k).max(if k < 64 { 1.0 } else { 0.0 }) } else { 1.0 };
                    (v.0 * g, v.1 * g)
                }
                None => (self.in_buf.pop_front().unwrap_or(0.0), self.in_buf.pop_front().unwrap_or(0.0)),
                // not yet: nothing of it (and none of it read)
                Some((from, _)) if self.done < from => { self.spin_lag += 1.0; (0.0, 0.0) }
                // spinning up: read at a rising speed (between its frames, in a line), as a turntable starting
                Some((from, len)) => {
                    let speed = (self.done - from) as f64 / len.max(1) as f64;
                    let i = self.spin_at.floor() as usize;
                    let w = (self.spin_at - i as f64) as f32;
                    let at = |k: usize, c: usize| self.in_buf.get(k * 2 + c).copied().unwrap_or(0.0);
                    let v = (at(i, 0) + (at(i + 1, 0) - at(i, 0)) * w, at(i, 1) + (at(i + 1, 1) - at(i, 1)) * w);
                    self.spin_at += speed;
                    self.spin_lag += 1.0 - speed;
                    let gone = (self.spin_at.floor() as usize).min(self.in_buf.len() / 2);
                    self.in_buf.drain(..gone * 2);
                    self.spin_at -= gone as f64;
                    v
                }
            };
            match self.handoff {
                None => {
                    into.push(ol * go + il * gi);
                    into.push(or * go + ir * gi);
                }
                // a band at a time, each at its moment; the one going out's mids and highs echoed as it leaves
                Some(h) => {
                    let f = self.done;
                    // (the sweeps: their cutoffs moved every 32 frames, smooth enough to hear as one glide)
                    let (mut ol, mut or, mut il, mut ir) = (ol, or, il, ir);
                    // (the voices: the one going out's down, the one coming in's held out, then up)
                    let q = |at: u64, len: u64| if f < at { 0.0 } else { ((f - at) as f32 / len.max(1) as f32).min(1.0) };
                    if let Some((at, len)) = h.out_voice { (ol, or) = self.centres[0].run(ol, or, 1.0 - q(at, len)); }
                    if let Some((at, len)) = h.in_voice { (il, ir) = self.centres[1].run(il, ir, q(at, len)); }
                    for (side, w) in [h.out_sweep, h.in_sweep].into_iter().enumerate() {
                        let Some(w) = w else { continue };
                        if f % 32 == 0 { for st in &mut self.sweeps[side] { st.tune(self.rate, w.hz_at(f), w.high, std::f32::consts::FRAC_1_SQRT_2); } }
                        let (l, r) = if side == 0 { (&mut ol, &mut or) } else { (&mut il, &mut ir) };
                        let [s0, s1] = &mut self.sweeps[side];
                        *l = s1.run(0, s0.run(0, *l));
                        *r = s1.run(1, s0.run(1, *r));
                    }
                    let (lo, li) = h.low.gains(f);
                    let (mo, mi) = h.mid.gains(f);
                    let (ho, hi) = h.high.gains(f);
                    let mut side = [0.0f32; 2];
                    let mut send = [0.0f32; 2];
                    for (ch, (o, i)) in [(ol, il), (or, ir)].into_iter().enumerate() {
                        let (a_l, a_m, a_h) = self.bands[0].split(ch, o);
                        let (b_l, b_m, b_h) = self.bands[1].split(ch, i);
                        side[ch] = a_l * lo + a_m * mo + a_h * ho + b_l * li + b_m * mi + b_h * hi;
                        // (its voice echoed: its mids, not its hats)
                        send[ch] = a_m * mo;
                    }
                    if let Some((from, len, delay)) = h.echo {
                        let sending = if f >= from && f < from + len { 1.0 } else { 0.0 };
                        let (wl, wr) = self.echo.tick(delay, (send[0] * sending, send[1] * sending));
                        // (its tail dies away over the mix's last eighth: nothing of it cut off at the end)
                        let tail = self.total / 8;
                        let wet = ECHO_WET * if f + tail >= self.total { (self.total - f) as f32 / tail.max(1) as f32 } else { 1.0 };
                        side[0] += wl * wet;
                        side[1] += wr * wet;
                    }
                    into.push(side[0]);
                    into.push(side[1]);
                }
            }
            self.done += 1;
            if self.done >= self.total { break; }
        }
        // (the one coming in ran out before the fade did: it's over)
        Ok(self.done >= self.total || (self.in_ended && self.in_buf.is_empty()))
    }
}

/// What a mix waits for: the next source, its gain, the moment (ms of the one playing), its length and handover
struct Waiting {
    next: Pipeline,
    at_ms: u64,
    ms: u64,
    handoff: Option<Handoff>,
}

/// Starts a crossfade (or a mix) into `next`: from here, it's the one the position, length and end are of
fn begin_fade(shared: &Shared, next: Pipeline, total_frames: u64, handoff: Option<Handoff>) -> Fading {
    let rate = shared.rate.load(Ordering::SeqCst);
    let queued = shared.queue.lock().unwrap().len() as u64 / 2;
    shared.played.store(0, Ordering::SeqCst);
    shared.skew.store(queued, Ordering::SeqCst);
    shared.produced.store(0, Ordering::SeqCst);
    shared.stretch_off.store(0, Ordering::SeqCst);
    // (where it starts from: its own start, or where it was put)
    shared.base_ms.store(next.src_position_ms().unwrap_or(0), Ordering::SeqCst);
    shared.duration_ms.store(next.duration_ms().unwrap_or(0), Ordering::SeqCst);
    shared.source_done.store(false, Ordering::SeqCst);
    shared.fading_in.store(true, Ordering::SeqCst);
    Fading::new(next, total_frames, handoff, rate)
}

fn decode_loop(shared: Arc<Shared>, cmds: Receiver<Cmd>, ev: Sender<Event>) {
    // (above the app's other work, and a game's: the sound is only 0.35 s ahead of what's heard)
    #[cfg(windows)]
    unsafe { windows_sys::Win32::System::Threading::SetThreadPriority(windows_sys::Win32::System::Threading::GetCurrentThread(), windows_sys::Win32::System::Threading::THREAD_PRIORITY_HIGHEST); }
    let mut pipe: Option<Pipeline> = None;
    let mut fading: Option<Fading> = None;
    let mut waiting: Option<Waiting> = None;
    let mut eq = vec![0.0f32; 10];
    let mut ended_sent = true;
    let mut scratch = Vec::with_capacity(8192);
    let mut mixed: Vec<f32> = Vec::with_capacity(XFADE_CHUNK);
    loop {
        // commands first; wait for one when there's nothing to do
        let idle = pipe.is_none() || shared.source_done.load(Ordering::SeqCst);
        let got = if idle { cmds.recv_timeout(Duration::from_millis(50)).ok() } else { cmds.try_recv().ok() };
        if let Some(c) = got {
            match c {
                Cmd::Crossfade(src, db, ms, handoff) if pipe.is_some() && !shared.source_done.load(Ordering::SeqCst) => {
                    let rate = shared.rate.load(Ordering::SeqCst);
                    // (one fading in already: it's the one playing now)
                    if let Some(f) = fading.take() { pipe = Some(f.next); }
                    let mut next = Pipeline::new(src, rate);
                    next.chain.eq.set(&eq);
                    next.chain.set_match_db(db, true);
                    // its position counts from here: what's queued still is the one going out's
                    waiting = None;
                    shared.mix_waiting.store(false, Ordering::SeqCst);
                    fading = Some(begin_fade(&shared, next, ms * rate as u64 / 1000, handoff));
                    ended_sent = false;
                }
                // a mix: waits for its moment in the one playing
                Cmd::MixAt { src, db, at_ms, ms, handoff } if pipe.is_some() && !shared.source_done.load(Ordering::SeqCst) => {
                    let rate = shared.rate.load(Ordering::SeqCst);
                    if let Some(f) = fading.take() { pipe = Some(f.next); shared.fading_in.store(false, Ordering::SeqCst); }
                    let mut next = Pipeline::new(src, rate);
                    next.chain.eq.set(&eq);
                    next.chain.set_match_db(db, true);
                    waiting = Some(Waiting { next, at_ms, ms, handoff });
                    shared.mix_waiting.store(true, Ordering::SeqCst);
                }
                Cmd::MixAt { src, db, .. } => {
                    // (nothing playing to mix from: it just starts, as a load)
                    let _ = cmds_self_load(&shared, &mut pipe, &mut fading, src, db, &eq);
                    ended_sent = false;
                }
                // (nothing playing to fade from: it just starts)
                Cmd::Crossfade(src, db, _, _) | Cmd::Load(src, db) => {
                    fading = None;
                    waiting = None;
                    shared.mix_waiting.store(false, Ordering::SeqCst);
                    shared.fading_in.store(false, Ordering::SeqCst);
                    shared.skew.store(0, Ordering::SeqCst);
                    shared.produced.store(0, Ordering::SeqCst);
                    shared.stretch_off.store(0, Ordering::SeqCst);
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
                    // (mid-crossfade: the one coming in is the one playing; a mix waiting is off)
                    if let Some(f) = fading.take() { pipe = Some(f.next); shared.fading_in.store(false, Ordering::SeqCst); }
                    waiting = None;
                    shared.mix_waiting.store(false, Ordering::SeqCst);
                    shared.skew.store(0, Ordering::SeqCst);
                    shared.produced.store(0, Ordering::SeqCst);
                    shared.stretch_off.store(0, Ordering::SeqCst);
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
                    fading = None;
                    waiting = None;
                    shared.mix_waiting.store(false, Ordering::SeqCst);
                    shared.fading_in.store(false, Ordering::SeqCst);
                    shared.skew.store(0, Ordering::SeqCst);
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
                    if let Some(f) = fading.as_mut() {
                        f.next.chain.eq.set(&eq);
                    }
                    if let Some(w) = waiting.as_mut() {
                        w.next.chain.eq.set(&eq);
                    }
                }
                Cmd::StretchPlaying(tempo) => {
                    // (the one playing: going out of a mix, or not yet in one; not one coming in)
                    if let Some(p) = pipe.as_mut() {
                        // (it couldn't be: it just goes on as it is, the mix unslowed; not an error to stop it for)
                        if let Err(e) = p.stretch(tempo) { eprintln!("stretch: {e}"); }
                    }
                }
                Cmd::Match(db, at_once) => {
                    // (a measurement for the one playing: the one coming in, mid-crossfade)
                    match fading.as_mut() {
                        Some(f) => f.next.chain.set_match_db(db, at_once),
                        None => if let Some(p) = pipe.as_mut() { p.chain.set_match_db(db, at_once); },
                    }
                }
                Cmd::Rate(rate) => {
                    // a device at another rate: rebuild the conversion and chain where playback was
                    // (mid-crossfade: the one coming in, where it is)
                    if let Some(f) = fading.take() { pipe = Some(f.next); shared.fading_in.store(false, Ordering::SeqCst); shared.skew.store(0, Ordering::SeqCst); }
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
        // a mix waiting: when the one playing reaches its moment (within this piece: split exactly there)
        if fading.is_none() && waiting.is_some() {
            let rate_u = shared.rate.load(Ordering::Relaxed) as u64;
            let now_ms = shared.base_ms.load(Ordering::Relaxed) as i64 + (shared.produced.load(Ordering::Relaxed) * 1000 / rate_u.max(1)) as i64 + shared.stretch_off.load(Ordering::Relaxed);
            let at = waiting.as_ref().unwrap().at_ms as i64;
            scratch.clear();
            let t0 = std::time::Instant::now();
            let more = match p.fill(&mut scratch) { Ok(m) => m, Err(e) => { let _ = ev.send(Event::Error(e)); false } };
            perf_add(&shared, if p.src_position_ms().is_some() { 1 } else { 0 }, t0, scratch.len() as u64 / 2);
            let frames = scratch.len() as u64 / 2;
            let until = ((at - now_ms).max(0) as u64 * rate_u / 1000).min(frames);
            shared.queue.lock().unwrap().extend(scratch[..(until * 2) as usize].iter().copied());
            shared.produced.fetch_add(until, Ordering::Relaxed);
            if until < frames || !more {
                // its moment: the rest of this piece is the first of the one going out, in the mix
                let w = waiting.take().unwrap();
                shared.mix_waiting.store(false, Ordering::SeqCst);
                let total = w.ms * rate_u / 1000;
                let mut f = begin_fade(&shared, w.next, total, w.handoff);
                f.out_buf.extend(scratch[(until * 2) as usize..].iter().copied());
                f.out_ended = !more;
                fading = Some(f);
                let _ = ev.send(Event::Mixed);
            }
            continue;
        }
        // a crossfade: both, mixed, until the one coming in is the one playing
        if let Some(f) = fading.as_mut() {
            mixed.clear();
            let t0 = std::time::Instant::now();
            let r = f.mix(p, &mut mixed, &mut scratch);
            perf_add(&shared, 2, t0, mixed.len() as u64 / 2);
            match r {
                Ok(over) => {
                    *shared.level.lock().unwrap() = f.next.level;
                    shared.queue.lock().unwrap().extend(mixed.iter().copied());
                    shared.produced.fetch_add(mixed.len() as u64 / 2, Ordering::Relaxed);
                    stretch_note(&shared, &f.next);
                    if over {
                        let f = fading.take().unwrap();
                        // (a tape start: it read less than it played; where it is, put right)
                        if f.spin_lag >= 1.0 {
                            let lag_ms = (f.spin_lag * 1000.0 / shared.rate.load(Ordering::SeqCst).max(1) as f64).round() as u64;
                            let b = shared.base_ms.load(Ordering::SeqCst);
                            shared.base_ms.store(b.saturating_sub(lag_ms), Ordering::SeqCst);
                        }
                        // what of it was read ahead goes on as it is
                        shared.queue.lock().unwrap().extend(f.in_buf.iter().copied());
                        shared.produced.fetch_add(f.in_buf.len() as u64 / 2, Ordering::Relaxed);
                        if f.in_ended { shared.source_done.store(true, Ordering::SeqCst); }
                        pipe = Some(f.next);
                        shared.fading_in.store(false, Ordering::SeqCst);
                    }
                }
                Err(e) => {
                    // the one coming in failed: it's the one playing all the same, and it says so
                    let f = fading.take().unwrap();
                    pipe = Some(f.next);
                    shared.fading_in.store(false, Ordering::SeqCst);
                    shared.source_done.store(true, Ordering::SeqCst);
                    let _ = ev.send(Event::Error(e));
                }
            }
            continue;
        }
        scratch.clear();
        let t0 = std::time::Instant::now();
        let r = p.fill(&mut scratch);
        perf_add(&shared, if p.src_position_ms().is_some() { 1 } else { 0 }, t0, scratch.len() as u64 / 2);
        match r {
            Ok(more) => {
                *shared.level.lock().unwrap() = p.level;
                shared.queue.lock().unwrap().extend(scratch.iter().copied());
                shared.produced.fetch_add(scratch.len() as u64 / 2, Ordering::Relaxed);
                stretch_note(&shared, p);
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

/// A stretched source: where it is in its own time, against where the frames given say (ms)
fn stretch_note(shared: &Shared, p: &Pipeline) {
    if let Some(src_ms) = p.src_position_ms() {
        let rate = shared.rate.load(Ordering::Relaxed).max(1) as u64;
        let given = shared.base_ms.load(Ordering::Relaxed) + shared.produced.load(Ordering::Relaxed) * 1000 / rate;
        shared.stretch_off.store(src_ms as i64 - given as i64, Ordering::Relaxed);
    }
}

/// A load (as Cmd::Load), for a mix with nothing playing to mix from
fn cmds_self_load(shared: &Shared, pipe: &mut Option<Pipeline>, fading: &mut Option<Fading>, src: Box<dyn Source>, db: f32, eq: &[f32]) -> bool {
    *fading = None;
    shared.fading_in.store(false, Ordering::SeqCst);
    shared.mix_waiting.store(false, Ordering::SeqCst);
    let rate = shared.rate.load(Ordering::SeqCst);
    let mut p = Pipeline::new(src, rate);
    p.chain.eq.set(eq);
    p.chain.set_match_db(db, true);
    shared.queue.lock().unwrap().clear();
    shared.played.store(0, Ordering::SeqCst);
    shared.skew.store(0, Ordering::SeqCst);
    shared.produced.store(0, Ordering::SeqCst);
    shared.stretch_off.store(0, Ordering::SeqCst);
    shared.base_ms.store(p.src_position_ms().unwrap_or(0), Ordering::SeqCst);
    shared.duration_ms.store(p.duration_ms().unwrap_or(0), Ordering::SeqCst);
    shared.source_done.store(false, Ordering::SeqCst);
    shared.loaded.store(true, Ordering::SeqCst);
    *pipe = Some(p);
    true
}

/// Renders a crossfade without a device: `a` from `a_from_ms`, then `b` coming in over `ms` (for tests)
pub fn render_crossfade(a: Box<dyn Source>, b: Box<dyn Source>, out_rate: u32, a_secs: f32, ms: u64, max_secs: f32) -> Result<Vec<f32>, String> {
    render_mix(a, b, (0.0, 0.0), out_rate, a_secs, ms, None, max_secs)
}

/// A mix rendered without a device: as render_crossfade, handed over as `handoff` says, each one
/// at its own loudness match (`dbs`: the one going out's, the one coming in's), as the player has it
pub fn render_mix(a: Box<dyn Source>, b: Box<dyn Source>, dbs: (f32, f32), out_rate: u32, a_secs: f32, ms: u64, handoff: Option<Handoff>, max_secs: f32) -> Result<Vec<f32>, String> {
    render_mix_ramped(a, b, dbs, out_rate, a_secs, ms, handoff, None, max_secs)
}

/// The same, the one going out stretched from the mix's start by `ramp`'s tempo handle, and its
/// function told how far in the mix is (0..1) as it goes, to move the tempos (as the app does)
#[allow(clippy::too_many_arguments)]
pub fn render_mix_ramped(a: Box<dyn Source>, b: Box<dyn Source>, dbs: (f32, f32), out_rate: u32, a_secs: f32, ms: u64, handoff: Option<Handoff>, ramp: Option<(&crate::stretch::Tempo, &dyn Fn(f64))>, max_secs: f32) -> Result<Vec<f32>, String> {
    let mut pa = Pipeline::new(a, out_rate);
    pa.chain.set_match_db(dbs.0, true);
    let mut out = Vec::new();
    let lead = (a_secs * out_rate as f32) as usize * 2;
    while out.len() < lead {
        if !pa.fill(&mut out)? { break; }
    }
    // (what was read past the moment is the one going out's first in the mix, as the player has it)
    let past = out.split_off(lead.min(out.len()));
    let mut pb = Pipeline::new(b, out_rate);
    pb.chain.set_match_db(dbs.1, true);
    let mut f = Fading::new(pb, ms * out_rate as u64 / 1000, handoff, out_rate);
    f.out_buf.extend(past);
    if let Some((tempo, _)) = ramp { pa.stretch(tempo.clone())?; }
    let mut scratch = Vec::new();
    loop {
        if let Some((_, r)) = ramp { r(f.done as f64 / f.total as f64); }
        if f.mix(&mut pa, &mut out, &mut scratch)? { break; }
    }
    out.extend(f.in_buf.iter().copied());
    let limit = (max_secs * out_rate as f32) as usize * 2;
    while out.len() < limit {
        if !f.next.fill(&mut out)? { break; }
    }
    out.truncate(limit);
    Ok(out)
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

#[cfg(test)]
mod xfade_tests {
    use super::*;
    use crate::source::Tone;

    fn rms(x: &[f32]) -> f32 {
        (x.iter().map(|v| v * v).sum::<f32>() / x.len().max(1) as f32).sqrt()
    }

    #[test]
    fn a_crossfade_keeps_the_level_and_hands_over() {
        let rate = 44_100u32;
        let a = Box::new(Tone::new(rate, 440.0, 0.3, 10.0));
        let b = Box::new(Tone::new(rate, 660.0, 0.3, 10.0));
        let out = render_crossfade(a, b, rate, 2.0, 2000, 6.0).unwrap();
        assert_eq!(out.len(), 6 * rate as usize * 2);
        let at = |s: f32| { let i = (s * rate as f32) as usize * 2; rms(&out[i..i + 8820]) };
        let (before, mid, after) = (at(1.0), at(2.95), at(5.0));
        let db = |x: f32| 20.0 * x.log10();
        // equal power: as loud all the way through (two different tones add as power)
        assert!((db(mid) - db(before)).abs() < 1.0, "{before} {mid}");
        assert!((db(after) - db(before)).abs() < 1.0, "{before} {after}");
        // and after it, only the one that came in: 660 Hz, so about 66 upward zero crossings in 0.1 s
        let i = (5.0 * rate as f32) as usize * 2;
        let crossings = out[i..i + 2 * 4410].chunks(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((60..=72).contains(&crossings), "{crossings}");
    }
}

#[cfg(test)]
mod takeover_tests {
    use super::*;
    use crate::source::Tone;

    #[test]
    fn a_stretch_takes_over_unheard_then_slows() {
        let rate = 44_100u32;
        let mut p = Pipeline::new(Box::new(Tone::new(rate, 440.0, 0.3, 8.0)), rate);
        let mut out = Vec::new();
        while out.len() < rate as usize * 2 * 2 { p.fill(&mut out).unwrap(); }
        let at = out.len();
        let tempo = crate::stretch::Tempo::new(1.0);
        p.stretch(tempo.clone()).unwrap();
        while out.len() < at + rate as usize * 2 { p.fill(&mut out).unwrap(); }
        // no click where it took over: no step between samples bigger than a 440 Hz sine's own (with room)
        let max_step = 0.3 * 2.0 * std::f32::consts::PI * 440.0 / rate as f32;
        let worst = out[at - 2000..at + 4000].chunks(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).map(|w| (w[1] - w[0]).abs()).fold(0.0f32, f32::max);
        assert!(worst < max_step * 1.6, "a step of {worst} (a sine's: {max_step})");
        // and as loud
        let rms = |x: &[f32]| (x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt();
        assert!((rms(&out[at + 2000..at + 20000]) / rms(&out[at - 20000..at - 2000]) - 1.0).abs() < 0.1);
        // where it is: where it took over, give or take
        assert!(p.src_position_ms().is_some_and(|ms| (2900..3100).contains(&ms)), "{:?}", p.src_position_ms());
        // slowed to 0.8: the rest (5 s of it) takes 6.25 s
        tempo.set(0.8);
        let from = out.len();
        while p.fill(&mut out).unwrap() {}
        let secs = (out.len() - from) as f64 / 2.0 / rate as f64;
        assert!((secs - 5.0 / 0.8).abs() < 0.3, "{secs} s");
    }
}

#[cfg(test)]
mod mix_tests {
    use super::*;
    use crate::source::Tone;

    /// How many times a channel crosses zero upward in a stretch (a tone's frequency, near enough)
    fn ups(out: &[f32], rate: u32, from_s: f32, secs: f32) -> usize {
        let a = (from_s * rate as f32) as usize * 2;
        let b = a + (secs * rate as f32) as usize * 2;
        let ch: Vec<f32> = out[a..b].chunks(2).map(|f| f[0]).collect();
        ch.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
    }

    #[test]
    fn the_bass_is_handed_over_at_once() {
        let rate = 44_100u32;
        // two basslines: 55 Hz going out, 82 Hz coming in; a 2 s mix from 2 s, the bass swapped 1.2 s in
        let a = Box::new(Tone::new(rate, 55.0, 0.4, 10.0));
        let b = Box::new(Tone::new(rate, 82.0, 0.4, 10.0));
        let out = render_mix(a, b, (0.0, 0.0), rate, 2.0, 2000, Some(Handoff::bass_swap(2000, 1200)), 6.0).unwrap();
        // before the swap: only the one going out's (its 55 Hz: ~11 in 0.2 s), the other's held back
        let before = ups(&out, rate, 2.9, 0.2);
        assert!((10..=12).contains(&before), "before {before}");
        // after it (and its quarter second): only the one coming in's (82 Hz: ~16 in 0.2 s)
        let after = ups(&out, rate, 3.6, 0.2);
        assert!((15..=18).contains(&after), "after {after}");
    }

    #[test]
    fn a_tape_stop_slows_it_to_a_halt_then_the_next_one_goes() {
        let rate = 44_100u32;
        let a = Box::new(Tone::new(rate, 440.0, 0.3, 10.0));
        let b = Box::new(Tone::new(rate, 660.0, 0.3, 10.0));
        // from 2 s: a second's stop, then the next one
        let out = render_mix(a, b, (0.0, 0.0), rate, 2.0, 1300, Some(Handoff::tape_stop(1000)), 5.0).unwrap();
        // upward zero crossings in a tenth of a second (a tone's frequency, near enough)
        let ups = |at: f32| { let i = (at * rate as f32) as usize * 2; out[i..i + 2 * 4410].chunks(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() };
        // before it: 440 Hz (44); a quarter in: three quarters the speed (33); three quarters in: a quarter (11)
        assert!((42..=46).contains(&ups(1.8)), "before {}", ups(1.8));
        assert!((30..=36).contains(&ups(2.2)), "slowing {}", ups(2.2));
        assert!((8..=14).contains(&ups(2.7)), "nearly stopped {}", ups(2.7));
        // after it: only the next one (660 Hz: 66)
        assert!((63..=69).contains(&ups(3.5)), "after {}", ups(3.5));
        // and no click as it stops: nothing louder than the tones themselves
        let peak = out[(2.9 * rate as f32) as usize * 2..(3.1 * rate as f32) as usize * 2].iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(peak <= 0.36, "{peak}");
    }

    #[test]
    fn a_tape_start_spins_the_next_one_up_as_the_other_stops() {
        let rate = 44_100u32;
        let a = Box::new(Tone::new(rate, 440.0, 0.3, 10.0));
        let b = Box::new(Tone::new(rate, 660.0, 0.3, 10.0));
        // from 2 s: a two-second stop, the next one spinning up over its second second (no gap)
        let out = render_mix(a, b, (0.0, 0.0), rate, 2.0, 2300, Some(Handoff::tape(2000, 1000)), 6.0).unwrap();
        // (the next one alone, past the stop: its zero crossings in a tenth of a second)
        let ups = |at: f32| { let i = (at * rate as f32) as usize * 2; out[i..i + 2 * 4410].chunks(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count() };
        // the one going out slowing, alone, before the next one starts (a quarter in: 3/4 of 440 Hz)
        assert!((30..=36).contains(&ups(2.45)), "slowing {}", ups(2.45));
        // the next one at its speed as the other stops, and after (660 Hz: 66)
        assert!((63..=69).contains(&ups(4.05)), "at speed {}", ups(4.05));
        assert!((63..=69).contains(&ups(4.6)), "after {}", ups(4.6));
        // under the stop's end, both heard: something there, never silent between them
        let quiet = (31..39).map(|t| t as f32 / 10.0).map(|t| { let i = (t * rate as f32) as usize * 2; out[i..i + 882].iter().fold(0.0f32, |m, v| m.max(v.abs())) }).fold(1.0f32, f32::min);
        assert!(quiet > 0.05, "a gap: {quiet}");
    }

    #[test]
    fn a_word_looped_plays_the_same_beat_again_then_the_next_one_comes_in() {
        let rate = 44_100u32;
        let a = Box::new(Tone::new(rate, 440.0, 0.3, 10.0));
        let b = Box::new(Tone::new(rate, 660.0, 0.3, 10.0));
        // from 2 s: a 257 ms beat (not a whole number of the tone's cycles) heard, then 3 more times; the
        // next one in on the beat after (2 + 4 x 0.257 s)
        let beat = 257u64;
        let out = render_mix(a, b, (0.0, 0.0), rate, 2.0, 4 * beat + 400, Some(Handoff::word_cut(4 * beat, beat, 3)), 4.0).unwrap();
        // (in frames: the loop is a whole number of them)
        let (from, len) = (2 * rate as usize, beat as usize * rate as usize / 1000);
        let at = |f: usize| out[f * 2];
        // the same moment of the beat, each time round (the tone going on would be at another point by then)
        for pass in 1..4 {
            for k in [2205usize, 4410, 8820] {
                let (first, again) = (at(from + k), at(from + pass * len + k));
                assert!((first - again).abs() < 0.02, "pass {pass} at {k}: {first} vs {again}");
            }
        }
        // after it: the next one alone (660 Hz: 66 zero crossings in a tenth of a second)
        let i = ((2.0 + 4.0 * beat as f32 / 1000.0 + 0.1) * rate as f32) as usize * 2;
        let ups = out[i..i + 2 * 4410].chunks(2).map(|f| f[0]).collect::<Vec<_>>().windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((63..=69).contains(&ups), "after {ups}");
    }

    #[test]
    fn a_wipe_never_has_both_in_one_band() {
        let rate = 44_100u32;
        // the one going out: 55 Hz, 600 Hz, 6 kHz; the one coming in: 82 Hz, 900 Hz, 9 kHz
        let tri = |a: f32, b: f32, c: f32| -> Box<dyn Source> { Box::new(crate::source::Sum(vec![Tone::new(rate, a, 0.3, 12.0), Tone::new(rate, b, 0.2, 12.0), Tone::new(rate, c, 0.1, 12.0)])) };
        // 8 s of mix from 2 s, beats half a second, the swap 4 s in (6 s)
        let out = render_mix(tri(55.0, 600.0, 6000.0), tri(82.0, 900.0, 9000.0), (0.0, 0.0), rate, 2.0, 8000, Some(Handoff::wipe(500.0, 4000)), 11.0).unwrap();
        let has = |hz: f32, at: f32| {
            let n = (rate as f32 * 0.3) as usize;
            let i = (at * rate as f32) as usize - n / 2;
            let (mut c, mut s) = (0.0f32, 0.0f32);
            for k in 0..n {
                let x = out[(i + k) * 2] * (1.0 - (2.0 * std::f32::consts::PI * k as f32 / n as f32).cos()) * 0.5;
                let w = 2.0 * std::f32::consts::PI * hz * k as f32 / rate as f32;
                c += x * w.cos();
                s += x * w.sin();
            }
            4.0 * (c * c + s * s).sqrt() / n as f32
        };
        // a second in (the split near 13 kHz): the one going out whole but its very top; the one coming
        // in's voice and bass not at all
        assert!(has(600.0, 3.0) > 0.18 && has(55.0, 3.0) > 0.25, "early out {} {}", has(600.0, 3.0), has(55.0, 3.0));
        assert!(has(900.0, 3.0) < 0.01 && has(82.0, 3.0) < 0.01, "early held {} {}", has(900.0, 3.0), has(82.0, 3.0));
        // three seconds in, a beat before the swap (the split near 6.3 kHz): the one coming in's top,
        // the one going out's voice all there and still bright (its 6 kHz half there), the one coming in's not yet
        assert!(has(9000.0, 5.0) > 0.06 && has(600.0, 5.0) > 0.18 && has(6000.0, 5.0) > 0.04, "kept {} {} {}", has(9000.0, 5.0), has(600.0, 5.0), has(6000.0, 5.0));
        assert!(has(900.0, 5.0) < 0.01, "voice held {}", has(900.0, 5.0));
        // past the swap: only the one that came in, all of it
        assert!(has(82.0, 7.0) > 0.25 && has(55.0, 7.0) < 0.01 && has(600.0, 7.0) < 0.01, "after {} {} {}", has(82.0, 7.0), has(55.0, 7.0), has(600.0, 7.0));
        // and as loud all through (within 3 dB)
        let rms = |at: f32| { let i = (at * rate as f32) as usize * 2; let x = &out[i..i + 8820]; 20.0 * ((x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()).log10() };
        let before = rms(1.0);
        for t in [2.5f32, 3.0, 3.5, 4.0, 4.5, 5.0, 5.5, 6.5, 7.5, 9.0] { assert!((rms(t) - before).abs() < 3.0, "level at {t}: {} against {before}", rms(t)); }
    }

    #[test]
    fn staged_a_band_at_a_time() {
        let rate = 44_100u32;
        // the one going out: a bass at 55 Hz, a mid at 600, a high at 6 kHz; the one coming in: 82, 900, 9 kHz
        let tri = |a: f32, b: f32, c: f32| -> Box<dyn Source> { Box::new(crate::source::Sum(vec![Tone::new(rate, a, 0.3, 12.0), Tone::new(rate, b, 0.2, 12.0), Tone::new(rate, c, 0.1, 12.0)])) };
        // 8 s of mix from 2 s, beats half a second, swapped 4 s in
        let h = Handoff::staged(8000, 500.0, 4000);
        let out = render_mix(tri(55.0, 600.0, 6000.0), tri(82.0, 900.0, 9000.0), (0.0, 0.0), rate, 2.0, 8000, Some(h), 11.0).unwrap();
        // how much of a frequency there is around a time (its correlation with a sine and a cosine, over
        // a Hann window of 0.3 s centred there: the tones near it kept out)
        let has = |hz: f32, at: f32| {
            let n = (rate as f32 * 0.3) as usize;
            let i = (at * rate as f32) as usize - n / 2;
            let (mut c, mut s) = (0.0f32, 0.0f32);
            for k in 0..n {
                let x = out[(i + k) * 2] * (1.0 - (2.0 * std::f32::consts::PI * k as f32 / n as f32).cos()) * 0.5;
                let w = 2.0 * std::f32::consts::PI * hz * k as f32 / rate as f32;
                c += x * w.cos();
                s += x * w.sin();
            }
            4.0 * (c * c + s * s).sqrt() / n as f32
        };
        // early on (the highs cross over all through the mix: its beats are long): the one going out's highs mostly
        assert!(has(6000.0, 2.5) > 0.08 && has(9000.0, 2.5) < 0.04, "highs early {} {}", has(6000.0, 2.5), has(9000.0, 2.5));
        assert!(has(900.0, 3.0) < 0.02 && has(82.0, 3.0) < 0.02, "held back {} {}", has(900.0, 3.0), has(82.0, 3.0));
        // past it: the one coming in's highs only
        assert!(has(9000.0, 9.8) > 0.08 && has(6000.0, 9.8) < 0.01, "highs late {} {}", has(9000.0, 9.8), has(6000.0, 9.8));
        // and never louder or quieter through it than either alone (within 1.5 dB)
        let rms = |at: f32| { let i = (at * rate as f32) as usize * 2; let x = &out[i..i + 8820]; 20.0 * ((x.iter().map(|v| v * v).sum::<f32>() / x.len() as f32).sqrt()).log10() };
        let before = rms(1.0);
        for t in [2.5f32, 3.5, 4.5, 5.5, 6.5, 7.5, 8.5, 9.5, 10.5] { assert!((rms(t) - before).abs() < 1.5, "level at {t}: {} against {before}", rms(t)); }
        // before the swap (6 s: 2 + 4): the one going out's mids and bass
        assert!(has(600.0, 5.0) > 0.15 && has(55.0, 5.0) > 0.2 && has(82.0, 5.0) < 0.03, "before {} {} {}", has(600.0, 5.0), has(55.0, 5.0), has(82.0, 5.0));
        // after it: the one coming in's (the other's mids gone, but for their echo)
        assert!(has(900.0, 7.0) > 0.15, "mids in {}", has(900.0, 7.0));
        assert!(has(82.0, 6.5) > 0.2 && has(55.0, 6.5) < 0.03, "bass after {} {}", has(82.0, 6.5), has(55.0, 6.5));
        // after it: only the one that came in
        assert!(has(6000.0, 10.5) < 0.01 && has(600.0, 10.5) < 0.01, "left {} {}", has(6000.0, 10.5), has(600.0, 10.5));
    }
}

#[cfg(test)]
mod centre_tests {
    use super::Centre;

    /// The level (RMS) of what comes out of `g`'s centre cut, for a centred tone at `hz` (or one panned
    /// hard left), its first half second left out (the filters settling)
    fn through(hz: f32, panned: bool, g: f32) -> f32 {
        let rate = 44_100;
        let mut c = Centre::new(rate);
        let (mut sum, mut n) = (0.0f32, 0);
        for i in 0..rate as usize * 2 {
            let x = (2.0 * std::f32::consts::PI * hz * i as f32 / rate as f32).sin() * 0.5;
            let (l, r) = c.run(x, if panned { 0.0 } else { x }, g);
            if i > rate as usize / 2 { sum += l * l + r * r; n += 2; }
        }
        (sum / n as f32).sqrt()
    }

    #[test]
    fn the_centres_voice_goes_its_bass_and_whats_panned_stay() {
        let full = through(1000.0, false, 1.0);
        assert!((full - 0.3536).abs() < 0.01, "as it is at full: {full}");
        assert!(through(1000.0, false, 0.0) < full * 0.1, "a centred voice gone");
        assert!(through(60.0, false, 0.0) > full * 0.9, "its bass kept");
        // (panned hard: half of it is in both channels alike, so about 70% of it stays: the centre trick's
        // way, as the old karaoke switches)
        let panned = through(1000.0, true, 0.0) / through(1000.0, true, 1.0);
        assert!((0.65..0.75).contains(&panned), "what's panned mostly kept: {panned}");
    }
}
