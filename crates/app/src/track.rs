// A track, from any platform, the way the app shows and plays it. Its key matches the web app's ids
// (SoundCloud's number, "sp:<id>", "yt:<id>", a local file's id), so favourites, stats and play
// history from the web app line up.

use serde_json::Value;

#[derive(Clone, Debug, PartialEq)]
pub enum Src {
    SoundCloud(u64),
    Spotify(String),
    YouTube(String),
    Local(String),
}

#[derive(Clone, Debug, Default)]
pub struct Track {
    pub src: Src,
    pub key: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub ms: u64,
    pub art: String,
    pub genre: String,
    /// its page on the platform (SoundCloud's; the others' are made from the id: see web_link)
    pub link: String,
    /// a SoundCloud Go+ track: SoundCloud only gives this app its 30-second preview, so it plays from
    /// YouTube Music instead (crate::versions)
    pub go_plus: bool,
    /// its licence, as SoundCloud gives it ("cc-by", "all-rights-reserved"...; "" from elsewhere)
    pub licence: String,
    /// its record label, where it's given (for Stream-Safe Music)
    pub label: String,
}

impl Default for Src {
    fn default() -> Src {
        Src::Local(String::new())
    }
}

/// Sets & Podcasts: anything over 15 minutes
pub const MIX_MS: u64 = 15 * 60 * 1000;

impl Track {
    pub fn plat(&self) -> &'static str {
        match self.src {
            Src::SoundCloud(_) => "soundcloud",
            Src::Spotify(_) => "spotify",
            Src::YouTube(_) => "youtube",
            Src::Local(_) => "local",
        }
    }

    /// Where the track is on the web, for Discord's button and the like ("" for your files)
    pub fn web_link(&self) -> String {
        if !self.link.is_empty() {
            return self.link.clone();
        }
        match &self.src {
            Src::Spotify(uri) => format!("https://open.spotify.com/track/{}", uri.rsplit(':').next().unwrap_or("")),
            Src::YouTube(id) => format!("https://music.youtube.com/watch?v={id}"),
            _ => String::new(),
        }
    }

    pub fn is_mix(&self) -> bool {
        self.ms > MIX_MS
    }

    pub fn row(&self, playing: bool) -> crate::Row {
        crate::Row {
            title: self.title.clone().into(),
            artist: self.artist.clone().into(),
            album: crate::titles::album_shown(&self.album, &self.title, &self.artist).into(),
            dur: fmt_ms(self.ms).into(),
            plat: self.plat().into(),
            playing,
            head: Default::default(),
            auto: false,
            art: Default::default(),
            liked: false,
            ..crate::share_flags(self)
        }
    }

    /// Its title tidied as the web app does (crate::titles): promo tags out, "Artist - Song" split
    /// (SoundCloud), and featured artists out of the title and into the artists
    pub fn tidied(mut self) -> Self {
        self.title = crate::titles::clean_title(&lookalikes(&self.title));
        self.artist = lookalikes(&self.artist);
        let mut t = self.split_artist();
        let (title, artist) = crate::titles::tidy_feat(&t.title, &t.artist);
        t.title = title;
        t.artist = artist;
        t
    }

    /// A SoundCloud upload titled "Artist - Song": the artist from the title, as the web app shows it
    pub fn split_artist(mut self) -> Self {
        if let Src::SoundCloud(_) = self.src {
            if let Some((a, t)) = self.title.split_once(" - ") {
                let (a, t) = (a.trim(), t.trim());
                let version_only = t.split_whitespace().count() <= 3 && ["mix", "edit", "remix", "version", "vip", "bootleg"].iter().any(|w| t.to_lowercase().ends_with(w));
                if !a.is_empty() && !t.is_empty() && a.len() <= 80 && !a.contains('(') && !version_only {
                    self.artist = a.to_string();
                    self.title = t.to_string();
                }
            }
        }
        self
    }

    pub fn soundcloud(t: &Value) -> Option<Track> {
        let t = if t.get("track").is_some() { &t["track"] } else { t };
        let id = t["id"].as_u64()?;
        if t["kind"].as_str().is_some_and(|k| k != "track") {
            return None;
        }
        Some(Track {
            src: Src::SoundCloud(id),
            key: id.to_string(),
            title: t["title"].as_str().unwrap_or("Untitled").to_string(),
            artist: t["user"]["username"].as_str().unwrap_or("").to_string(),
            album: String::new(),
            ms: t["full_duration"].as_u64().or(t["duration"].as_u64()).unwrap_or(0),
            art: t["artwork_url"].as_str().or(t["user"]["avatar_url"].as_str()).unwrap_or("").replace("-large", "-t200x200"),
            genre: t["genre"].as_str().unwrap_or("").to_string(),
            link: if t["sharing"].as_str() == Some("private") { String::new() } else { t["permalink_url"].as_str().unwrap_or("").to_string() },
            go_plus: t["access"].as_str() == Some("preview") || t["policy"].as_str() == Some("SNIP"),
            licence: t["license"].as_str().unwrap_or("").to_string(),
            label: t["label_name"].as_str().unwrap_or("").to_string(),
        }
        .tidied())
    }

    pub fn spotify(t: &Value) -> Option<Track> {
        let uri = t["uri"].as_str()?;
        if t["is_playable"].as_bool() == Some(false) {
            return None;
        }
        Some(Track {
            src: Src::Spotify(uri.to_string()),
            key: format!("sp:{}", t["id"].as_str().unwrap_or("")),
            title: t["name"].as_str().unwrap_or("Untitled").to_string(),
            artist: t["artists"].as_array().map(|a| a.iter().filter_map(|x| x["name"].as_str()).collect::<Vec<_>>().join(", ")).unwrap_or_default(),
            album: t["album"]["name"].as_str().unwrap_or("").to_string(),
            ms: t["duration_ms"].as_u64().unwrap_or(0),
            art: t["album"]["images"].as_array().and_then(|i| i.get(1).or(i.first())).and_then(|i| i["url"].as_str()).unwrap_or("").to_string(),
            genre: String::new(),
            link: String::new(),
            go_plus: false, ..Default::default()
        }.tidied())
    }

    pub fn youtube(t: &omnipulse_core::youtube::YtTrack) -> Track {
        Track {
            src: Src::YouTube(t.id.clone()),
            key: format!("yt:{}", t.id),
            title: t.title.clone(),
            artist: t.artists.clone(),
            album: t.album.clone(),
            ms: t.duration_ms,
            art: t.thumb.clone(),
            genre: String::new(),
            link: String::new(),
            go_plus: false, ..Default::default()
        }
        .tidied()
    }

    /// The same saved form, for the session (read back with from_snapshot)
    pub fn to_snapshot(&self) -> Value {
        let mut s = serde_json::json!({
            "id": self.key, "source": self.plat(), "title": self.title, "displayArtist": self.artist,
            "album": self.album, "duration": self.ms, "artwork_url": self.art, "genre": self.genre,
            "permalink_url": self.link,
        });
        if !self.licence.is_empty() {
            s["license"] = serde_json::json!(self.licence);
        }
        if !self.label.is_empty() {
            s["label_name"] = serde_json::json!(self.label);
        }
        if let Src::Spotify(uri) = &self.src {
            s["uri"] = serde_json::json!(uri);
        }
        s
    }

    /// The web app's saved form of a track (favourites, its own playlists, YouTube Music likes)
    pub fn from_snapshot(s: &Value) -> Option<Track> {
        let id = s["id"].as_str().map(String::from).or_else(|| s["id"].as_u64().map(|n| n.to_string()))?;
        let source = s["source"].as_str().unwrap_or("soundcloud");
        let src = match source {
            "spotify" => Src::Spotify(s["uri"].as_str().map(String::from).unwrap_or_else(|| format!("spotify:track:{}", id.trim_start_matches("sp:")))),
            "youtube" => Src::YouTube(id.trim_start_matches("yt:").to_string()),
            "local" => {
                let path = omnipulse_core::local::local_path_for(&id)?;
                Src::Local(path.to_string_lossy().into_owned())
            }
            _ => Src::SoundCloud(id.parse().ok()?),
        };
        Some(Track {
            src,
            key: id,
            title: s["title"].as_str().unwrap_or("Untitled").to_string(),
            artist: s["displayArtist"].as_str().or(s["user"]["username"].as_str()).unwrap_or("").to_string(),
            album: s["album"].as_str().or(s["spAlbum"]["name"].as_str()).unwrap_or("").to_string(),
            ms: s["duration"].as_u64().unwrap_or(0),
            art: s["artwork_url"].as_str().unwrap_or("").to_string(),
            genre: s["genre"].as_str().unwrap_or("").to_string(),
            link: s["permalink_url"].as_str().unwrap_or("").to_string(),
            go_plus: s["access"].as_str() == Some("preview") || s["policy"].as_str() == Some("SNIP"),
            licence: s["license"].as_str().unwrap_or("").to_string(),
            label: s["label_name"].as_str().unwrap_or("").to_string(),
        })
    }
}

/// Characters the app's font hasn't got, as ones it has that look the same: "Ginsu \u{2a2f} Ghosts" (a
/// maths cross) shows as "Ginsu \u{d7} Ghosts". (The web view borrows another font for them; this can't.)
pub fn lookalikes(s: &str) -> String {
    s.chars().map(|c| match c { '\u{2a2f}' | '\u{2715}' | '\u{2716}' => '\u{d7}', _ => c }).collect()
}

pub fn fmt_ms(ms: u64) -> String {
    let s = ms / 1000;
    if s >= 3600 { format!("{}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60) } else { format!("{}:{:02}", s / 60, s % 60) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn soundcloud_titles_name_their_artist() {
        let t = Track::soundcloud(&serde_json::json!({ "id": 5, "kind": "track", "title": "Krewella - Alive (Sumant Remix)", "user": { "username": "sumant" }, "duration": 180000 })).unwrap();
        assert_eq!((t.artist.as_str(), t.title.as_str()), ("Krewella", "Alive (Sumant Remix)"));
        let t = Track::soundcloud(&serde_json::json!({ "id": 6, "title": "Song - Extended Mix", "user": { "username": "x" } })).unwrap();
        assert_eq!(t.title, "Song - Extended Mix");
        assert_eq!(t.key, "6");
    }

    #[test]
    fn web_snapshots_become_tracks() {
        let s = serde_json::json!({ "source": "spotify", "id": "sp:abc", "uri": "spotify:track:abc", "title": "X", "user": { "username": "A, B" }, "duration": 200000 });
        let t = Track::from_snapshot(&s).unwrap();
        assert_eq!(t.src, Src::Spotify("spotify:track:abc".into()));
        assert_eq!((t.key.as_str(), t.artist.as_str()), ("sp:abc", "A, B"));
        let y = Track::from_snapshot(&serde_json::json!({ "source": "youtube", "id": "yt:dQw4w9WgXcQ", "title": "Y" })).unwrap();
        assert_eq!(y.src, Src::YouTube("dQw4w9WgXcQ".into()));
        let sc = Track::from_snapshot(&serde_json::json!({ "id": 123, "title": "Z" })).unwrap();
        assert_eq!(sc.src, Src::SoundCloud(123));
    }
}

/// Your own title and artist for tracks (the web app's Track preferences: prefs.trackFixes, by the
/// track's id, which is its key here too). An empty artist there means its own.
pub fn apply_fixes(prefs: &Value, list: &mut [Track]) {
    let Some(fixes) = prefs["trackFixes"].as_object() else { return };
    if fixes.is_empty() {
        return;
    }
    for t in list.iter_mut() {
        if let Some(fx) = fixes.get(&t.key) {
            if let Some(title) = fx["title"].as_str().filter(|s| !s.trim().is_empty()) {
                t.title = title.to_string();
            }
            if let Some(artist) = fx["artist"].as_str().filter(|s| !s.trim().is_empty()) {
                t.artist = artist.to_string();
            }
        }
    }
}

#[cfg(test)]
mod fix_tests {
    use super::*;

    #[test]
    fn your_titles_and_artists_win() {
        let mut list = vec![Track::youtube(&omnipulse_core::youtube::YtTrack { id: "abc".into(), title: "Song (feat. X) feat. X".into(), artists: "Label".into(), ..Default::default() })];
        let prefs = serde_json::json!({ "trackFixes": { "yt:abc": { "title": "Song (feat. X)", "artist": "Marc Benjamin" } } });
        apply_fixes(&prefs, &mut list);
        assert_eq!(list[0].title, "Song (feat. X)");
        assert_eq!(list[0].artist, "Marc Benjamin");
        // a title-only edit keeps the artist
        let prefs = serde_json::json!({ "trackFixes": { "yt:abc": { "title": "Other", "artist": null } } });
        apply_fixes(&prefs, &mut list);
        assert_eq!((list[0].title.as_str(), list[0].artist.as_str()), ("Other", "Marc Benjamin"));
    }
}

/// The same song on any platform: its first artist and its title's words (no tags in brackets)
pub fn song_key(t: &Track) -> String {
    let words = |s: &str| s.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(String::from).collect::<Vec<_>>();
    let mut title = String::new();
    let mut depth = 0;
    for c in t.title.chars() {
        match c {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => title.push(c),
            _ => {}
        }
    }
    let artist = t.artist.split([',', '&']).next().unwrap_or("");
    let title = words(&title).join(" ");
    if title.is_empty() { String::new() } else { format!("{}|{title}", words(artist).join("")) }
}

/// What's in your likes, as keys and as songs, to leave out of new music
pub struct Liked {
    keys: std::collections::HashSet<String>,
    songs: std::collections::HashSet<String>,
}

impl Liked {
    pub fn of(list: &[Track]) -> Liked {
        Liked { keys: list.iter().map(|t| t.key.clone()).collect(), songs: list.iter().map(song_key).filter(|k| !k.is_empty()).collect() }
    }
    pub fn has(&self, t: &Track) -> bool {
        self.keys.contains(&t.key) || self.songs.contains(&song_key(t))
    }
}

#[cfg(test)]
mod liked_tests {
    use super::*;

    fn t(src: Src, key: &str, title: &str, artist: &str) -> Track {
        Track { src, key: key.into(), title: title.into(), artist: artist.into(), album: String::new(), ms: 0, art: String::new(), genre: String::new(), link: String::new(), go_plus: false, ..Default::default() }
    }

    #[test]
    fn a_song_liked_anywhere_counts() {
        let liked = Liked::of(&[t(Src::Spotify("spotify:track:a".into()), "sp:a", "Strobe", "deadmau5")]);
        // the same song on YouTube Music, with its tags
        assert!(liked.has(&t(Src::YouTube("v".into()), "yt:v", "Strobe (Club Edit)", "deadmau5, Someone")));
        assert!(!liked.has(&t(Src::YouTube("w".into()), "yt:w", "Ghosts 'n' Stuff", "deadmau5")));
    }
}
