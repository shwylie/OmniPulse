// Plays what the app sends (see main.rs) on the default playback device, through cpal (WASAPI,
// shared mode), resampled from the app's rate to the device's. Its audio session is named
// "OmniPulse Input", and can start muted in the Sound Mixer.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc, Mutex};

pub const SESSION_NAME: &str = "OmniPulse Input";

struct Feed {
    frames: VecDeque<[f32; 2]>,
    rate: u32,
    phase: f64,
    primed: bool,
}

static FEED: Mutex<Feed> = Mutex::new(Feed { frames: VecDeque::new(), rate: 48000, phase: 0.0, primed: false });
static RUNNING: Mutex<Option<Arc<AtomicBool>>> = Mutex::new(None);

// how much is kept ready (it starts playing once there's this much), and the most it may lag by
const PREFILL_MS: usize = 60;
const MAX_MS: usize = 400;

pub fn push_pcm(bytes: &[u8], rate: u32) {
    if RUNNING.lock().unwrap().is_none() {
        return;
    }
    let mut f = FEED.lock().unwrap();
    if rate > 0 && rate != f.rate {
        f.rate = rate;
        f.frames.clear();
        f.primed = false;
    }
    for c in bytes.as_chunks::<4>().0 {
        let l = i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0;
        let r = i16::from_le_bytes([c[2], c[3]]) as f32 / 32768.0;
        f.frames.push_back([l, r]);
    }
    let max = f.rate as usize * MAX_MS / 1000;
    if f.frames.len() > max {
        // fell behind (the device stalled): drop the oldest, back to the usual margin
        let keep = f.rate as usize * PREFILL_MS * 2 / 1000;
        let drop = f.frames.len() - keep;
        f.frames.drain(..drop);
    }
}

/// Fills one device buffer: linear resampling from the page's rate, silence while there's too little
pub(crate) fn fill(out: &mut [f32], channels: usize, out_rate: u32) {
    let mut f = FEED.lock().unwrap();
    let step = f.rate as f64 / out_rate as f64;
    let prefill = f.rate as usize * PREFILL_MS / 1000;
    if !f.primed && f.frames.len() >= prefill.max(2) {
        f.primed = true;
    }
    for frame in out.chunks_mut(channels.max(1)) {
        if !f.primed || f.frames.len() < 2 {
            f.primed = false;
            frame.iter_mut().for_each(|s| *s = 0.0);
            continue;
        }
        let (a, b, t) = (f.frames[0], f.frames[1], f.phase as f32);
        let l = a[0] + (b[0] - a[0]) * t;
        let r = a[1] + (b[1] - a[1]) * t;
        match frame.len() {
            1 => frame[0] = (l + r) * 0.5,
            _ => {
                frame[0] = l;
                frame[1] = r;
                frame[2..].iter_mut().for_each(|s| *s = 0.0);
            }
        }
        f.phase += step;
        while f.phase >= 1.0 && !f.frames.is_empty() {
            f.frames.pop_front();
            f.phase -= 1.0;
        }
    }
}

/// Starts playing (on the default playback device); returns that device's name. `muted`: start muted
/// in Windows' mixer (where it can be unmuted once it's routed)
pub fn start(muted: bool) -> Result<String, String> {
    stop();
    let alive = Arc::new(AtomicBool::new(true));
    {
        let mut f = FEED.lock().unwrap();
        f.frames.clear();
        f.primed = false;
        f.phase = 0.0;
    }
    let (tx, rx) = mpsc::channel::<Result<String, String>>();
    let flag = alive.clone();
    // a cpal stream stays on the thread that made it
    std::thread::spawn(move || {
        let made = (|| -> Result<(cpal::Stream, String), String> {
            let host = cpal::default_host();
            let dev = host.default_output_device().ok_or("There's no playback device.")?;
            let name = dev.name().unwrap_or_else(|_| "the default device".into());
            let sup = dev.default_output_config().map_err(|e| e.to_string())?;
            let fmt = sup.sample_format();
            let cfg: cpal::StreamConfig = sup.into();
            let (ch, rate) = (cfg.channels as usize, cfg.sample_rate.0);
            let err = |e| eprintln!("OmniPulse Input: {e}");
            let stream = match fmt {
                cpal::SampleFormat::F32 => dev.build_output_stream(&cfg, move |d: &mut [f32], _| fill(d, ch, rate), err, None),
                cpal::SampleFormat::I16 => {
                    let mut tmp = Vec::new();
                    dev.build_output_stream(&cfg, move |d: &mut [i16], _| {
                        tmp.resize(d.len(), 0.0);
                        fill(&mut tmp, ch, rate);
                        d.iter_mut().zip(&tmp).for_each(|(o, s)| *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16);
                    }, err, None)
                }
                other => return Err(format!("The playback device uses a sample format ({other}) this can't play.")),
            }
            .map_err(|e| e.to_string())?;
            stream.play().map_err(|e| e.to_string())?;
            name_session(muted);
            Ok((stream, name))
        })();
        match made {
            Ok((stream, name)) => {
                let _ = tx.send(Ok(name));
                while flag.load(Ordering::SeqCst) {
                    std::thread::park_timeout(std::time::Duration::from_millis(500));
                }
                drop(stream);
            }
            Err(e) => {
                let _ = tx.send(Err(e));
            }
        }
    });
    match rx.recv_timeout(std::time::Duration::from_secs(10)) {
        Ok(Ok(name)) => {
            *RUNNING.lock().unwrap() = Some(alive);
            Ok(name)
        }
        Ok(Err(e)) => {
            alive.store(false, Ordering::SeqCst);
            Err(format!("Couldn't start the OmniPulse Input: {e}"))
        }
        Err(_) => {
            alive.store(false, Ordering::SeqCst);
            Err("Couldn't start the OmniPulse Input: the playback device didn't answer.".into())
        }
    }
}

pub fn stop() {
    if let Some(a) = RUNNING.lock().unwrap().take() {
        a.store(false, Ordering::SeqCst);
    }
}

#[cfg(test)]
pub fn running() -> bool {
    RUNNING.lock().unwrap().is_some()
}

// Windows' mixer (and Wave Link) shows this stream as "OmniPulse Input": the name of this process's
// audio session on the default device, which is the session this stream plays in
#[cfg(windows)]
fn name_session(muted: bool) {
    use windows::core::HSTRING;
    use windows::Win32::Media::Audio::{eConsole, eRender, IAudioSessionManager, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let r = (|| -> windows::core::Result<()> {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let mgr: IAudioSessionManager = dev.Activate(CLSCTX_ALL, None)?;
            let ctl = mgr.GetAudioSessionControl(None, 0)?;
            ctl.SetDisplayName(&HSTRING::from(SESSION_NAME), std::ptr::null())?;
            if muted {
                mgr.GetSimpleAudioVolume(None, 0)?.SetMute(true, std::ptr::null())?;
            }
            Ok(())
        })();
        if let Err(e) = r {
            eprintln!("OmniPulse Input: couldn't name the session: {e}");
        }
    }
}

#[cfg(not(windows))]
fn name_session(_muted: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resamples_and_waits_for_enough() {
        *RUNNING.lock().unwrap() = Some(Arc::new(AtomicBool::new(true)));
        {
            let mut f = FEED.lock().unwrap();
            f.frames.clear();
            f.primed = false;
            f.phase = 0.0;
        }
        // too little yet: silence
        let one: Vec<u8> = [16384i16, -16384].iter().flat_map(|s| s.to_le_bytes()).collect();
        push_pcm(&one.repeat(10), 48000);
        let mut out = vec![1.0f32; 8];
        fill(&mut out, 2, 48000);
        assert!(out.iter().all(|&s| s == 0.0));
        // enough: the sound, at the device's rate (44.1 kHz from 48 kHz takes fewer frames)
        push_pcm(&one.repeat(4800), 48000);
        let mut out = vec![0.0f32; 2 * 441];
        fill(&mut out, 2, 44100);
        assert!((out[0] - 0.5).abs() < 1e-3 && (out[1] + 0.5).abs() < 1e-3);
        let left = FEED.lock().unwrap().frames.len();
        assert!((4810 - 480 - 2..=4810 - 480 + 2).contains(&left), "{left}");
        // a 6-channel device gets the sound on the front pair only
        let mut out = vec![9.0f32; 12];
        fill(&mut out, 6, 48000);
        assert_eq!(&out[2..6], &[0.0; 4]);
        stop();
    }
}

#[cfg(test)]
mod live {
    // a real (silent) stream on this PC's default device: cargo test -p omnipulse-core omni_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn omni_live() {
        let name = super::start(true).expect("starts");
        println!("playing on {name}");
        super::push_pcm(&vec![0u8; 48000 * 4 / 2], 48000);
        std::thread::sleep(std::time::Duration::from_millis(1500));
        assert!(super::running());
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Media::Audio::{eConsole, eRender, IAudioSessionManager, IMMDeviceEnumerator, MMDeviceEnumerator};
            use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let mgr: IAudioSessionManager = en.GetDefaultAudioEndpoint(eRender, eConsole).unwrap().Activate(CLSCTX_ALL, None).unwrap();
            let vol = mgr.GetSimpleAudioVolume(None, 0).unwrap();
            let muted = vol.GetMute().unwrap().as_bool();
            let name = mgr.GetAudioSessionControl(None, 0).unwrap().GetDisplayName().unwrap().to_string().unwrap();
            println!("session: {name}, muted: {muted}");
            // leave the test program's session as it was
            vol.SetMute(false, std::ptr::null()).unwrap();
            assert!(muted);
            assert_eq!(name, super::SESSION_NAME);
        }
        super::stop();
    }
}
