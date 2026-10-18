// A transition's plan (what the engine plays: where, how long, at what tempo and key), and the curves its
// tempo is ridden on; shared by the desktop app and the Android app

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
    /// a transition of its own, if it is one: a roll (smart::ROLL)
    pub style: Option<&'static str>,
    /// of `bars`, the one playing's after the swap (Smart Transitions', its outro going); 0: the usual 4
    pub tail_bars: u32,
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

/// The tempo while easing back after the mix: from `rate` to 1.0 over `ms`, `t` ms in (smoothly)
pub fn ease(rate: f64, t: u64, ms: u64) -> f64 {
    if t >= ms { return 1.0; }
    let p = t as f64 / ms.max(1) as f64;
    let s = p * p * (3.0 - 2.0 * p);
    rate + (1.0 - rate) * s
}
