// Playlists as files (as the web app's dist/js/playlistfiles.js): an M3U or CSV file's songs, read for
// importing, and a playlist written as either. Shared by the native and phone apps; reading and
// writing the file itself is playlist_file_read / playlist_file_write (lib.rs).

/// One song in a playlist file: what it says of it (any of these can be empty)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub ms: u64,
    pub isrc: String,
    pub link: String,
    pub path: String,
}

/// A song to write: its platform's name ("SoundCloud"...), its link, its file on this PC
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub title: String,
    pub artist: String,
    pub album: String,
    pub ms: u64,
    pub isrc: String,
    pub platform: String,
    pub link: String,
    pub path: String,
}

fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let (mut row, mut cell, mut quoted) = (Vec::new(), String::new(), false);
    let mut it = text.chars().peekable();
    while let Some(c) = it.next() {
        if quoted {
            if c == '"' && it.peek() == Some(&'"') { cell.push('"'); it.next(); }
            else if c == '"' { quoted = false; }
            else { cell.push(c); }
        } else if c == '"' {
            quoted = true;
        } else if c == ',' {
            row.push(std::mem::take(&mut cell));
        } else if c == '\n' || c == '\r' {
            if c == '\r' && it.peek() == Some(&'\n') { it.next(); }
            row.push(std::mem::take(&mut cell));
            if row.iter().any(|x: &String| !x.trim().is_empty()) { rows.push(std::mem::take(&mut row)); } else { row.clear(); }
        } else {
            cell.push(c);
        }
    }
    row.push(cell);
    if row.iter().any(|x| !x.trim().is_empty()) { rows.push(row); }
    rows
}

fn csv_cell(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) { format!("\"{}\"", s.replace('"', "\"\"")) } else { s.to_string() }
}

/// The songs in a CSV (its header row names the columns: Exportify's and the app's own, among others)
pub fn csv_entries(text: &str) -> Vec<Entry> {
    let rows = csv_rows(text.trim_start_matches('\u{feff}'));
    if rows.len() < 2 { return Vec::new(); }
    let head: Vec<String> = rows[0].iter().map(|h| h.trim().to_lowercase()).collect();
    let col = |names: &[&str]| head.iter().position(|h| names.contains(&h.as_str()));
    let Some(ti) = col(&["title", "track name", "track", "name", "song", "song name"]) else { return Vec::new() };
    let ar = col(&["artist", "artist name(s)", "artists", "artist name", "artist(s)"]);
    let al = col(&["album", "album name"]);
    let du = col(&["duration (ms)", "duration", "length"]);
    let is = col(&["isrc"]);
    let li = col(&["link", "url", "track uri", "spotify uri", "uri", "spotify url"]);
    let pa = col(&["path", "file", "location"]);
    let get = |r: &Vec<String>, i: Option<usize>| i.and_then(|i| r.get(i)).map(|s| s.trim().to_string()).unwrap_or_default();
    rows[1..].iter().map(|r| {
        let dur = get(r, du);
        // (a duration in m:ss, or ms)
        let ms = match dur.split_once(':') {
            Some((m, s)) if s.len() == 2 => m.parse::<u64>().unwrap_or(0) * 60_000 + s.parse::<u64>().unwrap_or(0) * 1000,
            _ => dur.parse::<f64>().map(|v| v as u64).unwrap_or(0),
        };
        Entry { title: get(r, Some(ti)), artist: get(r, ar).replace(';', ", "), album: get(r, al), ms, isrc: get(r, is), link: get(r, li), path: get(r, pa) }
    }).filter(|e| !e.title.is_empty() || !e.link.is_empty() || !e.path.is_empty()).collect()
}

/// The songs in an M3U (with its #PLAYLIST name, if it has one)
pub fn m3u_entries(text: &str) -> (String, Vec<Entry>) {
    let mut out = Vec::new();
    let mut name = String::new();
    let mut info: Option<Entry> = None;
    for raw in text.trim_start_matches('\u{feff}').lines() {
        let line = raw.trim();
        if line.is_empty() { continue; }
        if line.len() >= 10 && line[..10].eq_ignore_ascii_case("#PLAYLIST:") { name = line[10..].trim().to_string(); continue; }
        if line.len() >= 8 && line[..8].eq_ignore_ascii_case("#EXTINF:") {
            let rest = &line[8..];
            let (head, label) = rest.split_once(',').unwrap_or((rest, ""));
            let secs: f64 = head.split_whitespace().next().unwrap_or("0").parse().unwrap_or(0.0);
            let label = label.trim();
            let split = label.split_once(" - ").or_else(|| label.split_once(" \u{2013} "));
            let (artist, title) = match split { Some((a, t)) => (a.trim().to_string(), t.trim().to_string()), None => (String::new(), label.to_string()) };
            info = Some(Entry { ms: if secs > 0.0 { (secs * 1000.0) as u64 } else { 0 }, artist, title, ..Default::default() });
            continue;
        }
        if line.starts_with('#') { continue; }
        let mut e = info.take().unwrap_or_default();
        let lower = line.to_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("spotify:") {
            e.link = line.to_string();
        } else {
            let p = if lower.starts_with("file:///") { &line[8..] } else if lower.starts_with("file://") { &line[7..] } else { line };
            e.path = p.to_string();
        }
        // (no #EXTINF: the file's name is the best there is)
        if e.title.is_empty() && !e.path.is_empty() {
            let file = e.path.rsplit(['\\', '/']).next().unwrap_or("");
            e.title = file.rsplit_once('.').map(|(a, _)| a).unwrap_or(file).to_string();
        }
        out.push(e);
    }
    (name, out)
}

/// A playlist as CSV (the app's own columns)
pub fn csv_text(rows: &[Row]) -> String {
    let mut out = String::from("Title,Artist,Album,Duration (ms),ISRC,Platform,Link,Path\r\n");
    for r in rows {
        let ms = if r.ms > 0 { r.ms.to_string() } else { String::new() };
        let cells = [r.title.as_str(), &r.artist, &r.album, &ms, &r.isrc, &r.platform, &r.link, &r.path];
        out.push_str(&cells.iter().map(|c| csv_cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    out
}

/// A playlist as M3U: each song's file, or else its link (one with neither is left out)
pub fn m3u_text(name: &str, rows: &[Row]) -> String {
    let mut out = format!("#EXTM3U\n#PLAYLIST:{name}\n");
    for r in rows {
        let at = if r.path.is_empty() { &r.link } else { &r.path };
        if at.is_empty() { continue; }
        out.push_str(&format!("#EXTINF:{},{} - {}\n{at}\n", (r.ms + 500) / 1000, r.artist, r.title));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_both_ways() {
        let rows = vec![Row { title: "Hello, again".into(), artist: "A \"B\"".into(), ms: 200_000, platform: "Spotify".into(), link: "https://open.spotify.com/track/x".into(), ..Default::default() }];
        let text = csv_text(&rows);
        let back = csv_entries(&text);
        assert_eq!(back.len(), 1);
        assert_eq!((back[0].title.as_str(), back[0].artist.as_str(), back[0].ms), ("Hello, again", "A \"B\"", 200_000));
        // Exportify's columns, a m:ss length and artists with semicolons
        let e = csv_entries("Track Name,Artist Name(s),Duration\nStrobe,deadmau5;Someone,10:37\n");
        assert_eq!((e[0].artist.as_str(), e[0].ms), ("deadmau5, Someone", 637_000));
        assert!(csv_entries("Nope\nx\n").is_empty());
    }

    #[test]
    fn m3u_both_ways() {
        let rows = vec![
            Row { title: "Strobe".into(), artist: "deadmau5".into(), ms: 637_000, path: "C:\\Music\\Strobe.mp3".into(), ..Default::default() },
            Row { title: "Gone".into(), ..Default::default() },
            Row { title: "Clarity".into(), artist: "Zedd".into(), ms: 271_000, link: "https://music.youtube.com/watch?v=abc".into(), ..Default::default() },
        ];
        let (name, back) = m3u_entries(&m3u_text("Mine", &rows));
        assert_eq!(name, "Mine");
        assert_eq!(back.len(), 2);
        assert_eq!((back[0].artist.as_str(), back[0].title.as_str(), back[0].path.as_str()), ("deadmau5", "Strobe", "C:\\Music\\Strobe.mp3"));
        assert_eq!(back[1].link, "https://music.youtube.com/watch?v=abc");
        // a bare list of files
        let (_, plain) = m3u_entries("file:///D:/x/Song One.flac\n");
        assert_eq!((plain[0].title.as_str(), plain[0].path.as_str()), ("Song One", "D:/x/Song One.flac"));
    }
}
