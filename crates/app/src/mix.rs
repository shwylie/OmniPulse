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
/// Tape stops allowed (Settings > Sound > Tape Stops): off, two songs too far apart to blend aren't
/// tape-stopped into each other (they change over the usual way)
pub static TAPE_OK: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// A beat that's not clear enough isn't mixed on
const MIN_CONFIDENCE: f64 = 0.15;

#[derive(Clone, Debug, PartialEq, serde::Serialize)]
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
    /// the one playing's tempo by the time the next one's beat comes in (1.0: as it is), eased to over
    /// its beatless outro (out_ramp): a tempo too far from the next one's to match on the beat, slowed
    /// (or sped) into it. The next one goes with it: from the one playing's tempo (`rate`: 1/out_tempo)
    /// to its own by then (in_ramp), the two at one tempo all through.
    pub out_tempo: f64,
    /// how long that takes (ms of the mix: to the switch-over), 0 with no ramp
    pub ramp_ms: u64,
    /// a tape stop, then a tape start: the one playing slows to a halt on its last beat, and as it stops
    /// the next one spins up to its speed (songs too far apart, in tempo or key, to blend: never over
    /// each other). `swap_ms`: the stop's length; the spin-up's, what's left of `ms` but its last TAPE_TAIL_MS
    pub tape_stop: bool,
    /// a loop of the one playing, or of the next one: (from, length: ms from the mix's start; played that
    /// many more times), so neither has to be cut short to fit (its singing never under the mix)
    pub out_loop: Option<(u64, u64, u32)>,
    pub in_loop: Option<(u64, u64, u32)>,
    /// Wordplay: the word (or words) the one playing's loop sings and the next one comes in on
    pub word: Option<String>,
    /// a transition of its own, if it is one: "wordplay", "drop swap", "melody play"
    pub style: Option<&'static str>,
}

/// A tempo this near the next one's isn't eased into (it's played at the one playing's, then its own)
const RAMP_MIN: f64 = 0.002;

/// How far the one playing is slowed (or sped) into the next one's tempo over its outro, at most
// (as far as two songs' beats are matched: further apart, easing one onto the other is heard as the
// song going wrong, and their beats still don't meet, so it's a transition made for that instead)
const MAX_RAMP: f64 = MAX_STRETCH;

/// A bar with a beat as loud as this, against the song's loud ones (their 80th percentile), is the song
/// (a verse, its singing), not an intro to it: the next one's start is never left out past its first two of them (an
/// intro, or a build to the drop, is left out only while it's quieter)
const CONTENT_BELOW_DB: f32 = 9.0;

/// Where the next one's content starts (its first two bars with a beat near its loud ones): its end if
/// it has none
fn content_start(m: &Measured, n: usize) -> usize {
    let (bars, beat) = (&m.rhythm.bar_db, &m.rhythm.bar_beat);
    let Some(line) = loud_line(bars).map(|l| l + LOUD_WITHIN_DB - CONTENT_BELOW_DB) else { return n.saturating_sub(1) };
    // (a bar of it: a beat, and near its loud ones; a beatless intro, however loud, is still an intro)
    let content = |i: usize| bars[i] > line && beat.get(i).is_some_and(|b| *b >= BEAT_LINE);
    (0..bars.len().saturating_sub(1)).find(|&i| content(i) && content(i + 1)).unwrap_or(n.saturating_sub(1)).min(n.saturating_sub(1))
}

/// A stripped-back outro (a DJ's outro): its last bars, at least this much quieter than its loud part
const OUTRO_BELOW_DB: f32 = 3.0;

/// Where a song's main part ends, its outro after it: the last of the last two bars in a row with a beat
/// and as loud as its loud part (within OUTRO_BELOW_DB); the last bar if there's none
fn content_end(m: &Measured, n: usize) -> usize {
    let (bars, beat) = (&m.rhythm.bar_db, &m.rhythm.bar_beat);
    let Some(line) = loud_line(bars).map(|l| l + LOUD_WITHIN_DB - OUTRO_BELOW_DB) else { return n.saturating_sub(1) };
    let content = |i: usize| bars[i] > line && beat.get(i).is_some_and(|b| *b >= BEAT_LINE);
    (1..bars.len()).rev().find(|&i| content(i) && content(i - 1)).unwrap_or(n.saturating_sub(1)).min(n.saturating_sub(1))
}

/// A loop that fills `want` bars from `have` (have < want): as long as can be (4 bars, 2, or 1) and
/// fitting exactly, and how many more times it's played
fn loop_fill(have: usize, want: usize) -> Option<(usize, u32)> {
    if have == 0 || have >= want { return None; }
    [4usize, 2, 1].into_iter().find(|&l| l <= have && (want - have) % l == 0).map(|l| (l, ((want - have) / l) as u32))
}

/// The tempo the one playing goes to over its outro to arrive at the next one's (the same tempo, or
/// half or double time: the nearest), if it's not too far
fn ramp_to(a_bpm: f64, b_bpm: f64) -> Option<f64> {
    if a_bpm <= 0.0 || b_bpm <= 0.0 { return None; }
    [1.0, 2.0, 0.5].iter().map(|m| b_bpm * m / a_bpm).filter(|r| (r - 1.0).abs() <= MAX_RAMP).min_by(|x, y| (x - 1.0).abs().total_cmp(&(y - 1.0).abs()))
}

/// The one playing's tempo `p` (0..1) of the way through a mix that slows it to `target`: eased there
/// (slowly at first, slowly at the last), arriving as the mix ends (the switch-over: the next one's beat)
pub fn out_ramp(target: f64, p: f64) -> f64 {
    let q = p.clamp(0.0, 1.0);
    1.0 + (target - 1.0) * q * q * (3.0 - 2.0 * q)
}

/// The next one's tempo then: the one playing's, from its own start (1/target of its own) to its own
/// by the end, the two together all through
pub fn in_ramp(target: f64, p: f64) -> f64 {
    out_ramp(target, p) / target
}

/// A key not clear enough isn't shifted for
// (a quarter of songs measure below this: a guess, not a key; neither a clash nor a shift is decided on it)
const MIN_KEY_CONFIDENCE: f64 = 0.25;

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

/// How much louder a drop is than the bars before it, and how near the song's loudest it gets (dB, 8 bars each)
const DROP_RISE_DB: f64 = 3.0;
const DROP_NEAR_DB: f64 = 3.0;

/// A song's drop, if its intro builds to one: of its phrases' starts (every 4 bars) in its first 40%
/// where its next 8 bars are 3 dB louder than its last 8 and within 3 dB of its loudest 8, the one that
/// rises most and comes nearest its loudest (a build has smaller steps up before it). None for one
/// that's into it from the start (no such rise).
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

/// Whether bar k starts a phrase (or every `every` bars of one) of a song whose phrases start at `off`
fn on_phrase(k: usize, off: usize, every: usize) -> bool { (k + PHRASE - off % PHRASE) % every == 0 }

/// Each bar's downbeat in a measured track (ms from its start), from its tracked beats
fn downbeats(m: &Measured) -> Vec<f64> {
    m.rhythm.beats.iter().skip(m.rhythm.downbeat).step_by(4).map(|b| m.from_ms as f64 + b).collect()
}

/// How a plan hands over: on the beat, a wipe (one split sweeping down through the sound, the one going
/// out below it, the one coming in above it, never both in one band; down to the bass by the swap,
/// where the next one's main part starts). `bands` (Settings' EQ Band Handoff): a band at a time
/// instead, as a DJ's EQ (quicker). Over a soft part: a crossfade with the bass swapped halfway.
/// (after a tape start: the next one alone a moment, at its speed, before the mix is over)
const TAPE_TAIL_MS: u64 = 300;

pub fn handoff(p: &Plan, bands: bool) -> omnipulse_audio::Handoff {
    if p.tape_stop { return omnipulse_audio::Handoff::tape(p.swap_ms, p.ramp_ms); }
    if let (Some(_), Some((_, beat, reps))) = (&p.word, p.out_loop) { return omnipulse_audio::Handoff::word_cut(p.swap_ms, beat, reps); }
    if p.style == Some("drop swap") { return omnipulse_audio::Handoff::cut(p.swap_ms); }
    let mut h = if !p.beatmatched {
        omnipulse_audio::Handoff::bass_swap(p.ms, p.ms / 2)
    } else {
        let beat = p.ms.saturating_sub(p.swap_ms) as f64 / 4.0;
        if bands { omnipulse_audio::Handoff::staged(p.ms, beat, p.swap_ms) } else { omnipulse_audio::Handoff::wipe(beat, p.swap_ms) }
    };
    let as_loop = |(from, len, reps): (u64, u64, u32)| omnipulse_audio::Loop { from, len, reps };
    h.out_loop = p.out_loop.map(as_loop);
    h.in_loop = p.in_loop.map(as_loop);
    h
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
    // (the next one's start left out: its quiet intro, never its content: from its phrase at the latest)
    // (its content never part of the mix: on a blend, its run-up ends where its content starts at the
    // latest; on a tape start, it spins up in its intro, a phrase before its content)
    let first = content_start(b, bd.len());
    let cap = (0..first).rev().find(|&c| on_phrase(c, b_off, PHRASE)).unwrap_or(0);
    let drop_at = drop_bar(b, b_off).map_or(b_main, |d| d.max(b_main));
    // (where the next one starts from: no later than that)
    let b_at = |k: usize| bd[k.min(cap).min(bd.len() - 1)];
    let (soonest, audible) = (now_ms as f64 + 4000.0, a_end_ms as f64 - 300.0);
    let a_bar = 4.0 * a.rhythm.period_ms;
    // (bar k's downbeat; past the last one tracked, a bar on from it for each)
    let at = |k: usize| ad.get(k).copied().unwrap_or_else(|| ad[ad.len() - 1] + (k + 1 - ad.len()) as f64 * a_bar);
    // (one tempo, keys that clash: still on the beat, but a short blend: half a phrase, the bass swapped
    // on its downbeat, so the two keys are over each other only briefly, not a tape stop)
    let clash = !keys_fit(a, b);
    let matched = match_rate(a.rhythm.bpm, b.rhythm.bpm).map(|(r, _)| r);
    if let Some(r) = matched {
        let runs: &[usize] = if clash { &[4] } else { &[PHRASE, 4] };
        let fits = |k: usize, n: u32| at(k) >= soonest && k + n as usize <= a_end && at(k) + a_bar * n as f64 <= audible;
        // the run-up to the swap: a phrase (8 bars), else half of one; the swap on a phrase's start of
        // both. The one playing: the latest of its own phrases' starts that has its beat to the swap
        // (giving up no more than a phrase of its bars with one, else any); the next one: from its own
        // phrase's start, the soonest that has its main part (its beat) in by the swap, its intro before
        // that cut. After the swap, a bar of the next one alone.
        let place = |n: usize| {
            let ideal = a_end.checked_sub(n)?;
            let latest = |every: usize| (0..=ideal).rev().find(|&k| on_phrase(k, a_off, every) && fits(k, n as u32));
            [PHRASE, 4].iter().filter_map(|&e| latest(e)).find(|&k| k + PHRASE >= ideal).or_else(|| latest(1))
        };
        let (run, k) = runs.iter().find_map(|&n| place(n).map(|k| (n, k)))?;
        // (the next one's bars in the run-up: as many as the one playing's, or half or twice as many in
        // half or double time: one of its bars as long as two of the other's, or half of one)
        let a_per_b = b.rhythm.period_ms / (r * a.rhythm.period_ms);
        let run_b = ((run as f64 / a_per_b).round() as usize).max(1);
        let every = if run_b >= PHRASE { PHRASE } else { run_b.clamp(1, 4) };
        // (an intro that builds to a drop: the switch-over on the drop, the build before the run-up cut)
        let main = drop_at;
        let from_bar = (0..bd.len()).find(|&c| on_phrase(c, b_off, every) && c + run_b >= main).unwrap_or(0).min(bd.len() - 1).min(first.saturating_sub(run_b));
        let (bars, swap_bar) = (run as u32 + 1, run);
        // stretched to each one's tempo over the bars mixed, not its average (within a percent of it)
        let n = bars as usize * 4;
        let rate = match (period_at(a, at(k), n), period_at(b, bd[from_bar], (n as f64 * r).round() as usize)) {
            (Some(pa), Some(pb)) => {
                let local = r * (pb / b.rhythm.period_ms) / (pa / a.rhythm.period_ms);
                if (local / r - 1.0).abs() <= 0.01 { local } else { r }
            }
            _ => r,
        };
        // the two at one tempo all through: from the one playing's (the next one stretched to it) to the
        // next one's own (the one playing slowed or sped to it) by the switch-over, eased; then a bar
        // of the next one alone, at its own. (Its run-up takes as long as its bars do at the eased tempo.)
        let target = 1.0 / rate;
        let (swap_ms, len, out_tempo) = if (target - 1.0).abs() < RAMP_MIN {
            (a_bar * swap_bar as f64, a_bar * bars as f64, 1.0)
        } else {
            let swap = a_bar * swap_bar as f64 * 2.0 / (1.0 + target);
            (swap, swap + a_bar / target, target)
        };
        // loops, so nothing's cut short to fit (with the two at one tempo all through: no tempo eased)
        let (mut at_bar, mut out_loop, mut in_loop) = (k, None, None);
        if out_tempo == 1.0 {
            // the next one's intro shorter than the run-up: its last bars before its content looped, its
            // content coming in on the swap (never under the run-up)
            if from_bar == 0 && first < run_b {
                if let Some((l, reps)) = loop_fill(first, run_b) {
                    let (s0, s1) = (bd[first - l], bd[first.min(bd.len() - 1)]);
                    if s1 > s0 { in_loop = Some((((s0 - bd[0]) / rate).round() as u64, ((s1 - s0) / rate).round() as u64, reps)); }
                }
            }
            // the one playing's outro shorter than the run-up: the mix from where its outro starts, its last
            // bars looped (its loud part, its singing, never under the mix)
            let a_last = content_end(a, ad.len());
            let outro = a_end.saturating_sub(a_last + 1);
            if k <= a_last {
                if let Some((l, reps)) = loop_fill(outro, run) {
                    let k2 = a_end - outro;
                    if at(k2) >= soonest {
                        at_bar = k2;
                        out_loop = Some(((at(a_end - l) - at(k2)).round() as u64, (at(a_end) - at(a_end - l)).round() as u64, reps));
                    }
                }
            }
        }
        return Some(Plan {
            at_ms: at(at_bar).round() as u64,
            ms: len.round() as u64,
            swap_ms: swap_ms.round() as u64,
            start_ms: bd[from_bar].round() as u64,
            rate,
            bars,
            semitones: key_shift(a, b),
            beatmatched: true,
            out_tempo,
            ramp_ms: if out_tempo == 1.0 { 0 } else { swap_ms.round() as u64 },
            tape_stop: false,
            out_loop,
            in_loop,
            word: None,
        style: None,
        });
    }
    // (eased onto one tempo, `target` not 1.0: the next one from the one playing's tempo to its own over all of it)
    let soft = |at_ms: f64, ms: f64, start_ms: f64, bars: u32, target: f64| Some(Plan {
        at_ms: at_ms.round() as u64,
        ms: ms.round() as u64,
        swap_ms: (ms / 2.0).round() as u64,
        start_ms: start_ms.round() as u64,
        rate: 1.0 / target,
        bars,
        semitones: 0,
        beatmatched: false,
        out_tempo: target,
        ramp_ms: if target == 1.0 { 0 } else { ms.round() as u64 },
        tape_stop: false,
        out_loop: None,
        in_loop: None,
        word: None,
        style: None,
    });
    // (how far the one playing is eased, if it can be; and how many of its bars go to one of the next one's:
    // half or double time, 2 or 1/2)
    let ramp = ramp_to(a.rhythm.bpm, b.rhythm.bpm);
    // not blended on the beat (tempos too far apart, or keys that clash): never faded over each other. A
    // tape stop, then a tape start: the one playing brakes over its last beat with one (two beats long:
    // slowing evenly, it goes one), coming to rest on the downbeat its beat ends at (its outro cut); as it
    // stops, the next one spins up from its start (its intro's quiet part cut, never its content), eased
    // in rather than dropped in
    let _ = (ramp, soft);
    if !TAPE_OK.load(std::sync::atomic::Ordering::Relaxed) { return None; }
    let beat = a.rhythm.period_ms;
    let stop = (2.0 * beat).clamp(600.0, 1600.0);
    // (its spin-up under the stop's second half: at its speed as the other halts)
    let spin = stop / 2.0;
    let from = at(a_end) - beat;
    if from < soonest || at(a_end) > audible + 300.0 { return None; }
    Some(Plan {
        at_ms: from.round() as u64,
        ms: stop.round() as u64 + TAPE_TAIL_MS,
        swap_ms: stop.round() as u64,
        // (from half its spin-up before its downbeat: spinning up reads half as much as it plays, so the
        // downbeat comes as it's at its speed)
        start_ms: (b_at(drop_at) - spin / 2.0).max(0.0).round() as u64,
        rate: 1.0,
        bars: 1,
        semitones: 0,
        beatmatched: false,
        out_tempo: 1.0,
        ramp_ms: (spin.round() as u64),
        tape_stop: true,
        out_loop: None,
        in_loop: None,
        word: None,
        style: None,
    })
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
        out_tempo: 1.0,
        ramp_ms: 0,
        tape_stop: false,
        out_loop: None,
        in_loop: None,
        word: None,
        style: None,
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

// ---------- Wordplay: the one playing's word looped on the beat, the next one coming in singing it ----------
/// Words too common to make wordplay of (a match on one of them alone is no match)
const PLAIN_WORDS: [&str; 40] = [
    "the", "and", "but", "you", "your", "i'm", "im", "for", "with", "this", "that", "are", "was", "she", "he's", "his", "her", "our", "they", "them", "its", "it's", "all", "not", "can", "now", "got", "get", "just", "like", "yeah", "ooh", "ohh", "hey", "nah", "woah", "whoa", "uh", "ah", "oh",
];

/// A line's words, plain (lower case; its letters, digits and apostrophes)
fn words_of(line: &str) -> Vec<String> {
    line.split_whitespace()
        .map(|w| w.chars().filter(|c| c.is_alphanumeric() || *c == '\'').collect::<String>().to_lowercase().replace('’', "'"))
        .filter(|w| !w.is_empty())
        .collect()
}

/// How many of two lines' first words are the same (up to three); a single one counts only if it's a
/// word of its own (not "the", "oh"), four letters or more
fn opening_match(a: &[String], b: &[String]) -> usize {
    let n = a.iter().zip(b).take(3).take_while(|(x, y)| x == y).count();
    match n {
        0 => 0,
        1 if a[0].chars().count() < 4 || PLAIN_WORDS.contains(&a[0].as_str()) => 0,
        n => n,
    }
}

/// The nearest of a song's beats to `t` (ms from its start), if it's within a third of a beat: (its
/// time, its place in the bar: 0 a downbeat)
fn beat_near(m: &Measured, t: f64) -> Option<(f64, usize)> {
    let (i, at) = m.rhythm.beats.iter().enumerate().map(|(i, b)| (i, m.from_ms as f64 + b)).min_by(|x, y| (x.1 - t).abs().total_cmp(&(y.1 - t).abs()))?;
    ((at - t).abs() <= m.rhythm.period_ms / 3.0).then_some((at, (i + 4 - m.rhythm.downbeat % 4) % 4))
}

/// Wordplay, from both songs' timed lyrics (each line: ms from the song's start, its words): a line
/// late in the one playing and one early in the next that start with the same word (or words). The
/// one playing's word (its beat) is looped to the next bar's downbeat (four to seven times in all), and
/// cut there as the next one comes in on the same word, at the one playing's tempo if they're near
/// enough (then eased back to its own), else at its own. None without such a pair.
pub fn plan_wordplay(a: &Measured, a_lines: &[(u64, String)], a_end_ms: u64, now_ms: u64, b: &Measured, b_lines: &[(u64, String)]) -> Option<Plan> {
    if a.rhythm.tempo_confidence < MIN_CONFIDENCE || b.rhythm.tempo_confidence < MIN_CONFIDENCE { return None; }
    let b_len = (b.to_ms.max(b.from_ms)) as f64;
    let (soonest, a_end) = ((now_ms + 8000) as f64, a_end_ms as f64);
    let rate = match_rate(a.rhythm.bpm, b.rhythm.bpm).map_or(1.0, |(r, _)| r);
    let beat = a.rhythm.period_ms;
    // (the best: the most words the same, then the latest in the one playing, then the soonest in the next)
    let mut best: Option<(usize, f64, f64, Plan)> = None;
    for (ta, la) in a_lines {
        let ta = *ta as f64;
        // late in it (past halfway), with time for its loop before its sound ends
        if ta < soonest.max(a_end * 0.5) || ta + 8.0 * beat > a_end { continue; }
        let wa = words_of(la);
        let Some((a_at, a_place)) = beat_near(a, ta) else { continue };
        // (to the downbeat after: four beats at least)
        let n = (4..8).find(|n| (a_place + n) % 4 == 0).unwrap_or(4);
        let swap = n as f64 * beat;
        for (tb, lb) in b_lines {
            let tb = *tb as f64;
            if tb < 2000.0 || tb > b_len * 0.6 { continue; }
            let same = opening_match(&wa, &words_of(lb));
            if same == 0 { continue; }
            let Some((b_at, _)) = beat_near(b, tb) else { continue };
            // (the next one read from the mix's start: as far before its word as the loop is long)
            let start = b_at - swap * rate;
            if start < 0.0 { continue; }
            let better = best.as_ref().is_none_or(|(s, x, y, _)| same > *s || (same == *s && (a_at > *x + 1.0 || ((a_at - *x).abs() <= 1.0 && b_at < *y))));
            if !better { continue; }
            let word = wa[..same].join(" ");
            best = Some((same, a_at, b_at, Plan {
                at_ms: a_at.round() as u64,
                ms: (swap + 400.0).round() as u64,
                swap_ms: swap.round() as u64,
                start_ms: start.round() as u64,
                rate,
                bars: 1,
                semitones: 0,
                beatmatched: false,
                out_tempo: 1.0,
                ramp_ms: 0,
                tape_stop: false,
                out_loop: Some((0, beat.round() as u64, n as u32 - 1)),
                in_loop: None,
                word: Some(word),
                style: Some("wordplay"),
            }));
        }
    }
    best.map(|b| b.3)
}

// ---------- Drop Swap: the next one's drop where the one playing's final drop would have been ----------
/// A song's final drop: of its phrases' starts (every 4 bars) past its middle where its next 8 bars are
/// 3 dB louder than its last 8 and within 3 dB of its loudest 8, the last run of them; of that run, the
/// one rising most (the drop itself, not a bar of the build before it)
fn last_drop(m: &Measured, off: usize) -> Option<usize> {
    let d = &m.rhythm.bar_db;
    let n = d.len();
    if n < 24 { return None; }
    let mean = |a: usize, b: usize| (a..b).map(|i| d[i].max(-40.0) as f64).sum::<f64>() / (b - a) as f64;
    let loudest = (0..=n - 8).map(|i| mean(i, i + 8)).fold(f64::MIN, f64::max);
    let rise = |i: usize| mean(i, i + 8) - mean(i - 8, i);
    let ok: Vec<usize> = (n / 2..=n - 8).filter(|&i| on_phrase(i, off, 4) && rise(i) >= DROP_RISE_DB && loudest - mean(i, i + 8) <= DROP_NEAR_DB).collect();
    // (the last rise, as a run of phrase starts that each count: the one of them rising most)
    ok.last()?;
    let mut from = ok.len() - 1;
    while from > 0 && ok[from - 1] + 4 >= ok[from] { from -= 1; }
    ok[from..].iter().copied().max_by(|&x, &y| rise(x).total_cmp(&rise(y)))
}

/// Drop Swap: the one playing's build to its final drop plays out, and on the drop's downbeat the next
/// one's drop comes in instead (cut on the beat, at the one playing's tempo, then eased back to its own).
/// Tempos that go together, a final drop late in the one playing (its last 45%), a drop the next one
/// builds to; None without.
pub fn plan_drop_swap(a: &Measured, a_end_ms: u64, now_ms: u64, b: &Measured) -> Option<Plan> {
    if a.rhythm.tempo_confidence < MIN_CONFIDENCE || b.rhythm.tempo_confidence < MIN_CONFIDENCE { return None; }
    let r = match_rate(a.rhythm.bpm, b.rhythm.bpm).map(|(r, _)| r)?;
    let (ad, bd) = (downbeats(a), downbeats(b));
    let a_drop = last_drop(a, phrase_offset(a))?;
    let b_drop = drop_bar(b, phrase_offset(b))?;
    if a_drop * 100 < a.rhythm.bar_db.len() * 55 || a_drop < 1 { return None; }
    let (from, at) = (*ad.get(a_drop - 1)?, *ad.get(a_drop)?);
    if from < (now_ms + 8000) as f64 || at > a_end_ms as f64 { return None; }
    let bar = at - from;
    let start = bd.get(b_drop)? - bar * r;
    if start < 0.0 { return None; }
    Some(Plan {
        at_ms: from.round() as u64,
        ms: (bar + 400.0).round() as u64,
        swap_ms: bar.round() as u64,
        start_ms: start.round() as u64,
        rate: r,
        bars: 1,
        semitones: key_shift(a, b),
        beatmatched: false,
        out_tempo: 1.0,
        ramp_ms: 0,
        tape_stop: false,
        out_loop: None,
        in_loop: None,
        word: None,
        style: Some("drop swap"),
    })
}

// ---------- Melody Play: the one playing's melody looped into the same melody coming in ----------
/// A bar's notes as one profile (the bars from `k`, `n` of them), shifted up `s` semitones
fn notes_of(m: &Measured, k: usize, n: usize, s: i32) -> Option<[f32; 12]> {
    let bars = m.rhythm.bar_chroma.get(k..k + n)?;
    let mut c = [0.0f32; 12];
    for b in bars { for i in 0..12 { c[(i as i32 + s).rem_euclid(12) as usize] += b[i]; } }
    let sum: f32 = c.iter().sum();
    (sum > 0.0).then(|| c.map(|v| v / sum))
}

/// Notes that make a tune (a few pitch classes most of it), not a wash of all of them
fn tuneful(c: &[f32; 12]) -> bool {
    let mut v = *c;
    v.sort_by(|a, b| b.total_cmp(a));
    v[0] + v[1] + v[2] >= 0.5
}

fn alike(x: &[f32; 12], y: &[f32; 12]) -> f32 {
    let (d, nx, ny) = (0..12).fold((0.0, 0.0, 0.0), |(d, a, b), i| (d + x[i] * y[i], a + x[i] * x[i], b + y[i] * y[i]));
    if nx > 0.0 && ny > 0.0 { d / (nx * ny).sqrt() } else { 0.0 }
}

/// How alike two melodies' notes must be (cosine, 0..1) to play one into the other
const MELODY_ALIKE: f32 = 0.92;

/// Melody Play: a bar (or two) of the one playing's melody, late in it, looped for four bars as the next
/// one comes in under it, on the beat; the swap where the next one's own melody starts with the same
/// notes (the next one shifted in key to fit, as a mix on the beat does), so the tune carries on from one
/// song into the other. None without such a pair (or their notes measured).
pub fn plan_melody(a: &Measured, a_end_ms: u64, now_ms: u64, b: &Measured) -> Option<Plan> {
    if a.rhythm.tempo_confidence < MIN_CONFIDENCE || b.rhythm.tempo_confidence < MIN_CONFIDENCE { return None; }
    if a.rhythm.bar_chroma.is_empty() || b.rhythm.bar_chroma.is_empty() { return None; }
    let r = match_rate(a.rhythm.bpm, b.rhythm.bpm).map(|(r, _)| r).filter(|_| keys_fit(a, b))?;
    let (ad, bd) = (downbeats(a), downbeats(b));
    let (_, a_end) = beat_span(&proper(a)?)?;
    let (a_off, b_off) = (phrase_offset(a), phrase_offset(b));
    let shift = key_shift(a, b);
    let soonest = (now_ms + 8000) as f64;
    let na = a.rhythm.bar_chroma.len().min(ad.len());
    let nb = (b.rhythm.bar_chroma.len().min(bd.len())) / 2;
    let mut best: Option<(f32, Plan)> = None;
    for len in [2usize, 1] {
        let reps = (4 / len) as u32 - 1;
        for k in (na * 55 / 100..na.saturating_sub(len)).filter(|&k| on_phrase(k, a_off, 4)) {
            // (its loop, then a bar of the next one alone, all before its beat ends)
            if k + 5 > a_end || *ad.get(k)? < soonest || ad.get(k + 5).is_none_or(|t| *t > a_end_ms as f64) { continue; }
            let Some(mine) = notes_of(a, k, len, 0).filter(tuneful) else { continue };
            for j in (4..nb).filter(|&j| on_phrase(j, b_off, 4)) {
                let Some(theirs) = notes_of(b, j, len, shift).filter(tuneful) else { continue };
                let like = alike(&mine, &theirs);
                if like < MELODY_ALIKE || best.as_ref().is_some_and(|(l, _)| *l >= like) { continue; }
                let bar = ad[k + 1] - ad[k];
                best = Some((like, Plan {
                    at_ms: ad[k].round() as u64,
                    ms: (5.0 * bar).round() as u64,
                    swap_ms: (4.0 * bar).round() as u64,
                    start_ms: bd[j - 4].round() as u64,
                    rate: r,
                    bars: 5,
                    semitones: shift,
                    beatmatched: true,
                    out_tempo: 1.0,
                    ramp_ms: 0,
                    tape_stop: false,
                    out_loop: Some((0, (len as f64 * bar).round() as u64, reps)),
                    in_loop: None,
                    word: None,
                    style: Some("melody play"),
                }));
            }
        }
    }
    best.map(|b| b.1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omnipulse_core::rhythm::{Rhythm, VERSION};

    fn m(bpm: f64, first_beat: f64, downbeat: usize, conf: f64) -> Measured {
        let period = 60_000.0 / bpm;
        let beats: Vec<f64> = (0..64).map(|i| first_beat + i as f64 * period).collect();
        Measured { from_ms: 0, to_ms: 60_000, rhythm: Rhythm { bpm, first_beat_ms: first_beat, period_ms: period, beats, downbeat, tempo_confidence: conf, key: "A minor".into(), camelot: "8A".into(), key_confidence: 0.5, bar_db: Vec::new(), bar_beat: Vec::new(), bar_chroma: Vec::new() }, v: VERSION }
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
    fn wordplay_loops_the_word_to_the_downbeat_and_comes_in_on_it() {
        // 128 BPM, beats from 500 ms (a beat 468.75 ms); the one playing sings "Heat, up the night" on
        // its 3rd beat of bar 60, the next one "heat up the floor" at bar 10's downbeat
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        let b = shaped(m(126.0, 300.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        let (beat_a, beat_b): (f64, f64) = (60_000.0 / 128.0, 60_000.0 / 126.0);
        let ta = 500.0 + (60.0 * 4.0 + 2.0) * beat_a;
        let tb = 300.0 + 40.0 * beat_b;
        let end = (500.0 + 88.0 * 4.0 * beat_a) as u64;
        let a_lines = vec![(30_000, "Oh, the night".to_string()), ((ta + 40.0) as u64, "Heat, up the night".to_string())];
        let b_lines = vec![(tb as u64 - 30, "heat up the floor".to_string()), (60_000, "the night".to_string())];
        let p = plan_wordplay(&a, &a_lines, end, 10_000, &b, &b_lines).expect("wordplay");
        assert_eq!(p.word.as_deref(), Some("heat up the"));
        // from the word's beat, looped to the next downbeat: 2 beats in that bar, then 4 more (6 in all)
        assert_eq!(p.at_ms, ta.round() as u64);
        assert_eq!(p.out_loop, Some((0, beat_a.round() as u64, 5)));
        assert_eq!(p.swap_ms, (6.0 * beat_a).round() as u64);
        // the next one at the one playing's tempo, its word on the swap
        assert!((p.rate - 128.0 / 126.0).abs() < 1e-9);
        assert!((p.start_ms as f64 + p.swap_ms as f64 * p.rate - tb).abs() < 2.0, "{p:?}");
        // "oh" and "the" alone aren't wordplay; nothing early enough in the one playing either
        assert!(plan_wordplay(&a, &[(ta as u64, "Oh my".into())], end, 10_000, &b, &[(tb as u64, "oh no".into())]).is_none());
        assert!(plan_wordplay(&a, &[(20_000, "Heat".into())], end, 10_000, &b, &[(tb as u64, "Heat".into())]).is_none());
        // a handoff that loops it, then cuts
        let h = handoff(&p, false);
        assert_eq!(h.out_loop.map(|l| l.reps), Some(5));
    }

    #[test]
    fn a_drop_swap_lands_the_next_drop_on_the_final_one() {
        // the one playing: 40 loud, 16 of build, its final drop at bar 56 (24 loud), 8 of outro; the next:
        // a 16-bar build to its drop at bar 16
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(40, -9.0, true), (16, -16.0, true), (24, -9.0, true), (8, -15.0, true)]));
        let b = shaped(m(127.0, 300.0, 0, 0.5), &song(&[(16, -17.0, true), (72, -9.0, true)]));
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let end = (500.0 + 88.0 * bar) as u64;
        let p = plan_drop_swap(&a, end, 10_000, &b).expect("a drop swap");
        assert_eq!(p.style, Some("drop swap"));
        // from a bar before its final drop, the swap on that drop
        assert_eq!(p.at_ms, (500.0 + 55.0 * bar).round() as u64);
        assert_eq!(p.swap_ms, bar.round() as u64);
        // the next one's drop on the swap (at the one playing's tempo)
        let b_drop = 300.0 + 16.0 * 4.0 * 60_000.0 / 127.0;
        assert!((p.start_ms as f64 + p.swap_ms as f64 * p.rate - b_drop).abs() < 2.0, "{p:?}");
        // a cut on the beat
        let h = handoff(&p, false);
        assert_eq!((h.low.in_at, h.low.out_at + h.low.out_len), (p.swap_ms, p.swap_ms));
        // tempos too far apart: none
        assert!(plan_drop_swap(&a, end, 10_000, &shaped(m(100.0, 300.0, 0, 0.5), &song(&[(16, -17.0, true), (72, -9.0, true)]))).is_none());
    }

    #[test]
    fn melody_play_loops_a_tune_into_the_same_one() {
        // notes: a tune on A, C and E (most of the bar), and a wash (all twelve alike)
        let mut tune = [0.02f32; 12];
        (tune[9], tune[0], tune[4]) = (0.3, 0.25, 0.25);
        let wash = [1.0 / 12.0; 12];
        let mut a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        let mut b = shaped(m(128.0, 300.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        a.rhythm.bar_chroma = (0..88).map(|i| if (64..66).contains(&i) { tune } else { wash }).collect();
        b.rhythm.bar_chroma = (0..88).map(|i| if (12..14).contains(&i) { tune } else { wash }).collect();
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let end = (500.0 + 88.0 * bar) as u64;
        let p = plan_melody(&a, end, 10_000, &b).expect("melody play");
        assert_eq!(p.style, Some("melody play"));
        // its two bars of tune looped (once more: four bars), the next one's tune on the swap
        assert_eq!(p.at_ms, (500.0 + 64.0 * bar).round() as u64);
        assert_eq!(p.out_loop, Some((0, (2.0 * bar).round() as u64, 1)));
        assert_eq!(p.start_ms, (300.0 + 8.0 * bar).round() as u64);
        assert_eq!(p.swap_ms, (4.0 * bar).round() as u64);
        // nothing tuneful to match: none
        b.rhythm.bar_chroma = vec![wash; 88];
        assert!(plan_melody(&a, end, 10_000, &b).is_none());
    }

    #[test]
    fn a_short_intro_or_outro_looped_rather_than_mixed_over() {
        // the next one: 4 bars of beat intro, then its loud part (its singing): looped to fill the 8-bar
        // run-up, its content coming in on the swap
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(64, -10.0, true), (24, -18.0, true)]));
        let b = shaped(m(128.0, 300.0, 0, 0.5), &song(&[(4, -20.0, true), (84, -9.0, true)]));
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let end = (500.0 + 88.0 * bar) as u64;
        let p = plan_structure(&a, end, 10_000, &b).expect("a plan");
        assert!(p.beatmatched);
        assert_eq!(p.in_loop, Some((0, (4.0 * bar).round() as u64, 1)), "{p:?}");
        assert_eq!(p.out_loop, None, "an outro long enough: {p:?}");
        // the one playing: its loud part right to 4 bars before its end (a 4-bar outro): the mix from its
        // outro, its last 4 bars looped once more (never over its loud part)
        let a2 = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(84, -10.0, true), (4, -13.0, true)]));
        let b2 = shaped(m(128.0, 300.0, 0, 0.5), &song(&[(24, -20.0, true), (64, -9.0, true)]));
        let q = plan_structure(&a2, end, 10_000, &b2).expect("a plan");
        assert_eq!(q.at_ms, (500.0 + 84.0 * bar).round() as u64, "{q:?}");
        assert_eq!(q.out_loop, Some((0, (4.0 * bar).round() as u64, 1)), "{q:?}");
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
        // a phrase's run-up to the swap, then a bar of the next one alone
        assert_eq!((p.bars, p.swap_ms), (9, (8.0 * bar).round() as u64));
        // from bar 56 (a phrase), the swap where its beat ends (bar 64), its beatless outro cut
        assert_eq!(p.at_ms, (500.0 + 56.0 * bar).round() as u64);
        // the next one from its bar 16 (a phrase), its beat in (bar 24) on the swap, the rest of its intro cut
        assert_eq!(p.start_ms, (300.0 + 16.0 * bar).round() as u64);
        // an intro with a beat that builds (quiet, then fuller, then a long build) to its drop at bar 32: its
        // fuller part (with a beat, near its loud ones: a verse, say, sung) never part of the mix: its
        // run-up from bar 0, that part in on the swap, after the one playing has gone
        let built = shaped(m(128.0, 300.0, 0, 0.5), &song(&[(8, -16.0, true), (8, -11.0, true), (16, -12.0, true), (48, -7.0, true)]));
        assert_eq!(drop_bar(&built, 0), Some(32));
        // a smaller step up first (8 bars 3 dB up, still 3 dB under its loudest): not the drop
        let stepped = shaped(m(128.0, 300.0, 0, 0.5), &song(&[(8, -13.0, true), (8, -10.0, true), (16, -12.0, true), (48, -7.0, true)]));
        assert_eq!(drop_bar(&stepped, 0), Some(32));
        let p = plan_structure(&a, end, 10_000, &built).expect("a plan");
        assert_eq!(p.start_ms, 300);
        // into it from the start (a few quiet bars, no rise to speak of): no drop, its beat as before
        assert_eq!(drop_bar(&shaped(m(128.0, 300.0, 0, 0.5), &song(&[(4, -12.0, false), (80, -10.0, true)])), 0), None);
        // a beat all through, end to end: the latest phrase that fits before the sound ends
        let flat = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        let p = plan_structure(&flat, end, 10_000, &shaped(m(128.0, 300.0, 0, 0.5), &song(&[(88, -10.0, true)]))).expect("a plan");
        assert_eq!((p.bars, p.start_ms), (9, 300));
        assert_eq!(p.at_ms, (500.0 + 72.0 * bar).round() as u64);
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
        // out on its phrase (bar 57: the swap on bar 65, where its beat ends), not a bar early; the next
        // one from its bar 8 (on its phrase), its beat in on the swap
        assert_eq!(p.at_ms, (500.0 + 57.0 * bar).round() as u64);
        assert_eq!(p.start_ms, (300.0 + 8.0 * bar).round() as u64);
        assert_eq!(p.swap_ms, (8.0 * bar).round() as u64);
    }

    #[test]
    fn matched_eased_onto_one_tempo_by_the_switch_over() {
        // 128 into 122: on the beat, the two at one tempo from 128 to 122 by the switch-over
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(64, -10.0, true), (24, -11.0, false)]));
        let b = shaped(m(122.0, 300.0, 0, 0.5), &song(&[(24, -12.0, false), (64, -9.0, true)]));
        // (half time: 128 into 64, its intro 8 of its own bars, its beat on the switch-over: from its bar 4,
        // as its 4 bars are the other's 8)
        let half = shaped(m(64.0, 300.0, 0, 0.5), &song(&[(8, -12.0, false), (40, -9.0, true)]));
        let h = plan_structure(&a, (500.0 + 88.0 * 4.0 * 60_000.0 / 128.0) as u64, 10_000, &half).expect("a half-time plan");
        assert!(h.beatmatched && (h.rate - 1.0).abs() < 1e-6, "{h:?}");
        assert_eq!(h.start_ms, (300.0 + 4.0 * 4.0 * 60_000.0 / 64.0f64).round() as u64);
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let p = plan_structure(&a, (500.0 + 88.0 * bar) as u64, 10_000, &b).expect("a plan");
        let target = 122.0 / 128.0;
        assert!(p.beatmatched && (p.out_tempo - target).abs() < 1e-6 && (p.rate - 1.0 / target).abs() < 1e-6, "{p:?}");
        for x in [0.0, 0.3, 0.6, 1.0] { assert!((out_ramp(p.out_tempo, x) * 128.0 - in_ramp(p.out_tempo, x) * 122.0).abs() < 1e-6, "one tempo at {x}"); }
        assert!((in_ramp(p.out_tempo, 1.0) - 1.0).abs() < 1e-9);
        // its run-up (8 bars) as long as they take eased; the ramp ends on the switch-over; a bar after at 122
        let swap = bar * 8.0 * 2.0 / (1.0 + target);
        assert_eq!((p.swap_ms, p.ramp_ms), (swap.round() as u64, swap.round() as u64));
        assert_eq!(p.ms, (swap + bar / target).round() as u64);
    }

    #[test]
    fn tempos_apart_over_the_soft_part() {
        let bar: f64 = 4.0 * 60_000.0 / 128.0;
        let beat = bar / 4.0;
        let fast_bar: f64 = 4.0 * 60_000.0 / 174.0;
        let a = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(64, -10.0, true), (24, -11.0, false)]));
        let end = (500.0 + 88.0 * bar) as u64;
        // 128 into 110 (14% apart): past what's eased (as far as beats are matched, no further): a tape stop,
        // not the two stretched to meet
        let slower = shaped(m(110.0, 300.0, 0, 0.5), &song(&[(24, -12.0, false), (64, -9.0, true)]));
        let q = plan_structure(&a, end, 10_000, &slower).expect("a plan");
        assert!(q.tape_stop && !q.beatmatched && q.out_tempo == 1.0, "{q:?}");
        // 128 into 174 (32% from it, and from its half time): too far: a tape stop. It brakes over its
        // last beat with one (two beats long), to rest on the downbeat its beat ends at (bar 64: its
        // outro cut); the next one goes as it stops, from its main part (bar 24: its intro cut)
        let fast = shaped(m(174.0, 300.0, 0, 0.5), &song(&[(24, -12.0, false), (64, -9.0, true)]));
        let p = plan_structure(&a, end, 10_000, &fast).expect("a plan");
        assert!(p.tape_stop && !p.beatmatched && (p.rate, p.semitones, p.out_tempo) == (1.0, 0, 1.0), "{p:?}");
        assert_eq!(p.at_ms, (500.0 + 64.0 * bar - beat).round() as u64);
        assert_eq!(p.swap_ms, (2.0 * beat).round() as u64);
        // (it spins up in its intro: a phrase before its beat comes in, bar 16, from half its spin-up (a quarter
        // of the stop) before it, so that downbeat is at its speed)
        let stop = (2.0 * beat).clamp(600.0, 1600.0);
        assert_eq!(p.start_ms, (300.0 + 16.0 * fast_bar - stop / 4.0).round() as u64);
        assert_eq!(p.ramp_ms, (stop / 2.0).round() as u64);
        // a beat to its very end: the same, on its last downbeat; into one with no intro: from its start
        let flat = shaped(m(128.0, 500.0, 0, 0.5), &song(&[(88, -10.0, true)]));
        let p = plan_structure(&flat, end, 10_000, &shaped(m(174.0, 300.0, 0, 0.5), &song(&[(88, -9.0, true)]))).expect("a plan");
        assert!(p.tape_stop);
        assert_eq!((p.at_ms, p.start_ms), ((500.0 + 88.0 * bar - beat).round() as u64, (300.0 - stop / 4.0).round() as u64));
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
    // (OMNIPULSE_MIX_BANDS=1: a band at a time, not a wipe)
    let filter = std::env::var("OMNIPULSE_MIX_BANDS").is_ok_and(|v| v == "1");
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
    println!("phrases from bar {} of A, {} of B; B's drop {:?}", phrase_offset(&a), phrase_offset(&b), drop_bar(&b, phrase_offset(&b)));
    for (n, m) in [("A", &a), ("B", &b)] {
        println!("{n}: {:.2} BPM, {}, {} beats, bars {:?}", m.rhythm.bpm, m.rhythm.camelot, m.rhythm.beats.len(), m.rhythm.bar_db.iter().zip(&m.rhythm.bar_beat).map(|(d, b)| format!("{}/{:.1}", d.round() as i32, b)).collect::<Vec<_>>());
    }
    let t = |id: &str| Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.to_string(), ..Default::default() };
    let mut sa = rt.block_on(crate::player::open_source(&t(&a_id), rt.handle().clone())).expect("A opens");
    let sb = rt.block_on(crate::player::open_source(&t(&b_id), rt.handle().clone())).expect("B opens");
    let dur = sa.duration_ms().unwrap_or(0);
    // each one at its own loudness match (Settings > Sound's Normal: -14 LUFS), as the app plays them
    let db = |id: &str| {
        let lufs = rt.block_on(omnipulse_core::loudness::analyse("soundcloud", id)).ok().and_then(|a| a["lufs"].as_f64());
        crate::sound::loud_db(&serde_json::json!({}), &t(id), lufs, false)
    };
    let dbs = (db(&a_id), db(&b_id));
    println!("loudness matched: A {:+.1} dB, B {:+.1} dB", dbs.0, dbs.1);
    let p = plan_structure(&a, dur, 0, &b).expect("a plan");
    println!("{p:?}");
    let lead = 12_000;
    sa.seek(p.at_ms - lead).unwrap();
    let b_tempo = omnipulse_audio::Tempo::new(p.rate);
    let mut sb = omnipulse_audio::Stretched::shifted(sb, b_tempo.clone(), p.semitones).unwrap();
    omnipulse_audio::Source::seek(&mut sb, p.start_ms).unwrap();
    let rate = 44_100;
    let out = {
        let (target, a_tempo, b_tempo) = (p.out_tempo, omnipulse_audio::Tempo::new(1.0), b_tempo.clone());
        let share = p.ms as f64 / p.ramp_ms.max(1) as f64;
        let ramp = { let a_tempo = a_tempo.clone(); move |x: f64| { a_tempo.set(out_ramp(target, x * share)); b_tempo.set(in_ramp(target, x * share)); } };
        omnipulse_audio::engine::render_mix_ramped(sa, Box::new(sb), dbs, rate, lead as f32 / 1000.0, p.ms, Some(handoff(&p, filter)), (target != 1.0).then_some((&a_tempo, &ramp as &dyn Fn(f64))), lead as f32 / 1000.0 + 20.0 + p.ms as f32 / 1000.0).unwrap()
    };
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
        omnipulse_audio::engine::render_mix(sa, Box::new(sb), dbs, rate, lead as f32 / 1000.0, p.ms, Some(handoff(&p, filter)), lead as f32 / 1000.0 + p.ms as f32 / 1000.0 + 1.0).unwrap()
    };
    println!("the blend: lined up {:?}, half a beat off {:?}", held(&out), held(&off));
    // each of the next one's beats in the blend (where it lands, stretched) against the nearest of the one
    // playing's, as both were tracked whole: how far apart they are (ms), for this plan and the old one
    // each of the next one's beats in the blend against the nearest of the one playing's (as both were
    // tracked whole), as heard (ms of the mix): both followed through it a millisecond at a time, at their
    // tempos as the plan moves them (eased onto one, or the next one at its rate)
    let apart = |p: &Plan| {
        let ab: Vec<f64> = a.rhythm.beats.iter().map(|x| a.from_ms as f64 + x).collect();
        let bb: Vec<f64> = b.rhythm.beats.iter().map(|x| b.from_ms as f64 + x).collect();
        let tempos = |t: f64| if p.ramp_ms > 0 { let x = t / p.ramp_ms as f64; (out_ramp(p.out_tempo, x), in_ramp(p.out_tempo, x)) } else { (1.0, p.rate) };
        let until = if p.beatmatched { p.swap_ms } else { p.ms } as f64;
        let (mut pa, mut pb, mut t) = (p.at_ms as f64, p.start_ms as f64, 0.0);
        let mut next = bb.iter().position(|x| *x >= pb).unwrap_or(bb.len());
        let mut d = Vec::new();
        while t < until && next < bb.len() {
            let (fa, fb) = tempos(t);
            pa += fa;
            pb += fb;
            t += 1.0;
            if pb >= bb[next] {
                d.push(ab.iter().map(|y| (pa - y).abs()).fold(f64::MAX, f64::min) / fa);
                next += 1;
            }
        }
        let bar_of = |m: &omnipulse_core::rhythm::Measured, t: f64| m.rhythm.beats.iter().position(|x| m.from_ms as f64 + x >= t - 30.0).map(|i| (i as i64 - m.rhythm.downbeat as i64).rem_euclid(4));
        (d.len(), d.iter().sum::<f64>() / d.len().max(1) as f64, d.iter().copied().fold(0.0, f64::max), bar_of(&a, p.at_ms as f64), bar_of(&b, p.start_ms as f64))
    };
    println!("beats apart (how many, mean ms, worst ms, beat of A's bar it goes out on, of B's it comes in on): {:?}", apart(&p));
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

/// Where a song's kicks and snares fall in its bars, as tracked (for a downbeat that feels off): each
/// beat of the bar (and the half beat after it), its kick (lows) and snare (upper mids) at their
/// strongest within 30 ms, averaged over bars \`OMNIPULSE_PROBE_BARS\` ("24..32")
/// OMNIPULSE_PROBE_ID=1293532780 cargo test --release -p omnipulse --bin OmniPulse-native phase_probe -- --ignored --nocapture
#[test]
#[ignore]
fn phase_probe() {
    let id = std::env::var("OMNIPULSE_PROBE_ID").unwrap();
    let (from, to) = std::env::var("OMNIPULSE_PROBE_BARS").ok().and_then(|r| { let (a, b) = r.split_once("..")?; Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)) }).unwrap_or((24, 32));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let m = omnipulse_core::rhythm::known(&format!("full:{id}")).expect("measured whole");
    let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", &id, 1200.0)).unwrap();
    // the lows (one-pole at 120 Hz, twice) and the snare band (2-pole high-pass at 1.5 kHz, low-passed at 6)
    let lp = |x: &[f32], hz: f32| { let a = 1.0 - (-2.0 * std::f32::consts::PI * hz / rate as f32).exp(); let (mut z1, mut z2) = (0.0f32, 0.0f32); x.iter().map(|v| { z1 += a * (v - z1); z2 += a * (z1 - z2); z2 }).collect::<Vec<f32>>() };
    let low = lp(&mono, 120.0);
    let l15 = lp(&mono, 1500.0);
    let l6 = lp(&mono, 6000.0);
    let mid: Vec<f32> = l6.iter().zip(&l15).map(|(a, b)| a - b).collect();
    let peak = |e: &[f32], ms: f64| { let c = (ms / 1000.0 * rate as f64) as i64; let w = (0.03 * rate as f64) as i64; (c - w..c + w).filter(|&i| i >= 0 && (i as usize) < e.len()).map(|i| e[i as usize].abs()).fold(0.0f32, f32::max) };
    let mut table = [[0.0f32; 4]; 8];
    let beats: Vec<f64> = m.rhythm.beats.iter().map(|b| m.from_ms as f64 + b).collect();
    for bar in from..to {
        for ph in 0..4 {
            let i = m.rhythm.downbeat + bar * 4 + ph;
            let (Some(&t), Some(&t2)) = (beats.get(i), beats.get(i + 1)) else { continue };
            let half = (t + t2) / 2.0;
            table[ph * 2][0] += peak(&low, t);
            table[ph * 2][1] += peak(&mid, t);
            table[ph * 2 + 1][0] += peak(&low, half);
            table[ph * 2 + 1][1] += peak(&mid, half);
        }
    }
    let n = (to - from) as f32;
    println!("{id}: bars {from}..{to} (downbeat beat {}), kick / snare by where in the bar:", m.rhythm.downbeat);
    for (i, r) in table.iter().enumerate() {
        println!("  {}{}  kick {:.3}  snare {:.3}", i / 2 + 1, if i % 2 == 1 { " and" } else { "    " }, r[0] / n, r[1] / n);
    }
}

/// Where in each beat a song's hits start, as tracked: its snare band's (and its lows') onsets over bars
/// \`OMNIPULSE_PROBE_BARS\`, in 16 steps of a beat (0: on the beat, 8: the half beat): a grid that's off
/// shows its hits a few steps in, all the same way
#[test]
#[ignore]
fn onset_probe() {
    let id = std::env::var("OMNIPULSE_PROBE_ID").unwrap();
    let (from, to) = std::env::var("OMNIPULSE_PROBE_BARS").ok().and_then(|r| { let (a, b) = r.split_once("..")?; Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)) }).unwrap_or((24, 32));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let m = omnipulse_core::rhythm::known(&format!("full:{id}")).expect("measured whole");
    let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", &id, 1200.0)).unwrap();
    let lp = |x: &[f32], hz: f32| { let a = 1.0 - (-2.0 * std::f32::consts::PI * hz / rate as f32).exp(); let (mut z1, mut z2) = (0.0f32, 0.0f32); x.iter().map(|v| { z1 += a * (v - z1); z2 += a * (z1 - z2); z2 }).collect::<Vec<f32>>() };
    let (l15, l6) = (lp(&mono, 1500.0), lp(&mono, 6000.0));
    let bands = [("snare", l6.iter().zip(&l15).map(|(a, b)| a - b).collect::<Vec<f32>>()), ("lows", lp(&mono, 120.0))];
    let beats: Vec<f64> = m.rhythm.beats.iter().map(|b| m.from_ms as f64 + b).collect();
    for (name, x) in &bands {
        // its envelope (5 ms), and where it rises (over 10 ms)
        let hop = (rate as usize) / 1000;
        let env: Vec<f32> = x.chunks(hop).map(|c| c.iter().map(|v| v * v).sum::<f32>().sqrt()).collect();
        let rise = |ms: usize| if ms >= 10 && ms < env.len() { (env[ms] - env[ms - 10]).max(0.0) } else { 0.0 };
        let mut bins = [0.0f32; 16];
        for i in m.rhythm.downbeat + from * 4..(m.rhythm.downbeat + to * 4).min(beats.len() - 1) {
            let (t0, t1) = (beats[i], beats[i + 1]);
            for ms in t0 as usize..t1 as usize {
                let k = (((ms as f64 - t0) / (t1 - t0)) * 16.0) as usize;
                bins[k.min(15)] += rise(ms);
            }
        }
        let top = bins.iter().copied().fold(0.0f32, f32::max).max(1e-9);
        println!("{id} bars {from}..{to} {name:5}: {}", bins.iter().map(|b| format!("{:>3}", (b / top * 99.0).round() as i32)).collect::<Vec<_>>().join(" "));
    }
}

/// How far apart the two are as heard in a mix's run-up: each rendered alone, as the mix plays it
/// (stretched, eased), their hits' rises cross-correlated over ±300 ms: the lag that fits best is how
/// far the next one is off (positive: late)
/// OMNIPULSE_MIX_A=.. OMNIPULSE_MIX_B=.. cargo test --release -p omnipulse --bin OmniPulse-native align_probe -- --ignored --nocapture
#[test]
#[ignore]
fn align_probe() {
    use crate::track::{Src, Track};
    let (a_id, b_id) = (std::env::var("OMNIPULSE_MIX_A").unwrap(), std::env::var("OMNIPULSE_MIX_B").unwrap());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let full = |id: &str| omnipulse_core::rhythm::known(&format!("full:{id}")).expect("measured whole");
    let (a, b) = (full(&a_id), full(&b_id));
    let t = |id: &str| Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.to_string(), ..Default::default() };
    let open = |id: &str| rt.block_on(crate::player::open_source(&t(id), rt.handle().clone())).unwrap();
    let dur = open(&a_id).duration_ms().unwrap_or(0);
    let p = plan_structure(&a, dur, 0, &b).expect("a plan");
    println!("{p:?}");
    let (rate, lead) = (44_100u32, 4000u64);
    let silent = || -> Box<dyn omnipulse_audio::Source> { Box::new(omnipulse_audio::Tone::new(rate, 0.0, 0.0, 900.0)) };
    let render = |with_a: bool| {
        let mut sa = if with_a { open(&a_id) } else { silent() };
        sa.seek(p.at_ms - lead).unwrap();
        let b_tempo = omnipulse_audio::Tempo::new(p.rate);
        let sb: Box<dyn omnipulse_audio::Source> = if with_a { silent() } else {
            let mut s = omnipulse_audio::Stretched::shifted(open(&b_id), b_tempo.clone(), p.semitones).unwrap();
            omnipulse_audio::Source::seek(&mut s, p.start_ms).unwrap();
            Box::new(s)
        };
        let (target, a_tempo) = (p.out_tempo, omnipulse_audio::Tempo::new(1.0));
        let share = p.ms as f64 / p.ramp_ms.max(1) as f64;
        let ramp = { let a_tempo = a_tempo.clone(); move |x: f64| { a_tempo.set(out_ramp(target, x * share)); b_tempo.set(in_ramp(target, x * share)); } };
        // (a plain crossfade: each one whole, at its gain; the gains don't move a hit)
        omnipulse_audio::engine::render_mix_ramped(sa, sb, (0.0, 0.0), rate, lead as f32 / 1000.0, p.ms, None, (target != 1.0).then_some((&a_tempo, &ramp as &dyn Fn(f64))), lead as f32 / 1000.0 + p.ms as f32 / 1000.0).unwrap()
    };
    let (oa, ob) = (render(true), render(false));
    // each one's rises (1 ms steps), over the run-up but its first second and last half
    let rises = |x: &[f32]| {
        let hop = rate as usize / 1000;
        let env: Vec<f32> = x.chunks(hop * 2).map(|c| c.iter().map(|v| v * v).sum::<f32>().sqrt()).collect();
        (0..env.len()).map(|i| if i >= 8 { (env[i] - env[i - 8]).max(0.0) } else { 0.0 }).collect::<Vec<f32>>()
    };
    let (ra, rb) = (rises(&oa), rises(&ob));
    let (from, to) = (lead as usize + 1000, lead as usize + p.swap_ms as usize - 500);
    let corr = |lag: i64| (from..to).map(|i| ra[i] as f64 * rb.get((i as i64 + lag) as usize).copied().unwrap_or(0.0) as f64).sum::<f64>();
    let mut all: Vec<(i64, f64)> = (-300..=300).map(|l| (l, corr(l))).collect();
    let zero = corr(0);
    all.sort_by(|x, y| y.1.total_cmp(&x.1));
    println!("the run-up ({} s): best fits at lags {:?} (ms: positive, the next one late); at 0: {:.0}% of the best", (to - from) / 1000, all.iter().take(5).map(|(l, c)| format!("{l} ({:.0}%)", c / all[0].1 * 100.0)).collect::<Vec<_>>(), zero / all[0].1 * 100.0);
}

/// A song measured whole again, with the analysis' debug lines (OMNIPULSE_RHYTHM_DEBUG)
#[test]
#[ignore]
fn rhythm_debug_probe() {
    let id = std::env::var("OMNIPULSE_PROBE_ID").unwrap();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", &id, 1200.0)).unwrap();
    let r = omnipulse_core::rhythm::analyse(&mono, rate).unwrap();
    println!("{id}: {} BPM, first beat {:.0} ms, downbeat {}", r.bpm, r.beats[0], r.downbeat);
}

/// Where a song's kicks land against its tracked beats, finely: its lows' (below 150 Hz) energy over
/// 8 ms, where it rises most within each beat, as ms after the beat (median, and the spread), over
/// bars \`OMNIPULSE_PROBE_BARS\`
#[test]
#[ignore]
fn kick_probe() {
    let id = std::env::var("OMNIPULSE_PROBE_ID").unwrap();
    let (from, to) = std::env::var("OMNIPULSE_PROBE_BARS").ok().and_then(|r| { let (a, b) = r.split_once("..")?; Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)) }).unwrap_or((24, 32));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let m = omnipulse_core::rhythm::known(&format!("full:{id}")).expect("measured whole");
    let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", &id, 1200.0)).unwrap();
    let lp = |x: &[f32], hz: f32| { let a = 1.0 - (-2.0 * std::f32::consts::PI * hz / rate as f32).exp(); let (mut z1, mut z2) = (0.0f32, 0.0f32); x.iter().map(|v| { z1 += a * (v - z1); z2 += a * (z1 - z2); z2 }).collect::<Vec<f32>>() };
    let low = lp(&mono, 150.0);
    let hi: Vec<f32> = { let l = lp(&mono, 2000.0); mono.iter().zip(&l).map(|(a, b)| a - b).collect() };
    let per_ms = rate as usize / 1000;
    // energy over 8 ms, every ms
    let env = |x: &[f32]| (0..x.len() / per_ms).map(|i| { let a = i * per_ms; let b = (a + 8 * per_ms).min(x.len()); (x[a..b].iter().map(|v| v * v).sum::<f32>() / (b - a) as f32).sqrt() }).collect::<Vec<f32>>();
    let beats: Vec<f64> = m.rhythm.beats.iter().map(|b| m.from_ms as f64 + b).collect();
    for (name, e) in [("kick (lows)", env(&low)), ("hats (highs)", env(&hi))] {
        let mut at = Vec::new();
        for i in m.rhythm.downbeat + from * 4..(m.rhythm.downbeat + to * 4).min(beats.len() - 1) {
            // from a quarter beat before it to three quarters after: where it rises most (over 6 ms)
            let (t, p) = (beats[i], beats[i + 1] - beats[i]);
            let (a, b) = ((t - p / 4.0).max(6.0) as usize, ((t + p * 0.75) as usize).min(e.len() - 1));
            let best = (a..b).max_by(|&x, &y| (e[x] - e[x - 6]).total_cmp(&(e[y] - e[y - 6]))).unwrap();
            at.push(best as f64 - 3.0 - t);
        }
        at.sort_by(|x, y| x.total_cmp(y));
        let q = |f: f64| at[((at.len() - 1) as f64 * f) as usize];
        println!("{id} bars {from}..{to} {name}: rises {:.0} ms after the beat (median; quartiles {:.0} / {:.0}; beat {:.0} ms)", q(0.5), q(0.25), q(0.75), m.rhythm.period_ms);
    }
}

/// A song's loudness and kick beat by beat over bars \`OMNIPULSE_PROBE_BARS\` (to find where its drop is)
#[test]
#[ignore]
fn beat_probe() {
    let id = std::env::var("OMNIPULSE_PROBE_ID").unwrap();
    let (from, to) = std::env::var("OMNIPULSE_PROBE_BARS").ok().and_then(|r| { let (a, b) = r.split_once("..")?; Some((a.parse::<usize>().ok()?, b.parse::<usize>().ok()?)) }).unwrap_or((28, 36));
    let rt = tokio::runtime::Runtime::new().unwrap();
    let m = omnipulse_core::rhythm::known(&format!("full:{id}")).expect("measured whole");
    let (mono, rate) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", &id, 1200.0)).unwrap();
    let lp = |x: &[f32], hz: f32| { let a = 1.0 - (-2.0 * std::f32::consts::PI * hz / rate as f32).exp(); let (mut z1, mut z2) = (0.0f32, 0.0f32); x.iter().map(|v| { z1 += a * (v - z1); z2 += a * (z1 - z2); z2 }).collect::<Vec<f32>>() };
    let low = lp(&mono, 120.0);
    let beats: Vec<f64> = m.rhythm.beats.iter().map(|b| m.from_ms as f64 + b).collect();
    let db = |x: &[f32], a: f64, b: f64| { let (i, j) = ((a / 1000.0 * rate as f64) as usize, ((b / 1000.0 * rate as f64) as usize).min(x.len())); 10.0 * ((x[i..j].iter().map(|v| v * v).sum::<f32>() / (j - i).max(1) as f32).max(1e-12)).log10() };
    for bar in from..to {
        let row: Vec<String> = (0..4).map(|ph| {
            let i = m.rhythm.downbeat + bar * 4 + ph;
            let (t, t2) = (beats[i], beats[(i + 1).min(beats.len() - 1)]);
            format!("{:>4.0}/{:>4.0}", db(&mono, t, t2), db(&low, t, t2))
        }).collect();
        println!("bar {bar:>3} at {:>6.0} ms: {}   (all/lows dB, a beat each)", beats[m.rhythm.downbeat + bar * 4], row.join("  "));
    }
}

/// The demo's two songs as separate files, each whole (no filters, no crossfade), on the demo's
/// timeline: as stretched, eased and key-shifted as the mix has them, at their loudness match, the
/// one playing going on after the switch-over (at its eased tempo) and the next one at its own, for
/// mixing by hand. OMNIPULSE_MIX_A / _B as mix_demo; OMNIPULSE_MIX_OUT the path before "-A.wav"
#[test]
#[ignore]
fn mix_stems() {
    use crate::track::{Src, Track};
    use omnipulse_audio::engine::Pipeline;
    let (a_id, b_id) = (std::env::var("OMNIPULSE_MIX_A").unwrap(), std::env::var("OMNIPULSE_MIX_B").unwrap());
    let base = std::env::var("OMNIPULSE_MIX_OUT").unwrap_or_else(|_| "mix-stems".into());
    let rt = tokio::runtime::Runtime::new().unwrap();
    let full = |id: &str| omnipulse_core::rhythm::known(&format!("full:{id}")).expect("measured whole");
    let (a, b) = (full(&a_id), full(&b_id));
    let t = |id: &str| Track { src: Src::SoundCloud(id.parse().unwrap()), key: id.to_string(), ..Default::default() };
    let open = |id: &str| rt.block_on(crate::player::open_source(&t(id), rt.handle().clone())).unwrap();
    let dur = open(&a_id).duration_ms().unwrap_or(0);
    let p = plan_structure(&a, dur, 0, &b).expect("a plan");
    println!("{p:?}");
    let db = |id: &str| {
        let lufs = rt.block_on(omnipulse_core::loudness::analyse("soundcloud", id)).ok().and_then(|a| a["lufs"].as_f64());
        crate::sound::loud_db(&serde_json::json!({}), &t(id), lufs, false)
    };
    let (rate, lead) = (44_100u32, 12_000u64);
    let total = (lead + p.ms + 20_000) as usize * rate as usize / 1000 * 2;
    let lead_n = lead as usize * rate as usize / 1000 * 2;
    // (where in the mix it is, 0..1 of the ramp: from the frames given after the lead)
    let ramp_x = |given: usize| if p.ramp_ms == 0 { 1.0 } else { ((given.saturating_sub(lead_n) / 2) as f64 * 1000.0 / rate as f64 / p.ramp_ms as f64).min(1.0) };
    // the one playing: as it is to the mix's moment, then through the stretch, eased
    let mut pa = Pipeline::new(open(&a_id), rate);
    pa.seek(p.at_ms - lead).unwrap();
    pa.chain.set_match_db(db(&a_id), true);
    let a_tempo = omnipulse_audio::Tempo::new(1.0);
    let mut out_a = Vec::new();
    while out_a.len() < lead_n { if !pa.fill(&mut out_a).unwrap() { break; } }
    // (on from there through the stretch: nothing read past the moment let go)
    pa.stretch(a_tempo.clone()).unwrap();
    while out_a.len() < total {
        if p.out_tempo != 1.0 { a_tempo.set(out_ramp(p.out_tempo, ramp_x(out_a.len()))); }
        if !pa.fill(&mut out_a).unwrap() { break; }
    }
    out_a.resize(total, 0.0);
    // the next one: silent through the lead, then from where the mix has it, eased to its own tempo
    let b_tempo = omnipulse_audio::Tempo::new(p.rate);
    let mut sb = omnipulse_audio::Stretched::shifted(open(&b_id), b_tempo.clone(), p.semitones).unwrap();
    omnipulse_audio::Source::seek(&mut sb, p.start_ms).unwrap();
    let mut pb = Pipeline::new(Box::new(sb), rate);
    pb.chain.set_match_db(db(&b_id), true);
    let mut out_b = vec![0.0f32; lead_n];
    while out_b.len() < total {
        b_tempo.set(if p.out_tempo != 1.0 { in_ramp(p.out_tempo, ramp_x(out_b.len())) } else if ramp_x(out_b.len()) < 1.0 || p.ramp_ms == 0 && out_b.len() < lead_n + p.ms as usize * rate as usize / 1000 * 2 { p.rate } else { 1.0 });
        if !pb.fill(&mut out_b).unwrap() { break; }
    }
    out_b.resize(total, 0.0);
    let wav = |path: &str, out: &[f32]| {
        let mut w = Vec::new();
        let data = (out.len() * 2) as u32;
        w.extend_from_slice(b"RIFF"); w.extend_from_slice(&(36 + data).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt "); w.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] { w.extend_from_slice(&v.to_le_bytes()); }
        for v in [rate, rate * 4] { w.extend_from_slice(&v.to_le_bytes()); }
        for v in [4u16, 16] { w.extend_from_slice(&v.to_le_bytes()); }
        w.extend_from_slice(b"data"); w.extend_from_slice(&data.to_le_bytes());
        for s in out { w.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); }
        std::fs::write(path, w).unwrap();
    };
    wav(&format!("{base}-A.wav"), &out_a);
    wav(&format!("{base}-B.wav"), &out_b);
    println!("wrote {base}-A.wav and -B.wav: the mix from {:.1} s, the switch-over at {:.1} s", lead as f64 / 1000.0, (lead + p.swap_ms) as f64 / 1000.0);
}

/// Which cues tell a grid on the beats from one half a beat out, on songs whose grids are known right:
/// each cue's hits on the beats against halfway between (above 1: more between)
#[test]
#[ignore]
fn kick_cues_probe() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let rate = 22_050usize;
    for id in std::env::var("OMNIPULSE_PROBE_IDS").unwrap().split(',') {
        let Ok((mono, r0)) = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", id, 1200.0)) else { println!("{id}: no stream"); continue };
        let r = omnipulse_core::rhythm::analyse(&mono, r0).unwrap();
        let x = omnipulse_core::rhythm::at_analysis_rate(&mono, r0);
        let pole = |hz: f64| 1.0 - (-2.0 * std::f64::consts::PI * hz / rate as f64).exp();
        let lp = |x: &[f32], hz: f64| { let a = pole(hz); let (mut z1, mut z2) = (0.0f64, 0.0f64); x.iter().map(|&v| { z1 += a * (v as f64 - z1); z2 += a * (z1 - z2); z2 }).collect::<Vec<f64>>() };
        let (l40, l120, l1k, l4k, l6k) = (lp(&x, 40.0), lp(&x, 120.0), lp(&x, 1000.0), lp(&x, 4000.0), lp(&x, 6000.0));
        let bands: Vec<(&str, Vec<f64>)> = vec![
            ("lows", l120.iter().zip(&l40).map(|(a, b)| a - b).collect()),
            ("mids 1-4k", l4k.iter().zip(&l1k).map(|(a, b)| a - b).collect()),
            ("highs 6k+", x.iter().zip(&l6k).map(|(a, b)| *a as f64 - b).collect()),
        ];
        let per = rate / 1000;
        let env = |b: &[f64]| (0..b.len() / per).map(|i| { let a = i * per; let e = (a + 5 * per).min(b.len()); (b[a..e].iter().map(|v| v * v).sum::<f64>() / (e - a).max(1) as f64).sqrt() }).collect::<Vec<f64>>();
        let envs: Vec<(&str, Vec<f64>)> = bands.iter().map(|(n, b)| (*n, env(b))).collect();
        // a cue's rise near a moment: the biggest rise over 10 ms within 20 ms of it
        let rise = |e: &[f64], t: f64| { let c = t.round() as i64; (c - 20..=c + 20).filter(|&i| i >= 10 && (i as usize) < e.len()).map(|i| (e[i as usize] - e[i as usize - 10]).max(0.0)).fold(0.0, f64::max) };
        let halves: Vec<f64> = r.beats.windows(2).map(|w| (w[0] + w[1]) / 2.0).collect();
        let ratio = |e: &[f64]| { let on: f64 = r.beats.iter().map(|&t| rise(e, t)).sum::<f64>() / r.beats.len() as f64; let off: f64 = halves.iter().map(|&t| rise(e, t)).sum::<f64>() / halves.len() as f64; off / on.max(1e-12) };
        // the kick's click: lows and highs rising together
        let both = |t: f64| rise(&envs[0].1, t) * rise(&envs[2].1, t);
        let click = { let on: f64 = r.beats.iter().map(|&t| both(t)).sum::<f64>(); let off: f64 = halves.iter().map(|&t| both(t)).sum::<f64>(); off / on.max(1e-18) * r.beats.len() as f64 / halves.len() as f64 };
        // claps and snares: beats 2 and 4 of the bar (odd from the downbeat) against the half beats after them
        let snare = { let e = &envs[1].1; let odd: Vec<usize> = (0..r.beats.len() - 1).filter(|i| (*i + 4 - r.downbeat % 4) % 2 == 1).collect(); let on: f64 = odd.iter().map(|&i| rise(e, r.beats[i])).sum::<f64>(); let off: f64 = odd.iter().map(|&i| rise(e, halves[i])).sum::<f64>(); off / on.max(1e-12) };
        println!("{id} ({} BPM): between/on  {}  click {click:.2}  snare(2,4) {snare:.2}", r.bpm, envs.iter().map(|(n, e)| format!("{n} {:.2}", ratio(e))).collect::<Vec<_>>().join("  "));
    }
}

/// SoundCloud tracks for a search, with their tempo measured whole (for picking demo pairs):
/// OMNIPULSE_PROBE_Q="drum and bass" (and _N, how many)
#[test]
#[ignore]
fn find_probe() {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let q = std::env::var("OMNIPULSE_PROBE_Q").unwrap();
    let n: usize = std::env::var("OMNIPULSE_PROBE_N").ok().and_then(|v| v.parse().ok()).unwrap_or(8);
    let r = rt.block_on(omnipulse_core::soundcloud::api_get(format!("/tracks?q={}&access=playable&limit=20&linked_partitioning=true", q.replace(' ', "%20")))).unwrap();
    let list = r["collection"].as_array().cloned().unwrap_or_default();
    for t in list.iter().filter(|t| t["duration"].as_u64().is_some_and(|d| (120_000..420_000).contains(&d))).take(n) {
        let id = t["id"].as_u64().unwrap().to_string();
        let got = rt.block_on(omnipulse_core::loudness::mono_start("soundcloud", &id, 1200.0));
        let Ok((mono, rate)) = got else { println!("{id}: no stream  {}", t["title"].as_str().unwrap_or("")); continue };
        match omnipulse_core::rhythm::analyse(&mono, rate) {
            Some(r) => {
                omnipulse_core::rhythm::keep(&format!("full:{id}"), omnipulse_core::rhythm::Measured { from_ms: 0, to_ms: mono.len() as u64 * 1000 / rate as u64, rhythm: r.clone(), v: omnipulse_core::rhythm::VERSION });
                println!("{id}: {:.1} BPM ({:.2}), {}  | {} - {}", r.bpm, r.tempo_confidence, r.camelot, t["user"]["username"].as_str().unwrap_or(""), t["title"].as_str().unwrap_or(""));
            }
            None => println!("{id}: no beat  {}", t["title"].as_str().unwrap_or("")),
        }
    }
}

/// What each planner makes of two measured songs (state/rhythm.json's whole measurements), as the app
/// would at `OMNIPULSE_PROBE_POS` ms into the first: OMNIPULSE_PROBE_A, OMNIPULSE_PROBE_B (track keys)
#[test]
#[ignore]
fn pair_probe() {
    let k = |v: &str| omnipulse_core::rhythm::known(&format!("full:{}", std::env::var(v).unwrap())).expect("measured whole");
    let (a, b) = (k("OMNIPULSE_PROBE_A"), k("OMNIPULSE_PROBE_B"));
    let pos: u64 = std::env::var("OMNIPULSE_PROBE_POS").ok().and_then(|x| x.parse().ok()).unwrap_or(60_000);
    let end = a.to_ms;
    println!("a: {:.2} BPM (conf {:.2}), {} ({:.2}), {} bars, phrase off {}, drop {:?}, last drop {:?}, content end {}", a.rhythm.bpm, a.rhythm.tempo_confidence, a.rhythm.camelot, a.rhythm.key_confidence, a.rhythm.bar_db.len(), phrase_offset(&a), drop_bar(&a, phrase_offset(&a)), last_drop(&a, phrase_offset(&a)), content_end(&a, a.rhythm.bar_db.len()));
    println!("b: {:.2} BPM (conf {:.2}), {} ({:.2}), {} bars, phrase off {}, drop {:?}, content start {}", b.rhythm.bpm, b.rhythm.tempo_confidence, b.rhythm.camelot, b.rhythm.key_confidence, b.rhythm.bar_db.len(), phrase_offset(&b), drop_bar(&b, phrase_offset(&b)), content_start(&b, b.rhythm.bar_db.len()));
    println!("keys fit {}, key shift {}", keys_fit(&a, &b), key_shift(&a, &b));
    println!("drop swap: {:#?}", plan_drop_swap(&a, end, pos, &b));
    println!("melody: {:#?}", plan_melody(&a, end, pos, &b));
    println!("structure: {:#?}", plan_structure(&a, end, pos, &b));
    println!("grid: {:#?}", plan(&a, end, pos, &b, 16));
}
