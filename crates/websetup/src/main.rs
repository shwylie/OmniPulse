//! OmniPulse's web installer: the one file people download, the same for every version, so Windows' SmartScreen
//! comes to know it (and it's submitted to Microsoft once). It asks OmniPulse's relay for the latest stable
//! release (as the app's own updater does), downloads its installer, checks it's signed with OmniPulse's key,
//! and runs it. A file a program downloads isn't marked as from the internet, so that installer isn't checked
//! by SmartScreen again: only this one ever is.
#![windows_subsystem = "windows"]

use std::sync::{Arc, Mutex};

/// the stable releases, on the relay (the app's updater asks the same)
const LATEST: &str = "https://omnipulse-jam.shwylie101.workers.dev/updates/stable/latest.json";
/// where an installer may come from: the relay's downloads, or the releases on GitHub
const FROM: [&str; 2] = ["https://omnipulse-jam.shwylie101.workers.dev/updates/download/", "https://github.com/shwylie/omnipulse-native-releases/"];
/// the releases' page, for downloading it directly if this can't
const RELEASES: &str = "https://github.com/shwylie/omnipulse-native-releases/releases/latest";
/// OmniPulse's key (the updater's: a minisign public key, base64), the only one whose installers are run
const PUBKEY: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IENFQ0Y4MEMxMDNFNDhCRTYKUldUbWkrUUR3WURQem1TTE93NnBweCtpemIxdjRVWE5HMDVLQUc1QTZZV1E4TGoydG5OdWI2Z0QK";

#[derive(Clone, Default)]
enum Phase {
    #[default]
    Asking,
    Getting,
    Checking,
    Ready(std::path::PathBuf),
    Failed(String),
}

#[derive(Default)]
struct State {
    phase: Phase,
    version: String,
    got: u64,
    total: u64,
    cancelled: bool,
}

/// Whether `data` is what OmniPulse's key signed (`signature`: base64 of minisign's .sig text)
fn verify(data: &[u8], signature: &str) -> Result<(), String> {
    use base64::Engine as _;
    let b64 = base64::engine::general_purpose::STANDARD;
    let key = String::from_utf8(b64.decode(PUBKEY).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    let sig = String::from_utf8(b64.decode(signature.trim()).map_err(|_| "its signature isn't readable".to_string())?).map_err(|e| e.to_string())?;
    let pk = minisign_verify::PublicKey::decode(&key).map_err(|e| e.to_string())?;
    let sig = minisign_verify::Signature::decode(&sig).map_err(|_| "its signature isn't readable".to_string())?;
    pk.verify(data, &sig, false).map_err(|_| "it isn't signed with OmniPulse's key".to_string())
}

/// The latest installer: asked for, downloaded (its progress kept in `st`), checked, kept in the temp folder
fn fetch(st: &Arc<Mutex<State>>) -> Result<std::path::PathBuf, String> {
    let agent = ureq::AgentBuilder::new().timeout_connect(std::time::Duration::from_secs(15)).timeout_read(std::time::Duration::from_secs(60)).user_agent("OmniPulse-Setup/1").build();
    let v: serde_json::Value = agent.get(LATEST).call().map_err(|_| "OmniPulse's download server couldn't be reached. Are you online?".to_string())?
        .into_string().ok().and_then(|s| serde_json::from_str(&s).ok()).ok_or_else(|| "the release information wasn't readable".to_string())?;
    let w = &v["platforms"]["windows-x86_64"];
    let url = w["url"].as_str().filter(|u| FROM.iter().any(|f| u.starts_with(f))).ok_or("the release information wasn't readable")?.to_string();
    let signature = w["signature"].as_str().ok_or("the release has no signature")?.to_string();
    let version = v["version"].as_str().unwrap_or("").to_string();
    { let mut s = st.lock().unwrap(); s.version = version.clone(); s.phase = Phase::Getting; }
    let r = agent.get(&url).call().map_err(|_| "the installer couldn't be downloaded. Are you online?".to_string())?;
    let total = r.header("content-length").and_then(|n| n.parse::<u64>().ok()).unwrap_or(0);
    st.lock().unwrap().total = total;
    let mut body = r.into_reader();
    let mut bytes = Vec::with_capacity(total as usize);
    let mut buf = [0u8; 64 * 1024];
    loop {
        if st.lock().unwrap().cancelled { return Err(String::new()); }
        let n = std::io::Read::read(&mut body, &mut buf).map_err(|_| "the download stopped part way. Are you still online?".to_string())?;
        if n == 0 { break; }
        bytes.extend_from_slice(&buf[..n]);
        st.lock().unwrap().got = bytes.len() as u64;
        if bytes.len() > 500 * 1024 * 1024 { return Err("the installer was far bigger than it should be".into()); }
    }
    st.lock().unwrap().phase = Phase::Checking;
    verify(&bytes, &signature).map_err(|e| format!("the download didn't check out ({e}), so it wasn't run"))?;
    let path = std::env::temp_dir().join(format!("OmniPulse_{}_setup.exe", if version.is_empty() { "latest" } else { &version }));
    std::fs::write(&path, &bytes).map_err(|e| format!("it couldn't be saved to your temp folder: {e}"))?;
    Ok(path)
}

#[cfg(windows)]
mod ui {
    use super::*;
    use windows::core::{w, HRESULT, PCWSTR};
    use windows::Win32::Foundation::{HWND, LPARAM, S_FALSE, S_OK, WPARAM};
    use windows::Win32::UI::Controls::*;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, IDCANCEL, SW_SHOWNORMAL};

    fn wide(s: &str) -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() }
    fn mb(n: u64) -> String { format!("{:.1} MB", n as f64 / 1_048_576.0) }

    // (each tick: its words and its bar, as the download's going; done or failed, it closes)
    unsafe extern "system" fn tick(hwnd: HWND, msg: TASKDIALOG_NOTIFICATIONS, _w: WPARAM, _l: LPARAM, data: isize) -> HRESULT {
        let st = unsafe { &*(data as *const Mutex<State>) };
        if msg == TDN_CREATED {
            unsafe { SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_MARQUEE.0 as u32, Some(WPARAM(1)), Some(LPARAM(30))); }
        }
        if msg == TDN_BUTTON_CLICKED {
            // (Cancel: the download stops; closing itself, done or failed, isn't a cancel)
            let mut s = st.lock().unwrap();
            if !matches!(s.phase, Phase::Ready(_) | Phase::Failed(_)) { s.cancelled = true; }
            return S_OK;
        }
        if msg == TDN_TIMER {
            let s = st.lock().unwrap();
            let text = match &s.phase {
                Phase::Asking => "Finding the latest version...".to_string(),
                Phase::Getting if s.total > 0 => format!("Downloading OmniPulse {}: {} of {}", s.version, mb(s.got), mb(s.total)),
                Phase::Getting => format!("Downloading OmniPulse {}: {}", s.version, mb(s.got)),
                Phase::Checking => "Checking it's OmniPulse's own...".to_string(),
                Phase::Ready(_) | Phase::Failed(_) => {
                    drop(s);
                    unsafe { SendMessageW(hwnd, TDM_CLICK_BUTTON.0 as u32, Some(WPARAM(IDCANCEL.0 as usize)), Some(LPARAM(0))); }
                    return S_FALSE;
                }
            };
            let (got, total, getting) = (s.got, s.total, matches!(s.phase, Phase::Getting));
            drop(s);
            let t = wide(&text);
            unsafe {
                SendMessageW(hwnd, TDM_SET_ELEMENT_TEXT.0 as u32, Some(WPARAM(TDE_CONTENT.0 as usize)), Some(LPARAM(t.as_ptr() as isize)));
                if getting && total > 0 {
                    SendMessageW(hwnd, TDM_SET_MARQUEE_PROGRESS_BAR.0 as u32, Some(WPARAM(0)), Some(LPARAM(0)));
                    SendMessageW(hwnd, TDM_SET_PROGRESS_BAR_POS.0 as u32, Some(WPARAM(((got * 100) / total.max(1)) as usize)), Some(LPARAM(0)));
                }
            }
        }
        S_OK
    }

    /// The progress window, while `st`'s download runs: it closes itself when it's done (or failed)
    pub fn progress(st: &Arc<Mutex<State>>) {
        let cfg = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            dwFlags: TDF_SHOW_MARQUEE_PROGRESS_BAR | TDF_CALLBACK_TIMER | TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
            dwCommonButtons: TDCBF_CANCEL_BUTTON,
            pszWindowTitle: w!("OmniPulse Setup"),
            pszMainInstruction: w!("Getting OmniPulse"),
            pszContent: w!("Finding the latest version..."),
            pfCallback: Some(tick),
            lpCallbackData: Arc::as_ptr(st) as isize,
            cxWidth: 260,
            ..Default::default()
        };
        unsafe { let _ = TaskDialogIndirect(&cfg, None, None, None); }
    }

    /// Something went wrong: Try Again (true), or Download It Directly (the releases page), or Close
    pub fn failed(why: &str) -> bool {
        let text = wide(&format!("{why}\n\nYou can try again, or download the installer directly instead."));
        let (again, direct) = (wide("Try Again"), wide("Download It Directly"));
        let buttons = [TASKDIALOG_BUTTON { nButtonID: 100, pszButtonText: PCWSTR(again.as_ptr()) }, TASKDIALOG_BUTTON { nButtonID: 101, pszButtonText: PCWSTR(direct.as_ptr()) }];
        let cfg = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION,
            dwCommonButtons: TDCBF_CLOSE_BUTTON,
            pszWindowTitle: w!("OmniPulse Setup"),
            pszMainInstruction: w!("OmniPulse couldn't be downloaded"),
            pszContent: PCWSTR(text.as_ptr()),
            cButtons: buttons.len() as u32,
            pButtons: buttons.as_ptr(),
            nDefaultButton: 100,
            Anonymous1: TASKDIALOGCONFIG_0 { pszMainIcon: TD_WARNING_ICON },
            ..Default::default()
        };
        let mut picked = 0i32;
        unsafe { let _ = TaskDialogIndirect(&cfg, Some(&mut picked), None, None); }
        if picked == 101 {
            let url = wide(RELEASES);
            unsafe { ShellExecuteW(None, w!("open"), PCWSTR(url.as_ptr()), None, None, SW_SHOWNORMAL); }
        }
        picked == 100
    }
}

fn main() {
    // (a test: OMNIPULSE_SETUP_TEST=<file>: the latest fetched and checked, the result written there; no window,
    // and nothing run)
    if let Some(out) = std::env::var_os("OMNIPULSE_SETUP_TEST") {
        let st = Arc::new(Mutex::new(State::default()));
        let r = fetch(&st);
        let v = st.lock().unwrap().version.clone();
        let _ = std::fs::write(out, match r { Ok(p) => format!("ok {v} {} {}", p.display(), std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0)), Err(e) => format!("err {e}") });
        return;
    }
    loop {
        let st = Arc::new(Mutex::new(State::default()));
        let worker = { let st = st.clone(); std::thread::spawn(move || { let r = fetch(&st); st.lock().unwrap().phase = match r { Ok(p) => Phase::Ready(p), Err(e) => Phase::Failed(e) }; }) };
        #[cfg(windows)]
        ui::progress(&st);
        // (closed: Cancel, or done)
        if st.lock().unwrap().cancelled { return; }
        let _ = worker.join();
        let phase = st.lock().unwrap().phase.clone();
        match phase {
            // (the installer, as it always looks: it takes it from here)
            Phase::Ready(path) => { let _ = std::process::Command::new(&path).spawn(); return; }
            Phase::Failed(why) if !why.is_empty() => {
                #[cfg(windows)]
                if ui::failed(&why) { continue; }
                return;
            }
            _ => return,
        }
    }
}
