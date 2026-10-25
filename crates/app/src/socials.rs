// The Social Plug (Settings > Streaming > Social Plug, or the chatbot's page while it's on): your handles on other
// platforms, for OBS (/socials), one at a time in turn or all together. Each one: a platform it knows (its icon and
// colours) or one of your own (its name, your icon, your colours), your handle there, and whether it's shown.
// Kept in prefs.socials, in order; your icons are copied in as the alerts' pictures are ("social-<id>").

use serde_json::{json, Map, Value};

/// The platforms it knows: (id, name, colour, second colour); "other" is one of your own
pub const PLATFORMS: [(&str, &str, &str, &str); 17] = [
    ("twitch", "Twitch", "#9146ff", "#6441a5"),
    ("kick", "Kick", "#53fc18", "#1f8a0a"),
    ("youtube", "YouTube", "#ff0033", "#b3001f"),
    ("tiktok", "TikTok", "#25f4ee", "#fe2c55"),
    ("instagram", "Instagram", "#f58529", "#dd2a7b"),
    ("x", "X", "#2b2b2b", "#000000"),
    ("discord", "Discord", "#5865f2", "#404eed"),
    ("bluesky", "Bluesky", "#1185fe", "#0a5fc2"),
    ("facebook", "Facebook", "#1877f2", "#0c5fd0"),
    ("threads", "Threads", "#2b2b2b", "#000000"),
    ("spotify", "Spotify", "#1db954", "#168d40"),
    ("soundcloud", "SoundCloud", "#ff5500", "#d94800"),
    ("patreon", "Patreon", "#ff424d", "#d12f39"),
    ("kofi", "Ko-fi", "#29abe0", "#1b88b5"),
    ("throne", "Throne", "#7b61ff", "#4f3bd1"),
    ("website", "Website", "#8e9bff", "#b48eff"),
    ("other", "Your Own", "#8e9bff", "#b48eff"),
];

pub fn platform(id: &str) -> (&'static str, &'static str, &'static str, &'static str) {
    PLATFORMS.iter().copied().find(|p| p.0 == id).unwrap_or(PLATFORMS[PLATFORMS.len() - 1])
}

pub fn list(prefs: &Value) -> Vec<Map<String, Value>> {
    prefs["socials"].as_array().into_iter().flatten().filter_map(|x| x.as_object().cloned()).collect()
}

fn s<'a>(m: &'a Map<String, Value>, k: &str) -> &'a str { m.get(k).and_then(|v| v.as_str()).unwrap_or("") }

/// Its name as shown: yours for one of your own, else the platform's
pub fn name_of(m: &Map<String, Value>) -> String {
    let p = platform(s(m, "platform"));
    if p.0 == "other" { let n = s(m, "name").trim(); if n.is_empty() { "Your Own".into() } else { n.into() } } else { p.1.into() }
}

/// A new one: Twitch first, then whichever isn't in the list yet; its id unused
pub fn added(prefs: &Value) -> (String, Value) {
    let mut all = list(prefs);
    let have: Vec<String> = all.iter().map(|m| s(m, "platform").to_string()).collect();
    let pick = PLATFORMS.iter().find(|p| p.0 != "other" && !have.iter().any(|h| h == p.0)).map(|p| p.0).unwrap_or("other");
    let mut n = all.len() + 1;
    while all.iter().any(|m| s(m, "id") == format!("s{n}")) { n += 1; }
    let id = format!("s{n}");
    let mut m = Map::new();
    m.insert("id".into(), json!(id));
    m.insert("platform".into(), json!(pick));
    m.insert("handle".into(), json!(""));
    m.insert("on".into(), json!(true));
    all.push(m);
    (id, json!(all))
}

/// Moved up (-1) or down (1)
pub fn moved(prefs: &Value, id: &str, by: i32) -> Value {
    let mut all = list(prefs);
    if let Some(i) = all.iter().position(|m| s(m, "id") == id) {
        let j = (i as i32 + by).clamp(0, all.len() as i32 - 1) as usize;
        all.swap(i, j);
    }
    json!(all)
}

pub fn without(prefs: &Value, id: &str) -> Value {
    json!(list(prefs).into_iter().filter(|m| s(m, "id") != id).collect::<Vec<_>>())
}

/// One changed (a field, or its icon)
pub fn with(prefs: &Value, id: &str, f: impl Fn(&mut Map<String, Value>)) -> Value {
    json!(list(prefs).into_iter().map(|mut m| { if s(&m, "id") == id { f(&mut m); } m }).collect::<Vec<_>>())
}

// ---------- the list page ----------
pub fn list_values(prefs: &Value) -> Map<String, Value> {
    list(prefs).iter().map(|m| (format!("on:{}", s(m, "id")), json!(m.get("on").and_then(|v| v.as_bool()) != Some(false)))).collect()
}

pub fn list_rows(prefs: &Value) -> Vec<Value> {
    let all = list(prefs);
    let n = all.len();
    let mut f = Vec::new();
    for (i, m) in all.iter().enumerate() {
        let id = s(m, "id");
        let mut acts = vec![format!("socEdit:{id}")];
        let mut labels = vec!["Edit".to_string()];
        if i > 0 { acts.push(format!("socUp:{id}")); labels.push("\u{2191}".into()); }
        if i + 1 < n { acts.push(format!("socDown:{id}")); labels.push("\u{2193}".into()); }
        let handle = s(m, "handle").trim();
        f.push(json!({ "key": format!("on:{id}"), "label": name_of(m), "note": if handle.is_empty() { "No handle yet: Edit to add yours.".to_string() } else { handle.to_string() }, "type": "row", "actions": acts, "labels": labels }));
    }
    f.push(json!({ "key": "socAdd", "label": "Another Platform", "note": "Any platform. One it doesn't know gets your own name, icon and colours.", "type": "button", "value": "Add" }));
    f
}

pub fn list_save(prefs: &Value, vals: &Map<String, Value>) -> Value {
    json!(list(prefs).into_iter().map(|mut m| {
        let k = format!("on:{}", s(&m, "id"));
        if let Some(v) = vals.get(&k) { m.insert("on".into(), v.clone()); }
        m
    }).collect::<Vec<_>>())
}

// ---------- one platform's own page ----------
pub fn edit_values(prefs: &Value, id: &str) -> Map<String, Value> {
    let m = list(prefs).into_iter().find(|m| s(m, "id") == id).unwrap_or_default();
    let p = platform(s(&m, "platform"));
    let mut v = Map::new();
    v.insert("platform".into(), json!(p.0));
    v.insert("name".into(), json!(s(&m, "name")));
    v.insert("handle".into(), json!(s(&m, "handle")));
    v.insert("ownCol".into(), json!(m.get("ownCol").and_then(|x| x.as_bool()) == Some(true) || p.0 == "other"));
    v.insert("col1".into(), json!(if s(&m, "col1").is_empty() { p.2 } else { s(&m, "col1") }));
    v.insert("col2".into(), json!(if s(&m, "col2").is_empty() { p.3 } else { s(&m, "col2") }));
    v
}

pub fn edit_save(prefs: &Value, id: &str, vals: &Map<String, Value>) -> Value {
    with(prefs, id, |m| {
        for k in ["platform", "name", "handle", "ownCol", "col1", "col2"] { if let Some(v) = vals.get(k) { m.insert(k.into(), v.clone()); } }
    })
}

/// A platform's icon (24 x 24), as the widget draws it: read from the widget's own page, so the two never
/// differ; "" for one it hasn't (your own: its first letter)
pub fn icon_path(platform: &str) -> String {
    static ICONS: std::sync::OnceLock<Vec<(String, String)>> = std::sync::OnceLock::new();
    let all = ICONS.get_or_init(|| {
        let page = include_str!("../../core/widget/socials.html");
        let Some(start) = page.find("const ICONS = {") else { return Vec::new() };
        let body = &page[start..];
        let body = &body[..body.find("};").unwrap_or(body.len())];
        body.lines().filter_map(|l| {
            let (k, v) = l.trim().split_once(": \"")?;
            Some((k.trim().to_string(), v.trim_end_matches(',').trim_end_matches('"').to_string()))
        }).collect()
    });
    all.iter().find(|(k, _)| k == platform).map(|(_, v)| v.clone()).unwrap_or_default()
}

/// Its own icon, if one's been copied in: its file's name
pub fn icon_of(m: &Map<String, Value>) -> Option<&str> { m.get("icon").and_then(|v| v.as_str()).filter(|v| !v.is_empty()) }

// ---------- what the widget's sent ----------
pub fn widget_json(prefs: &Value) -> Value {
    let shown: Vec<Value> = list(prefs).iter().filter(|m| m.get("on").and_then(|v| v.as_bool()) != Some(false) && !s(m, "handle").trim().is_empty()).map(|m| {
        let p = platform(s(m, "platform"));
        let own = m.get("ownCol").and_then(|x| x.as_bool()) == Some(true) || p.0 == "other";
        let at = m.get("iconAt").and_then(|v| v.as_u64()).unwrap_or(0);
        json!({
            "platform": p.0, "name": name_of(m), "handle": s(m, "handle").trim(),
            "col1": if own && !s(m, "col1").is_empty() { s(m, "col1") } else { p.2 },
            "col2": if own && !s(m, "col2").is_empty() { s(m, "col2") } else { p.3 },
            "icon": icon_of(m).map(|_| format!("/image/social-{}?v={at}", s(m, "id"))).unwrap_or_default(),
        })
    }).collect();
    json!({
        "list": shown,
        "secs": prefs["socialSecs"].as_u64().unwrap_or(8),
        "style": prefs["socialStyle"].as_str().unwrap_or("card"),
        "align": prefs["socialAlign"].as_str().unwrap_or("left"),
        "all": prefs["socialAll"].as_bool() == Some(true),
        "rounded": prefs["socialRounded"].as_bool() != Some(false),
        "coloured": prefs["socialColoured"].as_bool() == Some(true),
        "motion": prefs["socialMotion"].as_str().unwrap_or("rise"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn added_moved_shown() {
        let (id, l) = added(&json!({}));
        assert_eq!(id, "s1");
        let p = json!({ "socials": l });
        let (id2, l2) = added(&p);
        let p = json!({ "socials": l2 });
        assert_eq!(list(&p)[1]["platform"], "kick");
        let p = json!({ "socials": moved(&p, &id2, -1) });
        assert_eq!(list(&p)[0]["id"], id2.as_str());
        // (only those with a handle, and switched on, shown)
        let p = json!({ "socials": with(&p, &id, |m| { m.insert("handle".into(), json!("@me")); }) });
        let w = widget_json(&p);
        assert_eq!(w["list"].as_array().unwrap().len(), 1);
        assert_eq!(w["list"][0]["col1"], "#9146ff");
        // (one of your own: your name and colours)
        let p = json!({ "socials": with(&p, &id, |m| { m.insert("platform".into(), json!("other")); m.insert("name".into(), json!("Fansly")); m.insert("col1".into(), json!("#123456")); }) });
        assert_eq!(widget_json(&p)["list"][0]["name"], "Fansly");
        assert_eq!(widget_json(&p)["list"][0]["col1"], "#123456");
    }
}
