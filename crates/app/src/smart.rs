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

/// Where a transition starts in the one playing (ms): in its last quarter; a blend that won't fit there,
/// earlier, as late as it fits
pub fn earliest_ms(end_ms: u64) -> u64 {
    end_ms * 3 / 4
}

/// And never before halfway, however long the blend (one that won't fit after it: a fade instead)
pub fn never_before_ms(end_ms: u64) -> u64 {
    end_ms / 2
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
    // (in its last quarter: a fade always fits there)
    let len = len.min(end - earliest_ms(end)).max(MIN_FADE_MS.min(end));
    Fade { out_from_ms: end.saturating_sub(len), out_to_ms: end, in_from_ms: b.first_ms }
}

// ---------- Tier 1: when a blend on the beat is safe ----------
/// A beat clear enough to trust (core::rhythm's tempo_confidence)
const TRUST_CONFIDENCE: f64 = 0.3;
/// How far from its own tempo the next one plays (DJs: mostly under 5%)
pub const MAX_RATE: f64 = 0.04;
/// How far apart two songs' tempos can be for a blend with the tempo ridden from one to the other (as a
/// DJ ramps between genres: Goodbye's 92 into One's 125, 174 counted as 87 into 128); past 4%, the blend's
/// always 32 beats, room for it
pub const MAX_RAMP_BLEND: f64 = 0.50;
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
    // (its beats on the song's pulse: Beat This!'s tempo agrees, as it is or at half or double)
    let model = m.rhythm.model_bpm;
    if model > 0.0 && ![1.0, 2.0, 0.5].iter().any(|k| ((bpm * k) / model - 1.0).abs() <= STATED_WITHIN) { return false; }
    // (its bars sure too: a downbeat a beat out matches the tempo, but not the bars)
    if m.rhythm.downbeat_confidence < omnipulse_core::rhythm::DOWNBEAT_TRUSTED { return false; }
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
    let near = |x: f64| (x / mb.rhythm.bpm - 1.0).abs() <= MAX_RAMP_BLEND;
    if match_rate(ma.rhythm.bpm, mb.rhythm.bpm).is_none() && !near(ma.rhythm.bpm) && !near(ma.rhythm.bpm / 2.0) { return Some("tempos too far apart (more than 50%)"); }
    // (keys that don't go together: still a blend, a short one: the clash only briefly, as a DJ does)
    None
}

/// Where a song's 16-beat groups (4 bars) start: the bar (0..4, counted from its first downbeat) its
/// section changes land on (a change: its next 4 bars' level against its last 4, the biggest within 3
/// bars of it counted once), and how sure that is (the share of them that land there). None: too short.
pub fn groups(m: &Measured) -> Option<(usize, f64)> {
    let d = &m.rhythm.bar_db;
    let n = d.len();
    if n < 16 { return None; }
    let mean = |a: usize, b: usize| d[a..b].iter().map(|x| x.max(-40.0) as f64).sum::<f64>() / (b - a) as f64;
    // (near its start or end, as many bars either side as there are, two at least)
    let mut jumps: Vec<(f64, usize)> = (2..=n - 2).map(|i| { let w = i.min(n - i).min(4); ((mean(i, i + w) - mean(i - w, i)).abs(), i) }).filter(|j| j.0 >= 1.5).collect();
    jumps.sort_by(|a, b| b.0.total_cmp(&a.0));
    let mut by = [0.0f64; 4];
    let mut kept: Vec<usize> = Vec::new();
    for (j, i) in jumps {
        if kept.iter().all(|k| k.abs_diff(i) > 3) { kept.push(i); by[i % 4] += j; }
    }
    let total: f64 = by.iter().sum();
    // (moved by hand, Track Preferences' Phrases Start: sure, then)
    let shift = m.rhythm.phrase_shift.rem_euclid(4) as usize;
    if total <= 0.0 { return Some((shift, if shift > 0 { 1.0 } else { 0.0 })); }
    let best = (0..4).max_by(|&a, &b| by[a].total_cmp(&by[b]))?;
    Some(((best + shift) % 4, if shift > 0 { 1.0 } else { by[best] / total }))
}

/// 16-beat groups sure enough to mix on: most of its changes (45%) on one of the four bars
const GROUPS_SURE: f64 = 0.45;

/// The group grid of a song, if it's sure
fn sure_groups(m: &Measured) -> Option<usize> {
    groups(m).filter(|g| g.1 >= GROUPS_SURE).map(|g| g.0)
}

/// A blend's bars past its swap: the one playing's beat carried while the next one's settles in
pub const TAIL_BARS: usize = 4;

/// Tier 1: a blend on the beat, as DJs do: every 16 beats of the two together (both songs' 4-bar groups
/// lined up: it starts, swaps and ends on lines of both). Out over the one playing's outro (from the line
/// it starts on), in from the start of the next one's intro (what each is made for): its intro to its main
/// part as the run-up, in whole lines (16 beats, or 32; a longer intro, from 32 beats before its main
/// part), the swap where its main part starts, then TAIL_BARS of the one playing's beat going. The next
/// one at the playing one's tempo (at most 4% off, its key as it is). None: a fade instead.
pub fn blend(a: &Side, b: &Side, now_ms: u64) -> Option<crate::mix::Plan> {
    blend_as(a, b, now_ms, false)
}

/// The same, `forced` (for listening only: no pair refused, however far apart or unsure; the
/// renderer's OMNIPULSE_SMART_FORCE)
pub fn blend_as(a: &Side, b: &Side, now_ms: u64, forced: bool) -> Option<crate::mix::Plan> {
    let why = |w: &str| if std::env::var_os("OMNIPULSE_MIX_DEBUG").is_some() { eprintln!("smart debug: no blend: {w}"); };
    if !forced && blend_blocked(a, b).is_some() { return None; }
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    // (a big slowdown sounds wrong: the one playing counted at half time instead (174 as 87) and sped up)
    let half;
    let ma = if ma.rhythm.bpm > mb.rhythm.bpm * (1.0 + MAX_RATE) && (ma.rhythm.bpm / 2.0 / mb.rhythm.bpm - 1.0).abs() <= MAX_RAMP_BLEND && ma.rhythm.bpm / 2.0 < mb.rhythm.bpm {
        half = halved(ma);
        &half
    } else { ma };
    // (same time only: a half- or double-time pair's bars don't line up one to one)
    let ramp = (ma.rhythm.bpm / mb.rhythm.bpm - 1.0).abs();
    if ramp > MAX_RAMP_BLEND && !forced { return None; }
    // (16-beat lines not sure: the likeliest; the user would rather a blend on it than a fade)
    let _ = forced;
    let grid = |m: &Measured| groups(m).map(|g| g.0);
    // (listening only: a song's 16-beat line set by hand, OMNIPULSE_SMART_LINES="bpm:line", by its tempo)
    let set = |m: &Measured, g: Option<usize>| std::env::var("OMNIPULSE_SMART_LINES").ok().and_then(|v| v.split(',').find_map(|x| x.split_once(':').filter(|(b, _)| (b.trim().parse::<f64>().unwrap_or(0.0) - m.rhythm.bpm).abs() < 0.05).and_then(|(_, l)| l.trim().parse::<usize>().ok()))).or(g);
    let grid = |m: &Measured| set(m, grid(m));
    let (Some(ga), Some(gb)) = (grid(ma), grid(mb)) else { why(&format!("16-beat lines not sure: {:?} {:?}", groups(ma), groups(mb))); return None };
    let bars = |m: &Measured| m.rhythm.beats.len().saturating_sub(m.rhythm.downbeat) / 4;
    let (na, nb) = (bars(ma), bars(mb));
    let on = |k: usize, g: usize| k % 4 == g % 4;
    // the next one: from its first line, its intro (to its main part) the run-up
    let s0 = gb % 4;
    let first_b = body_line(mb).and_then(|l| mb.rhythm.bar_db.iter().position(|d| *d >= l)).unwrap_or(s0);
    let intro = first_b.saturating_sub(s0) / 4 * 4;
    // (a big change of tempo: 32 beats for it, whatever the intro; keys that clash: 16, the clash brief)
    let clash = !keys_go(ma, mb);
    let run = if clash { 4 } else if ramp > MAX_RATE { 8 } else { intro.clamp(4, 8) };
    let from = if intro > run { s0 + intro - run } else { s0 };
    if from + run + TAIL_BARS >= nb { return None; }
    // the one playing: from the line its outro starts on (where its main part ends, snapped back to a
    // line), a line earlier at a time if that runs past its sound's end; at least 5 s from now
    let line = body_line(ma)?;
    let outro = ma.rhythm.bar_db.iter().rposition(|d| *d >= line)? + 1;
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms } as f64;
    // (in its last quarter; else, as late as it fits, but never before halfway)
    let fits = |floor: u64| (0..=outro.min(na)).rev().find(|&k| {
        on(k, ga) && k + run + TAIL_BARS <= na && bar_ms(ma, k).is_some_and(|t| t >= now_ms as f64 + 5000.0 && t >= floor as f64) && bar_ms(ma, k + run + TAIL_BARS).is_some_and(|t| t <= end)
    });
    let Some(k) = fits(earliest_ms(end as u64)).or_else(|| fits(never_before_ms(end as u64))) else { why(&format!("no line to start on before its outro (bar {outro} of {na})")); return None };
    let (at, swap, over) = (bar_ms(ma, k)?, bar_ms(ma, k + run)?, bar_ms(ma, k + run + TAIL_BARS)?);
    // its tempo as it is over the run-up (a song's drifts), the next one's over its own
    let (pa, pb) = ((swap - at) / (run * 4) as f64, (bar_ms(mb, from + run)? - bar_ms(mb, from)?) / (run * 4) as f64);
    let rate = pb / pa;
    if (rate - 1.0).abs() > MAX_RAMP_BLEND && !forced { return None; }
    // the tempo ridden over all of it, as a DJ does: the two together at the one playing's at first, at
    // the next one's own by the end (the one playing eased to it, the next one with it): nothing to ease
    // back after. Its bars take longer or less as it's eased: the times, as heard
    // (ridden over the run-up: at the next one's own tempo by the swap, where the one playing stops being
    // heard; its last bars, fading, at that tempo too)
    let target = 1.0 / rate;
    let swap_at = ramp_time(swap - at, swap - at, target);
    let len = swap_at + (over - swap) / target;
    Some(crate::mix::Plan {
        at_ms: at.round() as u64, ms: len.round() as u64, swap_ms: swap_at.round() as u64, start_ms: bar_ms(mb, from)?.round() as u64,
        rate, bars: (run + TAIL_BARS) as u32, semitones: 0, beatmatched: true, out_tempo: target, ramp_ms: swap_at.round() as u64, tape_stop: false,
        out_loop: None, in_loop: None, word: None, style: None,
    })
}

/// When (ms, as heard) the one playing has played `d` ms of itself, its tempo eased (mix::out_ramp: a
/// smoothstep from 1 to `target`) over the time it takes to play `total` of itself
fn ramp_time(d: f64, total: f64, target: f64) -> f64 {
    // (its place at time t of L: t + (target - 1) L (q^3 - q^4 / 2), q = t / L; all of it by L, so
    // L = total / (1 + (target - 1) / 2); then t found by halves)
    let l = total / (1.0 + (target - 1.0) / 2.0);
    let place = |t: f64| { let q = (t / l).clamp(0.0, 1.0); t + (target - 1.0) * l * (q.powi(3) - q.powi(4) / 2.0) };
    let (mut lo, mut hi) = (0.0, l * 1.2);
    for _ in 0..60 { let m = (lo + hi) / 2.0; if place(m) < d { lo = m } else { hi = m } }
    (lo + hi) / 2.0
}

/// A song with its beat 1 moved `n` beats (+ later, as Track Preferences has it): its bars from there,
/// each one's level from its beats' (else the nearest bar's as measured)
pub fn shifted(m: &Measured, n: i32) -> Measured {
    if n == 0 { return m.clone(); }
    let r = &m.rhythm;
    let mut d = r.downbeat as i64 + n as i64;
    while d < 0 { d += 4; }
    let d = d as usize;
    let mut h = m.clone();
    h.rhythm.downbeat = d;
    let bars = r.beats.len().saturating_sub(d) / 4;
    // (bar k now starts on beat d + 4k: the bar as measured nearest it)
    let near = |k: usize| (((d + 4 * k) as i64 - r.downbeat as i64 + 2).max(0) / 4) as usize;
    let at = |v: &[f32], k: usize| v.get(near(k)).or(v.last()).copied().unwrap_or(0.0);
    h.rhythm.bar_db = (0..bars).map(|k| {
        let from = d + 4 * k;
        match r.beat_db.get(from..from + 4) {
            Some(b) => (10.0 * (b.iter().map(|x| 10f64.powf(*x as f64 / 10.0)).sum::<f64>() / 4.0).log10()) as f32,
            None => at(&r.bar_db, k),
        }
    }).collect();
    h.rhythm.bar_beat = (0..bars).map(|k| at(&r.bar_beat, k)).collect();
    h.rhythm.bar_chroma = (0..bars).filter_map(|k| r.bar_chroma.get(near(k)).or(r.bar_chroma.last()).cloned()).collect();
    h
}

/// A song counted at half time: every other beat a beat (from its bars' first), two of its bars a bar
/// (paired so its 16-beat lines start one), its tempo half
pub fn halved(m: &Measured) -> Measured {
    let r = &m.rhythm;
    let skip = groups(m).map_or(0, |g| g.0 % 2);
    let mut h = m.clone();
    h.rhythm.beats = r.beats.iter().skip(r.downbeat + 4 * skip).step_by(2).copied().collect();
    h.rhythm.downbeat = 0;
    h.rhythm.bpm = r.bpm / 2.0;
    h.rhythm.period_ms = r.period_ms * 2.0;
    h.rhythm.model_bpm = r.model_bpm / 2.0;
    h.rhythm.bar_db = r.bar_db.iter().skip(skip).copied().collect::<Vec<_>>().chunks(2).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    h.rhythm.bar_beat = r.bar_beat.iter().skip(skip).copied().collect::<Vec<_>>().chunks(2).map(|c| c.iter().sum::<f32>() / c.len() as f32).collect();
    h.rhythm.bar_chroma = r.bar_chroma.iter().skip(skip).step_by(2).cloned().collect();
    h
}

/// After a blend, the next one back to its own tempo this slowly (a minute: 1.5% over it is far below
/// what's heard), or over a third of what's left of it, if that's less
pub fn ease_ms(left_ms: u64) -> u64 {
    60_000u64.min(left_ms / 3).max(4_000)
}

/// The handoff of a plan: a blend's, the highs and mids crossfaded over it, the bass swapped at once on
/// its bar line (the two basses never together), and with `voices`, the voices handed over there too
/// (the one going out's gone over the two beats before it, the next one's up over the two after: never
/// sung together); a roll's, or a tape stop's
pub fn handoff(p: &crate::mix::Plan, voices: bool) -> omnipulse_audio::Handoff {
    if p.style == Some(ROLL) { return omnipulse_audio::Handoff::roll(p.swap_ms / p.bars.max(1) as u64, p.bars); }
    if p.tape_stop { return crate::mix::handoff(p, false); }
    // (the one playing's beat carried: the next one rising under it, the bass swapped, then its top fading;
    // the voices, if both sing there, the one playing's faded over the run-up's second half, the next
    // one's up over the bars after the swap)
    let beat = p.swap_ms as f64 / ((p.bars as usize).saturating_sub(TAIL_BARS).max(1) * 4) as f64;
    let h = omnipulse_audio::Handoff::carry(p.ms, p.swap_ms, beat.round() as u64);
    if voices { h.with_voices(p.swap_ms, p.swap_ms / 2, p.ms.saturating_sub(p.swap_ms)) } else { h }
}

/// Whether a song sings between `from` and `to` (ms in it), by its timed lyrics: a line from its time to
/// the next one's (8 s at most: a line isn't sung for longer, and a gap after it is an instrumental part)
pub fn sings(lines: &[(u64, String)], from: u64, to: u64) -> bool {
    lines.iter().enumerate().any(|(i, (t, _))| {
        let end = lines.get(i + 1).map_or(t + 8000, |n| n.0.min(t + 8000));
        *t < to && end > from
    })
}

/// A fade's handoff: the voices handed over halfway (`voices`: both sing there), on a fade long enough
/// (4 s) for it to be heard as one song's voice giving way to the other's; None, a plain crossfade
pub fn fade_handoff(ms: u64, voices: bool) -> Option<omnipulse_audio::Handoff> {
    (voices && ms >= 4000).then(|| omnipulse_audio::Handoff::voiced_crossfade(ms))
}

// ---------- Tier 2: extras, as your ratings say ----------
/// A roll's style (and its ratings' kind)
pub const ROLL: &str = "roll";
/// How many bars a roll takes: its bar as it is, then halves, quarters, eighths
const ROLL_BARS: u32 = 4;
/// The most of the next one's intro left out, so it lands on its main part (bars); longer, it lands on its
/// first downbeat
const ROLL_SKIP_MAX: usize = 16;

/// A roll out (Traktor's Beatmasher, as a transition): for two songs too far apart to blend, each with
/// its bars sure. The one playing's last bar before a phrase line (at the end of its main part, not into
/// its outro) is rolled over 4 bars, a build; the next one lands on the downbeat its main part starts
/// on (its intro, up to 16 bars, left out), at its own tempo.
pub fn roll(a: &Side, b: &Side, now_ms: u64) -> Option<crate::mix::Plan> {
    if !grid_trusted(a) || !grid_trusted(b) { return None; }
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    let bars = |m: &Measured| (m.rhythm.beats.len().saturating_sub(m.rhythm.downbeat)) / 4;
    let (na, nb) = (bars(ma), bars(mb));
    let (ga, gb) = (sure_groups(ma)?, sure_groups(mb)?);
    let line = body_line(ma)?;
    let first_body = ma.rhythm.bar_db.iter().position(|d| *d >= line)?;
    let last_body = ma.rhythm.bar_db.iter().rposition(|d| *d >= line)?;
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms } as f64;
    // (its end on one of its 16-beat lines: the latest by its main part's end)
    let fits = |floor: u64| (first_body.max(ROLL_BARS as usize)..=(last_body + 1).min(na.saturating_sub(1))).rev().find(|&e| {
        e % 4 == ga % 4 && bar_ms(ma, e - ROLL_BARS as usize).is_some_and(|t| t >= now_ms as f64 + 5000.0 && t >= floor as f64) && bar_ms(ma, e).is_some_and(|t| t <= end)
    });
    let end_bar = fits(earliest_ms(end as u64)).or_else(|| fits(never_before_ms(end as u64)))?;
    let (at, to) = (bar_ms(ma, end_bar - ROLL_BARS as usize)?, bar_ms(ma, end_bar)?);
    // the next one: on its main part's first downbeat, its intro (if short) left out
    let line_b = body_line(mb)?;
    // (on one of its 16-beat lines: its main part's start, else the line before it)
    let main_b = mb.rhythm.bar_db.iter().position(|d| *d >= line_b).filter(|&k| k <= ROLL_SKIP_MAX).unwrap_or(0).min(nb.saturating_sub(1));
    let main_b = (0..=main_b).rev().find(|&k| k % 4 == gb % 4).unwrap_or(gb % 4);
    let start = bar_ms(mb, main_b)?;
    let len = (to - at).round() as u64;
    Some(crate::mix::Plan {
        at_ms: at.round() as u64, ms: len + 40, swap_ms: len, start_ms: start.round() as u64, rate: 1.0, bars: ROLL_BARS, semitones: 0,
        beatmatched: false, out_tempo: 1.0, ramp_ms: 0, tape_stop: false, out_loop: None, in_loop: None, word: None, style: Some(ROLL),
    })
}

/// An extra not turned down by your ratings: under 5 rated, or most (60%) of them good
pub fn not_rejected(ratings: &[(String, bool)], kind: &str) -> bool {
    let of: Vec<bool> = ratings.iter().filter(|(k, _)| k == kind).map(|(_, g)| *g).collect();
    of.len() < 5 || of.iter().filter(|g| **g).count() * 5 >= of.len() * 3
}
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
    p.filter(|p| p.tape_stop && p.semitones == 0 && p.at_ms >= never_before_ms(end))
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
        Measured { from_ms: 0, to_ms: (beats.len() as f64 * period) as u64, rhythm: Rhythm { bpm, first_beat_ms: 0.0, period_ms: period, beats, downbeat: 0, tempo_confidence: conf, key: String::new(), camelot: camelot.into(), key_confidence: 0.8, downbeat_confidence: 1.0, model_bpm: 0.0, beat_db: Vec::new(), phrase_shift: 0, bar_db: bars.to_vec(), bar_beat: vec![1.0; bars.len()], bar_chroma: Vec::new() }, v: omnipulse_core::rhythm::VERSION }
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
        let far = song(300.0, &levels(8, 48, 8), 0.6, "8A");
        let clash = song(128.0, &levels(8, 48, 8), 0.6, "2B");
        let blurry = song(128.0, &levels(8, 48, 8), 0.1, "8A");
        let side = |m| Side { dur_ms: 150_000, rhythm: Some(m), ..Default::default() };
        assert_eq!(blend_blocked(&side(&a), &side(&near)), None);
        assert!(blend_blocked(&side(&a), &side(&far)).is_some());
        // (keys that clash: a short blend, 16 beats and the bars after, not none)
        assert!(blend_blocked(&side(&a), &side(&clash)).is_none());
        assert_eq!(blend(&side(&a), &side(&clash), 0).map(|p| p.bars), Some(4 + TAIL_BARS as u32));
        assert!(blend_blocked(&side(&a), &side(&blurry)).is_some());
        // half time counts as the same tempo (a 64 BPM grid on a 128 song)
        assert!(match_rate(128.0, 64.0).is_some());
    }

    #[test]
    fn a_blend_lines_up_every_16_beats_of_both() {
        // A: its sections change on bars 8 and 56 (lines on 0, 4, 8...); B's on 6 and 54 (lines on 2, 6...)
        let a = song(128.0, &levels(8, 48, 8), 0.6, "8A");
        let b = song(126.0, &levels(6, 48, 8), 0.6, "8A");
        assert_eq!((groups(&a).unwrap().0, groups(&b).unwrap().0), (0, 2));
        let side = |m| Side { dur_ms: 150_000, rhythm: Some(m), ..Default::default() };
        let p = blend(&side(&a), &side(&b), 0).expect("a blend");
        let bar_a: f64 = 4.0 * 60_000.0 / 128.0;
        let bar_b: f64 = 4.0 * 60_000.0 / 126.0;
        // (B in from bar 2, its intro's start on its line, its main part on the swap 16 beats on; A out
        // from bar 56, where its outro starts; then 4 bars of A's beat)
        assert_eq!(p.start_ms, (2.0 * bar_b).round() as u64);
        assert_eq!(p.at_ms, (56.0 * bar_a).round() as u64);
        assert!((p.rate - 128.0 / 126.0).abs() < 1e-6);
        // (the tempo ridden from A's to B's over all of it: A ends at B's, B at its own; its 8 bars take
        // as long as they do at the average of the two)
        // (ridden over the run-up: at B's own by the swap; the bars after it at B's tempo)
        let t = 126.0 / 128.0;
        assert!((p.out_tempo - t).abs() < 1e-6 && p.ramp_ms == p.swap_ms);
        assert_eq!(p.bars, 8);
        let run = 4.0 * bar_a / ((1.0 + t) / 2.0);
        assert!((p.swap_ms as f64 - run).abs() <= 1.0, "{} vs {run}", p.swap_ms);
        assert!((p.ms as f64 - (run + 4.0 * bar_a / t)).abs() <= 1.0);
    }

    #[test]
    fn beat_one_moved_by_hand_moves_the_bars() {
        let mut m = song(120.0, &levels(4, 40, 4), 0.6, "8A");
        m.rhythm.beat_db = m.rhythm.bar_db.iter().flat_map(|d| [*d; 4]).collect();
        let e = shifted(&m, -1);
        // (a beat earlier: from beat 3 of four; its bars' levels from its beats, the intro's end a beat sooner)
        assert_eq!(e.rhythm.downbeat, 3);
        assert_eq!(e.rhythm.beats[e.rhythm.downbeat], m.rhythm.beats[3]);
        assert!((e.rhythm.bar_db[2] - m.rhythm.bar_db[2]).abs() < 0.01 && e.rhythm.bar_db[3] > m.rhythm.bar_db[3] + 3.0);
        assert_eq!(shifted(&m, 2).rhythm.downbeat, 2);
        assert_eq!(shifted(&m, 0).rhythm, m.rhythm);
    }

    #[test]
    fn phrases_moved_by_hand_move_the_lines() {
        let mut m = song(120.0, &levels(8, 48, 8), 0.6, "8A");
        assert_eq!(groups(&m).unwrap().0, 0);
        m.rhythm.phrase_shift = -1;
        assert_eq!(groups(&m).unwrap(), (3, 1.0));
    }

    #[test]
    fn a_big_slowdown_is_a_speedup_from_half_time() {
        // 174 into 128: counted as 87, both sped up (B in at 87, to its own; A from 87's bars on)
        let a = song(174.0, &levels(16, 96, 16), 0.6, "8A");
        let b = song(128.0, &levels(8, 48, 8), 0.6, "8A");
        // (each as long as its beats)
        let side = |m: &'static Measured| Side { dur_ms: m.to_ms, rhythm: Some(m), ..Default::default() };
        let (a, b): (&'static Measured, &'static Measured) = (Box::leak(Box::new(a)), Box::leak(Box::new(b)));
        let p = blend(&side(a), &side(b), 0).expect("a blend");
        assert!((p.rate - 87.0 / 128.0).abs() < 0.01 && p.out_tempo > 1.4, "{} {}", p.rate, p.out_tempo);
    }

    #[test]
    fn a_transition_starts_in_the_last_quarter_or_as_late_as_it_fits_never_before_halfway() {
        // 120 BPM (2 s a bar), 48 bars of 96 s, its main part to bar 44: a blend in its last quarter (72 s on)
        let a = song(120.0, &levels(4, 40, 4), 0.6, "8A");
        let b = song(120.0, &levels(8, 40, 4), 0.6, "8A");
        let side = |m, d| Side { dur_ms: d, rhythm: Some(m), ..Default::default() };
        let p = blend(&side(&a, 96_000), &side(&b, b.to_ms), 0).expect("a blend");
        assert!(p.at_ms >= earliest_ms(96_000), "{p:?}");
        // a short song (14 bars, 28 s): no room after 21 s, so earlier, but from 14 s at the soonest
        let short = song(120.0, &levels(1, 12, 1), 0.6, "8A");
        if let Some(p) = blend(&side(&short, 28_000), &side(&b, b.to_ms), 0) { assert!(p.at_ms >= never_before_ms(28_000), "{p:?}"); }
        // a fade never from before halfway
        let f = fade(&side(&short, 28_000), &side(&b, b.to_ms), 60_000, 6000);
        assert!(f.out_from_ms >= 14_000, "{f:?}");
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
    fn a_roll_ends_on_a_phrase_line_and_lands_on_the_next_ones_main_part() {
        // 120 BPM (2 s a bar): A's main part bars 4..44, its outro after; B's main part from bar 2
        let a = song(120.0, &levels(4, 40, 4), 0.6, "8A");
        let b = song(140.0, &levels(2, 40, 2), 0.6, "3B");
        let p = roll(&Side { dur_ms: 100_000, rhythm: Some(&a), ..Default::default() }, &Side { dur_ms: 90_000, rhythm: Some(&b), ..Default::default() }, 0).expect("a roll");
        // (its end: bar 44, the main part's end, on a phrase line from bar 4; 4 bars before it)
        assert_eq!((p.at_ms, p.swap_ms, p.style), (80_000, 8_000, Some(ROLL)));
        assert_eq!(p.start_ms, (2.0 * 4.0 * 60_000.0 / 140.0_f64).round() as u64);
        let h = handoff(&p, true);
        assert_eq!(h.roll, Some(omnipulse_audio::Roll { bar: 2_000, bars: 4 }));
        assert!(not_rejected(&[], ROLL));
    }

    #[test]
    fn sung_only_where_its_lines_are() {
        let l = vec![(10_000, "a".to_string()), (14_000, "b".to_string()), (60_000, "c".to_string())];
        assert!(sings(&l, 12_000, 13_000));
        // (a gap after a line: past 8 s of it, not sung)
        assert!(!sings(&l, 30_000, 50_000));
        assert!(!sings(&[], 0, 100_000));
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
            Ok(id) => rt.block_on(omnipulse_core::soundcloud::api_get(format!("/tracks/{id}"))).ok().and_then(|v| Track::soundcloud(&v)).map(|mut t| { t.key = s.to_string(); t }).unwrap_or(Track { src: Src::SoundCloud(id), key: s.to_string(), ..Default::default() }),
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
        // (OMNIPULSE_SMART_VOICES=0: the voices not handed over, to compare)
        let voices = std::env::var("OMNIPULSE_SMART_VOICES").map_or(true, |v| v != "0");
        for (n, pair) in pairs.split(';').filter(|p| !p.trim().is_empty()).enumerate() {
            let (a_id, b_id) = pair.split_once(',').expect("a pair: A,B");
            let (ta, tb) = (track(a_id.trim()), track(b_id.trim()));
            let ((ma, ana), (mb, anb)) = (measure(&ta), measure(&tb));
            let shift_of = |id: &str| std::env::var("OMNIPULSE_SMART_SHIFTS").ok().and_then(|v| v.split(',').find_map(|x| x.split_once(':').filter(|(k, _)| k.trim() == id).and_then(|(_, n)| n.trim().parse::<i32>().ok()))).unwrap_or(0);
            let (ma, mb) = (ma.map(|m| shifted(&m, shift_of(a_id.trim()))), mb.map(|m| shifted(&m, shift_of(b_id.trim()))));
            // (OMNIPULSE_SMART_PHRASES="id:bars": where its phrases start, moved, as Track Preferences does)
            let phrase_of = |id: &str| std::env::var("OMNIPULSE_SMART_PHRASES").ok().and_then(|v| v.split(',').find_map(|x| x.split_once(':').filter(|(k, _)| k.trim() == id).and_then(|(_, n)| n.trim().parse::<i32>().ok()))).unwrap_or(0);
            let with_phrase = |m: Option<Measured>, id: &str| m.map(|mut m| { m.rhythm.phrase_shift = phrase_of(id); m });
            let (ma, mb) = (with_phrase(ma, a_id.trim()), with_phrase(mb, b_id.trim()));
            let open = |t: &Track| rt.block_on(crate::player::open_source(t, rt.handle().clone())).expect("it opens");
            let (mut sa, sb) = (open(&ta), open(&tb));
            let a_dur = sa.duration_ms().unwrap_or(0);
            let first = |an: &serde_json::Value| an["silences"].as_array().and_then(|s| s.first()).filter(|s| s[0].as_u64() == Some(0)).and_then(|s| s[1].as_u64()).unwrap_or(0);
            let a = Side { dur_ms: a_dur, audible_ms: ana["audible_ms"].as_u64().unwrap_or(0), first_ms: 0, rhythm: ma.as_ref(), stated_bpm: None };
            let b = Side { dur_ms: anb["duration_ms"].as_u64().unwrap_or(0), audible_ms: anb["audible_ms"].as_u64().unwrap_or(0), first_ms: first(&anb), rhythm: mb.as_ref(), stated_bpm: None };
            let db = |t: &Track, an: &serde_json::Value| crate::sound::loud_db(&serde_json::json!({}), t, an["lufs"].as_f64(), false);
            let dbs = (db(&ta, &ana), db(&tb, &anb));
            let blocked = blend_blocked(&a, &b);
            let forced = std::env::var_os("OMNIPULSE_SMART_FORCE").is_some();
            let mut plan = blend_as(&a, &b, 0, forced).or_else(|| std::env::var_os("OMNIPULSE_SMART_ROLL").and_then(|_| roll(&a, &b, 0)));
            // (OMNIPULSE_SMART_NUDGE: the next one started this many of its beats later (or, below 0, earlier), to listen for which is right)
            let nudge = std::env::var("OMNIPULSE_SMART_NUDGE").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(0.0);
            if let (Some(p), Some(m)) = (plan.as_mut(), mb.as_ref()) { p.start_ms = (p.start_ms as f64 + nudge * m.rhythm.period_ms).max(0.0).round() as u64; }
            // (the voices handed over only where both sing, by their timed lyrics, as the app does)
            let lyrics = |t: &Track, m: &Option<Measured>| rt.block_on(crate::player::mix_lyrics(t, m.as_ref().map_or(0, |m| m.to_ms / 1000)));
            let (la, lb) = (lyrics(&ta, &ma), lyrics(&tb, &mb));
            let both_sing = |a_from: u64, a_to: u64, b_from: u64, b_to: u64| sings(&la, a_from, a_to) && sings(&lb, b_from, b_to);
            let (out, info) = match &plan {
                Some(p) if p.style == Some(ROLL) => {
                    sa.seek(p.at_ms.saturating_sub(lead)).unwrap();
                    let mut sb = sb;
                    sb.seek(p.start_ms).unwrap();
                    let out = omnipulse_audio::engine::render_mix(sa, sb, dbs, rate, lead as f32 / 1000.0, p.ms, Some(handoff(p, false)), (lead + p.ms) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "roll", "plan": p, "notOnTheBeat": blocked }))
                }
                Some(p) => {
                    sa.seek(p.at_ms.saturating_sub(lead)).unwrap();
                    let b_tempo = omnipulse_audio::Tempo::new(p.rate);
                    let mut sb = omnipulse_audio::Stretched::shifted(sb, b_tempo.clone(), 0).unwrap();
                    omnipulse_audio::Source::seek(&mut sb, p.start_ms).unwrap();
                    let (target, a_tempo) = (p.out_tempo, omnipulse_audio::Tempo::new(1.0));
                    let share = p.ms as f64 / p.ramp_ms.max(1) as f64;
                    let ramp = { let a_tempo = a_tempo.clone(); move |x: f64| { a_tempo.set(crate::mix::out_ramp(target, x * share)); b_tempo.set(crate::mix::in_ramp(target, x * share)); } };
                    let out = omnipulse_audio::engine::render_mix_ramped(sa, Box::new(sb), dbs, rate, lead as f32 / 1000.0, p.ms, Some(handoff(p, voices && both_sing(p.at_ms, p.at_ms + p.ms, p.start_ms, p.start_ms + (p.ms as f64 * p.rate) as u64))), (target != 1.0).then_some((&a_tempo, &ramp as &dyn Fn(f64))), (lead + p.ms) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "smart blend", "plan": p, "nudgeBeats": nudge, "voicesHanded": voices && both_sing(p.at_ms, p.at_ms + p.ms, p.start_ms, p.start_ms + (p.ms as f64 * p.rate) as u64), "lyricLines": [la.len(), lb.len()] }))
                }
                None => {
                    let f = fade(&a, &b, max_ms, FIXED_MS);
                    sa.seek(f.out_from_ms.saturating_sub(lead)).unwrap();
                    let mut sb = sb;
                    if f.in_from_ms > 0 { sb.seek(f.in_from_ms).unwrap(); }
                    let out = omnipulse_audio::engine::render_mix(sa, sb, dbs, rate, lead as f32 / 1000.0, f.len_ms(), fade_handoff(f.len_ms(), voices && both_sing(f.out_from_ms, f.out_to_ms, f.in_from_ms, f.in_from_ms + f.len_ms())), (lead + f.len_ms()) as f32 / 1000.0 + 15.0).unwrap();
                    (out, serde_json::json!({ "kind": "smart fade", "outFromMs": f.out_from_ms, "outToMs": f.out_to_ms, "inFromMs": f.in_from_ms, "ms": f.len_ms(), "notOnTheBeat": blocked.unwrap_or("no phrase to blend on"),
                        "outroFromMs": ma.as_ref().and_then(outro_from), "introToMs": mb.as_ref().and_then(intro_to) }))
                }
            };
            let side = |m: &Option<Measured>| m.as_ref().map(|m| serde_json::json!({ "bpm": m.rhythm.bpm, "confidence": m.rhythm.tempo_confidence, "camelot": m.rhythm.camelot, "keyConfidence": m.rhythm.key_confidence, "downbeatConfidence": m.rhythm.downbeat_confidence, "groups": groups(m) }));
            let info = serde_json::json!({ "a": a_id, "b": b_id, "titles": [format!("{} - {}", ta.artist, ta.title), format!("{} - {}", tb.artist, tb.title)], "aMeasured": side(&ma), "bMeasured": side(&mb), "dbs": [dbs.0, dbs.1], "startsAt": "12 s in", "transition": info });
            let base = dir.join(format!("{:02}", n + 1));
            write_wav(&base.with_extension("wav"), &out, rate);
            std::fs::write(base.with_extension("json"), serde_json::to_string_pretty(&info).unwrap()).unwrap();
            println!("{}: {}", base.display(), info["transition"]);
        }
    }

    /// How well a blend's beats line up as heard: each side rendered alone over the blend (the next one
    /// stretched and started as the plan has it), their kicks (below 150 Hz, where they rise) compared a
    /// bar at a time: the lag (ms) at which they match best, B against A (+: B late). A steady lag is an
    /// offset; one that grows, a tempo off.
    /// OMNIPULSE_SMART_PAIRS="A,B;..." cargo test --release -p omnipulse --bin OmniPulse-native blend_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn blend_probe() {
        use crate::track::{Src, Track};
        let rt = tokio::runtime::Runtime::new().unwrap();
        let rate = 44_100u32;
        for pair in std::env::var("OMNIPULSE_SMART_PAIRS").unwrap().split(';') {
            let (a_id, b_id) = pair.split_once(',').unwrap();
            let t = |id: &str| Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.to_string(), ..Default::default() };
            let (ta, tb) = (t(a_id), t(b_id));
            let full = |t: &Track| omnipulse_core::rhythm::known(&format!("full:{}", t.key)).expect("measured whole (run smart_render first)");
            let (ma, mb) = (full(&ta), full(&tb));
            let an = |id: &str| rt.block_on(omnipulse_core::loudness::analyse("soundcloud", id)).unwrap_or_default();
            let (ana, anb) = (an(a_id), an(b_id));
            let a = Side { dur_ms: ana["duration_ms"].as_u64().unwrap_or(0), audible_ms: ana["audible_ms"].as_u64().unwrap_or(0), rhythm: Some(&ma), ..Default::default() };
            let b = Side { dur_ms: anb["duration_ms"].as_u64().unwrap_or(0), audible_ms: anb["audible_ms"].as_u64().unwrap_or(0), rhythm: Some(&mb), ..Default::default() };
            let Some(p) = blend(&a, &b, 0) else { println!("{pair}: no blend ({:?})", blend_blocked(&a, &b)); continue };
            let open = |t: &Track| rt.block_on(crate::player::open_source(t, rt.handle().clone())).expect("it opens");
            let secs = p.ms as f32 / 1000.0;
            let mut sa = open(&ta);
            sa.seek(p.at_ms).unwrap();
            let pa = omnipulse_audio::engine::render(sa, rate, &[0.0; 10], 0.0, secs).unwrap();
            // (OMNIPULSE_PROBE_CONTROL: B is A itself, a bar on: its kicks should match at 0)
            let control = std::env::var_os("OMNIPULSE_PROBE_CONTROL").is_some();
            let (tb, start, r) = if control { (ta.clone(), bar_ms(&ma, (ma.rhythm.beats.iter().position(|x| ma.from_ms as f64 + x >= p.at_ms as f64).unwrap_or(0).saturating_sub(ma.rhythm.downbeat)) / 4 + 1).unwrap().round() as u64, 1.0) } else { (tb.clone(), p.start_ms, p.rate) };
            let mut sb = omnipulse_audio::Stretched::shifted(open(&tb), omnipulse_audio::Tempo::new(r), 0).unwrap();
            omnipulse_audio::Source::seek(&mut sb, start).unwrap();
            let pb = omnipulse_audio::engine::render(Box::new(sb), rate, &[0.0; 10], 0.0, secs).unwrap();
            // (each one's kicks, as heard: its lows' envelope (a millisecond at a time, smoothed over 8), each
            // sharp rise (half the strong ones' at least, 150 ms apart), timed where it's halfway up)
            let kicks = |x: &[f32]| -> Vec<f64> {
                let k = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / rate as f32).exp();
                let (mut z1, mut z2) = (0.0f32, 0.0f32);
                let raw: Vec<f32> = x.chunks(2 * 44).map(|c| c.chunks(2).map(|f| { z1 += k * ((f[0] + f[1]) * 0.5 - z1); z2 += k * (z1 - z2); z2 * z2 }).sum::<f32>()).collect();
                let n = raw.len();
                let e: Vec<f32> = (0..n).map(|i| { let (a, b) = (i.saturating_sub(4), (i + 4).min(n)); raw[a..b].iter().sum::<f32>() / (b - a) as f32 }).collect();
                let rise: Vec<f32> = (0..n).map(|i| if i >= 2 && i + 2 < n { e[i + 2] - e[i - 2] } else { 0.0 }).collect();
                let mut rs = rise.clone();
                rs.sort_by(|a, b| a.total_cmp(b));
                let line = rs[rs.len() * 995 / 1000] * 0.4;
                let mut out: Vec<f64> = Vec::new();
                let mut i = 2;
                while i + 30 < n {
                    if rise[i] >= line && (i..i + 20).all(|j| rise[j] <= rise[i] || j == i) {
                        let base = e[i.saturating_sub(15)..=i].iter().copied().fold(f32::MAX, f32::min);
                        let top = e[i..i + 30].iter().copied().fold(0.0, f32::max);
                        let at = (i.saturating_sub(15)..i + 30).find(|&j| e[j] >= base + (top - base) * 0.5).unwrap_or(i);
                        out.push(at as f64 * 44.0 * 1000.0 / rate as f64);
                        i += 150;
                    } else { i += 1; }
                }
                out
            };
            // (over the whole blend at once: each one's onsets, its lows' and its highs' rises (above 4 kHz:
            // the hats), a millisecond at a time, and the lag where they match best)
            let onsets = |x: &[f32]| -> Vec<f32> {
                let kl = 1.0 - (-2.0 * std::f32::consts::PI * 150.0 / rate as f32).exp();
                let kh = 1.0 - (-2.0 * std::f32::consts::PI * 4000.0 / rate as f32).exp();
                let (mut l1, mut l2, mut h1) = (0.0f32, 0.0f32, 0.0f32);
                let (lo, hi): (Vec<f32>, Vec<f32>) = x.chunks(2 * 44).map(|c| c.chunks(2).fold((0.0, 0.0), |(a, b), f| {
                    let m = (f[0] + f[1]) * 0.5;
                    l1 += kl * (m - l1); l2 += kl * (l1 - l2); h1 += kh * (m - h1);
                    (a + l2 * l2, b + (m - h1) * (m - h1))
                })).unzip();
                let rise = |e: &[f32]| -> Vec<f32> { let l: Vec<f32> = e.iter().map(|v| (v + 1e-9).ln()).collect(); (0..l.len()).map(|i| if i >= 4 { (l[i] - l[i - 4]).max(0.0) } else { 0.0 }).collect() };
                let (a, b) = (rise(&lo), rise(&hi));
                a.iter().zip(&b).map(|(x, y)| x + y).collect()
            };
            let (oa, ob) = (onsets(&pa), onsets(&pb));
            let n = oa.len().min(ob.len());
            let corr = |l: i64| (100..n - 100).map(|i| oa[i] * ob[(i as i64 + l) as usize]).sum::<f32>();
            let curve: Vec<(i64, f32)> = (-80..=80).map(|l| (l, corr(l))).collect();
            let best = curve.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
            let zero = curve.iter().find(|c| c.0 == 0).unwrap().1;
            println!("{pair}: whole blend, best lag {} ms (its match {:.0}, at 0 ms {:.0})", best.0, best.1, zero);
            let (ka, kb) = (kicks(&pa), kicks(&pb));
            let d: Vec<i64> = ka.iter().filter_map(|a| kb.iter().map(|b| b - a).min_by(|x, y| x.abs().total_cmp(&y.abs())).filter(|x| x.abs() < 120.0).map(|x| x.round() as i64)).collect();
            let mut sorted = d.clone();
            sorted.sort();
            println!("{pair}: rate {:.4}, kicks A {} B {}, B against A (ms, +: B late), median {:?}: {d:?}", p.rate, ka.len(), kb.len(), sorted.get(sorted.len() / 2));
        }
    }

    /// Each song measured again (as the app measures it whole), its downbeat's clues shown
    /// OMNIPULSE_RHYTHM_DEBUG=1 OMNIPULSE_PROBE_IDS=1,2 cargo test --release -p omnipulse --bin OmniPulse-native rhythm_probe -- --ignored --nocapture
    #[test]
    #[ignore]
    fn rhythm_probe() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for id in std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',') {
            let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)).unwrap();
            eprintln!("{id}:");
            let r = omnipulse_core::rhythm::analyse(&mono, rate).unwrap();
            eprintln!("  {} BPM, downbeat {}, first beats {:?}", r.bpm, r.downbeat, &r.beats[..4]);
        }
    }

    /// What measuring a song costs: wall time, and the CPU time all its threads took (Windows)
    /// OMNIPULSE_PROBE_IDS=1 cargo test --release -p omnipulse --bin OmniPulse-native rhythm_cost -- --ignored --nocapture
    #[test]
    #[ignore]
    fn rhythm_cost() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let cpu = || -> f64 {
            #[cfg(windows)]
            unsafe {
                #[repr(C)] #[derive(Default, Clone, Copy)] struct Ft { lo: u32, hi: u32 }
                #[link(name = "kernel32")] extern "system" { fn GetCurrentProcess() -> isize; fn GetProcessTimes(h: isize, c: *mut Ft, e: *mut Ft, k: *mut Ft, u: *mut Ft) -> i32; }
                let (mut c, mut e, mut k, mut u) = (Ft::default(), Ft::default(), Ft::default(), Ft::default());
                GetProcessTimes(GetCurrentProcess(), &mut c, &mut e, &mut k, &mut u);
                let t = |f: Ft| ((f.hi as u64) << 32 | f.lo as u64) as f64 / 1e7;
                return t(k) + t(u);
            }
            #[allow(unreachable_code)] 0.0
        };
        for id in std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',') {
            let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)).unwrap();
            let secs = mono.len() as f64 / rate as f64;
            // (once to load the model, then measured)
            let _ = omnipulse_core::rhythm::analyse(&mono[..(rate as usize * 10).min(mono.len())], rate);
            let (c0, t0) = (cpu(), std::time::Instant::now());
            let _ = omnipulse_core::rhythm::analyse(&mono, rate);
            let (c1, t1) = (cpu(), t0.elapsed().as_secs_f64());
            eprintln!("{id}: {secs:.0} s of song: {t1:.2} s to measure, {:.2} s of CPU ({:.1} cores while it ran), {} cores here", c1 - c0, (c1 - c0) / t1, std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0));
        }
    }

    /// The tempo heard in a render, a stretch at a time: OMNIPULSE_WAV=file OMNIPULSE_SPANS="2-10,20-27"
    #[test]
    #[ignore]
    fn wav_tempo() {
        let b = std::fs::read(std::env::var("OMNIPULSE_WAV").unwrap()).unwrap();
        let x: Vec<f32> = b[44..].chunks(4).map(|f| (i16::from_le_bytes([f[0], f[1]]) as f32 + i16::from_le_bytes([f[2], f[3]]) as f32) / 65536.0).collect();
        for span in std::env::var("OMNIPULSE_SPANS").unwrap().split(',') {
            let (a, b) = span.split_once('-').unwrap();
            let (a, b) = (a.parse::<f64>().unwrap(), b.parse::<f64>().unwrap());
            let part = &x[(a * 44_100.0) as usize..((b * 44_100.0) as usize).min(x.len())];
            let r = omnipulse_core::rhythm::analyse(part, 44_100).unwrap();
            // (its tempo from its beats' spacing, not rounded)
            let n = r.beats.len();
            eprintln!("{span} s: {:.2} BPM ({} beats, confidence {:.2})", 60_000.0 * (n - 1) as f64 / (r.beats[n - 1] - r.beats[0]), n, r.tempo_confidence);
        }
    }

    /// A SoundCloud track's own details (its bpm, genre, description), as its API gives them
    #[test]
    #[ignore]
    fn sc_details() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for id in std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',') {
            let v = rt.block_on(omnipulse_core::soundcloud::api_get(format!("/tracks/{id}"))).unwrap_or_default();
            eprintln!("{id}: {} | bpm {} | genre {} | tags {} | {}", v["title"], v["bpm"], v["genre"], v["tag_list"], v["description"].as_str().unwrap_or("").chars().take(300).collect::<String>().replace(['\n', '\r'], " "));
        }
    }

    /// Memory as songs are measured one after another (Windows: the process's working set and private
    /// bytes after each): OMNIPULSE_PROBE_IDS=1,2,3...
    #[test]
    #[ignore]
    fn rhythm_memory() {
        #[cfg(windows)]
        fn mem() -> (f64, f64) {
            #[repr(C)] #[derive(Default)] struct Pmc { cb: u32, faults: u32, peak_ws: usize, ws: usize, qpp: usize, qp: usize, qnpp: usize, qnp: usize, pagefile: usize, peak_pagefile: usize, private: usize }
            #[link(name = "kernel32")] extern "system" { fn GetCurrentProcess() -> isize; fn K32GetProcessMemoryInfo(h: isize, p: *mut Pmc, cb: u32) -> i32; }
            let mut p = Pmc { cb: std::mem::size_of::<Pmc>() as u32, ..Default::default() };
            unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut p, p.cb); }
            (p.ws as f64 / 1e6, p.private as f64 / 1e6)
        }
        let rt = tokio::runtime::Runtime::new().unwrap();
        let ids: Vec<String> = std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',').map(String::from).collect();
        eprintln!("start: {:?} MB (working set, private)", mem());
        for round in 0..2 {
            for id in &ids {
                let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)).unwrap();
                let secs = mono.len() / rate as usize;
                let _ = omnipulse_core::rhythm::analyse(&mono, rate);
                drop(mono);
                eprintln!("round {round}, {id} ({secs} s): {:?} MB", mem());
            }
        }
    }

    /// Why pairs of the songs measured so far would blend or not (state/rhythm.json): each measured song
    /// into the next one measured, counted by reason
    #[test]
    #[ignore]
    fn blend_census() {
        let d = omnipulse_core::data_dir().unwrap().join("state").join("rhythm.json");
        let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(d).unwrap()).unwrap();
        let songs: Vec<(String, Measured)> = v.as_object().unwrap().iter().filter(|(k, _)| k.starts_with("full:"))
            .filter_map(|(k, x)| serde_json::from_value::<Measured>(x.clone()).ok().filter(|m| m.v == omnipulse_core::rhythm::VERSION).map(|m| (k.clone(), m))).collect();
        println!("{} songs measured whole at this version", songs.len());
        let mut why: std::collections::BTreeMap<String, usize> = Default::default();
        let mut n = 0;
        for (i, (_, a)) in songs.iter().enumerate() {
            for (_, b) in songs.iter().skip(i + 1).take(25) {
                let sa = Side { dur_ms: a.to_ms, rhythm: Some(a), ..Default::default() };
                let sb = Side { dur_ms: b.to_ms, rhythm: Some(b), ..Default::default() };
                let r = match blend_blocked(&sa, &sb) { Some(w) => w.to_string(), None => if blend(&sa, &sb, 0).is_some() { "blends".into() } else { "no line to start on".into() } };
                *why.entry(r).or_default() += 1;
                n += 1;
            }
        }
        for (k, c) in &why { println!("  {k}: {c} ({:.0}%)", *c as f64 * 100.0 / n as f64); }
        // and each song's own checks, alone
        let mut own: std::collections::BTreeMap<&str, usize> = Default::default();
        for (_, m) in &songs {
            let r = &m.rhythm;
            let s = Side { dur_ms: m.to_ms, rhythm: Some(m), ..Default::default() };
            let k = if r.tempo_confidence < TRUST_CONFIDENCE { "tempo unclear" } else if r.model_bpm > 0.0 && ![1.0, 2.0, 0.5].iter().any(|x| ((r.bpm * x) / r.model_bpm - 1.0).abs() <= STATED_WITHIN) { "model tempo disagrees" } else if r.downbeat_confidence < omnipulse_core::rhythm::DOWNBEAT_TRUSTED { "beat 1 unsure" } else if !grid_trusted(&s) { "other" } else { "trusted" };
            *own.entry(k).or_default() += 1;
        }
        println!("each song alone: {own:?}");
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
