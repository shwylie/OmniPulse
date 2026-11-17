//! The Stream Suite's A File Changing trigger (Streamer.bot's File Watcher): the folders its actions name, looked
//! at every two seconds, each file added, changed or removed since told to the chatbot (which picks the actions
//! whose files match)

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::SystemTime;

/// The folders watched, as the actions name them
static WANTED: Mutex<Vec<String>> = Mutex::new(Vec::new());
static STARTED: std::sync::Once = std::sync::Once::new();

/// A file as it was last seen: when it last changed, and its size
type Seen = HashMap<String, (Option<SystemTime>, u64)>;

/// The folders to watch (none: the watch idles); its thread started the first time, `heard` told of each change
/// (the folder as the action names it, the file's whole place, its name, and "added", "changed" or "removed")
pub fn want(folders: Vec<(String, String)>, heard: impl Fn(String, String, String, &'static str) + Send + 'static) {
    let mut list: Vec<String> = folders.into_iter().map(|(f, _)| f).collect();
    list.sort();
    list.dedup();
    *WANTED.lock().unwrap() = list;
    STARTED.call_once(|| { std::thread::spawn(move || watch(heard)); });
}

/// What's in a folder now (its files only, not the folders inside it); None if it can't be read
fn look(folder: &str) -> Option<Seen> {
    let mut out = Seen::new();
    for e in std::fs::read_dir(folder).ok()?.flatten() {
        let Ok(m) = e.metadata() else { continue };
        if !m.is_file() { continue; }
        out.insert(e.file_name().to_string_lossy().into_owned(), (m.modified().ok(), m.len()));
    }
    Some(out)
}

/// What changed between two looks at a folder: (the file's name, the change), by name
pub fn changes(before: &Seen, now: &Seen) -> Vec<(String, &'static str)> {
    let mut out = Vec::new();
    for (name, v) in now {
        match before.get(name) {
            None => out.push((name.clone(), "added")),
            Some(was) if was != v => out.push((name.clone(), "changed")),
            _ => {}
        }
    }
    for name in before.keys() { if !now.contains_key(name) { out.push((name.clone(), "removed")); } }
    out.sort();
    out
}

fn watch(heard: impl Fn(String, String, String, &'static str)) {
    // (each folder's last look: what was there when it began looking counts as already there)
    let mut last: HashMap<String, Seen> = HashMap::new();
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let wanted = WANTED.lock().unwrap().clone();
        last.retain(|k, _| wanted.contains(k));
        for folder in wanted {
            let Some(now) = look(&folder) else { last.remove(&folder); continue };
            if let Some(before) = last.get(&folder) {
                for (name, change) in changes(before, &now) {
                    let file = std::path::Path::new(&folder).join(&name).to_string_lossy().into_owned();
                    heard(folder.clone(), file, name, change);
                }
            }
            last.insert(folder, now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_folders_files_added_changed_and_removed_are_told_apart() {
        let t = |s: u64| Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s));
        let before: Seen = [("a.txt".to_string(), (t(1), 5)), ("b.txt".to_string(), (t(1), 5)), ("c.txt".to_string(), (t(1), 5))].into();
        let now: Seen = [("a.txt".to_string(), (t(1), 5)), ("b.txt".to_string(), (t(2), 6)), ("d.txt".to_string(), (t(3), 1))].into();
        assert_eq!(changes(&before, &now), vec![("b.txt".to_string(), "changed"), ("c.txt".to_string(), "removed"), ("d.txt".to_string(), "added")]);
    }

    #[test]
    fn a_real_folder_is_looked_at() {
        let dir = std::env::temp_dir().join("omnipulse-filewatch-test");
        let _ = std::fs::create_dir_all(&dir);
        std::fs::write(dir.join("one.txt"), "x").unwrap();
        let first = look(dir.to_str().unwrap()).unwrap();
        std::fs::write(dir.join("two.txt"), "y").unwrap();
        let _ = std::fs::remove_file(dir.join("one.txt"));
        let then = look(dir.to_str().unwrap()).unwrap();
        assert_eq!(changes(&first, &then), vec![("one.txt".to_string(), "removed"), ("two.txt".to_string(), "added")]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
