// SponsorBlock (sponsor.ajay.app): segments of YouTube videos that people have marked, here only its
// "music_offtopic" category, the non-music parts of a music video (a skit before the song, an
// interview after it). Asked by the first four characters of the video id's SHA-256, as SponsorBlock
// recommends for privacy: the answer has every video with that prefix, and the one wanted is picked
// out here, so SponsorBlock doesn't learn which video is playing. Kept for a day.

use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const API: &str = "https://sponsor.ajay.app/api/skipSegments";
const KEEP_FOR: Duration = Duration::from_secs(86400);
static CACHE: Mutex<Option<HashMap<String, (Instant, Vec<(f64, f64)>)>>> = Mutex::new(None);

fn prefix(video_id: &str) -> String {
    let h = Sha256::digest(video_id.as_bytes());
    h.iter().take(2).map(|b| format!("{b:02x}")).collect()
}

/// The segments of this video in the answer for its prefix: [(start, end)] in seconds, skips only
fn pick(answer: &Value, video_id: &str) -> Vec<(f64, f64)> {
    let mut out: Vec<(f64, f64)> = answer
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["videoID"].as_str() == Some(video_id))
        .flat_map(|v| v["segments"].as_array().cloned().unwrap_or_default())
        .filter(|s| s["category"].as_str() == Some("music_offtopic") && s["actionType"].as_str().unwrap_or("skip") == "skip")
        .filter_map(|s| Some((s["segment"][0].as_f64()?, s["segment"][1].as_f64()?)))
        .filter(|(a, b)| b > a)
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// The non-music parts of a YouTube video: [(start, end)] in seconds (none when nobody has marked any)
pub async fn music_offtopic(video_id: &str) -> Result<Vec<(f64, f64)>, String> {
    if video_id.len() != 11 || !video_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err("Not a YouTube video id".into());
    }
    if let Some((at, v)) = CACHE.lock().unwrap().as_ref().and_then(|c| c.get(video_id)).cloned() {
        if at.elapsed() < KEEP_FOR {
            return Ok(v);
        }
    }
    let res = reqwest::Client::new()
        .get(format!("{API}/{}", prefix(video_id)))
        .query(&[("categories", "[\"music_offtopic\"]"), ("actionTypes", "[\"skip\"]")])
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|e| format!("Couldn't reach SponsorBlock: {e}"))?;
    let found = match res.status().as_u16() {
        // (nothing marked for any video with this prefix)
        404 => Vec::new(),
        s if (200..300).contains(&s) => pick(&res.json::<Value>().await.map_err(|e| e.to_string())?, video_id),
        s => return Err(format!("SponsorBlock answered {s}")),
    };
    CACHE.lock().unwrap().get_or_insert_with(HashMap::new).insert(video_id.to_string(), (Instant::now(), found.clone()));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    #[test]
    fn only_this_videos_music_offtopic_skips() {
        let a = json!([
            { "videoID": "aaaaaaaaaaa", "segments": [
                { "segment": [200.5, 230.0], "category": "music_offtopic", "actionType": "skip" },
                { "segment": [0.0, 12.0], "category": "music_offtopic", "actionType": "skip" },
                { "segment": [50.0, 60.0], "category": "sponsor", "actionType": "skip" },
                { "segment": [70.0, 80.0], "category": "music_offtopic", "actionType": "mute" } ] },
            { "videoID": "bbbbbbbbbbb", "segments": [ { "segment": [1.0, 2.0], "category": "music_offtopic", "actionType": "skip" } ] }
        ]);
        assert_eq!(super::pick(&a, "aaaaaaaaaaa"), vec![(0.0, 12.0), (200.5, 230.0)]);
    }

    #[test]
    fn a_four_character_prefix() {
        assert_eq!(super::prefix("dQw4w9WgXcQ").len(), 4);
    }
}
