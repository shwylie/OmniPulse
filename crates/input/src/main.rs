// OmniPulse Input, as its own program: Wave Link, the Sound Mixer and the like list apps by program,
// so the app's second stream has to come from one of its own to be routed apart from its main sound.
//
// OmniPulse starts it (omnipulse-input [--muted]) while the OmniPulse Input is on and sends it the
// sound on its standard input: pieces of [rate: u32 LE][length: u32 LE][16-bit stereo PCM]. It
// answers one line ("ok <device>" or "error <why>") and stops when its input closes, which is also
// what happens if OmniPulse closes or crashes.

#![windows_subsystem = "windows"]

mod player;

use std::io::{Read, Write};

fn main() {
    let muted = std::env::args().any(|a| a == "--muted");
    let mut out = std::io::stdout();
    match player::start(muted) {
        Ok(device) => {
            let _ = writeln!(out, "ok {device}");
        }
        Err(e) => {
            let _ = writeln!(out, "error {e}");
            let _ = out.flush();
            return;
        }
    }
    let _ = out.flush();
    let mut input = std::io::stdin().lock();
    let mut head = [0u8; 8];
    let mut pcm = Vec::new();
    while input.read_exact(&mut head).is_ok() {
        let rate = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        if len > 4 * 1024 * 1024 {
            break; // not something OmniPulse sends
        }
        pcm.resize(len, 0);
        if input.read_exact(&mut pcm).is_err() {
            break;
        }
        player::push_pcm(&pcm, rate);
    }
    player::stop();
}
