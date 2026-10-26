// Stream-Safe Music (as the web app's js/streamsafe.js): which tracks may be played on stream. A
// track counts when it's under an open licence (SoundCloud's Creative Commons ones and
// "no-rights-reserved"), or its artist or record label is cleared for streaming (NCS, StreamBeats and
// the like, plus any you added in the web app). Nothing else says how it's licensed, so nothing else
// counts. (The web app also asks Spotify for album labels and YouTube for Creative Commons videos;
// this app doesn't yet, so those only count by their artists.)

use crate::track::Track;
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// (id, artists, labels, on unless switched off)
const CLEARED: [(&str, &[&str], &[&str], bool); 6] = [
    ("ncs", &["NoCopyrightSounds"], &["NCS", "NoCopyrightSounds"], true),
    ("streambeats", &["StreamBeats", "Harris Heller"], &["StreamBeats"], true),
    ("monstercat", &[], &["Monstercat", "Monstercat Silk", "Monstercat Instinct", "Monstercat Uncaged"], false),
    ("epidemic", &[], &["Epidemic Sound"], false),
    ("lofigirl", &[], &["Lofi Records", "Lofi Girl"], false),
    ("chillhop", &[], &["Chillhop Music", "Chillhop Records"], false),
];

/// Whether it's on (Settings > Streaming, or the Streamer Mode button's third step)
pub fn on(prefs: &Value) -> bool {
    prefs["streamSafe"].as_bool() == Some(true)
}

/// As the web app's artistKey: lower case, letters and digits only
pub fn artist_key(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric() || ('\u{c0}'..='\u{24f}').contains(c)).collect()
}

fn open_licence(t: &Track) -> bool {
    let l = t.licence.to_lowercase();
    l.starts_with("cc-") || l == "cc0" || l == "no-rights-reserved"
}

/// Everyone named as the track's artist: "A, B & C feat. D", and "Artist - Song" uploads
fn names(t: &Track) -> Vec<String> {
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    static DASH: OnceLock<Regex> = OnceLock::new();
    let split = SPLIT.get_or_init(|| Regex::new(r"(?i)\s*(?:,|&|/|\s+x\s+|\s+feat\.?\s+|\s+ft\.?\s+|\s+with\s+|\s+and\s+)\s*").expect("split"));
    let dash = DASH.get_or_init(|| Regex::new(r"^(.+?)\s+[-\x{2013}\x{2014}]\s+.+$").expect("dash"));
    let mut out: Vec<String> = split.split(&t.artist).map(artist_key).collect();
    out.push(artist_key(&t.artist));
    if let Some(c) = dash.captures(&t.title) {
        out.extend(split.split(&c[1]).map(artist_key));
    }
    out.retain(|k| !k.is_empty());
    out
}

/// Whether a track may be played on stream (whether or not Stream-Safe Music is on)
pub fn ok(prefs: &Value, t: &Track) -> bool {
    if open_licence(t) {
        return true;
    }
    let p = &prefs["streamCleared"];
    let set = |id: &str, dflt: bool| p["on"][id].as_bool().unwrap_or(dflt);
    let names = names(t);
    let label = artist_key(&t.label);
    let hit = |artists: &[String], labels: &[String]| {
        artists.iter().any(|a| names.contains(&artist_key(a))) || (!label.is_empty() && labels.iter().any(|l| artist_key(l) == label))
    };
    for (id, artists, labels, dflt) in CLEARED {
        if set(id, dflt) && hit(&artists.iter().map(|s| s.to_string()).collect::<Vec<_>>(), &labels.iter().map(|s| s.to_string()).collect::<Vec<_>>()) {
            return true;
        }
    }
    // the ones you added: { id, kind: "artist" | "label", name }
    p["custom"].as_array().into_iter().flatten().any(|c| {
        let id = c["id"].as_str().unwrap_or("");
        let name = c["name"].as_str().unwrap_or("").to_string();
        set(id, true) && if c["kind"].as_str() == Some("label") { hit(&[], &[name]) } else { hit(&[name], &[]) }
    })
}

/// The artists and labels cleared for streaming that are on: (name, the names to search for)
pub fn cleared_on(prefs: &Value) -> Vec<(String, Vec<String>)> {
    let p = &prefs["streamCleared"];
    let set = |id: &str, dflt: bool| p["on"][id].as_bool().unwrap_or(dflt);
    let shown = |id: &str| -> String { match id { "ncs" => "NCS", "streambeats" => "StreamBeats", "monstercat" => "Monstercat", "epidemic" => "Epidemic Sound", "lofigirl" => "Lofi Girl", "chillhop" => "Chillhop Music", x => x }.to_string() };
    let mut out: Vec<(String, Vec<String>)> = CLEARED.iter().filter(|(id, _, _, d)| set(id, *d))
        .map(|(id, artists, labels, _)| (shown(id), artists.iter().chain(labels.iter()).map(|s| s.to_string()).collect::<Vec<_>>()))
        .collect();
    for c in p["custom"].as_array().into_iter().flatten() {
        let (id, name) = (c["id"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""));
        if !name.is_empty() && set(id, true) { out.push((name.to_string(), vec![name.to_string()])); }
    }
    out.iter_mut().for_each(|(_, n)| { n.sort(); n.dedup(); n.truncate(4); });
    out
}

/// Creative Commons music by genre (the Streamer Mode page's, as the web app's STREAM_GENRES)
pub const GENRES: [&str; 8] = ["Electronic", "Lo-fi", "Chill", "Hip-hop", "Ambient", "Rock", "Synthwave", "Acoustic"];

/// Under an open licence (SoundCloud's Creative Commons ones, "no-rights-reserved")
pub fn open(t: &Track) -> bool {
    open_licence(t)
}

/// The cleared entry (on) a track comes under: (its id, its name)
fn cleared_entry(prefs: &Value, t: &Track) -> Option<(String, String)> {
    let p = &prefs["streamCleared"];
    let set = |id: &str, dflt: bool| p["on"][id].as_bool().unwrap_or(dflt);
    let names = names(t);
    let label = artist_key(&t.label);
    let hit = |artists: &[&str], labels: &[&str]| artists.iter().any(|a| names.contains(&artist_key(a))) || (!label.is_empty() && labels.iter().any(|l| artist_key(l) == label));
    let shown = |id: &str| -> String { match id { "ncs" => "NCS", "streambeats" => "StreamBeats", "monstercat" => "Monstercat", "epidemic" => "Epidemic Sound", "lofigirl" => "Lofi Girl", "chillhop" => "Chillhop Music", x => x }.to_string() };
    for (id, artists, labels, dflt) in CLEARED {
        if set(id, dflt) && hit(artists, labels) { return Some((id.to_string(), shown(id))); }
    }
    p["custom"].as_array().into_iter().flatten().find_map(|c| {
        let (id, name) = (c["id"].as_str().unwrap_or(""), c["name"].as_str().unwrap_or(""));
        let yes = set(id, true) && if c["kind"].as_str() == Some("label") { hit(&[], &[name]) } else { hit(&[name], &[]) };
        yes.then(|| (id.to_string(), name.to_string()))
    })
}

/// A track's menu line for Stream-Safe Music (the web app's clearedMenuLabel): "Allow <artist> on
/// stream" or "Stop allowing <name> on stream"; none for open licences
pub fn menu_label(prefs: &Value, t: &Track) -> Option<(String, bool)> {
    if open_licence(t) { return None; }
    if let Some((_, name)) = cleared_entry(prefs, t) { return Some((format!("Stop allowing {name} on stream"), false)); }
    let a = t.artist.split([',', '&']).next().unwrap_or("").trim();
    (!a.is_empty()).then(|| (format!("Allow {a} on stream"), true))
}

/// That menu line chosen: the new streamCleared, and what to say
pub fn menu_run(prefs: &Value, t: &Track) -> Option<(Value, String)> {
    let mut p = prefs["streamCleared"].clone();
    if !p.is_object() { p = serde_json::json!({ "custom": [], "on": {} }); }
    if !p["custom"].is_array() { p["custom"] = serde_json::json!([]); }
    if !p["on"].is_object() { p["on"] = serde_json::json!({}); }
    if let Some((id, name)) = cleared_entry(prefs, t) {
        p["on"][id.as_str()] = serde_json::json!(false);
        return Some((p, format!("{name} isn't cleared for streaming any more.")));
    }
    let a = t.artist.split([',', '&']).next().unwrap_or("").trim().to_string();
    if a.is_empty() { return None; }
    let key = artist_key(&a);
    let old = p["custom"].as_array().and_then(|l| l.iter().find(|c| c["kind"].as_str() == Some("artist") && artist_key(c["name"].as_str().unwrap_or("")) == key).and_then(|c| c["id"].as_str().map(String::from)));
    match old {
        Some(id) => p["on"][id.as_str()] = serde_json::json!(true),
        None => {
            let id = format!("c{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
            p["custom"].as_array_mut().unwrap().push(serde_json::json!({ "id": id, "kind": "artist", "name": a }));
        }
    }
    Some((p, format!("{a} is cleared for streaming: Stream-Safe Music plays them.")))
}

// ---------- What Counts as Safe (Settings > Streaming > Extended): the list, each switched, yours added ----------
fn shown_name(id: &str) -> String {
    match id { "ncs" => "NCS (NoCopyrightSounds)", "streambeats" => "StreamBeats by Harris Heller", "monstercat" => "Monstercat (with a Gold licence)", "epidemic" => "Epidemic Sound (with a subscription)", "lofigirl" => "Lofi Girl", "chillhop" => "Chillhop Music", x => x }.to_string()
}

/// The panel's values: each entry's switch ("on:<id>"), and the add form (its name and kind)
pub fn panel_values(prefs: &Value) -> serde_json::Map<String, Value> {
    let p = &prefs["streamCleared"];
    let mut m = serde_json::Map::new();
    for (id, _, _, dflt) in CLEARED { m.insert(format!("on:{id}"), serde_json::json!(p["on"][id].as_bool().unwrap_or(dflt))); }
    for c in p["custom"].as_array().into_iter().flatten() {
        if let Some(id) = c["id"].as_str() { m.insert(format!("on:{id}"), serde_json::json!(p["on"][id].as_bool().unwrap_or(true))); }
    }
    m.insert("safeName".into(), serde_json::json!(""));
    m.insert("safeKind".into(), serde_json::json!("artist"));
    m
}

/// The panel's fields
pub fn panel_fields(prefs: &Value) -> Vec<Value> {
    let tog = |k: String, label: String, note: &str| serde_json::json!({ "key": k, "label": label, "note": note, "type": "toggle" });
    let mut f = vec![serde_json::json!({ "key": "", "label": "With Extended on, only these play (and songs under an open licence, such as SoundCloud's Creative Commons ones). Switch off any you don't have a licence for.", "note": "", "type": "note" })];
    for (id, artists, labels, _) in CLEARED {
        let mut what: Vec<&str> = Vec::new();
        for x in artists.iter().chain(labels.iter()) { if !what.contains(x) { what.push(x); } }
        f.push(tog(format!("on:{id}"), shown_name(id), &what.join(", ")));
    }
    for c in prefs["streamCleared"]["custom"].as_array().into_iter().flatten() {
        let (Some(id), Some(name)) = (c["id"].as_str(), c["name"].as_str()) else { continue };
        f.push(tog(format!("on:{id}"), name.to_string(), if c["kind"].as_str() == Some("label") { "A label you added." } else { "An artist you added." }));
    }
    f.push(serde_json::json!({ "key": "", "label": "Add an artist or label you're cleared to play:", "note": "", "type": "note" }));
    f.push(serde_json::json!({ "key": "safeKind", "label": "It's", "note": "", "type": "select", "options": ["artist", "label"], "labels": ["An artist", "A record label"] }));
    f.push(serde_json::json!({ "key": "safeName", "label": "Artist name", "note": "Spelled as it is on their songs.", "type": "text" }));
    f.push(serde_json::json!({ "key": "safeAdd", "label": "", "note": "", "type": "button", "value": "Add" }));
    f
}

/// The switches saved: the new streamCleared
pub fn panel_save(prefs: &Value, vals: &serde_json::Map<String, Value>) -> Value {
    let mut p = prefs["streamCleared"].clone();
    if !p.is_object() { p = serde_json::json!({ "custom": [], "on": {} }); }
    if !p["on"].is_object() { p["on"] = serde_json::json!({}); }
    if !p["custom"].is_array() { p["custom"] = serde_json::json!([]); }
    for (k, v) in vals { if let Some(id) = k.strip_prefix("on:") { p["on"][id] = v.clone(); } }
    p
}

/// One added (the web app's clearedAdd): the new streamCleared, or None if it's on the list already
pub fn add(prefs: &Value, kind: &str, name: &str) -> Option<Value> {
    let name = name.trim();
    if name.is_empty() { return None; }
    let key = artist_key(name);
    let label = kind == "label";
    let set = |id: &str, dflt: bool| prefs["streamCleared"]["on"][id].as_bool().unwrap_or(dflt);
    if CLEARED.iter().any(|(id, a, l, d)| set(id, *d) && (if label { *l } else { *a }).iter().any(|x| artist_key(x) == key)) { return None; }
    let mut p = panel_save(prefs, &serde_json::Map::new());
    let old = p["custom"].as_array().and_then(|l| l.iter().find(|c| c["kind"].as_str() == Some(kind) && artist_key(c["name"].as_str().unwrap_or("")) == key).and_then(|c| c["id"].as_str().map(String::from)));
    match old {
        Some(id) if set(&id, true) => return None,
        Some(id) => p["on"][id.as_str()] = serde_json::json!(true),
        None => {
            let id = format!("c{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0));
            p["custom"].as_array_mut().unwrap().push(serde_json::json!({ "id": id, "kind": kind, "name": name }));
        }
    }
    Some(p)
}

/// On: whether it may play. Off: everything may.
pub fn allowed(prefs: &Value, t: &Track) -> bool {
    !on(prefs) || ok(prefs, t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::Src;

    fn t(artist: &str, title: &str, licence: &str, label: &str) -> Track {
        Track { src: Src::SoundCloud(1), key: "1".into(), title: title.into(), artist: artist.into(), licence: licence.into(), label: label.into(), ..Default::default() }
    }

    #[test]
    fn open_licences_and_cleared_artists_and_labels_count() {
        let prefs = serde_json::json!({ "streamSafe": true });
        assert!(ok(&prefs, &t("someone", "Song", "cc-by-nc", "")));
        assert!(ok(&prefs, &t("someone", "Song", "no-rights-reserved", "")));
        assert!(!ok(&prefs, &t("someone", "Song", "all-rights-reserved", "")));
        assert!(ok(&prefs, &t("Harris Heller", "Lofi", "", "")));
        assert!(ok(&prefs, &t("Uploader", "Alan Walker & NoCopyrightSounds - Fade", "", "")));
        assert!(ok(&prefs, &t("Elektronomia", "Sky High", "", "NCS")));
        // off until you've signed up with them
        assert!(!ok(&prefs, &t("Pegboard Nerds", "Hero", "", "Monstercat")));
        let mine = serde_json::json!({ "streamCleared": { "on": { "monstercat": true, "c1": true, "ncs": false }, "custom": [{ "id": "c1", "kind": "artist", "name": "Kites" }] } });
        assert!(ok(&mine, &t("Pegboard Nerds", "Hero", "", "Monstercat")));
        assert!(ok(&mine, &t("Kites, Ostra", "Song", "", "")));
        assert!(!ok(&mine, &t("Elektronomia", "Sky High", "", "NCS")));
        assert!(allowed(&serde_json::json!({}), &t("anyone", "Song", "", "")));
    }
}
