// OmniPulse's own sound, played from OmniPulse.exe (see crates/pcmout): the page sends its finished
// sound here instead of playing it from the WebView, whose process Discord's screen share and OBS's
// Application Audio Capture don't count as part of the app. The volume is applied here, and what's
// waiting is dropped on a pause, a seek or a new track, so those are heard at once.

use omnipulse_pcmout::{Options, Player};
use std::sync::Mutex;

static PLAYER: Mutex<Option<Player>> = Mutex::new(None);

/// Starts (or moves to) a device by name, "" for the one Windows gives the app; returns its name
pub fn start(device: &str) -> Result<String, String> {
    let p = Player::start(Options {
        name: "OmniPulse",
        muted: false,
        device: (!device.is_empty()).then(|| device.to_string()),
        prefill_ms: 120,
        max_ms: 600,
    })
    .map_err(|e| format!("Couldn't play on that device: {e}"))?;
    let name = p.device();
    *PLAYER.lock().unwrap() = Some(p);
    Ok(name)
}

pub fn push(bytes: &[u8], rate: u32) {
    if let Some(p) = PLAYER.lock().unwrap().as_ref() {
        p.push(bytes, rate);
    }
}

pub fn flush() {
    if let Some(p) = PLAYER.lock().unwrap().as_ref() {
        p.flush();
    }
}

pub fn set_volume(v: f32) {
    if let Some(p) = PLAYER.lock().unwrap().as_ref() {
        p.set_volume(v);
    }
}

pub fn stop() {
    PLAYER.lock().unwrap().take();
}
