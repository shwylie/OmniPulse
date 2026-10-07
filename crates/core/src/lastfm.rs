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
        "artist.getSimilar" | "artist.getTopTags" => (false, false),
        _ => return Err("That Last.fm method is not allowed".to_string()),
    };
    lfm_request(&cfg, &method, params, signed, post).await
}
