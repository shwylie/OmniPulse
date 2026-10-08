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
        let mut line = String::new();
        let _ = BufReader::new(stdout).read_line(&mut line);
        let _ = line_tx.send(line);
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
        let found = sessions();
        println!("{found:?}");
        super::stop();
        let ours = found.iter().find(|(exe, _, _)| exe.eq_ignore_ascii_case(super::PROGRAM)).expect("a session from omnipulse-input.exe");
        assert_eq!(ours.1, "OmniPulse Input");
        assert!(ours.2, "started muted");
    }

    /// The default device's audio sessions: (program, display name, muted)
    #[cfg(windows)]
    fn sessions() -> Vec<(String, String, bool)> {
        use windows::core::Interface;
        use windows::Win32::Media::Audio::*;
        use windows::Win32::System::Com::*;
        use windows::Win32::System::Threading::*;
        let mut out = Vec::new();
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let en: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).unwrap();
            let mgr: IAudioSessionManager2 = en.GetDefaultAudioEndpoint(eRender, eConsole).unwrap().Activate(CLSCTX_ALL, None).unwrap();
            let list = mgr.GetSessionEnumerator().unwrap();
            for i in 0..list.GetCount().unwrap() {
                let ctl = list.GetSession(i).unwrap();
                let c2: IAudioSessionControl2 = ctl.cast().unwrap();
                let pid = c2.GetProcessId().unwrap_or(0);
                let name = ctl.GetDisplayName().map(|s| s.to_string().unwrap_or_default()).unwrap_or_default();
                let muted = ctl.cast::<ISimpleAudioVolume>().and_then(|v| v.GetMute()).map(|b| b.as_bool()).unwrap_or(false);
                let mut exe = String::new();
                if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                    let mut buf = [0u16; 260];
                    let mut n = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, windows::core::PWSTR(buf.as_mut_ptr()), &mut n).is_ok() {
                        exe = String::from_utf16_lossy(&buf[..n as usize]).rsplit('\\').next().unwrap_or("").to_string();
                    }
                }
                out.push((exe, name, muted));
            }
        }
        out
    }
}
