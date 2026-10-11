// The diagnostic log: diag.log in the data folder. What the app was doing, step by step (Spotify's
// sign-in, say), with the time; the Spotify playback library's own warnings and errors (librespot logs
// through the log crate, which went nowhere); and whatever's written to the error output, which an app
// without a console loses: Rust's own last words when a thread overflows its stack (it aborts before
// any panic hook) go there. So an app that just closes, with nothing in crash.log and nothing in
// Windows' error log, leaves its last steps here. Started afresh past 300 KB (the last as diag.old.log).

use std::io::Write;
use std::sync::Mutex;

static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
const KEEP: u64 = 300_000;

struct Logger;

impl log::Log for Logger {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.level() <= log::Level::Warn || (m.target().starts_with("librespot") && m.level() <= log::Level::Info)
    }
    fn log(&self, r: &log::Record) {
        if self.enabled(r.metadata()) { note(&format!("[{} {}] {}", r.level(), r.target(), r.args())); }
    }
    fn flush(&self) {}
}

static LOGGER: Logger = Logger;

/// Opens the log (once, at the start), and sends the error output and the log crate's messages to it
pub fn install() {
    let Ok(dir) = crate::data_dir() else { return };
    let path = dir.join("diag.log");
    if std::fs::metadata(&path).map(|m| m.len() > KEEP).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("diag.old.log"));
    }
    let Ok(file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) else { return };
    #[cfg(windows)]
    if let Ok(err) = file.try_clone() {
        use std::os::windows::io::IntoRawHandle;
        let h = windows::Win32::Foundation::HANDLE(err.into_raw_handle());
        unsafe { let _ = windows::Win32::System::Console::SetStdHandle(windows::Win32::System::Console::STD_ERROR_HANDLE, h); }
    }
    if let Ok(mut g) = FILE.lock() { *g = Some(file); }
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
    note(&format!("---- started, version {}", env!("CARGO_PKG_VERSION")));
}

/// A step, written down at once (with the time: unix seconds and milliseconds)
pub fn note(msg: &str) {
    let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    if let Ok(mut g) = FILE.lock() {
        if let Some(f) = g.as_mut() {
            let _ = writeln!(f, "{}.{:03} {}", t.as_secs(), t.subsec_millis(), msg);
            let _ = f.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn steps_and_the_error_output_go_in() {
        let _turn = crate::TEST_ENV.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::temp_dir().join(format!("omnipulse-diag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        crate::set_data_dir(dir.clone());
        super::install();
        super::note("a step");
        log::warn!(target: "librespot_core::session", "a library's warning");
        // (what a thread overflowing its stack would say: written to the error output)
        use std::io::Write;
        let _ = writeln!(std::io::stderr(), "thread 'x' has overflowed its stack");
        let text = std::fs::read_to_string(dir.join("diag.log")).unwrap();
        assert!(text.contains("---- started") && text.contains("a step") && text.contains("a library's warning"), "{text}");
        assert!(text.contains("has overflowed its stack"), "{text}");
        *crate::DATA_DIR.lock().unwrap() = None;
    }
}
