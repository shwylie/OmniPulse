//! The Stream Suite's desktop steps and triggers: a program starting (Streamer.bot's Process Started), keys
//! pressed for you (its Keyboard Press), a file, program or web address opened (its Run Executable)

use std::collections::HashSet;
use std::sync::Mutex;

/// The programs the Suite's actions wait for: their names, lower case, without ".exe"
static WANTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STARTED: std::sync::Once = std::sync::Once::new();

/// A program's name as it's compared: lower case, no folder, no ".exe"
pub fn plain(name: &str) -> String {
    let n = name.trim().rsplit(['\\', '/']).next().unwrap_or("").to_lowercase();
    n.strip_suffix(".exe").unwrap_or(&n).to_string()
}

/// The programs to watch for (none: the watch idles); its thread started the first time, `heard` told of each
/// one that starts (its name and its whole place)
pub fn want(names: Vec<String>, heard: impl Fn(String, String) + Send + 'static) {
    *WANTED.lock().unwrap() = names.iter().map(|n| plain(n)).filter(|n| !n.is_empty()).collect();
    STARTED.call_once(|| { std::thread::spawn(move || watch(heard)); });
}

#[cfg(windows)]
fn running() -> Vec<(u32, String)> {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = Vec::new();
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut e).is_ok();
        while ok {
            let len = e.szExeFile.iter().position(|c| *c == 0).unwrap_or(e.szExeFile.len());
            out.push((e.th32ProcessID, String::from_utf16_lossy(&e.szExeFile[..len])));
            ok = Process32NextW(snap, &mut e).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    out
}
#[cfg(not(windows))]
fn running() -> Vec<(u32, String)> { Vec::new() }

/// A running program's whole place (C:\...\game.exe), or its name if that can't be read
#[cfg(windows)]
fn place(pid: u32, name: &str) -> String {
    use windows::core::PWSTR;
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return name.to_string() };
        let mut buf = [0u16; 1024];
        let mut size = buf.len() as u32;
        let got = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok();
        let _ = CloseHandle(h);
        if got { String::from_utf16_lossy(&buf[..size as usize]) } else { name.to_string() }
    }
}
#[cfg(not(windows))]
fn place(_: u32, name: &str) -> String { name.to_string() }

static LISTED: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// The programs open now, by name (A to Z, once each), for picking one: Windows' own left out (what's in
/// its folder, and the processes every PC has)
pub fn programs() -> Vec<String> {
    const SKIP: [&str; 18] = ["system", "idle", "registry", "smss", "csrss", "wininit", "winlogon", "services", "lsass", "svchost", "fontdrvhost", "dwm", "conhost", "sihost", "taskhostw", "runtimebroker", "dllhost", "omnipulse-native"];
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| "C:\\Windows".into()).to_lowercase();
    let mut seen = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for (pid, name) in running() {
        let n = plain(&name);
        if n.is_empty() || SKIP.contains(&n.as_str()) || !seen.insert(n.clone()) { continue; }
        let at = place(pid, &name).to_lowercase();
        // (Windows' own, and the ones it won't say where they are: not a program you opened)
        if at.starts_with(&win) || !at.contains('\\') { continue; }
        out.push(n);
    }
    out.sort();
    *LISTED.lock().unwrap() = out.clone();
    out
}

/// The list as it was last shown, for the one picked from it
pub fn programs_listed() -> Vec<String> { LISTED.lock().unwrap().clone() }

/// Every three seconds, while there's something to wait for: what's started since (not what was running
/// when it began looking, as Streamer.bot's)
fn watch(heard: impl Fn(String, String)) {
    let mut seen: Option<HashSet<u32>> = None;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3));
        let wanted = WANTED.lock().unwrap().clone();
        if wanted.is_empty() { seen = None; continue; }
        let now = running();
        if let Some(before) = &seen {
            for (pid, name) in &now {
                if before.contains(pid) { continue; }
                if wanted.contains(&plain(name)) { heard(plain(name), place(*pid, name)); }
            }
        }
        seen = Some(now.into_iter().map(|x| x.0).collect());
    }
}

/// A key's Windows code, by its name (A, 5, F1, Space, PageUp...)
fn vk(name: &str) -> Option<u16> {
    let n = name.trim().to_lowercase();
    let b = n.as_bytes();
    if b.len() == 1 && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) { return Some(b[0].to_ascii_uppercase() as u16); }
    if let Some(f) = n.strip_prefix('f').and_then(|x| x.parse::<u16>().ok()).filter(|x| (1..=24).contains(x)) { return Some(0x6F + f); }
    if let Some(d) = n.strip_prefix("numpad").and_then(|x| x.parse::<u16>().ok()).filter(|x| *x <= 9) { return Some(0x60 + d); }
    Some(match n.as_str() {
        "space" => 0x20, "enter" | "return" => 0x0D, "tab" => 0x09, "esc" | "escape" => 0x1B, "backspace" => 0x08,
        "home" => 0x24, "end" => 0x23, "pageup" => 0x21, "pagedown" => 0x22, "insert" => 0x2D, "delete" | "del" => 0x2E,
        "up" => 0x26, "down" => 0x28, "left" => 0x25, "right" => 0x27, "printscreen" => 0x2C, "pause" => 0x13,
        "playpause" | "mediaplaypause" => 0xB3, "nexttrack" | "medianext" => 0xB0, "prevtrack" | "mediaprevious" => 0xB1,
        "volumeup" => 0xAF, "volumedown" => 0xAE, "volumemute" | "mute" => 0xAD,
        ";" => 0xBA, "=" => 0xBB, "," => 0xBC, "-" => 0xBD, "." => 0xBE, "/" => 0xBF, "`" => 0xC0, "[" => 0xDB, "\\" => 0xDC, "]" => 0xDD, "'" => 0xDE,
        _ => return None,
    })
}

/// A key's name, by its Windows code: Streamer.bot keeps its Keyboard Press as the code
pub fn key_name(code: u64) -> String {
    let c = code as u16;
    match c {
        0x30..=0x39 | 0x41..=0x5A => (c as u8 as char).to_string(),
        0x70..=0x87 => format!("F{}", c - 0x6F),
        0x60..=0x69 => format!("Numpad{}", c - 0x60),
        _ => ["Space", "Enter", "Tab", "Esc", "Backspace", "Home", "End", "PageUp", "PageDown", "Insert", "Delete", "Up", "Down", "Left", "Right", "PrintScreen", "Pause", "PlayPause", "NextTrack", "PrevTrack", "VolumeUp", "VolumeDown", "Mute"]
            .into_iter().find(|n| vk(n) == Some(c)).map(String::from).unwrap_or_else(|| format!("Key{c}")),
    }
}

/// Keys pressed for you, as "Ctrl+Shift+V": the held ones down, the key pressed, then all let go
#[cfg(windows)]
pub fn press(keys: &str) -> Result<(), String> {
    use windows::Win32::UI::Input::KeyboardAndMouse::{SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP, VIRTUAL_KEY};
    let parts: Vec<&str> = keys.split('+').map(|x| x.trim()).filter(|x| !x.is_empty()).collect();
    let Some((last, held)) = parts.split_last() else { return Err("no keys to press".into()) };
    let mut codes = Vec::new();
    for h in held {
        codes.push(match h.to_lowercase().as_str() { "ctrl" | "control" => 0x11, "shift" => 0x10, "alt" => 0x12, "win" | "windows" | "super" => 0x5B, other => vk(other).ok_or_else(|| format!("there's no key called {h}"))? });
    }
    codes.push(vk(last).ok_or_else(|| format!("there's no key called {last}"))?);
    let key = |c: u16, up: bool| INPUT { r#type: INPUT_KEYBOARD, Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(c), wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } } };
    let mut all: Vec<INPUT> = codes.iter().map(|c| key(*c, false)).collect();
    all.extend(codes.iter().rev().map(|c| key(*c, true)));
    let sent = unsafe { SendInput(&all, std::mem::size_of::<INPUT>() as i32) };
    if sent as usize == all.len() { Ok(()) } else { Err("Windows didn't take the key presses".into()) }
}
#[cfg(not(windows))]
pub fn press(_: &str) -> Result<(), String> { Err("only on Windows".into()) }

/// A web address, a file or a program opened, as double-clicking it would; a program's own words after it
pub fn open(target: &str, args: &str) -> Result<(), String> {
    let t = target.trim().trim_matches('"');
    if t.is_empty() { return Err("nothing to open".into()); }
    let web = t.starts_with("http://") || t.starts_with("https://");
    if !web && !std::path::Path::new(t).exists() { return Err(format!("there's no {t} on this PC")); }
    let program = !web && t.to_lowercase().ends_with(".exe");
    let r = if program {
        let mut c = std::process::Command::new(t);
        c.args(args.split_whitespace());
        if let Some(dir) = std::path::Path::new(t).parent() { c.current_dir(dir); }
        c.spawn().map(|_| ())
    } else {
        std::process::Command::new("explorer").arg(t).spawn().map(|_| ())
    };
    r.map_err(|e| format!("couldn't open {t}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_and_names() {
        assert_eq!(plain("C:\\Games\\DestinyLauncher.EXE"), "destinylauncher");
        assert_eq!(key_name(86), "V");
        assert_eq!(key_name(0x70), "F1");
        assert_eq!(key_name(0x21), "PageUp");
        assert_eq!(vk("f13"), Some(0x7C));
        assert_eq!(vk("Numpad5"), Some(0x65));
    }
}
