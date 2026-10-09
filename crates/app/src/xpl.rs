// Playlists on this PC (as the web app's js/xplaylists.js): they hold tracks from any platform and
// are never sent anywhere. Each keeps a small snapshot of its tracks (the web app's saved form), so
// they show and play without asking the platform. This app keeps its own copy, in
// state/native-xplaylists.json, started from the web app's the first time.

use crate::sources::Playlist;
use crate::track::Track;
use serde_json::{json, Value};

fn file() -> Option<std::path::PathBuf> {
    let d = omnipulse_core::data_dir().ok()?.join("state");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("native-xplaylists.json"))
}

/// Every playlist on this PC, as [{ id, title, tracks: [snapshot], created }]
pub fn load() -> Value {
    if let Some(v) = file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str::<Value>(&t).ok()).filter(Value::is_array) {
        return v;
    }
    // the first time: the web app's
    omnipulse_core::load_web_export()
        .and_then(|v| v.get("sc.xplaylists").and_then(|s| s.as_str()).and_then(|s| serde_json::from_str::<Value>(s).ok()))
        .filter(Value::is_array)
        .unwrap_or_else(|| json!([]))
}

pub fn save(all: &Value) {
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() {
        return;
    }
    if let (Some(p), Ok(text)) = (file(), serde_json::to_string(all)) {
        let tmp = p.with_extension("json.tmp");
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &p);
        }
    }
}

/// As the sidebar and the pages take them
pub fn playlists(all: &Value) -> Vec<Playlist> {
    all.as_array().into_iter().flatten().map(|p| Playlist {
        plat: "local",
        id: p["id"].as_str().unwrap_or("").to_string(),
        title: p["title"].as_str().unwrap_or("Untitled playlist").to_string(),
        tracks: Some(p["tracks"].as_array().map(|t| t.iter().filter_map(Track::from_snapshot).collect()).unwrap_or_default()),
        own: true,
    }).collect()
}

pub fn is_own(id: &str) -> bool {
    id.starts_with("xpl:")
}

pub fn snap(t: &Track) -> Value {
    let mut s = t.to_snapshot();
    // a file's address changes from run to run: it's found again by its id
    if t.plat() == "local" {
        if let Some(o) = s.as_object_mut() { o.remove("artwork_url"); }
    }
    s
}

fn find<'a>(all: &'a mut Value, id: &str) -> Option<&'a mut Value> {
    all.as_array_mut()?.iter_mut().find(|p| p["id"].as_str() == Some(id))
}

/// A new playlist (first in the list, as the web app's); its id
pub fn create(all: &mut Value, title: &str, tracks: &[Track]) -> String {
    let now = omnipulse_core::now_secs();
    let n = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    let id = format!("xpl:{now:x}{n:x}");
    let p = json!({
        "id": id,
        "title": if title.trim().is_empty() { "Untitled playlist" } else { title.trim() },
        "tracks": tracks.iter().map(snap).collect::<Vec<_>>(),
        "created": now * 1000,
    });
    if !all.is_array() { *all = json!([]); }
    if let Some(a) = all.as_array_mut() { a.insert(0, p); }
    id
}

/// Adds what isn't in it already; how many went in
pub fn add(all: &mut Value, id: &str, tracks: &[Track]) -> usize {
    let Some(p) = find(all, id) else { return 0 };
    if !p["tracks"].is_array() { p["tracks"] = json!([]); }
    let list = p["tracks"].as_array_mut().expect("an array");
    let mut have: std::collections::HashSet<String> = list.iter().map(|s| s["id"].as_str().map(String::from).unwrap_or_else(|| s["id"].to_string())).collect();
    let fresh: Vec<Value> = tracks.iter().filter(|t| have.insert(t.key.clone())).map(snap).collect();
    let n = fresh.len();
    list.extend(fresh);
    n
}

pub fn remove(all: &mut Value, id: &str, key: &str) {
    if let Some(list) = find(all, id).and_then(|p| p["tracks"].as_array_mut()) {
        list.retain(|s| s["id"].as_str().map(String::from).unwrap_or_else(|| s["id"].to_string()) != key);
    }
}

pub fn rename(all: &mut Value, id: &str, title: &str) {
    if let Some(p) = find(all, id) {
        if !title.trim().is_empty() { p["title"] = json!(title.trim()); }
    }
}

pub fn delete(all: &mut Value, id: &str) {
    if let Some(a) = all.as_array_mut() { a.retain(|p| p["id"].as_str() != Some(id)); }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::Src;

    fn t(key: &str) -> Track {
        Track { src: Src::YouTube(key.into()), key: format!("yt:{key}"), title: key.into(), artist: "A".into(), ..Default::default() }
    }

    #[test]
    fn make_fill_rename_and_delete() {
        let mut all = json!([]);
        let id = create(&mut all, "  Road trip ", &[t("a")]);
        assert!(is_own(&id));
        assert_eq!(add(&mut all, &id, &[t("a"), t("b"), t("b")]), 1);
        let pl = playlists(&all);
        assert_eq!((pl[0].title.as_str(), pl[0].tracks.as_ref().unwrap().len()), ("Road trip", 2));
        remove(&mut all, &id, "yt:a");
        rename(&mut all, &id, "Drive");
        let pl = playlists(&all);
        assert_eq!((pl[0].title.as_str(), pl[0].tracks.as_ref().unwrap()[0].key.as_str()), ("Drive", "yt:b"));
        delete(&mut all, &id);
        assert!(playlists(&all).is_empty());
    }
}
