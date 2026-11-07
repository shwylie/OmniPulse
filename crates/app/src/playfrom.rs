// Play From (Settings > Plugins): the sources in your order. A song about to play is looked for in the
// sources above its own, and the first that has it plays it instead: your own files, or a plugin with the
// song in better quality, over a stream of it. The song stays as it is in your Library and the queue (its
// title, art and likes); only what's heard comes from elsewhere. Found once per song while the app runs.
//
// The order (prefs.playFrom: ids) lists the app's own sources, "local", "soundcloud" and "youtube", and
// each plugin that plays ("plug:<id>"). Sources missing from it come after, built-ins first. By default
// nothing changes: a song plays from where it is.

use crate::track::{Src, Track};
use crate::versions;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

pub const BUILT_IN: [(&str, &str); 3] = [("local", "Your Files"), ("soundcloud", "SoundCloud"), ("youtube", "YouTube Music")];
/// a match good enough to play instead (versions::score: words, the artist, the length, version tags)
const GOOD: f32 = 0.5;

/// What's been decided (by the song's key): the one that plays for it, or None for "itself"
static AS: Mutex<Option<HashMap<String, Option<Track>>>> = Mutex::new(None);

/// Every source that can play, in your order: (id, name)
pub fn sources(prefs: &Value) -> Vec<(String, String)> {
    let mut all: Vec<(String, String)> = BUILT_IN.iter().map(|(id, n)| (id.to_string(), n.to_string())).collect();
    for p in crate::plugrun::players() {
        all.push((format!("plug:{}", p.id), p.name.clone()));
    }
    let order = order(prefs, &all);
    order.into_iter().filter_map(|id| all.iter().find(|(i, _)| *i == id).cloned()).collect()
}

/// The ids in order: yours first (those still here), then the rest as `all` has them
fn order(prefs: &Value, all: &[(String, String)]) -> Vec<String> {
    let mut out: Vec<String> = prefs["playFrom"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).filter(|id| all.iter().any(|(i, _)| i == id)).collect();
    for (id, _) in all {
        if !out.contains(id) { out.push(id.clone()); }
    }
    out
}

/// One moved up a place: the new order to keep (every source listed, so it stays as you see it)
pub fn moved_up(prefs: &Value, id: &str) -> Vec<String> {
    let mut ids: Vec<String> = sources(prefs).into_iter().map(|(i, _)| i).collect();
    if let Some(i) = ids.iter().position(|x| x == id).filter(|i| *i > 0) {
        ids.swap(i, i - 1);
    }
    forget_all();
    ids
}

/// Where a song plays from of itself: its own source, or YouTube Music for one that plays as its
/// YouTube Music version (Spotify's, the catalogues')
pub fn source_of(t: &Track) -> String {
    match &t.src {
        Src::Local(_) => "local".into(),
        Src::SoundCloud(_) if !t.go_plus => "soundcloud".into(),
        Src::Plugin(id, _) => format!("plug:{id}"),
        _ => "youtube".into(),
    }
}

/// The one that plays for this song, once decided (itself: the song as it is)
pub fn as_played(t: &Track) -> Track {
    AS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&t.key).cloned().flatten()).unwrap_or_else(|| t.clone())
}

pub fn played_as(key: &str) -> Option<Track> {
    AS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(key).cloned().flatten())
}

pub fn forget_all() {
    *AS.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

fn decided(key: &str) -> Option<Option<Track>> {
    AS.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(key).cloned())
}

fn decide(key: &str, what: Option<Track>) {
    AS.lock().unwrap_or_else(|e| e.into_inner()).get_or_insert_with(HashMap::new).insert(key.to_string(), what);
}

/// A candidate as the matcher reads one
fn as_yt(c: &Track) -> omnipulse_core::youtube::YtTrack {
    omnipulse_core::youtube::YtTrack { id: c.key.clone(), title: c.title.clone(), artists: c.artist.clone(), album: c.album.clone(), duration_ms: c.ms, ..Default::default() }
}

/// The best of these for the song, if one's close enough
pub fn best(t: &Track, found: Vec<Track>) -> Option<Track> {
    let mut ranked: Vec<(f32, Track)> = found.into_iter().filter(|c| c.key != t.key).map(|c| (versions::score(t, &as_yt(&c)), c)).collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    ranked.into_iter().find(|(s, c)| *s >= GOOD && versions::artist_hit(t, &as_yt(c))).map(|(_, c)| c)
}

/// The song looked for in the sources above its own (`locals`: your files, for "local"); the first that
/// has it is kept for the song and given back. Itself: None. Decided once.
pub async fn find(t: &Track, prefs: &Value, locals: &[Track]) -> Option<Track> {
    if let Some(d) = decided(&t.key) { return d; }
    let own = source_of(t);
    let ids: Vec<String> = sources(prefs).into_iter().map(|(i, _)| i).take_while(|i| *i != own).collect();
    if ids.is_empty() || t.is_mix() {
        decide(&t.key, None);
        return None;
    }
    let (artist, title) = versions::guess_names(t);
    let q = format!("{artist} {title}").trim().to_string();
    let mut got: Option<Track> = None;
    for id in ids {
        let found: Vec<Track> = match id.as_str() {
            "local" => locals.to_vec(),
            "soundcloud" if omnipulse_core::soundcloud::signed_in() => crate::sources::sc_search(&q, 8).await,
            "soundcloud" => Vec::new(),
            "youtube" => match versions::find(t).await { Some(v) if v.exact => vec![Track::youtube(&v.yt)], _ => Vec::new() },
            p => match p.strip_prefix("plug:").and_then(crate::plugrun::get) {
                Some(plug) => plug.call("search", vec![serde_json::json!(q)]).await.map(|v| v.as_array().cloned().unwrap_or_default().iter().take(30).filter_map(|x| Track::plugin(&plug.id, x)).collect()).unwrap_or_default(),
                None => Vec::new(),
            },
        };
        if let Some(c) = best(t, found) {
            omnipulse_core::diag::note(&format!("Play From: {} - {} plays from {} ({})", t.artist, t.title, id, c.key));
            got = Some(c);
            break;
        }
    }
    decide(&t.key, got.clone());
    got
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_order_is_yours_then_the_rest_and_moves() {
        let all = vec![("local".to_string(), "Your Files".to_string()), ("soundcloud".into(), "SoundCloud".into()), ("youtube".into(), "YouTube Music".into())];
        assert_eq!(order(&json!({}), &all), vec!["local", "soundcloud", "youtube"]);
        assert_eq!(order(&json!({ "playFrom": ["youtube", "plug:gone", "local"] }), &all), vec!["youtube", "local", "soundcloud"]);
        let p = json!({ "playFrom": ["local", "soundcloud", "youtube"] });
        assert_eq!(moved_up(&p, "youtube"), vec!["local", "youtube", "soundcloud"]);
        assert_eq!(moved_up(&p, "local"), vec!["local", "soundcloud", "youtube"]);
    }

    #[test]
    fn the_best_is_the_same_song_close_enough() {
        let want = Track { key: "sp:1".into(), src: Src::Spotify("1".into()), title: "Strobe".into(), artist: "deadmau5".into(), ms: 636_000, ..Default::default() };
        let strobe = Track::plugin("p", &json!({ "id": "a", "title": "Strobe", "artist": "deadmau5", "album": "For Lack of a Better Name", "duration_ms": 635_000 })).unwrap();
        let edit = Track::plugin("p", &json!({ "id": "b", "title": "Strobe (Radio Edit)", "artist": "deadmau5", "duration_ms": 220_000 })).unwrap();
        let other = Track::plugin("p", &json!({ "id": "c", "title": "Ghosts n Stuff", "artist": "deadmau5", "duration_ms": 200_000 })).unwrap();
        assert_eq!(best(&want, vec![other.clone(), edit.clone(), strobe.clone()]).map(|c| c.key), Some("plug:p:a".to_string()));
        assert!(best(&want, vec![other, edit]).is_none());
        assert_eq!(source_of(&want), "youtube");
        assert_eq!(source_of(&strobe), "plug:p");
        // (decided once, and itself till then)
        assert_eq!(as_played(&want).key, "sp:1");
        decide(&want.key, Some(strobe.clone()));
        assert_eq!(as_played(&want).key, "plug:p:a");
        forget_all();
        assert_eq!(as_played(&want).key, "sp:1");
    }
}
