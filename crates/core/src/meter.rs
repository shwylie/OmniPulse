// A track's loudness and silences, measured from its samples as streaming services do: integrated
// loudness in LUFS (EBU R128 / ITU BS.1770, via the ebur128 crate), its sample peak, silences of 2 s
// or more, and where its sound really ends. core::loudness decodes and keeps them (desktop); the
// phone's engine feeds it what it decodes itself.

use serde::{Deserialize, Serialize};

pub const WINDOW_MS: u64 = 50; // silence is judged in windows this long
const SILENT_RMS_DB: f64 = -55.0; // a window is silent below this level...
const SILENT_PEAK_DB: f64 = -40.0; // ...with no peak above this
const MIN_SILENCE_MS: u64 = 2000; // shorter silences aren't noted
// Where the sound really ends: after the last window within this much of the track's own loudness
// (a fade-out or a quiet tail is already past it), but never below AUDIBLE_FLOOR_DB
const AUDIBLE_BELOW_LUFS_DB: f64 = 20.0;
const AUDIBLE_FLOOR_DB: f64 = -50.0;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Analysis {
    /// Integrated loudness, LUFS
    pub lufs: f64,
    /// Highest sample, dBFS
    pub peak_db: f64,
    /// How long the track is (from the stream's own length when only part was measured)
    pub duration_ms: u64,
    /// How much of it was measured: silences after this point aren't known
    pub analysed_ms: u64,
    /// Silent stretches of MIN_SILENCE_MS or more, [from, to] in ms
    pub silences: Vec<[u64; 2]>,
    /// Where the sound really ends (its fade-out or quiet tail starts), in ms; 0 when not known
    /// (only part of the track was measured). Fade Between Tracks fades out before it.
    #[serde(default)]
    pub audible_ms: u64,
}

// ---------- measuring ----------

pub struct Meter {
    r128: ebur128::EbuR128,
    channels: usize,
    rate: u64,
    frames: u64,
    win_frames: u64,
    win_sq: f64,
    win_peak: f32,
    win_n: u64,
    silent_from: Option<u64>, // frame where the current silent stretch began
    silences: Vec<[u64; 2]>,
    peak: f32,
    /// each window's level, dB rms, for finding where the sound ends
    levels: Vec<f32>,
}

impl Meter {
    pub fn new(channels: usize, rate: u32) -> Result<Meter, String> {
        let r128 = ebur128::EbuR128::new(channels as u32, rate, ebur128::Mode::I)
            .map_err(|e| format!("loudness meter: {e:?}"))?;
        Ok(Meter {
            r128,
            channels,
            rate: rate as u64,
            frames: 0,
            win_frames: (rate as u64 * WINDOW_MS / 1000).max(1),
            win_sq: 0.0,
            win_peak: 0.0,
            win_n: 0,
            silent_from: None,
            silences: Vec::new(),
            peak: 0.0,
            levels: Vec::new(),
        })
    }

    fn ms(&self, frames: u64) -> u64 {
        frames * 1000 / self.rate
    }

    fn close_silence(&mut self, end_frame: u64) {
        if let Some(from) = self.silent_from.take() {
            let (a, b) = (self.ms(from), self.ms(end_frame));
            if b - a >= MIN_SILENCE_MS {
                self.silences.push([a, b]);
            }
        }
    }

    fn end_window(&mut self) {
        let rms = (self.win_sq / (self.win_n.max(1) * self.channels as u64) as f64).sqrt();
        let silent = db(rms) < SILENT_RMS_DB && db(self.win_peak as f64) < SILENT_PEAK_DB;
        let start = self.frames - self.win_n;
        self.levels.push(db(rms) as f32);
        if silent {
            self.silent_from.get_or_insert(start);
        } else {
            self.close_silence(start);
        }
        self.win_sq = 0.0;
        self.win_peak = 0.0;
        self.win_n = 0;
    }

    /// Interleaved samples, `channels` per frame
    pub fn push(&mut self, samples: &[f32]) -> Result<(), String> {
        self.r128.add_frames_f32(samples).map_err(|e| format!("loudness meter: {e:?}"))?;
        for frame in samples.chunks_exact(self.channels) {
            for &s in frame {
                let a = s.abs();
                self.win_sq += (s as f64) * (s as f64);
                if a > self.win_peak {
                    self.win_peak = a;
                }
                if a > self.peak {
                    self.peak = a;
                }
            }
            self.frames += 1;
            self.win_n += 1;
            if self.win_n == self.win_frames {
                self.end_window();
            }
        }
        Ok(())
    }

    pub fn finish(mut self, duration_ms: Option<u64>) -> Result<Analysis, String> {
        if self.win_n > 0 {
            self.end_window();
        }
        let analysed_ms = self.ms(self.frames);
        let duration_ms = duration_ms.filter(|d| *d > analysed_ms).unwrap_or(analysed_ms);
        // silence that runs to the end of what was measured: to the end of the track if that's
        // all of it, otherwise only as far as we know
        self.close_silence(self.frames);
        let lufs = self.r128.loudness_global().map_err(|e| format!("loudness meter: {e:?}"))?;
        // the end of the last window loud enough to count, when the whole track was measured
        let audible_ms = if lufs.is_finite() && analysed_ms + 1500 >= duration_ms {
            let floor = (lufs - AUDIBLE_BELOW_LUFS_DB).max(AUDIBLE_FLOOR_DB) as f32;
            self.levels.iter().rposition(|&l| l >= floor).map_or(0, |i| ((i as u64 + 1) * WINDOW_MS).min(analysed_ms))
        } else {
            0
        };
        Ok(Analysis {
            // all silence (or too short to measure) comes out as -inf: call it very quiet
            lufs: if lufs.is_finite() { round1(lufs) } else { -70.0 },
            peak_db: if self.peak > 0.0 { round1(db(self.peak as f64)) } else { -100.0 },
            duration_ms,
            analysed_ms,
            silences: self.silences,
            audible_ms,
        })
    }
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-10).log10()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

