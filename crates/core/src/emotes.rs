//! BetterTTV, FrankerFaceZ and 7TV emotes, for the Chat Dock and the Multichat Overlay: their global ones and
//! your channel's (your Twitch account's, and your Kick account's on 7TV), from their public lists (no key, no
//! sign-in). Served at /dock/emotes.json as { n, list: [{ c: its code, u: its picture, f: bttv | ffz | 7tv }] };
//! n goes up each time they're fetched again, so a page knows to take them.

use serde_json::{json, Value};
use std::sync::Mutex;
use std::time::Duration;

static LIST: Mutex<Vec<(String, String, &'static str)>> = Mutex::new(Vec::new());
static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn client() -> reqwest::Client {
    reqwest::Client::builder().user_agent("OmniPulse").timeout(Duration::from_secs(10)).build().unwrap_or_default()
}

async fn get(c: &reqwest::Client, url: &str) -> Value {
    match c.get(url).send().await {
        Ok(r) if r.status().is_success() => r.json().await.unwrap_or(Value::Null),
        _ => Value::Null,
    }
}

fn https(u: &str) -> String { if let Some(rest) = u.strip_prefix("//") { format!("https://{rest}") } else { u.to_string() } }

fn bttv(list: &Value, out: &mut Vec<(String, String, &'static str)>) {
    for e in list.as_array().into_iter().flatten() {
        let (Some(id), Some(code)) = (e["id"].as_str(), e["code"].as_str()) else { continue };
        out.push((code.to_string(), format!("https://cdn.betterttv.net/emote/{id}/2x"), "bttv"));
    }
}

/// FFZ's sets: { sets: { "3": { emoticons: [{ name, urls: { "1", "2", "4" }, animated }] } } }
fn ffz(v: &Value, out: &mut Vec<(String, String, &'static str)>) {
    for set in v["sets"].as_object().into_iter().flat_map(|m| m.values()) {
        for e in set["emoticons"].as_array().into_iter().flatten() {
            let Some(name) = e["name"].as_str() else { continue };
            let urls = if e["animated"].is_object() { &e["animated"] } else { &e["urls"] };
            let Some(u) = urls["2"].as_str().or_else(|| urls["1"].as_str()) else { continue };
            out.push((name.to_string(), https(u), "ffz"));
        }
    }
}

/// 7TV's set: { emotes: [{ name, data: { host: { url: "//cdn.7tv.app/emote/ID" } } }] }
fn seventv(set: &Value, out: &mut Vec<(String, String, &'static str)>) {
    for e in set["emotes"].as_array().into_iter().flatten() {
        let (Some(name), Some(host)) = (e["name"].as_str(), e["data"]["host"]["url"].as_str()) else { continue };
        out.push((name.to_string(), format!("{}/2x.webp", https(host)), "7tv"));
    }
}

/// Fetched again: the global ones, and your channel's (Twitch's user id, Kick's): a channel's wins over a
/// global one of the same name, 7TV's over BTTV's over FFZ's
pub async fn refresh(twitch_id: &str, kick_id: &str) {
    let c = client();
    let mut out = Vec::new();
    ffz(&get(&c, "https://api.frankerfacez.com/v1/set/global").await, &mut out);
    bttv(&get(&c, "https://api.betterttv.net/3/cached/emotes/global").await, &mut out);
    seventv(&get(&c, "https://7tv.io/v3/emote-sets/global").await, &mut out);
    if !twitch_id.trim().is_empty() {
        let id = twitch_id.trim();
        ffz(&get(&c, &format!("https://api.frankerfacez.com/v1/room/id/{id}")).await, &mut out);
        let b = get(&c, &format!("https://api.betterttv.net/3/cached/users/twitch/{id}")).await;
        bttv(&b["channelEmotes"], &mut out);
        bttv(&b["sharedEmotes"], &mut out);
        seventv(&get(&c, &format!("https://7tv.io/v3/users/twitch/{id}")).await["emote_set"], &mut out);
    }
    if !kick_id.trim().is_empty() {
        seventv(&get(&c, &format!("https://7tv.io/v3/users/kick/{}", kick_id.trim())).await["emote_set"], &mut out);
    }
    // (none at all: kept as they were, as the lists may only be unreachable for now)
    if out.is_empty() { return; }
    *LIST.lock().unwrap() = out;
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// What a page asks for: the later of two with the same code wins, as fetched in that order
pub fn json() -> Value {
    let list = LIST.lock().unwrap();
    json!({ "n": N.load(std::sync::atomic::Ordering::Relaxed), "list": list.iter().map(|(c, u, f)| json!({ "c": c, "u": u, "f": f })).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_services_list_reads() {
        let mut out = Vec::new();
        bttv(&json!([{ "id": "abc", "code": "monkaS" }]), &mut out);
        ffz(&json!({ "sets": { "3": { "emoticons": [{ "name": "LUL", "urls": { "1": "//cdn.frankerfacez.com/1", "2": "//cdn.frankerfacez.com/2" } }] } } }), &mut out);
        seventv(&json!({ "emotes": [{ "name": "catJAM", "data": { "host": { "url": "//cdn.7tv.app/emote/X" } } }] }), &mut out);
        assert_eq!(out[0], ("monkaS".to_string(), "https://cdn.betterttv.net/emote/abc/2x".to_string(), "bttv"));
        assert_eq!(out[1].1, "https://cdn.frankerfacez.com/2");
        assert_eq!(out[2].1, "https://cdn.7tv.app/emote/X/2x.webp");
    }

    #[test]
    #[ignore]
    fn the_lists_are_reachable() {
        tokio::runtime::Runtime::new().unwrap().block_on(refresh("", ""));
        let v = json();
        assert!(v["list"].as_array().unwrap().len() > 50, "{}", v.to_string().chars().take(300).collect::<String>());
        for f in ["bttv", "ffz", "7tv"] { assert!(v["list"].as_array().unwrap().iter().any(|x| x["f"] == f), "none from {f}"); }
    }
}
