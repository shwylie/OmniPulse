// Apple Music, once: a read-only import of your loved songs and your playlists, with nothing changed
// on Apple Music's side. Apple only lets its own apps (and paying developers) read a library, so this
// does what Apple's web player does: the key is the one music.apple.com's own script carries, and you
// sign in there (in a private window the app opens). Your sign-in token stays in memory only, and is
// forgotten when the import is done.

use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

const SITE: &str = "https://music.apple.com";
const API: &str = "https://amp-api.music.apple.com";
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0 Safari/537.36";

/// the web player's key (kept for the run), and your sign-in token (only while importing)
static DEV: Mutex<Option<String>> = Mutex::new(None);
static USER: Mutex<Option<String>> = Mutex::new(None);

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent(UA).timeout(Duration::from_secs(30)).build().unwrap_or_default()
}

/// The web player's key: found in its script, and the first that Apple's catalogue answers to
pub async fn dev_token() -> Result<String, String> {
    if let Some(t) = DEV.lock().unwrap().clone() {
        return Ok(t);
    }
    let c = client();
    let page = c.get(format!("{SITE}/us/browse")).send().await.map_err(|e| format!("Couldn't reach Apple Music: {e}"))?.text().await.map_err(|e| e.to_string())?;
    let script = regex::Regex::new(r#"src="(/assets/index[^"]+\.js)""#).unwrap().captures(&page).map(|m| m[1].to_string()).ok_or("Apple Music's page has changed: its script wasn't found.")?;
    let js = c.get(format!("{SITE}{script}")).send().await.map_err(|e| e.to_string())?.text().await.map_err(|e| e.to_string())?;
    let mut keys: Vec<String> = regex::Regex::new(r"eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+").unwrap().find_iter(&js).map(|m| m.as_str().to_string()).collect();
    keys.dedup();
    // (the web player's own first, then the rest)
    keys.sort_by_key(|k| !jwt_kid(k).contains("WebPlay"));
    for k in keys {
        let ok = c.get(format!("{API}/v1/catalog/us/search?term=music&types=songs&limit=1")).bearer_auth(&k).header("Origin", SITE).send().await.map(|r| r.status().is_success()).unwrap_or(false);
        if ok {
            *DEV.lock().unwrap() = Some(k.clone());
            return Ok(k);
        }
    }
    Err("Apple Music's page has changed: no key it uses was found.".into())
}

/// A key's id, from its header
fn jwt_kid(t: &str) -> String {
    use base64::Engine;
    let head = t.split('.').next().unwrap_or("");
    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(head.trim_end_matches('=')).ok()
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["kid"].as_str().map(String::from))
        .unwrap_or_default()
}

/// Your sign-in (the web player's media-user-token), while importing
pub fn set_user_token(t: String) {
    *USER.lock().unwrap() = Some(t);
}

/// Forgets your sign-in (the import's done)
pub fn forget() {
    *USER.lock().unwrap() = None;
}

pub fn signed_in() -> bool {
    USER.lock().unwrap().is_some()
}

async fn get(path: &str) -> Result<Value, String> {
    let dev = dev_token().await?;
    let user = USER.lock().unwrap().clone().ok_or("Not signed in to Apple Music.")?;
    let mut tries = 0;
    loop {
        let r = client().get(format!("{API}{path}")).bearer_auth(&dev).header("Media-User-Token", &user).header("Origin", SITE).header("Referer", format!("{SITE}/")).send().await.map_err(|e| e.to_string())?;
        let status = r.status().as_u16();
        // too many at once: a moment, then again
        if status == 429 && tries < 4 {
            tries += 1;
            tokio::time::sleep(Duration::from_secs(2 * tries)).await;
            continue;
        }
        if status == 401 || status == 403 {
            return Err("Apple Music didn't accept the sign-in. Try connecting again.".into());
        }
        if !(200..300).contains(&status) {
            return Err(format!("Apple Music answered {status}."));
        }
        return r.json().await.map_err(|e| e.to_string());
    }
}

/// Every page of a list ("data" and "next")
async fn all(first: &str, cap: usize) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    let mut next = Some(first.to_string());
    while let Some(p) = next {
        let v = get(&p).await?;
        out.extend(v["data"].as_array().cloned().unwrap_or_default());
        next = v["next"].as_str().map(|n| if n.contains("include=") || !first.contains("include=") { n.to_string() } else { format!("{n}&include=catalog") });
        if out.len() >= cap { break; }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    Ok(out)
}

/// A library song as the app takes it: { id, title, artist, album, ms, isrc, art }
fn song(v: &Value) -> Option<Value> {
    let a = &v["attributes"];
    let title = a["name"].as_str()?;
    let cat = &v["relationships"]["catalog"]["data"][0]["attributes"];
    let art = a["artwork"]["url"].as_str().or(cat["artwork"]["url"].as_str()).unwrap_or("").replace("{w}", "300").replace("{h}", "300");
    Some(json!({
        "id": v["id"], "title": title, "artist": a["artistName"].as_str().unwrap_or(""), "album": a["albumName"].as_str().unwrap_or(""),
        "ms": a["durationInMillis"].as_u64().unwrap_or(0), "isrc": cat["isrc"].as_str().unwrap_or(""), "art": art,
    }))
}

/// What's imported, read only: { likes: [song], playlists: [{ name, tracks: [song] }] }
/// (likes: the songs you loved; the Favourite Songs list among the playlists is left out, as it's those)
pub async fn library() -> Result<Value, String> {
    let songs = all("/v1/me/library/songs?limit=100&include=catalog", 20_000).await?;
    // which you loved: their ratings, 50 at a time
    let mut loved = std::collections::HashSet::new();
    for chunk in songs.chunks(50) {
        let ids: Vec<&str> = chunk.iter().filter_map(|s| s["id"].as_str()).collect();
        if ids.is_empty() { continue; }
        let v = get(&format!("/v1/me/ratings/library-songs?ids={}", ids.join(","))).await.unwrap_or_default();
        for r in v["data"].as_array().into_iter().flatten() {
            if r["attributes"]["value"].as_i64() == Some(1) { if let Some(id) = r["id"].as_str() { loved.insert(id.to_string()); } }
        }
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    let likes: Vec<Value> = songs.iter().filter(|s| s["id"].as_str().is_some_and(|i| loved.contains(i))).filter_map(song).collect();
    let mut playlists = Vec::new();
    for p in all("/v1/me/library/playlists?limit=100", 2_000).await? {
        let (Some(id), name) = (p["id"].as_str(), p["attributes"]["name"].as_str().unwrap_or("Playlist")) else { continue };
        let lower = name.to_lowercase();
        if lower == "favourite songs" || lower == "favorite songs" { continue; }
        let tracks = all(&format!("/v1/me/library/playlists/{id}/tracks?limit=100&include=catalog"), 5_000).await.unwrap_or_default();
        playlists.push(json!({ "name": name, "tracks": tracks.iter().filter_map(song).collect::<Vec<_>>() }));
        tokio::time::sleep(Duration::from_millis(120)).await;
    }
    Ok(json!({ "likes": likes, "playlists": playlists, "songs": songs.len() }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_library_song() {
        let v = serde_json::json!({ "id": "i.abc", "attributes": { "name": "Strobe", "artistName": "deadmau5", "albumName": "For Lack of a Better Name", "durationInMillis": 637000,
            "artwork": { "url": "https://x/{w}x{h}bb.jpg" } }, "relationships": { "catalog": { "data": [{ "attributes": { "isrc": "USUS10900001" } }] } } });
        let s = super::song(&v).unwrap();
        assert_eq!(s["isrc"], "USUS10900001");
        assert_eq!(s["art"], "https://x/300x300bb.jpg");
        assert_eq!(s["ms"], 637000);
    }
}

