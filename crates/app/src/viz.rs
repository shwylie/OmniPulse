// Now Playing's visualiser: the sound as it's played (an engine tap, only while Now Playing shows one),
// as 32 bands from 40 Hz to 16 kHz, each 0 to 1, risen to at once and fallen from slowly, as a meter's.

use std::sync::Mutex;

/// The last of the sound, in mono, as it was sent out (a ring, written from `at`)
const RING: usize = 1 << 15;
/// (4096: a longer stretch of the sound each time, so the low end steadier)
const FFT: usize = 4096;
/// the Wave's points: the bands, eased into a curve between them
pub const CURVE: usize = 96;
pub const BANDS: usize = 32;

struct Ring { buf: Vec<f32>, at: usize, rate: u32 }

static SOUND: Mutex<Option<Ring>> = Mutex::new(None);
/// each band as it's shown (falling slowly from its peaks), and each one's top (held, then falling)
static SHOWN: Mutex<[f32; BANDS]> = Mutex::new([0.0; BANDS]);
static PEAKS: Mutex<[(f32, u32); BANDS]> = Mutex::new([(0.0, 0); BANDS]);

/// Each band's recent top (for Peaks): held for half a second, then falling
pub fn peaks(b: &[f32; BANDS]) -> [f32; BANDS] {
    let mut p = PEAKS.lock().unwrap();
    let mut out = [0.0; BANDS];
    for (k, v) in b.iter().enumerate() {
        let (top, held) = &mut p[k];
        if *v >= *top { *top = *v; *held = 15; } else if *held > 0 { *held -= 1; } else { *top = (*top - 0.015).max(*v); }
        out[k] = *top;
    }
    out
}

/// What the engine's tap feeds (its frames, stereo, at `rate`)
pub fn tap(rate: u32) -> omnipulse_audio::Tap {
    *SOUND.lock().unwrap() = Some(Ring { buf: vec![0.0; RING], at: 0, rate: rate.max(1) });
    Box::new(move |frames: &[f32]| {
        let Ok(mut g) = SOUND.lock() else { return };
        let Some(r) = g.as_mut() else { return };
        for f in frames.chunks_exact(2) {
            r.buf[r.at] = (f[0] + f[1]) * 0.5;
            r.at = (r.at + 1) % RING;
        }
    })
}

/// No more of it: its sound let go, its bands down
/// How loud it is now, 0 to 1, for the Glow: its low end counted most, the rest too (so a quiet intro
/// still moves it)
pub fn level(b: &[f32; BANDS]) -> f32 {
    let low = b[..8].iter().sum::<f32>() / 8.0;
    let all = b.iter().sum::<f32>() / BANDS as f32;
    (0.65 * low + 0.6 * all).min(1.0)
}

pub fn stop() {
    *SOUND.lock().unwrap() = None;
    *SHOWN.lock().unwrap() = [0.0; BANDS];
    *PEAKS.lock().unwrap() = [(0.0, 0); BANDS];
    *RING_TOP.lock().unwrap() = 0.08;
    *RING_EASE.lock().unwrap() = [Vec::new(), Vec::new()];
    *RING_BASS.lock().unwrap() = [0.0, 0.0];
}

/// The self-test's music (OMNIPULSE_SELFTEST: never heard, only drawn): a beat put straight into the tap's
/// sound, `t` seconds in (a kick on each beat, a bass note, hi-hats between), a frame's worth each call
pub fn test_feed(t: f32) {
    let Ok(mut g) = SOUND.lock() else { return };
    let Some(r) = g.as_mut() else { return };
    let rate = r.rate as f32;
    let n = (rate / 30.0) as usize;
    for i in 0..n {
        let x = t + i as f32 / rate;
        let beat = (x * 2.0).fract();
        let kick = (std::f32::consts::TAU * (55.0 + 90.0 * (-beat * 30.0).exp()) * x).sin() * (-beat * 9.0).exp() * 0.8;
        let bass = (std::f32::consts::TAU * 82.0 * x).sin() * 0.25 * (0.6 + 0.4 * (x * 0.5).sin());
        let half = ((x * 4.0).fract() - 0.5).abs();
        let hat = ((x * 9_871.0).sin() * (x * 13_331.0).sin()) * (-half * 40.0).exp() * 0.25;
        let pad = (std::f32::consts::TAU * 440.0 * x).sin() * 0.05 + (std::f32::consts::TAU * 2_200.0 * x).sin() * 0.03;
        r.buf[r.at] = kick + bass + hat + pad;
        r.at = (r.at + 1) % RING;
    }
}

/// The bands now: from the sound just sent to the speakers (the tap is fed as it's played), each 0 to 1
pub fn bands() -> [f32; BANDS] {
    let (window, rate) = {
        let g = SOUND.lock().unwrap();
        let Some(r) = g.as_ref() else { return [0.0; BANDS] };
        let end = r.at;
        let w: Vec<f32> = (0..FFT).map(|i| r.buf[(end + RING - FFT + i) % RING]).collect();
        (w, r.rate)
    };
    use realfft::RealFftPlanner;
    thread_local! { static PLAN: std::cell::RefCell<RealFftPlanner<f32>> = std::cell::RefCell::new(RealFftPlanner::new()); }
    let fft = PLAN.with(|p| p.borrow_mut().plan_fft_forward(FFT));
    // (a Hann window: no edge of the stretch heard as a click across every band)
    let mut input: Vec<f32> = window.iter().enumerate().map(|(i, v)| v * (0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / FFT as f32).cos())).collect();
    let mut spec = fft.make_output_vec();
    if fft.process(&mut input, &mut spec).is_err() { return [0.0; BANDS]; }
    let hz_per = rate as f32 / FFT as f32;
    let mut out = [0.0f32; BANDS];
    let (lo, hi) = (40.0f32, 16_000.0f32);
    for (b, o) in out.iter_mut().enumerate() {
        let f0 = lo * (hi / lo).powf(b as f32 / BANDS as f32);
        let f1 = lo * (hi / lo).powf((b + 1) as f32 / BANDS as f32);
        let (k0, k1) = ((f0 / hz_per) as usize, ((f1 / hz_per) as usize).max((f0 / hz_per) as usize + 1));
        let peak = spec[k0.min(spec.len() - 1)..k1.min(spec.len())].iter().map(|c| c.norm()).fold(0.0f32, f32::max);
        // (as dB, -60 to 0 of a full-scale tone's, read as 0 to 1; the highs lifted a little, as they're quieter)
        let db = 20.0 * (peak / (FFT as f32 / 4.0)).max(1e-6).log10() + 6.0 * (b as f32 / BANDS as f32);
        *o = ((db + 60.0) / 60.0).clamp(0.0, 1.0);
    }
    // smoothed: each band a little of its neighbours (no one bar flickering alone), then eased towards
    // where it is now, rising faster than it falls (a meter's ballistics, not a jump each frame)
    let spread: Vec<f32> = (0..BANDS).map(|k| {
        let (l, r) = (out[k.saturating_sub(1)], out[(k + 1).min(BANDS - 1)]);
        0.25 * l + 0.5 * out[k] + 0.25 * r
    }).collect();
    let mut shown = SHOWN.lock().unwrap();
    for (s, v) in shown.iter_mut().zip(spread) { *s += (v - *s) * if v > *s { 0.38 } else { 0.14 }; }
    *shown
}

/// The Wave's curve: the bands as CURVE points, eased between them (a smooth line, not steps)
pub fn curve(b: &[f32; BANDS]) -> [f32; CURVE] {
    let mut out = [0.0; CURVE];
    for (i, o) in out.iter_mut().enumerate() {
        let x = i as f32 * (BANDS - 1) as f32 / (CURVE - 1) as f32;
        let k = x.floor() as usize;
        let t = x - k as f32;
        let (a, c) = (b[k], b[(k + 1).min(BANDS - 1)]);
        // (eased, not straight: a curve through them)
        let e = t * t * (3.0 - 2.0 * t);
        *o = a + (c - a) * e;
    }
    out
}

/// The Ring: a smooth halo round the cover (as Proximity's round its logo), swelling with the music: the bass
/// at the foot, the highs at the top, the left half and the right mirrored. Its edge one smooth curve through
/// a point for each slice (each point its band and the bands beside it, eased frame to frame), so it breathes
/// rather than spikes; how far it swells, measured against the song's own recent loudest, so a quiet song
/// moves it as much as a loud one. As path commands in a 1000-unit square (the box it's drawn in), in
/// slices, each its own step of the accent's gradient (the software renderer fills a path with one colour),
/// each running a little into the next so no seam shows. `inner`: the circle it swells from, in those units
fn ring_shape(b: &[f32; BANDS], inner: f32, slot: usize) -> Vec<(String, f32)> {
    const OUT: f32 = 494.0;
    let depth = OUT - inner;
    let n = RING_PIECES;
    let step = std::f32::consts::TAU / n as f32;
    // (a band's level at an angle from straight down, round to straight up: 0 the bass, 1 the highs; between
    // bands, the two either side blended, and a little of their neighbours)
    let level = |th: f32| -> (f32, f32) {
        let th = th.rem_euclid(std::f32::consts::TAU);
        let away = if th <= std::f32::consts::PI { th } else { std::f32::consts::TAU - th } / std::f32::consts::PI;
        let x = away * (BANDS - 1) as f32;
        let k = x.floor() as usize;
        let t = x - k as f32;
        let at = |i: isize| b[(i.clamp(0, BANDS as isize - 1)) as usize];
        let (k, ki) = (k as isize, k as isize);
        let here = at(k) * (1.0 - t) + at(k + 1) * t;
        let around = (at(ki - 1) + at(ki + 2)) * 0.5;
        (0.75 * here + 0.25 * around, away)
    };
    // (how far it swells: against the loudest it's been lately, slowly forgotten, so every song moves it)
    let now_top = b.iter().cloned().fold(0.0f32, f32::max);
    // (forgotten within a couple of seconds: a quiet part after a loud one still moves it)
    let top = {
        let mut m = RING_TOP.lock().unwrap();
        *m = (*m * 0.985).max(now_top).max(0.08);
        *m
    };
    let raw: Vec<(f32, f32)> = (0..n).map(|k| { let (v, away) = level((k as f32 + 0.5) * step); ((v / top).clamp(0.0, 1.0).powf(1.15), away) }).collect();
    // (a kick: the bass over its own recent level, pushing the whole ring out for a moment)
    let bass = b[..3.min(BANDS)].iter().sum::<f32>() / 3.0;
    let punch = {
        let mut slow = RING_BASS.lock().unwrap();
        let p = ((bass - slow[slot]) / top).clamp(0.0, 1.0);
        slow[slot] += (bass - slow[slot]) * 0.08;
        p
    };
    // (eased, frame to frame: up at once, down quickly but not in a jump)
    let reach: Vec<f32> = {
        let mut all = RING_EASE.lock().unwrap();
        let e = &mut all[slot];
        if e.len() != n { *e = vec![0.0; n]; }
        for (k, (v, _)) in raw.iter().enumerate() {
            let v = (v + punch * 0.45).min(1.0);
            e[k] += (v - e[k]) * if v > e[k] { 0.75 } else { 0.3 };
        }
        e.clone()
    };
    let pt = |th: f32, r: f32| (500.0 - th.sin() * r, 500.0 + th.cos() * r);
    let pts: Vec<(f32, f32)> = (0..n).map(|k| pt((k as f32 + 0.5) * step, inner + depth * reach[k])).collect();
    let p = |k: isize| pts[k.rem_euclid(n as isize) as usize];
    (0..n)
        .map(|k| {
            let k = k as isize;
            let (p0, p1, p2, p3) = (p(k - 1), p(k), p(k + 1), p(k + 2));
            // (the curve from this point to the next, its handles from the points either side: Catmull-Rom, as Bézier)
            let c1 = (p1.0 + (p2.0 - p0.0) / 6.0, p1.1 + (p2.1 - p0.1) / 6.0);
            let c2 = (p2.0 - (p3.0 - p1.0) / 6.0, p2.1 - (p3.1 - p1.1) / 6.0);
            // (and a little on, into the next slice, at its edge: drawn over by it, so no seam and no notch)
            let th1 = (k as f32 + 0.5) * step;
            let th2 = (k as f32 + 1.5) * step + step * 0.12;
            let on = pt(th2, inner + depth * reach[((k + 1) as usize) % n]);
            // (a band, from the circle the cover sits in: hollow, so it's a halo with no cover too)
            let (i1, i2) = (pt(th1, inner - 2.0), pt(th2, inner - 2.0));
            let d = format!("M {:.1} {:.1} L {:.1} {:.1} C {:.1} {:.1} {:.1} {:.1} {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1} Z", i1.0, i1.1, p1.0, p1.1, c1.0, c1.1, c2.0, c2.1, p2.0, p2.1, on.0, on.1, i2.0, i2.1);
            (d, raw[k as usize].1)
        })
        .collect()
}

/// (the Ring's: its loudest lately, and each slice's reach as it's eased)
static RING_TOP: Mutex<f32> = Mutex::new(0.2);
/// (each Ring's bass, as it's been lately: a kick is the bass over it)
static RING_BASS: Mutex<[f32; 2]> = Mutex::new([0.0, 0.0]);
/// (the cover's Ring, and the player's, each eased on its own)
static RING_EASE: Mutex<[Vec<f32>; 2]> = Mutex::new([Vec::new(), Vec::new()]);

/// The Ring round Now Playing's cover (and full screen's): from a circle just clear of the cover's corners
pub fn ring_pieces(b: &[f32; BANDS]) -> Vec<(String, f32)> {
    ring_shape(b, RING_COVER * 500.0 * std::f32::consts::SQRT_2 + 10.0, 0)
}

/// The Ring round the player's cover (smaller for it, 0.7 of its place): its box the place and 10 px each
/// side, from a circle just clear of the cover's corners
pub fn ring_small(b: &[f32; BANDS]) -> Vec<(String, f32)> {
    ring_shape(b, 345.0, 1)
}

/// The Ring's cover: this much of the box's width, in its middle (the spikes round it the rest)
pub const RING_COVER: f32 = 0.46;

/// The Ring's spikes, each its own slice and step of the gradient (half each way round: a band each)
pub const RING_PIECES: usize = 64;

#[cfg(test)]
mod tests {
    #[test]
    fn a_tone_lights_its_own_band() {
        for hz in [60.0f32, 1000.0, 8000.0] {
            let mut t = super::tap(48_000);
            let frames: Vec<f32> = (0..48_000).flat_map(|i| { let v = 0.3 * (std::f32::consts::TAU * hz * i as f32 / 48_000.0).sin(); [v, v] }).collect();
            t(&frames);
            *super::SHOWN.lock().unwrap() = [0.0; super::BANDS];
            let b = super::bands();
            let top = b.iter().enumerate().max_by(|x, y| x.1.total_cmp(y.1)).unwrap();
            println!("{hz} Hz: loudest band {} at {:.2}; bands {:?}", top.0, top.1, b.iter().map(|v| (v * 10.0).round() / 10.0).collect::<Vec<_>>());
        }
    }
}
