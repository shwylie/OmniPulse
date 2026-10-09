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
