// Synced lyrics from lrclib.net

use serde_json::Value;

// Lets the page look up lyrics without running into cross-site rules
pub async fn lyrics_get(url: String) -> Result<Value, String> {
    if !url.starts_with("https://lrclib.net/api/") {
        return Err("That address is not allowed".to_string());
    }
    let res = reqwest::Client::new()
        .get(&url)
        .header("User-Agent", "OmniPulse (personal music player)")
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() {
        return Err(format!("Lyrics lookup failed ({status})"));
    }
    serde_json::from_str(&text).map_err(|e| e.to_string())
}
