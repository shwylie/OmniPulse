// The web app's saved data (favourites, stats, its own playlists, YouTube Music likes, play history),
// from the copy it writes to the data folder (dist/js/export.js), plus the plays this app counts
// itself (state/native-stats.json), so Stats and Rediscover see both.

use crate::sources::Playlist;
use crate::track::Track;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Play {
    pub ts: u64,
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub artist: String,
    #[serde(default)]
    pub ms: u64,
    #[serde(default)]
    pub src: String,
    #[serde(default)]
    pub art: String,
    #[serde(default)]
    pub genre: String,
}

#[derive(Default)]
pub struct WebState {
    pub favourites: Vec<Track>,
    pub plays: Vec<Play>,
    pub last_played: HashMap<String, u64>,
    pub own_playlists: Vec<Playlist>,
    pub yt_likes: Vec<Track>,
    pub prefs: Value,
    pub saved_at: u64,
}

/// One of the web app's saved values: stored as a JSON string, parsed
fn key(v: &Value, k: &str) -> Value {
    v.get(k).and_then(|s| s.as_str()).and_then(|s| serde_json::from_str(s).ok()).unwrap_or(Value::Null)
}

fn native_stats_file() -> Option<std::path::PathBuf> {
    let d = omnipulse_core::data_dir().ok()?.join("state");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("native-stats.json"))
}

fn native_prefs_file() -> Option<std::path::PathBuf> {
    let d = omnipulse_core::data_dir().ok()?.join("state");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("native-prefs.json"))
}

fn native_prefs() -> Value {
    native_prefs_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

/// One of this app's own settings (state/native-prefs.json)
pub fn native_pref(key: &str) -> Option<Value> {
    native_prefs().get(key).cloned()
}

/// A setting changed in this app (it wins over the web app's from then on)
pub fn save_pref(key: &str, value: Value) {
    let mut all = native_prefs();
    if !all.is_object() {
        all = Value::Object(Default::default());
    }
    all[key] = value;
    if let (Some(p), Ok(text)) = (native_prefs_file(), serde_json::to_string(&all)) {
        let _ = std::fs::write(p, text);
    }
}

pub fn native_plays() -> Vec<Play> {
    native_stats_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
}

/// A play counted here (30 s, or half the track)
pub fn count_play(t: &Track) {
    let mut list = native_plays();
    list.push(Play {
        ts: omnipulse_core::now_secs() * 1000,
        id: t.key.clone(),
        title: t.title.clone(),
        artist: t.artist.clone(),
        ms: t.ms,
        src: t.plat().into(),
        art: t.art.clone(),
        genre: t.genre.clone(),
    });
    let keep = list.len().saturating_sub(8000);
    if let (Some(p), Ok(text)) = (native_stats_file(), serde_json::to_string(&list[keep..])) {
        let _ = std::fs::write(p, text);
    }
}

pub fn load() -> WebState {
    let mut s = WebState::default();
    if let Some(v) = omnipulse_core::load_web_export() {
        s.saved_at = v["savedAt"].as_u64().unwrap_or(0);
        s.favourites = key(&v, "sc.favourites").as_array().map(|a| a.iter().filter_map(Track::from_snapshot).collect()).unwrap_or_default();
        s.plays = serde_json::from_value(key(&v, "sc.stats")).unwrap_or_default();
        s.yt_likes = key(&v, "sc.ytLikes").as_array().map(|a| a.iter().filter_map(Track::from_snapshot).collect()).unwrap_or_default();
        s.prefs = key(&v, "sc.prefs");
        // the web app's YouTube Music versions of Go+ tracks (crate::versions)
        crate::versions::seed_from_web(&key(&v, "sc.ytMatch"));
        if let Some(a) = key(&v, "sc.lastPlayed").as_array() {
            for e in a {
                if let (Some(id), Some(ts)) = (e.get(0).and_then(|x| x.as_str().map(String::from).or_else(|| x.as_u64().map(|n| n.to_string()))), e.get(1).and_then(|x| x.as_u64())) {
                    s.last_played.insert(id, ts);
                }
            }
        }
        if let Some(a) = key(&v, "sc.xplaylists").as_array() {
            for p in a {
                let tracks: Vec<Track> = p["tracks"].as_array().map(|t| t.iter().filter_map(Track::from_snapshot).collect()).unwrap_or_default();
                s.own_playlists.push(Playlist { plat: "local", id: p["id"].as_str().unwrap_or("").to_string(), title: p["title"].as_str().unwrap_or("Playlist").to_string(), tracks: Some(tracks) });
            }
        }
    }
    // settings changed in this app, over the web app's
    if !s.prefs.is_object() {
        s.prefs = Value::Object(Default::default());
    }
    if let (Some(mine), Some(all)) = (native_prefs().as_object().cloned(), s.prefs.as_object_mut()) {
        all.extend(mine);
    }
    // this app's own plays count too
    for p in native_plays() {
        let e = s.last_played.entry(p.id.clone()).or_insert(0);
        *e = (*e).max(p.ts);
        s.plays.push(p);
    }
    s.plays.sort_by_key(|p| p.ts);
    s
}
