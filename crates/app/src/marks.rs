// Likes and favourites, from the native app's player bar (the heart and the star):
//   - a like goes to the track's own platform: SoundCloud, Spotify (/me/library); YouTube Music's are
//     kept on this PC (as the web app keeps them), in state/native-yt-likes.json
//   - favourites are the web app's (sc.favourites, from its copy), with what's starred or unstarred
//     here on top, in state/native-favourites.json. Starring also likes, as in the web app.

use crate::track::{Src, Track};
use serde_json::{json, Value};

fn file(name: &str) -> Option<std::path::PathBuf> {
    omnipulse_core::data_dir().ok().map(|d| d.join("state").join(name))
}

fn read(name: &str) -> Value {
    file(name).and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or(Value::Null)
}

fn write(name: &str, v: &Value) {
    if let Some(p) = file(name) {
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(p, v.to_string());
    }
}

/// Likes or unlikes a track where it's from
pub async fn set_like(t: &Track, on: bool) -> Result<(), String> {
    match &t.src {
        Src::SoundCloud(id) => omnipulse_core::soundcloud::set_like(id.to_string(), on).await,
        Src::Spotify(uri) => {
            let q = format!("/me/library?uris={}", crate::sources::urlencode(uri));
            omnipulse_core::spotify::spotify_api(if on { "PUT" } else { "DELETE" }.into(), q, None).await.map(|_| ())
        }
        Src::Plugin(..) => Err("a plugin's track can't be liked (favourite it instead)".into()),
        Src::MusicBrainz(_) => Err("a MusicBrainz song can't be liked: it's only played from YouTube Music".into()),
        Src::YouTube(_) => {
            let mut v = read("native-yt-likes.json");
            let mut list: Vec<Value> = v.as_array().cloned().unwrap_or_default();
            list.retain(|x| x["id"].as_str() != Some(t.key.as_str()));
            if on { list.insert(0, t.to_snapshot()); }
            v = Value::Array(list);
            write("native-yt-likes.json", &v);
            Ok(())
        }
        Src::Local(_) => Err("your own files can't be liked".into()),
    }
}

/// YouTube Music likes made here (they go in the Library too)
pub fn yt_likes() -> Vec<Track> {
    read("native-yt-likes.json").as_array().map(|a| a.iter().filter_map(Track::from_snapshot).collect()).unwrap_or_default()
}

/// The favourites: the web app's, with what's starred and unstarred here
pub fn favourites(web: &[Track]) -> Vec<Track> {
    let v = read("native-favourites.json");
    let removed: Vec<String> = v["removed"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    let mut out: Vec<Track> = v["added"].as_array().map(|a| a.iter().filter_map(Track::from_snapshot).collect()).unwrap_or_default();
    for t in web {
        if !removed.contains(&t.key) && !out.iter().any(|x| x.key == t.key) {
            out.push(t.clone());
        }
    }
    out
}

pub fn set_favourite(t: &Track, on: bool) {
    let mut v = read("native-favourites.json");
    if !v.is_object() { v = json!({ "added": [], "removed": [] }); }
    let mut added: Vec<Value> = v["added"].as_array().cloned().unwrap_or_default();
    let mut removed: Vec<Value> = v["removed"].as_array().cloned().unwrap_or_default();
    added.retain(|x| x["id"].as_str() != Some(t.key.as_str()));
    removed.retain(|x| x.as_str() != Some(t.key.as_str()));
    if on { added.insert(0, t.to_snapshot()); } else { removed.push(json!(t.key)); }
    write("native-favourites.json", &json!({ "added": added, "removed": removed }));
}
