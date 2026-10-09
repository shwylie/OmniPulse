// Settings > Sound (as the web app's js/audio.js and js/loudness.js): the equalizer and Match
// Loudness. Each track is measured over its whole length (crates/core/src/loudness.rs) and played at
// the gain that brings it to the chosen target; until its measurement arrives, a typical gain is
// used. Spotify's tracks aren't touched (librespot plays them at their own level).

use crate::track::{Src, Track};
use serde_json::Value;

pub const EQ_PRESETS: [(&str, [f32; 10]); 5] = [
    ("Flat", [0.0; 10]),
    ("Bass Boost", [6.0, 5.0, 4.0, 2.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]),
    ("Vocals", [-2.0, -2.0, -1.0, 1.0, 3.0, 3.0, 2.0, 1.0, 0.0, -1.0]),
    ("Treble Boost", [0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 4.0, 5.0, 6.0]),
    ("Loudness (Smile)", [5.0, 4.0, 2.0, 0.0, -1.0, -1.0, 0.0, 2.0, 4.0, 5.0]),
];

/// The bands as set (whether or not it's on)
pub fn eq_bands(prefs: &Value) -> [f32; 10] {
    let mut g = [0.0; 10];
    if let Some(a) = prefs["eq"].as_array().filter(|a| a.len() == 10) {
        for (i, v) in a.iter().enumerate() {
            g[i] = (v.as_f64().unwrap_or(0.0) as f32).clamp(-12.0, 12.0);
        }
    }
    g
}

/// What the engine plays with: the bands, or flat while it's off
pub fn eq_heard(prefs: &Value) -> [f32; 10] {
    if prefs["eqOn"].as_bool() == Some(true) { eq_bands(prefs) } else { [0.0; 10] }
}

pub const LOUD_STEPS: [(&str, &str); 4] = [("off", "Off"), ("quiet", "Quiet"), ("normal", "Normal"), ("loud", "Loud")];
const LOUD_MAX_BOOST: f64 = 9.0;
const LOUD_TYPICAL_LUFS: f64 = -10.0;

/// "off", "quiet", "normal" (the default) or "loud"
pub fn loud_step(prefs: &Value) -> &'static str {
    let v = prefs["loudMatch"].as_str().unwrap_or("normal");
    LOUD_STEPS.iter().map(|(k, _)| *k).find(|k| *k == v).unwrap_or("normal")
}

fn loud_target(prefs: &Value) -> Option<f64> {
    match loud_step(prefs) {
        "quiet" => Some(-18.0),
        "normal" => Some(-14.0),
        "loud" => Some(-11.0),
        _ => None,
    }
}

/// The gain for a track, in dB: from its measured loudness, or a typical one until that's known
pub fn loud_db(prefs: &Value, t: &Track, lufs: Option<f64>) -> f32 {
    let Some(target) = loud_target(prefs) else { return 0.0 };
    if matches!(t.src, Src::Spotify(_)) {
        return 0.0;
    }
    (target - lufs.unwrap_or(LOUD_TYPICAL_LUFS)).clamp(-30.0, LOUD_MAX_BOOST) as f32
}

/// What to measure: ("local" | "soundcloud" | "youtube", id); a Go+ track by the version it plays
pub fn loud_source(t: &Track, yt_version: Option<&str>) -> Option<(&'static str, String)> {
    if let Some(v) = yt_version {
        return Some(("youtube", v.to_string()));
    }
    match &t.src {
        Src::SoundCloud(id) if !t.go_plus => Some(("soundcloud", id.to_string())),
        Src::YouTube(id) => Some(("youtube", id.clone())),
        Src::Local(_) => Some(("local", t.key.trim_start_matches("local:").to_string())),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn gains_and_bands() {
        let sc = Track { src: Src::SoundCloud(5), key: "5".into(), ..Default::default() };
        let sp = Track { src: Src::Spotify("spotify:track:x".into()), ..Default::default() };
        // normal unless set: -14 LUFS
        assert_eq!(loud_db(&json!({}), &sc, Some(-8.0)), -6.0);
        assert_eq!(loud_db(&json!({}), &sc, None), -4.0);
        assert_eq!(loud_db(&json!({ "loudMatch": "loud" }), &sc, Some(-30.0)), 9.0);
        assert_eq!(loud_db(&json!({ "loudMatch": "off" }), &sc, Some(-8.0)), 0.0);
        assert_eq!(loud_db(&json!({}), &sp, Some(-8.0)), 0.0);
        assert_eq!(loud_source(&sc, None), Some(("soundcloud", "5".into())));
        assert_eq!(loud_source(&sc, Some("v1")), Some(("youtube", "v1".into())));
        let p = json!({ "eqOn": false, "eq": [1, 2, 3, 4, 5, 6, 7, 8, 9, 40] });
        assert_eq!(eq_heard(&p), [0.0; 10]);
        assert_eq!(eq_bands(&p)[9], 12.0);
    }
}
