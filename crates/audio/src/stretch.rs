// A source at another tempo, its pitch kept (timestretch's keylock deck: pure Rust), for seamless
// mixing: the track coming in plays at the tempo of the one going out, then eases back to its own.
// The tempo is a shared handle the app moves while it plays (1.0: its own; 1.02: 2% faster).
// A key shift (whole semitones) is a pitch factor on top, one the app can move while it plays (a shared
// handle, as the tempo): the deck's output resampled by it (higher and faster), and the deck's tempo
// making up for the speed. So a mix's key shift can bend back to the track's own key once the mix is
// over, rather than staying for all of it.

use crate::source::Source;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use timestretch::engine::{Engine, EngineConfig, EngineController, EngineProcessor, EngineProfile, SourceProducer};

const BLOCK: usize = 1024;

/// A tempo the app sets and the source follows (1.0 is the track's own)
#[derive(Clone, Debug)]
pub struct Tempo(Arc<AtomicU64>);

impl Tempo {
    pub fn new(rate: f64) -> Self {
        Self(Arc::new(AtomicU64::new(rate.to_bits())))
    }
    pub fn set(&self, rate: f64) {
        self.0.store(rate.clamp(0.5, 2.0).to_bits(), Ordering::Relaxed);
    }
    pub fn get(&self) -> f64 {
        f64::from_bits(self.0.load(Ordering::Relaxed))
    }
}

/// The resampling for a key shift: its pitch able to move as it plays (the deck's output in, a block
/// at a time)
struct Bend {
    r: rubato::SincFixedIn<f32>,
    /// the deck's output not resampled yet (each channel)
    held: [Vec<f32>; 2],
    /// the pitch it's at
    applied: f64,
    /// its own small delay, still to be let go (so it's on time, as the deck's)
    skip: usize,
}

impl Bend {
    fn new(pitch: f64) -> Result<Self, String> {
        use rubato::{Resampler as _, SincInterpolationParameters, SincInterpolationType, WindowFunction};
        let params = SincInterpolationParameters { sinc_len: 64, f_cutoff: 0.95, interpolation: SincInterpolationType::Linear, oversampling_factor: 128, window: WindowFunction::BlackmanHarris2 };
        // (out per in: fewer frames, higher; within a third either way of where it starts: two
        // semitones and back is an eighth)
        let r = rubato::SincFixedIn::<f32>::new(1.0 / pitch, 1.33, params, 512, 2).map_err(|e| format!("the key shift wouldn't start: {e}"))?;
        let skip = r.output_delay();
        Ok(Self { r, held: [Vec::new(), Vec::new()], applied: pitch, skip })
    }

    /// Frames of the deck's output (interleaved) in; out, those resampled at `pitch`
    fn run(&mut self, pitch: f64, frames: &[f32], out: &mut Vec<f32>) {
        use rubato::Resampler as _;
        for f in frames.chunks_exact(2) { self.held[0].push(f[0]); self.held[1].push(f[1]); }
        if (pitch - self.applied).abs() > 1e-7 && self.r.set_resample_ratio(1.0 / pitch, true).is_ok() { self.applied = pitch; }
        while self.held[0].len() >= self.r.input_frames_next() {
            let n = self.r.input_frames_next();
            let Ok(res) = self.r.process(&[&self.held[0][..n], &self.held[1][..n]], None) else { break };
            self.held[0].drain(..n);
            self.held[1].drain(..n);
            let drop = self.skip.min(res[0].len());
            self.skip -= drop;
            for i in drop..res[0].len() { out.push(res[0][i]); out.push(res[1][i]); }
        }
    }

    fn reset(&mut self) {
        use rubato::Resampler as _;
        self.r.reset();
        // (rubato's reset puts the ratio back to the one it was made with, a mix's key shift: the pitch
        // it's at kept, or a seek after the bend back would play it shifted, and fast, again)
        let _ = self.r.set_resample_ratio(1.0 / self.applied, false);
        self.held = [Vec::new(), Vec::new()];
        self.skip = self.r.output_delay();
    }
}

struct Deck {
    ctl: EngineController,
    proc: EngineProcessor,
    feed: SourceProducer,
}

fn deck(rate: u32, tempo: f64) -> Result<Deck, String> {
    let h = Engine::build(EngineConfig { sample_rate: rate, channels: 2, profile: EngineProfile::Keylock, initial_tempo_rate: tempo, max_block_frames: BLOCK, ..EngineConfig::default() }).map_err(|e| format!("the tempo engine wouldn't start: {e:?}"))?;
    Ok(Deck { ctl: h.controller, proc: h.processor, feed: h.source })
}

pub struct Stretched {
    inner: Box<dyn Source>,
    tempo: Tempo,
    deck: Deck,
    rate: u32,
    /// where the deck's first frame is in the track (after a seek)
    start_ms: u64,
    /// read from the track but not taken by the deck yet
    pending: Vec<f32>,
    inner_done: bool,
    finished: bool,
    applied: f64,
    block: Vec<f32>,
    /// the track's frames pushed in all, once it's done (0 until then)
    real_frames: u64,
    /// a key shift, as a factor of pitch (1.0: none), the app's to move (see Stretched::pitch)
    pitch: Tempo,
    /// its resampling, for one shifted (none: never shifted, played as it is)
    bend: Option<Bend>,
    last_at: f64,
    /// what it gives from its start (or a seek) is where it starts, to the frame: its deck's own delay
    /// (a constant, in frames of what it gives, at any tempo) still to be let go
    delay_left: usize,
}

impl Stretched {
    pub fn new(inner: Box<dyn Source>, tempo: Tempo) -> Result<Self, String> {
        Self::shifted(inner, tempo, 0)
    }

    fn delayed(mut self) -> Self {
        self.delay_left = self.deck.proc.pipeline_latency_frames();
        self
    }

    /// One taking over from where `inner` is (`start_ms` into the track), not its start
    pub fn starting(inner: Box<dyn Source>, tempo: Tempo, start_ms: u64) -> Result<Self, String> {
        let mut s = Self::shifted(inner, tempo, 0)?;
        s.start_ms = start_ms;
        Ok(s)
    }

    /// Where its output is, in frames of what it's been given (to the frame: a takeover lines up on it)
    pub fn source_frames(&self) -> f64 {
        self.deck.ctl.source_position()
    }

    /// The same, its key shifted by whole semitones (up or down)
    pub fn shifted(inner: Box<dyn Source>, tempo: Tempo, semitones: i32) -> Result<Self, String> {
        let rate = inner.rate();
        let p = if semitones == 0 { 1.0 } else { 2f64.powf(semitones as f64 / 12.0) };
        let t = tempo.get() / p;
        let bend = if semitones == 0 { None } else { Some(Bend::new(p)?) };
        Ok(Self { deck: deck(rate, t)?, inner, tempo, rate, start_ms: 0, pending: Vec::new(), inner_done: false, finished: false, applied: t, block: vec![0.0; BLOCK * 2], real_frames: 0, last_at: -1.0, pitch: Tempo::new(p), bend, delay_left: 0 }.delayed())
    }

    /// Its key shift, to move as it plays (a factor of pitch: 1.0 its own key); only one made shifted
    /// can move (one that isn't stays as it is)
    pub fn pitch(&self) -> Tempo {
        self.pitch.clone()
    }

    /// Feeds the deck enough of the track for the next block
    fn feed(&mut self) -> Result<(), String> {
        let want = self.deck.feed.demand_hint(BLOCK, 2.0) * 2;
        while self.deck.feed.occupied_frames() < want {
            if !self.pending.is_empty() {
                let took = self.deck.feed.push(&self.pending);
                self.pending.drain(..took * 2);
                if !self.pending.is_empty() { break; }   // (the ring's full: the rest next time)
                continue;
            }
            if self.inner_done {
                if !self.finished {
                    // (the track's own frames, before the padding that flushes the deck)
                    if self.real_frames == 0 { self.real_frames = self.deck.feed.pushed_frames().max(1); }
                    self.finished = self.deck.feed.finish();
                }
                break;
            }
            let mut piece = Vec::new();
            if !self.inner.read(&mut piece)? { self.inner_done = true; }
            self.pending = piece;
        }
        Ok(())
    }
}

impl Source for Stretched {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn duration_ms(&self) -> Option<u64> {
        self.inner.duration_ms()
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        let p = if self.bend.is_some() { self.pitch.get() } else { 1.0 };
        let t = self.tempo.get() / p;
        if (t - self.applied).abs() > 1e-6 {
            self.deck.ctl.set_tempo_rate(t);
            self.applied = t;
        }
        self.feed()?;
        // the end: everything the track gave has come out
        // (the deck holds back its own small delay, so it's over once the track's all in and it stops
        // moving on)
        let at = self.deck.ctl.source_position();
        if self.finished && (at >= self.real_frames as f64 - 1.0 || at <= self.last_at) {
            return Ok(false);
        }
        self.last_at = at;
        self.deck.proc.process(&mut self.block);
        // (its delay, just started: let go)
        let drop = self.delay_left.min(self.block.len() / 2);
        self.delay_left -= drop;
        match self.bend.as_mut() {
            Some(b) => b.run(p, &self.block[drop * 2..], out),
            None => out.extend_from_slice(&self.block[drop * 2..]),
        }
        Ok(true)
    }
    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        let at = self.inner.seek(ms)?;
        let p = if self.bend.is_some() { self.pitch.get() } else { 1.0 };
        self.deck = deck(self.rate, self.tempo.get() / p)?;
        self.applied = self.tempo.get() / p;
        if let Some(b) = self.bend.as_mut() { b.reset(); }
        self.start_ms = at;
        self.pending.clear();
        self.inner_done = false;
        self.finished = false;
        self.real_frames = 0;
        self.last_at = -1.0;
        self.delay_left = self.deck.proc.pipeline_latency_frames();
        Ok(at)
    }
    fn position_ms(&self) -> Option<u64> {
        Some(self.start_ms + (self.deck.ctl.source_position().max(0.0) * 1000.0 / self.rate as f64) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::Tone;

    #[test]
    fn faster_takes_less_time_and_keeps_its_pitch() {
        let rate = 44_100;
        let tempo = Tempo::new(1.25);
        let mut s = Stretched::new(Box::new(Tone::new(rate, 440.0, 0.3, 4.0)), tempo).unwrap();
        let mut out = Vec::new();
        while s.read(&mut out).unwrap() { if out.len() > rate as usize * 2 * 10 { break; } }
        let secs = out.len() as f64 / 2.0 / rate as f64;
        assert!((secs - 4.0 / 1.25).abs() < 0.15, "{secs} s");
        // still 440 Hz: about 44 upward crossings in 0.1 s
        let mid = out.len() / 4;
        let ch: Vec<f32> = out[mid..mid + 2 * 4410].chunks(2).map(|f| f[0]).collect();
        let n = ch.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((40..=48).contains(&n), "{n}");
        // and where it is in the track, in the track's own time
        assert!(s.position_ms().unwrap() >= 3800, "{:?}", s.position_ms());
    }

    #[test]
    fn a_seek_after_the_key_bends_back_stays_in_its_own_key() {
        let rate = 44_100;
        let mut s = Stretched::shifted(Box::new(Tone::new(rate, 440.0, 0.3, 6.0)), Tempo::new(1.0), 3).unwrap();
        // (bent back to its own key, then sought)
        s.pitch().set(1.0);
        let mut out = Vec::new();
        while out.len() < rate as usize * 2 { s.read(&mut out).unwrap(); }
        s.seek(1000).unwrap();
        let mut out = Vec::new();
        while out.len() < rate as usize * 2 * 2 { s.read(&mut out).unwrap(); }
        // 440 Hz, not 523 (three semitones up): about 44 upward crossings in 0.1 s
        let mid = out.len() / 2;
        let ch: Vec<f32> = out[mid..mid + 2 * 4410].chunks(2).map(|f| f[0]).collect();
        let n = ch.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((40..=48).contains(&n), "{n}");
    }

    /// Clicks every half second (one sample each), for where the sound comes out
    struct Clicks { pos: u64, len: u64, rate: u32 }
    impl Source for Clicks {
        fn rate(&self) -> u32 { self.rate }
        fn duration_ms(&self) -> Option<u64> { Some(self.len * 1000 / self.rate as u64) }
        fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
            if self.pos >= self.len { return Ok(false); }
            let n = 1024.min(self.len - self.pos);
            for i in 0..n { let v = if (self.pos + i) % (self.rate as u64 / 2) == 0 { 1.0 } else { 0.0 }; out.extend_from_slice(&[v, v]); }
            self.pos += n;
            Ok(true)
        }
        fn seek(&mut self, ms: u64) -> Result<u64, String> { self.pos = ms * self.rate as u64 / 1000; Ok(ms) }
    }

    #[test]
    fn on_time_as_heard() {
        let rate = 44_100;
        for (t, semis) in [(1.0, 0), (1.0, 1), (1.056, 1), (0.95, 2), (1.0, -1)] {
            let mut s = Stretched::shifted(Box::new(Clicks { pos: 0, len: rate as u64 * 5, rate }), Tempo::new(t), semis).unwrap();
            s.seek(1000).unwrap();
            let out = crate::engine::render(Box::new(s), rate, &[0.0; 10], 0.0, 2.5).unwrap();
            let ch: Vec<f32> = out.chunks(2).map(|f| f[0].abs()).collect();
            let gap = (rate as f64 * 0.5 / t) as usize;
            let peaks: Vec<String> = (0..4).map(|k| { let a = (k * gap).saturating_sub(gap / 2); let b = (k * gap + gap / 2).min(ch.len()); let i = (a..b).max_by(|x, y| ch[*x].total_cmp(&ch[*y])).unwrap(); format!("{:.1} ({:.2})", (i as f64 - k as f64 * gap as f64) * 1000.0 / rate as f64, ch[i]) }).collect();
            // each click where it should be (but the chain's few ms, as every track has), not its deck's
            // delay or its resampling's behind (once 95 ms a semitone up, half a second two)
            for p in &peaks[1..] { let late: f64 = p.split(' ').next().unwrap().parse().unwrap(); assert!(late.abs() < 10.0, "tempo {t} {semis:+}: {peaks:?}"); }
        }
    }

    #[test]
    fn all_of_it_heard() {
        let rate = 44_100;
        for semis in [0, 1, 2] {
            let mut s = Stretched::shifted(Box::new(Tone::new(rate, 440.0, 0.3, 4.0)), Tempo::new(1.0), semis).unwrap();
            let mut n = 0usize; let mut blocks = 0;
            let mut out = Vec::new();
            while s.read(&mut out).unwrap() { blocks += 1; n = out.len(); if blocks > 1000 { break; } }
            let r = crate::engine::render(Box::new(Stretched::shifted(Box::new(Tone::new(rate, 440.0, 0.3, 4.0)), Tempo::new(1.0), semis).unwrap()), rate, &[0.0; 10], 0.0, 10.0).unwrap();
            let _ = (n, blocks);
            // all of it heard, its end too (out of the resampling)
            assert!((r.len() as f64 / 2.0 / rate as f64 - 4.0).abs() < 0.1, "{semis:+}: {:.2} s", r.len() as f64 / 2.0 / rate as f64);
        }
    }

    #[test]
    fn a_key_shift_keeps_the_tempo() {
        let rate = 44_100;
        // two semitones up, at its own tempo, through the engine's resampling (as it plays)
        let s = Stretched::shifted(Box::new(Tone::new(rate, 440.0, 0.3, 4.0)), Tempo::new(1.0), 2).unwrap();
        let out = crate::engine::render(Box::new(s), rate, &[0.0; 10], 0.0, 10.0).unwrap();
        let secs = out.len() as f64 / 2.0 / rate as f64;
        assert!((secs - 4.0).abs() < 0.15, "{secs} s");
        // 440 Hz two semitones up is 493.9 Hz: about 49 upward crossings in 0.1 s
        let mid = out.len() / 4;
        let ch: Vec<f32> = out[mid..mid + 2 * 4410].chunks(2).map(|f| f[0]).collect();
        let n = ch.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count();
        assert!((47..=52).contains(&n), "{n}");
    }

    #[test]
    fn a_key_shift_bends_back_as_it_plays() {
        let rate = 44_100;
        // two semitones up, then back to its own key partway through (as after a mix)
        let mut s = Stretched::shifted(Box::new(Tone::new(rate, 440.0, 0.3, 6.0)), Tempo::new(1.0), 2).unwrap();
        let pitch = s.pitch();
        let mut out = Vec::new();
        while s.read(&mut out).unwrap() {
            if out.len() > rate as usize * 2 * 2 { pitch.set(1.0); }
            if out.len() > rate as usize * 2 * 20 { break; }
        }
        let crossings = |at: f64| {
            let i = (at * rate as f64) as usize * 2;
            let ch: Vec<f32> = out[i..i + 2 * 4410].chunks(2).map(|f| f[0]).collect();
            ch.windows(2).filter(|w| w[0] < 0.0 && w[1] >= 0.0).count()
        };
        // 493.9 Hz before, 440 Hz after, and every second of it still there (6 s at its own tempo)
        assert!((47..=52).contains(&crossings(1.0)), "before {}", crossings(1.0));
        assert!((42..=46).contains(&crossings(4.0)), "after {}", crossings(4.0));
        let secs = out.len() as f64 / 2.0 / rate as f64;
        assert!((secs - 6.0).abs() < 0.2, "{secs} s");
    }
}

