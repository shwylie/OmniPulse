// MusicBrainz: an open music catalogue, searched with no account, so a fresh install has songs to
// find (each plays from YouTube Music, matched by its title, artist and length).
//
// Their rules (musicbrainz.org/doc/MusicBrainz_API/Rate_Limiting): about one request a second from
// an IP, and going over turns *every* request away with a 503 until it's back under; and a
// User-Agent that names the app, its version and a way to contact its maintainer. So requests here
// go one at a time, at least 1.1 s apart; a 503 pauses them all for a minute; and the same search
// isn't asked twice within an hour. Cover art comes from the Cover Art Archive (coverartarchive.org,
// no limit of its own), loaded by the app as plain image addresses.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const API: &str = "https://musicbrainz.org/ws/2/recording";
/// Who's asking, as MusicBrainz asks: the app, its version, and where its maintainer can be reached
const USER_AGENT: &str = concat!("OmniPulse/", env!("CARGO_PKG_VERSION"), " ( https://github.com/shwylie/OmniPulse )");
const GAP: Duration = Duration::from_millis(1100);
/// A search that would wait longer than this for its turn is refused (typing faster than 1/s)
const MAX_WAIT: Duration = Duration::from_secs(5);
const BACK_OFF: Duration = Duration::from_secs(60);
const CACHE_FOR: Duration = Duration::from_secs(3600);
const CACHE_MAX: usize = 200;

struct Gate {
    next: Option<Instant>,
    paused_until: Option<Instant>,
}
static GATE: Mutex<Gate> = Mutex::new(Gate { next: None, paused_until: None });
static CACHE: Mutex<Option<HashMap<String, (Instant, Vec<Value>)>>> = Mutex::new(None);

/// Waits for this request's turn (at least GAP after the last one), or says why not
async fn turn() -> Result<(), String> {
    let wait = {
        let mut g = GATE.lock().unwrap();
        let now = Instant::now();
        if let Some(p) = g.paused_until {
            if p > now {
                return Err("MusicBrainz asked OmniPulse to slow down: it's back in a minute.".into());
            }
        }
        let at = g.next.map_or(now, |n| n.max(now));
        if at - now > MAX_WAIT {
            return Err("Too many MusicBrainz searches at once: try again in a moment.".into());
        }
        g.next = Some(at + GAP);
        at - now
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
    Ok(())
}

fn pause() {
    GATE.lock().unwrap().paused_until = Some(Instant::now() + BACK_OFF);
}

/// Recordings matching the words, as the app's tracks (up to 25)
pub async fn search(query: &str) -> Result<Vec<Value>, String> {
    let q: String = query.trim().chars().take(200).collect();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let key = q.to_lowercase();
    if let Some((at, hit)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(&key)).cloned() {
        if at.elapsed() < CACHE_FOR {
            return Ok(hit);
        }
    }
    turn().await?;
    let res = reqwest::Client::new()
        .get(API)
        .query(&[("query", words_query(&q).as_str()), ("fmt", "json"), ("limit", "40")])
        .header("User-Agent", USER_AGENT)
        .header("Accept", "application/json")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach MusicBrainz: {e}"))?;
    if res.status().as_u16() == 503 || res.status().as_u16() == 429 {
        pause();
        return Err("MusicBrainz asked OmniPulse to slow down: it's back in a minute.".into());
    }
    if !res.status().is_success() {
        return Err(format!("MusicBrainz answered {}", res.status()));
    }
    let body: Value = res.json().await.map_err(|e| format!("MusicBrainz sent something unreadable: {e}"))?;
    // a song once (MusicBrainz keeps each recording on every release separately), and not as part of a DJ mix
    let mut seen = std::collections::HashSet::new();
    let list: Vec<Value> = body["recordings"]
        .as_array()
        .map(|a| a.iter()
            .filter(|r| !r["disambiguation"].as_str().unwrap_or("").to_lowercase().replace('\u{2010}', "-").contains("dj-mix"))
            .filter_map(track)
            .filter(|t| seen.insert(format!("{}\u{1}{}",t["title"].as_str().unwrap_or("").to_lowercase(), t["user"]["username"].as_str().unwrap_or("").to_lowercase())))
            .take(25)
            .collect())
        .unwrap_or_default();
    let mut c = CACHE.lock().unwrap();
    let c = c.get_or_insert_with(HashMap::new);
    if c.len() >= CACHE_MAX {
        c.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
        if c.len() >= CACHE_MAX {
            c.clear();
        }
    }
    c.insert(key, (Instant::now(), list.clone()));
    Ok(list)
}

/// The words as a search: Lucene's special characters escaped, so nothing typed changes its meaning
fn lucene(q: &str) -> String {
    let mut out = String::with_capacity(q.len() + 8);
    for ch in q.chars() {
        if "+-&|!(){}[]^\"~*?:\\/".contains(ch) {
            out.push('\\');
        }
        out.push(ch);
    }
    out
}

/// Every word in the title or the artist ("ghosts stuff deadmau5"), on an official release
fn words_query(q: &str) -> String {
    let words: Vec<String> = q.split_whitespace().take(12).map(lucene).collect();
    let all = words.iter().map(|w| format!("(recording:{w} OR artist:{w})")).collect::<Vec<_>>().join(" AND ");
    format!("({all}) AND status:official")
}

/// One recording as a track: its first official release gives the album and the cover
fn track(r: &Value) -> Option<Value> {
    let id = r["id"].as_str()?;
    let title = r["title"].as_str()?;
    let artist: String = r["artist-credit"]
        .as_array()
        .map(|a| a.iter().map(|c| format!("{}{}", c["name"].as_str().unwrap_or(""), c["joinphrase"].as_str().unwrap_or(""))).collect())
        .unwrap_or_default();
    let releases = r["releases"].as_array().cloned().unwrap_or_default();
    let rel = releases.iter().find(|x| x["status"].as_str() == Some("Official")).or_else(|| releases.first());
    let group = rel.and_then(|x| x["release-group"]["id"].as_str());
    Some(json!({
        "id": format!("mb:{id}"),
        "mbId": id,
        "source": "musicbrainz",
        "title": title,
        "user": { "username": artist.trim() },
        "duration": r["length"].as_u64().unwrap_or(0),
        "album": rel.and_then(|x| x["title"].as_str()).unwrap_or(""),
        "release_date": r["first-release-date"].as_str().unwrap_or(""),
        "artwork_url": group.map(|g| format!("https://coverartarchive.org/release-group/{g}/front-250")).unwrap_or_default(),
        "permalink_url": format!("https://musicbrainz.org/recording/{id}"),
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn special_characters_are_escaped() {
        assert_eq!(super::lucene("AC/DC: T.N.T"), "AC\\/DC\\: T.N.T");
        assert_eq!(super::lucene("a (b) \"c\""), "a \\(b\\) \\\"c\\\"");
    }

    #[test]
    fn every_word_in_the_title_or_artist() {
        assert_eq!(super::words_query("a b:"), r"((recording:a OR artist:a) AND (recording:b\: OR artist:b\:)) AND status:official");
    }

    #[test]
    fn a_recording_becomes_a_track() {
        let r = serde_json::json!({ "id": "abc", "title": "Song", "length": 200000,
            "artist-credit": [{ "name": "A", "joinphrase": " & " }, { "name": "B" }],
            "releases": [{ "title": "Bootleg", "status": "Bootleg", "release-group": { "id": "g1" } }, { "title": "Album", "status": "Official", "release-group": { "id": "g2" } }] });
        let t = super::track(&r).unwrap();
        assert_eq!(t["user"]["username"], "A & B");
        assert_eq!(t["album"], "Album");
        assert_eq!(t["artwork_url"], "https://coverartarchive.org/release-group/g2/front-250");
        assert_eq!(t["id"], "mb:abc");
    }

    #[tokio::test]
    async fn requests_wait_their_turn() {
        // two turns in a row: the second waits the gap
        super::turn().await.unwrap();
        let t = std::time::Instant::now();
        super::turn().await.unwrap();
        assert!(t.elapsed() >= super::GAP - std::time::Duration::from_millis(20));
    }
}
