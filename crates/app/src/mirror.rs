// Playlists on this PC, kept on SoundCloud and Spotify too (as the web app's js/xplmirror.js).
//
// Mirrors: every playlist here has a hidden playlist on each platform you're signed in to, holding
// that platform's songs: its own, and the same songs matched from the other platforms. A song from
// Spotify (or YouTube Music, or your files) goes on SoundCloud only as its Go+ upload (the label's
// own); a SoundCloud Go+ song goes on Spotify by the usual close match; any other SoundCloud upload
// goes only when the title, the artist and the length all match.
//
// Follows: a platform playlist you change here becomes a playlist here, and the original is hidden.
// One of yours keeps being written to; anyone else's can't be, so your changes stay here, and what's
// added or taken out over there still comes in.
//
// The same fields as the web app's (mirrors, made, follow, mirrorSig), so both use the same hidden
// playlists, and the same song matches (state/native-xplmatches.json, started from the web app's).

use crate::sources;
use crate::track::{Src, Track};
use crate::versions;
use omnipulse_core as core;
use omnipulse_core::youtube::YtTrack;
use serde_json::{json, Value};
use std::sync::Mutex;

pub const PLATS: [&str; 2] = ["soundcloud", "spotify"];
const MISS_TTL_SECS: u64 = 7 * 86_400;

// ---------- the matches, kept ----------
static MATCHES: Mutex<Option<Value>> = Mutex::new(None);

fn matches_file() -> Option<std::path::PathBuf> {
    let d = core::data_dir().ok()?.join("state");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("native-xplmatches.json"))
}

fn with_matches<R>(f: impl FnOnce(&mut Value) -> R) -> R {
    let mut g = MATCHES.lock().unwrap_or_else(|e| e.into_inner());
    let m = g.get_or_insert_with(|| {
        matches_file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok())
            .or_else(|| core::load_web_export().and_then(|v| v.get("sc.xplMatches").and_then(|s| s.as_str()).and_then(|s| serde_json::from_str(s).ok())))
            .filter(Value::is_object)
            .unwrap_or_else(|| json!({}))
    });
    f(m)
}

fn matches_save() {
    if std::env::var_os("OMNIPULSE_SELFTEST").is_some() {
        return;
    }
    let text = with_matches(|m| m.to_string());
    if let Some(p) = matches_file() {
        let _ = std::fs::write(p, text);
    }
}

// ---------- the same song on the other platform ----------
fn as_yt(c: &Track) -> YtTrack {
    YtTrack { id: c.key.clone(), title: c.title.clone(), artists: c.artist.clone(), duration_ms: c.ms, ..Default::default() }
}

fn names(t: &Track) -> (String, String) {
    let (a, _) = versions::guess_names(t);
    (if a.is_empty() { t.artist.clone() } else { a }, versions::match_title(t))
}

/// Its Go+ upload on SoundCloud (the label's own release of it)
async fn find_sc(t: &Track) -> Option<String> {
    let (artist, title) = names(t);
    let found = sources::sc_search_previews(&format!("{artist} {title}"), 20).await;
    found.iter().filter(|c| c.go_plus).map(|c| (c, versions::score(t, &as_yt(c)))).filter(|(c, s)| *s >= 0.8 && versions::artist_hit(t, &as_yt(c)))
        .max_by(|a, b| a.1.total_cmp(&b.1)).map(|(c, _)| c.key.clone())
}

/// Strict: a SoundCloud upload that isn't Go+ (the title, the artist and the length must all match)
fn sp_ok(t: &Track, c: &Track, s: f32) -> bool {
    let strict = matches!(t.src, Src::SoundCloud(_)) && !t.go_plus;
    if strict { s >= 1.5 && t.ms > 0 && t.ms.abs_diff(c.ms) <= 3000 } else { s >= 0.8 }
}

async fn find_sp(t: &Track) -> Option<String> {
    let (artist, title) = names(t);
    let mut found = Vec::new();
    for q in [format!("track:\"{title}\" artist:\"{artist}\""), format!("{artist} {title}")] {
        found = sources::sp_search(&q).await;
        if !found.is_empty() {
            break;
        }
    }
    found.iter().map(|c| (c, versions::score(t, &as_yt(c)))).filter(|(c, s)| versions::artist_hit(t, &as_yt(c)) && sp_ok(t, c, *s))
        .max_by(|a, b| a.1.total_cmp(&b.1)).and_then(|(c, _)| if let Src::Spotify(uri) = &c.src { Some(uri.clone()) } else { None })
}

/// The song's id on a platform (a SoundCloud id, a Spotify uri), or None if it isn't there
pub async fn counterpart(t: &Track, plat: &str) -> Option<String> {
    match (&t.src, plat) {
        (Src::SoundCloud(id), "soundcloud") => return Some(id.to_string()),
        (Src::Spotify(uri), "spotify") => return Some(uri.clone()),
        _ => {}
    }
    let saved = with_matches(|m| m[&t.key][plat].clone());
    if let Some(id) = saved["id"].as_str().map(String::from).or_else(|| saved["id"].as_u64().map(|n| n.to_string())) {
        return Some(id);
    }
    if saved["none"].as_bool() == Some(true) && core::now_secs().saturating_sub(saved["at"].as_u64().unwrap_or(0) / 1000) < MISS_TTL_SECS {
        return None;
    }
    let found = if plat == "soundcloud" { find_sc(t).await } else { find_sp(t).await };
    with_matches(|m| {
        if !m[&t.key].is_object() { m[&t.key] = json!({}); }
        m[&t.key][plat] = match &found { Some(id) => json!({ "id": id }), None => json!({ "none": true, "at": core::now_secs() * 1000 }) };
    });
    matches_save();
    found
}

// ---------- the platforms' side ----------
async fn create(plat: &str, title: &str) -> Result<String, String> {
    if plat == "soundcloud" {
        let c = core::soundcloud::playlist_write("POST".into(), "/playlists".into(), Some(json!({ "playlist": { "title": title, "sharing": "private", "tracks": [] } }))).await?;
        return c["id"].as_u64().map(|n| n.to_string()).ok_or_else(|| "SoundCloud didn't say what it made".to_string());
    }
    let c = core::spotify::spotify_api("POST".into(), "/me/playlists".into(), Some(json!({ "name": title, "public": false, "description": "Kept in step with OmniPulse." }))).await?;
    c["id"].as_str().map(String::from).ok_or_else(|| "Spotify didn't say what it made".to_string())
}

async fn write(plat: &str, id: &str, title: &str, ids: &[String]) -> Result<(), String> {
    if plat == "soundcloud" {
        let tracks: Vec<Value> = ids.iter().filter_map(|x| x.parse::<u64>().ok()).map(|x| json!({ "id": x })).collect();
        core::soundcloud::playlist_write("PUT".into(), format!("/playlists/{id}"), Some(json!({ "playlist": { "title": title, "tracks": tracks } }))).await?;
        return Ok(());
    }
    core::spotify::spotify_api("PUT".into(), format!("/playlists/{id}"), Some(json!({ "name": title }))).await?;
    core::spotify::spotify_api("PUT".into(), format!("/playlists/{id}/items"), Some(json!({ "uris": ids.iter().take(100).collect::<Vec<_>>() }))).await?;
    for chunk in ids.chunks(100).skip(1) {
        core::spotify::spotify_api("POST".into(), format!("/playlists/{id}/items"), Some(json!({ "uris": chunk }))).await?;
    }
    Ok(())
}

/// A hidden playlist made for one here, gone with it
pub async fn remove(plat: &str, id: &str) {
    let _ = if plat == "soundcloud" {
        core::soundcloud::playlist_write("DELETE".into(), format!("/playlists/{id}"), None).await
    } else {
        core::spotify::spotify_api("DELETE".into(), format!("/me/library?uris={}", sources::urlencode(&format!("spotify:playlist:{id}"))), None).await
    };
}

/// How the sidebar's hidden list knows a platform playlist
pub fn side_id(plat: &str, id: &str) -> String {
    if plat == "spotify" { format!("sp:{id}") } else { id.to_string() }
}

/// What a platform playlist holds, as track keys (None if it couldn't be read: nothing is taken out then)
async fn plat_keys(plat: &'static str, id: &str) -> Option<Vec<Track>> {
    let list = sources::playlist_tracks(&sources::Playlist { plat, id: id.to_string(), title: String::new(), tracks: None, own: false }).await;
    (!list.is_empty()).then_some(list)
}

fn keys_of(p: &Value) -> Vec<String> {
    p["tracks"].as_array().into_iter().flatten().map(|s| s["id"].as_str().map(String::from).unwrap_or_else(|| s["id"].to_string())).collect()
}

/// What changed over there, brought in here; true if anything did
async fn pull(p: &mut Value) -> bool {
    let plat = match p["follow"]["plat"].as_str() { Some("spotify") => "spotify", Some("soundcloud") => "soundcloud", _ => return false };
    let Some(id) = p["follow"]["id"].as_str().map(String::from) else { return false };
    let Some(now) = plat_keys(plat, &id).await else { return false };
    let now_keys: Vec<String> = now.iter().map(|t| t.key.clone()).collect();
    let Some(seen) = p["follow"]["seen"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect::<Vec<_>>()) else {
        p["follow"]["seen"] = json!(now_keys);
        return false;
    };
    p["follow"]["seen"] = json!(now_keys);
    let gone: Vec<&String> = seen.iter().filter(|k| !now_keys.contains(k)).collect();
    let fresh: Vec<&Track> = now.iter().filter(|t| !seen.contains(&t.key)).collect();
    if gone.is_empty() && fresh.is_empty() {
        return false;
    }
    if let Some(list) = p["tracks"].as_array_mut() {
        list.retain(|s| !gone.iter().any(|g| s["id"].as_str().map(String::from).unwrap_or_else(|| s["id"].to_string()) == **g));
    }
    let mut have = keys_of(p);
    for t in fresh {
        if !have.contains(&t.key) {
            have.push(t.key.clone());
            if let Some(list) = p["tracks"].as_array_mut() { list.push(crate::xpl::snap(t)); }
        }
    }
    true
}

#[derive(Default)]
pub struct Outcome {
    /// its songs changed (from the platform's side)
    pub changed: bool,
    /// hidden playlists made for it, to hide (as the sidebar knows them)
    pub hide: Vec<String>,
    pub errors: Vec<String>,
}

/// One playlist, kept in step (on: the platforms signed in and switched on)
pub async fn sync_one(p: &mut Value, on: &[&'static str], with_pull: bool) -> Outcome {
    let mut out = Outcome { changed: with_pull && p["follow"].is_object() && pull(p).await, ..Default::default() };
    let sig = json!([p["title"], keys_of(p), on]).to_string();
    if !out.changed && p["mirrorSig"].as_str() == Some(sig.as_str()) {
        return out;
    }
    let title = p["title"].as_str().unwrap_or("Untitled playlist").to_string();
    let tracks: Vec<Track> = p["tracks"].as_array().into_iter().flatten().filter_map(Track::from_snapshot).collect();
    let mut failed = false;
    for &plat in on {
        let mut ids: Vec<String> = Vec::new();
        for t in &tracks {
            if let Some(id) = counterpart(t, plat).await {
                if !ids.contains(&id) { ids.push(id); }
            }
        }
        let mut id = p["mirrors"][plat].as_str().map(String::from);
        if id.is_none() && ids.is_empty() {
            continue;
        }
        if id.is_none() {
            match create(plat, &title).await {
                Ok(made) => {
                    if !p["mirrors"].is_object() { p["mirrors"] = json!({}); }
                    if !p["made"].is_object() { p["made"] = json!({}); }
                    p["mirrors"][plat] = json!(made);
                    p["made"][plat] = json!(true);
                    out.hide.push(side_id(plat, &made));
                    id = Some(made);
                }
                Err(e) => { failed = true; out.errors.push(fail_note(&title, plat, &e)); continue; }
            }
        }
        let id = id.unwrap_or_default();
        match write(plat, &id, &title, &ids).await {
            Ok(()) => {
                // (what was written there isn't a change from that side)
                if p["follow"]["plat"].as_str() == Some(plat) && p["follow"]["id"].as_str() == Some(id.as_str()) {
                    p["follow"]["seen"] = json!(ids.iter().map(|x| if plat == "spotify" { format!("sp:{}", x.rsplit(':').next().unwrap_or("")) } else { x.clone() }).collect::<Vec<_>>());
                }
            }
            Err(e) => { failed = true; out.errors.push(fail_note(&title, plat, &e)); }
        }
    }
    if !failed {
        p["mirrorSig"] = json!(sig);
    }
    out
}

fn fail_note(title: &str, plat: &str, e: &str) -> String {
    let name = if plat == "spotify" { "Spotify" } else { "SoundCloud" };
    let why = if plat == "spotify" && (e.contains("403") || e.to_lowercase().contains("scope") || e.to_lowercase().contains("insufficient")) {
        "sign in to Spotify again to let OmniPulse keep your playlists there.".to_string()
    } else {
        e.to_string()
    };
    format!("Couldn't keep \"{title}\" up to date on {name}: {why}")
}

/// A platform playlist you changed here: its songs, for a playlist here that follows it (and writes
/// back to it, if it's yours)
pub async fn adopt(p: &sources::Playlist) -> Value {
    let tracks = sources::playlist_tracks(p).await;
    let mut all = json!([]);
    let id = crate::xpl::create(&mut all, &p.title, &tracks);
    let mut copy = all[0].take();
    copy["from"] = json!(side_id(p.plat, &p.id));
    copy["follow"] = json!({ "plat": p.plat, "id": p.id, "seen": tracks.iter().map(|t| t.key.clone()).collect::<Vec<_>>() });
    if p.own {
        copy["mirrors"] = json!({ p.plat: p.id });
    }
    let _ = id;
    copy
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(title: &str, artist: &str, ms: u64, go_plus: bool) -> Track {
        Track { src: Src::SoundCloud(1), key: "1".into(), title: title.into(), artist: artist.into(), ms, go_plus, ..Default::default() }
    }
    fn sp(title: &str, artist: &str, ms: u64) -> Track {
        Track { src: Src::Spotify("spotify:track:x".into()), key: "sp:x".into(), title: title.into(), artist: artist.into(), ms, ..Default::default() }
    }

    #[test]
    fn other_soundcloud_uploads_match_spotify_only_exactly() {
        let mb = sp("Misery Business", "Paramore", 211_500);
        let ok = |t: &Track| { let s = versions::score(t, &as_yt(&mb)); sp_ok(t, &mb, s) };
        assert!(ok(&sc("Paramore - Misery Business", "uploader", 211_000, false)));
        assert!(!ok(&sc("Paramore - Misery Business (MELLIK Remix)", "uploader", 211_000, false)));
        assert!(!ok(&sc("Paramore - Misery Business", "uploader", 230_000, false)));
        // a Go+ one, by the usual close match
        assert!(ok(&sc("Misery Business", "Paramore", 216_000, true)));
    }
}
