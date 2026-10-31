// Tape Stops (Seamless Mix's page): two songs too far apart to blend on the beat, the one playing braked
// to a stop over its last beat with one, the next one spinning up from its start as it halts.

use crate::plan::Plan;
use crate::smart::{never_before_ms, Side};
use omnipulse_core::rhythm::Measured;

/// A beat that's not clear enough isn't stopped on
const MIN_CONFIDENCE: f64 = 0.15;
/// (after the spin-up: the next one alone a moment, at its speed, before the change is over)
const TAPE_TAIL_MS: u64 = 300;
/// A phrase: 8 bars
const PHRASE: usize = 8;
/// A bar's loudness "loud enough to be the song proper": within this of its loud bars' (80th percentile)
const LOUD_WITHIN_DB: f32 = 5.0;
/// A bar with a beat: at least this strong, against the song's usual (core::rhythm's bar_beat)
const BEAT_LINE: f32 = 0.6;
/// A bar with a beat this near the song's loud ones is its content (a verse), not an intro to it
const CONTENT_BELOW_DB: f32 = 9.0;
/// A drop: 3 dB louder than the 8 bars before it, and within 3 dB of the song's loudest 8
const DROP_RISE_DB: f64 = 3.0;
const DROP_NEAR_DB: f64 = 3.0;

fn loud_line(bars: &[f32]) -> Option<f32> {
    if bars.len() < 8 { return None; }
    let mut v: Vec<f32> = bars.to_vec();
    v.sort_by(|a, b| a.total_cmp(b));
    Some(v[(v.len() * 4) / 5] - LOUD_WITHIN_DB)
}

/// Which bars are the song proper: a beat, and loud enough (not fading out)
fn proper(m: &Measured) -> Option<Vec<bool>> {
    let r = &m.rhythm;
    if r.bar_beat.len() != r.bar_db.len() { return None; }
    let line = loud_line(&r.bar_db)?;
    Some(r.bar_db.iter().zip(&r.bar_beat).map(|(d, b)| *d >= line && *b >= BEAT_LINE).collect())
}

/// Where its beat starts (the first of two bars in a row with one) and ends (after its last two)
fn beat_span(p: &[bool]) -> Option<(usize, usize)> {
    let first = (0..p.len().saturating_sub(1)).find(|&i| p[i] && p[i + 1])?;
    let last = (1..p.len()).rev().find(|&i| p[i] && p[i - 1])?;
    Some((first, last + 1))
}

/// Where its content starts (its first two bars with a beat near its loud ones); its end if none
fn content_start(m: &Measured, n: usize) -> usize {
    let (bars, beat) = (&m.rhythm.bar_db, &m.rhythm.bar_beat);
    let Some(line) = loud_line(bars).map(|l| l + LOUD_WITHIN_DB - CONTENT_BELOW_DB) else { return n.saturating_sub(1) };
    let content = |i: usize| bars[i] > line && beat.get(i).is_some_and(|b| *b >= BEAT_LINE);
    (0..bars.len().saturating_sub(1)).find(|&i| content(i) && content(i + 1)).unwrap_or(n.saturating_sub(1)).min(n.saturating_sub(1))
}

/// Where its phrases start (0..8 bars from its first downbeat): where its sections change most
fn phrase_offset(m: &Measured) -> usize {
    let r = &m.rhythm;
    let n = r.bar_db.len();
    if n < 16 || r.bar_beat.len() != n { return 0; }
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
    (0..PHRASE).fold(0, |best, o| if score[o] > score[best] + 1e-9 { o } else { best })
}

fn on_phrase(k: usize, off: usize, every: usize) -> bool { (k + PHRASE - off % PHRASE) % every == 0 }

/// Its drop, if its intro builds to one (a phrase's start in its first 40% with that rise)
fn drop_bar(m: &Measured, off: usize) -> Option<usize> {
    let d = &m.rhythm.bar_db;
    let n = d.len();
    if n < 24 { return None; }
    let mean = |a: usize, b: usize| (a..b).map(|i| d[i].max(-40.0) as f64).sum::<f64>() / (b - a) as f64;
    let loudest = (0..=n - 8).map(|i| mean(i, i + 8)).fold(f64::MIN, f64::max);
    (8..=(n * 2 / 5).min(n - 8)).filter(|&i| on_phrase(i, off, 4)).filter_map(|i| {
        let (after, before) = (mean(i, i + 8), mean(i - 8, i));
        (after - before >= DROP_RISE_DB && loudest - after <= DROP_NEAR_DB).then_some((i, (after - before) - (loudest - after)))
    }).fold(None, |best: Option<(usize, f64)>, x| if best.is_none_or(|b| x.1 > b.1 + 1e-9) { Some(x) } else { best }).map(|(i, _)| i)
}

/// Each bar's downbeat (ms from its start)
fn downbeats(m: &Measured) -> Vec<f64> {
    m.rhythm.beats.iter().skip(m.rhythm.downbeat).step_by(4).map(|b| m.from_ms as f64 + b).collect()
}

/// A tape stop from `a` (playing, at `now_ms`) into `b`: the one playing brakes over its last beat with
/// one (two beats long), coming to rest on the downbeat its beat ends at (its outro cut); as it stops,
/// the next one spins up from its start (its quiet intro cut, never its content; from its drop where it
/// builds to one). None without both measured whole, no room left, or before halfway.
pub fn tape(a: &Side, b: &Side, now_ms: u64) -> Option<Plan> {
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms };
    if ma.rhythm.tempo_confidence < MIN_CONFIDENCE || mb.rhythm.tempo_confidence < MIN_CONFIDENCE { return None; }
    let (ad, bd) = (downbeats(ma), downbeats(mb));
    if ad.is_empty() || bd.is_empty() { return None; }
    let (_, a_end) = beat_span(&proper(ma)?)?;
    let (b_main, _) = beat_span(&proper(mb)?)?;
    let b_off = phrase_offset(mb);
    let first = content_start(mb, bd.len());
    let cap = (0..first).rev().find(|&c| on_phrase(c, b_off, PHRASE)).unwrap_or(0);
    let drop_at = drop_bar(mb, b_off).map_or(b_main, |d| d.max(b_main));
    let b_at = |k: usize| bd[k.min(cap).min(bd.len() - 1)];
    let (soonest, audible) = (now_ms as f64 + 4000.0, end as f64 - 300.0);
    let a_bar = 4.0 * ma.rhythm.period_ms;
    let at = |k: usize| ad.get(k).copied().unwrap_or_else(|| ad[ad.len() - 1] + (k + 1 - ad.len()) as f64 * a_bar);
    let beat = ma.rhythm.period_ms;
    let stop = (2.0 * beat).clamp(600.0, 1600.0);
    // (its spin-up under the stop's second half: at its speed as the other halts)
    let spin = stop / 2.0;
    let from = at(a_end) - beat;
    if from < soonest || at(a_end) > audible + 300.0 { return None; }
    let p = Plan {
        tail_bars: 0,
        at_ms: from.round() as u64,
        ms: stop.round() as u64 + TAPE_TAIL_MS,
        swap_ms: stop.round() as u64,
        // (from half its spin-up before its downbeat: spinning up reads half as much as it plays)
        start_ms: (b_at(drop_at) - spin / 2.0).max(0.0).round() as u64,
        rate: 1.0,
        bars: 1,
        semitones: 0,
        beatmatched: false,
        out_tempo: 1.0,
        roll_speed: 1.0,
        clash: false,
        ramp_ms: spin.round() as u64,
        tape_stop: true,
        style: None,
    };
    (p.at_ms >= never_before_ms(end)).then_some(p)
}
