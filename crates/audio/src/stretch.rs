// A source at another tempo, its pitch kept (timestretch's keylock deck: pure Rust), for seamless
// mixing: the track coming in plays at the tempo of the one going out, then eases back to its own.
// The tempo is a shared handle the app moves while it plays (1.0: its own; 1.02: 2% faster).
// A key shift (whole semitones, for the whole track) is a pitch factor on top: the source says it's at
// that much higher a rate (so the engine's resampler plays it that much higher, and faster), and the
// deck's tempo makes up for the speed.

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
    /// a key shift, as a factor of pitch (1.0: none)
    pitch: f64,
    last_at: f64,
}

impl Stretched {
    pub fn new(inner: Box<dyn Source>, tempo: Tempo) -> Result<Self, String> {
        Self::shifted(inner, tempo, 0)
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
        let pitch = 2f64.powf(semitones as f64 / 12.0);
        let t = tempo.get() / pitch;
        Ok(Self { deck: deck(rate, t)?, inner, tempo, rate, start_ms: 0, pending: Vec::new(), inner_done: false, finished: false, applied: t, block: vec![0.0; BLOCK * 2], real_frames: 0, last_at: -1.0, pitch })
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
        // (shifted: said to be at a higher or lower rate, so it's played that much higher or lower)
        (self.rate as f64 * self.pitch).round() as u32
    }
    fn duration_ms(&self) -> Option<u64> {
        self.inner.duration_ms()
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        let t = self.tempo.get() / self.pitch;
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
        out.extend_from_slice(&self.block);
        Ok(true)
    }
    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        let at = self.inner.seek(ms)?;
        self.deck = deck(self.rate, self.tempo.get() / self.pitch)?;
        self.applied = self.tempo.get() / self.pitch;
        self.start_ms = at;
        self.pending.clear();
        self.inner_done = false;
        self.finished = false;
        self.real_frames = 0;
        self.last_at = -1.0;
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
}

