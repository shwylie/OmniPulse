// Last.fm: connecting, now playing, scrobbles and similar artists

use crate::{data_dir, load_config, Config};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{fs, time::Duration};

#[derive(Serialize, Deserialize, Clone)]
pub struct LfmSession {
    key: String,
    name: String,
}

pub fn load_lfm() -> Option<LfmSession> {
    let text = fs::read_to_string(data_dir().ok()?.join("lastfm.json")).ok()?;
    serde_json::from_str(&text).ok()
}

pub fn save_lfm(s: &LfmSession) -> Result<(), String> {
    let text = serde_json::to_string(s).map_err(|e| e.to_string())?;
    fs::write(data_dir()?.join("lastfm.json"), text).map_err(|e| e.to_string())
}

pub fn lfm_sign(params: &[(String, String)], secret: &str) -> String {
    let mut sorted: Vec<&(String, String)> = params
        .iter()
        .filter(|(k, _)| k != "format" && k != "callback")
        .collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    let mut s = String::new();
    for (k, v) in sorted {
        s.push_str(k);
        s.push_str(v);
    }
    s.push_str(secret);
    format!("{:x}", md5::compute(s.as_bytes()))
}

pub async fn lfm_request(
    cfg: &Config,
    method: &str,
    mut params: Vec<(String, String)>,
    signed: bool,
    post: bool,
) -> Result<Value, String> {
    if cfg.lastfm_api_key.is_empty() {
        return Err("Add lastfm_api_key (and lastfm_secret) to config.json first".to_string());
    }
    params.push(("method".to_string(), method.to_string()));
    params.push(("api_key".to_string(), cfg.lastfm_api_key.clone()));
    if signed {
        if cfg.lastfm_secret.is_empty() {
            return Err("Add lastfm_secret to config.json first".to_string());
        }
        if method != "auth.getToken" && method != "auth.getSession" {
            if let Some(s) = load_lfm() {
                params.push(("sk".to_string(), s.key));
            }
        }
        let sig = lfm_sign(&params, &cfg.lastfm_secret);
        params.push(("api_sig".to_string(), sig));
    }
    params.push(("format".to_string(), "json".to_string()));
    let client = reqwest::Client::new();
    let req = if post {
        client.post("https://ws.audioscrobbler.com/2.0/").form(&params)
    } else {
        client.get("https://ws.audioscrobbler.com/2.0/").query(&params)
    };
    let res = req.send().await.map_err(|e| e.to_string())?;
    let status = res.status();
    let text = res.text().await.map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_str(&text)
        .map_err(|_| format!("Last.fm replied {status}: {text}"))?;
    if let Some(code) = v.get("error") {
        return Err(format!(
            "Last.fm error {}: {}",
            code,
            v["message"].as_str().unwrap_or("")
        ));
    }
    Ok(v)
}

pub fn lastfm_status() -> Value {
    match load_lfm() {
        Some(s) => serde_json::json!({ "connected": true, "name": s.name }),
        None => serde_json::json!({ "connected": false, "name": "" }),
    }
}

pub fn lastfm_disconnect() -> Result<(), String> {
    let path = data_dir()?.join("lastfm.json");
    if path.exists() {
        fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    Ok(())
}

// Opens Last.fm in the browser and waits (up to two minutes) for you to approve
pub async fn lastfm_connect() -> Result<String, String> {
    let cfg = load_config()?;
    let tok = lfm_request(&cfg, "auth.getToken", Vec::new(), true, false).await?;
    let token = tok["token"].as_str().ok_or("No token from Last.fm")?.to_string();
    let url = format!(
        "https://www.last.fm/api/auth/?api_key={}&token={}",
        urlencoding::encode(&cfg.lastfm_api_key),
        urlencoding::encode(&token)
    );
    opener::open(&url).map_err(|e| e.to_string())?;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_secs(3)).await;
        let params = vec![("token".to_string(), token.clone())];
        match lfm_request(&cfg, "auth.getSession", params, true, false).await {
            Ok(v) => {
                let key = v["session"]["key"]
                    .as_str()
                    .ok_or("No session key from Last.fm")?
                    .to_string();
                let name = v["session"]["name"].as_str().unwrap_or("").to_string();
                save_lfm(&LfmSession { key, name: name.clone() })?;
                return Ok(name);
            }
            Err(e) => {
                if !e.contains("error 14") {
                    return Err(e);
                }
            }
        }
    }
    Err("Timed out waiting for you to approve in the browser".to_string())
}

pub async fn lastfm_call(method: String, params: Vec<(String, String)>) -> Result<Value, String> {
    let cfg = load_config()?;
    let (signed, post) = match method.as_str() {
        "track.updateNowPlaying" | "track.scrobble" => {
            if load_lfm().is_none() {
                return Err("Not connected to Last.fm".to_string());
            }
            (true, true)
        }
        "artist.getSimilar" | "artist.getTopTags" | "tag.getTopArtists" => (false, false),
        _ => return Err("That Last.fm method is not allowed".to_string()),
    };
    lfm_request(&cfg, &method, params, signed, post).await
}

/// What you've scrobbled since `from` (unix seconds), newest first, as text only:
/// [{ artist, title, ts }] (ts in ms). Only your own account's, up to `pages` pages of 200.
pub async fn lastfm_recent(from: i64, pages: u32) -> Result<Value, String> {
    let cfg = load_config()?;
    let user = load_lfm().ok_or("Not connected to Last.fm")?.name;
    let mut out = Vec::new();
    for page in 1..=pages.clamp(1, 10) {
        let params = vec![
            ("user".to_string(), user.clone()),
            ("from".to_string(), from.to_string()),
            ("limit".to_string(), "200".to_string()),
            ("page".to_string(), page.to_string()),
        ];
        let v = lfm_request(&cfg, "user.getRecentTracks", params, false, false).await?;
        let list = &v["recenttracks"]["track"];
        let items: Vec<&Value> = match list {
            Value::Array(a) => a.iter().collect(),
            Value::Object(_) => vec![list],
            _ => Vec::new(),
        };
        for t in items {
            // the track playing right now has no date yet
            let Some(uts) = t["date"]["uts"].as_str().and_then(|s| s.parse::<i64>().ok()) else { continue };
            let artist = t["artist"]["#text"].as_str().or(t["artist"]["name"].as_str()).unwrap_or("");
            let title = t["name"].as_str().unwrap_or("");
            if !artist.is_empty() && !title.is_empty() {
                out.push(serde_json::json!({ "artist": artist, "title": title, "ts": uts * 1000 }));
            }
        }
        let total: u32 = v["recenttracks"]["@attr"]["totalPages"].as_str().and_then(|s| s.parse().ok()).unwrap_or(1);
        if page >= total {
            break;
        }
    }
    Ok(Value::Array(out))
}

#[cfg(test)]
mod live {
    // your own scrobbles from the past week (read-only): cargo test -p omnipulse-core lfm_recent_live -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn lfm_recent_live() {
        let from = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() - 7 * 86400) as i64;
        match super::lastfm_recent(from, 5).await {
            Ok(v) => {
                let n = v.as_array().map(|a| a.len()).unwrap_or(0);
                let ok = v.as_array().unwrap().iter().all(|x| x["artist"].is_string() && x["title"].is_string() && x["ts"].is_i64());
                println!("scrobbles this week: {n}, all with artist/title/time: {ok}");
            }
            Err(e) => println!("not available: {e}"),
        }
    }
}
