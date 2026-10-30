//! Badges chatters carry from outside your stream: FrankerFaceZ's, BetterTTV's and Chatterino's (by Twitch
//! account, from their public lists) and 7TV's (Twitch or Kick, asked for each chatter as they're first seen).
//! For the Chat Dock and the Multichat Overlay, served at /dock/badges.json as
//! { n, users: { "twitch:ID": [{ u: its picture, t: its name }] } } for the chatters seen lately; n goes up
//! whenever any of them changes, so a page knows to take them again.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

type Badge = (String, String);

/// Twitch user id: their badges from FFZ, BTTV and Chatterino
static LISTS: Mutex<Option<HashMap<String, Vec<Badge>>>> = Mutex::new(None);
/// 7TV's badges by id: (picture, name)
static CATALOG: Mutex<Option<HashMap<String, Badge>>> = Mutex::new(None);
/// The chatters seen lately ("twitch:ID", "kick:ID"), oldest first, each with its 7TV badge once asked
static SEEN: Mutex<Vec<(String, Option<Option<Badge>>)>> = Mutex::new(Vec::new());
static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// how many chatters are kept
const KEPT: usize = 600;

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

/// FFZ: { badges: [{ id, title, urls: { "2" } }], users: { badge id: [twitch ids] } }
fn ffz(v: &Value, into: &mut HashMap<String, Vec<Badge>>) {
    let by_id: HashMap<String, Badge> = v["badges"].as_array().into_iter().flatten().filter_map(|b| {
        let u = b["urls"]["2"].as_str().or_else(|| b["image"].as_str())?;
        Some((b["id"].to_string(), (https(u), b["title"].as_str().unwrap_or("FrankerFaceZ").to_string())))
    }).collect();
    for (id, users) in v["users"].as_object().into_iter().flatten() {
        let Some(b) = by_id.get(id) else { continue };
        for u in users.as_array().into_iter().flatten() {
            let key = u.as_str().map(String::from).unwrap_or_else(|| u.to_string());
            into.entry(key).or_default().push(b.clone());
        }
    }
}

/// BTTV: [{ providerId, badge: { description, svg } }]
fn bttv(v: &Value, into: &mut HashMap<String, Vec<Badge>>) {
    for e in v.as_array().into_iter().flatten() {
        let (Some(id), Some(u)) = (e["providerId"].as_str(), e["badge"]["svg"].as_str()) else { continue };
        into.entry(id.to_string()).or_default().push((https(u), e["badge"]["description"].as_str().unwrap_or("BetterTTV").to_string()));
    }
}

/// Chatterino: { badges: [{ tooltip, image2, users: [twitch ids] }] }
fn chatterino(v: &Value, into: &mut HashMap<String, Vec<Badge>>) {
    for b in v["badges"].as_array().into_iter().flatten() {
        let Some(u) = b["image2"].as_str().or_else(|| b["image1"].as_str()) else { continue };
        let t = b["tooltip"].as_str().unwrap_or("Chatterino").to_string();
        for id in b["users"].as_array().into_iter().flatten().filter_map(|x| x.as_str()) {
            into.entry(id.to_string()).or_default().push((https(u), t.clone()));
        }
    }
}

/// 7TV's badges: { data: { cosmetics: { badges: [{ id, tooltip, host: { url } }] } } }
fn seventv_catalog(v: &Value) -> HashMap<String, Badge> {
    v["data"]["cosmetics"]["badges"].as_array().into_iter().flatten().filter_map(|b| {
        Some((b["id"].as_str()?.to_string(), (format!("{}/2x.webp", https(b["host"]["url"].as_str()?)), b["tooltip"].as_str().unwrap_or("7TV").to_string())))
    }).collect()
}

/// Fetched again: the lists of who has which (an hour old at most is fine; they change rarely)
pub async fn refresh() {
    let c = client();
    let mut lists = HashMap::new();
    ffz(&get(&c, "https://api.frankerfacez.com/v1/badges/ids").await, &mut lists);
    bttv(&get(&c, "https://api.betterttv.net/3/cached/badges/twitch").await, &mut lists);
    chatterino(&get(&c, "https://api.chatterino.com/badges").await, &mut lists);
    let q = json!({ "query": "{cosmetics{badges{id tooltip host{url}}}}" });
    let cat = match c.post("https://7tv.io/v3/gql").json(&q).send().await { Ok(r) if r.status().is_success() => seventv_catalog(&r.json().await.unwrap_or(Value::Null)), _ => HashMap::new() };
    // (none at all: kept as they were, as the lists may only be unreachable for now)
    if !lists.is_empty() { *LISTS.lock().unwrap() = Some(lists); }
    if !cat.is_empty() { *CATALOG.lock().unwrap() = Some(cat); }
    N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// A chatter seen (platform "twitch" or "kick", their account id there): kept, to be asked 7TV about
pub fn seen(platform: &str, id: &str) {
    if id.is_empty() || !matches!(platform, "twitch" | "kick") { return; }
    let key = format!("{platform}:{id}");
    let mut s = SEEN.lock().unwrap();
    if let Some(at) = s.iter().position(|x| x.0 == key) {
        let x = s.remove(at);
        s.push(x);
        return;
    }
    s.push((key, None));
    if s.len() > KEPT { let over = s.len() - KEPT; s.drain(..over); }
}

/// The chatters not asked about yet: 7TV asked for each one's badge (a few at a time)
pub async fn fill() {
    let ask: Vec<String> = SEEN.lock().unwrap().iter().filter(|x| x.1.is_none()).map(|x| x.0.clone()).take(8).collect();
    if ask.is_empty() { return; }
    let c = client();
    for key in ask {
        let Some((plat, id)) = key.split_once(':') else { continue };
        let v = get(&c, &format!("https://7tv.io/v3/users/{plat}/{id}")).await;
        let badge = v["user"]["style"]["badge_id"].as_str().and_then(|b| CATALOG.lock().unwrap().as_ref().and_then(|c| c.get(b).cloned()));
        let mut s = SEEN.lock().unwrap();
        if let Some(x) = s.iter_mut().find(|x| x.0 == key) { x.1 = Some(badge.clone()); }
        if badge.is_some() { N.fetch_add(1, std::sync::atomic::Ordering::Relaxed); }
    }
}

/// What a page asks for: each chatter seen lately who has any
pub fn json() -> Value {
    let lists = LISTS.lock().unwrap();
    let mut users = serde_json::Map::new();
    for (key, sev) in SEEN.lock().unwrap().iter() {
        let mut all: Vec<&Badge> = Vec::new();
        if let (Some(id), Some(l)) = (key.strip_prefix("twitch:"), lists.as_ref()) { all.extend(l.get(id).into_iter().flatten()); }
        if let Some(Some(b)) = sev { all.push(b); }
        if !all.is_empty() { users.insert(key.clone(), json!(all.iter().map(|(u, t)| json!({ "u": u, "t": t })).collect::<Vec<_>>())); }
    }
    json!({ "n": N.load(std::sync::atomic::Ordering::Relaxed), "users": users })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn each_services_list_reads() {
        let mut m = HashMap::new();
        ffz(&json!({ "badges": [{ "id": 3, "title": "FFZ Supporter", "urls": { "2": "https://cdn.frankerfacez.com/badge/3/2" } }], "users": { "3": [41, "42"] } }), &mut m);
        bttv(&json!([{ "providerId": "41", "badge": { "description": "NightDev Developer", "svg": "https://cdn.betterttv.net/badges/developer.svg" } }]), &mut m);
        chatterino(&json!({ "badges": [{ "tooltip": "Chatterino Top Donator", "image2": "https://fourtf.com/x2.png", "users": ["42"] }] }), &mut m);
        assert_eq!(m["41"], vec![("https://cdn.frankerfacez.com/badge/3/2".to_string(), "FFZ Supporter".to_string()), ("https://cdn.betterttv.net/badges/developer.svg".to_string(), "NightDev Developer".to_string())]);
        assert_eq!(m["42"].len(), 2);
        let cat = seventv_catalog(&json!({ "data": { "cosmetics": { "badges": [{ "id": "B", "tooltip": "7TV Subscriber", "host": { "url": "//cdn.7tv.app/badge/B" } }] } } }));
        assert_eq!(cat["B"], ("https://cdn.7tv.app/badge/B/2x.webp".to_string(), "7TV Subscriber".to_string()));
    }

    #[test]
    #[ignore]
    fn the_lists_are_reachable() {
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            refresh().await;
            // (a 7TV account known to have a badge: 7TV's own)
            seen("twitch", "71092938");
            fill().await;
        });
        assert!(LISTS.lock().unwrap().as_ref().is_some_and(|l| l.len() > 1000));
        assert!(CATALOG.lock().unwrap().as_ref().is_some_and(|c| c.len() > 10));
        let v = json();
        assert!(v["users"]["twitch:71092938"].as_array().is_some_and(|a| !a.is_empty()), "{v}");
    }
}
