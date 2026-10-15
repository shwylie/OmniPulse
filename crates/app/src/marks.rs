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
            // (through Spotify's own library service when signed in that way; else the Web API)
            let done = omnipulse_core::spotify_engine::spe_signed_in()
                && omnipulse_core::spotify_library::spe_set_liked(uri.clone(), on).await
                    .map_err(|e| omnipulse_core::diag::note(&format!("like: Spotify's library failed: {e}: the Web API instead"))).is_ok();
            if !done {
                let q = format!("/me/library?uris={}", crate::sources::urlencode(uri));
                omnipulse_core::spotify::spotify_api(if on { "PUT" } else { "DELETE" }.into(), q, None).await?;
            }
            // (and in this PC's copy of your likes, at once)
            crate::sources::spotify_liked_note(t, on);
            Ok(())
        }
        Src::Plugin(..) => Err("a plugin's track can't be liked (favourite it instead)".into()),
        // a catalogue's song (Deezer's, Apple Music's, MusicBrainz's) plays from YouTube Music: its like
        // is kept with YouTube Music's, here (as the web app keeps them)
        Src::YouTube(_) | Src::Catalog(_) => {
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

/// With your YouTube account connected (the web app's ytRate): a YouTube Music song liked there too; and
/// with Like Other Tracks There Too (ytMirror), a SoundCloud or Spotify song's same song on YouTube, its
/// video kept (state/native-yt-mirror.json) so unliking takes the same one off. Why not, if it couldn't.
pub async fn youtube_too(t: &Track, on: bool, mirror: bool) -> Option<String> {
    if omnipulse_core::yt_account::status()["connected"].as_bool() != Some(true) { return None; }
    let (artist, title) = crate::versions::guess_names(t);
    let r = match &t.src {
        Src::YouTube(id) => omnipulse_core::yt_account::rate(Some(id.clone()), &title, &artist, on).await.map(|_| ()),
        Src::SoundCloud(_) | Src::Spotify(_) if mirror => {
            let mut ids = read("native-yt-mirror.json");
            let was = ids[&t.key].as_str().map(String::from);
            match omnipulse_core::yt_account::rate(was, &title, if artist.is_empty() { &t.artist } else { &artist }, on).await {
                Ok(id) => {
                    if !ids.is_object() { ids = json!({}); }
                    if on { ids[&t.key] = json!(id); } else if let Some(o) = ids.as_object_mut() { o.remove(&t.key); }
                    write("native-yt-mirror.json", &ids);
                    Ok(())
                }
                Err(e) => Err(e),
            }
        }
        _ => return None,
    };
    r.err().map(|e| format!("{} here, but not on YouTube: {e}", if on { "Liked" } else { "Unliked" }))
}

/// YouTube Music likes made here (they go in the Library too)
/// The YouTube Music artists you follow (YouTube Music has no follows for an app: they're kept on this
/// PC, as the web app's sc.ytFollows): theirs from the web app, with what's followed or unfollowed here
/// on top, in state/native-yt-follows.json. (channel id, name, picture)
pub fn yt_follows(web: &Value) -> Vec<(String, String, String)> {
    let v = read("native-yt-follows.json");
    let list = if v.is_array() { v } else { web.clone() };
    list.as_array().into_iter().flatten().filter_map(|x| Some((x["id"].as_str()?.to_string(), x["name"].as_str().unwrap_or("").to_string(), x["img"].as_str().unwrap_or("").to_string()))).collect()
}

pub fn set_yt_follow(web: &Value, id: &str, name: &str, img: &str, on: bool) {
    let mut list: Vec<Value> = yt_follows(web).into_iter().filter(|x| x.0 != id).map(|x| json!({ "id": x.0, "name": x.1, "img": x.2 })).collect();
    if on { list.insert(0, json!({ "id": id, "name": name, "img": img })); }
    write("native-yt-follows.json", &Value::Array(list));
}

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
