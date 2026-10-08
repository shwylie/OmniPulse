// The OmniPulse Input (Settings > Streaming): the app's sound, with its own volume and offset, played
// a second time so it can be routed apart from the main sound (to a stream mix only, say).
//
// Wave Link and the Sound Mixer list apps by program, so it's played by a small program of its own,
// omnipulse-input.exe (crates/input), shown as "OmniPulse Input". It runs only while the setting is
// on: this starts it, sends it the page's sound on its standard input, and stops it by closing that.

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::Mutex;
use std::time::Duration;

pub const PROGRAM: &str = "omnipulse-input.exe";

struct Running {
    child: Child,
    tx: SyncSender<Vec<u8>>,
}

static RUNNING: Mutex<Option<Running>> = Mutex::new(None);
/// The device it's playing on (it follows wherever it's routed)
static DEVICE: Mutex<String> = Mutex::new(String::new());

/// Where the program is: next to OmniPulse (the installer puts it there; so does a build)
fn program_path() -> Result<std::path::PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let dir = exe.parent().ok_or("no folder")?;
    let p = dir.join(PROGRAM);
    if p.exists() {
        return Ok(p);
    }
    // a test build of the core sits a folder deeper (target/debug/deps)
    let up = dir.parent().map(|d| d.join(PROGRAM));
    match up {
        Some(p) if p.exists() => Ok(p),
        _ => Err(format!("{PROGRAM} is missing next to OmniPulse; reinstalling should bring it back.")),
    }
}

/// Starts it (playing on the default playback device, muted in the Sound Mixer if asked); returns that
/// device's name
pub fn start(muted: bool) -> Result<String, String> {
    stop();
    let mut cmd = Command::new(program_path()?);
    if muted {
        cmd.arg("--muted");
    }
    cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // no console window
    }
    let mut child = cmd.spawn().map_err(|e| format!("Couldn't start the OmniPulse Input: {e}"))?;
    let stdout = child.stdout.take().ok_or("no output")?;
    let (line_tx, line_rx) = mpsc::channel();
    std::thread::spawn(move || {
        // its first line answers; later ones ("moved <device>") are read so its output never fills up
        let mut lines = BufReader::new(stdout).lines();
        let first = lines.next().and_then(|l| l.ok()).unwrap_or_default();
        let _ = line_tx.send(first);
        for l in lines.map_while(Result::ok) {
            if let Some(d) = l.strip_prefix("moved ") {
                *DEVICE.lock().unwrap() = d.to_string();
            }
        }
    });
    let line = line_rx.recv_timeout(Duration::from_secs(10)).unwrap_or_default();
    let line = line.trim();
    let Some(device) = line.strip_prefix("ok ") else {
        let _ = child.kill();
        let why = line.strip_prefix("error ").unwrap_or("it didn't answer");
        return Err(format!("Couldn't start the OmniPulse Input: {why}"));
    };
    // the sound goes through a short queue, so a slow moment there never holds up the app
    let mut stdin = child.stdin.take().ok_or("no input")?;
    let (tx, rx) = mpsc::sync_channel::<Vec<u8>>(32);
    std::thread::spawn(move || {
        while let Ok(piece) = rx.recv() {
            if stdin.write_all(&piece).is_err() {
                break;
            }
        }
    });
    *RUNNING.lock().unwrap() = Some(Running { child, tx });
    *DEVICE.lock().unwrap() = device.to_string();
    Ok(device.to_string())
}

/// 16-bit stereo PCM from the page, at its rate
pub fn push_pcm(bytes: &[u8], rate: u32) {
    let guard = RUNNING.lock().unwrap();
    let Some(r) = guard.as_ref() else { return };
    let mut piece = Vec::with_capacity(8 + bytes.len());
    piece.extend_from_slice(&rate.to_le_bytes());
    piece.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    piece.extend_from_slice(bytes);
    match r.tx.try_send(piece) {
        Ok(()) | Err(TrySendError::Full(_)) => {} // full: this piece is dropped rather than waited for
        Err(TrySendError::Disconnected(_)) => {}
    }
}

/// Stops it: its input closes and it ends by itself (and is ended if it doesn't)
pub fn stop() {
    let Some(mut r) = RUNNING.lock().unwrap().take() else { return };
    drop(r.tx);
    std::thread::spawn(move || {
        for _ in 0..20 {
            if matches!(r.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = r.child.kill();
    });
}

/// Where it's playing now
pub fn device() -> String {
    DEVICE.lock().unwrap().clone()
}

pub fn running() -> bool {
    RUNNING.lock().unwrap().is_some()
}

#[cfg(test)]
mod live {
    // the real thing on this PC's default device, silent: build the program first
    // (cargo build -p omnipulse-input), then cargo test -p omnipulse-core omni_program_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn omni_program_live() {
        let device = super::start(true).expect("starts");
        println!("playing on {device}");
        for _ in 0..30 {
            super::push_pcm(&vec![0u8; 4800 * 4], 48000);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        // it may be routed anywhere (Wave Link keys its routing to the program's name): every device's sessions
        super::diag::omni_sessions_dump_now();
        super::stop();
    }
}

#[cfg(test)]
mod diag {
    // every output device's sessions, with mute and how loud each is right now:
    // cargo test -p omnipulse-core omni_sessions_dump -- --ignored --nocapture
    #[test]
    #[ignore]
    #[cfg(windows)]
    fn omni_sessions_dump() {
        omni_sessions_dump_now();
    }

    #[cfg(windows)]
    pub(super) fn omni_sessions_dump_now() {
        use windows::core::Interface;
        use windows::Win32::Media::Audio::Endpoints::IAudioMeterInformation;
        use windows::Win32::Media::Audio::*;
        use windows::Win32::System::Com::StructuredStorage::PropVariantToStringAlloc;
        use windows::Win32::System::Com::*;
        use windows::Win32::System::Threading::*;
        use windows::Win32::Foundation::PROPERTYKEY;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let devs = en.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE).unwrap();
            let name_key = PROPERTYKEY { fmtid: windows::core::GUID::from_u128(0xa45c254e_df1c_4efd_8020_67d146a850e0), pid: 14 };
            for d in 0..devs.GetCount().unwrap() {
                let dev = devs.Item(d).unwrap();
                let props = dev.OpenPropertyStore(STGM_READ).unwrap();
                let dname = props.GetValue(&name_key).ok().and_then(|v| PropVariantToStringAlloc(&v).ok()).map(|p| p.to_string().unwrap_or_default()).unwrap_or_default();
                let mgr: IAudioSessionManager2 = dev.Activate(CLSCTX_ALL, None).unwrap();
                let list = mgr.GetSessionEnumerator().unwrap();
                for i in 0..list.GetCount().unwrap() {
                    let ctl = list.GetSession(i).unwrap();
                    let c2: IAudioSessionControl2 = ctl.cast().unwrap();
                    let pid = c2.GetProcessId().unwrap_or(0);
                    let mut exe = String::new();
                    if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                        let mut buf = [0u16; 260];
                        let mut n = buf.len() as u32;
                        if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok() {
                            exe = String::from_utf16_lossy(&buf[..n as usize]).rsplit('\\').next().unwrap_or("").to_string();
                        }
                    }
                    if !exe.to_lowercase().contains("omnipulse") && !exe.to_lowercase().contains("webview") {
                        continue;
                    }
                    let state = ctl.GetState().map(|s| s.0).unwrap_or(-1);
                    let muted = ctl.cast::<ISimpleAudioVolume>().and_then(|v| v.GetMute()).map(|b| b.as_bool()).unwrap_or(false);
                    let peak = ctl.cast::<IAudioMeterInformation>().and_then(|m| m.GetPeakValue()).unwrap_or(-1.0);
                    let disp = ctl.GetDisplayName().map(|s| s.to_string().unwrap_or_default()).unwrap_or_default();
                    println!("{dname} | {exe} (pid {pid}) | \"{disp}\" | state {state} (1 = active) | muted {muted} | peak {peak:.3}");
                }
            }
        }
    }
}

#[cfg(test)]
mod diag_installed {
    // starts the installed omnipulse-input.exe (silent) to see which device Windows puts it on:
    // cargo test -p omnipulse-core omni_installed_device -- --ignored --nocapture
    #[test]
    #[ignore]
    fn omni_installed_device() {
        use std::io::{BufRead, BufReader, Write};
        let exe = std::path::Path::new(&std::env::var("LOCALAPPDATA").unwrap()).join("OmniPulse").join(super::PROGRAM);
        let mut child = std::process::Command::new(&exe).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().expect("installed program");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        println!("it says: {}", line.trim());
        let mut stdin = child.stdin.take().unwrap();
        for _ in 0..20 {
            let mut piece = 48000u32.to_le_bytes().to_vec();
            piece.extend_from_slice(&(4800u32 * 4).to_le_bytes());
            piece.extend(vec![0u8; 4800 * 4]);
            let _ = stdin.write_all(&piece);
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        super::diag::omni_sessions_dump_now();
        drop(stdin);
        let _ = child.wait();
    }
}
