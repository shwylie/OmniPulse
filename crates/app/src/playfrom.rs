// Play From (Settings > Plugins): the sources in your order. A song about to play is looked for in the
// sources above its own, and the first that has it plays it instead: your own files, or a plugin with the
// song in better quality, over a stream of it. The song stays as it is in your Library and the queue (its
// title, art and likes); only what's heard comes from elsewhere. Found once per song while the app runs.
//
// The order (prefs.playFrom: ids) lists the app's own sources, "local", "soundcloud" and "youtube", and
// each plugin that plays ("plug:<id>"). Sources missing from it come after, built-ins first. One switched
// off (prefs.playFromOff) keeps its place but is never played from: its own songs come from any source
// that's on, in the order. By default nothing changes: a song plays from where it is.

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

/// The sources switched off (prefs.playFromOff): never played from; their own songs come from a source
/// that's on and has them (and, for now, from their own when none does, so they still play)
pub fn off(prefs: &Value) -> Vec<String> {
    prefs["playFromOff"].as_array().into_iter().flatten().filter_map(|v| v.as_str().map(String::from)).collect()
}

/// The Play From page's switches: "on:<id>", on unless switched off
pub fn list_values(prefs: &Value) -> serde_json::Map<String, Value> {
    let off = off(prefs);
    let mut m: serde_json::Map<String, Value> = sources(prefs).into_iter().map(|(id, _)| (format!("on:{id}"), Value::Bool(!off.contains(&id)))).collect();
    m.insert("playFromOwn".into(), Value::Bool(own_first(prefs)));
    m
}

/// The page saved: the ids switched off, to keep as prefs.playFromOff
pub fn list_save(prefs: &Value, vals: &serde_json::Map<String, Value>) -> Value {
    let ids: Vec<Value> = sources(prefs).into_iter().filter(|(id, _)| vals.get(&format!("on:{id}")).and_then(|v| v.as_bool()) == Some(false)).map(|(id, _)| Value::String(id)).collect();
    forget_all();
    Value::Array(ids)
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

/// The best of these for the song, if one's close enough: the same version, by them (a remix of it
/// isn't it, however well the rest matches). A Go+ track's length isn't judged: SoundCloud's figure for one
/// is its preview's, or an edit's, not the song's
pub fn best(t: &Track, found: Vec<Track>) -> Option<Track> {
    let want = if t.go_plus { Track { ms: 0, ..t.clone() } } else { t.clone() };
    let t = &want;
    let mut ranked: Vec<(f32, Track)> = found.into_iter().filter(|c| c.key != t.key).map(|c| (versions::score(t, &as_yt(&c)), c)).collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    // (or a remix titled exactly as the song is, whoever's credited: the remixer's name in the title is the
    // identification, when its length agrees)
    let titled = |c: &Track| { let y = as_yt(c); versions::same_titled_version(t, &y) && versions::same_version(t, &y) && versions::length_ok(t, &y) };
    ranked.into_iter().find(|(s, c)| (*s >= GOOD && versions::artist_hit(t, &as_yt(c)) && versions::same_version(t, &as_yt(c))) || titled(c)).map(|(_, c)| c)
}

/// Where You Liked It (prefs.playFromOwn, on unless switched off): a song plays from its own source when
/// it can; the order only steps in for one that plays from elsewhere anyway (`elsewhere`: a Go+ preview,
/// a Spotify song without Premium...)
pub fn own_first(prefs: &Value) -> bool {
    prefs["playFromOwn"].as_bool().unwrap_or(true)
}

/// The song looked for in the sources above its own (`locals`: your files, for "local"); the first that
/// has it is kept for the song and given back. Itself: None. Decided once. `elsewhere`: it can't play in
/// full from its own source here, so another's looked for even with Where You Liked It on.
pub async fn find(t: &Track, prefs: &Value, locals: &[Track], elsewhere: bool) -> Option<Track> {
    if let Some(d) = decided(&t.key) { return d; }
    let own = source_of(t);
    let off = off(prefs);
    // (its own source switched off: never played from; the song from any source that's on, in your order)
    let own_off = off.contains(&own);
    if own_first(prefs) && !elsewhere && !own_off {
        decide(&t.key, None);
        return None;
    }
    let ids: Vec<String> = sources(prefs).into_iter().map(|(i, _)| i).take_while(|i| own_off || *i != own).filter(|i| !off.contains(i) && *i != own).collect();
    if ids.is_empty() || t.is_mix() {
        decide(&t.key, None);
        return None;
    }
    let got = look_in(&ids, t, locals).await;
    decide(&t.key, got.clone());
    got
}

/// What one source has for the song (the query: its artist and title, as guessed)
async fn search(id: &str, t: &Track, q: &str, locals: &[Track]) -> Vec<Track> {
    match id {
        "local" => locals.to_vec(),
        "soundcloud" if omnipulse_core::soundcloud::signed_in() => crate::sources::sc_search(q, 8).await,
        "soundcloud" => Vec::new(),
        "youtube" => match versions::find(t).await { Some(v) if v.exact => vec![Track::youtube(&v.yt)], _ => Vec::new() },
        p => match p.strip_prefix("plug:").and_then(crate::plugrun::get) {
            Some(plug) => plug.call("search", vec![serde_json::json!(q)]).await.map(|v| v.as_array().cloned().unwrap_or_default().iter().take(30).filter_map(|x| Track::plugin(&plug.id, x)).collect()).unwrap_or_default(),
            None => Vec::new(),
        },
    }
}

/// What to ask a source for the song, in order: its artist and bare title; its title with its version
/// alone ("Gold Dust Fox Stevenson Remix": an uploader's name as the artist, "DJFreshUK", finds nothing);
/// then both in full
pub fn queries(t: &Track) -> Vec<String> {
    let (artist, title) = versions::guess_names(t);
    let full = versions::match_title(t);
    let mut out: Vec<String> = Vec::new();
    for q in [format!("{artist} {title}"), full.clone(), format!("{artist} {full}")] {
        let q = q.split_whitespace().collect::<Vec<_>>().join(" ");
        if !q.is_empty() && !out.iter().any(|x| x.eq_ignore_ascii_case(&q)) { out.push(q); }
    }
    out
}

/// The song looked for in these sources, in order: the first with it (each asked up to three ways)
async fn look_in(ids: &[String], t: &Track, locals: &[Track]) -> Option<Track> {
    let qs = queries(t);
    for id in ids {
        for q in &qs {
            if let Some(c) = best(t, search(id, t, q, locals).await) {
                omnipulse_core::diag::note(&format!("Play From: {} - {} plays from {} ({}), asked \"{q}\"", t.artist, t.title, id, c.key));
                return Some(c);
            }
            // (your files and YouTube Music: one look is every look)
            if matches!(id.as_str(), "local" | "youtube") { break; }
        }
    }
    None
}

/// The song's own way failed (a preview, YouTube paused, no version of it): every source that's on asked,
/// in your order, below its own too, before it's given up on. What's found is kept for the song
pub async fn find_anywhere(t: &Track, prefs: &Value, locals: &[Track]) -> Option<Track> {
    if t.is_mix() { return None; }
    let own = source_of(t);
    let off = off(prefs);
    let ids: Vec<String> = sources(prefs).into_iter().map(|(i, _)| i).filter(|i| *i != own && !off.contains(i)).collect();
    if ids.is_empty() { return None; }
    omnipulse_core::diag::note(&format!("Play From: {} - {} couldn't play its own way: every source that's on asked", t.artist, t.title));
    let got = look_in(&ids, t, locals).await;
    if got.is_some() { decide(&t.key, got.clone()); }
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
        // (switched off: still listed, and saved as off from the page's switches)
        let p = json!({ "playFromOff": ["soundcloud"] });
        assert_eq!(off(&p), vec!["soundcloud"]);
        assert_eq!(list_values(&p).get("on:soundcloud"), Some(&json!(false)));
        assert_eq!(list_values(&p).get("on:local"), Some(&json!(true)));
        assert_eq!(list_values(&p).get("playFromOwn"), Some(&json!(true)));
        assert!(own_first(&p) && !own_first(&json!({ "playFromOwn": false })));
        let mut vals = list_values(&p);
        vals.insert("on:youtube".into(), json!(false));
        vals.insert("on:soundcloud".into(), json!(true));
        assert_eq!(list_save(&p, &vals), json!(["youtube"]));
        // (a source off: its own songs looked for in the others, with Where You Liked It on too)
        let yt = Track { key: "yt:1".into(), src: Src::YouTube("1".into()), title: "Strobe".into(), artist: "deadmau5".into(), ms: 636_000, ..Default::default() };
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        forget_all();
        assert!(rt.block_on(find(&yt, &json!({}), &[], false)).is_none());
        let strobe = Track { key: "l:1".into(), src: Src::Local("C:/m/strobe.flac".into()), title: "Strobe".into(), artist: "deadmau5".into(), ms: 636_000, ..Default::default() };
        forget_all();
        assert_eq!(rt.block_on(find(&yt, &json!({ "playFromOff": ["youtube"] }), &[strobe.clone()], false)).map(|c| c.key), Some("l:1".to_string()));
        forget_all();
        assert!(rt.block_on(find(&yt, &json!({ "playFromOff": ["youtube", "local", "soundcloud"] }), &[strobe], false)).is_none());
        forget_all();
    }

    #[test]
    fn the_best_is_the_same_song_close_enough() {
        let want = Track { key: "sp:1".into(), src: Src::Spotify("1".into()), title: "Strobe".into(), artist: "deadmau5".into(), ms: 636_000, ..Default::default() };
        let strobe = Track::plugin("p", &json!({ "id": "a", "title": "Strobe", "artist": "deadmau5", "album": "For Lack of a Better Name", "duration_ms": 635_000 })).unwrap();
        let edit = Track::plugin("p", &json!({ "id": "b", "title": "Strobe (Radio Edit)", "artist": "deadmau5", "duration_ms": 220_000 })).unwrap();
        let other = Track::plugin("p", &json!({ "id": "c", "title": "Ghosts n Stuff", "artist": "deadmau5", "duration_ms": 200_000 })).unwrap();
        assert_eq!(best(&want, vec![other.clone(), edit.clone(), strobe.clone()]).map(|c| c.key), Some("plug:p:a".to_string()));
        assert!(best(&want, vec![other, edit]).is_none());
        // (a remix on SoundCloud, titled "Artist - Song [X Remix]": the artist's name and the length match,
        // the version doesn't: not it)
        let starship = Track { key: "sp:2".into(), src: Src::Spotify("2".into()), title: "Nothing's Gonna Stop Us Now".into(), artist: "Starship".into(), ms: 270_000, ..Default::default() };
        let remix = Track::soundcloud(&json!({ "id": 534827904, "title": "Starship - Nothing's Gonna Stop Us Now [Cole Remix]", "user": { "username": "ColeOfficial" }, "duration": 268_000 })).unwrap();
        assert_eq!((remix.artist.as_str(), remix.title.as_str()), ("Starship", "Nothing's Gonna Stop Us Now [Cole Remix]"));
        assert!(best(&starship, vec![remix]).is_none());
        let original = Track::soundcloud(&json!({ "id": 1, "title": "Starship - Nothing's Gonna Stop Us Now", "user": { "username": "Starship" }, "duration": 270_000 })).unwrap();
        assert_eq!(best(&starship, vec![original]).map(|c| c.key), Some("1".to_string()));
        // (a Go+ track's length is its preview's or an edit's: not judged. 185 s against the song's 241 s)
        let goplus = Track { key: "9".into(), src: Src::SoundCloud(9), title: "Mountain Song".into(), artist: "Jane's Addiction".into(), ms: 185_129, go_plus: true, ..Default::default() };
        let full = Track::plugin("p", &json!({ "id": "m", "title": "Mountain Song", "artist": "Jane's Addiction", "album": "Nothing's Shocking", "duration_ms": 241_000 })).unwrap();
        assert_eq!(best(&goplus, vec![full]).map(|c| c.key), Some("plug:p:m".to_string()));
        // (a SoundCloud upload with the uploader as its artist: asked by title and version too, and the
        // uploader's name taken for the artist's when it's the same squashed together)
        let gold = Track { key: "7".into(), src: Src::SoundCloud(7), title: "Gold Dust (Fox Stevenson Remix)".into(), artist: "DJFreshUK".into(), ms: 233_000, ..Default::default() };
        assert_eq!(queries(&gold), vec!["DJFreshUK Gold Dust", "Gold Dust (Fox Stevenson Remix)", "DJFreshUK Gold Dust (Fox Stevenson Remix)"]);
        let fox = Track::plugin("p", &json!({ "id": "g", "title": "Gold Dust (Fox Stevenson Remix)", "artist": "DJ Fresh", "duration_ms": 233_000 })).unwrap();
        let shy = Track::plugin("p", &json!({ "id": "s", "title": "Gold Dust (SHY FX Re-Edit)", "artist": "DJ Fresh", "duration_ms": 183_000 })).unwrap();
        assert_eq!(best(&gold, vec![shy.clone(), fox]).map(|c| c.key), Some("plug:p:g".to_string()));
        // (the same remix title exactly, from someone else entirely: still it; the wrong remix never)
        let other = Track::plugin("p", &json!({ "id": "o", "title": "Gold Dust (Fox Stevenson Remix)", "artist": "Some Uploader", "duration_ms": 234_000 })).unwrap();
        assert_eq!(best(&gold, vec![shy, other]).map(|c| c.key), Some("plug:p:o".to_string()));
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
