//! Google Fonts for the widgets' previews: a font picked for a widget is fetched once (kept in the data
//! folder's fonts), then handed to Slint, so its preview shows it as OBS will

use std::cell::RefCell;
use std::collections::HashSet;
use std::sync::Arc;

thread_local! {
    // (handed to Slint: drawn with its name; asked for: being fetched, or failed, so it's not asked again)
    static READY: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    static ASKED: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
}

fn dir() -> Option<std::path::PathBuf> { omnipulse_core::data_dir().ok().map(|d| d.join("fonts")) }

fn file_name(name: &str, i: usize) -> String { format!("{}-{i}.ttf", name.replace(' ', "_")) }

/// Its files, as kept: handed to Slint (on the UI thread)
fn register(name: &str) -> bool {
    let Some(d) = dir() else { return false };
    let mut c = slint::fontique_011::shared_collection();
    let mut any = false;
    for i in 0..3 {
        let Ok(bytes) = std::fs::read(d.join(file_name(name, i))) else { continue };
        any |= !c.register_fonts(slint::fontique_011::fontique::Blob::new(Arc::new(bytes)), None).is_empty();
    }
    any
}

/// Its regular, bold and extra-bold, from Google Fonts (as TrueType: the older address gives those)
async fn fetch(name: &str) -> bool {
    let Some(d) = dir() else { return false };
    let url = format!("https://fonts.googleapis.com/css?family={}:400,700,800", name.replace(' ', "+"));
    let Ok(css) = async { reqwest::get(&url).await?.error_for_status()?.text().await }.await else { return false };
    let urls: Vec<String> = css.split("url(").skip(1).filter_map(|x| x.split(')').next()).filter(|u| u.starts_with("https://fonts.gstatic.com/")).map(String::from).take(3).collect();
    if urls.is_empty() || std::fs::create_dir_all(&d).is_err() { return false; }
    let mut got = false;
    for (i, u) in urls.iter().enumerate() {
        let Ok(b) = async { reqwest::get(u).await?.error_for_status()?.bytes().await }.await else { continue };
        got |= std::fs::write(d.join(file_name(name, i)), &b).is_ok();
    }
    got
}

/// The family to draw a Google Font with: its name once Slint has it, else "" (the app's own) while it's
/// fetched; `then` runs on the UI thread once it's ready, to draw the preview again
pub fn family(rt: &tokio::runtime::Handle, name: &str, then: fn()) -> String {
    // (only a name Google Fonts could have: letters, digits and spaces)
    let name = name.trim();
    if name.is_empty() || name.len() > 60 || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == ' ') { return String::new(); }
    if READY.with(|r| r.borrow().contains(name)) { return name.into(); }
    if !ASKED.with(|a| a.borrow_mut().insert(name.to_string())) { return String::new(); }
    // (kept from before: handed over at once)
    if register(name) { READY.with(|r| r.borrow_mut().insert(name.to_string())); return name.into(); }
    let owned = name.to_string();
    rt.spawn(async move {
        if !fetch(&owned).await { return; }
        let _ = slint::invoke_from_event_loop(move || {
            if register(&owned) { READY.with(|r| r.borrow_mut().insert(owned)); then(); }
        });
    });
    String::new()
}
