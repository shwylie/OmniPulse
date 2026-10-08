// Track titles, as the web app tidies them (dist/js/titles.js): promo tags dropped ("FREE DL",
// "[OUT NOW]"), "(feat. X)" taken out of the title with X added to the artists (and a repeat of it,
// or of any tag, kept once), and the album line only when it says something new.

use regex::Regex;
use std::sync::OnceLock;

fn re(cell: &'static OnceLock<Regex>, src: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(src).expect("a title pattern"))
}

const JUNK: &str = r"(?i)\b(?:free\s*(?:dl|d/l|download|dwnld)|free\s*=?\s*buy|buy\s*=\s*free|click\s*buy|out\s*now|premiere|exclusive|supported\s+by\b.*|played\s+by\b.*|download\s+(?:link|in\s+desc\w*)|link\s+in\s+(?:bio|desc\w*)|hypeddit|toneden)";
const SEP_JUNK: &str = r"(?i)\b(?:free\s*(?:dl|d/l|download|dwnld)|click\s*buy|buy\s*=\s*free|out\s*now|supported\s+by|played\s+by|link\s+in)\b";
const KEEP: &str = r"(?i)\b(?:re-?mix|mix|edit|bootleg|flip|vip|feat\.?|ft\.?|prod\.?|rework|cover|version|instrumental|acapella|live|remaster\w*)\b";
const ALWAYS_JUNK: &str = r"(?i)\s*\b(?:free\s*(?:dl|d/l|download|dwnld)|click\s*buy|buy\s*=\s*free|free\s*=\s*buy|out\s*now|link\s+in\s+(?:bio|desc\w*))\b[!.]*";

/// A title without promo tags; the title as it was if nothing would be left
pub fn clean_title(title: &str) -> String {
    static LEAD: OnceLock<Regex> = OnceLock::new();
    static BRACKETS: OnceLock<Regex> = OnceLock::new();
    static JUNK_RE: OnceLock<Regex> = OnceLock::new();
    static KEEP_RE: OnceLock<Regex> = OnceLock::new();
    static SEP: OnceLock<Regex> = OnceLock::new();
    static SEP_JUNK_RE: OnceLock<Regex> = OnceLock::new();
    static STARS: OnceLock<Regex> = OnceLock::new();
    static ALWAYS: OnceLock<Regex> = OnceLock::new();
    static SPACES: OnceLock<Regex> = OnceLock::new();
    static BEFORE_CLOSE: OnceLock<Regex> = OnceLock::new();
    static ENDS: OnceLock<Regex> = OnceLock::new();
    let junk = re(&JUNK_RE, JUNK);
    let keep = re(&KEEP_RE, KEEP);
    let mut t = re(&LEAD, r"(?i)^\s*(?:premiere|exclusive)\s*[:|\-\x{2013}\x{2014}]\s*").replace(title, "").into_owned();
    // bracketed tags: (FREE DL), [OUT NOW]; "(Exclusive Remix)" keeps "(Remix)"
    t = re(&BRACKETS, r"[(\[{\x{3010}]([^)\]}\x{3011}]*)[)\]}\x{3011}]")
        .replace_all(&t, |c: &regex::Captures| {
            let whole = &c[0];
            let inner = &c[1];
            if !junk.is_match(inner) {
                return whole.to_string();
            }
            if !keep.is_match(inner) {
                return " ".into();
            }
            let kept = junk.replace_all(inner, " ");
            let kept = kept.split_whitespace().collect::<Vec<_>>().join(" ");
            if kept.is_empty() {
                " ".into()
            } else {
                let open = whole.chars().next().unwrap_or('(');
                let close = whole.chars().last().unwrap_or(')');
                format!("{open}{kept}{close}")
            }
        })
        .into_owned();
    // promo text after a separator: "Title - FREE DL", "Title | Supported by ..."
    t = re(&SEP, r"\s+(?:\||//|-{1,2}|\x{2013}|\x{2014}|~)\s+([^|\-\x{2013}\x{2014}~]+)$")
        .replace(&t, |c: &regex::Captures| if re(&SEP_JUNK_RE, SEP_JUNK).is_match(&c[1]) && !keep.is_match(&c[1]) { String::new() } else { c[0].to_string() })
        .into_owned();
    // **FREE DOWNLOAD**
    t = re(&STARS, r"\*+([^*]*)\*+").replace_all(&t, |c: &regex::Captures| if junk.is_match(&c[1]) { " ".to_string() } else { c[0].to_string() }).into_owned();
    t = re(&ALWAYS, ALWAYS_JUNK).replace_all(&t, " ").into_owned();
    t = re(&SPACES, r"\s{2,}").replace_all(&t, " ").into_owned();
    t = re(&BEFORE_CLOSE, r"\s+([,)\]])").replace_all(&t, "$1").into_owned();
    t = re(&ENDS, r"^[\s\-\x{2013}\x{2014}|~:]+|[\s\-\x{2013}\x{2014}|~:]+$").replace_all(&t, "").trim().to_string();
    if t.is_empty() { title.to_string() } else { t }
}

fn name_key(s: &str) -> String {
    s.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()
}

fn split_names(s: &str) -> Vec<String> {
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    re(&SPLIT, r"(?i)\s*(?:,|&|\band\b|\bx\b|\bvs\.?\b)\s*").split(s).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect()
}

/// "Song (feat. X) (feat. X)" by A: the title "Song", by "A, X". Any tag twice is kept once.
pub fn tidy_feat(title: &str, artist: &str) -> (String, String) {
    static FEAT: OnceLock<Regex> = OnceLock::new();
    static TAG: OnceLock<Regex> = OnceLock::new();
    static SPACES: OnceLock<Regex> = OnceLock::new();
    let mut featured: Vec<String> = Vec::new();
    let t = re(&FEAT, r"(?i)\s*[(\[]\s*(?:feat\.?|ft\.?|featuring)\s+([^)\]]+)[)\]]|\s+(?:feat\.?|ft\.?|featuring)\s+([^()\[\]]+?)\s*$")
        .replace_all(title, |c: &regex::Captures| {
            featured.extend(split_names(c.get(1).or(c.get(2)).map_or("", |m| m.as_str())));
            String::new()
        })
        .into_owned();
    let mut seen = std::collections::HashSet::new();
    let t = re(&TAG, r"\s*([(\[])([^)\]]+)([)\]])").replace_all(&t, |c: &regex::Captures| if seen.insert(name_key(&c[2])) { c[0].to_string() } else { String::new() }).into_owned();
    let t = re(&SPACES, r"\s{2,}").replace_all(&t, " ").trim().to_string();
    let title = if t.is_empty() { title.to_string() } else { t };
    let mut have: std::collections::HashSet<String> = split_names(artist).iter().map(|n| name_key(n)).collect();
    let add: Vec<String> = featured.into_iter().filter(|n| { let k = name_key(n); !k.is_empty() && have.insert(k) }).collect();
    let artist = if add.is_empty() { artist.to_string() } else { std::iter::once(artist.to_string()).filter(|a| !a.is_empty()).chain(add).collect::<Vec<_>>().join(", ") };
    (title, artist)
}

/// The album, unless the title and artist already say all it does ("Artist - Song", a single)
pub fn album_shown(album: &str, title: &str, artist: &str) -> String {
    const FILLER: [&str; 10] = ["single", "ep", "the", "a", "feat", "ft", "featuring", "x", "and", "vs"];
    let words = |s: &str| s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect::<Vec<_>>();
    if album.trim().is_empty() {
        return String::new();
    }
    let have: std::collections::HashSet<String> = words(&format!("{title} {artist}")).into_iter().collect();
    if words(album).iter().all(|w| have.contains(w) || FILLER.contains(&w.as_str())) { String::new() } else { album.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn promo_tags_go_and_versions_stay() {
        assert_eq!(clean_title("Song [FREE DL]"), "Song");
        assert_eq!(clean_title("Premiere: Artist - Song"), "Artist - Song");
        assert_eq!(clean_title("Song (Exclusive Remix)"), "Song (Remix)");
        assert_eq!(clean_title("Song - OUT NOW"), "Song");
        assert_eq!(clean_title("Song **FREE DOWNLOAD**"), "Song");
        assert_eq!(clean_title("Song (VIP Mix)"), "Song (VIP Mix)");
        assert_eq!(clean_title("FREE DL"), "FREE DL");   // nothing else: left as it was
    }

    #[test]
    fn featured_artists_move_to_the_artists() {
        assert_eq!(tidy_feat("Song (feat. Zed) (feat. Zed)", "Ava"), ("Song".into(), "Ava, Zed".into()));
        assert_eq!(tidy_feat("Song ft. Zed & Yuna", "Ava, Zed"), ("Song".into(), "Ava, Zed, Yuna".into()));
        assert_eq!(tidy_feat("Song (Remix) (Remix)", "A"), ("Song (Remix)".into(), "A".into()));
        assert_eq!(tidy_feat("Song", "A"), ("Song".into(), "A".into()));
    }

    #[test]
    fn the_album_only_when_it_says_more() {
        assert_eq!(album_shown("Song", "Song", "A"), "");
        assert_eq!(album_shown("Song - Single", "Song", "A"), "");
        assert_eq!(album_shown("Night Drives", "Song", "A"), "Night Drives");
    }
}
