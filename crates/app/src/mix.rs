// Seamless mixing (Fuser's idea, a DJ's way): when the track playing and the next one both have a beat
// measured (core::rhythm), the next one comes in on a downbeat of the one playing, at its tempo (its
// pitch kept: crate::stretch's keylock), their bass handed over at once halfway (a bass swap), over a
// few bars that end before the one playing's sound does; then it eases back to its own tempo.
// Only for tracks played from YouTube Music, a plugin or your files: never Spotify's own playback.

use omnipulse_core::rhythm::Measured;

/// How far from its own tempo a track is played to match (8%: a DJ deck's usual range)
const MAX_STRETCH: f64 = 0.08;
/// A beat that's not clear enough isn't mixed on
const MIN_CONFIDENCE: f64 = 0.15;

#[derive(Clone, Debug, PartialEq)]
pub struct Plan {
    /// where in the one playing it starts (a downbeat, ms)
    pub at_ms: u64,
    /// how long it takes (whole bars of the one playing)
    pub ms: u64,
    /// where in it the bass is handed over (a bar boundary)
    pub swap_ms: u64,
    /// where in the next one it starts from (its first downbeat)
    pub start_ms: u64,
    /// the next one's tempo meanwhile (1.0 its own), then eased back
    pub rate: f64,
    pub bars: u32,
    /// its key shifted by this many semitones (for the whole track), to one that goes with the
    /// playing one's (0: as it is)
    pub semitones: i32,
}

/// A key not clear enough isn't shifted for
const MIN_KEY_CONFIDENCE: f64 = 0.1;

/// The smallest shift (up to two semitones) that puts `b`'s key next to `a`'s on the Camelot wheel;
/// 0 when they go together already, or either key isn't clear
pub fn key_shift(a: &Measured, b: &Measured) -> i32 {
    use omnipulse_core::rhythm::{keys_mix, transposed};
    if a.rhythm.key_confidence < MIN_KEY_CONFIDENCE || b.rhythm.key_confidence < MIN_KEY_CONFIDENCE || keys_mix(&a.rhythm.camelot, &b.rhythm.camelot) { return 0; }
    for s in [1, -1, 2, -2] {
        if transposed(&b.rhythm.key, s).is_some_and(|(_, c)| keys_mix(&a.rhythm.camelot, &c)) { return s; }
    }
    0
}

/// A downbeat's time in a measured track (ms from its start): its first tracked downbeat
fn first_downbeat(m: &Measured) -> Option<f64> {
    m.rhythm.beats.get(m.rhythm.downbeat).map(|b| m.from_ms as f64 + b)
}

/// The tempo the next one plays at to match (and how many of its beats to one of the playing one's)
fn match_rate(a_bpm: f64, b_bpm: f64) -> Option<(f64, f64)> {
    if a_bpm <= 0.0 || b_bpm <= 0.0 { return None; }
    // (the same tempo, or half or double time)
    [1.0, 2.0, 0.5].iter().map(|m| (a_bpm / (b_bpm * m), *m)).filter(|(r, _)| (r - 1.0).abs() <= MAX_STRETCH).min_by(|x, y| (x.0 - 1.0).abs().total_cmp(&(y.0 - 1.0).abs()))
}

/// The plan for mixing `b` in after `a` (`a_end_ms`: where `a`'s sound ends; `now_ms`: where it is),
/// over `bars` bars, or None (no clear beat, tempos too far apart, no room left)
pub fn plan(a: &Measured, a_end_ms: u64, now_ms: u64, b: &Measured, bars: u32) -> Option<Plan> {
    if a.rhythm.tempo_confidence < MIN_CONFIDENCE || b.rhythm.tempo_confidence < MIN_CONFIDENCE { return None; }
    let (rate, _per) = match_rate(a.rhythm.bpm, b.rhythm.bpm)?;
    let bar = 4.0 * a.rhythm.period_ms;
    let first = first_downbeat(a)?;
    let len = bar * bars as f64;
    // the latest downbeat that leaves room before the sound ends, a phrase's start if it can be
    // (every 8 bars, else every 4, else any bar), and far enough ahead to get the next one ready
    let last_k = ((a_end_ms as f64 - 500.0 - len - first) / bar).floor();
    if last_k < 0.0 { return None; }
    let soonest = now_ms as f64 + 4000.0;
    let pick = |every: i64| (0..=last_k as i64).rev().find(|k| k % every == 0 && first + *k as f64 * bar >= soonest);
    let k = pick(8).or_else(|| pick(4)).or_else(|| pick(1))?;
    let at = first + k as f64 * bar;
    Some(Plan {
        at_ms: at.round() as u64,
        ms: len.round() as u64,
        swap_ms: (bar * (bars / 2) as f64).round() as u64,
        start_ms: first_downbeat(b)?.round() as u64,
        rate,
        bars,
        semitones: key_shift(a, b),
    })
}

/// How well `b` mixes after `a`: 2 its tempo and key both go, 1 its tempo, 0 neither (or not measured)
pub fn fit(a: &Measured, b: Option<&Measured>) -> u8 {
    let Some(b) = b else { return 0 };
    if match_rate(a.rhythm.bpm, b.rhythm.bpm).is_none() { return 0; }
    if omnipulse_core::rhythm::keys_mix(&a.rhythm.camelot, &b.rhythm.camelot) { 2 } else { 1 }
}

/// A fresh measurement of a bit of the track (near its end), lined up with what was measured of its
/// start: the same bars (its downbeats where the first measurement's grid has them), the tempo and beats
/// as they are there. None if they don't agree (another tempo there: the first measurement stands).
pub fn relined(first: &Measured, local: &Measured) -> Option<Measured> {
    let (a, b) = (first.rhythm.bpm, local.rhythm.bpm);
    if a <= 0.0 || (b / a - 1.0).abs() > 0.02 { return None; }
    let down = first_downbeat(first)?;
    let bar = 4.0 * first.rhythm.period_ms;
    // the local beat nearest a downbeat of the first grid: that's a downbeat
    let (idx, off) = local.rhythm.beats.iter().enumerate().map(|(i, t)| {
        let at = local.from_ms as f64 + t;
        let k = ((at - down) / bar).round();
        (i, (at - (down + k * bar)).abs())
    }).min_by(|x, y| x.1.total_cmp(&y.1))?;
    // (more than half a beat from any of its downbeats: they don't line up)
    if off > first.rhythm.period_ms / 2.0 { return None; }
    let mut m = local.clone();
    m.rhythm.downbeat = idx % 4;
    Some(m)
}

/// The tempo while easing back after the mix: from `rate` to 1.0 over `ms`, `t` ms in (smoothly)
pub fn ease(rate: f64, t: u64, ms: u64) -> f64 {
    if t >= ms { return 1.0; }
    let p = t as f64 / ms.max(1) as f64;
    let s = p * p * (3.0 - 2.0 * p);
    rate + (1.0 - rate) * s
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnipulse_core::rhythm::{Rhythm, VERSION};

    fn m(bpm: f64, first_beat: f64, downbeat: usize, conf: f64) -> Measured {
        let period = 60_000.0 / bpm;
        let beats: Vec<f64> = (0..64).map(|i| first_beat + i as f64 * period).collect();
        Measured { from_ms: 0, to_ms: 60_000, rhythm: Rhythm { bpm, first_beat_ms: first_beat, period_ms: period, beats, downbeat, tempo_confidence: conf, key: "A minor".into(), camelot: "8A".into(), key_confidence: 0.5 }, v: VERSION }
    }

    #[test]
    fn on_a_phrase_before_the_end() {
        // 128 BPM (a bar is 1875 ms), the first downbeat at 500 ms; the sound ends at 200 s
        let a = m(128.0, 500.0, 0, 0.5);
        let b = m(126.0, 300.0, 1, 0.5);
        let p = plan(&a, 200_000, 60_000, &b, 8).expect("a plan");
        let bar = 1875.0;
        // on a downbeat of a phrase (every 8 bars), its 8 bars over before the end
        let k = ((p.at_ms as f64 - 500.0) / bar).round() as i64;
        assert_eq!(k % 8, 0);
        assert!((p.at_ms as f64 - (500.0 + k as f64 * bar)).abs() < 1.0);
        assert!(p.at_ms + p.ms <= 200_000 - 500, "{p:?}");
        // the latest one that fits
        assert!(p.at_ms as f64 + 8.0 * bar + p.ms as f64 > 199_500.0, "{p:?}");
        // the next one starts from its first downbeat (its second beat here), at the playing one's tempo
        assert_eq!(p.start_ms, (300.0f64 + 60_000.0 / 126.0).round() as u64);
        assert!((p.rate - 128.0 / 126.0).abs() < 1e-9);
        assert_eq!(p.swap_ms, (4.0 * bar) as u64);
    }

    #[test]
    fn tempos_too_far_apart_or_no_beat() {
        assert!(plan(&m(128.0, 0.0, 0, 0.5), 200_000, 0, &m(100.0, 0.0, 0, 0.5), 8).is_none());
        assert!(plan(&m(128.0, 0.0, 0, 0.05), 200_000, 0, &m(128.0, 0.0, 0, 0.5), 8).is_none());
        // half time is fine: 64 against 128
        assert!(plan(&m(128.0, 0.0, 0, 0.5), 200_000, 0, &m(64.5, 0.0, 0, 0.5), 8).is_some());
        // no room left
        assert!(plan(&m(128.0, 0.0, 0, 0.5), 200_000, 190_000, &m(128.0, 0.0, 0, 0.5), 8).is_none());
    }

    #[test]
    fn keys_shifted_to_go_together() {
        let mut a = m(128.0, 0.0, 0, 0.5);
        let mut b = m(128.0, 0.0, 0, 0.5);
        (a.rhythm.key, a.rhythm.camelot) = ("A minor".into(), "8A".into());
        // B minor (10A) is two steps away: down two semitones is A minor
        (b.rhythm.key, b.rhythm.camelot) = ("B minor".into(), "10A".into());
        assert_eq!(key_shift(&a, &b), -2);
        // E minor (9A) is next to it already
        (b.rhythm.key, b.rhythm.camelot) = ("E minor".into(), "9A".into());
        assert_eq!(key_shift(&a, &b), 0);
        // a key not clear enough: left as it is
        (b.rhythm.key, b.rhythm.camelot, b.rhythm.key_confidence) = ("B minor".into(), "10A".into(), 0.02);
        assert_eq!(key_shift(&a, &b), 0);
    }

    #[test]
    fn a_fresh_look_near_the_end() {
        // measured at the start: 128 BPM, downbeats from 500 ms
        let first = m(128.0, 500.0, 0, 0.5);
        // near the end, 127.5 BPM really, its beats from 180 s on (a beat late of the old grid's by 30 ms)
        let mut local = m(127.5, 0.0, 0, 0.5);
        local.from_ms = 180_000;
        let bar = 4.0f64 * 60_000.0 / 128.0;
        let k = ((180_000.0 - 500.0) / bar).ceil();
        let down = 500.0 + k * bar;
        let period = 60_000.0 / 127.5;
        local.rhythm.beats = (0..32).map(|i| down - 180_000.0 + 30.0 - 2.0 * period + i as f64 * period).collect();
        let r = relined(&first, &local).expect("lined up");
        // its first downbeat is the beat on the old grid's downbeat (the third here)
        assert_eq!(r.rhythm.downbeat, 2);
        // another tempo there entirely: the first measurement stands
        let mut other = local.clone();
        other.rhythm.bpm = 140.0;
        assert!(relined(&first, &other).is_none());
    }

    #[test]
    fn easing_back() {
        assert!((ease(1.02, 0, 8000) - 1.02).abs() < 1e-9);
        assert!((ease(1.02, 8000, 8000) - 1.0).abs() < 1e-9);
        let mid = ease(1.02, 4000, 8000);
        assert!(mid < 1.02 && mid > 1.0);
    }
}

/// A mix rendered to a WAV file, to listen to: the measured song OMNIPULSE_MIX_A (a SoundCloud track id)
/// into OMNIPULSE_MIX_B, planned as the app plans it, from 12 s before its moment to 20 s after it.
/// OMNIPULSE_MIX_A=.. OMNIPULSE_MIX_B=.. OMNIPULSE_MIX_OUT=mix.wav cargo test -p omnipulse --bin OmniPulse-native mix_demo -- --ignored
#[cfg(test)]
#[test]
#[ignore]
fn mix_demo() {
    use crate::track::{Src, Track};
    let (a_id, b_id) = (std::env::var("OMNIPULSE_MIX_A").unwrap(), std::env::var("OMNIPULSE_MIX_B").unwrap());
    let out_path = std::env::var("OMNIPULSE_MIX_OUT").unwrap_or_else(|_| "mix-demo.wav".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (a, b) = (omnipulse_core::rhythm::known(&a_id).expect("A measured"), omnipulse_core::rhythm::known(&b_id).expect("B measured"));
    let t = |id: &str| Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.to_string(), ..Default::default() };
    let mut sa = rt.block_on(crate::player::open_source(&t(&a_id), rt.handle().clone())).expect("A opens");
    let sb = rt.block_on(crate::player::open_source(&t(&b_id), rt.handle().clone())).expect("B opens");
    let dur = sa.duration_ms().unwrap_or(0);
    let p = plan(&a, dur, 0, &b, 8).expect("a plan");
    println!("{p:?}");
    let lead = 12_000;
    sa.seek(p.at_ms - lead).unwrap();
    let mut sb = omnipulse_audio::Stretched::shifted(sb, omnipulse_audio::Tempo::new(p.rate), p.semitones).unwrap();
    omnipulse_audio::Source::seek(&mut sb, p.start_ms).unwrap();
    let rate = 44_100;
    let out = omnipulse_audio::engine::render_mix(sa, Box::new(sb), rate, lead as f32 / 1000.0, p.ms, Some(p.swap_ms), lead as f32 / 1000.0 + 20.0 + p.ms as f32 / 1000.0).unwrap();
    // a 16-bit stereo WAV
    let mut w = Vec::new();
    let data = (out.len() * 2) as u32;
    for (tag, n) in [(b"RIFF", 36 + data)] { w.extend_from_slice(tag); w.extend_from_slice(&n.to_le_bytes()); }
    w.extend_from_slice(b"WAVEfmt ");
    for v in [16u32] { w.extend_from_slice(&v.to_le_bytes()); }
    for v in [1u16, 2] { w.extend_from_slice(&v.to_le_bytes()); }
    for v in [rate, rate * 4] { w.extend_from_slice(&v.to_le_bytes()); }
    for v in [4u16, 16] { w.extend_from_slice(&v.to_le_bytes()); }
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data.to_le_bytes());
    for s in &out { w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); }
    std::fs::write(&out_path, w).unwrap();
    println!("wrote {out_path}: the mix starts 12 s in");
}
