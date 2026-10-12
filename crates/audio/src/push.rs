// Sound pushed in by something else as it decodes: Spotify's librespot, which hands over stereo f32 at
// 44.1 kHz in pieces tagged with a generation (bumped on every new track or seek). Older generations'
// pieces are dropped; the producer is told how much has been played, so it stays only a little ahead;
// a seek asks it to start again from there.

use crate::source::Source;
use std::collections::VecDeque;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

struct State {
    queue: VecDeque<f32>,
    gen: u32,
    ended: bool,
    closed: bool,
}

struct Inner {
    state: Mutex<State>,
    more: Condvar,
}

/// The producer's end: pieces go in here
#[derive(Clone)]
pub struct PushFeed(Arc<Inner>);

impl PushFeed {
    /// A piece of interleaved stereo; false if it's from an old generation (and was dropped)
    pub fn send(&self, gen: u32, samples: &[f32]) -> bool {
        let mut s = self.0.state.lock().unwrap();
        if s.closed || gen < s.gen {
            return false;
        }
        if gen > s.gen {
            // a new track or a seek: what's waiting is from before it
            s.queue.clear();
            s.gen = gen;
            s.ended = false;
        }
        s.queue.extend(samples.iter().copied());
        drop(s);
        self.0.more.notify_all();
        true
    }

    /// The producer has sent all of this generation
    pub fn end(&self, gen: u32) {
        let mut s = self.0.state.lock().unwrap();
        if gen >= s.gen {
            s.ended = true;
        }
        drop(s);
        self.0.more.notify_all();
    }
}

type SeekFn = Box<dyn FnMut(u64) -> Result<u32, String> + Send>;
type PlayedFn = Box<dyn Fn(u32, u64) + Send>;

/// The engine's end: a Source like any other
pub struct PushSource {
    inner: Arc<Inner>,
    rate: u32,
    duration_ms: Option<u64>,
    seek_fn: SeekFn,
    played_fn: PlayedFn,
    /// how long to wait for the producer before giving up on this read (it may be buffering)
    patience: Duration,
    /// frames of the current generation taken so far (the producer is told the total, not each piece)
    taken: (u32, u64),
}

impl PushSource {
    /// `seek` asks the producer to restart at a time and returns the generation it'll send;
    /// `played` tells it how many frames of a generation have been taken
    pub fn new(rate: u32, duration_ms: Option<u64>, seek: SeekFn, played: PlayedFn) -> (Self, PushFeed) {
        let inner = Arc::new(Inner { state: Mutex::new(State { queue: VecDeque::new(), gen: 0, ended: false, closed: false }), more: Condvar::new() });
        let feed = PushFeed(inner.clone());
        (Self { inner, rate, duration_ms, seek_fn: seek, played_fn: played, patience: Duration::from_secs(10), taken: (0, 0) }, feed)
    }
}

impl Drop for PushSource {
    fn drop(&mut self) {
        self.inner.state.lock().unwrap().closed = true;
    }
}

impl Source for PushSource {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        let mut s = self.inner.state.lock().unwrap();
        let deadline = std::time::Instant::now() + self.patience;
        while s.queue.is_empty() && !s.ended {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return Err("Spotify stopped sending sound.".into());
            }
            s = self.inner.more.wait_timeout(s, left.min(Duration::from_millis(250))).unwrap().0;
        }
        if s.queue.is_empty() {
            return Ok(false); // ended, and all of it played
        }
        let n = s.queue.len().min(4096 * 2) & !1;
        out.extend(s.queue.drain(..n));
        let gen = s.gen;
        drop(s);
        if self.taken.0 != gen { self.taken = (gen, 0); }
        self.taken.1 += (n / 2) as u64;
        (self.played_fn)(gen, self.taken.1);
        Ok(true)
    }

    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        let gen = (self.seek_fn)(ms)?;
        let mut s = self.inner.state.lock().unwrap();
        // from here on, only what comes after the seek counts
        if gen > s.gen {
            s.queue.clear();
            s.gen = gen;
        }
        s.ended = false;
        Ok(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn plays_what_is_pushed_and_drops_older_generations() {
        let played = Arc::new(AtomicU64::new(0));
        let seen = played.clone();
        let next_gen = Arc::new(Mutex::new(1u32));
        let g2 = next_gen.clone();
        let (src, feed) = PushSource::new(
            44100,
            Some(1000),
            Box::new(move |_ms| { let mut g = g2.lock().unwrap(); *g += 1; Ok(*g) }),
            // (as spotify_engine::spe_played: the total so far of this generation)
            Box::new(move |_gen, frames| { seen.store(frames, Ordering::SeqCst); }),
        );
        // a second of a tone, pushed from another thread in 100 ms pieces
        let producer = {
            let feed = feed.clone();
            std::thread::spawn(move || {
                for k in 0..10 {
                    let piece: Vec<f32> = (0..4410).flat_map(|i| { let v = 0.25 * ((k * 4410 + i) as f32 * 0.0627).sin(); [v, v] }).collect();
                    assert!(feed.send(1, &piece));
                    std::thread::sleep(Duration::from_millis(5));
                }
                feed.end(1);
            })
        };
        let out = crate::render(Box::new(src), 48000, &[0.0; 10], 0.0, 5.0).unwrap();
        producer.join().unwrap();
        // a second at the output's rate, and all of it reported as played
        assert!((out.len() as i64 / 2 - 48000).abs() < 1200, "{}", out.len() / 2);
        assert_eq!(played.load(Ordering::SeqCst), 44100);
        // a piece from an old generation is refused once a newer one has started
        let (mut src, feed) = PushSource::new(44100, None, Box::new(|_| Ok(5)), Box::new(|_, _| {}));
        assert!(feed.send(3, &[0.1; 8]));
        src.seek(2000).unwrap(); // generation 5 from here
        assert!(!feed.send(4, &[0.2; 8]));
        assert!(feed.send(5, &[0.3; 8]));
        let mut piece = Vec::new();
        assert!(src.read(&mut piece).unwrap());
        assert_eq!(piece, vec![0.3; 8]);
    }
}
