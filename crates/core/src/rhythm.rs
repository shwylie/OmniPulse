// A song's tempo, beats and key, measured from its sound (for seamless mixing: beat-matched
// crossfades, phrase-aligned cuts, and key-compatible order). The apps give it mono samples (the
// first part of a track as it plays); nothing here knows where they came from.
//
// - Onsets: spectral flux (log magnitudes, 5 ms hops at 22.05 kHz), a bass band's on its own too.
// - Tempo: the onsets' autocorrelation, weighted towards 120 BPM (as people hear tempo), with a check
//   for double time (drum & bass at 174, not 87).
// - Beats: dynamic programming over the onsets (Ellis's beat tracker), each beat then placed on its
//   onset's peak (to about 5 ms); a straight grid fitted through them gives the period and first beat.
// - Downbeat: the one of each four where the bass hits hardest.
// - Key: a chromagram (pitch classes, 65 Hz to 2 kHz) against the Krumhansl-Kessler major and minor
//   profiles (Sha'ath's, as KeyFinder's), with its Camelot code (the DJ's wheel: neighbours mix well).

use realfft::RealFftPlanner;
use serde::{Deserialize, Serialize};

const RATE: u32 = 22_050;
const HOP: usize = 110; // ~5 ms
const WIN: usize = 512; // ~23 ms
const FPS: f64 = RATE as f64 / HOP as f64;
/// How much a song's section changes count, against its bass, for which beat of four is its first
const DOWNBEAT_CHANGE_WEIGHT: f64 = 2.0;
/// The share of section changes on one beat that makes it the downbeat for sure
pub const DOWNBEAT_SURE: f64 = 0.45;
/// A downbeat sure enough to mix on (its confidence; with Beat This!'s, the share of its downbeats on it:
/// a mashup's sections, each its own, can disagree)
pub const DOWNBEAT_TRUSTED: f64 = 0.7;
const CHROMA_WIN: usize = 8192;
const CHROMA_HOP: usize = 2048;

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct Rhythm {
    pub bpm: f64,
    /// the grid: the first beat (ms from where the measured part starts) and the beat's length (ms)
    pub first_beat_ms: f64,
    pub period_ms: f64,
    /// each beat found, ms from where the measured part starts
    pub beats: Vec<f64>,
    /// which of the beats (0..3) is the first downbeat (the "one" of a bar)
    pub downbeat: usize,
    /// 0..1: how clearly it has a beat
    pub tempo_confidence: f64,
    /// "A minor", "C major"; its Camelot code ("8A", "8B"); 0..1 how clearly
    pub key: String,
    pub camelot: String,
    pub key_confidence: f64,
    /// how sure the downbeat is (0..1): the share of the song's section changes that land on it (a
    /// drop, a breakdown, a new verse: on a bar's first beat, near always). Low: a guess from its bass
    #[serde(default)]
    pub downbeat_confidence: f64,
    /// the tempo Beat This!'s own beats are at (0: not known); one that doesn't agree with `bpm` (as
    /// it is, or at half or double) says the tracked beats are on another pulse (two thirds of the
    /// tempo, say, a syncopated song's), and aren't to be mixed on
    #[serde(default)]
    pub model_bpm: f64,
    /// each beat's loudness (dB, from it to the next): where a half-time song's phrases start, which can
    /// be half of one of its bars
    #[serde(default)]
    pub beat_db: Vec<f32>,
    /// where its phrases start, moved by hand (bars; + later): set as it's used, never kept with it
    #[serde(default, skip_serializing)]
    pub phrase_shift: i32,
    /// each bar's loudness (dB, from the first downbeat on): where the intro ends and the outro begins
    #[serde(default)]
    pub bar_db: Vec<f32>,
    /// each bar's beat: about 1 as strong as the song's usual, well under where it has none (a
    /// beatless intro or outro)
    #[serde(default)]
    pub bar_beat: Vec<f32>,
    /// each bar's notes: how much of each pitch class (C to B, summing to 1) it has (Melody Play: a
    /// melody looped against what's coming in)
    #[serde(default)]
    pub bar_chroma: Vec<[f32; 12]>,
}

const NOTES: [&str; 12] = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
// Camelot numbers of the major keys, by pitch class (C is 8B); a minor key's is its relative major's
const CAMELOT_MAJOR: [u8; 12] = [8, 3, 10, 5, 12, 7, 2, 9, 4, 11, 6, 1];
// the key profiles: Sha'ath's (KeyFinder's), which tested better here than Krumhansl-Kessler's
const KK_MAJOR: [f64; 12] = [7.24, 3.50, 3.58, 2.85, 5.82, 4.56, 2.45, 6.99, 3.39, 4.56, 4.07, 4.46];
const KK_MINOR: [f64; 12] = [7.00, 3.14, 4.36, 5.40, 3.67, 4.09, 3.91, 6.20, 3.63, 2.87, 5.35, 3.83];

/// Mono samples at any rate down to 22.05 kHz (a simple averaging decimator: the onsets and the
/// chroma's range don't need more)
pub fn to_analysis_rate(mono: &[f32], rate: u32) -> Vec<f32> {
    if rate == RATE {
        return mono.to_vec();
    }
    let step = rate as f64 / RATE as f64;
    let n = (mono.len() as f64 / step) as usize;
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        let a = (i as f64 * step) as usize;
        let b = (((i + 1) as f64 * step) as usize).min(mono.len()).max(a + 1);
        out.push(mono[a..b].iter().sum::<f32>() / (b - a) as f32);
    }
    out
}

fn hann(n: usize) -> Vec<f32> {
    (0..n).map(|i| 0.5 - 0.5 * (2.0 * std::f32::consts::PI * i as f32 / n as f32).cos()).collect()
}

/// The onset envelope (all frequencies) and the bass band's (below 150 Hz), one value per hop
fn onsets(x: &[f32]) -> (Vec<f64>, Vec<f64>) {
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WIN);
    let w = hann(WIN);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let bass_bins = (150.0 * WIN as f64 / RATE as f64).ceil() as usize;
    let frames = if x.len() > WIN { (x.len() - WIN) / HOP + 1 } else { 0 };
    let mut prev: Vec<f32> = vec![0.0; WIN / 2 + 1];
    let (mut all, mut bass) = (Vec::with_capacity(frames), Vec::with_capacity(frames));
    for f in 0..frames {
        let s = &x[f * HOP..f * HOP + WIN];
        for i in 0..WIN { input[i] = s[i] * w[i]; }
        if fft.process(&mut input, &mut spec).is_err() { break; }
        let (mut a, mut b) = (0.0f64, 0.0f64);
        for (k, c) in spec.iter().enumerate() {
            let m = (1.0 + 100.0 * c.norm()).ln();
            let d = (m - prev[k]).max(0.0) as f64;
            a += d;
            if k > 0 && k <= bass_bins { b += d; }
            prev[k] = m;
        }
        all.push(a);
        bass.push(b);
    }
    // less its local mean (about a quarter of a second), and no less than nothing
    let detrend = |v: &mut Vec<f64>| {
        let r = 25usize;
        let mut sum: f64 = v.iter().take(r).sum();
        let orig = v.clone();
        for i in 0..orig.len() {
            if i + r < orig.len() { sum += orig[i + r]; }
            if i >= r + 1 { sum -= orig[i - r - 1]; }
            let n = (i + r + 1).min(orig.len()) - i.saturating_sub(r);
            v[i] = (orig[i] - sum / n as f64).max(0.0);
        }
        let peak = v.iter().cloned().fold(0.0, f64::max);
        if peak > 0.0 { v.iter_mut().for_each(|x| *x /= peak); }
    };
    detrend(&mut all);
    detrend(&mut bass);
    (all, bass)
}

fn autocorr(o: &[f64], lag: usize) -> f64 {
    if lag >= o.len() { return 0.0; }
    o[..o.len() - lag].iter().zip(&o[lag..]).map(|(a, b)| a * b).sum::<f64>() / (o.len() - lag) as f64
}

/// The beat's length in frames (fractional), and how clearly there's one
fn tempo(o: &[f64]) -> (f64, f64) {
    let lag_of = |bpm: f64| 60.0 * FPS / bpm;
    let (lo, hi) = (lag_of(200.0).floor() as usize, lag_of(60.0).ceil() as usize);
    let r: Vec<f64> = (0..=4 * hi + 2).map(|l| autocorr(o, l)).collect();
    let weight = |lag: f64| {
        let bpm = 60.0 * FPS / lag;
        (-0.5 * ((bpm / 120.0).log2() / 0.9).powi(2)).exp()
    };
    // (a beat's multiples count too: two beats, a bar of four; a 3-3-2 rhythm's pull to 4/3 of
    // the tempo has none)
    let at = |l: usize| if l < r.len() { r[l] } else { 0.0 };
    let mut best = (0usize, f64::MIN);
    for l in lo..=hi {
        let s = (r[l] + 0.5 * at(2 * l) + 0.25 * at(4 * l)) * weight(l as f64);
        if s > best.1 { best = (l, s); }
    }
    let refine = |l: usize| -> f64 {
        if l == 0 || l + 1 >= r.len() { return l as f64; }
        let (a, b, c) = (r[l - 1], r[l], r[l + 1]);
        let d = a - 2.0 * b + c;
        if d.abs() < 1e-12 { l as f64 } else { l as f64 + 0.5 * (a - c) / d }
    };
    let mut lag = refine(best.0);
    // double time: as strong at half the length (and a tempo people dance to)
    let half = (lag / 2.0).round() as usize;
    if half >= 2 && 60.0 * FPS / (lag / 2.0) <= 185.0 && r[half] >= 0.8 * r[best.0] {
        lag = refine(half);
    }
    let base = r[0].max(1e-12);
    (lag, (r[lag.round() as usize] / base).clamp(0.0, 1.0))
}

/// Ellis's dynamic-programming beat tracker: beats where the onsets are, about a period apart
fn track(o: &[f64], period: f64) -> Vec<usize> {
    let n = o.len();
    if n == 0 || period < 2.0 { return Vec::new(); }
    let alpha = 100.0;
    let mut score = vec![0.0f64; n];
    let mut from = vec![usize::MAX; n];
    let (lo, hi) = ((period / 2.0).round() as usize, (2.0 * period).round() as usize);
    for t in 0..n {
        let mut best = (0.0f64, usize::MAX);
        for back in lo.max(1)..=hi {
            if back > t { break; }
            let p = t - back;
            let pen = (back as f64 / period).ln().powi(2) * alpha;
            let s = score[p] - pen;
            if best.1 == usize::MAX || s > best.0 { best = (s, p); }
        }
        score[t] = o[t] + if best.1 == usize::MAX { 0.0 } else { best.0.max(0.0) };
        from[t] = if best.1 != usize::MAX && best.0 > 0.0 { best.1 } else { usize::MAX };
    }
    // the best end within the last period, then back
    let tail = n.saturating_sub(period.round() as usize);
    let mut t = (tail..n).max_by(|&a, &b| score[a].total_cmp(&score[b])).unwrap_or(n - 1);
    let mut beats = vec![t];
    while from[t] != usize::MAX {
        t = from[t];
        beats.push(t);
    }
    beats.reverse();
    beats
}

/// A beat's frame placed on its onset's peak (parabolic, within two frames), as ms
fn place(o: &[f64], f: usize) -> f64 {
    let (a, b) = (f.saturating_sub(2), (f + 2).min(o.len() - 1));
    let k = (a..=b).max_by(|&x, &y| o[x].total_cmp(&o[y])).unwrap_or(f);
    let off = if k > 0 && k + 1 < o.len() {
        let (l, c, r) = (o[k - 1], o[k], o[k + 1]);
        let d = l - 2.0 * c + r;
        if d.abs() < 1e-12 { 0.0 } else { (0.5 * (l - r) / d).clamp(-0.5, 0.5) }
    } else { 0.0 };
    // (a frame's onset is where its window starts to hear it: half a window in)
    ((k as f64 + off) * HOP as f64 + WIN as f64 / 2.0) / RATE as f64 * 1000.0
}

/// The key: the chroma (pitch classes) against the major and minor profiles; and each frame's chroma
/// (its middle, ms; its pitch classes, summing to 1)
fn key(x: &[f32]) -> (usize, bool, f64, Vec<(f64, [f32; 12])>) {
    let mut each = Vec::new();
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(CHROMA_WIN);
    let w = hann(CHROMA_WIN);
    let mut input = fft.make_input_vec();
    let mut spec = fft.make_output_vec();
    let mut chroma = [0.0f64; 12];
    // each bin's note (C2 to C6, 65 Hz to 1 kHz). Each note counts once per octave (its strongest bin),
    // not once per bin: high octaves have many more bins to a note, and they're mostly overtones
    const LOW: i64 = 36;
    const HIGH: i64 = 84;
    let notes: Vec<Option<usize>> = (0..=CHROMA_WIN / 2).map(|k| {
        let f = k as f64 * RATE as f64 / CHROMA_WIN as f64;
        if f <= 0.0 { return None; }
        let midi = (69.0 + 12.0 * (f / 440.0).log2()).round() as i64;
        (LOW..HIGH).contains(&midi).then_some((midi - LOW) as usize)
    }).collect();
    let frames = if x.len() > CHROMA_WIN { (x.len() - CHROMA_WIN) / CHROMA_HOP + 1 } else { 0 };
    for f in 0..frames {
        let s = &x[f * CHROMA_HOP..f * CHROMA_HOP + CHROMA_WIN];
        for i in 0..CHROMA_WIN { input[i] = s[i] * w[i]; }
        if fft.process(&mut input, &mut spec).is_err() { break; }
        let mut note = [0.0f64; (HIGH - LOW) as usize];
        for (k, c) in spec.iter().enumerate() {
            if let Some(n) = notes[k] { note[n] = note[n].max(c.norm() as f64); }
        }
        let mut fr = [0.0f64; 12];
        for (n, e) in note.iter().enumerate() { fr[(n + LOW as usize) % 12] += e.sqrt(); }
        let sum: f64 = fr.iter().sum();
        if sum > 0.0 { for i in 0..12 { chroma[i] += fr[i] / sum; } }
        let mid = (f * CHROMA_HOP + CHROMA_WIN / 2) as f64 / RATE as f64 * 1000.0;
        each.push((mid, if sum > 0.0 { fr.map(|v| (v / sum) as f32) } else { [0.0; 12] }));
    }
    let corr = |profile: &[f64; 12], tonic: usize| {
        let p: Vec<f64> = (0..12).map(|i| profile[(i + 12 - tonic) % 12]).collect();
        let (mx, mp) = (chroma.iter().sum::<f64>() / 12.0, p.iter().sum::<f64>() / 12.0);
        let num: f64 = (0..12).map(|i| (chroma[i] - mx) * (p[i] - mp)).sum();
        let den = ((0..12).map(|i| (chroma[i] - mx).powi(2)).sum::<f64>() * (0..12).map(|i| (p[i] - mp).powi(2)).sum::<f64>()).sqrt();
        if den > 0.0 { num / den } else { 0.0 }
    };
    let mut all: Vec<(f64, usize, bool)> = Vec::new();
    for t in 0..12 {
        all.push((corr(&KK_MAJOR, t), t, false));
        all.push((corr(&KK_MINOR, t), t, true));
    }
    all.sort_by(|a, b| b.0.total_cmp(&a.0));
    let (best, second) = (all[0], all[1]);
    (best.1, best.2, ((best.0 - second.0) * 4.0).clamp(0.0, 1.0), each)
}

/// The first reading of a key (each note's loudest bin, Krumhansl's profiles), of mono at any rate:
/// tonic, minor, confidence (for comparing with core::keys)
pub fn key_first_reading(mono: &[f32], rate: u32) -> (usize, bool, f64) {
    let x = to_analysis_rate(mono, rate);
    let (t, m, c, _) = key(&x);
    (t, m, c)
}

pub fn key_name(tonic: usize, minor: bool) -> String {
    format!("{} {}", NOTES[tonic % 12], if minor { "minor" } else { "major" })
}

pub fn camelot(tonic: usize, minor: bool) -> String {
    if minor { format!("{}A", CAMELOT_MAJOR[(tonic + 3) % 12]) } else { format!("{}B", CAMELOT_MAJOR[tonic % 12]) }
}

/// Mono samples at the analysis rate (for kick_strength)
pub fn at_analysis_rate(mono: &[f32], rate: u32) -> Vec<f32> { to_analysis_rate(mono, rate) }

#[cfg(feature = "model")]
type BeatModel = beat_this::BeatThis<<beat_this::RtenRuntime as beat_this::Runtime>::Model>;

/// Beat This!'s model while it's in use: loaded when a song's measured, let go once it's been idle a
/// while or Seamless Mix is switched off (release_model): none of its memory kept for nothing
#[cfg(feature = "model")]
static MODEL: std::sync::Mutex<Option<(BeatModel, std::time::Instant)>> = std::sync::Mutex::new(None);

/// The model, loaded if it isn't (its files written next to the app's data the first time), for `f`
#[cfg(feature = "model")]
fn with_model<R>(f: impl FnOnce(&mut BeatModel) -> R) -> Option<R> {
    // (one thread for it, at background priority: it's seconds of work a song, done ahead, and never to
    // be had at a game's expense; set before its pool's first use, which reads it, and only once)
    static POOL: std::sync::Once = std::sync::Once::new();
    POOL.call_once(|| {
        if std::env::var_os("RTEN_NUM_THREADS").is_none() {
            // SAFETY: set once, before the model's thread pool is made; nothing else reads it
            unsafe { std::env::set_var("RTEN_NUM_THREADS", "1") };
        }
        rten::thread_pool().run(background_begin);
    });
    let mut g = MODEL.lock().ok()?;
    if g.is_none() {
        let dir = crate::data_dir().ok()?.join("models");
        std::fs::create_dir_all(&dir).ok()?;
        let put = |name: &str, bytes: &[u8]| -> Option<std::path::PathBuf> {
            let p = dir.join(name);
            if std::fs::metadata(&p).map(|m| m.len() as usize).ok() != Some(bytes.len()) { std::fs::write(&p, bytes).ok()?; }
            Some(p)
        };
        let mel = put("mel_spectrogram.onnx", include_bytes!("../models/mel_spectrogram.onnx"))?;
        let beats = put("beat_this_small.onnx", include_bytes!("../models/beat_this_small.onnx"))?;
        *g = Some((beat_this::BeatThis::new(&beat_this::RtenRuntime, &mel, &beats).ok()?, std::time::Instant::now()));
    }
    let (m, used) = g.as_mut()?;
    *used = std::time::Instant::now();
    Some(f(m))
}

/// The model let go if it hasn't been used for `idle` (Duration::ZERO: now): loaded again when it's next needed
pub fn release_model(idle: std::time::Duration) {
    #[cfg(feature = "model")]
    if let Ok(mut g) = MODEL.try_lock() {
        if g.as_ref().is_some_and(|(_, used)| used.elapsed() >= idle) { *g = None; }
    }
    let _ = idle;
}

/// The thread calling this in the background from now on (Windows: its CPU and disk priority lowered, so
/// anything in the foreground, a game, comes first)
pub fn background_begin() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" { fn GetCurrentThread() -> isize; fn SetThreadPriority(h: isize, p: i32) -> i32; }
        // SAFETY: the pseudo-handle of the calling thread, a documented priority mode
        unsafe { SetThreadPriority(GetCurrentThread(), 0x0001_0000); }
    }
    // (Android: the lowest priority, so the phone's scheduler keeps it off the big cores where it can)
    #[cfg(target_os = "android")]
    {
        extern "C" { fn gettid() -> i32; fn setpriority(which: i32, who: u32, prio: i32) -> i32; }
        // SAFETY: the calling thread's id; PRIO_PROCESS (0) with a tid sets that thread's nice value
        unsafe { setpriority(0, gettid() as u32, 19); }
    }
}

/// Back out of the background (THREAD_MODE_BACKGROUND_END)
pub fn background_end() {
    #[cfg(windows)]
    {
        #[link(name = "kernel32")]
        extern "system" { fn GetCurrentThread() -> isize; fn SetThreadPriority(h: isize, p: i32) -> i32; }
        // SAFETY: as above
        unsafe { SetThreadPriority(GetCurrentThread(), 0x0002_0000); }
    }
}

/// A steady grid through a model's beats (as DJ software grids electronic music: one tempo all through):
/// each beat numbered by its place at `bpm`, a straight line through them (least squares), and a beat on
/// it every period from the first. Its beats one at a time can be out by tens of ms; the line isn't.
fn steady_grid(beats: &[f64], bpm: f64) -> Vec<f64> {
    let p0 = 60_000.0 / bpm;
    let t0 = beats[0];
    let pts: Vec<(f64, f64)> = beats.iter().map(|t| (((t - t0) / p0).round(), *t)).collect();
    let n = pts.len() as f64;
    let (mx, my) = (pts.iter().map(|p| p.0).sum::<f64>() / n, pts.iter().map(|p| p.1).sum::<f64>() / n);
    let (sxy, sxx) = pts.iter().fold((0.0, 0.0), |(a, b), p| (a + (p.0 - mx) * (p.1 - my), b + (p.0 - mx).powi(2)));
    let period = if sxx > 0.0 { sxy / sxx } else { p0 };
    let mut first = my - period * mx;
    while first - period >= 0.0 { first -= period; }
    while first < 0.0 { first += period; }
    let last = beats[beats.len() - 1] + period / 2.0;
    (0..).map(|k| first + k as f64 * period).take_while(|t| *t <= last).collect()
}

/// What Beat This! found: its beats and downbeats (ms), and its beats' tempo
pub(crate) struct ModelBeats {
    beats: Vec<f64>,
    downbeats: Vec<f64>,
    bpm: f64,
    /// of stretches of it only (model_sampled), not all of it
    sampled: bool,
}

/// The model on two 30-second stretches of a song (at three tenths and seven tenths of it; a short song,
/// all of it): its beats and downbeats there, in the song's time, and its tempo from them
/// Each run of the model followed by a rest as long as it took (a phone's: half the heat, spread out;
/// measuring's done well ahead of the mix it's for)
pub static PACE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn paced<T>(f: impl FnOnce() -> T) -> T {
    let t = std::time::Instant::now();
    let r = f();
    if PACE.load(std::sync::atomic::Ordering::Relaxed) { std::thread::sleep(t.elapsed()); }
    r
}

fn model_sampled(mono: &[f32], rate: u32) -> Option<ModelBeats> {
    let len = mono.len() as f64 / rate.max(1) as f64;
    if len <= 100.0 { return model_windowed(mono, rate); }
    let mut all = ModelBeats { beats: Vec::new(), downbeats: Vec::new(), bpm: 0.0, sampled: true };
    let mut bpms = Vec::new();
    for frac in [0.25, 0.4, 0.6, 0.75] {
        let from = ((frac * len - 7.5).max(0.0) * rate as f64) as usize;
        let to = (from + 15 * rate as usize).min(mono.len());
        let Some(m) = paced(|| model_run(&mono[from..to], rate)) else { continue };
        let off = from as f64 * 1000.0 / rate as f64;
        // (a stretch's first and last beat or so left out: the model's least sure at its edges)
        all.beats.extend(m.beats.iter().filter(|b| **b > 1500.0 && **b < 13_500.0).map(|b| b + off));
        all.downbeats.extend(m.downbeats.iter().filter(|b| **b > 1500.0 && **b < 13_500.0).map(|b| b + off));
        if m.bpm > 0.0 { bpms.push(m.bpm); }
    }
    if bpms.is_empty() { return None; }
    bpms.sort_by(|a, b| a.total_cmp(b));
    all.bpm = bpms[bpms.len() / 2];
    Some(all)
}

/// The model over all of it, 15 s at a time (its own stretches are 30 s, and its memory goes with
/// their square: 400 MB at 30 s, a quarter of it at 15), each window's first and last 1.5 s left to
/// its neighbours, which overlap it by as much
fn model_windowed(mono: &[f32], rate: u32) -> Option<ModelBeats> {
    let (win, edge) = (15 * rate as usize, (1.5 * rate as f64) as usize);
    if mono.len() <= win { return paced(|| model_run(mono, rate)); }
    let mut all = ModelBeats { beats: Vec::new(), downbeats: Vec::new(), bpm: 0.0, sampled: false };
    let mut bpms = Vec::new();
    let mut from = 0usize;
    loop {
        let to = (from + win).min(mono.len());
        let last = to == mono.len();
        if let Some(m) = paced(|| model_run(&mono[from..to], rate)) {
            let off = from as f64 * 1000.0 / rate as f64;
            // (the part this window answers for: its middle; the first window from its start, the last to its end)
            let lo = if from == 0 { f64::MIN } else { 1500.0 };
            let hi = if last { f64::MAX } else { (to - from - edge) as f64 * 1000.0 / rate as f64 };
            all.beats.extend(m.beats.iter().filter(|b| **b >= lo && **b < hi).map(|b| b + off));
            all.downbeats.extend(m.downbeats.iter().filter(|b| **b >= lo && **b < hi).map(|b| b + off));
            if m.bpm > 0.0 { bpms.push(m.bpm); }
        }
        if last { break; }
        from = to - 2 * edge;
    }
    if bpms.is_empty() { return None; }
    bpms.sort_by(|a, b| a.total_cmp(b));
    all.bpm = bpms[bpms.len() / 2];
    Some(all)
}

#[cfg(feature = "model")]
fn model_run(mono: &[f32], rate: u32) -> Option<ModelBeats> {
    let a = with_model(|m| m.analyze_audio(mono, rate))?.ok()?;
    // (its own beats' tempo: the gaps between them near their median, averaged: they're whole frames of
    // 20 ms, so one gap alone is up to 2% out)
    let mut gaps: Vec<f64> = a.beats.windows(2).map(|w| (w[1] - w[0]) as f64).filter(|g| *g > 0.2 && *g < 1.5).collect();
    gaps.sort_by(|x, y| x.total_cmp(y));
    let mid = gaps.get(gaps.len() / 2).copied().unwrap_or(0.0);
    let near: Vec<f64> = gaps.iter().copied().filter(|g| (g / mid - 1.0).abs() < 0.1).collect();
    let model_bpm = if near.is_empty() { 0.0 } else { 60.0 * near.len() as f64 / near.iter().sum::<f64>() };
    Some(ModelBeats { beats: a.beats.iter().map(|b| *b as f64 * 1000.0).collect(), downbeats: a.downbeats.iter().map(|d| *d as f64 * 1000.0).collect(), bpm: model_bpm, sampled: false })
}

#[cfg(not(feature = "model"))]
fn model_run(_: &[f32], _: u32) -> Option<ModelBeats> { None }

/// Which of four beats (from the first) is each bar's first, by Beat This!'s downbeats (each one onto
/// the nearest beat), and the share of them that agree. None: too few to go by.
fn model_downbeat(a: &ModelBeats, beats: &[f64]) -> Option<(usize, f64)> {
    let mut by = [0usize; 4];
    for d in &a.downbeats {
        let t = *d;
        let i = beats.partition_point(|b| *b < t);
        let near = [i.checked_sub(1), Some(i)].into_iter().flatten().filter(|&j| j < beats.len()).min_by(|&x, &y| (beats[x] - t).abs().total_cmp(&(beats[y] - t).abs()))?;
        // (onto the nearest of these beats, as long as it's nearer it than the next: which beat of the bar it
        // is, not where to the millisecond; ours and the model's can sit tens of ms apart)
        let period = if beats.len() > 1 { (beats[beats.len() - 1] - beats[0]) / (beats.len() - 1) as f64 } else { 500.0 };
        if (beats[near] - t).abs() <= period * 0.4 { by[near % 4] += 1; }
    }
    let n: usize = by.iter().sum();
    if n < 8 { return None; }
    let best = (0..4).max_by_key(|&k| by[k])?;
    if std::env::var_os("OMNIPULSE_RHYTHM_DEBUG").is_some() { eprintln!("rhythm debug: Beat This! downbeats by beat of four {by:?} (of {}), its beats at {:.2} BPM", a.downbeats.len(), a.bpm); }
    Some((best, by[best] as f64 / n as f64))
}

/// Where a song's snares (or claps) fall, by beat of four (from its first tracked beat): each beat's
/// upper mids (1.5 to 6 kHz: a snare's crack, not its hats) rising within 40 ms of it, summed, as shares
fn snare_by_phase(x: &[f32], beats: &[f64]) -> [f64; 4] {
    let lp = |hz: f32| 1.0 - (-2.0 * std::f32::consts::PI * hz / RATE as f32).exp();
    let (a15, a6) = (lp(1500.0), lp(6000.0));
    let (mut l15, mut l6) = (0.0f32, 0.0f32);
    const FRAME: usize = RATE as usize / 1000;
    let e: Vec<f32> = x.chunks(FRAME).map(|c| c.iter().map(|v| { l15 += a15 * (v - l15); l6 += a6 * (v - l6); let m = l6 - l15; m * m }).sum::<f32>() / c.len() as f32).collect();
    let n = e.len();
    let sm: Vec<f32> = (0..n).map(|i| { let (a, b) = (i.saturating_sub(3), (i + 3).min(n)); e[a..b].iter().sum::<f32>() / (b - a) as f32 }).collect();
    let fms = FRAME as f64 * 1000.0 / RATE as f64;
    let mut by = [0.0f64; 4];
    for (k, b) in beats.iter().enumerate() {
        let c = (b / fms) as usize;
        let (lo, hi) = (c.saturating_sub(40).max(3), (c + 40).min(n.saturating_sub(3)));
        if hi <= lo { continue; }
        // (its sharpest rise near the beat, against the level before it: a crack, not a held sound)
        let r = (lo..hi).map(|i| (sm[i + 3] - sm[i - 3]).max(0.0) / (sm[i - 3] + 1e-9)).fold(0.0f32, f32::max);
        by[k % 4] += (r as f64).min(50.0);
    }
    let t: f64 = by.iter().sum::<f64>().max(1e-9);
    by.map(|v| v / t)
}

/// Tempo, beats and key of mono samples at `rate` (a few seconds at least; the more the better)
pub fn analyse(mono: &[f32], rate: u32) -> Option<Rhythm> {
    // (measuring is never in a hurry: in the background while it runs)
    struct Bg;
    impl Drop for Bg { fn drop(&mut self) { background_end(); } }
    background_begin();
    let _bg = Bg;
    analyse_now(mono, rate)
}

fn analyse_now(mono: &[f32], rate: u32) -> Option<Rhythm> {
    let t0 = std::time::Instant::now();
    let x = to_analysis_rate(mono, rate);
    if std::env::var_os("OMNIPULSE_RHYTHM_DEBUG").is_some() { eprintln!("rhythm debug: resampled in {:.2} s", t0.elapsed().as_secs_f64()); }
    if x.len() < RATE as usize * 6 { return None; }
    let (o, bass) = onsets(&x);
    let (period, conf) = tempo(&o);
    let frames = track(&o, period);
    // Beat This!'s beats, where it can; where ours are on another pulse than its (as they are, or at half
    // or double: two thirds of the tempo, say, a syncopated song's), its beats instead, each placed on our
    // onsets (its frames are 20 ms; ours 5)
    let t_model = std::time::Instant::now();
    // (two stretches of it, not all of it: what the model's for here (its first beats, its tempo) is the
    // same throughout, and it's most of the work; all of it only where its beats are to replace ours)
    let mut model = model_sampled(mono, rate);
    if std::env::var_os("OMNIPULSE_RHYTHM_DEBUG").is_some() { eprintln!("rhythm debug: the model took {:.2} s", t_model.elapsed().as_secs_f64()); }
    let tracked_bpm = 60_000.0 / (period / FPS * 1000.0);
    let agrees = |m: f64| [1.0, 2.0, 0.5].iter().any(|k| ((tracked_bpm * k) / m - 1.0).abs() <= 0.03);
    if model.as_ref().is_some_and(|m| m.bpm > 0.0 && !agrees(m.bpm) && m.sampled) { model = model_windowed(mono, rate); }
    let (frames, from_model) = match &model {
        Some(m) if m.bpm > 0.0 && m.beats.len() >= 32 && !agrees(m.bpm) => (steady_grid(&m.beats, m.bpm).iter().map(|t| (t / 1000.0 * FPS).round() as usize).filter(|&f| f < o.len()).collect::<Vec<_>>(), true),
        _ => (frames, false),
    };
    if std::env::var_os("OMNIPULSE_RHYTHM_DEBUG").is_some() && from_model { eprintln!("rhythm debug: tracked at {tracked_bpm:.2} BPM, Beat This! at {:.2}: its beats used", model.as_ref().map_or(0.0, |m| m.bpm)); }
    if frames.len() < 4 { return None; }
    let beats: Vec<f64> = frames.iter().map(|&f| place(&o, f)).collect();
    // a straight grid through them: least squares of beat time against its number
    let n = beats.len() as f64;
    let (mi, mt) = ((n - 1.0) / 2.0, beats.iter().sum::<f64>() / n);
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (i, t) in beats.iter().enumerate() {
        sxy += (i as f64 - mi) * (t - mt);
        sxx += (i as f64 - mi).powi(2);
    }
    let period_ms = if sxx > 0.0 { sxy / sxx } else { period / FPS * 1000.0 };
    let mut first = mt - period_ms * mi;
    while first - period_ms >= 0.0 { first -= period_ms; }
    while first < 0.0 { first += period_ms; }
    // the downbeat: of each four, the one where the bass hits hardest, and (more) where the song's
    // sections change (a drop, a breakdown, a new verse: on a bar's first beat, near always): each beat's
    // loudness, its next 4 against its last 4, summed by where in the four it is. A song that changes a
    // lot leans on its changes; one that hardly does, on its bass.
    let mut phase = [0.0f64; 4];
    for (i, &f) in frames.iter().enumerate() {
        let (a, b) = (f.saturating_sub(2), (f + 2).min(bass.len() - 1));
        phase[i % 4] += (a..=b).map(|k| bass[k]).fold(0.0, f64::max);
    }
    let loud: Vec<f64> = beats.windows(2).map(|w| {
        let (i, j) = (((w[0] / 1000.0) * RATE as f64) as usize, (((w[1] / 1000.0) * RATE as f64) as usize).min(x.len()));
        if j <= i { return -120.0; }
        (10.0 * (x[i..j].iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / (j - i) as f64).max(1e-12).log10()).max(-60.0)
    }).collect();
    // (each change once, where it's biggest: its next 8 beats against its last 8, the biggest within a
    // bar of it kept; 2 dB and up, so the beat-to-beat wobble of a steady part doesn't count)
    let mut jumps: Vec<(f64, usize)> = (8..loud.len().saturating_sub(8)).map(|i| ((loud[i..i + 8].iter().sum::<f64>() - loud[i - 8..i].iter().sum::<f64>()).abs() / 8.0, i)).filter(|j| j.0 >= 2.0).collect();
    jumps.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut change = [0.0f64; 4];
    let mut kept: Vec<usize> = Vec::new();
    for (d, i) in jumps {
        if kept.iter().all(|k| k.abs_diff(i) > 3) { kept.push(i); change[i % 4] += d; }
    }
    let share = |v: &[f64; 4]| { let t: f64 = v.iter().sum(); if t > 0.0 { v.map(|x| x / t) } else { [0.25; 4] } };
    let (bass_share, change_share) = (share(&phase), share(&change));
    // (the changes decisive: most of them on one beat, well ahead of the next: that's the downbeat,
    // whatever the bass says (a kick on 1 and 3 can't tell them apart); else both, and not sure)
    let mut order: Vec<usize> = (0..4).collect();
    order.sort_by(|&a, &b| change_share[b].total_cmp(&change_share[a]));
    let decisive = change_share[order[0]] >= DOWNBEAT_SURE && change_share[order[0]] >= 1.4 * change_share[order[1]];
    let score: Vec<f64> = (0..4).map(|k| bass_share[k] + DOWNBEAT_CHANGE_WEIGHT * change_share[k]).collect();
    let downbeat = if decisive { order[0] } else { (0..4).max_by(|&a, &b| score[a].total_cmp(&score[b])).unwrap_or(0) };
    let downbeat_confidence = if decisive { change_share[order[0]] } else { change_share[downbeat].min(DOWNBEAT_SURE - 0.01) };
    // (Beat This!, a model trained on annotated songs, where it can: its downbeats, onto these beats; the
    // share of them on one beat of four, how sure. The clues above disagree on halftime tracks, mashups)
    // (its stretches not enough to be sure which beat is the first: all of it, then)
    if model.as_ref().is_some_and(|m| m.sampled && model_downbeat(m, &beats).is_none_or(|d| d.1 < DOWNBEAT_TRUSTED)) { model = model_windowed(mono, rate); }
    let model_bpm = model.as_ref().map_or(0.0, |m| m.bpm);
    let (downbeat, downbeat_confidence) = model.as_ref().and_then(|m| model_downbeat(m, &beats)).unwrap_or((downbeat, downbeat_confidence));
    // (its beats the model's: as sure of them as of a clear beat of ours)
    let conf = if from_model { conf.max(0.5) } else { conf };
    if std::env::var_os("OMNIPULSE_RHYTHM_DEBUG").is_some() {
        let old = (0..4).max_by(|&a, &b| phase[a].total_cmp(&phase[b])).unwrap_or(0);
        eprintln!("rhythm debug: downbeat by bass {old}, now {downbeat}; bass {:?} changes {:?}", bass_share.map(|v| (v * 100.0).round()), change_share.map(|v| (v * 100.0).round()));
    }
    let (tonic, minor, kconf, frames_chroma) = key(&x);
    // (the key itself read as core::keys does, KeyFinder's way; the frames' notes as they were, for the bars')
    let (tonic, minor, kconf) = crate::keys::estimate(&x, &crate::keys::KeyOpts::default()).map_or((tonic, minor, kconf), |k| (k.tonic, k.minor, k.confidence));
    if std::env::var_os("OMNIPULSE_RHYTHM_DEBUG").is_some() {
        // (each beat's notes, and how much they change from the two before to the two after it, by beat of four)
        let per: Vec<[f32; 12]> = beats.windows(2).map(|w| {
            let mut c = [0.0f32; 12];
            for (_, fr) in frames_chroma.iter().filter(|(t, _)| *t >= w[0] && *t < w[1]) { for k in 0..12 { c[k] += fr[k]; } }
            c
        }).collect();
        let mut chord = [0.0f64; 4];
        for i in 2..per.len().saturating_sub(2) {
            let sum = |a: usize, b: usize| { let mut c = [0.0f32; 12]; for v in &per[a..b] { for k in 0..12 { c[k] += v[k]; } } c };
            let (x0, x1) = (sum(i - 2, i), sum(i, i + 2));
            let dot: f32 = (0..12).map(|k| x0[k] * x1[k]).sum();
            let n = (x0.iter().map(|v| v * v).sum::<f32>() * x1.iter().map(|v| v * v).sum::<f32>()).sqrt().max(1e-9);
            chord[i % 4] += (1.0 - dot / n) as f64;
        }
        let t: f64 = chord.iter().sum::<f64>().max(1e-9);
        let snare = snare_by_phase(&x, &beats);
        eprintln!("rhythm debug: snares by beat of four {:?}", snare.map(|v| (v * 100.0).round()));
        eprintln!("rhythm debug: downbeat {downbeat} (sure {:.2}); by beat of four (0..4 from the first tracked): bass {:?}, section changes {:?}, chord changes {:?}", downbeat_confidence, bass_share.map(|v| (v * 100.0).round()), change_share.map(|v| (v * 100.0).round()), chord.map(|v| (v / t * 100.0).round()));
    }
    // each bar's loudness, from the first downbeat on (four tracked beats to a bar), and how strong its
    // beat is: its onsets and its bass at its beats, against the song's usual (a median beat's: 1)
    let peak = |e: &[f64], f: usize| { let (a, b) = (f.saturating_sub(2), (f + 2).min(e.len() - 1)); (a..=b).map(|k| e[k]).fold(0.0, f64::max) };
    let median = |e: &[f64]| { let mut v: Vec<f64> = frames.iter().map(|&f| peak(e, f)).collect(); v.sort_by(|a, b| a.total_cmp(b)); v[v.len() / 2].max(1e-12) };
    let (mo, mb) = (median(&o), median(&bass));
    let mut bar_db = Vec::new();
    let mut bar_beat = Vec::new();
    let mut bar_chroma = Vec::new();
    let mut b = downbeat;
    while b < beats.len() {
        let fs = &frames[b..(b + 4).min(frames.len())];
        let strength = fs.iter().map(|&f| 0.5 * peak(&o, f) / mo + 0.5 * peak(&bass, f) / mb).sum::<f64>() / fs.len() as f64;
        let start = beats[b];
        let end = beats.get(b + 4).copied().unwrap_or(start + 4.0 * period_ms);
        let (i, j) = (((start / 1000.0) * RATE as f64) as usize, (((end / 1000.0) * RATE as f64) as usize).min(x.len()));
        if j <= i { break; }
        let ms = x[i..j].iter().map(|v| (*v as f64) * (*v as f64)).sum::<f64>() / (j - i) as f64;
        bar_db.push((10.0 * ms.max(1e-12).log10()) as f32);
        bar_beat.push(strength as f32);
        // (its notes: its frames', summed, as shares)
        let mut c = [0.0f32; 12];
        for (_, fr) in frames_chroma.iter().filter(|(t, _)| *t >= start && *t < end) { for k in 0..12 { c[k] += fr[k]; } }
        let sum: f32 = c.iter().sum();
        if sum > 0.0 { for v in &mut c { *v /= sum; } }
        bar_chroma.push(c);
        b += 4;
    }
    Some(Rhythm {
        bpm: (60_000.0 / period_ms * 100.0).round() / 100.0,
        first_beat_ms: first,
        period_ms,
        beats,
        downbeat,
        tempo_confidence: conf,
        key: key_name(tonic, minor),
        camelot: camelot(tonic, minor),
        key_confidence: kconf,
        downbeat_confidence,
        model_bpm,
        phrase_shift: 0,
        beat_db: loud.iter().map(|v| (*v * 10.0).round() as f32 / 10.0).collect(),
        bar_db,
        bar_beat,
        bar_chroma,
    })
}

// ---------- what's been measured, by track (state/rhythm.json) ----------
/// A track's measurement: where in it the measured part starts (its beats are from there)
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Measured {
    pub from_ms: u64,
    pub to_ms: u64,
    #[serde(flatten)]
    pub rhythm: Rhythm,
    /// this measurement's method (measured again when it changes)
    pub v: u32,
}

/// (2-3: beats moved half a beat where the bass hit harder between them, wrongly: an off-beat bass
/// louder than the kick isn't a beat tracked off it; 4: that undone, the downbeat from where the
/// sections change too)
pub const VERSION: u32 = 13;

static STORE: std::sync::Mutex<Option<std::collections::HashMap<String, Measured>>> = std::sync::Mutex::new(None);

fn store_file() -> Option<std::path::PathBuf> {
    let d = crate::data_dir().ok()?.join("state");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("rhythm.json"))
}

fn with_store<T>(f: impl FnOnce(&mut std::collections::HashMap<String, Measured>) -> T) -> T {
    let mut g = STORE.lock().unwrap();
    if g.is_none() {
        *g = Some(store_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default());
    }
    f(g.as_mut().unwrap())
}

/// A track's measurement (by its key), if it has one of this method
pub fn known(key: &str) -> Option<Measured> {
    with_store(|m| m.get(key).filter(|x| x.v == VERSION).cloned())
}

/// Keeps a track's measurement
pub fn keep(key: &str, m: Measured) {
    let text = with_store(|all| {
        all.insert(key.to_string(), m);
        serde_json::to_string(all).ok()
    });
    if let (Some(p), Some(t)) = (store_file(), text) { let _ = std::fs::write(p, t); }
}

/// A key's tonic (0 is C) and whether it's minor, from its name ("A minor")
pub fn key_parts(key: &str) -> Option<(usize, bool)> {
    let (note, mode) = key.split_once(' ')?;
    Some((NOTES.iter().position(|n| *n == note)?, mode == "minor"))
}

/// A key shifted by semitones: its name and Camelot code
pub fn transposed(key: &str, semitones: i32) -> Option<(String, String)> {
    let (t, minor) = key_parts(key)?;
    let t = ((t as i32 + semitones).rem_euclid(12)) as usize;
    Some((key_name(t, minor), camelot(t, minor)))
}

/// Whether two keys mix well (the Camelot wheel: the same, one step round, or the relative major/minor)
pub fn keys_mix(a: &str, b: &str) -> bool {
    let parse = |c: &str| -> Option<(i32, char)> { let (n, l) = c.split_at(c.len().checked_sub(1)?); Some((n.parse().ok()?, l.chars().next()?)) };
    let (Some((na, la)), Some((nb, lb))) = (parse(a), parse(b)) else { return false };
    let step = ((na - nb).rem_euclid(12)).min((nb - na).rem_euclid(12));
    (la == lb && step <= 1) || (la != lb && step == 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A beat at `bpm` from `offset_ms`: a kick (low thump) on each downbeat of four, clicks between
    fn beat_track(bpm: f64, offset_ms: f64, secs: f64, rate: u32) -> Vec<f32> {
        let n = (secs * rate as f64) as usize;
        let mut x = vec![0.0f32; n];
        let period = 60.0 / bpm;
        let mut k = 0;
        loop {
            let t0 = offset_ms / 1000.0 + k as f64 * period;
            if t0 >= secs { break; }
            let s0 = (t0 * rate as f64) as usize;
            let kick = k % 4 == 0;
            for i in 0..(rate as usize / 10) {
                if s0 + i >= n { break; }
                let t = i as f64 / rate as f64;
                let v = if kick { (2.0 * std::f64::consts::PI * 55.0 * t).sin() * (-t * 30.0).exp() } else { (2.0 * std::f64::consts::PI * 3000.0 * t).sin() * (-t * 200.0).exp() * 0.5 };
                x[s0 + i] += v as f32 * 0.6;
            }
            k += 1;
        }
        x
    }

    #[test]
    fn tempo_and_beats_of_a_click_track() {
        let rate = 44_100;
        for (bpm, off) in [(128.0, 230.0), (174.0, 90.0), (93.0, 410.0)] {
            let x = beat_track(bpm, off, 30.0, rate);
            let r = analyse(&x, rate).expect("analysed");
            assert!((r.bpm - bpm).abs() < 0.5, "{bpm}: got {}", r.bpm);
            // the grid's first beat lands where the first click is (within 10 ms)
            assert!((r.first_beat_ms - off).abs() < 10.0, "{bpm}: first beat {} vs {off}", r.first_beat_ms);
            // and every beat found is on a click
            let period = 60_000.0 / bpm;
            for b in &r.beats {
                let k = ((b - off) / period).round();
                assert!((b - (off + k * period)).abs() < 12.0, "{bpm}: beat at {b}");
            }
            // the kicks are on the ones
            let first_down = r.beats[r.downbeat];
            let k = ((first_down - off) / period).round() as i64;
            assert_eq!(k.rem_euclid(4), 0, "{bpm}: downbeat");
        }
    }

    /// Chords of a key, a second each: tonic, subdominant, dominant, tonic
    fn chords(notes: &[[f64; 3]], secs_each: f64, rate: u32) -> Vec<f32> {
        let mut x = Vec::new();
        for _ in 0..4 {
            for ch in notes {
                for i in 0..(secs_each * rate as f64) as usize {
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
        let rate = 44_100;
        // C major: C E G, F A C, G B D, C E G
        let c = chords(&[[60.0, 64.0, 67.0], [65.0, 69.0, 72.0], [67.0, 71.0, 74.0], [60.0, 64.0, 67.0]], 1.0, rate);
        let r = analyse(&c, rate).expect("analysed");
        assert_eq!((r.key.as_str(), r.camelot.as_str()), ("C major", "8B"));
        // A minor: A C E, D F A, E G# B, A C E
        let a = chords(&[[57.0, 60.0, 64.0], [62.0, 65.0, 69.0], [64.0, 68.0, 71.0], [57.0, 60.0, 64.0]], 1.0, rate);
        let r = analyse(&a, rate).expect("analysed");
        assert_eq!((r.key.as_str(), r.camelot.as_str()), ("A minor", "8A"));
    }

    #[test]
    fn the_camelot_wheel() {
        assert_eq!(camelot(0, false), "8B");
        assert_eq!(camelot(9, true), "8A");
        assert_eq!(camelot(7, false), "9B");
        assert_eq!(camelot(4, true), "9A");
        assert!(keys_mix("8A", "9A") && keys_mix("8A", "8B") && keys_mix("12B", "1B"));
        assert!(!keys_mix("8A", "10A") && !keys_mix("8A", "9B"));
        assert_eq!(transposed("A minor", 2), Some(("B minor".into(), "10A".into())));
        assert_eq!(transposed("C major", -1), Some(("B major".into(), "1B".into())));
    }
}

/// OMNIPULSE_WAV=<16-bit stereo wav> cargo test -p omnipulse-core rhythm_of_a_wav -- --ignored --nocapture:
/// the beat of each 12 s of it (a mix's beats in step show one clear tempo through it)
#[cfg(test)]
#[test]
#[ignore]
fn rhythm_of_a_wav() {
    let b = std::fs::read(std::env::var("OMNIPULSE_WAV").unwrap()).unwrap();
    let rate = u32::from_le_bytes([b[24], b[25], b[26], b[27]]);
    let mono: Vec<f32> = b[44..].chunks(4).map(|f| (i16::from_le_bytes([f[0], f[1]]) as f32 + i16::from_le_bytes([f[2], f[3]]) as f32) / 65536.0).collect();
    let win = rate as usize * 12;
    for (i, part) in mono.chunks(win).enumerate() {
        if part.len() < win / 2 { break; }
        match analyse(part, rate) {
            Some(r) => println!("{:>3}-{:>3} s: {:.2} BPM, confidence {:.2}", i * 12, i * 12 + 12, r.bpm, r.tempo_confidence),
            None => println!("{:>3} s: no beat", i * 12),
        }
    }
}
