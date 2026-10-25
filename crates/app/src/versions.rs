// Go+ tracks on YouTube Music: SoundCloud gives this app only a 30-second preview of them, so they
// play from YouTube Music, matched as the web app matches (dist/js/youtube.js, ytMatchScore): the
// title's words, the artist, the length and version tags (a remix isn't the original). The best
// close match plays; with none close enough, the closest does, and the status line says so. A closest
// version heard to the end is kept for that track. The preview itself never plays.
//
// Matches are kept in state/yt-versions.json, on top of the web app's own (sc.ytMatch, from its copy).

use crate::track::{Src, Track};
use omnipulse_core::youtube::YtTrack;
use regex::Regex;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

static SAVED: Mutex<Option<HashMap<String, Value>>> = Mutex::new(None);

fn file() -> Option<std::path::PathBuf> {
    omnipulse_core::data_dir().ok().map(|d| d.join("state").join("yt-versions.json"))
}

fn with_saved<R>(f: impl FnOnce(&mut HashMap<String, Value>) -> R) -> R {
    let mut g = SAVED.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(|| {
        file().and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default()
    });
    f(map)
}

/// Its YouTube Music version, if one's known already (for share links)
pub fn saved_id(key: &str) -> Option<String> {
    with_saved(|m| m.get(key).and_then(|v| v["id"].as_str().map(String::from)))
}

fn save_all() {
    let text = with_saved(|m| serde_json::to_string(m).unwrap_or_default());
    if let Some(p) = file() {
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        let _ = std::fs::write(p, text);
    }
}

/// The web app's matches, under this app's own (yours, chosen here, win)
pub fn seed_from_web(web: &Value) {
    let Some(o) = web.as_object() else { return };
    with_saved(|m| {
        for (k, v) in o {
            if v["id"].is_string() && !m.get(k).is_some_and(|x| x["id"].is_string()) {
                m.insert(k.clone(), v.clone());
            }
        }
    });
}

fn snap(c: &YtTrack) -> Value {
    json!({ "id": c.id, "title": c.title, "artists": c.artists, "album": c.album, "duration_ms": c.duration_ms, "thumb": c.thumb, "explicit": c.explicit })
}

fn from_snap(v: &Value) -> Option<YtTrack> {
    Some(YtTrack {
        id: v["id"].as_str()?.to_string(),
        title: v["title"].as_str().unwrap_or("").to_string(),
        artists: v["artists"].as_str().unwrap_or("").to_string(),
        album: v["album"].as_str().unwrap_or("").to_string(),
        duration_ms: v["duration_ms"].as_u64().unwrap_or(0),
        thumb: v["thumb"].as_str().unwrap_or("").to_string(),
        explicit: v["explicit"].as_bool().unwrap_or(false),
        ..Default::default()
    })
}

// ---------- scoring, as the web app's ----------
fn rx(cell: &'static OnceLock<Regex>, src: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(src).expect("a match pattern"))
}

const VERSION_WORDS: [&str; 15] = ["remix", "edit", "mix", "vip", "flip", "bootleg", "live", "acoustic", "instrumental", "remaster", "cover", "sped", "slowed", "nightcore", "karaoke"];
/// Versions that are never the song asked for, however close the rest: not a match at all (in its title,
/// or its album's: "The Atlas Underground (Instrumentals)")
const NEVER_WORDS: [&str; 10] = ["instrumental", "instrumentals", "karaoke", "cover", "live", "acoustic", "sped", "slowed", "nightcore", "reverb"];

/// "Artist - Song" (or Spotify's own), its artist and title, without bracketed tags
pub fn guess_names(t: &Track) -> (String, String) {
    static DASH: OnceLock<Regex> = OnceLock::new();
    static SP_SUFFIX: OnceLock<Regex> = OnceLock::new();
    static BRACKETS: OnceLock<Regex> = OnceLock::new();
    let (mut artist, mut title) = (t.artist.trim().to_string(), t.title.trim().to_string());
    if matches!(t.src, Src::Spotify(_)) {
        artist = artist.split(',').next().unwrap_or("").trim().to_string();
        let cut = rx(&SP_SUFFIX, r"\s+[-\x{2013}\x{2014}]\s+[^-\x{2013}\x{2014}]*$").replace(&title, "").trim().to_string();
        if !cut.is_empty() { title = cut; }
    } else if let Some(c) = rx(&DASH, r"^(.+?)\s+[-\x{2013}\x{2014}]\s+(.+)$").captures(&title.clone()) {
        artist = c[1].trim().to_string();
        title = c[2].trim().to_string();
    }
    let bare = rx(&BRACKETS, r"\s*[(\[][^)\]]*[)\]]\s*").replace_all(&title, " ").split_whitespace().collect::<Vec<_>>().join(" ");
    (artist, if bare.is_empty() { title } else { bare })
}

/// A title without what uploads add ("(Official Audio)", "| Official Video")
fn upload_title(s: &str) -> String {
    static TAGS: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    static BAR: OnceLock<Regex> = OnceLock::new();
    let tag = rx(&TAG, r"(?i)\b(official|audio|video|lyrics?|visuali[sz]er|hd|hq|4k|mv|m/v)\b");
    let s = rx(&TAGS, r"[(\[]([^)\]]*)[)\]]").replace_all(s, |c: &regex::Captures| if tag.is_match(&c[1]) { " ".to_string() } else { c[0].to_string() });
    rx(&BAR, r"\s[|\x{FF5C}]\s.*$").replace(&s, " ").into_owned()
}

/// The title to match on: without "Artist - " (SoundCloud and uploads put it there), with its version
pub fn match_title(t: &Track) -> String {
    static DASH: OnceLock<Regex> = OnceLock::new();
    let title = t.title.trim();
    let title = if matches!(t.src, Src::Spotify(_)) { title.to_string() } else {
        rx(&DASH, r"^(.+?)\s+[-\x{2013}\x{2014}]\s+(.+)$").captures(title).map_or(title.to_string(), |c| c[2].to_string())
    };
    upload_title(&title)
}

fn match_words(s: &str) -> Vec<String> {
    static ORIGINAL: OnceLock<Regex> = OnceLock::new();
    static FILLER: OnceLock<Regex> = OnceLock::new();
    let s = rx(&ORIGINAL, r"(?i)[(\[]\s*original( mix| version)?\s*[)\]]|\boriginal mix\b").replace_all(s, " ").to_lowercase();
    let s = rx(&FILLER, r"\b(feat|ft|featuring|prod|with)\b\.?").replace_all(&s, " ");
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// The artist is credited, or named in the title: not a cover. Every word of their name ("the" aside), so
/// Alex Goot isn't Alex Melton
pub fn artist_hit(t: &Track, c: &YtTrack) -> bool {
    let (ga, _) = guess_names(t);
    let artist: Vec<String> = match_words(if ga.is_empty() { &t.artist } else { &ga }).into_iter().filter(|w| w != "the").collect();
    let place: HashSet<String> = match_words(&format!("{} {}", c.artists, c.title)).into_iter().collect();
    artist.is_empty() || artist.iter().all(|w| place.contains(w))
}

pub fn score(t: &Track, c: &YtTrack) -> f32 {
    static MUSIC_VIDEO: OnceLock<Regex> = OnceLock::new();
    let (ga, _) = guess_names(t);
    let artist = match_words(if ga.is_empty() { &t.artist } else { &ga });
    // who's featured ("(with JENNIE)", or credited after the first artist): not part of the title, and
    // a version that credits them is the one (one that doesn't is likely another)
    let (mut feat, bare) = omnipulse_core::ranking::featured(&match_title(t));
    feat.extend(t.artist.split(',').skip(1).map(|a| a.trim().to_string()).filter(|a| !a.is_empty()));
    let feat: Vec<Vec<String>> = feat.iter().map(|f| match_words(f)).filter(|w| !w.is_empty()).collect();
    let want: HashSet<String> = match_words(&bare).into_iter().collect();
    let got: HashSet<String> = match_words(&omnipulse_core::ranking::featured(&upload_title(&c.title)).1).into_iter()
        .filter(|w| !(artist.contains(w) && !want.contains(w)) && !feat.iter().any(|f| f.contains(w) && !want.contains(w))).collect();
    if want.is_empty() || got.is_empty() {
        return 0.0;
    }
    let album: HashSet<String> = match_words(&c.album).into_iter().collect();
    if NEVER_WORDS.iter().any(|v| !want.contains(*v) && (got.contains(*v) || album.contains(*v))) {
        return 0.0;
    }
    let shared = want.iter().filter(|w| got.contains(*w)).count() as f32;
    let mut s = shared / want.len().max(got.len()) as f32;
    if s < 0.75 {
        return 0.0;
    }
    s += if artist_hit(t, c) { 0.3 } else { -0.6 };
    // (theirs, from their own channel, over someone else's upload that names them; their audio over
    // their video, which may have more to it than the song)
    let by: HashSet<String> = match_words(&c.artists).into_iter().collect();
    if !artist.is_empty() && artist.iter().all(|w| by.contains(w)) { s += 0.1; }
    if rx(&MUSIC_VIDEO, r"(?i)[(\[][^)\]]*\bvideo\b[^)\]]*[)\]]").is_match(&c.title) { s -= 0.05; }
    if !feat.is_empty() {
        let place: HashSet<String> = match_words(&format!("{} {}", c.artists, c.title)).into_iter().collect();
        let hits = feat.iter().filter(|f| f.iter().all(|w| place.contains(w))).count();
        s += if hits == feat.len() { 0.3 } else if hits > 0 { 0.1 } else { -0.3 };
    }
    for v in VERSION_WORDS {
        if want.contains(v) != got.contains(v) {
            s -= 0.6;
        }
    }
    let (d1, d2) = (t.ms as f32, c.duration_ms as f32);
    if d1 > 40_000.0 && d2 > 0.0 {
        let off = (d1 - d2).abs() / d1;
        s += if off < 0.03 { 0.3 } else if off < 0.1 { 0.1 } else if off > 0.25 { -0.5 } else { 0.0 };
    }
    s
}

// ---------- finding one ----------
pub struct Version {
    pub yt: YtTrack,
    /// a close match; false: only the closest there was
    pub exact: bool,
}

const NONE_TTL_SECS: u64 = 86_400;

/// The version of a Go+ track to play: one kept from before, the best close match, or the closest
pub async fn find(t: &Track) -> Option<Version> {
    let saved = with_saved(|m| m.get(&t.key).cloned());
    // (one found by itself before who's featured counted ("fv"): looked for again, once; yours stay)
    let featured = !omnipulse_core::ranking::featured(&match_title(t)).0.is_empty() || t.artist.contains(',');
    // (and once more since never-the-song versions (instrumentals...) and explicit ones counted ("xv"))
    let before = saved.as_ref().filter(|s| s["chosen"].as_bool() != Some(true) && ((featured && s["fv"].is_null()) || s["xv"].is_null())).and_then(from_snap);
    if before.is_none() {
        // (one found by itself that isn't by them after all, as a name shared with someone else let through
        // before: looked for again; yours stay)
        let yours = saved.as_ref().is_some_and(|s| s["chosen"].as_bool() == Some(true));
        if let Some(v) = saved.as_ref().and_then(from_snap).filter(|v| yours || artist_hit(t, v)) {
            return Some(Version { yt: v, exact: true });
        }
    }
    // looked for lately and nothing close: the closest from then
    if let Some(s) = saved.as_ref().filter(|s| !s["xv"].is_null()) {
        if s["none"].as_bool() == Some(true) && omnipulse_core::now_secs().saturating_sub(s["at"].as_u64().unwrap_or(0) / 1000) < NONE_TTL_SECS {
            return from_snap(&s["near"]).map(|yt| Version { yt, exact: false });
        }
    }
    let (ga, gt) = guess_names(t);
    let first = format!("{} {}", if ga.is_empty() { &t.artist } else { &ga }, gt);
    let found = match omnipulse_core::youtube::search(first.trim()).await {
        Ok(f) => f,
        Err(_) => return before.map(|yt| Version { yt, exact: true }),
    };
    let mut ranked: Vec<(f32, YtTrack)> = found.into_iter().take(10).map(|c| (score(t, &c), c)).collect();
    let good = |r: &[(f32, YtTrack)]| r.iter().any(|(s, c)| *s >= 0.5 && artist_hit(t, c));
    if !good(&ranked) {
        if let Ok(more) = omnipulse_core::youtube::search(&format!("{} {}", t.artist, match_title(t))).await {
            ranked.extend(more.into_iter().take(10).map(|c| (score(t, &c), c)));
        }
    }
    // (not among its songs: among its videos, as some are only uploaded that way)
    if !good(&ranked) {
        if let Ok(more) = omnipulse_core::youtube::search_videos(first.trim()).await {
            ranked.extend(more.into_iter().take(10).map(|c| (score(t, &c) - 0.05, c)));
        }
    }
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
    let fits: Vec<&(f32, YtTrack)> = ranked.iter().filter(|(s, c)| *s >= 0.5 && artist_hit(t, c)).collect();
    // (of the ones about as good as the best, the explicit one: the song as written, as its lyrics show it)
    let best = fits.first().map(|x| x.0).unwrap_or(0.0);
    let pick = fits.iter().find(|(s, c)| c.explicit && *s >= best - 0.15).or(fits.first()).map(|(_, c)| c.clone());
    // (looked for again and nothing better: the one from before stays, unless it's one never to play)
    let pick = pick.or(before.filter(|b| score(t, b) > 0.0));
    match pick {
        Some(c) => {
            with_saved(|m| m.insert(t.key.clone(), { let mut s = snap(&c); s["fv"] = json!(1); s["xv"] = json!(1); s }));
            save_all();
            Some(Version { yt: c, exact: true })
        }
        None => {
            // (the closest only if it's by the artist and its title's close: never another song)
            let near = ranked.iter().find(|(s, c)| *s > 0.0 && artist_hit(t, c)).map(|(_, c)| c.clone());
            with_saved(|m| m.insert(t.key.clone(), json!({ "none": true, "at": omnipulse_core::now_secs() * 1000, "any": true, "xv": 1, "near": near.as_ref().map(snap) })));
            save_all();
            near.map(|yt| Version { yt, exact: false })
        }
    }
}

/// A closest version heard to the end: it's the one from now on
pub fn keep(t: &Track, yt: &YtTrack) {
    with_saved(|m| m.insert(t.key.clone(), {
        let mut s = snap(yt);
        s["chosen"] = json!(true);
        s
    }));
    save_all();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sc(title: &str, artist: &str, ms: u64) -> Track {
        Track { src: Src::SoundCloud(1), key: "1".into(), title: title.into(), artist: artist.into(), album: String::new(), ms, art: String::new(), genre: String::new(), link: String::new(), go_plus: true, ..Default::default() }
    }
    fn yt(title: &str, artists: &str, ms: u64) -> YtTrack {
        YtTrack { id: "v".into(), title: title.into(), artists: artists.into(), duration_ms: ms, ..Default::default() }
    }

    #[test]
    fn the_version_with_who_is_featured_comes_first() {
        // Deezer's "Dracula (with JENNIE)": the solo "Dracula" is another song (a little shorter, too)
        let t = sc("Dracula (with JENNIE)", "Tame Impala", 209_700);
        let solo = score(&t, &yt("Dracula", "Tame Impala", 206_000));
        let with = score(&t, &yt("Dracula", "Tame Impala, JENNIE", 210_000));
        assert!(with > solo + 0.3, "with {with}, solo {solo}");
        assert!(solo > 0.0);
        // (a song named with "with" in it isn't someone featured)
        assert!(score(&sc("Stay With Me", "Sam Smith", 172_000), &yt("Stay With Me", "Sam Smith", 172_000)) >= 1.0);
    }

    #[test]
    fn someone_sharing_a_first_name_is_someone_else() {
        // (Alex Melton's Drops of Jupiter isn't Alex Goot's, though both are Alex)
        let t = Track { src: Src::Spotify("x".into()), key: "sp:x".into(), title: "Drops of Jupiter".into(), artist: "Alex Melton".into(), ms: 230_000, ..Default::default() };
        assert!(!artist_hit(&t, &yt("Drops Of Jupiter", "Alex Goot, Kurt Hugo Schneider", 233_000)));
        assert!(artist_hit(&t, &yt("Drops of Jupiter", "Alex Melton", 230_000)));
        // ("the" left out: The Weeknd's own upload credits Weeknd sometimes)
        let w = Track { src: Src::Spotify("y".into()), key: "sp:y".into(), title: "Blinding Lights".into(), artist: "The Weeknd".into(), ms: 200_000, ..Default::default() };
        assert!(artist_hit(&w, &yt("Blinding Lights", "Weeknd", 200_000)));
    }

    #[test]
    fn an_instrumental_or_a_cover_is_never_the_song() {
        let t = Track { src: Src::Spotify("x".into()), key: "sp:x".into(), title: "We Don't Need You".into(), artist: "Tom Morello, VIC MENSA".into(), ms: 183_881, ..Default::default() };
        assert_eq!(score(&t, &yt("We Don't Need You (Instrumental)", "Tom Morello", 184_000)), 0.0);
        let mut c = yt("We Don't Need You", "Tom Morello", 184_000);
        c.album = "The Atlas Underground (Instrumentals)".into();
        assert_eq!(score(&t, &c), 0.0);
        assert_eq!(score(&t, &yt("We Don't Need You (Live)", "Tom Morello", 184_000)), 0.0);
        // the song itself, its audio a little over its video
        let audio = score(&t, &yt("Tom Morello - We Don't Need You (ft. Vic Mensa) [Official Audio]", "Tom Morello", 185_000));
        let video = score(&t, &yt("Tom Morello - We Don't Need You (ft. Vic Mensa) [Official Music Video]", "Tom Morello", 188_000));
        assert!(audio >= 1.0 && audio > video, "audio {audio}, video {video}");
    }

    #[test]
    fn scores_as_the_web_app_does() {
        let t = sc("Mara Lune - Track 14", "uploader", 200_000);
        // the same song by the artist, about as long: a match
        assert!(score(&t, &yt("Track 14", "Mara Lune", 205_000)) >= 0.8);
        // another artist's: no
        assert!(score(&t, &yt("Track 14", "Someone Else", 200_000)) < 0.5 || !artist_hit(&t, &yt("Track 14", "Someone Else", 200_000)));
        // a remix isn't the original
        assert!(score(&t, &yt("Track 14 (Extended Mix)", "Mara Lune", 400_000)) < 0.5);
        // upload tags don't count
        assert!(score(&t, &yt("Track 14 (Official Audio)", "Mara Lune", 200_000)) >= 0.8);
        assert!(score(&t, &yt("Something Else", "Mara Lune", 200_000)) == 0.0);
    }

    /// (live: what YouTube Music gives for a Spotify song, and each one's score)
    #[test]
    #[ignore]
    fn live_scores() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for (title, artist, ms) in [("I Will Not Break", "Disturbed", 292_058u64), ("We Don't Need You", "Tom Morello, VIC MENSA", 183_881), ("You Can Get It", "Arkells, K.Flay", 176_445)] {
            let t = Track { src: Src::Spotify("x".into()), key: "sp:x".into(), title: title.into(), artist: artist.into(), ms, ..Default::default() };
            let (ga, gt) = guess_names(&t);
            let q = format!("{ga} {gt}");
            let found = rt.block_on(omnipulse_core::youtube::search(&q)).unwrap();
            let mut found = found; found.extend(rt.block_on(omnipulse_core::youtube::search_videos(&q)).unwrap().into_iter().map(|mut c| { c.title = format!("[video] {}", c.title); c }));
            println!("== {q}");
            for c in found.iter() { println!("{:.2} hit {} explicit {} | {} | {} | {} | {}", score(&t, c), artist_hit(&t, c), c.explicit, c.title, c.artists, c.album, c.duration_ms); }
        }
    }
}
