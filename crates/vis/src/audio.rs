// What the scenes hear: the sound's levels (bass, mids, highs), its hits (kicks, snares, hats, each an
// envelope that jumps to 1 on a hit and dies away), whether it's building (the highs rising and the low end
// gone, as before a drop) or dropping (the low end back hard after a build), how wide it is in stereo, its
// spectrum, and its left and right waves. Worked out from the last stretch of what's played, each frame.

use realfft::{RealFftPlanner, RealToComplex};
use std::sync::Arc;

pub const BANDS: usize = 64;
pub const WAVE: usize = 256;
/// how much of the sound each look takes (at 48 kHz, 43 ms)
pub const WINDOW: usize = 2048;

#[derive(Clone, Debug)]
pub struct Audio {
    pub bass: f32,
    pub mid: f32,
    pub high: f32,
    pub level: f32,
    pub kick: f32,
    pub snare: f32,
    pub hat: f32,
    /// kicks heard so far (a scene can step on each)
    pub beat: u32,
    /// any hit heard so far (kicks, snares)
    pub hits: u32,
    pub build: f32,
    pub drop: f32,
    pub width: f32,
    pub spectrum: [f32; BANDS],
    pub left: [f32; WAVE],
    pub right: [f32; WAVE],
}

impl Default for Audio {
    fn default() -> Self {
        Audio { bass: 0.0, mid: 0.0, high: 0.0, level: 0.0, kick: 0.0, snare: 0.0, hat: 0.0, beat: 0, hits: 0, build: 0.0, drop: 0.0, width: 0.0, spectrum: [0.0; BANDS], left: [0.0; WAVE], right: [0.0; WAVE] }
    }
}

impl Audio {
    /// The spectrum at x (0: the bass end, 1: the top), between its bands
    pub fn spectrum_at(&self, x: f32) -> f32 {
        let p = x.clamp(0.0, 1.0) * (BANDS - 1) as f32;
        let k = p.floor() as usize;
        let t = p - k as f32;
        self.spectrum[k] * (1.0 - t) + self.spectrum[(k + 1).min(BANDS - 1)] * t
    }

    pub fn wave_at(&self, i: usize) -> (f32, f32) { (self.left[i.min(WAVE - 1)], self.right[i.min(WAVE - 1)]) }
}

/// A kind of hit: the band it's heard in, how far over its recent average it must jump, how soon another
/// may come, and how fast its envelope dies away
struct Hit { lo: f32, hi: f32, over: f32, gap: f32, fade: f32, avg: f32, prev: f32, since: f32, env: f32 }

impl Hit {
    fn new(lo: f32, hi: f32, over: f32, gap: f32, fade: f32) -> Hit { Hit { lo, hi, over, gap, fade, avg: 0.0, prev: 0.0, since: 10.0, env: 0.0 } }

    /// This frame's energy in its band: whether it's a hit (and its envelope, as it is now)
    fn step(&mut self, e: f32, dt: f32, floor: f32) -> bool {
        self.since += dt;
        self.env *= (-dt / self.fade).exp();
        let hit = e > floor && e > self.avg * self.over && e > self.prev * 1.12 && self.since >= self.gap;
        if hit { self.since = 0.0; self.env = 1.0; }
        // (its average: over about half a second, so a loud passage raises the bar)
        let k = 1.0 - (-dt / 0.45).exp();
        self.avg += (e - self.avg) * k;
        self.prev = e;
        hit
    }
}

pub struct Analyser {
    rate: f32,
    fft: Arc<dyn RealToComplex<f32>>,
    window: Vec<f32>,
    kick: Hit,
    snare: Hit,
    snare_noise: Hit,
    hat: Hit,
    out: Audio,
    /// the low end and the highs: over two seconds, and over eight (a build is the highs rising over
    /// their long average while the low end's gone under its)
    low_short: f32,
    low_long: f32,
    high_short: f32,
    high_long: f32,
    built: f32,
    since_build: f32,
    /// the waves' loudest of late: risen to at once, let go slowly (so a quiet song's traces fill the view too)
    wave_top: f32,
}

fn ease(v: &mut f32, to: f32, dt: f32, up: f32, down: f32) {
    let tau = if to > *v { up } else { down };
    *v += (to - *v) * (1.0 - (-dt / tau).exp());
}

impl Analyser {
    pub fn new(rate: u32) -> Analyser {
        let fft = RealFftPlanner::<f32>::new().plan_fft_forward(WINDOW);
        let window = (0..WINDOW).map(|i| 0.5 - 0.5 * (std::f32::consts::TAU * i as f32 / WINDOW as f32).cos()).collect();
        Analyser {
            rate: rate.max(1) as f32,
            fft,
            window,
            kick: Hit::new(40.0, 120.0, 1.5, 0.14, 0.22),
            snare: Hit::new(150.0, 260.0, 1.45, 0.12, 0.18),
            snare_noise: Hit::new(2000.0, 6000.0, 1.5, 0.12, 0.18),
            hat: Hit::new(7000.0, 16000.0, 1.4, 0.06, 0.08),
            out: Audio::default(),
            low_short: 0.0,
            low_long: 0.0,
            high_short: 0.0,
            high_long: 0.0,
            built: 0.0,
            since_build: 100.0,
            wave_top: 0.05,
        }
    }

    /// The sound as it is now (its last WINDOW frames, left and right), `dt` seconds after the last look
    pub fn analyse(&mut self, left: &[f32], right: &[f32], dt: f32) -> &Audio {
        let dt = dt.clamp(1e-3, 0.25);
        let n = WINDOW.min(left.len()).min(right.len());
        let (l, r) = (&left[left.len() - n..], &right[right.len() - n..]);
        let mut input: Vec<f32> = (0..WINDOW).map(|i| if i < n { (l[i] + r[i]) * 0.5 * self.window[i] } else { 0.0 }).collect();
        let mut spec = self.fft.make_output_vec();
        let _ = self.fft.process(&mut input, &mut spec);
        let hz = self.rate / WINDOW as f32;
        let power = |lo: f32, hi: f32| -> f32 {
            let (a, b) = ((lo / hz) as usize, ((hi / hz) as usize).min(spec.len() - 1).max((lo / hz) as usize + 1));
            spec[a.min(spec.len() - 1)..b].iter().map(|c| c.norm_sqr()).sum::<f32>() / (WINDOW as f32 * WINDOW as f32 / 16.0)
        };
        // (a level: as dB, -60 to 0, read as 0 to 1)
        let level = |p: f32| ((10.0 * p.max(1e-9).log10() + 60.0) / 60.0).clamp(0.0, 1.0);

        // the spectrum: 64 bands from 30 Hz to 16 kHz, rising at once and falling slowly
        let o = &mut self.out;
        for b in 0..BANDS {
            let f0 = 30.0 * (16000.0f32 / 30.0).powf(b as f32 / BANDS as f32);
            let f1 = 30.0 * (16000.0f32 / 30.0).powf((b + 1) as f32 / BANDS as f32);
            let v = (level(power(f0, f1.max(f0 + hz))) + 0.08 * b as f32 / BANDS as f32).min(1.0);
            ease(&mut o.spectrum[b], v, dt, 0.02, 0.18);
        }
        let (bass, mid, high) = (level(power(30.0, 200.0)), level(power(200.0, 3000.0)), level(power(3000.0, 16000.0)));
        ease(&mut o.bass, bass, dt, 0.03, 0.2);
        ease(&mut o.mid, mid, dt, 0.03, 0.25);
        ease(&mut o.high, high, dt, 0.02, 0.2);
        ease(&mut o.level, (0.6 * bass + 0.3 * mid + 0.2 * high).min(1.0), dt, 0.03, 0.3);

        // the hits: each band's energy jumping over its recent average
        let floor = 1e-5;
        let (ek, es, en, eh) = (power(self.kick.lo, self.kick.hi), power(self.snare.lo, self.snare.hi), power(self.snare_noise.lo, self.snare_noise.hi), power(self.hat.lo, self.hat.hi));
        if self.kick.step(ek, dt, floor) { o.beat += 1; o.hits += 1; }
        let body = self.snare.step(es, dt, floor);
        let noise = self.snare_noise.step(en, dt, floor * 0.1);
        // (a snare: its crack (the noise) with, or without, its body; not a kick's own low thud)
        if noise && self.kick.since > 0.03 { o.hits += 1; self.snare.env = 1.0; } else if body && self.kick.since > 0.03 && en > self.snare_noise.avg { o.hits += 1; self.snare.env = 1.0; }
        self.hat.step(eh, dt, floor * 0.01);
        o.kick = self.kick.env;
        o.snare = self.snare.env;
        o.hat = self.hat.env;

        // building and dropping
        ease(&mut self.low_short, bass, dt, 1.0, 1.0);
        ease(&mut self.low_long, bass, dt, 8.0, 8.0);
        ease(&mut self.high_short, high, dt, 1.5, 1.5);
        ease(&mut self.high_long, high, dt, 8.0, 8.0);
        let rising = ((self.high_short - self.high_long) * 6.0).clamp(0.0, 1.0);
        let gone = ((self.low_long - self.low_short) * 4.0).clamp(0.0, 1.0);
        let build = (0.5 * rising + 0.7 * gone).min(1.0);
        ease(&mut o.build, build, dt, 0.8, 0.4);
        if o.build > 0.35 { self.built = self.built.max(o.build); self.since_build = 0.0; } else { self.since_build += dt; }
        o.drop *= (-dt / 1.2).exp();
        // (the drop: a kick, the low end back over its build-time level, within a few seconds of a build)
        if self.kick.since == 0.0 && self.built > 0.35 && self.since_build < 4.0 && bass > self.low_short + 0.08 {
            o.drop = 1.0;
            self.built = 0.0;
            o.build = 0.0;
            self.low_short = bass;
        }

        // stereo width: how much of it is in the sides (0: mono, 1: as wide as can be)
        let (mut m, mut sd) = (0.0f32, 0.0f32);
        for i in 0..n { let (a, b) = (l[i], r[i]); m += (a + b) * (a + b); sd += (a - b) * (a - b); }
        ease(&mut o.width, if m + sd > 1e-9 { (sd / (m + sd) * 2.0).min(1.0) } else { 0.0 }, dt, 0.1, 0.4);

        // the waves: from an upward crossing (so a steady tone stands still), every other frame
        let start = (n / 2..n.saturating_sub(WAVE * 2)).rev().find(|&i| i > 0 && l[i - 1] <= 0.0 && l[i] > 0.0).unwrap_or(n.saturating_sub(WAVE * 2));
        // (levelled: against their loudest of late, up to 8 times, so quiet music still draws big traces; silence
        // stays still)
        let peak = (0..WAVE).map(|k| { let i = (start + k * 2).min(n.saturating_sub(1)); l[i].abs().max(r[i].abs()) }).fold(0.0f32, f32::max);
        self.wave_top = if peak > self.wave_top { peak } else { (self.wave_top * (1.0 - 0.6 * dt)).max(peak).max(0.02) };
        let gain = (0.85 / self.wave_top).clamp(1.0, 8.0);
        for k in 0..WAVE {
            let i = (start + k * 2).min(n.saturating_sub(1));
            o.left[k] = (l[i] * gain).clamp(-1.0, 1.0);
            o.right[k] = (r[i] * gain).clamp(-1.0, 1.0);
        }
        &self.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48000;

    /// A made-up track, `secs` long: kicks (a falling 55 Hz thump) every `every` seconds, hats (noise, high)
    /// on the offbeats, a riser (rising noise, no kick) in [riser_from, riser_to]
    fn track(secs: f32, every: f32, riser: Option<(f32, f32)>) -> (Vec<f32>, Vec<f32>) {
        let n = (secs * RATE as f32) as usize;
        let mut seed = 1u32;
        let mut noise = || { seed ^= seed << 13; seed ^= seed >> 17; seed ^= seed << 5; (seed as f32 / u32::MAX as f32) * 2.0 - 1.0 };
        let mut prev = 0.0f32;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = i as f32 / RATE as f32;
            let in_riser = riser.map(|(a, b)| t >= a && t < b).unwrap_or(false);
            let w = noise();
            // (high noise: the difference of white noise, mostly its top)
            let hi = w - prev;
            prev = w;
            let mut v = 0.0;
            if !in_riser {
                let k = t % every;
                v += 0.8 * (std::f32::consts::TAU * 55.0 * k).sin() * (-k * 18.0).exp();
                let h = (t + every / 2.0) % every;
                v += 0.12 * hi * (-h * 60.0).exp();
            } else {
                let (a, b) = riser.unwrap();
                v += 0.25 * hi * ((t - a) / (b - a));
            }
            out.push(v);
        }
        (out.clone(), out)
    }

    fn run(l: &[f32], r: &[f32], fps: f32, mut each: impl FnMut(f32, &Audio)) {
        let mut a = Analyser::new(RATE);
        let step = (RATE as f32 / fps) as usize;
        let mut at = WINDOW;
        while at <= l.len() {
            let x = a.analyse(&l[at - WINDOW..at], &r[at - WINDOW..at], 1.0 / fps).clone();
            each(at as f32 / RATE as f32, &x);
            at += step;
        }
    }

    #[test]
    fn kicks_are_counted_and_hats_heard() {
        let (l, r) = track(8.0, 0.5, None);
        let mut last = Audio::default();
        let mut hat_peaks = 0;
        let (mut was, mut most_bass) = (0.0, 0.0f32);
        run(&l, &r, 60.0, |_, a| { if a.hat > 0.9 && was <= 0.9 { hat_peaks += 1; } was = a.hat; most_bass = most_bass.max(a.bass); last = a.clone(); });
        // (16 kicks in 8 s: the first may go unheard while its average is still nothing)
        assert!((14..=17).contains(&last.beat), "beats {}", last.beat);
        assert!(hat_peaks >= 10, "hats {hat_peaks}");
        assert!(last.width < 0.05, "a mono track's width {}", last.width);
        assert!(most_bass > 0.4, "bass {most_bass}");
    }

    #[test]
    fn a_build_then_the_drop() {
        // 10 s of the beat, 6 s of a riser with no kick, then the beat back
        let (l, r) = track(20.0, 0.5, Some((10.0, 16.0)));
        let (mut most_build, mut drop_at) = (0.0f32, None);
        run(&l, &r, 60.0, |t, a| {
            if (13.0..16.0).contains(&t) { most_build = most_build.max(a.build); }
            if a.drop > 0.95 && drop_at.is_none() { drop_at = Some(t); }
        });
        assert!(most_build > 0.5, "build {most_build}");
        let at = drop_at.expect("a drop");
        assert!((15.9..16.8).contains(&at), "dropped at {at}");
    }

    #[test]
    fn stereo_width() {
        let n = WINDOW * 4;
        let l: Vec<f32> = (0..n).map(|i| (i as f32 * 0.05).sin() * 0.5).collect();
        let r: Vec<f32> = l.iter().map(|v| -v).collect();
        let mut a = Analyser::new(RATE);
        let mut w = 0.0;
        for _ in 0..30 { w = a.analyse(&l, &r, 1.0 / 60.0).width; }
        assert!(w > 0.95, "width {w}");
    }

    #[test]
    fn quiet_music_still_draws_big_waves() {
        let n = WINDOW * 4;
        let l: Vec<f32> = (0..n).map(|i| (i as f32 * 0.05).sin() * 0.04).collect();
        let mut a = Analyser::new(RATE);
        let mut top = 0.0f32;
        for _ in 0..30 { top = a.analyse(&l, &l, 1.0 / 60.0).left.iter().fold(0.0f32, |m, v| m.max(v.abs())); }
        assert!(top > 0.25, "a quiet tone's waves reach {top}");
        // (silence stays still)
        let quiet = vec![0.0f32; n];
        let mut a = Analyser::new(RATE);
        let still = a.analyse(&quiet, &quiet, 1.0 / 60.0).left.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        assert!(still < 1e-6, "silence's waves reach {still}");
    }
}
