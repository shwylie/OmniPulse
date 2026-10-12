// The theme, as in the web app (js/appearance.js): a step from light to dark (no step in between,
// where text would be hard to read), a tint and its strength, and the accent colours (or the album
// art's). The colours are worked out the same way, and checked the same way: text and muted text are
// moved lighter or darker until they read at 7:1 and 4.5:1 on every surface.

use serde_json::Value;

/// name, light?, background, panel, raised, lines, text, muted (lightness, %)
pub type Step = (&'static str, bool, f32, f32, f32, f32, f32, f32);
pub const STEPS: [Step; 5] = [
    ("Bright", true, 92.0, 100.0, 95.5, 88.5, 11.0, 42.0),
    ("Soft", true, 87.5, 96.5, 92.5, 84.0, 12.0, 40.0),
    ("Dim", false, 13.0, 17.0, 21.5, 27.5, 94.0, 68.0),
    ("Dark", false, 4.5, 8.5, 12.5, 18.0, 94.0, 64.0),
    ("Black", false, 0.0, 4.5, 8.0, 14.0, 93.0, 62.0),
];
pub const STRENGTHS: [&str; 4] = ["Off", "Subtle", "Medium", "Strong"];
const SAT_DARK: [f32; 4] = [0.0, 0.2, 0.36, 0.55];
const SAT_LIGHT: [f32; 4] = [0.0, 0.14, 0.26, 0.4];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    #[cfg(test)]
    pub fn hex(&self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.0, self.1, self.2)
    }
    pub fn parse(s: &str) -> Option<Rgb> {
        let s = s.trim().trim_start_matches('#');
        let n = u32::from_str_radix(s.get(..6)?, 16).ok()?;
        Some(Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8))
    }
    pub fn slint(&self) -> slint::Color {
        slint::Color::from_rgb_u8(self.0, self.1, self.2)
    }
}

/// hue in degrees, saturation 0..1, lightness in %
pub fn hsl(h: f32, s: f32, l: f32) -> Rgb {
    let (h, s, l) = (h.rem_euclid(360.0) / 360.0, s.clamp(0.0, 1.0), (l / 100.0).clamp(0.0, 1.0));
    if s == 0.0 {
        let v = (l * 255.0).round() as u8;
        return Rgb(v, v, v);
    }
    let q = if l < 0.5 { l * (1.0 + s) } else { l + s - l * s };
    let p = 2.0 * l - q;
    let f = |mut t: f32| {
        t = t.rem_euclid(1.0);
        let v = if t < 1.0 / 6.0 { p + (q - p) * 6.0 * t } else if t < 0.5 { q } else if t < 2.0 / 3.0 { p + (q - p) * (2.0 / 3.0 - t) * 6.0 } else { p };
        (v * 255.0).round() as u8
    };
    Rgb(f(h + 1.0 / 3.0), f(h), f(h - 1.0 / 3.0))
}

fn luminance(c: Rgb) -> f32 {
    let ch = |v: u8| {
        let v = v as f32 / 255.0;
        if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
    };
    0.2126 * ch(c.0) + 0.7152 * ch(c.1) + 0.0722 * ch(c.2)
}

pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgb,
    pub panel: Rgb,
    pub elevated: Rgb,
    pub line: Rgb,
    pub text: Rgb,
    pub muted: Rgb,
    pub light: bool,
}

pub fn palette(step: usize, hue: f32, strength: usize) -> Palette {
    let (_, light, bg, panel, raised, line, text, muted) = STEPS[step.min(STEPS.len() - 1)];
    let sat = if light { SAT_LIGHT } else { SAT_DARK }[strength.min(3)];
    let mut p = Palette {
        bg: hsl(hue, sat, bg),
        panel: hsl(hue, sat * if light { 0.6 } else { 1.0 }, panel),
        elevated: hsl(hue, sat, raised),
        line: hsl(hue, sat * 0.9, line),
        text: Rgb(0, 0, 0),
        muted: Rgb(0, 0, 0),
        light,
    };
    let fit = |mut l: f32, s: f32, need: f32, p: &Palette| {
        let mut c = hsl(hue, s, l);
        for _ in 0..60 {
            if [p.bg, p.panel, p.elevated].iter().all(|&x| contrast(c, x) >= need) {
                break;
            }
            l += if light { -1.0 } else { 1.0 };
            c = hsl(hue, s, l);
        }
        c
    };
    p.text = fit(text, sat * 0.3, 7.0, &p);
    p.muted = fit(muted, sat * 0.55, 4.5, &p);
    p
}

/// Readable text on an accent colour: dark or light
pub fn on_accent(c: Rgb) -> Rgb {
    if 0.299 * c.0 as f32 + 0.587 * c.1 as f32 + 0.114 * c.2 as f32 > 150.0 { Rgb(0x11, 0x13, 0x18) } else { Rgb(255, 255, 255) }
}

/// The theme's settings, from the app's prefs (the web app's names); the old named themes too
pub fn from_prefs(prefs: &Value) -> (usize, f32, usize) {
    if let Some(step) = prefs["themeStep"].as_f64() {
        return (step.round().clamp(0.0, 4.0) as usize, prefs["tintHue"].as_f64().unwrap_or(0.0) as f32, prefs["tintStrength"].as_f64().unwrap_or(0.0).clamp(0.0, 3.0) as usize);
    }
    match prefs["theme"].as_str().unwrap_or("dark") {
        "midnight" => (3, 230.0, 2),
        "graphite" => (3, 0.0, 0),
        "forest" => (3, 145.0, 2),
        "rose" => (3, 340.0, 2),
        "light" => (0, 240.0, 1),
        "paper" => (1, 42.0, 1),
        _ => (3, 225.0, 1),
    }
}

/// The album art's colours tuned for the theme (as the web app does): bright enough on dark themes,
/// deep enough on light ones
/// tune_art's colour a shade darker (lighter on a light theme): the second accent from a cover with
/// only one colour, as the web app's
pub fn tune_art_shade(c: Rgb, light: bool) -> Rgb {
    let t = tune_art(c, light);
    let (r, g, b) = (t.0 as f32 / 255.0, t.1 as f32 / 255.0, t.2 as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d == 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    let h = if d == 0.0 { 0.0 } else if max == r { 60.0 * ((g - b) / d).rem_euclid(6.0) } else if max == g { 60.0 * ((b - r) / d + 2.0) } else { 60.0 * ((r - g) / d + 4.0) };
    let l = if light { (l + 0.1).min(0.55) } else { (l - 0.13).max(0.45) };
    hsl(h, s, l * 100.0)
}

pub fn tune_art(c: Rgb, light: bool) -> Rgb {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d == 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()) };
    let h = if d == 0.0 { 0.0 } else if max == r { 60.0 * ((g - b) / d).rem_euclid(6.0) } else if max == g { 60.0 * ((b - r) / d + 2.0) } else { 60.0 * ((r - g) / d + 4.0) };
    let l = if light { (l * 100.0).clamp(30.0, 45.0) } else { (l * 100.0).clamp(58.0, 72.0) };
    hsl(h, s.max(0.5), l)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_theme_reads_well() {
        // as in the web app: 5 steps, 4 strengths, every 30 degrees of hue
        for step in 0..5 {
            for k in 0..4 {
                for h in (0..360).step_by(30) {
                    let p = palette(step, h as f32, k);
                    for s in [p.bg, p.panel, p.elevated] {
                        assert!(contrast(p.text, s) >= 7.0, "text {step} {h} {k}");
                        assert!(contrast(p.muted, s) >= 4.5, "muted {step} {h} {k}");
                    }
                }
            }
        }
        // and the old default comes out as it was: a near-black blue
        let p = palette(3, 225.0, 1);
        assert_eq!(p.bg.hex(), "#090a0e");
    }
}
