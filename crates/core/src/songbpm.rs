// Songs' keys from SongBPM (songbpm.com): Spotify's own analysis, shown on public pages, no account.
// A song's page is at /@artist/title (both made into slugs); failing that, its search (a form post,
// answered with a page of results, each with its own page). A page is only taken if its length is the
// song's (within 4%) and its title has the song's words: a remix or an edit isn't the original, and
// its key may not be either. Used to trust a key before Seamless Mix shifts a song's pitch: our own
// readings (core::rhythm) put a song a fifth out often enough to make a clash worse.

use std::time::Duration;

const ORIGIN: &str = "https://songbpm.com";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/140.0.0.0 Safari/537.36";
/// How close its length has to be to the song's
const LENGTH_WITHIN: f64 = 0.04;
/// Results from a search looked at, at most
const RESULTS: usize = 4;

#[derive(Clone, Debug, PartialEq)]
pub struct SongKey {
    /// the key's tonic (0 C .. 11 B) and its mode
    pub tonic: usize,
    pub minor: bool,
    pub bpm: f64,
    /// its length, as SongBPM has it
    pub ms: u64,
    /// the page it came from
    pub url: String,
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent(USER_AGENT).timeout(Duration::from_secs(12)).build().unwrap_or_default()
}

/// A name as SongBPM's addresses have it: lower case, words joined by hyphens, apostrophes dropped
pub fn slug(s: &str) -> String {
    let s: String = s.to_lowercase().replace(['\'', '\u{2019}'], "").replace('&', " and ");
    s.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join("-")
}

fn words(s: &str) -> Vec<String> {
    slug(s).split('-').filter(|w| !w.is_empty() && !matches!(*w, "feat" | "ft" | "with" | "the" | "a")).map(String::from).collect()
}

/// A page's text, its tags taken out
fn text(html: &str) -> String {
    let mut out = String::with_capacity(html.len() / 2);
    let mut tag = false;
    for c in html.chars() {
        match c {
            '<' => { tag = true; out.push(' '); }
            '>' => tag = false,
            _ if !tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A note's pitch class ("G♯/A♭", "C", "B♭")
fn pitch_class(s: &str) -> Option<usize> {
    let mut ch = s.trim().chars();
    let base = match ch.next()? { 'C' => 0, 'D' => 2, 'E' => 4, 'F' => 5, 'G' => 7, 'A' => 9, 'B' => 11, _ => return None };
    let shift = match ch.next() { Some('♯') | Some('#') => 1, Some('♭') | Some('b') => -1, _ => 0 };
    Some(((base + shift + 12) % 12) as usize)
}

/// A song's page read: its key, mode, tempo and length
pub fn parse_page(html: &str, url: &str) -> Option<SongKey> {
    let t = text(html);
    let at = t.find("Key ")?;
    let rest = &t[at + 4..];
    let key = rest.split_whitespace().next()?;
    let tonic = pitch_class(key.split('/').next()?)?;
    let dur = t.find("Duration ").map(|i| &t[i + 9..]).and_then(|d| d.split_whitespace().next()).and_then(|d| {
        let (m, s) = d.split_once(':')?;
        Some((m.parse::<u64>().ok()? * 60 + s.parse::<u64>().ok()?) * 1000)
    })?;
    let bpm = t.find("Tempo (BPM) ").map(|i| &t[i + 12..]).and_then(|b| b.split_whitespace().next()).and_then(|b| b.parse::<f64>().ok()).unwrap_or(0.0);
    let minor = if t.contains("minor mode") { true } else if t.contains("major mode") { false } else { return None };
    Some(SongKey { tonic, minor, bpm, ms: dur, url: url.to_string() })
}

/// It's the song: its length, and the song's title words in its address
fn fits(k: &SongKey, title: &str, ms: u64) -> bool {
    let len_ok = ms == 0 || (k.ms as f64 - ms as f64).abs() / ms as f64 <= LENGTH_WITHIN;
    let have = words(k.url.rsplit('/').next().unwrap_or(""));
    len_ok && words(title).iter().all(|w| have.contains(w))
}

async fn page(url: &str) -> Result<Option<SongKey>, String> {
    let res = client().get(url).send().await.map_err(|e| format!("Couldn't reach SongBPM: {e}"))?;
    if res.status().as_u16() == 404 { return Ok(None); }
    if !res.status().is_success() { return Err(format!("SongBPM said {}", res.status())); }
    let html = res.text().await.map_err(|e| e.to_string())?;
    Ok(parse_page(&html, url))
}

/// The song's key, if SongBPM has the song (its title as you'd name it, with its version: "Spotless",
/// "Symphony (Ryos Remix)"; its length in ms, 0 if not known). None: not found, or not this version
pub async fn lookup(artist: &str, title: &str, ms: u64) -> Result<Option<SongKey>, String> {
    let (a, t) = (slug(artist), slug(title));
    if a.is_empty() || t.is_empty() { return Ok(None); }
    // its own page
    if let Some(k) = page(&format!("{ORIGIN}/@{a}/{t}")).await? {
        if fits(&k, title, ms) { return Ok(Some(k)); }
    }
    // its search: the results' pages, the first few that could be it
    let res = client().post(format!("{ORIGIN}/searches")).form(&[("query", format!("{artist} {title}"))]).send().await.map_err(|e| format!("Couldn't reach SongBPM: {e}"))?;
    if !res.status().is_success() { return Ok(None); }
    let html = res.text().await.map_err(|e| e.to_string())?;
    let mut seen = Vec::new();
    for part in html.split("href=\"/@").skip(1) {
        let Some(path) = part.split('"').next() else { continue };
        if path.contains("/apple-music") || path.matches('/').count() != 1 || seen.iter().any(|p: &String| p == path) { continue; }
        let tail = words(path.rsplit('/').next().unwrap_or(""));
        if !words(title).iter().all(|w| tail.contains(w)) { continue; }
        seen.push(path.to_string());
        if seen.len() > RESULTS { break; }
    }
    for path in seen {
        if let Some(k) = page(&format!("{ORIGIN}/@{path}")).await? {
            if fits(&k, title, ms) { return Ok(Some(k)); }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_as_its_addresses_have_them() {
        assert_eq!(slug("Killswitch Engage"), "killswitch-engage");
        assert_eq!(slug("We Don't Need You"), "we-dont-need-you");
        assert_eq!(slug("Butterfly Effect"), "butterfly-effect");
    }

    #[test]
    fn a_page_read() {
        let html = r#"<dt> Key </dt> <dd> G♯/A♭ </dd> <dt>Duration</dt><dd>3:28</dd><dt>Tempo (BPM)</dt><dd>174</dd> <p>a G♯/A♭ key and a minor mode.</p>"#;
        let k = parse_page(html, "https://songbpm.com/@koven/butterfly-effect").unwrap();
        assert_eq!((k.tonic, k.minor, k.ms, k.bpm), (8, true, 208_000, 174.0));
        assert!(fits(&k, "Butterfly Effect", 207_500));
        // (another length: another version)
        assert!(!fits(&k, "Butterfly Effect", 260_000));
        assert_eq!(pitch_class("B♭"), Some(10));
        assert_eq!(pitch_class("C"), Some(0));
    }

    /// (live: what it finds for a few songs)
    #[test]
    #[ignore]
    fn live_lookup() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for (a, t, ms) in [("Koven", "Butterfly Effect", 208_000u64), ("Alex Clare", "Too Close", 256_000), ("RYOS", "Symphony (Ryos Remix)", 0), ("Martin Garrix", "Spotless", 0)] {
            println!("{a} - {t}: {:?}", rt.block_on(lookup(a, t, ms)));
        }
    }
}
