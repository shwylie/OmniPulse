// OmniPulse Offset (Settings > Streaming), as the web app's js/output2.js: the sound played a second
// time by its own program (omnipulse-offset.exe, through crates/core/src/omni_input.rs), so it shows
// apart in Wave Link or Windows' mixer, with its own volume and an offset to line the two up. Above 0
// it's later; below 0 the main output is held back instead. The same settings as the web app's:
// out2On, out2Offset (ms, -5000 to 5000) and out2Volume (0 to 1).

use omnipulse_audio::Engine;
use omnipulse_core::omni_input;
use serde_json::Value;
use std::collections::VecDeque;

const MAX_MS: i64 = 5000;
/// the pieces sent to it: 20 ms each, so it stays close to the main sound
const PIECE_MS: u64 = 20;

/// Starts, changes or stops it as the settings say; what to tell you (the device it plays on, or why not)
pub fn apply(engine: &Engine, prefs: &Value) -> Result<String, String> {
    // (while casting it rests: the sound is on the device, so it would only echo; it's back after)
    if prefs["out2On"].as_bool() != Some(true) || crate::casting().is_some() {
        engine.set_tap("offset", None);
        engine.set_main_delay_ms(0);
        omni_input::stop();
        return Ok(String::new());
    }
    let offset = prefs["out2Offset"].as_f64().unwrap_or(0.0).round() as i64;
    let offset = offset.clamp(-MAX_MS, MAX_MS);
    let vol = prefs["out2Volume"].as_f64().unwrap_or(1.0).clamp(0.0, 1.0) as f32;
    let device = if omni_input::running() { omni_input::device() } else { omni_input::start(false)? };
    engine.set_main_delay_ms(if offset < 0 { (-offset) as u32 } else { 0 });
    let rate = engine.output_rate();
    let later = if offset > 0 { offset as usize * rate as usize / 1000 } else { 0 };
    let piece = (rate as u64 * PIECE_MS / 1000) as usize * 4;
    engine.set_tap("offset", Some(tap(vol, later, piece, move |b| omni_input::push_pcm(b, rate))));
    Ok(device)
}

/// 16-bit stereo PCM of the feed, for another output (casting uses it too)
pub fn pcm(s: &[f32], out: &mut Vec<u8>) {
    for &x in s {
        out.extend_from_slice(&((x.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
    }
}

/// The feed: its volume, its own delay line (`later` frames, when it's the later one), then 16-bit
/// stereo pieces of `piece` bytes to `send`
fn tap(vol: f32, later: usize, piece: usize, mut send: impl FnMut(&[u8]) + Send + 'static) -> omnipulse_audio::Tap {
    let mut line: VecDeque<f32> = std::iter::repeat_n(0.0, later * 2).collect();
    let mut bytes: Vec<u8> = Vec::with_capacity(piece * 2);
    Box::new(move |s: &[f32]| {
        for &x in s {
            line.push_back(x * vol);
            let y = line.pop_front().unwrap_or(0.0);
            bytes.extend_from_slice(&((y.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        if bytes.len() >= piece {
            send(&bytes);
            bytes.clear();
        }
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn later_by_its_offset_at_its_volume() {
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::<i16>::new()));
        let g = got.clone();
        // 2 frames later, half as loud, sent 3 frames (12 bytes) at a time
        let mut t = super::tap(0.5, 2, 12, move |b| g.lock().unwrap().extend(b.chunks(2).map(|c| i16::from_le_bytes([c[0], c[1]]))));
        t(&[1.0, 1.0, 1.0, 1.0, 1.0, 1.0]);
        let v = got.lock().unwrap().clone();
        assert_eq!(v, [0, 0, 0, 0, 16383, 16383]);
    }
}

