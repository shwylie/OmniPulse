// Song requests from chat (as in js/requests.js): a bot calls the requests server in core, which asks
// the app; the app finds the song and queues it, and the reply is posted in chat. Punctuation is
// ignored, "song by artist" is understood, the song as most people know it is picked (your own copy
// of it if you like it; SoundCloud only when nothing else has it), links go to their own platform,
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
            // the artist, or the "Artist - " an uploader often puts in front of the title; or the one its
            // version is credited to ("Rapture (Avicii New Generation Mix)" is by Avicii, asked for so)
            let lead = t.title.split_once(" - ").map(|x| x.0).unwrap_or("");
            let have = words(&format!("{} {lead}", t.artist));
            want.iter().all(|w| have.contains(w)) || names_version(t, &format!("{} {}", artist, words(&t.title).join(" ")))
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
    let mark = MARK.get_or_init(|| regex::Regex::new(r"(?i)\b(remix|rmx|bootleg|boot|flip|vip|mash ?up|cover|live|acoustic|instrumental|sped ?up|slowed|nightcore|karaoke|rework|refix|re-?edit|dub|edit|mix|version|vs|bass ?boost(ed)?|8d|reverb(ed)?|tribute|in the style of|piano|orchestral|lullaby|8 ?bit)\b").expect("mark"));
    mark.is_match(&same.replace_all(s, " "))
}

/// A version that's named in what was asked for: a word asked for that's only in its version's credit
/// ("rapture avicii" is Nadia Ali's "Rapture (Avicii New Generation Mix)": Avicii is its remixer)
fn names_version(t: &Track, asked: &str) -> bool {
    // (the title's parts: its name, then each bracket or " - " part after it)
    let title = &t.title;
    let mut cuts: Vec<usize> = [" (", " [", " - "].iter().flat_map(|m| title.match_indices(m).map(|(i, _)| i)).collect();
    cuts.sort_unstable();
    let Some(&first) = cuts.first() else { return false };
    let base = words(&title[..first]);
    let named = |w: &String| !base.contains(w);
    let want = words(asked);
    if !has_all(t, asked) || extra_version(title, asked) { return false; }
    // the part that names what was asked for; and nothing else in it another version (a piano cover of the
    // remix, its VIP, a tribute to it)
    let ends: Vec<usize> = cuts.iter().skip(1).copied().chain([title.len()]).collect();
    cuts.iter().zip(ends).any(|(&a, b)| {
        let part = words(&title[a..b]);
        let rest = format!("{}{}", &title[..a], &title[b..]);
        part.iter().any(|w| want.contains(w) && named(w)) && !is_version(&rest)
    })
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
    let top: Vec<&Track> = found.iter().take(10).filter(|x| want_version || !is_version(&x.title) || names_version(x, asked)).collect();
    let same = |x: &&Track| same_song(x, asked) || names_version(x, asked);
    let all = |x: &&Track| has_all(x, asked);
    // (of those that fit a tier, the most played: the song as most people know it, not an odd re-upload the
    // search happened to list first; on a tie, the search's order)
    fn most<'b>(it: impl Iterator<Item = &'b Track>) -> Option<&'b Track> {
        it.fold(None, |best: Option<&Track>, x| match best { Some(b) if b.plays >= x.plays => Some(b), _ => Some(x) })
    }
    // (every word asked for, and no more of a version than was asked for: "cinema skrillex remix" is the
    // remix, not its flip, its VIP or somebody's re-remix of it)
    let plain = |x: &&Track| has_all(x, asked) && !extra_version(&x.title, asked);
    let tiers = [
        most(top.iter().take(5).copied().filter(|x| same(x) && liked(x))),
        most(top.iter().copied().filter(same)),
        most(top.iter().copied().filter(plain)),
        most(top.iter().copied().filter(all)),
        most(found.iter().take(10).filter(|x| has_all(x, asked))),
        most(top.iter().take(5).copied().filter(|x| liked(x))),
        top.first().copied(),
        most(found.iter().take(5).filter(|x| liked(x))),
        found.first(),
    ];
    tiers.iter().position(Option::is_some).map(|i| (tiers[i].unwrap(), i))
}

/// A title that's a version of a version nobody asked for: a flip, a VIP, a re-remix, a bootleg...
fn extra_version(title: &str, asked: &str) -> bool {
    const MORE: [&str; 14] = ["flip", "vip", "v.i.p", "re-remix", "reremix", "bootleg", "reboot", "mashup", "rework", "sped up", "slowed", "nightcore", "cover", "edit"];
    let (t, a) = (title.to_lowercase(), asked.to_lowercase());
    MORE.iter().any(|w| t.contains(w) && !a.contains(w))
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
        "musicbrainz" => core::musicbrainz::search(q).await.map(|l| l.iter().filter_map(Track::musicbrainz).collect()).unwrap_or_default(),
        "catalog" => core::catalog::search(q).await.map(|l| l.iter().filter_map(Track::catalog).collect()).unwrap_or_default(),
        "plugins" => crate::sources::search_plugins(q).await,
        "local" => {
            let want = words(q);
            local.iter().filter(|t| { let have = words(&format!("{} {} {}", t.title, t.artist, t.album)); want.iter().all(|w| have.contains(w)) }).take(10).cloned().collect()
        }
        _ => Vec::new(),
    }
}

/// Whether a track is by `artist` (the first name of it, as "Rick Astley" of "Rick Astley, X"): its artist,
/// or the "Artist - " an uploader puts in front of its title
fn same_artist(t: &Track, artist: &str) -> bool {
    let main = artist.split([',', '&', '/']).next().unwrap_or(artist);
    let want = words(&main.replace(" feat.", ",").replace(" ft.", ","));
    let lead = t.title.split_once(" - ").map(|x| x.0).unwrap_or("");
    let have = words(&format!("{} {lead}", t.artist));
    !want.is_empty() && want.iter().all(|w| have.contains(w))
}

/// The track for a request. The song as most people know it: every platform but SoundCloud searched
/// (at once), and of the best matches the most played; SoundCloud only if none of them has the song
/// itself (its uploads are where the odd versions are: bass-boosted, tributes, live mashups). One of
/// your likes is played instead only if it's that song by the same artist (your copy of it, not
/// somebody's cover that you happen to like). Then a radio edit if wanted.
/// Err is the reply's key (notFound / empty / notSafe).
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
    // (the platforms in two rounds: all but SoundCloud first, SoundCloud only if they don't have it)
    let first: Vec<&str> = if is_link { order.clone() } else { order.iter().copied().filter(|p| *p != "soundcloud").collect() };
    let fallback: Vec<&str> = if is_link { Vec::new() } else { order.iter().copied().filter(|p| *p == "soundcloud").collect() };
    let mut unsafe_only = false;
    // (tier, plays, found, asked, at): the best so far
    let mut best: Option<(usize, u64, Vec<Track>, String, usize)> = None;
    let better = |tier: usize, plays: u64, b: &Option<(usize, u64, Vec<Track>, String, usize)>| b.as_ref().is_none_or(|b| tier < b.0 || (tier == b.0 && plays > b.1));
    for round in [&first, &fallback] {
        if round.is_empty() || best.as_ref().is_some_and(|b| b.0 <= 1) { continue; }
        for (qq, artist) in &qs {
            let lists = futures_util::future::join_all(round.iter().map(|p| find(p, qq, local))).await;
            for list in lists {
                let mut got = by_artist(list.into_iter().filter(|t| !t.is_mix()).collect(), artist);
                let all = got.len();
                got.retain(|t| safe(t));
                unsafe_only |= all > 0 && got.is_empty();
                let r = if is_link { got.first().map(|_| (0usize, 0usize)) } else { rank(&got, qq, &|_| false).map(|(t, tier)| (tier, got.iter().position(|x| std::ptr::eq(x, t)).unwrap_or(0))) };
                if let Some((tier, at)) = r {
                    let plays = got[at].plays;
                    if better(tier, plays, &best) { best = Some((tier, plays, got, qq.clone(), at)); }
                }
            }
            if best.as_ref().is_some_and(|b| b.0 <= 1) { break; }
        }
    }
    // your copy of it, if you like it: the same song by the same artist
    let liked_copy = |song: &str, artist: Option<&str>| likes.iter().filter(|t| order.contains(&t.plat()) && !t.is_mix() && safe(t))
        .find(|t| same_song(t, song) && artist.is_none_or(|a| same_artist(t, a))).cloned();
    let (mut t, found, asked) = match best {
        Some((tier, _, got, asked, at)) if !is_link => {
            let top = got[at].clone();
            // (only for the song itself: a like is never swapped in for a near miss)
            let mine = (tier <= 1).then(|| liked_copy(&asked, Some(&top.artist))).flatten();
            (mine.unwrap_or(top), got, asked)
        }
        Some((_, _, got, asked, at)) => (got[at].clone(), got, asked),
        // nothing found anywhere: one of your likes with that title, if there is one
        None => match (!is_link).then(|| qs.iter().find_map(|(qq, artist)| liked_copy(qq, artist.as_deref()).map(|t| (t, qq.clone())))).flatten() {
            Some((t, qq)) => (t, Vec::new(), qq),
            None => return Err(if unsafe_only { "notSafe" } else { "notFound" }),
        },
    };
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

    /// A request as chat would make it, against the real searches: OMNIPULSE_REQ="cinema skrillex remix"
    #[test]
    #[ignore]
    fn request_probe() {
        let q = std::env::var("OMNIPULSE_REQ").unwrap();
        let rt = tokio::runtime::Runtime::new().unwrap();
        for p in ["soundcloud", "spotify"] {
            for t in rt.block_on(find(p, &q, &[])).iter().take(10) { println!("  {p}: {} - {} ({} plays, {} ms) same {} all {} version {}", t.artist, t.title, t.plays, t.ms, same_song(t, &q), has_all(t, &q), is_version(&t.title)); }
        }
        for plats in [vec!["soundcloud", "spotify", "youtube", "catalog"]] {
            let r = rt.block_on(pick(&q, &plats, &[], &[], false, &|_| true));
            println!("{plats:?}: {:?}", r.map(|t| format!("{} - {} ({} plays, {})", t.artist, t.title, t.plays, t.plat())));
        }
    }

    #[test]
    fn of_the_same_song_the_most_played_is_picked() {
        let t = |k: &str, title: &str, artist: &str, plays: u64| Track { key: k.into(), title: title.into(), artist: artist.into(), plays, ..Default::default() };
        let found = vec![
            t("odd", "Cinema (Skrillex Remix)", "some uploader", 900),
            t("vip", "Cinema (Skrillex Remix) - Final Version V.I.P", "Benny Benassi", 120_000),
            t("real", "Cinema (Skrillex Remix)", "Benny Benassi", 40_000_000),
            t("other", "Cinema", "Benny Benassi", 90_000_000),
        ];
        let (got, _) = rank(&found, "cinema skrillex remix", &|_| false).unwrap();
        assert_eq!(got.key, "real");
        // (only versions of it: the one that's no more than was asked for, before the most played VIP)
        let only = vec![t("vip", "Cinema (Skrillex Remix) - Final Version V.I.P", "Benny Benassi", 120_000), t("rr", "Cinema (Skrillex Remix) [Some Re-Remix]", "X", 60_000), t("plain", "Cinema (Skrillex Remix)", "uploader", 5_000)];
        assert_eq!(rank(&only, "cinema skrillex remix", &|_| false).unwrap().0.key, "plain");
    }

    #[test]
    fn a_liked_cover_isnt_the_song_and_altered_uploads_are_versions() {
        let cover = t("Never Gonna Give You Up", "RadioClub", 200_000);
        assert!(same_artist(&t("Never Gonna Give You Up", "Rick Astley", 213_000), "Rick Astley"));
        assert!(!same_artist(&cover, "Rick Astley"));
        assert!(same_artist(&t("Rick Astley - Never Gonna Give You Up", "some uploader", 213_000), "Rick Astley, X"));
        assert!(names_version(&t("Rapture - Avicii New Generation Mix", "Nadia Ali", 221_000), "avicii rapture"));
        assert!(!names_version(&t("Rapture (Avicii Remix) - Piano Version", "Pianostalgia FM", 208_000), "avicii rapture"));
        assert!(is_version("Chelsea Dagger (Bass Boosted)") && is_version("Dancing Queen (8D Audio)") && is_version("Rapture - ABBA Tribute"));
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
