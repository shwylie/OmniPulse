// A song's key, the way KeyFinder (Mixxx's key detector) reads one, rather than core::rhythm's first
// reading (each note's loudest FFT bin, against Krumhansl's profiles), which put songs a fifth out
// often enough to make a key shift worse: a note's third harmonic is its fifth, so overtones, bass and
// drums all pulled it. Here: each note's energy over its own band (a semitone wide, from 55 Hz to
// 1.76 kHz), the track's tuning found first (off A440 by up to half a semitone), each frame whitened
// (the spectrum's tilt taken out), what's held over time kept and what's struck (drums) let go, each
// note's overtones taken off the notes above it, then the notes against major and minor profiles.
// Each step can be switched off (KeyOpts), to score them against keys known from outside (key_eval).

use realfft::RealFftPlanner;

/// What it's given: mono at this rate (core::rhythm's), taken down to half for the analysis
pub const RATE: u32 = 22_050;
const HALF: u32 = RATE / 2;
const WIN: usize = 8192;
const HOP: usize = 4096;
/// the notes looked at (MIDI): C2 (65 Hz) to C6 (1 kHz). (Scored against 117 of the user's songs'
/// keys from SongBPM, 2026-10-17: 48.7% the key, a fifth out 12.8%, against 44.4% and 17.1% for the
/// first reading; A1 to A6, 47.9%. SongBPM's own keys are often a fifth or the relative out too, so
/// none of this is the whole truth.)
const LOW: i32 = 36;
const HIGH: i32 = 84;

/// Krumhansl-Kessler's profiles (core::rhythm's), and Temperley's
pub const KK: ([f64; 12], [f64; 12]) = ([7.24, 3.50, 3.58, 2.85, 5.82, 4.56, 2.45, 6.99, 3.39, 4.56, 4.07, 4.46], [7.00, 3.14, 4.36, 5.40, 3.67, 4.09, 3.91, 6.20, 3.63, 2.87, 5.35, 3.83]);
pub const TEMPERLEY: ([f64; 12], [f64; 12]) = ([5.0, 2.0, 3.5, 2.0, 4.5, 4.0, 2.0, 4.5, 2.0, 3.5, 1.5, 4.0], [5.0, 2.0, 3.5, 4.5, 2.0, 4.0, 2.0, 4.5, 3.5, 2.0, 1.5, 4.0]);

#[derive(Clone, Debug)]
pub struct KeyOpts {
    pub tuning: bool,
    pub whiten: bool,
    /// held notes kept, struck ones (drums) let go
    pub harmonic: bool,
    /// each note's overtones taken off the notes an octave, a twelfth and two octaves above it
    pub overtones: bool,
    /// the notes below C3 counted this many times over (a bass line names its key)
    pub bass: f64,
    pub profiles: ([f64; 12], [f64; 12]),
    /// only its main part: frames within 12 dB of its loud ones (not a quiet intro or breakdown)
    pub body_only: bool,
    /// the notes looked at (MIDI, from, to)
    pub range: (i32, i32),
}

impl Default for KeyOpts {
    fn default() -> Self {
        Self { tuning: true, whiten: true, harmonic: true, overtones: true, bass: 1.0, profiles: KK, body_only: false, range: (LOW, HIGH) }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct KeyReading {
    pub tonic: usize,
    pub minor: bool,
    /// how far the best key is ahead of the next (0..1)
    pub confidence: f64,
    /// the notes, summed over the song (each of 12, summing to 1)
    pub chroma: [f64; 12],
}

/// The best key for a chroma, by `profiles` (major, minor): tonic, minor, its score, the next one's
pub fn best_key(chroma: &[f64; 12], profiles: &([f64; 12], [f64; 12])) -> (usize, bool, f64, f64) {
    let corr = |profile: &[f64; 12], tonic: usize| {
        let p: Vec<f64> = (0..12).map(|i| profile[(i + 12 - tonic) % 12]).collect();
        let (mx, mp) = (chroma.iter().sum::<f64>() / 12.0, p.iter().sum::<f64>() / 12.0);
        let num: f64 = (0..12).map(|i| (chroma[i] - mx) * (p[i] - mp)).sum();
        let den = ((0..12).map(|i| (chroma[i] - mx).powi(2)).sum::<f64>() * (0..12).map(|i| (p[i] - mp).powi(2)).sum::<f64>()).sqrt();
        if den > 0.0 { num / den } else { 0.0 }
    };
    let mut all: Vec<(f64, usize, bool)> = Vec::with_capacity(24);
    for t in 0..12 {
        all.push((corr(&profiles.0, t), t, false));
        all.push((corr(&profiles.1, t), t, true));
    }
    all.sort_by(|a, b| b.0.total_cmp(&a.0));
    (all[0].1, all[0].2, all[0].0, all[1].0)
}

/// Each frame's note energies (LOW..HIGH), the tuning applied
fn notes(x: &[f32], o: &KeyOpts) -> Vec<Vec<f64>> {
    // (down to half the rate: two samples averaged, enough below 1.76 kHz)
    let y: Vec<f32> = x.chunks_exact(2).map(|p| (p[0] + p[1]) * 0.5).collect();
    if y.len() < WIN { return Vec::new(); }
    let mut planner = RealFftPlanner::<f32>::new();
    let fft = planner.plan_fft_forward(WIN);
    let w: Vec<f32> = (0..WIN).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WIN as f32).cos()).collect();
    let (mut input, mut spec) = (fft.make_input_vec(), fft.make_output_vec());
    let mut mags: Vec<Vec<f32>> = Vec::new();
    let mut f = 0;
    while f + WIN <= y.len() {
        for i in 0..WIN { input[i] = y[f + i] * w[i]; }
        if fft.process(&mut input, &mut spec).is_err() { break; }
        mags.push(spec.iter().map(|c| c.norm()).collect());
        f += HOP;
    }
    let bin_hz = HALF as f64 / WIN as f64;
    // a note's band: its bins within half a semitone, weighted by a raised cosine on the distance
    let band = |midi: f64| -> Vec<(usize, f64)> {
        let fc = 440.0 * 2f64.powf((midi - 69.0) / 12.0);
        let (lo, hi) = (fc * 2f64.powf(-0.5 / 12.0), fc * 2f64.powf(0.5 / 12.0));
        let (a, b) = ((lo / bin_hz).floor().max(1.0) as usize, (hi / bin_hz).ceil() as usize);
        (a..=b.min(WIN / 2)).filter_map(|k| {
            let d = 12.0 * ((k as f64 * bin_hz) / fc).log2();
            (d.abs() <= 0.5).then(|| (k, (std::f64::consts::PI * d).cos().powi(2)))
        }).collect()
    };
    // the tuning: of tenths of a semitone either side, where the notes' energy sits most
    let tune = if o.tuning {
        let sample: Vec<&Vec<f32>> = mags.iter().step_by((mags.len() / 60).max(1)).collect();
        (-5..=5).map(|t| t as f64 / 10.0).map(|t| {
            let e: f64 = (o.range.0..o.range.1).map(|n| band(n as f64 + t)).map(|b| sample.iter().map(|m| b.iter().map(|(k, w)| m[*k] as f64 * w).sum::<f64>()).sum::<f64>()).sum();
            (t, e)
        }).max_by(|a, b| a.1.total_cmp(&b.1)).map(|x| x.0).unwrap_or(0.0)
    } else { 0.0 };
    let bands: Vec<Vec<(usize, f64)>> = (o.range.0..o.range.1).map(|n| band(n as f64 + tune)).collect();
    mags.iter().map(|m| bands.iter().map(|b| b.iter().map(|(k, w)| m[*k] as f64 * w).sum::<f64>()).collect()).collect()
}

fn median(v: &mut [f64]) -> f64 {
    if v.is_empty() { return 0.0; }
    v.sort_by(|a, b| a.total_cmp(b));
    v[v.len() / 2]
}

/// The key of mono samples at RATE
pub fn estimate(x: &[f32], o: &KeyOpts) -> Option<KeyReading> {
    let mut e = notes(x, o);
    if e.is_empty() { return None; }
    // (its main part only: the frames within 12 dB of its loud ones (their 75th percentile))
    if o.body_only {
        let level: Vec<f64> = e.iter().map(|fr| fr.iter().sum::<f64>()).collect();
        let mut sorted = level.clone();
        sorted.sort_by(|a, b| a.total_cmp(b));
        let line = sorted[sorted.len() * 3 / 4] * 0.25;
        let kept: Vec<Vec<f64>> = e.iter().zip(&level).filter(|(_, l)| **l >= line).map(|(fr, _)| fr.clone()).collect();
        if kept.len() >= 8 { e = kept; }
    }
    let (frames, n) = (e.len(), e[0].len());
    // (held notes kept, struck ones let go: a note's level over ~4 s against its frame's over nearby
    // notes, as a harmonic/percussive split does)
    if o.harmonic {
        let mut h = vec![vec![0.0; n]; frames];
        let mut p = vec![vec![0.0; n]; frames];
        for t in 0..frames {
            for k in 0..n {
                let mut a: Vec<f64> = (t.saturating_sub(5)..(t + 6).min(frames)).map(|u| e[u][k]).collect();
                h[t][k] = median(&mut a);
                let mut b: Vec<f64> = (k.saturating_sub(4)..(k + 5).min(n)).map(|j| e[t][j]).collect();
                p[t][k] = median(&mut b);
            }
        }
        for t in 0..frames { for k in 0..n {
            let (hh, pp) = (h[t][k] * h[t][k], p[t][k] * p[t][k]);
            e[t][k] *= if hh + pp > 0.0 { hh / (hh + pp) } else { 0.0 };
        } }
    }
    // (the spectrum's tilt taken out: each note over its neighbours' average, an octave wide)
    if o.whiten {
        for fr in e.iter_mut() {
            let env: Vec<f64> = (0..n).map(|k| { let (a, b) = (k.saturating_sub(6), (k + 7).min(n)); fr[a..b].iter().sum::<f64>() / (b - a) as f64 }).collect();
            for k in 0..n { fr[k] = if env[k] > 0.0 { (fr[k] / env[k] - 1.0).max(0.0) } else { 0.0 }; }
        }
    }
    // (a note's overtones taken off the notes above it: an octave, an octave and a fifth, two octaves)
    if o.overtones {
        for fr in e.iter_mut() {
            for k in 0..n {
                let v = fr[k];
                for (up, share) in [(12usize, 0.5), (19, 0.33), (24, 0.25)] {
                    if k + up < n { fr[k + up] = (fr[k + up] - share * v).max(0.0); }
                }
            }
        }
    }
    let mut chroma = [0.0f64; 12];
    for fr in &e {
        for k in 0..n {
            let midi = o.range.0 as usize + k;
            chroma[midi % 12] += fr[k] * if midi < 48 { o.bass } else { 1.0 };
        }
    }
    let sum: f64 = chroma.iter().sum();
    if sum <= 0.0 { return None; }
    for c in &mut chroma { *c /= sum; }
    let (tonic, minor, best, second) = best_key(&chroma, &o.profiles);
    Some(KeyReading { tonic, minor, confidence: ((best - second) * 4.0).clamp(0.0, 1.0), chroma })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A chord held (its notes, a few of their overtones) over a kick, at RATE
    fn chords(notes: &[f64], secs: f64) -> Vec<f32> {
        let n = (RATE as f64 * secs) as usize;
        (0..n).map(|i| {
            let t = i as f64 / RATE as f64;
            let tone: f64 = notes.iter().map(|&m| { let f = 440.0 * 2f64.powf((m - 69.0) / 12.0); (1..=4).map(|h| (std::f64::consts::TAU * f * h as f64 * t).sin() / h as f64).sum::<f64>() }).sum();
            let beat = t % 0.5;
            let kick = (-beat * 30.0).exp() * (std::f64::consts::TAU * (60.0 - 40.0 * beat) * beat).sin() * 2.0;
            ((tone * 0.05) + kick * 0.3) as f32
        }).collect()
    }

    #[test]
    fn a_minor_chord_over_a_kick_is_its_key() {
        // A minor (A2, C3, E3, A3) then its iv and v (D minor, E minor), A minor again
        let mut x = chords(&[45.0, 48.0, 52.0, 57.0], 6.0);
        x.extend(chords(&[50.0, 53.0, 57.0], 3.0));
        x.extend(chords(&[52.0, 55.0, 59.0], 3.0));
        x.extend(chords(&[45.0, 48.0, 52.0, 57.0], 6.0));
        let k = estimate(&x, &KeyOpts::default()).unwrap();
        assert_eq!((k.tonic, k.minor), (9, true), "{k:?}");
    }

    /// Our readings scored against keys known from outside (SongBPM), for songs of yours: the YouTube
    /// Music versions kept (state/yt-versions.json), each looked up, then decoded once (kept in
    /// OMNIPULSE_KEY_EVAL_DIR as 16-bit mono at RATE) and read every way. OMNIPULSE_KEY_EVAL_N: how many
    /// with a key found (120).
    /// OMNIPULSE_KEY_EVAL_DIR=... cargo test --release -p omnipulse-core key_eval -- --ignored --nocapture
    #[test]
    #[ignore]
    fn key_eval() {
        use serde_json::{json, Value};
        let dir = std::path::PathBuf::from(std::env::var("OMNIPULSE_KEY_EVAL_DIR").expect("OMNIPULSE_KEY_EVAL_DIR"));
        std::fs::create_dir_all(&dir).unwrap();
        let want: usize = std::env::var("OMNIPULSE_KEY_EVAL_N").ok().and_then(|v| v.parse().ok()).unwrap_or(120);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let versions: serde_json::Map<String, Value> = serde_json::from_str(&std::fs::read_to_string(crate::data_dir().unwrap().join("state").join("yt-versions.json")).unwrap()).unwrap();
        let truth_file = dir.join("truth.json");
        let mut truth: serde_json::Map<String, Value> = std::fs::read_to_string(&truth_file).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut songs: Vec<(String, String, usize, bool)> = Vec::new();
        for v in versions.values() {
            if songs.len() >= want { break; }
            let (Some(id), Some(title), Some(artists)) = (v["id"].as_str(), v["title"].as_str(), v["artists"].as_str()) else { continue };
            if songs.iter().any(|s| s.0 == id) { continue; }
            if !truth.contains_key(id) {
                let artist = artists.split(',').next().unwrap_or("").trim();
                let title = crate::ranking::featured(title).1;
                let found = rt.block_on(crate::songbpm::lookup(artist, &title, v["duration_ms"].as_u64().unwrap_or(0)));
                let Ok(found) = found else { continue };
                truth.insert(id.into(), match found { Some(k) => json!({ "tonic": k.tonic, "minor": k.minor, "url": k.url, "name": format!("{artist} - {title}") }), None => json!({ "none": true }) });
                std::fs::write(&truth_file, serde_json::to_string_pretty(&truth).unwrap()).unwrap();
                std::thread::sleep(std::time::Duration::from_millis(300));
            }
            let t = &truth[id];
            if let (Some(tonic), Some(minor)) = (t["tonic"].as_u64(), t["minor"].as_bool()) { songs.push((id.into(), t["name"].as_str().unwrap_or("").into(), tonic as usize, minor)); }
        }
        println!("{} songs with a key from SongBPM", songs.len());
        // (each decoded once)
        let audio = |id: &str| -> Option<Vec<f32>> {
            let p = dir.join(format!("{id}.pcm"));
            if let Ok(b) = std::fs::read(&p) { return Some(b.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0).collect()); }
            let (mono, rate) = rt.block_on(crate::loudness::mono_start("youtube", id, 1200.0)).ok()?;
            let x = crate::rhythm::to_analysis_rate(&mono, rate);
            let b: Vec<u8> = x.iter().flat_map(|v| ((v.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()).collect();
            std::fs::write(&p, b).ok()?;
            Some(x)
        };
        let names = ["C", "C#", "D", "Eb", "E", "F", "F#", "G", "Ab", "A", "Bb", "B"];
        let k = |t: usize, m: bool| format!("{}{}", names[t % 12], if m { "m" } else { "" });
        let camelot = |t: usize, m: bool| crate::rhythm::camelot(t, m);
        // (how a reading scores: the key; its tonic; a key beside it on the wheel (mixes); a fifth out)
        #[derive(Default, Clone)]
        struct Score { n: usize, exact: usize, tonic: usize, wheel: usize, fifth: usize }
        fn tally(s: &mut Score, got: (usize, bool), want: (usize, bool)) {
            let camelot = |t: usize, m: bool| crate::rhythm::camelot(t, m);
            s.n += 1;
            if got == want { s.exact += 1; }
            if got.0 == want.0 { s.tonic += 1; }
            if crate::rhythm::keys_mix(&camelot(got.0, got.1), &camelot(want.0, want.1)) { s.wheel += 1; }
            if got.1 == want.1 && ((got.0 + 7) % 12 == want.0 || (want.0 + 7) % 12 == got.0) { s.fifth += 1; }
        }
        let _ = camelot;
        let mut scores: Vec<(String, Score)> = Vec::new();
        let mut add = |name: &str, got: (usize, bool), want: (usize, bool)| {
            let i = scores.iter().position(|s| s.0 == name).unwrap_or_else(|| { scores.push((name.into(), Score::default())); scores.len() - 1 });
            tally(&mut scores[i].1, got, want);
        };
        let variants: Vec<(&str, KeyOpts)> = vec![
            ("new", KeyOpts::default()),
            ("new, no tuning", KeyOpts { tuning: false, ..Default::default() }),
            ("new, no whitening", KeyOpts { whiten: false, ..Default::default() }),
            ("new, drums kept", KeyOpts { harmonic: false, ..Default::default() }),
            ("new, overtones kept", KeyOpts { overtones: false, ..Default::default() }),
            ("new, bass x2", KeyOpts { bass: 2.0, ..Default::default() }),
            ("new, Temperley", KeyOpts { profiles: TEMPERLEY, ..Default::default() }),
            ("new, none of it", KeyOpts { tuning: false, whiten: false, harmonic: false, overtones: false, ..Default::default() }),
            ("new, main part only", KeyOpts { body_only: true, ..Default::default() }),
            ("new, C2-C6", KeyOpts { range: (36, 84), ..Default::default() }),
            ("new, C3-C6", KeyOpts { range: (48, 84), ..Default::default() }),
            ("new, main part, C2-C6", KeyOpts { body_only: true, range: (36, 84), ..Default::default() }),
        ];
        let mut chromas: Vec<([f64; 12], usize, bool)> = Vec::new();
        let mut lines = Vec::new();
        for (i, (id, name, tonic, minor)) in songs.iter().enumerate() {
            let Some(x) = audio(id) else { println!("{name}: couldn't be decoded"); continue };
            let want = (*tonic, *minor);
            let first = crate::rhythm::key_first_reading(&x, RATE);
            add("first reading (now)", (first.0, first.1), want);
            let mut line = format!("{:>3} {name}: {} | now {}", i + 1, k(want.0, want.1), k(first.0, first.1));
            for (vname, o) in &variants {
                if let Some(r) = estimate(&x, o) {
                    add(vname, (r.tonic, r.minor), want);
                    if *vname == "new" { line += &format!(" | new {} ({:.2})", k(r.tonic, r.minor), r.confidence); chromas.push((r.chroma, *tonic, *minor)); }
                }
            }
            println!("{line}");
            lines.push(line);
        }
        // profiles learned from the others (each song left out of its own): the notes of each key, as heard
        let mut learned = Score::default();
        for (i, (c, t, m)) in chromas.iter().enumerate() {
            let mut prof = ([0.0f64; 12], [0.0f64; 12]);
            for (j, (c2, t2, m2)) in chromas.iter().enumerate() {
                if i == j { continue; }
                let p = if *m2 { &mut prof.1 } else { &mut prof.0 };
                for q in 0..12 { p[q] += c2[(q + t2) % 12]; }
            }
            let (bt, bm, _, _) = best_key(c, &prof);
            tally(&mut learned, (bt, bm), (*t, *m));
        }
        scores.push(("new, profiles learned (left out)".into(), learned));
        println!();
        let mut report = Vec::new();
        for (name, s) in &scores {
            let pct = |x: usize| 100.0 * x as f64 / s.n.max(1) as f64;
            let l = format!("{name:>34}: key {:>5.1}%  tonic {:>5.1}%  beside it {:>5.1}%  a fifth out {:>5.1}%  (of {})", pct(s.exact), pct(s.tonic), pct(s.wheel), pct(s.fifth), s.n);
            println!("{l}");
            report.push(l);
        }
        std::fs::write(dir.join("report.txt"), format!("{}\n\n{}", lines.join("\n"), report.join("\n"))).unwrap();
    }
}
