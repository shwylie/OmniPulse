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

/// Fade Between Tracks: how long, if it's on (1 to 8 seconds; 3 unless set)
pub fn fade_ms(prefs: &Value) -> Option<u64> {
    (prefs["fadeOn"].as_bool() == Some(true)).then(|| prefs["fadeMs"].as_u64().unwrap_or(3000).clamp(500, 10_000))
}

/// Skip Silences: silences this long (or longer) are jumped over; 0 is off (8 s unless set)
pub fn skip_silence_ms(prefs: &Value) -> u64 {
    prefs["skipSilence"].as_u64().unwrap_or(8) * 1000
}

pub const LOUD_STEPS: [(&str, &str); 4] = [("off", "Off"), ("quiet", "Quiet"), ("normal", "Normal"), ("loud", "Loud")];
const LOUD_MAX_BOOST: f64 = 9.0;
const LOUD_TYPICAL_LUFS: f64 = -10.0;

/// "off", "quiet", "normal" (the default) or "loud"
pub fn loud_step(prefs: &Value) -> &'static str {
    let v = prefs["loudMatch"].as_str().unwrap_or("normal");
    LOUD_STEPS.iter().map(|(k, _)| *k).find(|k| *k == v).unwrap_or("normal")
}

/// Spotify's own levelling (its files' loudness values, applied by its player), at the same target:
/// (on, target LUFS)
pub fn spotify_norm(prefs: &Value) -> (bool, f64) {
    match loud_target(prefs) { Some(t) => (true, t), None => (false, -14.0) }
}

/// A preview of Match Loudness, as a WAV: a stretch of the sample track as if it were mastered quietly
/// (about -22 LUFS), then the next as if it were mastered loudly (about -8), each turned up or down as Match
/// Loudness would turn a song made that way. Off: the jump between them as it is; on: the two level. `a` and `b`:
/// interleaved stereo at `rate`
pub fn preview_wav(prefs: &Value, a: &[f32], b: &[f32], rate: u32) -> Vec<u8> {
    let target = loud_target(prefs);
    let gain = |lufs: f64| -> f32 { 10f32.powf(target.map_or(0.0, |t| (t - lufs).clamp(-30.0, LOUD_MAX_BOOST)) as f32 / 20.0) };
    // (a stretch at a loudness, by its RMS; faded in and out so neither clicks)
    let at = |v: &[f32], rms_db: f32| -> Vec<f32> {
        let rms = (v.iter().map(|x| x * x).sum::<f32>() / v.len().max(1) as f32).sqrt().max(1e-6);
        let k = 10f32.powf(rms_db / 20.0) / rms;
        let (n, fade) = (v.len() / 2, (0.05 * rate as f32) as usize);
        v.iter().enumerate().map(|(i, x)| { let f = i / 2; x * k * ((f.min(n.saturating_sub(1 + f))) as f32 / fade.max(1) as f32).min(1.0) }).collect()
    };
    let mut all: Vec<f32> = at(a, -22.0).into_iter().map(|x| x * gain(-22.0)).collect();
    all.extend(std::iter::repeat_n(0.0, (0.5 * rate as f32) as usize * 2));
    all.extend(at(b, -8.0).into_iter().map(|x| x * gain(-8.0)));
    // (a 16-bit stereo WAV)
    let data: Vec<u8> = all.iter().flat_map(|x| ((x.clamp(-1.0, 1.0) * 32000.0) as i16).to_le_bytes()).collect();
    let mut w = Vec::with_capacity(44 + data.len());
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    w
}

#[cfg(test)]
mod preview_tests {
    /// Off: the second stretch far louder than the first; Normal: the two within a dB or so
    #[test]
    fn the_preview_levels_when_on() {
        let tone: Vec<f32> = (0..44100 * 2).map(|i| ((i / 2) as f32 * 0.05).sin() * 0.3).collect();
        let level = |w: &[u8]| {
            let s: Vec<f32> = w[44..].chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]) as f32 / 32000.0).collect();
            let half = s.len() / 2;
            let rms = |p: &[f32]| 20.0 * ((p.iter().map(|x| x * x).sum::<f32>() / p.len() as f32).sqrt()).log10();
            (rms(&s[..half * 8 / 10]), rms(&s[half * 12 / 10..]))
        };
        let (a, b) = level(&super::preview_wav(&serde_json::json!({ "loudMatch": "off" }), &tone, &tone, 44100));
        assert!(b - a > 10.0, "off: {a} then {b}");
        let (a, b) = level(&super::preview_wav(&serde_json::json!({ "loudMatch": "normal" }), &tone, &tone, 44100));
        assert!((b - a).abs() < 1.5, "normal: {a} then {b}");
    }

    /// The sample track that ships in the app: read from 30 s in, two 7 s stretches, made into a preview
    #[test]
    fn the_sample_track_reads() {
        use omnipulse_audio::Source;
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/sound-test.ogg");
        let mut d = omnipulse_audio::Decoded::open_file(&path).unwrap();
        let rate = d.rate();
        d.seek(30_000).unwrap();
        let want = (7 * rate) as usize * 2;
        let mut all = Vec::new();
        while all.len() < want * 2 { if !d.read(&mut all).unwrap() { break; } }
        assert!(all.len() >= want * 2, "only {} samples", all.len());
        let w = super::preview_wav(&serde_json::json!({ "loudMatch": "normal" }), &all[..want], &all[want..want * 2], rate);
        assert!(w.len() > 44 + want * 4);
    }
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
/// (`via_version`: it plays from another recording, its YouTube Music version: measured, and matched,
/// as that; a Spotify song Spotify plays is levelled by Spotify's own player instead, see spotify_norm)
pub fn loud_db(prefs: &Value, t: &Track, lufs: Option<f64>, via_version: bool) -> f32 {
    let Some(target) = loud_target(prefs) else { return 0.0 };
    if matches!(t.src, Src::Spotify(_)) && !via_version {
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
        assert_eq!(loud_db(&json!({}), &sc, Some(-8.0), false), -6.0);
        assert_eq!(loud_db(&json!({}), &sc, None, false), -4.0);
        assert_eq!(loud_db(&json!({ "loudMatch": "loud" }), &sc, Some(-30.0), false), 9.0);
        assert_eq!(loud_db(&json!({ "loudMatch": "off" }), &sc, Some(-8.0), false), 0.0);
        // Spotify's own: levelled by its player (0 here); its YouTube Music version: matched as that
        assert_eq!(loud_db(&json!({}), &sp, Some(-8.0), false), 0.0);
        assert_eq!(loud_db(&json!({}), &sp, Some(-8.0), true), -6.0);
        assert_eq!(spotify_norm(&json!({ "loudMatch": "quiet" })), (true, -18.0));
        assert_eq!(spotify_norm(&json!({ "loudMatch": "off" })).0, false);
        assert_eq!(loud_source(&sc, None), Some(("soundcloud", "5".into())));
        assert_eq!(loud_source(&sc, Some("v1")), Some(("youtube", "v1".into())));
        let p = json!({ "eqOn": false, "eq": [1, 2, 3, 4, 5, 6, 7, 8, 9, 40] });
        assert_eq!(eq_heard(&p), [0.0; 10]);
        assert_eq!(eq_bands(&p)[9], 12.0);
    }
}
