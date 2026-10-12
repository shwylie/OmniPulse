// Hidden & Not Recommended (the web app's js/library.js): a track, an album or an artist you've said not
// to recommend is left out of what the app picks for you (Discover, Rediscover, autoplay), and a Go+
// track hidden for having no full version anywhere (noVersion) out of every list. The same prefs as the
// web app's: hideOn (on unless switched off), hiddenRecs (track ids) with hiddenRecNames, hiddenAlbums
// and hiddenArtists (key -> name), noVersion (id -> name).

use crate::track::Track;
use omnipulse_core::ranking::artist_key;
use serde_json::{json, Value};

pub fn on(prefs: &Value) -> bool { prefs["hideOn"].as_bool() != Some(false) }

/// An album's key, as the web app's albumKeyOf: its title without "(Deluxe Edition)"-style tags, and its
/// first artist
pub fn album_key(title: &str, artist: &str) -> String {
    let mut t = title.to_lowercase();
    // (bracketed deluxe / edition / remaster / expanded / bonus tags out)
    loop {
        let Some(open) = t.find(['(', '[']) else { break };
        let close = t[open..].find([')', ']']).map(|c| open + c);
        let Some(close) = close else { break };
        let inner = &t[open..=close];
        if ["deluxe", "edition", "remaster", "expanded", "bonus"].iter().any(|w| inner.contains(w)) {
            t = format!("{}{}", &t[..open], &t[close + 1..]);
        } else {
            break;
        }
    }
    let t = t.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() { return String::new(); }
    format!("{}|{t}", artist_key(artist.split(',').next().unwrap_or("")))
}

/// The artist a track is "by", as the menu names it (its first credited one)
pub fn artist_of(t: &Track) -> String {
    let (a, _) = crate::versions::guess_names(t);
    let a = if a.is_empty() { t.artist.clone() } else { a };
    a.split([',', '&']).next().unwrap_or("").trim().to_string()
}

fn has(prefs: &Value, list: &str, key: &str) -> bool {
    match &prefs[list] {
        Value::Object(m) => m.contains_key(key),
        Value::Array(a) => a.iter().any(|x| x.as_str() == Some(key)),
        _ => false,
    }
}

/// Whether the app may pick this track for you
pub fn rec_ok(prefs: &Value, t: &Track) -> bool {
    if !on(prefs) { return true; }
    if has(prefs, "hiddenRecs", &t.key) { return false; }
    let names = std::iter::once(crate::versions::guess_names(t).0).chain(t.artist.split([',', '&']).map(|s| s.trim().to_string()));
    if names.into_iter().any(|n| !n.is_empty() && has(prefs, "hiddenArtists", &artist_key(&n))) { return false; }
    t.album.is_empty() || !has(prefs, "hiddenAlbums", &album_key(&t.album, &artist_of(t)))
}

/// Hidden from every list (a Go+ track with no full version anywhere)
pub fn no_version(prefs: &Value, t: &Track) -> bool { has(prefs, "noVersion", &t.key) }

/// What hiding one changes in the prefs: (key, its new value)
pub fn hide(prefs: &Value, t: &Track, what: &str) -> Option<(Vec<(&'static str, Value)>, String)> {
    let mut out: Vec<(&'static str, Value)> = vec![("hideOn", json!(true))];
    let said = match what {
        "track" => {
            let mut recs: Vec<Value> = prefs["hiddenRecs"].as_array().cloned().unwrap_or_default();
            if !recs.iter().any(|x| x.as_str() == Some(&t.key)) { recs.push(json!(t.key)); }
            let start = recs.len().saturating_sub(3000);
            let recs: Vec<Value> = recs[start..].to_vec();
            let mut names = prefs["hiddenRecNames"].as_object().cloned().unwrap_or_default();
            names.insert(t.key.clone(), json!(format!("{}{}", t.title, if t.artist.is_empty() { String::new() } else { format!(" \u{2014} {}", t.artist) })));
            names.retain(|k, _| recs.iter().any(|x| x.as_str() == Some(k)));
            out.push(("hiddenRecs", json!(recs)));
            out.push(("hiddenRecNames", Value::Object(names)));
            "It won't be recommended again.".to_string()
        }
        "album" => {
            let a = artist_of(t);
            let k = album_key(&t.album, &a);
            if k.is_empty() { return None; }
            let mut m = prefs["hiddenAlbums"].as_object().cloned().unwrap_or_default();
            m.insert(k, json!(format!("{}{}", t.album, if a.is_empty() { String::new() } else { format!(" \u{2014} {a}") })));
            out.push(("hiddenAlbums", Value::Object(m)));
            format!("\"{}\" won't be recommended again.", t.album)
        }
        _ => {
            let a = artist_of(t);
            let k = artist_key(&a);
            if k.is_empty() { return None; }
            let mut m = prefs["hiddenArtists"].as_object().cloned().unwrap_or_default();
            m.insert(k, json!(a));
            out.push(("hiddenArtists", Value::Object(m)));
            format!("{a} won't be recommended again.")
        }
    };
    Some((out, said))
}

/// Recommended (or shown) again: (list, key) -> the list's new value
pub fn unhide(prefs: &Value, list: &str, key: &str) -> Vec<(&'static str, Value)> {
    let drop = |name: &'static str| -> (&'static str, Value) {
        let mut m = prefs[name].as_object().cloned().unwrap_or_default();
        m.remove(key);
        (name, Value::Object(m))
    };
    match list {
        "a" => vec![drop("hiddenArtists")],
        "l" => vec![drop("hiddenAlbums")],
        "v" => vec![drop("noVersion")],
        _ => {
            let recs: Vec<Value> = prefs["hiddenRecs"].as_array().cloned().unwrap_or_default().into_iter().filter(|x| x.as_str() != Some(key)).collect();
            vec![("hiddenRecs", json!(recs)), drop("hiddenRecNames")]
        }
    }
}

/// The panel's fields (Settings > Library > Hidden & Not Recommended): what's there, and each to undo
pub fn fields(prefs: &Value) -> Vec<Value> {
    let sorted = |name: &str| -> Vec<(String, String)> {
        let mut l: Vec<(String, String)> = prefs[name].as_object().into_iter().flatten().map(|(k, v)| (k.clone(), v.as_str().unwrap_or(k).to_string())).collect();
        l.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
        l
    };
    let artists = sorted("hiddenArtists");
    let albums = sorted("hiddenAlbums");
    let names = prefs["hiddenRecNames"].as_object().cloned().unwrap_or_default();
    let recs: Vec<String> = prefs["hiddenRecs"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    let mut tracks: Vec<(String, String)> = recs.iter().filter_map(|id| names.get(id).and_then(|n| n.as_str()).map(|n| (id.clone(), n.to_string()))).collect();
    tracks.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    let unnamed = recs.len() - tracks.len();
    let nv = sorted("noVersion");
    let mut f = Vec::new();
    let plural = |n: usize, one: &str, many: &str| format!("{n} {}", if n == 1 { one } else { many });
    let mut parts = Vec::new();
    if !artists.is_empty() { parts.push(plural(artists.len(), "artist", "artists")); }
    if !albums.is_empty() { parts.push(plural(albums.len(), "album", "albums")); }
    if !recs.is_empty() { parts.push(plural(recs.len(), "track", "tracks")); }
    let summary = if !on(prefs) { "Off: nothing is hidden or left out. Your lists are kept.".to_string() }
        else if parts.is_empty() && nv.is_empty() { "Nothing yet. Right-click a track to stop it (or its album, or its artist) being recommended.".to_string() }
        else { format!("{}{}", if parts.is_empty() { String::new() } else { format!("{} {} recommended. ", parts.join(", "), if artists.len() + albums.len() + recs.len() == 1 { "isn't" } else { "aren't" }) }, if nv.is_empty() { String::new() } else { format!("{} hidden for having no full version.", plural(nv.len(), "track", "tracks")) }) };
    f.push(json!({ "key": "", "label": summary, "note": "", "type": "note" }));
    let row = |list: &str, k: &str, name: &str, press: &str| json!({ "key": format!("un{list}:{k}"), "label": name, "note": "", "type": "button", "value": press });
    if !artists.is_empty() { f.push(json!({ "key": "", "label": format!("Artists Not Recommended ({})", artists.len()), "note": "", "type": "note" })); }
    for (k, n) in &artists { f.push(row("a", k, n, "Recommend Again")); }
    if !albums.is_empty() { f.push(json!({ "key": "", "label": format!("Albums Not Recommended ({})", albums.len()), "note": "", "type": "note" })); }
    for (k, n) in &albums { f.push(row("l", k, n, "Recommend Again")); }
    if !recs.is_empty() { f.push(json!({ "key": "", "label": format!("Tracks Not Recommended ({}){}", recs.len(), if unnamed > 0 { format!(": {unnamed} from before names were kept") } else { String::new() }), "note": "", "type": "note" })); }
    for (k, n) in &tracks { f.push(row("t", k, n, "Recommend Again")); }
    if !nv.is_empty() { f.push(json!({ "key": "", "label": format!("Hidden: No Full Version ({})", nv.len()), "note": "", "type": "note" })); }
    for (k, n) in &nv { f.push(row("v", k, n, "Show Again")); }
    f
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(key: &str, title: &str, artist: &str, album: &str) -> Track { Track { key: key.into(), title: title.into(), artist: artist.into(), album: album.into(), ..Default::default() } }
    #[test]
    fn hidden_tracks_albums_and_artists_are_left_out() {
        let song = t("sp:1", "Song", "The Band, Guest", "Album (Deluxe Edition)");
        let none = json!({});
        assert!(rec_ok(&none, &song));
        let (ch, _) = hide(&none, &song, "artist").unwrap();
        let p = json!({ ch[1].0: ch[1].1.clone() });
        assert!(!rec_ok(&p, &song));
        assert!(!rec_ok(&p, &t("x", "Other", "Band", "")));   // ("The Band" and "Band" are one artist)
        assert!(rec_ok(&json!({ "hideOn": false, "hiddenArtists": p["hiddenArtists"].clone() }), &song));
        let (ch, _) = hide(&none, &song, "album").unwrap();
        let p = json!({ ch[1].0: ch[1].1.clone() });
        assert!(!rec_ok(&p, &t("y", "Two", "The Band", "Album")));
        let (ch, _) = hide(&none, &song, "track").unwrap();
        let p = json!({ "hiddenRecs": ch[1].1.clone(), "hiddenRecNames": ch[2].1.clone() });
        assert!(!rec_ok(&p, &song));
        let back = unhide(&p, "t", "sp:1");
        assert!(rec_ok(&json!({ back[0].0: back[0].1.clone() }), &song));
        assert!(fields(&p).iter().any(|f| f["key"] == "unt:sp:1"));
    }
}
