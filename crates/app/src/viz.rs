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

/// The Ring: a spike for each band standing out in a circle round the cover (as Proximity's round its logo),
/// the bass at the foot, the highs at the top, the left half and the right mirrored; between two spikes it
/// falls back toward the circle. As path commands in a 1000-unit square (the box it's drawn in), in slices,
/// each its own step of the accent's gradient (the software renderer fills a path with one colour): each
/// slice one spike, from the middle, running a little into the next so no seam shows. `inner`: the circle
/// it stands out from, in those units
fn ring_shape(b: &[f32; BANDS], inner: f32) -> Vec<(String, f32)> {
    const OUT: f32 = 494.0;
    let depth = OUT - inner;
    // (a band's level at an angle from straight down, round to straight up: 0 the bass, 1 the highs)
    let at = |th: f32| -> (f32, f32) {
        let th = th.rem_euclid(std::f32::consts::TAU);
        let away = if th <= std::f32::consts::PI { th } else { std::f32::consts::TAU - th } / std::f32::consts::PI;
        let x = away * (BANDS - 1) as f32;
        let k = (x.round() as usize).min(BANDS - 1);
        (b[k], away)
    };
    // (each band against the others: what they all have taken off, so the loud ones stand out from the rest
    // however loud the song is, and the louder sharper still)
    let mean = b.iter().sum::<f32>() / BANDS as f32;
    let floor = mean * 0.75;
    let reach = |v: f32| ((v - floor) / (1.0 - floor).max(0.05)).clamp(0.0, 1.0).powf(1.25);
    let pt = |th: f32, r: f32| (500.0 - th.sin() * r, 500.0 + th.cos() * r);
    let step = std::f32::consts::TAU / RING_PIECES as f32;
    // each spike's tip, and the dip before it (its own level and the one before's, the lower, mostly gone)
    let tips: Vec<((f32, f32), f32)> = (0..RING_PIECES).map(|k| { let th = (k as f32 + 0.5) * step; let (v, away) = at(th); (pt(th, inner + depth * reach(v)), away) }).collect();
    let dips: Vec<(f32, f32)> = (0..RING_PIECES).map(|k| {
        let th = k as f32 * step;
        let (a, _) = at(th - step / 2.0);
        let (c, _) = at(th + step / 2.0);
        pt(th, inner + depth * reach(a.min(c)) * 0.15)
    }).collect();
    (0..RING_PIECES)
        .map(|k| {
            let (tip, away) = tips[k];
            let (d0, d1) = (dips[k], dips[(k + 1) % RING_PIECES]);
            let next = tips[(k + 1) % RING_PIECES].0;
            // (into the next a third of the way up its spike: drawn over by it)
            let over = (d1.0 + (next.0 - d1.0) / 3.0, d1.1 + (next.1 - d1.1) / 3.0);
            let d = format!("M 500 500 L {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1} L {:.1} {:.1} Z", d0.0, d0.1, tip.0, tip.1, d1.0, d1.1, over.0, over.1);
            (d, away)
        })
        .collect()
}

/// The Ring round Now Playing's cover (and full screen's): from a circle just clear of the cover's corners
pub fn ring_pieces(b: &[f32; BANDS]) -> Vec<(String, f32)> {
    ring_shape(b, RING_COVER * 500.0 * std::f32::consts::SQRT_2 + 10.0)
}

/// The Ring round the player's cover (smaller for it, 0.7 of its place): its box the place and 10 px each
/// side, from a circle just clear of the cover's corners
pub fn ring_small(b: &[f32; BANDS]) -> Vec<(String, f32)> {
    ring_shape(b, 345.0)
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
