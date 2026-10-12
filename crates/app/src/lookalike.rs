// Adding several songs to a playlist (as the web app's js/select.js): exactly the same track is left
// out; a song that's probably the same one under a slightly different name (another upload, another
// platform's) is asked about: Keep Both, Keep the One There, or Use the New One.

use crate::track::Track;
use regex::Regex;
use std::sync::OnceLock;
use unicode_normalization::UnicodeNormalization;

/// A song's name for telling it apart: its first artist and its title, without the noise uploads add
/// ("Official Video", "Lyrics", "HD", a remaster's year), its featured artists or its artist repeated
pub fn song_key(t: &Track) -> String {
    static NOISE: OnceLock<Regex> = OnceLock::new();
    static KEEP: OnceLock<Regex> = OnceLock::new();
    static BRACKETS: OnceLock<Regex> = OnceLock::new();
    static FEAT: OnceLock<Regex> = OnceLock::new();
    static SPLIT: OnceLock<Regex> = OnceLock::new();
    let noise = NOISE.get_or_init(|| Regex::new(r"(?i)\b(official( music)?( video| audio| lyric video)?|lyrics?( video)?|audio|video|visuali[sz]er|hq|hd|4k|explicit|clean|remaster(ed)?( \d{4})?|\d{4} remaster(ed)?|free download|out now)\b").unwrap());
    let keep = KEEP.get_or_init(|| Regex::new(r"(?i)\b(remix|mix|edit|live|acoustic|vip|bootleg|flip|cover|version|instrumental)\b").unwrap());
    let brackets = BRACKETS.get_or_init(|| Regex::new(r"\s*[(\[]([^)\]]*)[)\]]").unwrap());
    let feat = FEAT.get_or_init(|| Regex::new(r"(?i)\b(feat\.?|ft\.?|featuring)\s.*$").unwrap());
    let split = SPLIT.get_or_init(|| Regex::new(r"(?i)\s*(?:,|&| x | feat\.? | ft\.? )\s*").unwrap());
    let (g_artist, g_title) = crate::versions::guess_names(t);
    let title = if t.title.is_empty() { g_title } else { t.title.clone() };
    let title = brackets.replace_all(&title, |c: &regex::Captures| {
        let inner = &c[1];
        if noise.is_match(inner) && !keep.is_match(inner) { " ".to_string() } else { format!(" {inner} ") }
    });
    let title = noise.replace_all(&feat.replace(&title, ""), " ").to_string();
    let artist = if g_artist.is_empty() { t.artist.clone() } else { g_artist };
    let artist = split.split(&artist).next().unwrap_or("").to_string();
    let norm = |s: &str| -> String {
        let plain: String = s.to_lowercase().nfkd().filter(|c| !('\u{300}'..='\u{36f}').contains(c)).collect();
        plain.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ")
    };
    let a = norm(&artist);
    let mut ti = norm(&title);
    // (a title that already starts with the artist, "Artist - Song", counts the same as without)
    if !a.is_empty() && ti.starts_with(&format!("{a} ")) { ti = ti[a.len() + 1..].to_string(); }
    format!("{a}|{ti}")
}

/// Probably the same song: the same track, or the same name and within 15 seconds of the same length
pub fn likely_same(a: &Track, b: &Track) -> bool {
    if a.key == b.key { return true; }
    if song_key(a) != song_key(b) { return false; }
    a.ms == 0 || b.ms == 0 || a.ms.abs_diff(b.ms) <= 15_000
}

/// What to do with a look-alike
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Choice { Both, Old, New }

impl Choice {
    pub fn label(self) -> &'static str { match self { Choice::Both => "Keep both", Choice::Old => "Keep the one there", Choice::New => "Use the new one" } }
    pub fn next(self) -> Choice { match self { Choice::Both => Choice::Old, Choice::Old => Choice::New, Choice::New => Choice::Both } }
}

/// A song being added that looks like one there already (or one added just before it)
#[derive(Clone, Debug)]
pub struct Conflict { pub new: Track, pub twin: Track, pub choice: Choice }

/// Of `tracks` to add to a playlist that has `have`: those added as they are (not there yet, nothing like
/// them there), and those to ask about
pub fn plan(tracks: &[Track], have: &[Track]) -> (Vec<Track>, Vec<Conflict>) {
    let mut seen: std::collections::HashSet<String> = have.iter().map(|t| t.key.clone()).collect();
    let (mut add, mut ask): (Vec<Track>, Vec<Conflict>) = (Vec::new(), Vec::new());
    for t in tracks {
        if seen.contains(&t.key) { continue; }   // (exactly this track is there already)
        let twin = have.iter().find(|h| likely_same(h, t)).or_else(|| add.iter().find(|h| likely_same(h, t))).cloned();
        seen.insert(t.key.clone());
        match twin {
            Some(twin) => ask.push(Conflict { new: t.clone(), twin, choice: Choice::Both }),
            None => add.push(t.clone()),
        }
    }
    (add, ask)
}

/// The answers applied: what's added, and what there is replaced by what (old key, new track)
pub fn decide(mut add: Vec<Track>, answers: &[Conflict]) -> (Vec<Track>, Vec<(String, Track)>) {
    let mut replace = Vec::new();
    for c in answers {
        match c.choice {
            Choice::Both => add.push(c.new.clone()),
            Choice::Old => {}
            Choice::New => match add.iter().position(|x| x.key == c.twin.key) {
                Some(i) => add[i] = c.new.clone(),
                None => replace.push((c.twin.key.clone(), c.new.clone())),
            },
        }
    }
    (add, replace)
}

/// A line for a song in the dialog: its title, artist, platform and length
pub fn line(t: &Track) -> String {
    let len = if t.ms > 0 { format!(" \u{b7} {}:{:02}", t.ms / 60_000, (t.ms / 1000) % 60) } else { String::new() };
    let plat = match t.plat() { "spotify" => "Spotify", "youtube" => "YouTube Music", "local" => "Your files", "soundcloud" => "SoundCloud", p => p };
    format!("{} \u{2014} {} \u{b7} {plat}{len}", if t.title.is_empty() { "Untitled" } else { &t.title }, if t.artist.is_empty() { "Unknown" } else { &t.artist })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn t(key: &str, title: &str, artist: &str, ms: u64) -> Track { Track { key: key.into(), title: title.into(), artist: artist.into(), ms, ..Default::default() } }

    #[test]
    fn the_same_song_under_another_name() {
        let a = t("sc:1", "Antidote (Official Video)", "Swedish House Mafia", 200_000);
        let b = t("sp:2", "Antidote", "Swedish House Mafia, Knife Party", 205_000);
        assert!(likely_same(&a, &b));
        // a remix is another song; so is one a lot longer
        assert!(!likely_same(&a, &t("sc:3", "Antidote (Knife Party Remix)", "Swedish House Mafia", 200_000)));
        assert!(!likely_same(&a, &t("sc:4", "Antidote", "Swedish House Mafia", 400_000)));
        // "Artist - Song" as the title, by the uploader
        assert_eq!(song_key(&t("yt:5", "Swedish House Mafia - Antidote [HD]", "Swedish House Mafia", 0)), song_key(&a));
    }

    #[test]
    fn exactly_there_is_left_out_look_alikes_asked_and_answered() {
        let have = vec![t("sc:1", "Antidote", "SHM", 200_000)];
        let (add, ask) = plan(&[t("sc:1", "Antidote", "SHM", 200_000), t("sp:2", "Antidote", "SHM", 201_000), t("sc:9", "Other", "X", 1)], &have);
        assert_eq!(add.iter().map(|x| x.key.as_str()).collect::<Vec<_>>(), ["sc:9"]);
        assert_eq!(ask.len(), 1);
        let mut ans = ask.clone();
        ans[0].choice = Choice::New;
        let (add2, replace) = decide(add.clone(), &ans);
        assert_eq!(add2.len(), 1);
        assert_eq!(replace.len(), 1);
        assert_eq!(replace[0].0, "sc:1");
        ans[0].choice = Choice::Both;
        assert_eq!(decide(add, &ans).0.len(), 2);
    }
}
