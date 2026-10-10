// Collaborative playlists (as the web app's js/collab.js): a playlist on this PC that friends can add to
// and take from too. Its owner invites them: the playlist is shared on the relay (crate::share) with an
// edit key, and the invite link carries it. Opening the invite joins it: it's one of their playlists
// from then on. Each change anyone makes goes to the relay at once, and every app takes the others' in
// when the playlist is opened and once a minute. Your own music files stay yours (never sent).
//
// A playlist's "collab": { code, relay, k (the owner's key, or the edit key), owner, rev }

use crate::track::Track;
use serde_json::{json, Value};

pub const PULL_SECS: u64 = 60;

pub fn is_collab(x: &Value) -> bool {
    x["collab"]["code"].as_str().is_some_and(|c| !c.is_empty())
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build().unwrap_or_default()
}

async fn answer(r: reqwest::Response) -> Result<Value, String> {
    let ok = r.status().is_success();
    let code = r.status().as_u16();
    let v: Value = r.json().await.unwrap_or_default();
    if !ok { return Err(v["error"].as_str().map(String::from).unwrap_or_else(|| format!("the relay answered {code}"))); }
    Ok(v)
}

/// The owner: made collaborative (it's shared already: `share` is its { code, key, relay }); the invite link
pub async fn invite(share: &Value) -> Result<(String, Value), String> {
    let (Some(code), Some(key), Some(relay)) = (share["code"].as_str(), share["key"].as_str(), share["relay"].as_str()) else { return Err("it isn't shared yet".into()) };
    let v = answer(client().post(format!("{relay}/api/shares/{code}/collab?key={}", crate::sources::urlencode(key))).send().await.map_err(|e| e.to_string())?).await?;
    let edit = v["editKey"].as_str().ok_or("the relay gave no edit key")?;
    Ok((format!("{relay}/p/{code}?e={edit}"), json!({ "code": code, "relay": relay, "k": key, "owner": true, "rev": 0 })))
}

/// The playlist as it is on the relay
pub async fn fetch(relay: &str, code: &str) -> Result<Value, String> {
    answer(client().get(format!("{relay}/api/shares/{code}")).send().await.map_err(|e| e.to_string())?).await
}

/// A change here, to everyone: the relay's revision after it
pub async fn send(c: &Value, op: Value) -> Result<u64, String> {
    let (Some(code), Some(k), Some(relay)) = (c["code"].as_str(), c["k"].as_str(), c["relay"].as_str()) else { return Err("not collaborative".into()) };
    let v = answer(client().post(format!("{relay}/api/shares/{code}/edit?k={}", crate::sources::urlencode(k))).json(&op).send().await.map_err(|e| e.to_string())?).await?;
    Ok(v["rev"].as_u64().unwrap_or(0))
}

pub fn add_op(t: &Track) -> Value {
    let links: serde_json::Map<String, Value> = crate::share::links(t).into_iter().map(|(k, v)| (k.to_string(), json!(v))).collect();
    json!({ "op": "add", "track": { "title": t.title, "artist": t.artist, "ms": t.ms, "links": links, "snap": t.to_snapshot() } })
}

pub fn remove_op(key: &str) -> Value {
    json!({ "op": "remove", "id": key })
}

/// Stopping it (the owner): everyone keeps a copy
pub async fn stop(c: &Value) {
    if let (Some(code), Some(k), Some(relay)) = (c["code"].as_str(), c["k"].as_str(), c["relay"].as_str()) {
        let _ = client().post(format!("{relay}/api/shares/{code}/collab?stop=1&key={}", crate::sources::urlencode(k))).send().await;
    }
}

/// What a pull brings: the relay's tracks (snapshots, files left out) with your own files kept, its title
/// and revision; None when nothing's changed. Err(()) when its owner stopped it (it's kept as yours).
pub fn merged(x: &Value, share: &Value) -> Result<Option<(Vec<Value>, String, u64)>, ()> {
    if share["collab"].as_bool() == Some(false) && x["collab"]["owner"].as_bool() != Some(true) {
        return Err(());
    }
    let rev = share["rev"].as_u64().unwrap_or(0);
    if rev == x["collab"]["rev"].as_u64().unwrap_or(0) {
        return Ok(None);
    }
    let local = |s: &Value| s["source"].as_str() == Some("local");
    let mut tracks: Vec<Value> = share["tracks"].as_array().into_iter().flatten().map(|t| t["snap"].clone()).filter(|s| !s["id"].is_null() && !local(s)).collect();
    tracks.extend(x["tracks"].as_array().into_iter().flatten().filter(|s| local(s)).cloned());
    let title = share["title"].as_str().filter(|t| !t.is_empty()).or(x["title"].as_str()).unwrap_or("Playlist").to_string();
    Ok(Some((tracks, title, rev)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pulls_keep_your_files() {
        let x = json!({ "title": "Ours", "collab": { "code": "abcd1234", "rev": 1, "owner": false }, "tracks": [{ "id": "f1", "source": "local" }, { "id": "1", "source": "soundcloud" }] });
        let share = json!({ "title": "Ours!", "rev": 2, "tracks": [{ "snap": { "id": "sp:a", "source": "spotify" } }, { "snap": { "id": "f9", "source": "local" } }] });
        let (tracks, title, rev) = merged(&x, &share).unwrap().unwrap();
        assert_eq!(tracks.iter().map(|t| t["id"].as_str().unwrap()).collect::<Vec<_>>(), ["sp:a", "f1"]);
        assert_eq!((title.as_str(), rev), ("Ours!", 2));
        assert!(merged(&x, &json!({ "rev": 1 })).unwrap().is_none());
        assert!(merged(&x, &json!({ "collab": false, "rev": 3 })).is_err());
        assert!(is_collab(&x));
    }
}

/// cargo test -p omnipulse collab_live -- --ignored (with the relay running here: cd jam-server; npx wrangler dev --port 8799 --local)
#[cfg(test)]
mod live {
    use super::*;
    use crate::track::{Src, Track};

    #[test]
    #[ignore]
    fn collab_live() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let relay = "http://127.0.0.1:8799";
            let a = Track { src: Src::YouTube("dQw4w9WgXcQ".into()), key: "yt:dQw4w9WgXcQ".into(), title: "Never Gonna Give You Up".into(), artist: "Rick Astley".into(), ms: 213_000, ..Default::default() };
            let (_, info) = crate::share::share_playlist(relay, "Ours", "Tester", std::slice::from_ref(&a), &Value::Null).await.expect("shared");
            let (link, c) = invite(&info).await.expect("invited");
            assert!(link.contains("?e="));
            // a friend joins with the edit key, and adds a song
            let edit = link.split("?e=").nth(1).unwrap();
            let friend = json!({ "code": c["code"], "relay": relay, "k": edit, "owner": false, "rev": 0 });
            let b = Track { src: Src::YouTube("9bZkp7q19f0".into()), key: "yt:9bZkp7q19f0".into(), title: "Gangnam Style".into(), artist: "PSY".into(), ms: 252_000, ..Default::default() };
            let rev = send(&friend, add_op(&b)).await.expect("sent");
            assert!(rev >= 1);
            // the owner pulls it in
            let x = json!({ "title": "Ours", "collab": c, "tracks": [a.to_snapshot()] });
            let share = fetch(relay, c["code"].as_str().unwrap()).await.expect("fetched");
            let (tracks, _, got) = merged(&x, &share).unwrap().expect("changed");
            assert_eq!(got, rev);
            assert_eq!(tracks.len(), 2);
            // removing it, then stopping it: the friend's copy is told
            send(&friend, remove_op("yt:9bZkp7q19f0")).await.expect("removed");
            stop(&c).await;
            let after = fetch(relay, c["code"].as_str().unwrap()).await.expect("fetched");
            assert!(merged(&friend_x(&friend), &after).is_err());
        });
    }

    fn friend_x(c: &Value) -> Value {
        json!({ "title": "Ours", "collab": c, "tracks": [] })
    }
}
