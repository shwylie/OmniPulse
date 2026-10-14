// Versions of the same song together (artist pages), as the web app's js/artists.js groupVersions: a
// song's radio edit, extended mix, remixes, a live one... under one row. The song itself shows, and the
// rest open beneath it. "Song (Radio Edit)", "Song - Extended Mix" and "Song [VIP]" are all "song".

use crate::track::Track;
use regex::Regex;
use std::sync::LazyLock;

/// What a version adds (in brackets, or after " - "): only these words make a part of a title its
/// version ("Pt. II (Dusk)" is another song)
static VERSION_WORDS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\b(remix|rmx|mix|edit|version|remaster(ed)?|live|vip|flip|bootleg|mash ?up|cover|acoustic|instrumental|extended|radio|rework|refix|dub|demo|sped ?up|slowed|nightcore|club|original|re-?edit|session)\b").unwrap());
static FEAT_INNER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^\s*(feat|ft|featuring|with)\b").unwrap());
static BRACKETS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*[(\[]([^)\]]*)[)\]]").unwrap());
static DASH_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+[-–—|]\s+(.*)$").unwrap());
static FEAT_TAIL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)\s+(feat|ft|featuring)\b.*$").unwrap());
static ARTIST_DASH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(.+?)\s+[-–—]\s+(.+)$").unwrap());
static NOT_WORD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[^a-z0-9À-ɏ]+").unwrap());
static FEAT_BRACKET: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"[(\[]\s*(feat|ft|featuring|with)\b[^)\]]*[)\]]").unwrap());
static FEAT_END: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+(feat|ft|featuring)\b.*$").unwrap());
static ORIGINAL: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\boriginal (mix|version)\b").unwrap());
static REMIXY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(remix|rmx|bootleg|vip|flip|live)\b").unwrap());
static EDITY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(radio|single|album|extended|clean|explicit|remaster(ed)?|edit)\b").unwrap());
static OTHER: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(remix|rmx|bootleg|vip|flip|mash ?up|live|acoustic|cover|rework|refix|dub)\b").unwrap());

fn words(s: &str) -> Vec<String> {
    NOT_WORD.split(&s.to_lowercase()).filter(|w| !w.is_empty()).map(String::from).collect()
}

/// The title, without an "Artist - " in front (only when that part is the artist: "Song - Extended
/// Mix" stays)
fn title_of(t: &Track, page_artist: &str) -> String {
    let Some(m) = ARTIST_DASH.captures(&t.title) else { return t.title.clone() };
    let who: std::collections::HashSet<String> = words(&format!("{} {} {page_artist}", t.artist, t.uploader)).into_iter().collect();
    if words(&m[1]).iter().all(|w| who.contains(w)) { m[2].to_string() } else { t.title.clone() }
}

/// The title without its version parts: the song's own name
fn strip(title: &str) -> String {
    let s = BRACKETS.replace_all(title, |c: &regex::Captures| if VERSION_WORDS.is_match(&c[1]) || FEAT_INNER.is_match(&c[1]) { " ".to_string() } else { c[0].to_string() });
    let s = DASH_TAIL.replace(&s, |c: &regex::Captures| if VERSION_WORDS.is_match(&c[1]) { " ".to_string() } else { c[0].to_string() });
    FEAT_TAIL.replace(&s, " ").into_owned()
}

fn base(t: &Track, page_artist: &str) -> String {
    NOT_WORD.replace_all(&strip(&title_of(t, page_artist)).to_lowercase(), " ").trim().to_string()
}

/// Which goes on top: the song itself (or its original mix), then edits of it, then other versions
fn rank(t: &Track, page_artist: &str) -> u8 {
    let title = title_of(t, page_artist).to_lowercase();
    // (what's added to the song's name: a featured artist isn't a version)
    let bare = FEAT_END.replace(&FEAT_BRACKET.replace_all(&title, " "), " ").into_owned();
    let extra = NOT_WORD.replace_all(&bare, " ").trim().replacen(&base(t, page_artist), "", 1).trim().to_string();
    if extra.is_empty() || ORIGINAL.is_match(&title) && !REMIXY.is_match(&title) { return 0; }
    if EDITY.is_match(&extra) && !OTHER.is_match(&extra) { return 1; }
    2
}

/// The list with each song's versions together: the one on top, and the rest (best first), in the
/// order the songs first came
pub fn group(list: Vec<Track>, page_artist: &str) -> Vec<(Track, Vec<Track>)> {
    let mut order: Vec<String> = Vec::new();
    let mut groups: std::collections::HashMap<String, Vec<(Track, u8, usize)>> = Default::default();
    for (k, t) in list.into_iter().enumerate() {
        let b = base(&t, page_artist);
        // (nothing to go by: on its own)
        let key = if b.chars().count() >= 2 { b } else { format!("\u{0}{k}") };
        let r = rank(&t, page_artist);
        if !groups.contains_key(&key) { order.push(key.clone()); }
        groups.entry(key).or_default().push((t, r, k));
    }
    order.into_iter().map(|key| {
        let mut g = groups.remove(&key).unwrap_or_default();
        g.sort_by_key(|x| (x.1, x.2));
        let mut it = g.into_iter().map(|x| x.0);
        let head = it.next().unwrap();
        (head, it.collect())
    }).collect()
}

/// The song's own name, for More versions' search ("Song", not "Song (Radio Edit)")
pub fn base_title(t: &Track, page_artist: &str) -> String {
    strip(&title_of(t, page_artist)).split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str) -> Track {
        Track { key: title.into(), title: title.into(), artist: "deadmau5".into(), ..Default::default() }
    }

    #[test]
    fn versions_go_under_the_song() {
        let g = group(vec![t("Strobe (Radio Edit)"), t("Strobe"), t("Strobe (Club Edit)"), t("Ghosts 'n' Stuff"), t("Strobe - Extended Mix"), t("deadmau5 - Strobe (Remix)"), t("Pt. II (Dusk)"), t("Pt. II")], "deadmau5");
        let heads: Vec<&str> = g.iter().map(|(h, _)| h.title.as_str()).collect();
        assert_eq!(heads, ["Strobe", "Ghosts 'n' Stuff", "Pt. II (Dusk)", "Pt. II"]);
        let strobe: Vec<&str> = g[0].1.iter().map(|v| v.title.as_str()).collect();
        assert_eq!(strobe, ["Strobe (Radio Edit)", "Strobe (Club Edit)", "Strobe - Extended Mix", "deadmau5 - Strobe (Remix)"]);
        assert_eq!(base_title(&g[0].1[3], "deadmau5"), "Strobe");
    }

    #[test]
    fn a_featured_artist_isnt_a_version() {
        let g = group(vec![t("Song (feat. Someone)"), t("Song (VIP)")], "deadmau5");
        assert_eq!(g.len(), 1);
        assert_eq!(g[0].0.title, "Song (feat. Someone)");
    }
}
