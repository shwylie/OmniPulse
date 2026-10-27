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

/// A colour's hue (degrees), saturation (0 to 1) and lightness (0 to 100), as hsl() takes them
pub fn to_hsl(c: Rgb) -> (f32, f32, f32) {
    let (r, g, b) = (c.0 as f32 / 255.0, c.1 as f32 / 255.0, c.2 as f32 / 255.0);
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let l = (max + min) / 2.0;
    if max == min { return (0.0, 0.0, l * 100.0); }
    let d = max - min;
    let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
    let h = if max == r { (g - b) / d + if g < b { 6.0 } else { 0.0 } } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    (h * 60.0, s, l * 100.0)
}

/// Tiles' own colours, from the theme's accent: six hues spread round it, as bright and as strong as it
/// (a little stronger if it's nearly grey), so cards side by side differ and still look like the theme
pub fn tints(accent: Rgb, light: bool) -> Vec<Rgb> {
    let (h, s, _) = to_hsl(accent);
    let s = s.clamp(0.45, 0.85);
    let l = if light { 46.0 } else { 64.0 };
    [0.0, 48.0, 105.0, 165.0, 225.0, 290.0].iter().map(|o| hsl(h + o, s, l)).collect()
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

/// A theme of its own (Settings > App & Look > Theme): its surfaces and accents set, not worked out from a
/// step and a tint, and how it looks besides its colours. "custom" is the step and tint as before.
pub struct Preset {
    pub id: &'static str,
    pub name: &'static str,
    pub light: bool,
    /// background, panel, raised, lines, text, muted text, accent, second accent
    pub c: [u32; 8],
    /// its headings' font ("": the app's own)
    pub font: &'static str,
    /// what's playing glows in the accent
    pub glow: bool,
    /// see-through panels over a soft glow from the accents (Aurora)
    pub aurora: bool,
    /// covers round, as records (Vinyl)
    pub round_art: bool,
}

pub const PRESETS: [Preset; 8] = [
    Preset { id: "neon", name: "Neon", light: false, c: [0x0b0716, 0x140c24, 0x1e1334, 0x2e1f4d, 0xf1ecff, 0xb3a6d9, 0x00e5ff, 0xff2bd6], font: "", glow: true, aurora: false, round_art: false },
    Preset { id: "vinyl", name: "Vinyl Lounge", light: false, c: [0x1c1410, 0x261b15, 0x33251c, 0x4a3828, 0xf6ead6, 0xc9b394, 0xe8a33d, 0xd2643a], font: "Georgia", glow: false, aurora: false, round_art: true },
    Preset { id: "paper", name: "Paper", light: true, c: [0xf4efe4, 0xfffdf8, 0xebe4d4, 0xd9d0bd, 0x1d1b17, 0x5a5346, 0xc8372d, 0x8a3a2f], font: "Georgia", glow: false, aurora: false, round_art: false },
    Preset { id: "studio", name: "Studio", light: false, c: [0x16181a, 0x1e2124, 0x272b2f, 0x363b40, 0xe3e6e8, 0xa2a9af, 0x46e07a, 0xf2b33d], font: "Consolas", glow: false, aurora: false, round_art: false },
    Preset { id: "phosphor", name: "Phosphor", light: false, c: [0x020703, 0x06120a, 0x0b1f12, 0x14391f, 0x8dffb2, 0x4fd47c, 0x39ff7a, 0xb6ff5e], font: "Consolas", glow: true, aurora: false, round_art: false },
    Preset { id: "aurora", name: "Aurora", light: false, c: [0x0d1b2a, 0x13263a, 0x1a3149, 0x29435e, 0xe6f4ff, 0xa3bcd3, 0x5ef0c8, 0xa58bff], font: "", glow: false, aurora: true, round_art: false },
    Preset { id: "sunset", name: "Sunset", light: true, c: [0xfff4ec, 0xffffff, 0xffe9dc, 0xf3d0bc, 0x2b1d1a, 0x6a4e46, 0xe0503a, 0x7c4ff0], font: "", glow: false, aurora: false, round_art: false },
    Preset { id: "contrast", name: "High Contrast", light: false, c: [0x000000, 0x000000, 0x161616, 0xffffff, 0xffffff, 0xe6e6e6, 0xffd400, 0xffffff], font: "", glow: false, aurora: false, round_art: false },
];

fn rgb(n: u32) -> Rgb { Rgb((n >> 16) as u8, (n >> 8) as u8, n as u8) }

impl Preset {
    pub fn palette(&self) -> Palette {
        Palette { bg: rgb(self.c[0]), panel: rgb(self.c[1]), elevated: rgb(self.c[2]), line: rgb(self.c[3]), text: rgb(self.c[4]), muted: rgb(self.c[5]), light: self.light }
    }
    pub fn accents(&self) -> (Rgb, Rgb) { (rgb(self.c[6]), rgb(self.c[7])) }
}

/// The theme picked: one of its own, or None for Custom (the step and tint). Nothing picked yet: Neon for a
/// new install, Custom for one that's set a step or an old named theme (it stays as it was)
pub fn preset(prefs: &Value) -> Option<&'static Preset> {
    let id = match prefs["themeName"].as_str() {
        Some(id) => id,
        None if prefs["themeStep"].is_number() || prefs["theme"].is_string() => "custom",
        None => "neon",
    };
    PRESETS.iter().find(|p| p.id == id)
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
        // the themes of their own read as well (and their accents stand out from where they're shown)
        for t in &PRESETS {
            let p = t.palette();
            for s in [p.bg, p.panel, p.elevated] {
                assert!(contrast(p.text, s) >= 7.0, "{} text {:.1}", t.id, contrast(p.text, s));
                assert!(contrast(p.muted, s) >= 4.5, "{} muted {:.1}", t.id, contrast(p.muted, s));
            }
            assert!(contrast(t.accents().0, p.panel) >= 3.0, "{} accent", t.id);
        }
    }

    #[test]
    fn neon_for_a_new_install_custom_for_one_set_up() {
        assert_eq!(preset(&serde_json::json!({})).map(|p| p.id), Some("neon"));
        assert!(preset(&serde_json::json!({ "themeStep": 3 })).is_none());
        assert!(preset(&serde_json::json!({ "theme": "forest" })).is_none());
        assert_eq!(preset(&serde_json::json!({ "themeStep": 3, "themeName": "paper" })).map(|p| p.id), Some("paper"));
        assert!(preset(&serde_json::json!({ "themeName": "custom" })).is_none());
    }
}
