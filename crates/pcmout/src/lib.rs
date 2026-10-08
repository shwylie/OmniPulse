// Plays 16-bit stereo PCM sent in pieces (by the web page, through the app) on a playback device,
// through cpal (WASAPI, shared mode), resampled from the sender's rate to the device's. Used twice:
//   - OmniPulse's own sound, from OmniPulse.exe itself, so programs that capture an app's sound by its
//     window (Discord's screen share, OBS's Application Audio Capture) hear it; the WebView's own
//     sound comes from another process they don't count as the app
//   - the OmniPulse Input, from its own program (crates/input), so Wave Link lists it apart
//
// Its audio session gets a name, can start muted in the Sound Mixer, and with no device chosen it
// follows wherever Windows routes the program (Wave Link, Windows' per-app sound settings).

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::time::Duration;

pub struct Options {
    /// the session's name in the Sound Mixer
    pub name: &'static str,
    pub muted: bool,
    /// a device by name; None: the one Windows gives this program (and follow it)
    pub device: Option<String>,
    /// how much is kept ready: it starts playing once there's this much
    pub prefill_ms: usize,
    /// the most it may lag by before the oldest is dropped
    pub max_ms: usize,
}

struct Feed {
    frames: VecDeque<[f32; 2]>,
    rate: u32,
    phase: f64,
    primed: bool,
    prefill_ms: usize,
    max_ms: usize,
}

struct Inner {
    feed: Mutex<Feed>,
    volume: AtomicU32,
    alive: AtomicBool,
    device: Mutex<String>,
}

pub struct Player {
    inner: Arc<Inner>,
}

impl Player {
    /// Opens the device and starts playing (silence until sound arrives); errors say why it can't
    pub fn start(opts: Options) -> Result<Self, String> {
        let inner = Arc::new(Inner {
            feed: Mutex::new(Feed { frames: VecDeque::new(), rate: 48000, phase: 0.0, primed: false, prefill_ms: opts.prefill_ms, max_ms: opts.max_ms }),
            volume: AtomicU32::new(1.0f32.to_bits()),
            alive: AtomicBool::new(true),
            device: Mutex::new(String::new()),
        });
        let (tx, rx) = mpsc::channel::<Result<String, String>>();
        let shared = inner.clone();
        // a cpal stream stays on the thread that made it
        std::thread::Builder::new()
            .name("op-pcmout".into())
            .spawn(move || run(shared, opts, tx))
            .map_err(|e| e.to_string())?;
        match rx.recv_timeout(Duration::from_secs(10)) {
            Ok(Ok(name)) => {
                *inner.device.lock().unwrap() = name;
                Ok(Self { inner })
            }
            Ok(Err(e)) => Err(e),
            Err(_) => Err("the playback device didn't answer".into()),
        }
    }

    /// 16-bit stereo PCM at `rate`
    pub fn push(&self, bytes: &[u8], rate: u32) {
        let mut f = self.inner.feed.lock().unwrap();
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
        let max = f.rate as usize * f.max_ms / 1000;
        if f.frames.len() > max {
            // fell behind (the device stalled): drop the oldest, back to twice the usual margin
            let keep = f.rate as usize * f.prefill_ms * 2 / 1000;
            let drop = f.frames.len() - keep;
            f.frames.drain(..drop);
        }
    }

    /// Drops what's waiting to be played (a pause, a seek, a new track: it would be heard late)
    pub fn flush(&self) {
        let mut f = self.inner.feed.lock().unwrap();
        f.frames.clear();
        f.primed = false;
        f.phase = 0.0;
    }

    /// 0..1, applied as it plays (so it's heard at once)
    pub fn set_volume(&self, v: f32) {
        self.inner.volume.store(v.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
    }

    /// The device it's playing on
    pub fn device(&self) -> String {
        self.inner.device.lock().unwrap().clone()
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        self.inner.alive.store(false, Ordering::SeqCst);
    }
}

fn find_device(name: &Option<String>) -> Result<cpal::Device, String> {
    let host = cpal::default_host();
    match name {
        Some(n) if !n.is_empty() => host
            .output_devices()
            .map_err(|e| e.to_string())?
            .find(|d| d.name().map(|x| &x == n).unwrap_or(false))
            .ok_or_else(|| format!("{n} isn't connected")),
        _ => host.default_output_device().ok_or_else(|| "there's no playback device".to_string()),
    }
}

fn open(inner: &Arc<Inner>, opts: &Options, muted: bool) -> Result<(cpal::Stream, String), String> {
    let dev = find_device(&opts.device)?;
    let name = dev.name().unwrap_or_else(|_| "the default device".into());
    let sup = dev.default_output_config().map_err(|e| e.to_string())?;
    let fmt = sup.sample_format();
    let cfg: cpal::StreamConfig = sup.into();
    let (ch, rate) = (cfg.channels as usize, cfg.sample_rate.0);
    let err = |e| eprintln!("pcmout: {e}");
    let a = inner.clone();
    let mut vol = f32::from_bits(inner.volume.load(Ordering::Relaxed));
    let mut fill = move |out: &mut [f32]| {
        fill_from(&a.feed, out, ch, rate);
        // the volume eases over about 20 ms
        let target = f32::from_bits(a.volume.load(Ordering::Relaxed));
        let k = (-1.0 / (0.02 * rate as f32)).exp();
        for frame in out.chunks_mut(ch.max(1)) {
            vol = target + (vol - target) * k;
            frame.iter_mut().for_each(|s| *s *= vol);
        }
    };
    let stream = match fmt {
        cpal::SampleFormat::F32 => dev.build_output_stream(&cfg, move |d: &mut [f32], _| fill(d), err, None),
        cpal::SampleFormat::I16 => {
            let mut tmp = Vec::new();
            dev.build_output_stream(&cfg, move |d: &mut [i16], _| {
                tmp.resize(d.len(), 0.0);
                fill(&mut tmp);
                d.iter_mut().zip(&tmp).for_each(|(o, s)| *o = (s.clamp(-1.0, 1.0) * 32767.0) as i16);
            }, err, None)
        }
        other => return Err(format!("the playback device uses a sample format ({other}) this can't play")),
    }
    .map_err(|e| e.to_string())?;
    stream.play().map_err(|e| e.to_string())?;
    name_session(opts.name, muted);
    Ok((stream, name))
}

fn run(inner: Arc<Inner>, opts: Options, tx: mpsc::Sender<Result<String, String>>) {
    let (mut stream, mut name) = match open(&inner, &opts, opts.muted) {
        Ok(x) => x,
        Err(e) => {
            let _ = tx.send(Err(e));
            return;
        }
    };
    let _ = tx.send(Ok(name.clone()));
    let follow = opts.device.as_deref().map(str::is_empty).unwrap_or(true);
    let mut ticks = 0u32;
    while inner.alive.load(Ordering::SeqCst) {
        std::thread::park_timeout(Duration::from_millis(250));
        ticks += 1;
        // an open stream stays where it was opened: follow the program's routing (about once a second)
        if !follow || !ticks.is_multiple_of(4) {
            continue;
        }
        let wanted = cpal::default_host().default_output_device().and_then(|d| d.name().ok());
        if let Some(w) = wanted {
            if w != name {
                if let Ok((s, n)) = open(&inner, &opts, false) {
                    stream = s; // the old one stops as it's dropped
                    name = n;
                    *inner.device.lock().unwrap() = name.clone();
                }
            }
        }
    }
    drop(stream);
}

/// Fills one device buffer: linear resampling from the sender's rate, silence while there's too little
fn fill_from(feed: &Mutex<Feed>, out: &mut [f32], channels: usize, out_rate: u32) {
    let mut f = feed.lock().unwrap();
    let step = f.rate as f64 / out_rate as f64;
    let prefill = f.rate as usize * f.prefill_ms / 1000;
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

// The Sound Mixer (and Wave Link) shows the stream under this name: this process's audio session on
// the device Windows gives it, which is where the stream plays
#[cfg(windows)]
fn name_session(name: &str, muted: bool) {
    use windows::core::HSTRING;
    use windows::Win32::Media::Audio::{eConsole, eRender, IAudioSessionManager, IMMDeviceEnumerator, MMDeviceEnumerator};
    use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_ALL, COINIT_MULTITHREADED};
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let r = (|| -> windows::core::Result<()> {
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let dev = en.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let mgr: IAudioSessionManager = dev.Activate(CLSCTX_ALL, None)?;
            mgr.GetAudioSessionControl(None, 0)?.SetDisplayName(&HSTRING::from(name), std::ptr::null())?;
            if muted {
                mgr.GetSimpleAudioVolume(None, 0)?.SetMute(true, std::ptr::null())?;
            }
            Ok(())
        })();
        if let Err(e) = r {
            eprintln!("pcmout: couldn't name the session: {e}");
        }
    }
}

#[cfg(not(windows))]
fn name_session(_name: &str, _muted: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(prefill_ms: usize) -> Mutex<Feed> {
        Mutex::new(Feed { frames: VecDeque::new(), rate: 48000, phase: 0.0, primed: false, prefill_ms, max_ms: 400 })
    }

    fn push(feed: &Mutex<Feed>, frames: usize, l: f32, r: f32) {
        let mut f = feed.lock().unwrap();
        for _ in 0..frames {
            f.frames.push_back([l, r]);
        }
    }

    #[test]
    fn resamples_and_waits_for_enough() {
        let f = feed(60);
        // too little yet: silence
        push(&f, 10, 0.5, -0.5);
        let mut out = vec![1.0f32; 8];
        fill_from(&f, &mut out, 2, 48000);
        assert!(out.iter().all(|&s| s == 0.0));
        // enough: the sound, at the device's rate (44.1 kHz from 48 kHz takes fewer frames)
        push(&f, 4800, 0.5, -0.5);
        let mut out = vec![0.0f32; 2 * 441];
        fill_from(&f, &mut out, 2, 44100);
        assert!((out[0] - 0.5).abs() < 1e-3 && (out[1] + 0.5).abs() < 1e-3);
        let left = f.lock().unwrap().frames.len();
        assert!((4810 - 480 - 2..=4810 - 480 + 2).contains(&left), "{left}");
        // a 6-channel device gets the sound on the front pair only
        let mut out = vec![9.0f32; 12];
        fill_from(&f, &mut out, 6, 48000);
        assert_eq!(&out[2..6], &[0.0; 4]);
    }
}

#[cfg(test)]
mod live {
    // a real (silent) stream on this PC's default device: cargo test -p omnipulse-pcmout pcmout_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn pcmout_live() {
        let p = super::Player::start(super::Options { name: "OmniPulse test", muted: false, device: None, prefill_ms: 120, max_ms: 600 }).expect("starts");
        println!("playing on {}", p.device());
        for _ in 0..20 {
            p.push(&vec![0u8; 4800 * 4], 48000);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        p.flush();
        p.set_volume(0.5);
    }
}
