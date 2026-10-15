// Smart Transitions (docs/dj-transitions.md), in place of Seamless Mix v1: what DJ software does, and
// real DJs do (Kim et al., ISMIR 2020: 20,765 transitions; Mixxx's Auto DJ, read from its source).
//
//   Tier 0, every pair: the silences left out (the one playing goes out where its sound ends, the next
//           one comes in where its starts), and a crossfade over the one playing's outro and the next
//           one's intro (as long as the shorter of them, as Mixxx's Full Intro + Outro), found from the
//           bars' levels where they're measured; else a fixed length.
//   Tier 1, a beatmatched blend, only when it's safe: both songs' beats measured clearly (and their
//           tempo not contradicted by one written in their title or description: "100bpm"), their
//           tempos within 4% (DJs: 86% of changes under 5%), and their keys going together. Then it's
//           on phrases (32 beats), the bass swapped at once on a bar line, the rest crossfaded; never a
//           key shifted (DJs: 2.5% of tracks), no loops, no styles.
//   Tier 2, extras (a tape stop): only when switched on and your ratings back them.

use omnipulse_core::rhythm::Measured;

/// One side of a transition, as it's known
#[derive(Default)]
pub struct Side<'a> {
    /// how long it is (ms)
    pub dur_ms: u64,
    /// where its sound ends (the loudness analysis' audible_ms; 0 not known)
    pub audible_ms: u64,
    /// where its sound starts (after a silence it starts with; 0 none, or not known)
    pub first_ms: u64,
    /// its beat and bars, measured whole
    pub rhythm: Option<&'a Measured>,
    /// a tempo its title or description states ("100bpm"), if any
    pub stated_bpm: Option<f64>,
}

/// Tier 0: the one playing fades from `out_from_ms` to `out_to_ms` (where its sound ends); the next one
/// starts from `in_from_ms`, rising over the same time
#[derive(Clone, Debug, PartialEq)]
pub struct Fade {
    pub out_from_ms: u64,
    pub out_to_ms: u64,
    pub in_from_ms: u64,
}

impl Fade {
    pub fn len_ms(&self) -> u64 { self.out_to_ms.saturating_sub(self.out_from_ms) }
}

/// A fade's shortest (shorter: a cut, not a transition)
pub const MIN_FADE_MS: u64 = 1500;
/// A soft end or start shorter than this is none (the two loud parts together: a short overlap, HARD_MS)
const SOFT_MIN_MS: u64 = 3000;
const HARD_MS: u64 = 2000;
/// A bar "the song proper": within this of its usual loud bars' level (its 75th percentile)
const BODY_WITHIN_DB: f32 = 6.0;

/// Where a measured song's bars start (ms in the song): bar k's downbeat, from its tracked beats
fn bar_ms(m: &Measured, k: usize) -> Option<f64> {
    let r = &m.rhythm;
    r.beats.get(r.downbeat + 4 * k).map(|b| m.from_ms as f64 + b)
}

/// Its body's level: the song proper's bars at least this loud
fn body_line(m: &Measured) -> Option<f32> {
    let bars = &m.rhythm.bar_db;
    if bars.len() < 8 { return None; }
    let mut v = bars.clone();
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[v.len() * 3 / 4] - BODY_WITHIN_DB)
}

/// Its outro: from where its last stretch of the song proper ends (ms), if it has one before its sound
/// ends
pub fn outro_from(m: &Measured) -> Option<u64> {
    let line = body_line(m)?;
    let last = m.rhythm.bar_db.iter().rposition(|d| *d >= line)?;
    bar_ms(m, last + 1).map(|x| x as u64)
}

/// Its intro: up to where the song proper first comes in (ms)
pub fn intro_to(m: &Measured) -> Option<u64> {
    let line = body_line(m)?;
    let first = m.rhythm.bar_db.iter().position(|d| *d >= line)?;
    bar_ms(m, first).map(|x| x as u64)
}

/// Tier 0: over the outro or the intro (the longer of the two), at most `max_ms`; with neither
/// measured, `fixed_ms` (Mixxx's fixed fade); silences left out
pub fn fade(a: &Side, b: &Side, max_ms: u64, fixed_ms: u64) -> Fade {
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms };
    // (measured, a song with no soft end or start has one of 0: it ends cold, or starts at once)
    let outro = a.rhythm.map(|m| outro_from(m).map_or(0, |x| end.saturating_sub(x)));
    let intro = b.rhythm.map(|m| intro_to(m).map_or(0, |x| x.saturating_sub(b.first_ms)));
    // Over the longer of the two soft parts: the next one's loud start under the one playing's outro, or
    // its quiet intro rising under the one playing's last loud bars. (Not Mixxx's shorter one: its inner
    // points are set by hand, and found from levels, most songs' outros are a bar or so: a cut.) Both
    // hard (a cold end into a loud start): only a short overlap, the two loud parts barely together.
    let len = match (outro, intro) {
        (None, None) => fixed_ms.min(max_ms),
        (o, i) => {
            let (o, i) = (o.unwrap_or(fixed_ms), i.unwrap_or(fixed_ms));
            if o.max(i) < SOFT_MIN_MS { HARD_MS } else { o.max(i) }
        }
    };
    // (never more than a third of what's left of the one coming in, nor what there is of the one going out)
    let room_b = b.dur_ms.saturating_sub(b.first_ms) / 3;
    let len = len.clamp(MIN_FADE_MS, max_ms.max(MIN_FADE_MS)).min(end).min(if room_b > 0 { room_b } else { u64::MAX });
    Fade { out_from_ms: end.saturating_sub(len), out_to_ms: end, in_from_ms: b.first_ms }
}

// ---------- Tier 1: when a blend on the beat is safe ----------
/// A beat clear enough to trust (core::rhythm's tempo_confidence)
const TRUST_CONFIDENCE: f64 = 0.3;
/// How far from its own tempo the next one plays (DJs: mostly under 5%)
pub const MAX_RATE: f64 = 0.04;
/// A stated tempo agrees with the measured one within this (or half or double of it)
const STATED_WITHIN: f64 = 0.03;

/// A tempo written in a title or description: "100bpm", "174 BPM", "(128bpm)"
pub fn stated_bpm(text: &str) -> Option<f64> {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE.get_or_init(|| regex::Regex::new(r"(?i)\b(\d{2,3}(?:\.\d+)?)\s*bpm\b").unwrap());
    re.captures(text).and_then(|c| c[1].parse::<f64>().ok()).filter(|b| (50.0..=220.0).contains(b))
}

/// Its beat grid can be trusted: measured clearly, a tempo people play at, and not contradicted by one
/// it states (a grid at 4:3 of the real tempo, as Ozymandias' 133 against its 100, is worse than none)
pub fn grid_trusted(s: &Side) -> bool {
    let Some(m) = s.rhythm else { return false };
    let bpm = m.rhythm.bpm;
    if m.rhythm.tempo_confidence < TRUST_CONFIDENCE || !(60.0..=200.0).contains(&bpm) || m.rhythm.beats.len() < 32 { return false; }
    // (its bars sure too: a downbeat a beat out matches the tempo, but not the bars)
    if m.rhythm.downbeat_confidence < omnipulse_core::rhythm::DOWNBEAT_SURE { return false; }
    match s.stated_bpm {
        Some(st) => [1.0, 2.0, 0.5].iter().any(|k| ((bpm * k) / st - 1.0).abs() <= STATED_WITHIN),
        None => true,
    }
}

/// The rate the next one plays at to match the one playing (same tempo, or half or double time), if
/// it's within MAX_RATE
pub fn match_rate(a_bpm: f64, b_bpm: f64) -> Option<f64> {
    if a_bpm <= 0.0 || b_bpm <= 0.0 { return None; }
    [1.0, 2.0, 0.5].iter().map(|m| a_bpm / (b_bpm * m)).filter(|r| (r - 1.0).abs() <= MAX_RATE).min_by(|x, y| (x - 1.0).abs().total_cmp(&(y - 1.0).abs()))
}

/// Their keys go together as they are (the same, a step round the Camelot wheel, or relative major and
/// minor); a key not clear enough to say counts as going together
pub fn keys_go(a: &Measured, b: &Measured) -> bool {
    const CLEAR: f64 = 0.25;
    a.rhythm.key_confidence < CLEAR || b.rhythm.key_confidence < CLEAR || omnipulse_core::rhythm::keys_mix(&a.rhythm.camelot, &b.rhythm.camelot)
}

/// Why not a blend on the beat (None: it can be)
pub fn blend_blocked(a: &Side, b: &Side) -> Option<&'static str> {
    if !grid_trusted(a) { return Some("the one playing's beat isn't sure"); }
    if !grid_trusted(b) { return Some("the next one's beat isn't sure"); }
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    if match_rate(ma.rhythm.bpm, mb.rhythm.bpm).is_none() { return Some("tempos more than 4% apart"); }
    if !keys_go(ma, mb) { return Some("keys that don't go together"); }
    None
}

/// Tier 1: a blend on the beat (Seamless Mix v1's planner, kept to what DJs do: no key shifting, no
/// loops, no tape stops, on the beat only, the bass swapped on a bar line), or None (a fade instead)
pub fn blend(a: &Side, b: &Side, now_ms: u64) -> Option<crate::mix::Plan> {
    if blend_blocked(a, b).is_some() { return None; }
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    {
        use std::sync::atomic::Ordering::Relaxed;
        crate::mix::KEYS_OK.store(false, Relaxed);
        crate::mix::LOOPS_OK.store(false, Relaxed);
        crate::mix::TAPE_OK.store(false, Relaxed);
        crate::mix::FORCE_TAPE.store(false, Relaxed);
        crate::mix::BEAT_ONLY.store(true, Relaxed);
        crate::mix::RUN_BARS.store(0, Relaxed);
    }
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms };
    let p = crate::mix::plan_structure(ma, end, now_ms, mb)?;
    // (only a blend on the beat, at most 4% off, its key as it is)
    (p.beatmatched && !p.tape_stop && p.semitones == 0 && p.style.is_none() && p.out_loop.is_none() && p.in_loop.is_none() && (p.rate - 1.0).abs() <= MAX_RATE + 1e-9).then_some(p)
}

/// The handoff of a Tier 1 blend: the highs and mids crossfaded over it, the bass swapped at once on its
/// bar line (the two basses never together)
pub fn handoff(p: &crate::mix::Plan) -> omnipulse_audio::Handoff {
    omnipulse_audio::Handoff::bass_swap(p.ms, p.swap_ms)
}

// ---------- Tier 2: extras, as your ratings say ----------
/// A tape stop (v1's), for two songs too far apart to blend: only when its ratings back it, and never a
/// key shifted
pub fn tape(a: &Side, b: &Side, now_ms: u64) -> Option<crate::mix::Plan> {
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    {
        use std::sync::atomic::Ordering::Relaxed;
        crate::mix::KEYS_OK.store(false, Relaxed);
        crate::mix::LOOPS_OK.store(false, Relaxed);
        crate::mix::TAPE_OK.store(true, Relaxed);
        crate::mix::FORCE_TAPE.store(true, Relaxed);
        crate::mix::BEAT_ONLY.store(false, Relaxed);
        crate::mix::RUN_BARS.store(0, Relaxed);
    }
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms };
    let p = crate::mix::plan_structure(ma, end, now_ms, mb);
    crate::mix::FORCE_TAPE.store(false, std::sync::atomic::Ordering::Relaxed);
    p.filter(|p| p.tape_stop && p.semitones == 0)
}

/// An extra's ratings (state/mix-feedback.jsonl, "kind" as the app wrote it): enough of them (5), and
/// most of them (60%) good
pub fn backed(ratings: &[(String, bool)], kind: &str) -> bool {
    let of: Vec<bool> = ratings.iter().filter(|(k, _)| k == kind).map(|(_, g)| *g).collect();
    of.len() >= 5 && of.iter().filter(|g| **g).count() * 5 >= of.len() * 3
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnipulse_core::rhythm::{Measured, Rhythm};

    /// A song at `bpm` of `bars` bars (each its level), its first downbeat at 0
    fn song(bpm: f64, bars: &[f32], conf: f64, camelot: &str) -> Measured {
        let period = 60_000.0 / bpm;
        let beats: Vec<f64> = (0..bars.len() * 4 + 4).map(|i| i as f64 * period).collect();
        Measured { from_ms: 0, to_ms: (beats.len() as f64 * period) as u64, rhythm: Rhythm { bpm, first_beat_ms: 0.0, period_ms: period, beats, downbeat: 0, tempo_confidence: conf, key: String::new(), camelot: camelot.into(), key_confidence: 0.8, downbeat_confidence: 1.0, bar_db: bars.to_vec(), bar_beat: vec![1.0; bars.len()], bar_chroma: Vec::new() }, v: omnipulse_core::rhythm::VERSION }
    }

    fn levels(intro: usize, body: usize, outro: usize) -> Vec<f32> {
        std::iter::repeat_n(-20.0, intro).chain(std::iter::repeat_n(-8.0, body)).chain(std::iter::repeat_n(-20.0, outro)).collect()
    }

    #[test]
    fn a_fade_is_over_the_longer_soft_part_and_leaves_out_silences() {
        // 120 BPM: 2 s a bar. A: 4 bars of outro (8 s) before its sound ends; B: 2 bars of intro (4 s)
        let a = song(120.0, &levels(4, 40, 4), 0.6, "8A");
        let b = song(120.0, &levels(2, 40, 2), 0.6, "8A");
        let f = fade(&Side { dur_ms: 100_000, audible_ms: 96_000, rhythm: Some(&a), ..Default::default() }, &Side { dur_ms: 92_000, first_ms: 0, rhythm: Some(&b), ..Default::default() }, 12_000, 6_000);
        assert_eq!(f, Fade { out_from_ms: 88_000, out_to_ms: 96_000, in_from_ms: 0 });
        // a cold end into a loud start: a short overlap
        let (cold, hard) = (song(120.0, &levels(4, 44, 0), 0.6, "8A"), song(120.0, &levels(0, 44, 2), 0.6, "8A"));
        let f = fade(&Side { dur_ms: 96_000, rhythm: Some(&cold), ..Default::default() }, &Side { dur_ms: 92_000, rhythm: Some(&hard), ..Default::default() }, 12_000, 6_000);
        assert_eq!(f.len_ms(), 2_000);
        // nothing measured: the longest allowed, ending where the sound does, the next one past its silence
        let f = fade(&Side { dur_ms: 200_000, audible_ms: 190_000, ..Default::default() }, &Side { dur_ms: 180_000, first_ms: 1_200, ..Default::default() }, 12_000, 6_000);
        assert_eq!(f, Fade { out_from_ms: 184_000, out_to_ms: 190_000, in_from_ms: 1_200 });
    }

    #[test]
    fn a_blend_only_when_its_safe() {
        let a = song(128.0, &levels(8, 48, 8), 0.6, "8A");
        let near = song(126.0, &levels(8, 48, 8), 0.6, "9A");
        let far = song(118.0, &levels(8, 48, 8), 0.6, "8A");
        let clash = song(128.0, &levels(8, 48, 8), 0.6, "2B");
        let blurry = song(128.0, &levels(8, 48, 8), 0.1, "8A");
        let side = |m| Side { dur_ms: 150_000, rhythm: Some(m), ..Default::default() };
        assert_eq!(blend_blocked(&side(&a), &side(&near)), None);
        assert!(blend_blocked(&side(&a), &side(&far)).is_some());
        assert!(blend_blocked(&side(&a), &side(&clash)).is_some());
        assert!(blend_blocked(&side(&a), &side(&blurry)).is_some());
        // half time counts as the same tempo (a 64 BPM grid on a 128 song)
        assert!(match_rate(128.0, 64.0).is_some());
    }

    #[test]
    fn a_tempo_it_states_is_believed_over_a_grid_that_disagrees() {
        assert_eq!(stated_bpm("Hey guys, 100bpm midtempo self release"), Some(100.0));
        assert_eq!(stated_bpm("Strobe (Extended Mix) 128 BPM"), Some(128.0));
        assert_eq!(stated_bpm("no tempo here"), None);
        let ozy = song(133.0, &levels(8, 48, 8), 0.6, "5B");
        let s = |bpm| Side { dur_ms: 196_000, rhythm: Some(&ozy), stated_bpm: bpm, ..Default::default() };
        assert!(!grid_trusted(&s(Some(100.0))));
        assert!(grid_trusted(&s(Some(133.0))));
        assert!(grid_trusted(&s(None)));
    }

    #[test]
    fn extras_need_ratings_that_back_them() {
        let r = |good: usize, bad: usize| -> Vec<(String, bool)> { std::iter::repeat_n(("tape stop".to_string(), true), good).chain(std::iter::repeat_n(("tape stop".to_string(), false), bad)).collect() };
        assert!(!backed(&r(3, 12), "tape stop"));
        assert!(!backed(&r(3, 0), "tape stop"));
        assert!(backed(&r(4, 2), "tape stop"));
    }

    /// The offline renderer (step 3 of docs/dj-transitions.md): each pair as Smart Transitions would
    /// change it over, written to a WAV (from 12 s before it) with its plan beside it (a .json), to listen
    /// to and compare. Songs: SoundCloud ids, or paths to files.
    /// OMNIPULSE_SMART_PAIRS="111,222;333,C:/music/x.mp3" OMNIPULSE_SMART_OUT=renders cargo test --release -p omnipulse --bin OmniPulse-native smart_render -- --ignored --nocapture
    #[test]
    #[ignore]
    fn smart_render() {
        use crate::track::{Src, Track};
        let pairs = std::env::var("OMNIPULSE_SMART_PAIRS").expect("OMNIPULSE_SMART_PAIRS");
        let dir = std::path::PathBuf::from(std::env::var("OMNIPULSE_SMART_OUT").unwrap_or_else(|_| "smart-renders".into()));
        std::fs::create_dir_all(&dir).unwrap();
        let max_ms = std::env::var("OMNIPULSE_SMART_MAX").ok().and_then(|v| v.parse::<u64>().ok()).unwrap_or(10) * 1000;
        let rt = tokio::runtime::Runtime::new().unwrap();
        let track = |s: &str| match s.parse::<u64>() {
            Ok(id) => Track { src: Src::SoundCloud(id), key: s.to_string(), ..Default::default() },
            Err(_) => Track { src: Src::Local(s.to_string()), key: format!("local:{s}"), ..Default::default() },
        };
        let what = |t: &Track| match &t.src { Src::SoundCloud(id) => ("soundcloud", id.to_string()), Src::Local(p) => ("local", p.clone()), _ => unreachable!() };
        // (each measured whole, kept as the app keeps them; its loudness analysed: its silences, its end)
        let measure = |t: &Track| {
            let (kind, id) = what(t);
            let full = format!("full:{}", t.key);
            let m = omnipulse_core::rhythm::known(&full).filter(|m| !m.rhythm.bar_beat.is_empty()).or_else(|| {
                let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start(kind, &id, 1200.0)).ok()?;
                let heard = mono.len() as u64 * 1000 / rate.max(1) as u64;
                let m = Measured { from_ms: 0, to_ms: heard, rhythm: omnipulse_core::rhythm::analyse(&mono, rate)?, v: omnipulse_core::rhythm::VERSION };
                omnipulse_core::rhythm::keep(&full, m.clone());
                Some(m)
            });
            let an = rt.block_on(omnipulse_core::loudness::analyse(kind, &id)).unwrap_or_default();
            (m, an)
        };
        let rate = 44_100u32;
        let lead = 12_000u64;
        for (n, pair) in pairs.split(';').filter(|p| !p.trim().is_empty()).enumerate() {
            let (a_id, b_id) = pair.split_once(',').expect("a pair: A,B");
            let (ta, tb) = (track(a_id.trim()), track(b_id.trim()));
            let ((ma, ana), (mb, anb)) = (measure(&ta), measure(&tb));
            let open = |t: &Track| rt.block_on(crate::player::open_source(t, rt.handle().clone())).expect("it opens");
            let (mut sa, sb) = (open(&ta), open(&tb));
            let a_dur = sa.duration_ms().unwrap_or(0);
            let first = |an: &serde_json::Value| an["silences"].as_array().and_then(|s| s.first()).filter(|s| s[0].as_u64() == Some(0)).and_then(|s| s[1].as_u64()).unwrap_or(0);
            let a = Side { dur_ms: a_dur, audible_ms: ana["audible_ms"].as_u64().unwrap_or(0), first_ms: 0, rhythm: ma.as_ref(), stated_bpm: None };
            let b = Side { dur_ms: anb["duration_ms"].as_u64().unwrap_or(0), audible_ms: anb["audible_ms"].as_u64().unwrap_or(0), first_ms: first(&anb), rhythm: mb.as_ref(), stated_bpm: None };
            let db = |t: &Track, an: &serde_json::Value| crate::sound::loud_db(&serde_json::json!({}), t, an["lufs"].as_f64(), false);
            let dbs = (db(&ta, &ana), db(&tb, &anb));
            let blocked = blend_blocked(&a, &b);
            let plan = blend(&a, &b, 0);
            let (out, info) = match &plan {
                Some(p) => {
                    sa.seek(p.at_ms.saturating_sub(lead)).unwrap();
                    let b_tempo = omnipulse_audio::Tempo::new(p.rate);
                    let mut sb = omnipulse_audio::Stretched::shifted(sb, b_tempo.clone(), 0).unwrap();
                    omnipulse_audio::Source::seek(&mut sb, p.start_ms).unwrap();
                    let (target, a_tempo) = (p.out_tempo, omnipulse_audio::Tempo::new(1.0));
                    let share = p.ms as f64 / p.ramp_ms.max(1) as f64;
                    let ramp = { let a_tempo = a_tempo.clone(); move |x: f64| { a_tempo.set(crate::mix::out_ramp(target, x * share)); b_tempo.set(crate::mix::in_ramp(target, x * share)); } };
                    let out = omnipulse_audio::engine::render_mix_ramped(sa, Box::new(sb), dbs, rate, lead as f32 / 1000.0, p.ms, Some(handoff(p)), (target != 1.0).then_some((&a_tempo, &ramp as &dyn Fn(f64))), (lead + p.ms) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "smart blend", "plan": p }))
                }
                None => {
                    let f = fade(&a, &b, max_ms, FIXED_MS);
                    sa.seek(f.out_from_ms.saturating_sub(lead)).unwrap();
                    let mut sb = sb;
                    if f.in_from_ms > 0 { sb.seek(f.in_from_ms).unwrap(); }
                    let out = omnipulse_audio::engine::render_mix(sa, sb, dbs, rate, lead as f32 / 1000.0, f.len_ms(), None, (lead + f.len_ms()) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "smart fade", "outFromMs": f.out_from_ms, "outToMs": f.out_to_ms, "inFromMs": f.in_from_ms, "ms": f.len_ms(), "notOnTheBeat": blocked.unwrap_or("no phrase to blend on"),
                        "outroFromMs": ma.as_ref().and_then(outro_from), "introToMs": mb.as_ref().and_then(intro_to) }))
                }
            };
            let side = |m: &Option<Measured>| m.as_ref().map(|m| serde_json::json!({ "bpm": m.rhythm.bpm, "confidence": m.rhythm.tempo_confidence, "camelot": m.rhythm.camelot, "keyConfidence": m.rhythm.key_confidence }));
            let info = serde_json::json!({ "a": a_id, "b": b_id, "aMeasured": side(&ma), "bMeasured": side(&mb), "dbs": [dbs.0, dbs.1], "startsAt": "12 s in", "transition": info });
            let base = dir.join(format!("{:02}", n + 1));
            write_wav(&base.with_extension("wav"), &out, rate);
            std::fs::write(base.with_extension("json"), serde_json::to_string_pretty(&info).unwrap()).unwrap();
            println!("{}: {}", base.display(), info["transition"]);
        }
    }

    /// The fallback fade the app uses (main.rs' SMART_FIXED_MS)
    const FIXED_MS: u64 = 6000;

    /// A 16-bit stereo WAV
    fn write_wav(path: &std::path::Path, out: &[f32], rate: u32) {
        let mut w = Vec::new();
        let data = (out.len() * 2) as u32;
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] { w.extend_from_slice(&v.to_le_bytes()); }
        for v in [rate, rate * 4] { w.extend_from_slice(&v.to_le_bytes()); }
        for v in [4u16, 16] { w.extend_from_slice(&v.to_le_bytes()); }
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data.to_le_bytes());
        for s in out { w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); }
        std::fs::write(path, w).unwrap();
    }
}
