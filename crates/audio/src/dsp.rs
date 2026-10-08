// The sound chain, as the web app has it (dist/js/audio.js), so the two sound the same:
//
//   input -> match (each track's gain) -> 10-band EQ -> limiter -> volume
//
// - EQ: biquads with WebAudio's formulas (lowshelf at 31 Hz, peaking Q 1.1, highshelf at 16 kHz).
// - Limiter: WebAudio's DynamicsCompressor as the app sets it (threshold -1.5 dB, hard knee, ratio
//   20, attack 2 ms, release 120 ms), with its 6 ms look-ahead and its automatic make-up gain.
// - Gains move smoothly, like setTargetAtTime: the volume over 20 ms, the EQ over 50 ms, a track's
//   loudness gain over 400 ms (or at once when a track starts).
//
// Everything is interleaved stereo f32 at the output's rate.

pub const EQ_FREQS: [f32; 10] = [31.0, 62.0, 125.0, 250.0, 500.0, 1000.0, 2000.0, 4000.0, 8000.0, 16000.0];
const EQ_Q: f32 = 1.1;

/// A value that eases towards its target like WebAudio's setTargetAtTime (time constant `tau`)
#[derive(Clone, Copy, Debug)]
pub struct Smoothed {
    pub value: f32,
    pub target: f32,
    coef: f32,
}

impl Smoothed {
    pub fn new(value: f32, tau_s: f32, rate: f32) -> Self {
        Self { value, target: value, coef: (-1.0 / (tau_s * rate)).exp() }
    }
    pub fn set(&mut self, v: f32) {
        self.target = v;
    }
    pub fn jump(&mut self, v: f32) {
        self.value = v;
        self.target = v;
    }
    #[inline]
    pub fn step(&mut self) -> f32 {
        self.value = self.target + (self.value - self.target) * self.coef;
        self.value
    }
    pub fn settled(&self) -> bool {
        (self.value - self.target).abs() < 1e-5
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Kind {
    LowShelf,
    Peaking,
    HighShelf,
}

/// One biquad (direct form I), stereo
#[derive(Clone, Debug)]
pub struct Biquad {
    kind: Kind,
    freq: f32,
    b: [f32; 3],
    a: [f32; 2],
    x: [[f32; 2]; 2],
    y: [[f32; 2]; 2],
    gain_db: f32,
}

impl Biquad {
    pub fn new(kind: Kind, freq: f32, rate: f32) -> Self {
        let mut f = Self { kind, freq, b: [1.0, 0.0, 0.0], a: [0.0, 0.0], x: [[0.0; 2]; 2], y: [[0.0; 2]; 2], gain_db: f32::NAN };
        f.set_gain(0.0, rate);
        f
    }

    /// The coefficients for a gain, from the WebAudio spec's formulas (the Audio EQ Cookbook; shelves with S = 1)
    pub fn set_gain(&mut self, gain_db: f32, rate: f32) {
        if gain_db == self.gain_db {
            return;
        }
        self.gain_db = gain_db;
        let a = 10f32.powf(gain_db / 40.0);
        let w0 = 2.0 * std::f32::consts::PI * (self.freq / rate).min(0.4999);
        let (s, c) = w0.sin_cos();
        let (b0, b1, b2, a0, a1, a2) = match self.kind {
            Kind::Peaking => {
                let alpha = s / (2.0 * EQ_Q);
                (1.0 + alpha * a, -2.0 * c, 1.0 - alpha * a, 1.0 + alpha / a, -2.0 * c, 1.0 - alpha / a)
            }
            Kind::LowShelf => {
                let alpha = s / 2.0 * 2f32.sqrt();
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) - (a - 1.0) * c + k),
                    2.0 * a * ((a - 1.0) - (a + 1.0) * c),
                    a * ((a + 1.0) - (a - 1.0) * c - k),
                    (a + 1.0) + (a - 1.0) * c + k,
                    -2.0 * ((a - 1.0) + (a + 1.0) * c),
                    (a + 1.0) + (a - 1.0) * c - k,
                )
            }
            Kind::HighShelf => {
                let alpha = s / 2.0 * 2f32.sqrt();
                let k = 2.0 * a.sqrt() * alpha;
                (
                    a * ((a + 1.0) + (a - 1.0) * c + k),
                    -2.0 * a * ((a - 1.0) + (a + 1.0) * c),
                    a * ((a + 1.0) + (a - 1.0) * c - k),
                    (a + 1.0) - (a - 1.0) * c + k,
                    2.0 * ((a - 1.0) - (a + 1.0) * c),
                    (a + 1.0) - (a - 1.0) * c - k,
                )
            }
        };
        self.b = [b0 / a0, b1 / a0, b2 / a0];
        self.a = [a1 / a0, a2 / a0];
    }

    pub fn is_flat(&self) -> bool {
        self.gain_db.abs() < 1e-3
    }

    #[inline]
    pub fn process(&mut self, ch: usize, x0: f32) -> f32 {
        let [x1, x2] = self.x[ch];
        let [y1, y2] = self.y[ch];
        let y0 = self.b[0] * x0 + self.b[1] * x1 + self.b[2] * x2 - self.a[0] * y1 - self.a[1] * y2;
        self.x[ch] = [x0, x1];
        self.y[ch] = [y0, y1];
        y0
    }

    pub fn reset(&mut self) {
        self.x = [[0.0; 2]; 2];
        self.y = [[0.0; 2]; 2];
    }

    /// The filter's gain at a frequency, in dB (for tests)
    pub fn response_db(&self, freq: f32, rate: f32) -> f32 {
        let w = 2.0 * std::f64::consts::PI * freq as f64 / rate as f64;
        let (b, a) = (self.b.map(|v| v as f64), self.a.map(|v| v as f64));
        let z1 = (w.cos(), -w.sin());
        let z2 = ((2.0 * w).cos(), -(2.0 * w).sin());
        let num = (b[0] + b[1] * z1.0 + b[2] * z2.0, b[1] * z1.1 + b[2] * z2.1);
        let den = (1.0 + a[0] * z1.0 + a[1] * z2.0, a[0] * z1.1 + a[1] * z2.1);
        let mag = ((num.0 * num.0 + num.1 * num.1) / (den.0 * den.0 + den.1 * den.1)).sqrt();
        (20.0 * mag.log10()) as f32
    }
}

/// The 10-band equalizer
pub struct Eq {
    bands: Vec<Biquad>,
    gains: Vec<Smoothed>,
    rate: f32,
}

impl Eq {
    pub fn new(rate: f32) -> Self {
        let n = EQ_FREQS.len();
        let bands = EQ_FREQS
            .iter()
            .enumerate()
            .map(|(i, &f)| Biquad::new(if i == 0 { Kind::LowShelf } else if i == n - 1 { Kind::HighShelf } else { Kind::Peaking }, f, rate))
            .collect();
        Self { bands, gains: vec![Smoothed::new(0.0, 0.05, rate / 64.0); n], rate }
    }

    /// Band gains in dB (-12..12); all 0 when the equalizer is off
    pub fn set(&mut self, gains: &[f32]) {
        for (i, g) in self.gains.iter_mut().enumerate() {
            g.set(gains.get(i).copied().unwrap_or(0.0).clamp(-24.0, 24.0));
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        // coefficients follow the smoothed gains every 64 frames
        for block in buf.chunks_mut(128) {
            for (band, g) in self.bands.iter_mut().zip(self.gains.iter_mut()) {
                let v = g.step();
                band.set_gain(if v.abs() < 0.01 && g.target == 0.0 { 0.0 } else { v }, self.rate);
            }
            for band in self.bands.iter_mut() {
                if band.is_flat() {
                    continue;
                }
                for fr in block.as_chunks_mut::<2>().0.iter_mut() {
                    fr[0] = band.process(0, fr[0]);
                    fr[1] = band.process(1, fr[1]);
                }
            }
        }
    }

    pub fn reset(&mut self) {
        self.bands.iter_mut().for_each(Biquad::reset);
    }
}

/// WebAudio's DynamicsCompressor with the app's settings, as a limiter
pub struct Limiter {
    threshold_db: f32,
    ratio: f32,
    attack: f32,
    release: f32,
    makeup: f32,
    reduction_db: f32,
    /// the detected level: rises at once, falls back over the release time
    env: f32,
    delay: Vec<f32>,
    pos: usize,
}

const LOOKAHEAD_S: f32 = 0.006;

impl Limiter {
    pub fn new(rate: f32) -> Self {
        let (threshold_db, ratio) = (-1.5f32, 20.0f32);
        // WebAudio's make-up gain: (1 / the curve's gain at 0 dBFS) ^ 0.6
        let full_range_db = threshold_db + (0.0 - threshold_db) / ratio;
        let makeup = 10f32.powf((-full_range_db * 0.6) / 20.0);
        let n = ((LOOKAHEAD_S * rate) as usize).max(1) * 2;
        Self {
            threshold_db,
            ratio,
            attack: (-1.0 / (0.002 * rate)).exp(),
            release: (-1.0 / (0.12 * rate)).exp(),
            makeup,
            reduction_db: 0.0,
            env: 0.0,
            delay: vec![0.0; n],
            pos: 0,
        }
    }

    pub fn makeup_db(&self) -> f32 {
        20.0 * self.makeup.log10()
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for fr in buf.as_chunks_mut::<2>().0.iter_mut() {
            let peak = fr[0].abs().max(fr[1].abs());
            self.env = if peak > self.env { peak } else { peak + (self.env - peak) * self.release };
            let level_db = if self.env > 1e-9 { 20.0 * self.env.log10() } else { -180.0 };
            let over = level_db - self.threshold_db;
            let want = if over > 0.0 { -(over - over / self.ratio) } else { 0.0 };
            let coef = if want < self.reduction_db { self.attack } else { self.release };
            self.reduction_db = want + (self.reduction_db - want) * coef;
            let g = 10f32.powf(self.reduction_db / 20.0) * self.makeup;
            // the signal comes out 6 ms late, so the reduction is already there when a peak arrives
            let (l, r) = (self.delay[self.pos], self.delay[self.pos + 1]);
            self.delay[self.pos] = fr[0];
            self.delay[self.pos + 1] = fr[1];
            self.pos = (self.pos + 2) % self.delay.len();
            fr[0] = l * g;
            fr[1] = r * g;
        }
    }

    pub fn reset(&mut self) {
        self.reduction_db = 0.0;
        self.env = 0.0;
        self.delay.iter_mut().for_each(|v| *v = 0.0);
        self.pos = 0;
    }
}

/// The whole chain, before the volume (which the output applies, so it reacts at once)
pub struct Chain {
    pub matched: Smoothed,
    pub eq: Eq,
    pub limiter: Limiter,
}

impl Chain {
    pub fn new(rate: f32) -> Self {
        Self { matched: Smoothed::new(1.0, 0.4, rate), eq: Eq::new(rate), limiter: Limiter::new(rate) }
    }

    /// A track's loudness gain in dB: at once when it starts, eased in when a measurement arrives later
    pub fn set_match_db(&mut self, db: f32, at_once: bool) {
        let g = 10f32.powf(db / 20.0);
        if at_once {
            self.matched.jump(g);
        } else {
            self.matched.set(g);
        }
    }

    pub fn process(&mut self, buf: &mut [f32]) {
        for fr in buf.as_chunks_mut::<2>().0.iter_mut() {
            let g = self.matched.step();
            fr[0] *= g;
            fr[1] *= g;
        }
        self.eq.process(buf);
        self.limiter.process(buf);
    }

    pub fn reset(&mut self) {
        self.eq.reset();
        self.limiter.reset();
    }
}

/// How loud a block is, before any gain: (rms dBFS, peak dBFS), for skipping silence
pub fn level(buf: &[f32]) -> (f32, f32) {
    if buf.is_empty() {
        return (-180.0, -180.0);
    }
    let (mut sq, mut peak) = (0.0f64, 0.0f32);
    for &v in buf {
        sq += (v * v) as f64;
        peak = peak.max(v.abs());
    }
    let db = |x: f32| 20.0 * x.max(1e-9).log10();
    (db((sq / buf.len() as f64).sqrt() as f32), db(peak))
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: f32 = 48000.0;

    fn sine(freq: f32, amp: f32, secs: f32) -> Vec<f32> {
        let n = (RATE * secs) as usize;
        (0..n).flat_map(|i| {
            let v = amp * (2.0 * std::f32::consts::PI * freq * i as f32 / RATE).sin();
            [v, v]
        }).collect()
    }

    fn peak_db(buf: &[f32]) -> f32 {
        level(buf).1
    }

    #[test]
    fn eq_bands_have_the_gain_asked_for() {
        // peaking: the full gain at its own frequency
        let mut b = Biquad::new(Kind::Peaking, 1000.0, RATE);
        b.set_gain(6.0, RATE);
        assert!((b.response_db(1000.0, RATE) - 6.0).abs() < 0.01);
        assert!(b.response_db(100.0, RATE).abs() < 0.3);
        // shelves: half the gain at the corner, the full gain well past it
        let mut lo = Biquad::new(Kind::LowShelf, 31.0, RATE);
        lo.set_gain(-12.0, RATE);
        assert!((lo.response_db(31.0, RATE) + 6.0).abs() < 0.05);
        assert!((lo.response_db(5.0, RATE) + 12.0).abs() < 0.5);
        assert!(lo.response_db(1000.0, RATE).abs() < 0.1);
        let mut hi = Biquad::new(Kind::HighShelf, 16000.0, RATE);
        hi.set_gain(6.0, RATE);
        assert!((hi.response_db(16000.0, RATE) - 3.0).abs() < 0.2);
        assert!(hi.response_db(1000.0, RATE).abs() < 0.1);
    }

    #[test]
    fn eq_changes_the_sound_and_flat_leaves_it() {
        let mut eq = Eq::new(RATE);
        let mut flat = sine(1000.0, 0.5, 0.2);
        let orig = flat.clone();
        eq.process(&mut flat);
        assert_eq!(flat, orig, "flat is passed through untouched");
        let mut eq = Eq::new(RATE);
        eq.set(&[0.0, 0.0, 0.0, 0.0, 0.0, 6.0, 0.0, 0.0, 0.0, 0.0]);
        let mut x = sine(1000.0, 0.25, 1.0);
        eq.process(&mut x);
        let tail = &x[x.len() / 2..];
        // +6 dB on the 1 kHz band (its neighbours add a little)
        assert!((peak_db(tail) - (20.0 * 0.25f32.log10() + 6.0)).abs() < 1.0, "{}", peak_db(tail));
    }

    #[test]
    fn limiter_matches_webaudio() {
        let lim = Limiter::new(RATE);
        // the make-up gain WebAudio adds with these settings
        assert!((lim.makeup_db() - 0.855).abs() < 0.01, "{}", lim.makeup_db());
        // quiet: passed with the make-up gain only
        let mut l = Limiter::new(RATE);
        let mut q = sine(440.0, 0.1, 0.5);
        l.process(&mut q);
        assert!((peak_db(&q[q.len() / 2..]) - (-20.0 + 0.855)).abs() < 0.05);
        // full scale: held down near the threshold (+ make-up), well under 0 dBFS
        let mut l = Limiter::new(RATE);
        let mut f = sine(440.0, 1.0, 1.0);
        l.process(&mut f);
        let p = peak_db(&f[f.len() / 2..]);
        assert!(p < -0.3 && p > -1.5, "{p}");
        // a sudden peak never gets through at full height: the look-ahead catches it
        let mut l = Limiter::new(RATE);
        let mut burst = sine(440.0, 0.05, 0.3);
        burst.extend(sine(440.0, 1.0, 0.1));
        l.process(&mut burst);
        assert!(peak_db(&burst) < 0.0, "{}", peak_db(&burst));
    }

    #[test]
    fn match_gain_jumps_at_a_track_start_and_eases_later() {
        let mut c = Chain::new(RATE);
        c.set_match_db(-6.0, true);
        assert!((c.matched.value - 0.501).abs() < 0.01);
        c.set_match_db(0.0, false);
        let mut x = vec![0.0; 2 * 4800];
        c.process(&mut x);
        // 100 ms into a 400 ms ease: about a fifth of the way
        let done = (c.matched.value - 0.501) / (1.0 - 0.501);
        assert!(done > 0.15 && done < 0.3, "{done}");
    }
}
