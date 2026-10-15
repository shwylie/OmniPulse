// Song requests from chat (as in js/requests.js): a bot calls the requests server in core, which asks
// the app; the app finds the song and queues it, and the reply is posted in chat. Punctuation is
// ignored, "song by artist" is understood, your likes come first, links go to their own platform,
// and with Prefer Radio Edits a long version gives way to a radio edit of the same song.

use crate::track::{Src, Track};
use omnipulse_core as core;
use serde_json::Value;

/// The words in a request or a title, for matching (accents aside, as the web app's: "beyonce" is
/// "Beyoncé")
pub fn words(s: &str) -> Vec<String> {
    use unicode_normalization::UnicodeNormalization;
    let bare: String = s.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    bare.to_lowercase()
        .split(|c: char| !(c.is_alphanumeric()))
        .filter(|w| w.chars().count() > 1 || w.chars().any(|c| c.is_ascii_digit()))
        .map(String::from)
        .collect()
}

/// What chat asked for, tidied: punctuation dropped (apostrophes in words and names like P!nk kept)
pub fn clean(q: &str) -> String {
    let q = q.to_lowercase().replace(['\u{2018}', '\u{2019}', '\u{2bc}'], "'");
    let kept: String = q.chars().map(|c| if c.is_alphanumeric() || "'&+#$!".contains(c) { c } else { ' ' }).collect();
    kept.split_whitespace()
        .map(|w| w.trim_matches('\'').trim_end_matches('!'))
        .filter(|w| !w.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
}

/// The searches to try, best first: (words, artist that has to match), so "song by artist" is
/// searched as "artist song" but "Stand by Me" still works as typed
pub fn queries(q: &str) -> Vec<(String, Option<String>)> {
    let plain = clean(q);
    let mut out = Vec::new();
    if let Some(i) = plain.rfind(" by ") {
        let (song, artist) = (&plain[..i], &plain[i + 4..]);
        if !song.is_empty() && !artist.is_empty() {
            out.push((format!("{artist} {song}"), Some(artist.to_string())));
        }
    }
    if !plain.is_empty() {
        out.push((plain, None));
    }
    out
}

fn by_artist(list: Vec<Track>, artist: &Option<String>) -> Vec<Track> {
    let Some(artist) = artist else { return list };
    let want = words(artist);
    list.into_iter()
        .filter(|t| {
            // the artist, or the "Artist - " an uploader often puts in front of the title
            let lead = t.title.split_once(" - ").map(|x| x.0).unwrap_or("");
            let have = words(&format!("{} {lead}", t.artist));
            want.iter().all(|w| have.contains(w))
        })
        .collect()
}

const VERSION_WORDS: [&str; 22] = ["radio", "edit", "mix", "version", "original", "single", "short", "clean", "explicit", "the", "a", "and", "feat", "ft", "featuring", "with", "x", "vs", "remastered", "remaster", "mono", "stereo"];
const LONG_MS: u64 = 6 * 60 * 1000;

fn is_radio(title: &str) -> bool {
    let w = words(title);
    w.windows(2).any(|p| matches!((p[0].as_str(), p[1].as_str()), ("radio", "edit" | "mix" | "version") | ("single" | "short", "edit" | "version"))) || w.iter().any(|x| x == "edit")
}

/// The same song as asked for: every word there, and anything else in its title only a version
/// word, the artist's name or a year
pub fn same_song(t: &Track, q: &str) -> bool {
    let want = words(q);
    let artist = words(&t.artist);
    let title = words(&t.title);
    if !want.iter().all(|w| title.contains(w) || artist.contains(w)) {
        return false;
    }
    title.iter().all(|w| want.contains(w) || artist.contains(w) || VERSION_WORDS.contains(&w.as_str()) || (w.len() == 4 && w.chars().all(|c| c.is_ascii_digit())))
}

/// Another version of a song (a remix, bootleg, flip, VIP, mashup, cover, live...), as opposed to the
/// song itself (a radio edit, the original mix, a remaster are the song itself)
pub fn is_version(s: &str) -> bool {
    static SAME: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static MARK: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let same = SAME.get_or_init(|| regex::Regex::new(r"(?i)\b(radio (edit|mix|version)|single (edit|version)|short (edit|version)|clean (edit|version)|album (version|edit)|original (mix|version)|remaster(ed)?( \d{4})?|\d{4} remaster(ed)?)\b").expect("same"));
    let mark = MARK.get_or_init(|| regex::Regex::new(r"(?i)\b(remix|rmx|bootleg|boot|flip|vip|mash ?up|cover|live|acoustic|instrumental|sped ?up|slowed|nightcore|karaoke|rework|refix|re-?edit|dub|edit|mix|version|vs)\b").expect("mark"));
    mark.is_match(&same.replace_all(s, " "))
}

/// Every word asked for is in its title or artist (more words are fine)
fn has_all(t: &Track, q: &str) -> bool {
    let have = words(&format!("{} {}", t.title, t.artist));
    words(q).iter().all(|w| have.contains(w))
}

/// The best of a search for what was asked for, and how good (lower is better): the song itself that
/// you like, the song itself, one with every word asked for, one you like, then anything. Unless a
/// version was asked for, other versions only come after the song itself; but one with every word in
/// it beats one without them ("ghosts n stuff ginsu" is Ginsu's mashup, not the original).
fn rank<'a>(found: &'a [Track], asked: &str, liked: &dyn Fn(&Track) -> bool) -> Option<(&'a Track, usize)> {
    let want_version = is_version(asked);
    let top: Vec<&Track> = found.iter().take(10).filter(|x| want_version || !is_version(&x.title)).collect();
    let same = |x: &&&Track| same_song(x, asked);
    let all = |x: &&&Track| has_all(x, asked);
    let tiers = [
        top.iter().take(5).find(|x| same(x) && liked(x)).copied(),
        top.iter().find(same).copied(),
        top.iter().find(all).copied(),
        found.iter().take(10).find(|x| has_all(x, asked)),
        top.iter().take(5).find(|x| liked(x)).copied(),
        top.first().copied(),
        found.iter().take(5).find(|x| liked(x)),
        found.first(),
    ];
    tiers.iter().position(Option::is_some).map(|i| (tiers[i].unwrap(), i))
}

pub fn radio_pick(list: &[Track], q: &str) -> Option<Track> {
    let same: Vec<&Track> = list.iter().filter(|t| same_song(t, q) && t.ms >= 90_000).collect();
    let mut radio: Vec<&&Track> = same.iter().filter(|t| is_radio(&t.title)).collect();
    radio.sort_by_key(|t| t.ms);
    if let Some(t) = radio.first() {
        return Some((**t).clone());
    }
    same.into_iter().find(|t| t.ms < LONG_MS).cloned()
}

// ---------- the replies, which you can change in the web app's Settings ----------
/// Each reply, in the order Settings lists them, with its name there (the web app's REQ_MSG_LABELS)
pub const MSG_KEYS: [(&str, &str); 12] = [
    ("added", "Added"), ("playing", "Added, playing now"), ("notFound", "Not found"), ("full", "Queue full"),
    ("perUser", "Viewer limit"), ("duplicate", "Already queued"), ("closed", "Requests off"), ("notSafe", "Not safe on stream"),
    ("empty", "Nothing asked for"), ("wrongSong", "!wrongsong: removed"), ("noRequest", "!wrongsong: none of theirs"), ("noRequests", "!wrongsong: none at all (a mod)"),
];

pub fn default_msg(key: &str) -> &'static str {
    match key {
        "added" => "Added \"{title}\" by {artist} (#{position} in the queue).",
        "playing" => "Added \"{title}\" by {artist}, playing now.",
        "notFound" => "Couldn't find \"{query}\" on {platform}.",
        "full" => "The request queue is full. Try again in a bit!",
        "perUser" => "{user}, you already have {count} requests waiting.",
        "duplicate" => "\"{title}\" is already in the queue.",
        "closed" => "Song requests are closed right now.",
        "wrongSong" => "Removed \"{title}\" by {artist} (asked for by {by}).",
        "noRequest" => "{user}, you don't have a request waiting.",
        "noRequests" => "There's no request waiting.",
        "notSafe" => "Only music that's safe on stream plays right now, and nothing for \"{query}\" is.",
        _ => "Tell me what to play, for example: artist - song",
    }
}

pub fn msg(prefs: &Value, key: &str, vars: &[(&str, String)]) -> String {
    let custom = prefs["reqMsgs"][key].as_str().map(str::trim).filter(|s| !s.is_empty());
    let mut s = custom.unwrap_or(default_msg(key)).to_string();
    for (k, v) in vars {
        s = s.replace(&format!("{{{k}}}"), v);
    }
    s
}

// ---------- finding it ----------
fn yt_link_id(q: &str) -> Option<String> {
    for marker in ["watch?v=", "&v=", "shorts/", "youtu.be/"] {
        if let Some(i) = q.find(marker) {
            let id: String = q[i + marker.len()..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
            if id.len() == 11 {
                return Some(id);
            }
        }
    }
    None
}

/// Searches one platform for a request (a link to that platform finds that track)
pub async fn find(plat: &str, q: &str, local: &[Track]) -> Vec<Track> {
    match plat {
        "soundcloud" => {
            if q.contains("soundcloud.com/") {
                let url = q.split('?').next().unwrap_or(q);
                if let Ok(v) = core::soundcloud::api_get(format!("/resolve?url={}", crate::sources::urlencode(url))).await {
                    if let Some(t) = Track::soundcloud(&v) {
                        return vec![t];
                    }
                }
            }
            crate::sources::sc_search(q, 10).await
        }
        "spotify" => {
            if let Some(id) = q.split("/track/").nth(1).map(|s| s.chars().take_while(|c| c.is_ascii_alphanumeric()).collect::<String>()).filter(|s| !s.is_empty()) {
                if let Ok(v) = core::spotify::spotify_api("GET".into(), format!("/tracks/{id}?market=from_token"), None).await {
                    if let Some(t) = Track::spotify(&v) {
                        return vec![t];
                    }
                }
            }
            crate::sources::sp_search(q).await
        }
        "youtube" => match yt_link_id(q) {
            Some(id) => core::youtube::radio(&id).await.map(|l| l.iter().map(Track::youtube).filter(|t| matches!(&t.src, Src::YouTube(x) if *x == id)).take(1).collect()).unwrap_or_default(),
            None => core::youtube::search(q).await.map(|l| l.iter().map(Track::youtube).collect()).unwrap_or_default(),
        },
        "local" => {
            let want = words(q);
            local.iter().filter(|t| { let have = words(&format!("{} {} {}", t.title, t.artist, t.album)); want.iter().all(|w| have.contains(w)) }).take(10).cloned().collect()
        }
        _ => Vec::new(),
    }
}

/// The track for a request: a liked one with every word asked for, or the first match on the first
/// platform that has one (a liked one among its top five preferred), then a radio edit if wanted.
/// Err is the reply's key (notFound / empty).
pub async fn pick(q: &str, plats: &[&'static str], likes: &[Track], local: &[Track], prefer_radio: bool, safe: &(dyn Fn(&Track) -> bool + Sync)) -> Result<Track, &'static str> {
    let is_link = q.starts_with("http://") || q.starts_with("https://");
    let order: Vec<&str> = if q.contains("soundcloud.com/") {
        plats.iter().copied().filter(|p| *p == "soundcloud").collect()
    } else if q.contains("open.spotify.com/") {
        plats.iter().copied().filter(|p| *p == "spotify").collect()
    } else if yt_link_id(q).is_some() {
        plats.iter().copied().filter(|p| *p == "youtube").collect()
    } else {
        plats.to_vec()
    };
    let qs = if is_link { vec![(q.to_string(), None)] } else { queries(q) };
    if qs.is_empty() {
        return Err("empty");
    }
    let liked = |t: &Track| likes.iter().any(|l| l.key == t.key);
    let mut asked = qs[0].0.clone();
    let mut found: Vec<Track> = Vec::new();
    let mut chosen: Option<Track> = None;
    if !is_link {
        'likes: for (qq, _) in &qs {
            let want = words(qq);
            if want.is_empty() {
                continue;
            }
            // the same song, not just one with those words in it ("Beauty and a Strobe" isn't "Strobe")
            for t in likes.iter().filter(|t| order.contains(&t.plat()) && !t.is_mix() && safe(t)) {
                if same_song(t, qq) {
                    chosen = Some(t.clone());
                    asked = qq.clone();
                    break 'likes;
                }
            }
        }
    }
    if chosen.is_none() {
        // the song itself (yours if you like it) on the first platform that has it; if none has it,
        // the best that was found anywhere (with Stream-Safe Music on, only what's safe; none of it at
        // all is its own reply)
        let mut unsafe_only = false;
        let mut best: Option<(usize, Vec<Track>, String, usize)> = None;   // (tier, found, asked, at)
        'search: for (qq, artist) in &qs {
            for p in &order {
                let mut got = by_artist(find(p, qq, local).await.into_iter().filter(|t| !t.is_mix()).collect(), artist);
                let all = got.len();
                got.retain(|t| safe(t));
                unsafe_only |= all > 0 && got.is_empty();
                let r = if is_link { got.first().map(|_| (0usize, 0usize)) } else { rank(&got, qq, &liked).map(|(t, tier)| (tier, got.iter().position(|x| std::ptr::eq(x, t)).unwrap_or(0))) };
                if let Some((tier, at)) = r {
                    if best.as_ref().is_none_or(|b| tier < b.0) {
                        best = Some((tier, got, qq.clone(), at));
                    }
                }
                if best.as_ref().is_some_and(|b| b.0 <= 1) {
                    break 'search;
                }
            }
        }
        let Some((_, got, best_asked, at)) = best else {
            return Err(if unsafe_only { "notSafe" } else { "notFound" });
        };
        found = got;
        asked = best_asked;
        chosen = Some(found[at].clone());
    }
    let mut t = chosen.unwrap();
    if prefer_radio && !is_link && t.ms >= LONG_MS && !is_radio(&t.title) {
        if let Some(r) = radio_pick(&found, &asked) {
            t = r;
        } else {
            for p in &order {
                let more: Vec<Track> = find(p, &format!("{asked} radio edit"), local).await.into_iter().filter(|x| !x.is_mix() && safe(x)).collect();
                if let Some(r) = radio_pick(&more, &asked) {
                    t = r;
                    break;
                }
            }
        }
    }
    Ok(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accents_dont_matter() {
        assert_eq!(words("Beyoncé - Halo"), words("beyonce halo"));
    }

    fn t(title: &str, artist: &str, ms: u64) -> Track {
        Track { src: Src::YouTube(title.into()), key: title.into(), title: title.into(), artist: artist.into(), album: String::new(), ms, art: String::new(), genre: String::new(), link: String::new(), go_plus: false, ..Default::default() }
    }

    #[test]
    fn requests_are_tidied_and_by_is_understood() {
        assert_eq!(clean("Artist - Song: Part 2 (Live!)"), "artist song part 2 live");
        assert_eq!(clean("P!nk - 'So What'"), "p!nk so what");
        let q = queries("I Remember by deadmau5");
        assert_eq!(q[0], ("deadmau5 i remember".to_string(), Some("deadmau5".to_string())));
        assert_eq!(q[1].0, "i remember by deadmau5");
        assert_eq!(yt_link_id("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=x").as_deref(), Some("dQw4w9WgXcQ"));
    }

    #[test]
    fn a_radio_edit_of_the_same_song_is_found() {
        let list = vec![t("Strobe", "deadmau5", 637_000), t("Strobe (Radio Edit)", "deadmau5", 213_000), t("Strobe (Club Mix) Remix Contest", "x", 400_000)];
        assert!(same_song(&list[1], "deadmau5 strobe"));
        // a liked song with the words in it isn't the song asked for
        assert!(!same_song(&t("BEAUTY AND A STROBE (REEMIX) [FREE DOWNLOAD]", "JUSTIN BIEBER VS DEADMAU5", 200_000), "deadmau5 strobe"));
        assert!(!same_song(&list[2], "deadmau5 strobe"));
        assert_eq!(radio_pick(&list, "deadmau5 strobe").unwrap().title, "Strobe (Radio Edit)");
        let prefs = serde_json::json!({ "reqMsgs": { "added": "  " } });
        assert_eq!(msg(&prefs, "added", &[("title", "Strobe".into()), ("artist", "deadmau5".into()), ("position", "2".into())]), "Added \"Strobe\" by deadmau5 (#2 in the queue).");
    }

    #[test]
    fn the_song_itself_beats_a_liked_bootleg_unless_one_is_asked_for() {
        let boot = t("Colourfast (Labmig & Gimbal On & On Epic Bootleg)", "Pendulum", 240_000);
        let orig = t("Colourfast", "Pendulum", 230_000);
        let found = vec![boot.clone(), orig.clone()];
        let liked = |x: &Track| x.title == boot.title;
        assert_eq!(rank(&found, "pendulum colourfast", &liked).map(|(x, tier)| (x.title.as_str(), tier)), Some(("Colourfast", 1)));
        assert_eq!(rank(&found, "pendulum colourfast bootleg", &liked).map(|(x, _)| x.title.as_str()), Some(boot.title.as_str()));
        // a radio edit or a remaster is the song itself
        assert!(!is_version("Colourfast (Radio Edit)") && !is_version("Invisible Touch - 2007 Remaster"));
        assert!(is_version("Colourfast (VIP)") && is_version("Colourfast (Sub Focus Remix)"));
    }

}
