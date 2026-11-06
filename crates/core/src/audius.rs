// Audius (audius.co): a free, open music service with a public API (no key, no sign-in; it asks only for an
// app's name). Its songs are uploaded by their artists, electronic music above all, and play as MP3 from the
// address its stream call sends on to. Here it stands in for a song YouTube can't play just now, when it has
// the same one (crate::versions in the app decides that).

use serde_json::Value;
use std::time::Duration;

const API: &str = "https://api.audius.co/v1";
const APP: &str = "OmniPulse";

#[derive(Clone, Debug, PartialEq)]
pub struct AudiusTrack {
    pub id: String,
    pub title: String,
    /// who uploaded it (the artist, or whoever remixed it)
    pub artist: String,
    pub duration_ms: u64,
}

/// A search's songs: those anyone can play in full (not the ones kept for buyers or followers)
pub fn parse(v: &Value) -> Vec<AudiusTrack> {
    v["data"].as_array().map(|l| l.iter().filter_map(|t| {
        if t["is_streamable"].as_bool() == Some(false) || t["is_stream_gated"].as_bool() == Some(true) || t["is_delete"].as_bool() == Some(true) { return None; }
        let id = t["id"].as_str().filter(|x| !x.is_empty() && x.chars().all(|c| c.is_ascii_alphanumeric()))?;
        Some(AudiusTrack {
            id: id.to_string(),
            title: t["title"].as_str()?.trim().to_string(),
            artist: t["user"]["name"].as_str().unwrap_or("").trim().to_string(),
            duration_ms: t["duration"].as_u64().unwrap_or(0) * 1000,
        })
    }).collect()).unwrap_or_default()
}

pub async fn search(query: &str) -> Result<Vec<AudiusTrack>, String> {
    let client = reqwest::Client::builder().user_agent(APP).timeout(Duration::from_secs(10)).build().map_err(|e| e.to_string())?;
    let res = client.get(format!("{API}/tracks/search")).query(&[("query", query), ("app_name", APP), ("limit", "10")]).send().await.map_err(|e| format!("couldn't reach Audius: {e}"))?;
    if !res.status().is_success() { return Err(format!("Audius said {}", res.status().as_u16())); }
    Ok(parse(&res.json::<Value>().await.map_err(|e| e.to_string())?))
}

/// Where a song plays from (it sends the player on to the MP3 itself)
pub fn stream_url(id: &str) -> String {
    format!("{API}/tracks/{id}/stream?app_name={APP}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_songs_anyone_can_play_are_kept() {
        let v = json!({ "data": [
            { "id": "bvrxK", "title": " Holding On ", "user": { "name": "Dabin" }, "duration": 181, "is_streamable": true, "is_stream_gated": false },
            { "id": "gated1", "title": "Paid", "user": { "name": "X" }, "duration": 100, "is_stream_gated": true },
            { "id": "bad/id", "title": "Odd", "user": { "name": "X" }, "duration": 100 },
        ] });
        let got = parse(&v);
        assert_eq!(got, vec![AudiusTrack { id: "bvrxK".into(), title: "Holding On".into(), artist: "Dabin".into(), duration_ms: 181_000 }]);
        assert_eq!(stream_url("bvrxK"), "https://api.audius.co/v1/tracks/bvrxK/stream?app_name=OmniPulse");
        assert!(parse(&json!({})).is_empty());
    }
}
