// Seamless Mix's planning (what we called Smart Transitions while it was built; docs/dj-transitions.md),
// shared by the desktop app and the Android app: what DJ software does, and
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
    /// its key's from outside (SongBPM: Spotify's analysis), not only our own reading: one to shift a
    /// song's pitch by
    pub key_known: bool,
    /// its timed lyrics (ms, line), where they're known: where it sings, so a blend doesn't cut a line off
    pub lines: &'a [(u64, String)],
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
pub fn bar_ms(m: &Measured, k: usize) -> Option<f64> {
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
    if std::env::var_os("OMNIPULSE_SMART_OLDLEN").is_none() {
        if let (Some(ma), Some(mb)) = (a.rhythm, b.rhythm) { return fade_fitted(a, b, ma, mb, end, max_ms); }
    }
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

/// A soft part this long or more is one to fit a fade to (an outro, an intro); shorter, the song ends cold
/// or starts loud
const FIT_SOFT_MS: u64 = 3000;
/// The one playing's outro cut short for a next one that starts loud: over this
const LOUD_IN_MS: u64 = 3000;

/// A fade fitted to the two songs' structure, as a DJ fits one (2026-10-17; before: over the longer of
/// the outro and the intro, the next one always from its start):
/// - an outro and an intro: over the shorter of the two, the next one started so its main part lands
///   as the one playing's sound ends (its intro's start left out if it's the longer);
/// - a cold end into an intro: a short overlap at the end, the intro heard whole after;
/// - an outro into a loud start: the next one in where the one playing's main part ends, over 3 s,
///   not after its outro has run out;
/// - a cold end into a loud start: a short overlap, the two loud parts barely together.
/// Never longer than Longest Fade (`max_ms`), and in the one playing's last quarter.
fn fade_fitted(a: &Side, b: &Side, ma: &Measured, mb: &Measured, end: u64, max_ms: u64) -> Fade {
    let main_end = outro_from(ma).unwrap_or(end).min(end);
    let main_in = intro_to(mb).unwrap_or(b.first_ms).max(b.first_ms);
    let (outro, intro) = (end.saturating_sub(main_end), main_in.saturating_sub(b.first_ms));
    let cap = max_ms.max(MIN_FADE_MS);
    let floor = earliest_ms(end);
    let f = match (outro >= FIT_SOFT_MS, intro >= FIT_SOFT_MS) {
        (true, true) => {
            let len = outro.min(intro).min(cap);
            Fade { out_from_ms: end - len, out_to_ms: end, in_from_ms: main_in - len }
        }
        (false, true) => Fade { out_from_ms: end.saturating_sub(HARD_MS), out_to_ms: end, in_from_ms: b.first_ms },
        (true, false) => {
            let from = main_end.max(floor);
            Fade { out_from_ms: from, out_to_ms: (from + LOUD_IN_MS.min(cap)).min(end), in_from_ms: b.first_ms }
        }
        (false, false) => Fade { out_from_ms: end.saturating_sub(HARD_MS), out_to_ms: end, in_from_ms: b.first_ms },
    };
    // (in its last quarter; and never shorter than a fade's shortest)
    let from = f.out_from_ms.max(floor).min(f.out_to_ms.saturating_sub(MIN_FADE_MS.min(f.out_to_ms)));
    let _ = a;
    Fade { out_from_ms: from, ..f }
}

// ---------- Tier 1: when a blend on the beat is safe ----------
/// A beat clear enough to trust (core::rhythm's tempo_confidence)
const TRUST_CONFIDENCE: f64 = 0.3;
/// How far from its own tempo the next one plays (DJs: mostly under 5%)
pub const MAX_RATE: f64 = 0.04;
/// How far apart two songs' tempos can be for a blend with the tempo ridden from one to the other (as a
/// DJ ramps between genres: 87 into 120, 105 into 128), the faster over the slower; past 4%, the blend's
/// always 32 beats, room for it. (Your ratings, 2026-10-17: every ride of 46% or more was bad, the good
/// ones 22% or less; 50% before. 2026-10-30: 22% and 25% good, 25% and both of 36% bad: 40% before.)
pub const MAX_RAMP_BLEND: f64 = 0.30;
/// A slowdown up to this stays one (Goodbye's 36% sounded wrong); past it, the one playing is counted at half time and sped up instead
/// (Shooting Stars' 128 into Rocket Girl's 105 was counted as 64 and sped up 64%: rated bad)
const MAX_SLOWDOWN: f64 = 0.25;
/// The ride between two tempos: the faster over the slower, less one (a slowdown and a speed-up alike)
fn ride(a_bpm: f64, b_bpm: f64) -> f64 {
    if a_bpm <= 0.0 || b_bpm <= 0.0 { return f64::INFINITY; }
    (a_bpm / b_bpm).max(b_bpm / a_bpm) - 1.0
}

/// Match Keys (Seamless Mix's page): a blend of keys that clash ridden into one key; off, never shifted
pub static KEYS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// The key shift (semitones, up to two either way) that puts the next one's key beside the playing
/// one's on the Camelot wheel: where the next one starts, both then ridden to the next one's own key
/// over the run-up, as the tempo is. 0: they go together already, either key isn't clear, or Match
/// Keys is off
pub fn key_ride(a: &Measured, b: &Measured) -> i32 {
    use omnipulse_core::rhythm::{keys_mix, transposed};
    if !KEYS.load(std::sync::atomic::Ordering::Relaxed) || keys_go(a, b) { return 0; }
    [1, -1, 2, -2].into_iter().find(|&s| transposed(&b.rhythm.key, s).is_some_and(|(_, c)| keys_mix(&a.rhythm.camelot, &c))).unwrap_or(0)
}
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
    let near = |x: f64| ride(x, mb.rhythm.bpm) <= MAX_RAMP_BLEND;
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
pub fn blend(a: &Side, b: &Side, now_ms: u64) -> Option<crate::plan::Plan> {
    blend_as(a, b, now_ms, false)
}

/// The same, `forced` (for listening only: no pair refused, however far apart or unsure; the
/// renderer's OMNIPULSE_SMART_FORCE)
pub fn blend_as(a: &Side, b: &Side, now_ms: u64, forced: bool) -> Option<crate::plan::Plan> {
    let why = |w: &str| if std::env::var_os("OMNIPULSE_MIX_DEBUG").is_some() { eprintln!("smart debug: no blend: {w}"); };
    if !forced && blend_blocked(a, b).is_some() { return None; }
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    // (a big slowdown sounds wrong: the one playing counted at half time instead (174 as 87) and sped up)
    let half;
    let ma = if ma.rhythm.bpm > mb.rhythm.bpm * (1.0 + MAX_SLOWDOWN) && ride(ma.rhythm.bpm / 2.0, mb.rhythm.bpm) <= MAX_RAMP_BLEND && ma.rhythm.bpm / 2.0 < mb.rhythm.bpm {
        half = halved(ma);
        &half
    } else { ma };
    // (same time only: a half- or double-time pair's bars don't line up one to one)
    let ramp = ride(ma.rhythm.bpm, mb.rhythm.bpm);
    if ramp > MAX_RAMP_BLEND && !forced { return None; }
    // (a big slowdown that couldn't be a speed-up from half time: a fade, not a slowdown that sounds wrong)
    if ma.rhythm.bpm > mb.rhythm.bpm * (1.0 + MAX_SLOWDOWN) && !forced { return None; }
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
    // (a big change of tempo: 32 beats for it, whatever the intro; keys that clash: ridden into one key,
    // else 16 beats, the clash brief)
    // (only by keys known from outside: our own readings are a fifth out too often to shift a song by)
    let semitones = if a.key_known && b.key_known { key_ride(ma, mb) } else { 0 };
    let clash = !keys_go(ma, mb) && semitones == 0;
    // the one playing: its main part's end (its outro from there), and its sound's
    let line = body_line(ma)?;
    let outro = ma.rhythm.bar_db.iter().rposition(|d| *d >= line)? + 1;
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms } as f64;
    let sound_bars = (0..=na).rev().find(|&k| bar_ms(ma, k).is_some_and(|t| t <= end)).unwrap_or(na);
    let a_out = sound_bars.saturating_sub(outro);
    // how long they're together, and the one playing's tail after: by the two songs' structure
    let (run, tail) = if std::env::var_os("OMNIPULSE_SMART_OLDLEN").is_some() {
        // (as before 2026-10-17, for comparing: 8 bars for any change of tempo, 4 of tail)
        (if clash { 4 } else if ramp > MAX_RATE { 8 } else { intro.clamp(4, 8) }, TAIL_BARS)
    } else {
        blend_lengths(intro, a_out, ramp, clash, ma.rhythm.period_ms)
    };
    let from = if intro > run { s0 + intro - run } else { s0 };
    if from + run + tail >= nb { return None; }
    // the one playing: as late as the blend fits before its sound ends, so it's heard out (its outro whole,
    // the next one's intro under it; your ratings, 2026-10-30: the bad blends cut twice as much off its end
    // as the good ones, about 11 s against 6); of the latest few lines it fits on, the latest where its
    // singer isn't cut off mid-line as it fades after the swap (by its timed lyrics, where they're known).
    // At least 5 s from now, in its last quarter; else as late as it fits, but never before halfway.
    // (OMNIPULSE_SMART_OLDPLACE, for comparing: from the line its outro starts on, as before)
    let old = std::env::var_os("OMNIPULSE_SMART_OLDPLACE").is_some();
    let ok = |k: usize, floor: u64| on(k, ga) && k + run + tail <= na && bar_ms(ma, k).is_some_and(|t| t >= now_ms as f64 + 5000.0 && t >= floor as f64) && bar_ms(ma, k + run + tail).is_some_and(|t| t <= end);
    let voice_kept = |k: usize| match (bar_ms(ma, k + run), bar_ms(ma, k + run + tail)) {
        (Some(sw), Some(ov)) => !sings(a.lines, sw as u64, ov as u64),
        _ => true,
    };
    let fits = |floor: u64| -> Option<usize> {
        let top = if old { outro.min(na) } else { na };
        let mut lines = (0..=top).rev().filter(|&k| ok(k, floor));
        if old { return lines.next(); }
        let latest: Vec<usize> = lines.take(VOICE_LOOK_LINES).collect();
        latest.iter().copied().find(|&k| voice_kept(k)).or(latest.first().copied())
    };
    let Some(k) = fits(earliest_ms(end as u64)).or_else(|| fits(never_before_ms(end as u64))) else { why(&format!("no line to start on before its outro (bar {outro} of {na})")); return None };
    let (at, swap, over) = (bar_ms(ma, k)?, bar_ms(ma, k + run)?, bar_ms(ma, k + run + tail)?);
    // its tempo as it is over the run-up (a song's drifts), the next one's over its own
    let (pa, pb) = ((swap - at) / (run * 4) as f64, (bar_ms(mb, from + run)? - bar_ms(mb, from)?) / (run * 4) as f64);
    let rate = pb / pa;
    if ride(1.0, rate) > MAX_RAMP_BLEND && !forced { return None; }
    // the tempo ridden over all of it, as a DJ does: the two together at the one playing's at first, at
    // the next one's own by the end (the one playing eased to it, the next one with it): nothing to ease
    // back after. Its bars take longer or less as it's eased: the times, as heard
    // (ridden over the run-up: at the next one's own tempo by the swap, where the one playing stops being
    // heard; its last bars, fading, at that tempo too)
    let target = 1.0 / rate;
    let swap_at = ramp_time(swap - at, swap - at, target);
    let len = swap_at + (over - swap) / target;
    Some(crate::plan::Plan {
        at_ms: at.round() as u64, ms: len.round() as u64, swap_ms: swap_at.round() as u64, start_ms: bar_ms(mb, from)?.round() as u64,
        rate, bars: (run + tail) as u32, semitones, beatmatched: true, out_tempo: target, ramp_ms: swap_at.round() as u64, tape_stop: false, roll_speed: 1.0,
        style: None, tail_bars: tail as u32,
    })
}

/// How many of the latest lines a blend fits on are looked through for one that doesn't cut the one playing's
/// singer off (a line further back cuts more of its end)
const VOICE_LOOK_LINES: usize = 3;

/// The longest the two are together (seconds); past it, a phrase less
const TOGETHER_MAX_S: f64 = 24.0;
/// How steep a change of tempo can be ridden: this much a bar
const RIDE_PER_BAR: f64 = 0.05;

/// How long a blend's run-up is (bars, the two together) and the one playing's tail after the swap, by
/// the two songs' structure, as a DJ fits them: the next one's intro (whole phrases of 4 bars), so its
/// main part lands on the swap; no longer than the one playing has to give under it (its outro, and a
/// phrase of its main part); long enough to ride their tempos at most 5% a bar; at most 24 s together.
/// The tail: what's left of its outro, 1 to 4 bars. (Before 2026-10-17: 8 bars for any change of tempo,
/// whatever the songs, and 4 of tail: 23 to 33 s, as Us into Spotless, a 1-bar intro, took 24 s.)
pub fn blend_lengths(intro: usize, a_out: usize, ramp: f64, clash: bool, period_ms: f64) -> (usize, usize) {
    let phrases = |b: usize| b.div_ceil(4) * 4;
    let mut run = if clash { 4 } else { intro.clamp(4, 16) };
    run = run.min((phrases(a_out) + 4).max(4));
    if ramp > MAX_RATE { run = run.max(phrases((ramp / RIDE_PER_BAR).ceil() as usize)).min(16); }
    while run > 4 && run as f64 * 4.0 * period_ms > TOGETHER_MAX_S * 1000.0 { run -= 4; }
    (run, a_out.clamp(1, TAIL_BARS))
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

/// Where it's mixed sure, when the whole song isn't: its end's downbeat for the one going out (`end`), its
/// start's for the next one, if that part's is sure (its bars counted from there, and as sure as it is)
pub fn local_downbeat(m: &Measured, end: bool) -> Measured {
    let r = &m.rhythm;
    let part = if end { r.tail_downbeat } else { r.head_downbeat };
    match part {
        Some((d, c)) if r.downbeat_confidence < omnipulse_core::rhythm::DOWNBEAT_TRUSTED && c >= omnipulse_core::rhythm::DOWNBEAT_TRUSTED => {
            let n = (d as i32 - r.downbeat as i32).rem_euclid(4);
            let mut s = shifted(m, n);
            s.rhythm.downbeat_confidence = c;
            s
        }
        _ => m.clone(),
    }
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
pub fn handoff(p: &crate::plan::Plan, voices: bool) -> omnipulse_audio::Handoff {
    if p.tape_stop { return omnipulse_audio::Handoff::tape(p.swap_ms, p.ramp_ms); }
    if p.style == Some(ROLL) { return omnipulse_audio::Handoff::roll(p.swap_ms / p.bars.max(1) as u64, p.bars, p.roll_speed as f32); }
    // (the one playing's beat carried: the next one rising under it, the bass swapped, then its top fading;
    // the voices, if both sing there, the one playing's faded over the run-up's second half, the next
    // one's up over the bars after the swap)
    let tail = if p.tail_bars > 0 { p.tail_bars as usize } else { TAIL_BARS };
    let beat = p.swap_ms as f64 / ((p.bars as usize).saturating_sub(tail).max(1) * 4) as f64;
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
/// How far under the main part's level the bar a roll keeps can be (dB): an outro with something in it
const ROLL_QUIET_DB: f32 = 6.0;

/// A roll out (Traktor's Beatmasher, as a transition): for two songs too far apart to blend, each with
/// its bars sure. The one playing's bar before its last phrase line (by its end, the bar kept not near
/// silent: its outro played, if it has something in it) is rolled over 4 bars, a build, its speed ridden
/// to the next one's tempo meanwhile; the next one lands on the downbeat its main part starts on (its
/// intro, up to 16 bars, left out), at its own tempo.
pub fn roll(a: &Side, b: &Side, now_ms: u64) -> Option<crate::plan::Plan> {
    if !grid_trusted(a) || !grid_trusted(b) { return None; }
    let (ma, mb) = (a.rhythm?, b.rhythm?);
    let bars = |m: &Measured| (m.rhythm.beats.len().saturating_sub(m.rhythm.downbeat)) / 4;
    let (na, nb) = (bars(ma), bars(mb));
    let (ga, gb) = (sure_groups(ma)?, sure_groups(mb)?);
    let line = body_line(ma)?;
    let first_body = ma.rhythm.bar_db.iter().position(|d| *d >= line)?;
    let end = if a.audible_ms > 0 && a.audible_ms < a.dur_ms { a.audible_ms } else { a.dur_ms } as f64;
    // (the bar it keeps and rolls: something in it, not the near-silence at a song's very end)
    let heard = |k: usize| ma.rhythm.bar_db.get(k).is_some_and(|d| *d >= line - ROLL_QUIET_DB);
    // (its end on one of its 16-beat lines: the latest by the song's end)
    let fits = |floor: u64| (first_body.max(ROLL_BARS as usize)..=na.saturating_sub(1)).rev().find(|&e| {
        e % 4 == ga % 4 && heard(e - ROLL_BARS as usize) && bar_ms(ma, e - ROLL_BARS as usize).is_some_and(|t| t >= now_ms as f64 + 5000.0 && t >= floor as f64) && bar_ms(ma, e).is_some_and(|t| t <= end)
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
    // (its loop's speed by its end: the next one's tempo, as each was measured, within reason)
    let speed = if ma.rhythm.bpm > 0.0 && mb.rhythm.bpm > 0.0 { (mb.rhythm.bpm / ma.rhythm.bpm).clamp(0.5, 2.0) } else { 1.0 };
    Some(crate::plan::Plan {
        at_ms: at.round() as u64, ms: len + 40, swap_ms: len, start_ms: start.round() as u64, rate: 1.0, bars: ROLL_BARS, semitones: 0,
        beatmatched: false, out_tempo: 1.0, ramp_ms: 0, tape_stop: false, style: Some(ROLL), tail_bars: 0, roll_speed: speed,
    })
}

/// An extra not turned down by your ratings: under 5 rated, or most (60%) of them good
pub fn not_rejected(ratings: &[(String, bool)], kind: &str) -> bool {
    let of: Vec<bool> = ratings.iter().filter(|(k, _)| k == kind).map(|(_, g)| *g).collect();
    of.len() < 5 || of.iter().filter(|g| **g).count() * 5 >= of.len() * 3
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
        Measured { from_ms: 0, to_ms: (beats.len() as f64 * period) as u64, rhythm: Rhythm { bpm, first_beat_ms: 0.0, period_ms: period, beats, downbeat: 0, tempo_confidence: conf, key: String::new(), camelot: camelot.into(), key_confidence: 0.8, downbeat_confidence: 1.0, model_bpm: 0.0, beat_db: Vec::new(), phrase_shift: 0, bar_db: bars.to_vec(), bar_beat: vec![1.0; bars.len()], bar_chroma: Vec::new(), head_downbeat: None, tail_downbeat: None }, v: omnipulse_core::rhythm::VERSION }
    }

    fn levels(intro: usize, body: usize, outro: usize) -> Vec<f32> {
        std::iter::repeat_n(-20.0, intro).chain(std::iter::repeat_n(-8.0, body)).chain(std::iter::repeat_n(-20.0, outro)).collect()
    }

    #[test]
    fn a_fade_fits_the_two_songs_structure() {
        // 120 BPM: 2 s a bar. A: 4 bars of outro (8 s) before its sound ends (96 s); B: 2 bars of intro (4 s)
        let a = song(120.0, &levels(4, 40, 4), 0.6, "8A");
        let b = song(120.0, &levels(2, 40, 2), 0.6, "8A");
        let a_side = Side { dur_ms: 100_000, audible_ms: 96_000, rhythm: Some(&a), ..Default::default() };
        // an outro and an intro: over the shorter (4 s), B's main part (at 4 s) as A's sound ends
        let f = fade(&a_side, &Side { dur_ms: 92_000, first_ms: 0, rhythm: Some(&b), ..Default::default() }, 12_000, 6_000);
        assert_eq!(f, Fade { out_from_ms: 92_000, out_to_ms: 96_000, in_from_ms: 0 });
        // a longer intro (6 bars, 12 s): its start left out, so its main part still lands at A's end
        let long_in = song(120.0, &levels(6, 40, 2), 0.6, "8A");
        let f = fade(&a_side, &Side { dur_ms: 100_000, rhythm: Some(&long_in), ..Default::default() }, 12_000, 6_000);
        assert_eq!(f, Fade { out_from_ms: 88_000, out_to_ms: 96_000, in_from_ms: 4_000 });
        // an outro into a loud start: B in where A's main part ends (88 s), over 3 s, A's outro cut short
        let loud = song(120.0, &levels(0, 44, 2), 0.6, "8A");
        let f = fade(&a_side, &Side { dur_ms: 92_000, rhythm: Some(&loud), ..Default::default() }, 12_000, 6_000);
        assert_eq!(f, Fade { out_from_ms: 88_000, out_to_ms: 91_000, in_from_ms: 0 });
        // a cold end into a loud start: a short overlap
        let (cold, hard) = (song(120.0, &levels(4, 44, 0), 0.6, "8A"), song(120.0, &levels(0, 44, 2), 0.6, "8A"));
        let f = fade(&Side { dur_ms: 96_000, rhythm: Some(&cold), ..Default::default() }, &Side { dur_ms: 92_000, rhythm: Some(&hard), ..Default::default() }, 12_000, 6_000);
        assert_eq!(f.len_ms(), 2_000);
        // a cold end into an intro: a short overlap at the end, the intro whole
        let f = fade(&Side { dur_ms: 96_000, rhythm: Some(&cold), ..Default::default() }, &Side { dur_ms: 100_000, rhythm: Some(&long_in), ..Default::default() }, 12_000, 6_000);
        assert_eq!((f.len_ms(), f.in_from_ms), (2_000, 0));
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
        // 174 into 110: counted as 87, both sped up (B in at 87, to its own; A from 87's bars on)
        let a = song(174.0, &levels(16, 96, 16), 0.6, "8A");
        let b = song(110.0, &levels(8, 48, 8), 0.6, "8A");
        // (each as long as its beats)
        let side = |m: &'static Measured| Side { dur_ms: m.to_ms, rhythm: Some(m), ..Default::default() };
        let (a, b): (&'static Measured, &'static Measured) = (Box::leak(Box::new(a)), Box::leak(Box::new(b)));
        let p = blend(&side(a), &side(b), 0).expect("a blend");
        assert!((p.rate - 87.0 / 110.0).abs() < 0.01 && p.out_tempo > 1.2, "{} {}", p.rate, p.out_tempo);
        // 174 into 120 (87 sped up 38%) and into 128 (47%): too far, never ridden that far
        for bpm in [120.0, 128.0] {
            let c: &'static Measured = Box::leak(Box::new(song(bpm, &levels(8, 48, 8), 0.6, "8A")));
            assert!(blend(&side(a), &side(c), 0).is_none(), "{bpm}");
        }
    }

    #[test]
    fn a_blend_plays_the_one_playing_out_and_keeps_its_singer() {
        // 128 BPM (1.875 s a bar): A, 8 bars in, 48 of its main part, a 16-bar outro; B, an 8-bar intro
        let a: &'static Measured = Box::leak(Box::new(song(128.0, &levels(8, 48, 16), 0.6, "8A")));
        let b: &'static Measured = Box::leak(Box::new(song(128.0, &levels(8, 48, 8), 0.6, "8A")));
        let bar = 60_000.0 / 128.0 * 4.0;
        let a_end = 72.0 * bar;
        let side = |m: &'static Measured, lines: &'static [(u64, String)]| Side { dur_ms: (m.rhythm.beats.len() as f64 * 60_000.0 / 128.0) as u64, rhythm: Some(m), lines, ..Default::default() };
        // (as late as it fits: its outro heard nearly to its end, not cut from where the outro starts)
        let p = blend(&side(a, &[]), &side(b, &[]), 0).expect("a blend");
        let ends = (p.at_ms + p.ms) as f64;
        assert!(a_end - ends < 4.0 * bar, "ends {} bars before its end", (a_end - ends) / bar);
        // (its singer at a line where that blend would fade it: a line earlier, so the line isn't cut off)
        let fade_from = p.at_ms + p.swap_ms;
        let sung: &'static [(u64, String)] = Box::leak(vec![(fade_from + 500, "a line".to_string())].into_boxed_slice());
        let q = blend(&side(a, sung), &side(b, &[]), 0).expect("a blend");
        assert!(q.at_ms < p.at_ms, "{} {}", q.at_ms, p.at_ms);
        assert!(!sings(sung, q.at_ms + q.swap_ms, q.at_ms + q.ms));
        // (where nothing fits without cutting it, still a blend, as late as it fits)
        let all: &'static [(u64, String)] = Box::leak((0..72).map(|k| ((k as f64 * bar) as u64, "la".to_string())).collect::<Vec<_>>().into_boxed_slice());
        assert_eq!(blend(&side(a, all), &side(b, &[]), 0).map(|x| x.at_ms), Some(p.at_ms));
    }

    #[test]
    fn a_blend_is_as_long_as_the_songs_make_it() {
        let bar128 = 60_000.0 / 128.0;
        // a long intro under a long outro: the intro, in phrases (16 bars at most)
        assert_eq!(blend_lengths(12, 16, 0.0, false, bar128), (12, 4));
        // a long intro, but only a short outro to carry it: the outro and a phrase of its main part
        assert_eq!(blend_lengths(16, 2, 0.0, false, bar128), (8, 2));
        // Us into Spotless (a 1-bar intro, a 3-bar outro, 16% apart): 4 bars ridden, 3 of tail, not 8 and 4
        assert_eq!(blend_lengths(0, 3, 0.16, false, 60_000.0 / 110.0), (4, 3));
        // a big ride needs room: 36% at 5% a bar, 8 bars
        assert_eq!(blend_lengths(4, 8, 0.36, false, bar128).0, 8);
        // a slow song: at most 24 s together (16 bars at 87 BPM is 44 s: 8 bars)
        assert_eq!(blend_lengths(16, 16, 0.0, false, 60_000.0 / 87.0).0, 8);
        // a cold end: a bar of tail; keys that clash: 4 bars together
        assert_eq!(blend_lengths(8, 0, 0.0, false, bar128).1, 1);
        assert_eq!(blend_lengths(16, 16, 0.0, true, bar128).0, 4);
    }

    #[test]
    fn a_small_slowdown_stays_a_slowdown() {
        // Shooting Stars' 128 into Rocket Girl's 105: slowed 18%, not counted as 64 and sped up 64%
        let a: &'static Measured = Box::leak(Box::new(song(128.0, &levels(16, 96, 16), 0.6, "8A")));
        let b: &'static Measured = Box::leak(Box::new(song(105.0, &levels(8, 48, 8), 0.6, "8A")));
        let side = |m: &'static Measured| Side { dur_ms: m.to_ms, rhythm: Some(m), ..Default::default() };
        let p = blend(&side(a), &side(b), 0).expect("a blend");
        assert!(p.out_tempo < 1.0 && (p.out_tempo - 105.0 / 128.0).abs() < 0.01, "{p:?}");
    }

    #[test]
    fn keys_that_clash_are_ridden_into_one() {
        // A minor (8A) into F minor (4A): the next one in a semitone down (E minor, 9A, beside 8A)
        let mut a = song(124.0, &levels(16, 96, 16), 0.6, "8A");
        a.rhythm.key = "A minor".into();
        let mut b = song(124.0, &levels(8, 48, 8), 0.6, "4A");
        b.rhythm.key = "F minor".into();
        assert_eq!(key_ride(&a, &b), -1);
        let side = |m| Side { dur_ms: 240_000, rhythm: Some(m), key_known: true, ..Default::default() };
        let p = blend(&side(&a), &side(&b), 0).expect("a blend");
        assert_eq!(p.semitones, -1);
        // (by our own readings only: never shifted)
        let ours = |m| Side { dur_ms: 240_000, rhythm: Some(m), ..Default::default() };
        assert_eq!(blend(&ours(&a), &ours(&b), 0).expect("a blend").semitones, 0);
        // (keys that go together: none)
        let mut c = b.clone();
        c.rhythm.camelot = "9A".into();
        c.rhythm.key = "E minor".into();
        assert_eq!(key_ride(&a, &c), 0);
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
        // (its end: bar 48, its outro played (12 dB under its main part: something in it, rolled), on a phrase line from bar 4; 4 bars before it)
        assert_eq!((p.at_ms, p.swap_ms, p.style), (88_000, 8_000, Some(ROLL)));
        assert_eq!(p.start_ms, (2.0 * 4.0 * 60_000.0 / 140.0_f64).round() as u64);
        let h = handoff(&p, true);
        assert_eq!(h.roll, Some(omnipulse_audio::Roll { bar: 2_000, bars: 4, to_speed: (140.0_f64 / 120.0) as f32 }));
        assert!(not_rejected(&[], ROLL));
    }

    #[test]
    fn an_unsure_song_is_mixed_where_its_part_is_sure() {
        let mut m = song(128.0, &[-10.0; 40], 0.8, "8A");
        m.rhythm.downbeat_confidence = 0.45;
        m.rhythm.tail_downbeat = Some((1, 0.9));
        m.rhythm.head_downbeat = Some((0, 0.5));
        let end = local_downbeat(&m, true);
        assert_eq!((end.rhythm.downbeat, end.rhythm.downbeat_confidence), (1, 0.9));
        // (its start not sure either: as it was)
        let start = local_downbeat(&m, false);
        assert_eq!((start.rhythm.downbeat, start.rhythm.downbeat_confidence), (0, 0.45));
        // (a song sure as a whole: never moved)
        m.rhythm.downbeat_confidence = 0.9;
        assert_eq!(local_downbeat(&m, true).rhythm.downbeat, 0);
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

    /// Why pairs of the songs measured so far would blend or not (state/rhythm/): each measured song
    /// into the next one measured, counted by reason
    #[test]
    #[ignore]
    fn blend_census() {
        let mut songs: Vec<(String, Measured)> = omnipulse_core::rhythm::all().into_iter().filter(|(k, m)| k.starts_with("full:") && m.v == omnipulse_core::rhythm::VERSION).collect();
        songs.sort_by(|a, b| a.0.cmp(&b.0));
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

    /// The songs' measurements as kept now (compact, each in its own file) plan every blend as the old store's
    /// did: OMNIPULSE_RHYTHM_OLD=<a copy of the old state/rhythm.json>, each measured song into the next ten
    #[test]
    #[ignore]
    fn the_compact_store_plans_the_same() {
        let path = std::env::var("OMNIPULSE_RHYTHM_OLD").expect("OMNIPULSE_RHYTHM_OLD");
        let all: std::collections::HashMap<String, Measured> = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let mut songs: Vec<(String, Measured)> = all.into_iter().filter(|(k, m)| k.starts_with("full:") && m.v == omnipulse_core::rhythm::VERSION).collect();
        songs.sort_by(|a, b| a.0.cmp(&b.0));
        let compact: Vec<Measured> = songs.iter().map(|(k, m)| omnipulse_core::rhythm::compact(k, m).expect("reads back")).collect();
        let (mut pairs, mut same, mut close) = (0, 0, 0);
        let mut worst = 0.0f64;
        for i in 0..songs.len() {
            for j in (i + 1)..(i + 11).min(songs.len()) {
                fn side(m: &Measured) -> Side<'_> { Side { dur_ms: m.to_ms, rhythm: Some(m), ..Default::default() } }
                let (a0, b0, a1, b1) = (side(&songs[i].1), side(&songs[j].1), side(&compact[i]), side(&compact[j]));
                pairs += 1;
                let old = (blend_blocked(&a0, &b0), blend(&a0, &b0, 0));
                let new = (blend_blocked(&a1, &b1), blend(&a1, &b1, 0));
                if format!("{old:?}") == format!("{new:?}") { same += 1; continue; }
                // (the same plan but for numbers a hair apart: the largest difference, in ms or units)
                let nums = |s: String| s.split(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-')).filter_map(|x| x.parse::<f64>().ok()).collect::<Vec<_>>();
                let (x, y) = (nums(format!("{old:?}")), nums(format!("{new:?}")));
                let shape = |s: String| s.chars().filter(|c| !(c.is_ascii_digit() || *c == '.' || *c == '-')).collect::<String>();
                if x.len() == y.len() && shape(format!("{old:?}")) == shape(format!("{new:?}")) {
                    close += 1;
                    let here = x.iter().zip(&y).map(|(p, q)| (p - q).abs()).fold(0.0, f64::max);
                    if here > 50.0 && std::env::var_os("OMNIPULSE_SHOW").is_some() { println!("far: {} into {} ({here})
  old {old:?}
  new {new:?}", songs[i].0, songs[j].0); }
                    worst = worst.max(here);
                } else {
                    println!("differs: {} into {}\n  old {:?}\n  new {:?}", songs[i].0, songs[j].0, old.0, new.0);
                }
            }
        }
        let bytes: usize = songs.iter().map(|(k, m)| omnipulse_core::rhythm::compact_len(k, m)).sum();
        println!("{} songs, {:.1} MB, {:.1} KB each", songs.len(), bytes as f64 / 1e6, bytes as f64 / 1024.0 / songs.len() as f64);
        println!("{pairs} pairs: {same} the same, {close} the same plan with numbers at most {worst:.3} apart, {} different", pairs - same - close);
        assert_eq!(same + close, pairs);
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
