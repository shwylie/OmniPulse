// Lyrics, from several places, each asked in the order you choose (Settings > Lyrics):
//   - LRCLIB (lrclib.net): a free, open database of synced lyrics
//   - NetEase Cloud Music and KuGou: big public catalogues of synced lyrics (strong on music from
//     Asia, and on electronic music), searched by artist and title, and only taken when the title,
//     the artist and the length match
//   - YouTube Music: the plain lyrics it shows for a song (from its own licensed sources)
// Each answer: { synced, plain, source, note } (synced is LRC text), or null when there's none.

use crate::ranking::words;
use serde_json::{json, Value};
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent(UA).timeout(Duration::from_secs(10)).build().unwrap_or_default()
}

async fn get_json(req: reqwest::RequestBuilder) -> Result<Value, String> {
    let res = req.send().await.map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if status.as_u16() == 404 {
        return Ok(Value::Null);
    }
    if !status.is_success() {
        return Err(format!("Lyrics lookup failed ({status})"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

// Lets the page look up lyrics without running into cross-site rules (LRCLIB only)
pub async fn lyrics_get(url: String) -> Result<Value, String> {
    if !url.starts_with("https://lrclib.net/api/") {
        return Err("That address is not allowed".to_string());
    }
    get_json(client().get(&url).header("User-Agent", "OmniPulse (personal music player)")).await
}

/// Is this search result the song asked for: every word of the title, an artist word, and the
/// length within 4 seconds (when both are known)?
fn same_song(title: &str, artist: &str, secs: u64, c_title: &str, c_artist: &str, c_secs: u64) -> bool {
    let want = words(title);
    let have: std::collections::HashSet<String> = words(c_title).into_iter().collect();
    let artist_words = words(artist);
    let c_artist: std::collections::HashSet<String> = words(c_artist).into_iter().collect();
    !want.is_empty()
        && want.iter().all(|w| have.contains(w))
        && (artist_words.is_empty() || artist_words.iter().any(|w| c_artist.contains(w)))
        && (secs == 0 || c_secs == 0 || secs.abs_diff(c_secs) <= 4)
}

/// LRC without its tags and the credits some catalogues put in the first lines ("作词 : ...")
fn tidy_lrc(lrc: &str) -> String {
    let credit = regex::Regex::new(r"^\[[\d:.]+\]\s*(作词|作曲|编曲|制作人|词|曲|Lyricist|Composer|Arranger|Producer)\s*[:：]").unwrap();
    let tag = regex::Regex::new(r"^\[(id|ti|ar|al|by|offset|length|re|ve|hash|sign|qq|total):").unwrap();
    // "Lyrics by：...", "Composed by: ...", and a "Song - Artist" heading in the first seconds
    let by = regex::Regex::new(r"(?i)^\[[\d:.]+\]\s*(lyrics|composed|produced|written|arranged|music|words)\s+by\s*[:：]").unwrap();
    let heading = regex::Regex::new(r"^\[00:0\d(\.\d+)?\]\s*\S.*\s-\s.*\S").unwrap();
    lrc.lines()
        .map(str::trim)
        .filter(|l| !credit.is_match(l) && !tag.is_match(l) && !by.is_match(l) && !heading.is_match(l))
        .collect::<Vec<_>>()
        .join("\n")
}

fn has_timing(lrc: &str) -> bool {
    regex::Regex::new(r"(?m)^\[\d+:\d+(\.\d+)?\]\s*\S").unwrap().is_match(lrc)
}

fn answer(synced: Option<String>, plain: Option<String>, source: &str, note: &str) -> Value {
    let synced = synced.filter(|s| has_timing(s));
    let plain = plain.filter(|s| !s.trim().is_empty());
    if synced.is_none() && plain.is_none() {
        return Value::Null;
    }
    json!({ "synced": synced, "plain": plain, "source": source, "note": note })
}

async fn lrclib(artist: &str, title: &str, secs: u64) -> Result<Value, String> {
    let c = client();
    let mut q = vec![("artist_name", artist.to_string()), ("track_name", title.to_string())];
    if secs > 0 {
        q.push(("duration", secs.to_string()));
    }
    let mut v = get_json(c.get("https://lrclib.net/api/get").query(&q)).await.unwrap_or(Value::Null);
    if v.is_null() || v.get("statusCode").is_some() {
        let list = get_json(c.get("https://lrclib.net/api/search").query(&[("q", format!("{artist} {title}"))])).await?;
        v = list
            .as_array()
            .and_then(|a| {
                a.iter()
                    .find(|x| same_song(title, artist, secs, x["trackName"].as_str().unwrap_or(""), x["artistName"].as_str().unwrap_or(""), x["duration"].as_f64().unwrap_or(0.0) as u64))
                    .cloned()
            })
            .unwrap_or(Value::Null);
    }
    Ok(answer(v["syncedLyrics"].as_str().map(String::from), v["plainLyrics"].as_str().map(String::from), "LRCLIB", ""))
}

async fn netease(artist: &str, title: &str, secs: u64) -> Result<Value, String> {
    let c = client();
    let found = get_json(
        c.get("https://music.163.com/api/search/get")
            .header("Referer", "https://music.163.com")
            .query(&[("s", format!("{artist} {title}")), ("type", "1".into()), ("limit", "10".into())]),
    )
    .await?;
    let pick = found["result"]["songs"].as_array().and_then(|a| {
        a.iter().find(|s| {
            let who: Vec<&str> = s["artists"].as_array().into_iter().flatten().filter_map(|x| x["name"].as_str()).collect();
            same_song(title, artist, secs, s["name"].as_str().unwrap_or(""), &who.join(" "), s["duration"].as_u64().unwrap_or(0) / 1000)
        })
    });
    let Some(id) = pick.and_then(|s| s["id"].as_u64()) else { return Ok(Value::Null) };
    let l = get_json(c.get("https://music.163.com/api/song/lyric").header("Referer", "https://music.163.com").query(&[("id", id.to_string()), ("lv", "1".into())])).await?;
    let lrc = l["lrc"]["lyric"].as_str().map(tidy_lrc);
    // (an instrumental's "no lyrics" is a line saying so)
    if lrc.as_deref().is_some_and(|s| s.contains("纯音乐") || s.trim().is_empty()) {
        return Ok(Value::Null);
    }
    Ok(answer(lrc, None, "NetEase Cloud Music", ""))
}

async fn kugou(artist: &str, title: &str, secs: u64) -> Result<Value, String> {
    use base64::Engine;
    let c = client();
    let mut q = vec![("ver", "1".to_string()), ("man", "yes".into()), ("client", "pc".into()), ("keyword", format!("{artist} - {title}")), ("hash", String::new())];
    if secs > 0 {
        q.push(("duration", (secs * 1000).to_string()));
    }
    let found = get_json(c.get("https://lyrics.kugou.com/search").query(&q)).await?;
    let pick = found["candidates"].as_array().and_then(|a| {
        a.iter().find(|x| same_song(title, artist, secs, x["song"].as_str().unwrap_or(""), x["singer"].as_str().unwrap_or(""), x["duration"].as_u64().unwrap_or(0) / 1000))
    });
    let Some(x) = pick else { return Ok(Value::Null) };
    let d = get_json(c.get("https://lyrics.kugou.com/download").query(&[
        ("ver", "1"),
        ("client", "pc"),
        ("fmt", "lrc"),
        ("charset", "utf8"),
        ("id", x["id"].as_str().unwrap_or("")),
        ("accesskey", x["accesskey"].as_str().unwrap_or("")),
    ]))
    .await?;
    let lrc = d["content"]
        .as_str()
        .and_then(|b| base64::engine::general_purpose::STANDARD.decode(b).ok())
        .and_then(|b| String::from_utf8(b).ok())
        .map(|s| tidy_lrc(&s));
    Ok(answer(lrc, None, "KuGou", ""))
}

async fn youtube(video_id: &str) -> Result<Value, String> {
    if video_id.is_empty() {
        return Ok(Value::Null);
    }
    Ok(match crate::youtube::lyrics(video_id).await? {
        Some((words, from)) => answer(None, Some(words), "YouTube Music", &from),
        None => Value::Null,
    })
}

/// Lyrics from one place ("lrclib", "netease", "kugou", "youtube"): an answer, or null
pub async fn from(provider: &str, artist: &str, title: &str, secs: u64, video_id: &str) -> Result<Value, String> {
    match provider {
        "lrclib" => lrclib(artist, title, secs).await,
        "netease" => netease(artist, title, secs).await,
        "kugou" => kugou(artist, title, secs).await,
        "youtube" => youtube(video_id).await,
        _ => Err(format!("No lyrics provider called {provider}")),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_same_song() {
        assert!(super::same_song("Hello", "Adele", 295, "Hello", "Adele", 294));
        assert!(!super::same_song("Hello", "Adele", 295, "Hello", "Lionel Richie", 250));
        assert!(!super::same_song("Hello", "Adele", 295, "Hello", "Adele", 360));
        assert!(super::same_song("Strobe", "deadmau5", 0, "Strobe (Radio Edit)", "deadmau5", 200));
    }

    #[test]
    fn credits_go() {
        let lrc = "[ti:Hello]\n[00:00.000] 作词 : Adele\n[00:01.00] Composer: X\n[00:05.10]Hello, it's me";
        assert_eq!(super::tidy_lrc(lrc), "[00:05.10]Hello, it's me");
        assert!(super::has_timing("[00:05.10]Hello"));
    }

    /// cargo test -p omnipulse-core lyrics -- --ignored --nocapture (on the internet)
    #[test]
    #[ignore]
    fn every_provider() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        for p in ["lrclib", "netease", "kugou"] {
            let v = rt.block_on(super::from(p, "Adele", "Hello", 295, "")).unwrap();
            println!("{p}: {}", v.to_string().chars().take(140).collect::<String>());
            assert!(v["synced"].is_string(), "{p}");
        }
        for q in ["adele hello", "daft punk get lucky", "billie eilish birds of a feather"] {
            let id = rt.block_on(crate::youtube::search(q)).unwrap()[0].id.clone();
            let y = rt.block_on(super::from("youtube", "", "", 0, &id)).unwrap();
            println!("youtube {q}: {}", y.to_string().chars().take(120).collect::<String>());
        }
    }
}
