// AI music kept out of song requests (Settings > Streaming > Song Requests > Keep Out AI Music): artists
// on Soul Over AI's list of confirmed AI artists (souloverai.com, CC BY 4.0: fetched from its GitHub at
// most once a day, and only while the option's on), and anything its own title or artist says is AI-made
// (Suno, Udio, an "AI cover"...).

use crate::track::Track;
use std::collections::HashSet;
use std::sync::Mutex;

const LIST_URL: &str = "https://raw.githubusercontent.com/xoundbyte/soul-over-ai/main/dist/artists.json";
const FRESH_SECS: u64 = 24 * 3600;

/// The artists on the list, by their names as compared (see `plain`)
static NAMES: Mutex<Option<HashSet<String>>> = Mutex::new(None);

/// A name as compared: lower case, accents and punctuation gone, its spaces single
fn plain(s: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let bare: String = s.nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
    bare.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
}

fn cache_file() -> Option<std::path::PathBuf> {
    omnipulse_core::data_dir().ok().map(|d| d.join("state").join("ai-artists.json"))
}

fn names_of(text: &str) -> Option<HashSet<String>> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    Some(v.as_array()?.iter().filter(|a| a["removed"].as_bool() != Some(true)).filter_map(|a| a["name"].as_str()).map(plain).filter(|n| !n.is_empty()).collect())
}

/// The list, ready for `is_ai`: the kept copy at once, then a fresh one if the kept one's a day old
pub async fn refresh() {
    let file = cache_file();
    let kept = file.as_ref().and_then(|f| std::fs::read_to_string(f).ok());
    if NAMES.lock().unwrap().is_none() {
        if let Some(n) = kept.as_deref().and_then(names_of) { *NAMES.lock().unwrap() = Some(n); }
    }
    let age = file.as_ref().and_then(|f| std::fs::metadata(f).ok()?.modified().ok()?.elapsed().ok()).map(|d| d.as_secs());
    if age.is_some_and(|a| a < FRESH_SECS) && NAMES.lock().unwrap().is_some() { return; }
    let Ok(r) = reqwest::Client::new().get(LIST_URL).timeout(std::time::Duration::from_secs(20)).send().await else { return };
    let Ok(text) = r.text().await else { return };
    if let Some(n) = names_of(&text) {
        *NAMES.lock().unwrap() = Some(n);
        if let Some(f) = file {
            if let Some(d) = f.parent() { let _ = std::fs::create_dir_all(d); }
            let _ = std::fs::write(f, text);
        }
    }
}

/// Whether a track is AI music: by one of its artists on the list (their whole name, never part of
/// it), or by what its title or artist says of itself
pub fn is_ai(t: &Track) -> bool {
    let words = plain(&format!("{} {}", t.title, t.artist));
    let says = |w: &str| format!(" {words} ").contains(&format!(" {w} "));
    if ["suno", "suno ai", "udio", "ai cover", "ai song", "ai generated", "ai version", "ai remix", "ai music", "made with ai"].iter().any(|w| says(w)) { return true; }
    let g = NAMES.lock().unwrap();
    let Some(names) = g.as_ref() else { return false };
    let mut artists: Vec<String> = t.artist.split([',', '&', '/']).flat_map(|a| a.split(" feat").next().map(String::from)).map(|a| plain(&a)).filter(|a| !a.is_empty()).collect();
    // (an uploader's "Artist - Title", the artist in front)
    if let Some((lead, _)) = t.title.split_once(" - ") { artists.push(plain(lead)); }
    if !t.uploader.is_empty() { artists.push(plain(&t.uploader)); }
    artists.iter().any(|a| names.contains(a))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(title: &str, artist: &str) -> Track { Track { title: title.into(), artist: artist.into(), ..Default::default() } }

    #[test]
    fn listed_artists_and_self_described_ai_music_are_caught() {
        *NAMES.lock().unwrap() = names_of(r#"[{"name":"Velvet Sundown"},{"name":"Gone Artist","removed":true}]"#);
        assert!(is_ai(&t("Dust on the Wind", "The Velvet Sundown, x".replace("The ", "").as_str())));
        assert!(is_ai(&t("Never Gonna Give You Up (AI Cover)", "someone")));
        assert!(is_ai(&t("My Song [Suno]", "uploader")));
        // (a whole name only: not part of one; not one taken off the list; "ai" alone isn't AI)
        assert!(!is_ai(&t("Song", "Velvet Sundowners")));
        assert!(!is_ai(&t("Song", "Gone Artist")));
        assert!(!is_ai(&t("Ai Se Eu Te Pego", "Michel Teló")));
    }
}
