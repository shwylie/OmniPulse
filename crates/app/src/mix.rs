// Seamless mixing (Fuser's idea, a DJ's way): when the track playing and the next one both have a beat
// measured (core::rhythm), the next one comes in on a downbeat of the one playing, at its tempo (its
// pitch kept: crate::stretch's keylock), handed over a band at a time (omnipulse_audio::Handoff), the
// beat kept going all through: the mix is over the one playing's last bars with a beat (its beatless
// outro cut), the next one's beat in by its half (its beatless intro cut); then it eases back to its own
// tempo. Tempos (or keys) that don't go together: over the soft part instead (a beatless outro or
// intro), each at its own tempo, the two beats never heard at once.
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
    /// on the beat (false: over the soft part, each at its own tempo)
    pub beatmatched: bool,
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

/// A bar's loudness "loud enough to be the song proper": within this of its loud bars' (its 80th percentile)
const LOUD_WITHIN_DB: f32 = 5.0;
/// A bar with a beat: at least this strong, against the song's usual (core::rhythm's bar_beat)
const BEAT_LINE: f32 = 0.6;

fn loud_line(bars: &[f32]) -> Option<f32> {
    if bars.len() < 8 { return None; }
    let mut v: Vec<f32> = bars.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[(v.len() * 4) / 5] - LOUD_WITHIN_DB)
}

/// Which of a song's bars are the song proper: a beat, and loud enough (not fading out). None if it
/// wasn't measured for that.
fn proper(m: &Measured) -> Option<Vec<bool>> {
    let r = &m.rhythm;
    if r.bar_beat.len() != r.bar_db.len() { return None; }
    let line = loud_line(&r.bar_db)?;
    Some(r.bar_db.iter().zip(&r.bar_beat).map(|(d, b)| *d >= line && *b >= BEAT_LINE).collect())
}

/// Where a song's beat starts (its first of two bars in a row with one) and ends (after its last two)
fn beat_span(p: &[bool]) -> Option<(usize, usize)> {
    let first = (0..p.len().saturating_sub(1)).find(|&i| p[i] && p[i + 1])?;
    let last = (1..p.len()).rev().find(|&i| p[i] && p[i - 1])?;
    Some((first, last + 1))
}

/// Whether two keys go together (as they are, or shifted for), or either isn't clear enough to say
fn keys_fit(a: &Measured, b: &Measured) -> bool {
    use omnipulse_core::rhythm::{keys_mix, transposed};
    if a.rhythm.key_confidence < MIN_KEY_CONFIDENCE || b.rhythm.key_confidence < MIN_KEY_CONFIDENCE || keys_mix(&a.rhythm.camelot, &b.rhythm.camelot) { return true; }
    [1, -1, 2, -2].iter().any(|s| transposed(&b.rhythm.key, *s).is_some_and(|(_, c)| keys_mix(&a.rhythm.camelot, &c)))
}

/// A phrase: 8 bars (32 beats, four counts of eight)
const PHRASE: usize = 8;

/// Where a song's phrases start (0..8: bars from its first downbeat). Its sections change (louder or
/// quieter, a beat in or out) on a phrase's first bar, so it's the one of the 8 they change most at:
/// each bar's change, its next 4 bars against its last 4 (a fill at a phrase's end barely counts), the
/// fade into silence at its end left out.
fn phrase_offset(m: &Measured) -> usize {
    let r = &m.rhythm;
    let n = r.bar_db.len();
    if n < 16 || r.bar_beat.len() != n { return 0; }
    // (3 dB of loudness counts as much as the beat going from none to its usual)
    let mean = |a: usize, b: usize| {
        let k = (b - a) as f64;
        ((a..b).map(|i| r.bar_db[i] as f64 / 3.0).sum::<f64>() / k, (a..b).map(|i| r.bar_beat[i].min(2.0) as f64).sum::<f64>() / k)
    };
    let mut score = [0.0f64; PHRASE];
    for i in 4..n - 4 {
        if (i - 4..i + 4).any(|j| r.bar_db[j] < -35.0) { continue; }
        let (l, rr) = (mean(i - 4, i), mean(i, i + 4));
        score[i % PHRASE] += (rr.0 - l.0).abs() + (rr.1 - l.1).abs();
    }
    // (no change anywhere, or a tie: from its first downbeat)
    (0..PHRASE).fold(0, |best, o| if score[o] > score[best] + 1e-9 { o } else { best })
}

/// Whether bar k starts a phrase (or every `every` bars of one) of a song whose phrases start at `off`
fn on_phrase(k: usize, off: usize, every: usize) -> bool { (k + PHRASE - off % PHRASE) % every == 0 }

/// Each bar's downbeat in a measured track (ms from its start), from its tracked beats
fn downbeats(m: &Measured) -> Vec<f64> {
    m.rhythm.beats.iter().skip(m.rhythm.downbeat).step_by(4).map(|b| m.from_ms as f64 + b).collect()
}

/// How a plan hands over: on the beat, staged, a band at a time (the one coming in's highs up over its
/// first bar; its mids and bass in at the swap, where its main part starts, the one going out's mids
/// echoed as they go; the one going out's highs leaving over the last quarter); over a soft part, a
/// crossfade with the bass swapped halfway. `filtered`: through high-pass filters instead (Settings'
/// Filter Sweep Transitions): the one going out thinning out, the next coming in thin, opening on the swap.
pub fn handoff(p: &Plan, filtered: bool) -> omnipulse_audio::Handoff {
    if !p.beatmatched { return omnipulse_audio::Handoff::bass_swap(p.ms, p.ms / 2); }
    let beat = p.ms as f64 / (p.bars * 4) as f64;
    if filtered { omnipulse_audio::Handoff::filtered(p.ms, beat, p.swap_ms) } else { omnipulse_audio::Handoff::staged(p.ms, beat, p.swap_ms) }
}

/// A measured track's beat as it is over n beats from `from` (ms from its start), from its tracked beats: a
/// song's tempo drifts, so its average isn't quite its tempo there. None past its last beats.
fn period_at(m: &Measured, from: f64, n: usize) -> Option<f64> {
    let i = m.rhythm.beats.iter().position(|x| m.from_ms as f64 + x >= from - 30.0)?;
    let (b0, b1) = (m.rhythm.beats.get(i)?, m.rhythm.beats.get(i + n)?);
    Some((b1 - b0) / n as f64)
}

/// The plan, from both songs measured whole (each beat tracked; each bar's loudness and beat), for
/// mixing `b` in after `a` (`a_end_ms`: where `a`'s sound ends; `now_ms`: where it is).
///
/// Tempos and keys that go together: the beat kept going. The mix is over the one playing's last bars
/// with a beat (16, else 8, else 4; on a phrase, as near its beat's end as can be), its beatless outro
/// cut; the next one comes in so its beat starts by the mix's half, its beatless intro before that cut.
///
/// Tempos (or keys) that don't: over the soft part, each at its own tempo, the beats never together:
/// the one playing's beatless outro (up to 8 bars) as the next one comes in; else the next one's
/// beatless intro under the one playing's last bars with a beat; else a quick blend (2 bars) where the
/// one playing's beat ends, into the next one's.
///
/// None without what that needs (then plan, from a grid).
pub fn plan_structure(a: &Measured, a_end_ms: u64, now_ms: u64, b: &Measured) -> Option<Plan> {
    if a.rhythm.tempo_confidence < MIN_CONFIDENCE || b.rhythm.tempo_confidence < MIN_CONFIDENCE { return None; }
    let (ad, bd) = (downbeats(a), downbeats(b));
    if ad.is_empty() || bd.is_empty() { return None; }
    let (_, a_end) = beat_span(&proper(a)?)?;
    let (b_main, _) = beat_span(&proper(b)?)?;
    let (a_off, b_off) = (phrase_offset(a), phrase_offset(b));
    let (soonest, audible) = (now_ms as f64 + 4000.0, a_end_ms as f64 - 300.0);
    let (a_bar, b_bar) = (4.0 * a.rhythm.period_ms, 4.0 * b.rhythm.period_ms);
    // (bar k's downbeat; past the last one tracked, a bar on from it for each)
    let at = |k: usize| ad.get(k).copied().unwrap_or_else(|| ad[ad.len() - 1] + (k + 1 - ad.len()) as f64 * a_bar);
    let b_at = |k: usize| bd[k.min(bd.len() - 1)];
    let matched = match_rate(a.rhythm.bpm, b.rhythm.bpm).map(|(r, _)| r).filter(|_| keys_fit(a, b));
    if let Some(r) = matched {
        let fits = |k: usize, n: u32| at(k) >= soonest && k + n as usize <= a_end && at(k) + a_bar * n as f64 <= audible;
        // the latest start on its phrase (8 bars, else 4) that gives up no more than 4 of its bars with a beat; else any
        let (bars, k) = [16u32, 8, 4].iter().find_map(|&n| {
            let ideal = a_end.checked_sub(n as usize)?;
            let latest = |every: usize| (0..=ideal).rev().find(|&k| on_phrase(k, a_off, every) && fits(k, n));
            [8, 4].iter().filter_map(|&e| latest(e)).find(|&k| k + 4 >= ideal).or_else(|| latest(1)).map(|k| (n, k))
        })?;
        // the next one from its own phrase's start (the two counts of eight together), the one that puts
        // its beat in nearest the mix's half (rather sooner than later)
        let every = if bars >= 8 { PHRASE } else { 4 };
        let target = b_main.saturating_sub(bars as usize / 2);
        let from_bar = (0..bd.len()).filter(|&c| on_phrase(c, b_off, every))
            .min_by_key(|&c| ((c as i64 - target as i64).abs(), std::cmp::Reverse(c)))
            .unwrap_or(target).min(bd.len() - 1);
        // the swap (its mids and bass in, the other's out) where its main part starts, if that's in the
        // mix's middle half; else at the half (both on a phrase)
        let beat_in = b_main.saturating_sub(from_bar);
        let swap_bar = if (bars as usize / 4..=bars as usize * 3 / 4).contains(&beat_in) { beat_in } else { bars as usize / 2 };
        // stretched to each one's tempo over the bars mixed, not its average (within a percent of it)
        let n = bars as usize * 4;
        let rate = match (period_at(a, at(k), n), period_at(b, bd[from_bar], (n as f64 * r).round() as usize)) {
            (Some(pa), Some(pb)) => {
                let local = r * (pb / b.rhythm.period_ms) / (pa / a.rhythm.period_ms);
                if (local / r - 1.0).abs() <= 0.01 { local } else { r }
            }
            _ => r,
        };
        let len = a_bar * bars as f64;
        return Some(Plan {
            at_ms: at(k).round() as u64,
            ms: len.round() as u64,
            swap_ms: (a_bar * swap_bar as f64).round() as u64,
            start_ms: bd[from_bar].round() as u64,
            rate,
            bars,
            semitones: key_shift(a, b),
            beatmatched: true,
        });
    }
    let soft = |at_ms: f64, ms: f64, start_ms: f64, bars: u32| Some(Plan {
        at_ms: at_ms.round() as u64,
        ms: ms.round() as u64,
        swap_ms: (ms / 2.0).round() as u64,
        start_ms: start_ms.round() as u64,
        rate: 1.0,
        bars,
        semitones: 0,
        beatmatched: false,
    });
    // the one playing's beatless outro (what of it is heard), the next one coming in over it: its own
    // beatless intro too, as much as fits, its beat as the blend ends
    let a_soft = (a_end..).take_while(|&k| at(k) + a_bar <= audible).count();
    if a_soft >= 2 && at(a_end) >= soonest {
        let n = a_soft.min(8);
        let ms = a_bar * n as f64;
        let nb = ((ms / b_bar).floor() as usize).min(b_main);
        return soft(at(a_end), ms, b_at(b_main - nb), n as u32);
    }
    // the next one's beatless intro, under the one playing's last bars with a beat: its beat comes in
    // as the other's ends
    if b_main >= 2 {
        let nb = b_main.min(8);
        let ms = b_bar * nb as f64;
        let end = at(a_end).min(audible);
        if let Some(k) = (0..=a_end).rev().find(|&k| at(k) + ms <= end && at(k) >= soonest) {
            return soft(at(k), ms, b_at(b_main - nb), nb as u32);
        }
    }
    // neither has one: two bars where the one playing's beat ends, straight into the next one's
    let ms = 2.0 * a_bar;
    let k = (0..=a_end.saturating_sub(2)).rev().find(|&k| at(k) >= soonest && at(k) + ms <= audible)?;
    soft(at(k), ms, b_at(b_main), 2)
}

/// The plan for mixing `b` in after `a` (`a_end_ms`: where `a`'s sound ends; `now_ms`: where it is),
/// over `bars` bars, or None (no clear beat, tempos too far apart, no room left)
/// The plan, from a grid (a song measured only in part: its first 30%, say): the latest phrase-aligned
/// downbeat that leaves its bars before the sound ends, the next one from its first downbeat
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
        beatmatched: true,
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
        Measured { from_ms: 0, to_ms: 60_000, rhythm: Rhythm { bpm, first_beat_ms: first_beat, period_ms: period, beats, downbeat, tempo_confidence: conf, key: "A minor".into(), camelot: "8A".into(), key_confidence: 0.5, bar_db: Vec::new(), bar_beat: Vec::new() }, v: VERSION }
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

    /// A song of bars (each its loudness, and whether it has a beat), every beat tracked
    fn shaped(mut x: Measured, bars: &[(f32, bool)]) -> Measured {
        let period = x.rhythm.period_ms;
        let n = bars.len() * 4 + x.rhythm.downbeat + 4;
        x.rhythm.beats = (0..n).map(|i| x.rhythm.first_beat_ms + i as f64 * period).collect();
        x.rhythm.bar_db = bars.iter().map(|b| b.0).collect();
        x.rhythm.bar_beat = bars.iter().map(|b| if b.1 { 1.0 } else { 0.1 }).collect();
        x
    }

    fn song(parts: &[(usize, f32, bool)]) -> Vec<(f32, bool)> {
        parts.iter().flat_map(|&(n, db, beat)| std::iter::repeat((db, beat)).take(n)).collect()
    }

    #[test]
    fn the_beat_kept_going_its_beatless_parts_cut() {
        // the one playing: 64 bars with a beat, then a 24-bar outro without one (as loud: a pad, a voice);
        // the next: a 24-bar intro without a beat, then 64 with one
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(64, -10.0, true), (24, -11.0, false)]));
        let b = shaped(m(128.0, 300.0, 0, 0.5), &song(&[(24, -12.0, false), (64, -9.0, true)]));
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let end = (500.0 + 88.0 * bar) as u64;
        let p = plan_structure(&a, end, 10_000, &b).expect("a plan");
        assert!(p.beatmatched);
        assert_eq!(p.bars, 16);
        // over its last 16 bars with a beat (from bar 48, a phrase), its beatless outro cut
        assert_eq!(p.at_ms, (500.0 + 48.0 * bar).round() as u64);
        // the next one's beat (bar 24) in at the mix's half: from its bar 16, the rest of its intro cut
        assert_eq!(p.start_ms, (300.0 + 16.0 * bar).round() as u64);
        // a beat all through, end to end: the latest phrase that fits before the sound ends
        let flat = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        let p = plan_structure(&flat, end, 10_000, &shaped(m(128.0, 300.0, 0, 0.5), &song(&[(88, -10.0, true)]))).expect("a plan");
        assert_eq!((p.bars, p.start_ms), (16, 300));
        assert_eq!(p.at_ms, (500.0 + 68.0 * bar).round() as u64);
        assert!(p.at_ms + p.ms <= end);
    }

    #[test]
    fn on_each_ones_own_phrases() {
        // the one playing's sections change a bar after its first downbeat (bars 1, 25, 33, 65: a
        // pickup bar first), the next one's on its own
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(1, -20.0, false), (24, -10.0, true), (8, -14.0, true), (32, -10.0, true), (23, -11.0, false)]));
        let b = shaped(m(128.0, 300.0, 0, 0.5), &song(&[(16, -12.0, false), (64, -9.0, true)]));
        assert_eq!((phrase_offset(&a), phrase_offset(&b)), (1, 0));
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let p = plan_structure(&a, (500.0 + 88.0 * bar) as u64, 10_000, &b).expect("a plan");
        // out on its phrase (bar 49, 16 before its beat ends at 65), not a bar early; the next one from
        // its bar 8 (on its phrase), its beat in (and the swap) at the half
        assert_eq!(p.at_ms, (500.0 + 49.0 * bar).round() as u64);
        assert_eq!(p.start_ms, (300.0 + 8.0 * bar).round() as u64);
        assert_eq!(p.swap_ms, (8.0 * bar).round() as u64);
    }

    #[test]
    fn tempos_apart_over_the_soft_part() {
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let fast_bar: f64 = 4.0 * 60_000.0 / 174.0;
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(64, -10.0, true), (24, -11.0, false)]));
        let end = (500.0 + 88.0 * bar) as u64;
        // its beatless outro (8 bars of it) as the next one comes in, each at its own tempo: the next one's
        // beatless intro under it, as much as fits (10 of its bars), its beat as the blend ends
        let fast = shaped(m(174.0, 300.0, 0, 0.5), &song(&[(24, -12.0, false), (64, -9.0, true)]));
        let p = plan_structure(&a, end, 10_000, &fast).expect("a plan");
        assert!(!p.beatmatched);
        assert_eq!((p.bars, p.rate, p.semitones), (8, 1.0, 0));
        assert_eq!(p.at_ms, (500.0 + 64.0 * bar).round() as u64);
        assert_eq!(p.start_ms, (300.0 + 14.0 * fast_bar).round() as u64);
        // no outro without a beat: the next one's beatless intro under the one playing's last bars, its
        // beat coming in as the other's ends
        let flat = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        let p = plan_structure(&flat, end, 10_000, &fast).expect("a plan");
        assert_eq!(p.start_ms, (300.0 + 16.0 * fast_bar).round() as u64);
        assert!(p.at_ms + p.ms <= end - 300 && p.at_ms + p.ms + bar as u64 > end - 300);
        // neither: two bars where the one playing's beat ends, into the next one's
        let p = plan_structure(&flat, end, 10_000, &shaped(m(174.0, 300.0, 0, 0.5), &song(&[(88, -9.0, true)]))).expect("a plan");
        assert_eq!((p.bars, p.start_ms), (2, 300));
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
    // (OMNIPULSE_MIX_FILTER=1: through the filter sweeps)
    let filter = std::env::var("OMNIPULSE_MIX_FILTER").is_ok_and(|v| v == "1");
    let rt = tokio::runtime::Runtime::new().unwrap();
    // each measured whole (as the app does for a mix), kept as it keeps them
    let whole = |id: &str| omnipulse_core::rhythm::known(&format!("full:{id}")).filter(|m| !m.rhythm.bar_beat.is_empty()).unwrap_or_else(|| {
        let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)).expect("its sound");
        let heard = mono.len() as u64 * 1000 / rate as u64;
        let m = omnipulse_core::rhythm::Measured { from_ms: 0, to_ms: heard, rhythm: omnipulse_core::rhythm::analyse(&mono, rate).expect("a beat"), v: omnipulse_core::rhythm::VERSION };
        omnipulse_core::rhythm::keep(&format!("full:{id}"), m.clone());
        m
    });
    let (a, b) = (whole(&a_id), whole(&b_id));
    println!("phrases from bar {} of A, {} of B", phrase_offset(&a), phrase_offset(&b));
    for (n, m) in [("A", &a), ("B", &b)] {
        println!("{n}: {:.2} BPM, {}, {} beats, bars {:?}", m.rhythm.bpm, m.rhythm.camelot, m.rhythm.beats.len(), m.rhythm.bar_db.iter().zip(&m.rhythm.bar_beat).map(|(d, b)| format!("{}/{:.1}", d.round() as i32, b)).collect::<Vec<_>>());
    }
    let t = |id: &str| Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.to_string(), ..Default::default() };
    let mut sa = rt.block_on(crate::player::open_source(&t(&a_id), rt.handle().clone())).expect("A opens");
    let sb = rt.block_on(crate::player::open_source(&t(&b_id), rt.handle().clone())).expect("B opens");
    let dur = sa.duration_ms().unwrap_or(0);
    let p = plan_structure(&a, dur, 0, &b).expect("a plan");
    println!("{p:?}");
    let lead = 12_000;
    sa.seek(p.at_ms - lead).unwrap();
    let mut sb = omnipulse_audio::Stretched::shifted(sb, omnipulse_audio::Tempo::new(p.rate), p.semitones).unwrap();
    omnipulse_audio::Source::seek(&mut sb, p.start_ms).unwrap();
    let rate = 44_100;
    let out = omnipulse_audio::engine::render_mix(sa, Box::new(sb), rate, lead as f32 / 1000.0, p.ms, Some(handoff(&p, filter)), lead as f32 / 1000.0 + 20.0 + p.ms as f32 / 1000.0).unwrap();
    // how clearly one beat holds through the blend (the two lined up: one; off: two fighting), against the
    // same blend with the one coming in half a beat late
    let held = |out: &[f32]| {
        let from = lead as usize * rate as usize / 1000 * 2;
        let mono: Vec<f32> = out[from..from + p.ms as usize * rate as usize / 1000 * 2].chunks(2).map(|f| (f[0] + f[1]) * 0.5).collect();
        omnipulse_core::rhythm::analyse(&mono, rate).map(|r| (r.bpm, r.tempo_confidence))
    };
    let off = {
        let mut sa = rt.block_on(crate::player::open_source(&t(&a_id), rt.handle().clone())).unwrap();
        sa.seek(p.at_ms - lead).unwrap();
        let sb = rt.block_on(crate::player::open_source(&t(&b_id), rt.handle().clone())).unwrap();
        let mut sb = omnipulse_audio::Stretched::shifted(sb, omnipulse_audio::Tempo::new(p.rate), p.semitones).unwrap();
        omnipulse_audio::Source::seek(&mut sb, p.start_ms + (b.rhythm.period_ms / 2.0) as u64).unwrap();
        omnipulse_audio::engine::render_mix(sa, Box::new(sb), rate, lead as f32 / 1000.0, p.ms, Some(handoff(&p, filter)), lead as f32 / 1000.0 + p.ms as f32 / 1000.0 + 1.0).unwrap()
    };
    println!("the blend: lined up {:?}, half a beat off {:?}", held(&out), held(&off));
    // each of the next one's beats in the blend (where it lands, stretched) against the nearest of the one
    // playing's, as both were tracked whole: how far apart they are (ms), for this plan and the old one
    let apart = |p: &Plan| {
        let ab: Vec<f64> = a.rhythm.beats.iter().map(|x| a.from_ms as f64 + x).collect();
        let d: Vec<f64> = b.rhythm.beats.iter().map(|x| b.from_ms as f64 + x).filter(|x| *x >= p.start_ms as f64 && *x < p.start_ms as f64 + p.ms as f64 * p.rate)
            .map(|x| p.at_ms as f64 + (x - p.start_ms as f64) / p.rate)
            .map(|x| ab.iter().map(|y| (x - y).abs()).fold(f64::MAX, f64::min)).collect();
        let bar_of = |m: &omnipulse_core::rhythm::Measured, t: f64| m.rhythm.beats.iter().position(|x| m.from_ms as f64 + x >= t - 30.0).map(|i| (i as i64 - m.rhythm.downbeat as i64).rem_euclid(4));
        (d.iter().sum::<f64>() / d.len().max(1) as f64, d.iter().copied().fold(0.0, f64::max), bar_of(&a, p.at_ms as f64), bar_of(&b, p.start_ms as f64))
    };
    println!("beats apart (mean ms, worst ms, beat of A's bar it goes out on, of B's it comes in on): this plan {:?}", apart(&p));
    if let (Some(pa), Some(pb)) = (omnipulse_core::rhythm::known(&a_id), omnipulse_core::rhythm::known(&b_id)) {
        if let Some(old) = plan(&pa, dur, 0, &pb, 8) { println!("the old plan (from 30%): {old:?}: {:?}", apart(&old)); }
    }
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
    println!("wrote {out_path}: the mix starts 12 s in (A at {:.1} s of {:.1} s, B from {:.1} s)", p.at_ms as f64 / 1000.0, dur as f64 / 1000.0, p.start_ms as f64 / 1000.0);
}
