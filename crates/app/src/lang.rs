// Discover's language (the web app's recommendation language, js/discover.js): nothing records a
// song's language, so it's guessed from its title: other alphabets, accented letters, common words.
// A song it can't place goes by its artist's titles together ("Ella" says nothing; ten of theirs do),
// and is kept if those can't be placed either.

use crate::track::Track;
use std::collections::HashMap;

/// The languages to pick from: (pref value, name)
pub const CHOICES: [(&str, &str); 6] = [("any", "Any language"), ("en", "English"), ("es", "Spanish"), ("pt", "Portuguese"), ("fr", "French"), ("de", "German")];

const LANGS: [(&str, &str, &str); 5] = [
    ("en", "the and you your my me i'm im don't dont with for of in on we is it be feel never night baby heart all this that what when like just got love so up down", ""),
    ("es", "el la los las que y de con para por mi tu te amor corazón corazon noche quiero sin una eres como más mas bebé nada vida mía mia contigo", "ñ¿¡áéíóú"),
    ("pt", "não nao você voce meu minha com pra para uma amor coração coracao sem é eu te saudade quero vida também tambem", "ãõçâêô"),
    ("fr", "le la les de des et je tu pour avec sans mon ma toi moi amour coeur cœur nuit c'est est pas une", "àèùçœëêîï"),
    ("de", "der die das und ich nicht mit für fur dich mich liebe herz nacht ein eine ist du wir", "äöüß"),
];

fn other_script(c: char) -> bool {
    matches!(c as u32, 0x370..=0x3ff | 0x400..=0x4ff | 0x590..=0x5ff | 0x600..=0x6ff | 0x900..=0x97f | 0xe00..=0xe7f | 0x3040..=0x30ff | 0x3400..=0x9fff | 0xac00..=0xd7af)
}

/// A text's language: a code ("en"), one with "?" (only a weak hint), "other" (another alphabet) or
/// "unknown"
pub fn detect(text: &str) -> String {
    let text = text.to_lowercase();
    if text.chars().any(other_script) { return "other".into(); }
    let words: Vec<&str> = text.split(|c: char| !(c.is_ascii_lowercase() || ('\u{c0}'..='\u{24f}').contains(&c) || c == '\u{153}' || c == '\'')).filter(|w| !w.is_empty()).collect();
    let (mut best, mut best_score) = ("unknown", 0);
    for (code, list, chars) in LANGS {
        let set: Vec<&str> = list.split(' ').collect();
        let mut s = words.iter().filter(|w| set.contains(w)).count();
        s += text.chars().filter(|c| chars.contains(*c)).count() * 2;
        if s > best_score { best_score = s; best = code; }
    }
    match best_score { 0 => "unknown".into(), 1 => format!("{best}?"), _ => best.into() }
}

fn sure(d: &str) -> bool { d != "unknown" && !d.ends_with('?') }

fn first_artist(t: &Track) -> String {
    omnipulse_core::ranking::artist_key(t.artist.split(',').next().unwrap_or(""))
}

/// The songs in `want` (a code; "any": all of them): a song's own title first, else its artist's
pub fn filter(list: Vec<Track>, want: &str) -> Vec<Track> {
    if want == "any" || want.is_empty() { return list; }
    let mut text: HashMap<String, String> = HashMap::new();
    for t in &list { let e = text.entry(first_artist(t)).or_default(); e.push(' '); e.push_str(&t.title); }
    let theirs: HashMap<String, String> = text.into_iter().map(|(a, s)| (a, detect(&s))).collect();
    list.into_iter().filter(|t| {
        let d = detect(&t.title);
        if d == want { return true; }
        if sure(&d) { return false; }
        theirs.get(&first_artist(t)).is_none_or(|ad| !(sure(ad) && ad != want))
    }).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(title: &str, artist: &str) -> Track { Track { title: title.into(), artist: artist.into(), key: title.into(), ..Default::default() } }

    #[test]
    fn songs_by_their_titles_and_short_ones_by_their_artists() {
        assert_eq!(detect("Never Let You Go"), "en");
        assert_eq!(detect("Quiero Tu Amor"), "es");
        assert_eq!(detect("Ella"), "unknown");
        assert_eq!(detect("夜に駆ける"), "other");
        let list = vec![t("Quiero Tu Amor", "Ana"), t("Ella", "Ana"), t("Mi Corazón", "Ana"), t("Ella", "Sam"), t("Love Me Now", "Sam"), t("Never Let You Go", "Sam")];
        let kept: Vec<String> = filter(list, "en").into_iter().map(|x| format!("{} {}", x.title, x.artist)).collect();
        assert_eq!(kept, ["Ella Sam", "Love Me Now Sam", "Never Let You Go Sam"]);
    }
}
