// The catalogue: Deezer's and Apple's (iTunes) public music data, both official and open to anyone
// with no key or account. It's what search, Discover's charts and new releases, and artist pages
// know about songs that aren't on your own platforms; each song plays from YouTube Music (matched by
// its title, artist and length), as MusicBrainz's do.
//
// Their limits: Deezer about 50 requests every 5 seconds (api.deezer.com answers "Quota limit
// exceeded", code 4, past it); iTunes about 20 a minute. So requests go one at a time with a gap
// (100 ms for Deezer, 3 s for iTunes, which is skipped rather than waited for when it's busy), the
// same request isn't asked twice within an hour, and a quota answer pauses Deezer for 5 seconds.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const DEEZER: &str = "https://api.deezer.com";
const ITUNES: &str = "https://itunes.apple.com/search";
const CACHE_FOR: Duration = Duration::from_secs(3600);
const CACHE_MAX: usize = 400;

struct Gate {
    gap: Duration,
    next: Option<Instant>,
    /// when the last request went (a request you asked for goes right after it, ahead of those waiting)
    last: Option<Instant>,
}
static DZ_GATE: Mutex<Gate> = Mutex::new(Gate { gap: Duration::from_millis(100), next: None, last: None });
static IT_GATE: Mutex<Gate> = Mutex::new(Gate { gap: Duration::from_secs(3), next: None, last: None });
type Cache = HashMap<String, (Instant, Value)>;
static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

/// Waits for this request's turn, or None if that would be longer than `max`
async fn turn(gate: &Mutex<Gate>, max: Duration) -> Option<()> {
    let wait = {
        let mut g = gate.lock().unwrap();
        let now = Instant::now();
        let at = g.next.map_or(now, |n| n.max(now));
        if at - now > max {
            return None;
        }
        g.next = Some(at + g.gap);
        g.last = Some(at);
        at - now
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
    Some(())
}

fn cached(url: &str) -> Option<Value> {
    CACHE.lock().unwrap().as_ref().and_then(|c| c.get(url)).filter(|(at, _)| at.elapsed() < CACHE_FOR).map(|(_, v)| v.clone())
}

fn keep(url: &str, v: &Value) {
    let mut c = CACHE.lock().unwrap();
    let c = c.get_or_insert_with(HashMap::new);
    if c.len() >= CACHE_MAX {
        c.retain(|_, (at, _)| at.elapsed() < CACHE_FOR);
        if c.len() >= CACHE_MAX {
            c.clear();
        }
    }
    c.insert(url.to_string(), (Instant::now(), v.clone()));
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap_or_default()
}

/// What the app may ask Deezer for: its catalogue, read only
fn allowed(path: &str) -> bool {
    ["/search", "/track/", "/album/", "/artist/", "/chart", "/genre", "/editorial", "/radio", "/playlist/"].iter().any(|p| path.starts_with(p)) && !path.contains("..")
}

/// A request you're waiting on (a page you opened): ahead of the ones Discover makes in the background,
/// right after the last one sent; those waiting move back to make room, so Deezer's limit still holds
async fn turn_now(gate: &Mutex<Gate>) {
    let wait = {
        let mut g = gate.lock().unwrap();
        let now = Instant::now();
        let at = g.last.map_or(now, |l| (l + g.gap).max(now));
        g.last = Some(at);
        g.next = Some(g.next.map_or(at, |n| n.max(at)) + g.gap);
        at - now
    };
    if !wait.is_zero() {
        tokio::time::sleep(wait).await;
    }
}

/// One Deezer request for a page you opened (see turn_now): never turned away for the queue
pub async fn deezer_now(path: &str) -> Result<Value, String> {
    deezer_with(path, true).await
}

/// One Deezer request (`path` like "/artist/27/albums?limit=10"), as Deezer answers it
pub async fn deezer(path: &str) -> Result<Value, String> {
    deezer_with(path, false).await
}

async fn deezer_with(path: &str, now: bool) -> Result<Value, String> {
    if !allowed(path) {
        return Err("Not a Deezer catalogue request".into());
    }
    let url = format!("{DEEZER}{path}");
    if let Some(v) = cached(&url) {
        return Ok(v);
    }
    for attempt in 0..2 {
        if now { turn_now(&DZ_GATE).await; } else { turn(&DZ_GATE, Duration::from_secs(10)).await.ok_or("Too many catalogue requests at once: try again in a moment.")?; }
        let res = client().get(&url).send().await.map_err(|e| format!("Couldn't reach Deezer: {e}"))?;
        if !res.status().is_success() {
            return Err(format!("Deezer answered {}", res.status()));
        }
        let v: Value = res.json().await.map_err(|e| format!("Deezer sent something unreadable: {e}"))?;
        if let Some(err) = v.get("error") {
            // over the quota: a moment, then once more
            if err["code"].as_i64() == Some(4) && attempt == 0 {
                DZ_GATE.lock().unwrap().next = Some(Instant::now() + Duration::from_secs(5));
                continue;
            }
            return Err(format!("Deezer: {}", err["message"].as_str().unwrap_or("it couldn't answer")));
        }
        keep(&url, &v);
        return Ok(v);
    }
    Err("Deezer is busy: try again in a moment.".into())
}

/// A Deezer track as the app's track (the album can come separately, as on an album's own page)
pub fn dz_track(t: &Value, album: Option<&Value>) -> Option<Value> {
    let id = t["id"].as_u64()?;
    let title = t["title"].as_str()?;
    let al = album.unwrap_or(&t["album"]);
    let art = al["cover_big"].as_str().or(al["cover_xl"].as_str()).or(al["cover_medium"].as_str()).unwrap_or("");
    Some(json!({
        "id": format!("dz:{id}"),
        "dzId": id,
        "source": "deezer",
        "title": title,
        "user": { "username": t["artist"]["name"].as_str().unwrap_or("") },
        "dzArtistId": t["artist"]["id"],
        "dzAlbumId": al["id"],
        "album": al["title"].as_str().unwrap_or(""),
        "duration": t["duration"].as_u64().unwrap_or(0) * 1000,
        "artwork_url": art,
        "dzRank": t["rank"],
        "isrc": t["isrc"].as_str().unwrap_or(""),
        "explicit": t["explicit_lyrics"].as_bool().unwrap_or(false),
        "release_date": t["release_date"].as_str().or(al["release_date"].as_str()).unwrap_or(""),
        "permalink_url": t["link"].as_str().map(String::from).unwrap_or(format!("https://www.deezer.com/track/{id}")),
    }))
}

/// An iTunes song as the app's track
fn it_track(t: &Value) -> Option<Value> {
    let id = t["trackId"].as_u64()?;
    let title = t["trackName"].as_str()?;
    Some(json!({
        "id": format!("it:{id}"),
        "itId": id,
        "source": "itunes",
        "title": title,
        "user": { "username": t["artistName"].as_str().unwrap_or("") },
        "album": t["collectionName"].as_str().unwrap_or(""),
        "duration": t["trackTimeMillis"].as_u64().unwrap_or(0),
        // (its art comes 100 px; the same address gives a bigger one)
        "artwork_url": t["artworkUrl100"].as_str().unwrap_or("").replace("100x100bb", "600x600bb"),
        "genre": t["primaryGenreName"].as_str().unwrap_or(""),
        "explicit": t["trackExplicitness"].as_str() == Some("explicit"),
        "release_date": t["releaseDate"].as_str().unwrap_or("").chars().take(10).collect::<String>(),
        "permalink_url": t["trackViewUrl"].as_str().unwrap_or(""),
    }))
}

async fn itunes(term: &str) -> Result<Vec<Value>, String> {
    let url = format!("{ITUNES}?entity=song&limit=25&term={}", urlencoding::encode(term));
    let v = match cached(&url) {
        Some(v) => v,
        None => {
            // (iTunes is the extra: when it's busy, search goes without it)
            turn(&IT_GATE, Duration::from_secs(4)).await.ok_or("busy")?;
            let res = client().get(&url).send().await.map_err(|e| e.to_string())?;
            if !res.status().is_success() {
                return Err(format!("iTunes answered {}", res.status()));
            }
            let v: Value = res.json().await.map_err(|e| e.to_string())?;
            keep(&url, &v);
            v
        }
    };
    Ok(v["results"].as_array().into_iter().flatten().filter_map(it_track).collect())
}

/// Covers a song could have (Track Preferences' Change Cover): its releases' on Deezer and iTunes, the biggest
/// each gives, with the release's name; the same picture once, eight at most
pub async fn covers(artist: &str, title: &str) -> Vec<(String, String)> {
    let q = format!("artist:\"{}\" track:\"{}\"", artist.replace('"', ""), title.replace('"', ""));
    let dz = deezer(&format!("/search?limit=12&q={}", urlencoding::encode(&q))).await;
    let it = itunes(&format!("{artist} {title}")).await;
    let mut out: Vec<(String, String)> = Vec::new();
    let dz = dz.unwrap_or_default();
    let found = dz["data"].as_array().into_iter().flatten().map(|t| (t["album"]["cover_xl"].as_str().unwrap_or(""), t["album"]["title"].as_str().unwrap_or("")))
        .map(|(u, a)| (u.to_string(), a.to_string()))
        .chain(it.unwrap_or_default().into_iter().map(|t| (t["artwork_url"].as_str().unwrap_or("").to_string(), t["album"].as_str().unwrap_or("").to_string())));
    for (url, album) in found {
        if url.is_empty() || out.iter().any(|(u, _)| *u == url) { continue; }
        out.push((url, album));
        if out.len() >= 8 { break; }
    }
    out
}

/// How many albums' genres a search looks up (Deezer's search results don't carry one)
const GENRE_ALBUMS: usize = 12;

/// An album's first genre on Deezer ("Electro", "Pop"), if it has one
async fn album_genre(id: u64) -> Option<String> {
    let v = deezer(&format!("/album/{id}")).await.ok()?;
    v["genres"]["data"].as_array()?.first()?["name"].as_str().map(String::from).filter(|g| !g.is_empty())
}

/// The words of a title or artist, for telling two catalogues' copies of a song apart
fn same_key(t: &Value) -> String {
    let words = |s: &str| crate::ranking::words(s).join(" ");
    format!("{}\u{1}{}", words(t["title"].as_str().unwrap_or("")), words(t["user"]["username"].as_str().unwrap_or("")))
}

/// Songs matching the words: Deezer's, then iTunes's that Deezer hasn't got (a song once)
pub async fn search(query: &str) -> Result<Vec<Value>, String> {
    let q: String = query.trim().chars().take(200).collect();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let path = format!("/search?limit=40&q={}", urlencoding::encode(&q));
    let it = itunes(&q);
    // Deezer's songs, with their genres (its search doesn't say: their albums do), while iTunes is asked
    let dz = async {
        let v = deezer(&path).await?;
        let mut list: Vec<Value> = v["data"].as_array().into_iter().flatten().filter_map(|t| dz_track(t, None)).collect();
        let mut albums: Vec<u64> = Vec::new();
        for t in list.iter().take(20) {
            if let Some(id) = t["dzAlbumId"].as_u64() {
                if !albums.contains(&id) && albums.len() < GENRE_ALBUMS {
                    albums.push(id);
                }
            }
        }
        let genres = futures::future::join_all(albums.iter().map(|id| album_genre(*id))).await;
        let by: HashMap<u64, String> = albums.into_iter().zip(genres).filter_map(|(id, g)| g.map(|g| (id, g))).collect();
        for t in list.iter_mut() {
            if let Some(g) = t["dzAlbumId"].as_u64().and_then(|id| by.get(&id)) {
                t["genre"] = json!(g);
            }
        }
        Ok::<_, String>(list)
    };
    let (dz, it) = futures::join!(dz, it);
    if let (Err(e), Err(_)) = (&dz, &it) {
        return Err(e.clone());
    }
    let mut out: Vec<Value> = dz.unwrap_or_default();
    let mut seen: HashMap<String, usize> = out.iter().enumerate().map(|(i, t)| (same_key(t), i)).collect();
    for t in it.unwrap_or_default() {
        let k = same_key(&t);
        match seen.get(&k) {
            // (the copy already there gets iTunes's genre)
            Some(&i) => {
                if out[i]["genre"].is_null() {
                    out[i]["genre"] = t["genre"].clone();
                }
            }
            None => {
                seen.insert(k, out.len());
                out.push(t);
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn a_deezer_track() {
        let t = json!({ "id": 3135556, "title": "Harder, Better, Faster, Stronger", "duration": 224, "rank": 800000, "link": "https://www.deezer.com/track/3135556",
            "artist": { "id": 27, "name": "Daft Punk" }, "album": { "id": 302127, "title": "Discovery", "cover_xl": "https://x/1000.jpg", "cover_big": "https://x/500.jpg" } });
        let v = super::dz_track(&t, None).unwrap();
        assert_eq!(v["id"], "dz:3135556");
        assert_eq!(v["duration"], 224000);
        assert_eq!(v["artwork_url"], "https://x/500.jpg");
        assert_eq!(v["user"]["username"], "Daft Punk");
    }

    #[test]
    fn only_the_catalogue() {
        assert!(super::allowed("/artist/27/albums"));
        assert!(super::allowed("/chart/0/tracks?limit=10"));
        assert!(super::allowed("/playlist/908622995/tracks"));
        assert!(!super::allowed("/user/me/flow"));
        assert!(!super::allowed("/artist/../user/1"));
    }

    #[test]
    fn the_same_song_from_both() {
        let a = json!({ "title": "Strobe", "user": { "username": "deadmau5" } });
        let b = json!({ "title": "Strobe!", "user": { "username": "Deadmau5" } });
        assert_eq!(super::same_key(&a), super::same_key(&b));
    }

    /// cargo test -p omnipulse-core catalog -- --ignored (on the internet)
    #[test]
    #[ignore]
    fn searching() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        let list = rt.block_on(super::search("strobe deadmau5")).unwrap();
        println!("{}", list.iter().map(|t| format!("{} {} - {} [{}]", t["source"], t["user"]["username"], t["title"], t["genre"])).collect::<Vec<_>>().join("\n"));
        assert!(list.iter().any(|t| t["title"].as_str().unwrap_or("").contains("Strobe")));
        assert!(list.iter().any(|t| t["source"] == "itunes") || list.len() >= 10);
    }
}
