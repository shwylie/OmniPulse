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
    /// each bar's loudness (dB, from the first downbeat on): where the intro ends and the outro begins
    #[serde(default)]
    pub bar_db: Vec<f32>,
    /// each bar's beat: about 1 as strong as the song's usual, well under where it has none (a
    /// beatless intro or outro)
    #[serde(default)]
    pub bar_beat: Vec<f32>,
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

/// The key: the chroma (pitch classes) against the major and minor profiles
fn key(x: &[f32]) -> (usize, bool, f64) {
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
    (best.1, best.2, ((best.0 - second.0) * 4.0).clamp(0.0, 1.0))
}

pub fn key_name(tonic: usize, minor: bool) -> String {
    format!("{} {}", NOTES[tonic % 12], if minor { "minor" } else { "major" })
}

pub fn camelot(tonic: usize, minor: bool) -> String {
    if minor { format!("{}A", CAMELOT_MAJOR[(tonic + 3) % 12]) } else { format!("{}B", CAMELOT_MAJOR[tonic % 12]) }
}

/// Tempo, beats and key of mono samples at `rate` (a few seconds at least; the more the better)
pub fn analyse(mono: &[f32], rate: u32) -> Option<Rhythm> {
    let x = to_analysis_rate(mono, rate);
    if x.len() < RATE as usize * 6 { return None; }
    let (o, bass) = onsets(&x);
    let (period, conf) = tempo(&o);
    let frames = track(&o, period);
    if frames.len() < 4 { return None; }
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
    let mut change = [0.0f64; 4];
    for i in 4..loud.len().saturating_sub(4) {
        let (before, after) = (loud[i - 4..i].iter().sum::<f64>() / 4.0, loud[i..i + 4].iter().sum::<f64>() / 4.0);
        // (only a real change counts: 3 dB and up, so the beat-to-beat wobble of a steady part doesn't)
        let d = (after - before).abs();
        if d >= 3.0 { change[i % 4] += d; }
    }
    let share = |v: &[f64; 4]| { let t: f64 = v.iter().sum(); if t > 0.0 { v.map(|x| x / t) } else { [0.25; 4] } };
    let (bass_share, change_share) = (share(&phase), share(&change));
    let score: Vec<f64> = (0..4).map(|k| bass_share[k] + DOWNBEAT_CHANGE_WEIGHT * change_share[k]).collect();
    let downbeat = (0..4).max_by(|&a, &b| score[a].total_cmp(&score[b])).unwrap_or(0);
    if std::env::var_os("OMNIPULSE_RHYTHM_DEBUG").is_some() {
        let old = (0..4).max_by(|&a, &b| phase[a].total_cmp(&phase[b])).unwrap_or(0);
        eprintln!("rhythm debug: downbeat by bass {old}, now {downbeat}; bass {:?} changes {:?}", bass_share.map(|v| (v * 100.0).round()), change_share.map(|v| (v * 100.0).round()));
    }
    let (tonic, minor, kconf) = key(&x);
    // each bar's loudness, from the first downbeat on (four tracked beats to a bar), and how strong its
    // beat is: its onsets and its bass at its beats, against the song's usual (a median beat's: 1)
    let peak = |e: &[f64], f: usize| { let (a, b) = (f.saturating_sub(2), (f + 2).min(e.len() - 1)); (a..=b).map(|k| e[k]).fold(0.0, f64::max) };
    let median = |e: &[f64]| { let mut v: Vec<f64> = frames.iter().map(|&f| peak(e, f)).collect(); v.sort_by(|a, b| a.total_cmp(b)); v[v.len() / 2].max(1e-12) };
    let (mo, mb) = (median(&o), median(&bass));
    let mut bar_db = Vec::new();
    let mut bar_beat = Vec::new();
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
        bar_db,
        bar_beat,
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
pub const VERSION: u32 = 4;

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
