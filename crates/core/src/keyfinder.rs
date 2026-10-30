// A song's key exactly as libKeyFinder reads one (Mixxx's key detector; a port of its method, not a model:
// nothing learned from data). core::keys borrows some of its ideas; this follows it step for step:
//
// - mono, low-passed at ~2 kHz (a 160th-order windowed-sinc filter) and taken down to ~4.4 kHz;
// - 16384-sample frames (3.7 s) every 4096, under a Blackman window;
// - each of 72 semitones, C1 (32.7 Hz) to B6 (1.98 kHz), its energy through a spectral kernel (a raised
//   cosine 0.8 of a semitone wide, scaled by the note's frequency: close to a constant-Q transform);
// - the frames averaged into one 72-note vector, and that against Sha'ath's major and minor profiles,
//   each octave weighted as KeyFinder weights it, by cosine similarity: the best of the 24 keys.
//
// Ported from libKeyFinder (https://github.com/mixxxdj/libkeyfinder), Copyright 2011-2015 Ibrahim Sha'ath,
// under the GNU General Public License, version 3 or later, as OmniPulse is.

use realfft::RealFftPlanner;

const SEMITONES: usize = 12;
const OCTAVES: usize = 6;
const BANDS: usize = SEMITONES * OCTAVES;
const FFT: usize = 16384;
const HOP: usize = FFT / 4;
const STRETCH: f64 = 0.8;

/// Band b's frequency: C1 (MIDI 24) up a semitone at a time
fn band_hz(b: usize) -> f64 { 440.0 * 2f64.powf((24.0 + b as f64 - 69.0) / 12.0) }

/// Sha'ath's profiles, from the tonic up (KeyFinder's), and its weight for each octave
const MAJOR: [f64; 12] = [7.239_005_026_181_452, 3.503_511_667_251_587, 3.584_451_775_366_494, 2.845_118_164_786_763, 5.818_988_921_185_499, 4.558_650_574_153_210, 2.447_788_505_455_065, 6.994_731_921_468_295, 3.391_066_136_735_049, 4.556_142_566_551_430, 4.073_926_666_635_236, 4.459_327_573_788_869];
const MINOR: [f64; 12] = [7.002_550_450_602_844, 3.143_602_790_159_967, 4.359_043_197_149_625, 5.404_181_207_189_341, 3.672_344_208_793_061, 4.089_711_849_177_979, 3.907_914_359_915_540, 6.199_602_885_623_165, 3.634_246_256_252_774, 2.872_411_910_798_756, 5.354_679_997_945_427, 3.832_420_385_950_484];
const OCTAVE_WEIGHTS: [f64; OCTAVES] = [0.399_972_675_5, 0.556_344_252_483_006, 0.524_966_363_451_435, 0.608_475_483_842_777, 0.598_981_156_8, 0.490_724_353_179_610];

/// What it reads: the key (tonic 0 = C), whether minor, and how far ahead of the next key it is (0..1)
#[derive(Clone, Debug, PartialEq)]
pub struct Reading {
    pub tonic: usize,
    pub minor: bool,
    pub confidence: f64,
}

/// Low-passed (a windowed sinc, 161 taps, Hamming) and taken down by `factor`: only the samples kept are worked out
fn downsample(x: &[f32], rate: u32, cutoff: f64, factor: usize) -> Vec<f32> {
    const TAPS: usize = 161;
    let fc = cutoff / rate as f64;
    let mid = (TAPS / 2) as f64;
    let mut h: Vec<f64> = (0..TAPS).map(|n| {
        let t = n as f64 - mid;
        let sinc = if t == 0.0 { 2.0 * fc } else { (2.0 * std::f64::consts::PI * fc * t).sin() / (std::f64::consts::PI * t) };
        sinc * (0.54 - 0.46 * (2.0 * std::f64::consts::PI * n as f64 / (TAPS - 1) as f64).cos())
    }).collect();
    let sum: f64 = h.iter().sum();
    for v in &mut h { *v /= sum; }
    let mut out = Vec::with_capacity(x.len() / factor + 1);
    let mut i = 0;
    while i < x.len() {
        let mut acc = 0.0f64;
        for (k, c) in h.iter().enumerate() {
            let j = i as isize + k as isize - mid as isize;
            if j >= 0 && (j as usize) < x.len() { acc += c * x[j as usize] as f64; }
        }
        out.push(acc as f32);
        i += factor;
    }
    out
}

/// The key of mono audio at `rate` (None: too short, or silent)
pub fn read(x: &[f32], rate: u32) -> Option<Reading> {
    let last = band_hz(BANDS - 1);
    let factor = ((rate as f64 / 2.0 / (last * 1.10)).floor() as usize).max(1);
    let y = downsample(x, rate, last * 1.012, factor);
    let sr = rate as f64 / factor as f64;
    if y.len() < FFT { return None; }
    // (each band's kernel: the FFT bins it covers, and their weights)
    let q = STRETCH * (2f64.powf(1.0 / SEMITONES as f64) - 1.0);
    let kernel: Vec<(usize, Vec<f64>)> = (0..BANDS).map(|b| {
        let centre = band_hz(b) * FFT as f64 / sr;
        let width = centre * q;
        let begin = centre - width / 2.0;
        let first = begin.ceil() as usize;
        let mut w: Vec<f64> = (first..=(begin + width).floor() as usize).map(|bin| 1.0 - (2.0 * std::f64::consts::PI * (bin as f64 - begin) / width).cos()).collect();
        let s: f64 = w.iter().sum();
        if s > 0.0 { for v in &mut w { *v = *v / s * band_hz(b); } }
        (first, w)
    }).collect();
    let window: Vec<f64> = (0..FFT).map(|n| 0.42 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (FFT - 1) as f64).cos() + 0.08 * (4.0 * std::f64::consts::PI * n as f64 / (FFT - 1) as f64).cos()).collect();
    let fft = RealFftPlanner::<f64>::new().plan_fft_forward(FFT);
    let (mut inp, mut spec) = (fft.make_input_vec(), fft.make_output_vec());
    let mut sum = [0.0f64; BANDS];
    // (frames as KeyFinder makes them: zero-padded at the end to whole hops)
    let hops = (y.len() as f64 / HOP as f64).ceil() as usize;
    for h in 0..hops {
        for (n, v) in inp.iter_mut().enumerate() { *v = y.get(h * HOP + n).copied().unwrap_or(0.0) as f64 * window[n]; }
        fft.process(&mut inp, &mut spec).ok()?;
        for (b, (first, w)) in kernel.iter().enumerate() {
            sum[b] += w.iter().enumerate().map(|(j, c)| spec.get(first + j).map_or(0.0, |z| z.norm()) * c).sum::<f64>();
        }
    }
    if sum.iter().all(|v| *v == 0.0) { return None; }
    // (each key: cosine similarity of the 72 notes with the profile, each octave weighted, turned to its tonic)
    let score = |profile: &[f64; 12], tonic: usize| {
        let (mut dot, mut pn, mut xn) = (0.0, 0.0, 0.0);
        for b in 0..BANDS {
            let p = OCTAVE_WEIGHTS[b / SEMITONES] * profile[(b % SEMITONES + SEMITONES - tonic) % SEMITONES];
            dot += sum[b] * p; pn += p * p; xn += sum[b] * sum[b];
        }
        dot / (pn.sqrt() * xn.sqrt())
    };
    let mut all: Vec<(f64, usize, bool)> = (0..12).flat_map(|t| [(score(&MAJOR, t), t, false), (score(&MINOR, t), t, true)]).collect();
    all.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (best, next) = (all[0], all[1]);
    Some(Reading { tonic: best.1, minor: best.2, confidence: if best.0 > 0.0 { ((best.0 - next.0) / best.0).clamp(0.0, 1.0) } else { 0.0 } })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Chords of a key (tonic, subdominant, dominant, tonic), a second each, four times over
    fn chords(notes: &[[f64; 3]], rate: u32) -> Vec<f32> {
        let mut x = Vec::new();
        for _ in 0..4 {
            for ch in notes {
                for i in 0..rate as usize {
                    let t = i as f64 / rate as f64;
                    let v: f64 = ch.iter().map(|m| (2.0 * std::f64::consts::PI * 440.0 * 2f64.powf((m - 69.0) / 12.0) * t).sin()).sum::<f64>() / 3.0;
                    x.push(v as f32 * 0.4);
                }
            }
        }
        x
    }

    #[test]
    fn keys_of_chords() {
        for rate in [22_050, 44_100, 48_000] {
            // C major: C E G, F A C, G B D, C E G
            let r = read(&chords(&[[60.0, 64.0, 67.0], [65.0, 69.0, 72.0], [67.0, 71.0, 74.0], [60.0, 64.0, 67.0]], rate), rate).unwrap();
            assert_eq!((r.tonic, r.minor), (0, false), "{rate}: {r:?}");
            // A minor: A C E, D F A, E G# B, A C E
            let r = read(&chords(&[[57.0, 60.0, 64.0], [62.0, 65.0, 69.0], [64.0, 68.0, 71.0], [57.0, 60.0, 64.0]], rate), rate).unwrap();
            assert_eq!((r.tonic, r.minor), (9, true), "{rate}: {r:?}");
            // F# major (well away from C): F# A# C#, B D# F#, C# F G#, F# A# C#
            let r = read(&chords(&[[66.0, 70.0, 73.0], [71.0, 75.0, 78.0], [73.0, 77.0, 80.0], [66.0, 70.0, 73.0]], rate), rate).unwrap();
            assert_eq!((r.tonic, r.minor), (6, false), "{rate}: {r:?}");
        }
        assert!(read(&vec![0.0; 44_100 * 10], 44_100).is_none());
    }
}
