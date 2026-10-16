// Discord, Last.fm and the OBS widget, for the native app (as in js/integrations.js and
// js/widget.js). Their settings are the web app's own (from its saved copy) until this app has its
// Settings: Discord activity, scrobbling, the widget's design and alerts all follow what's set there.

use crate::track::{Src, Track};
use omnipulse_core as core;
use serde_json::{json, Value};

/// A web app setting that's on unless switched off
pub fn on(prefs: &Value, key: &str) -> bool {
    prefs[key].as_bool() != Some(false)
}

/// A web app setting that's off unless switched on
pub fn set(prefs: &Value, key: &str) -> bool {
    prefs[key].as_bool() == Some(true)
}

fn open_label(t: &Track) -> &'static str {
    match t.src {
        Src::Spotify(_) => "Open in Spotify",
        Src::YouTube(_) => "Open on YouTube Music",
        _ => "Open on SoundCloud",
    }
}

fn big_art(t: &Track) -> String {
    t.art.replace("-t200x200", "-t500x500")
}

// ---------- Discord activity ----------
/// Shows what's playing on your Discord profile (or nothing, when paused), as the web app does,
/// with its buttons: Listen along (SoundCloud, at about where you are) and Open
/// join: the Listen Along room's link (crate::together), for the Listen along and Ask to Jam buttons
pub fn presence(rt: &tokio::runtime::Handle, prefs: &Value, t: Option<&Track>, pos_ms: u64, playing: bool, join: Option<String>) {
    if !set(prefs, "presence") {
        return;
    }
    let Some(t) = t.filter(|_| playing).cloned() else {
        rt.spawn(async { let _ = core::discord::presence_clear().await; });
        return;
    };
    let (show_time, show_art, show_link, show_listen) = (on(prefs, "presTime"), on(prefs, "presArt"), on(prefs, "presLink"), on(prefs, "presListen"));
    let show_ask = set(prefs, "presAsk");
    rt.spawn(async move {
        let now = core::now_secs() as i64;
        let start = now - (pos_ms / 1000) as i64;
        let end = (t.ms > 0).then(|| start + (t.ms / 1000) as i64);
        let link = t.web_link();
        let mut buttons = Vec::new();
        if let Some(join) = join.filter(|j| !j.is_empty()) {
            // through the relay: Listen along joins your queue, Ask to Jam asks you for a Jam (two at most)
            if show_listen { buttons.push(core::discord::PresenceButton { label: "Listen along".into(), url: join.clone() }); }
            if show_ask { buttons.push(core::discord::PresenceButton { label: "Ask to Jam".into(), url: join.replace("/join/", "/ask/") }); }
            if show_link && !link.is_empty() { buttons.push(core::discord::PresenceButton { label: open_label(&t).into(), url: link.clone() }); }
            buttons.truncate(2);
        } else if !link.is_empty() {
            if matches!(t.src, Src::SoundCloud(_)) {
                if show_listen {
                    let s = pos_ms / 1000;
                    buttons.push(core::discord::PresenceButton { label: "Listen along".into(), url: format!("{}#t={}:{:02}", link.split('#').next().unwrap_or(&link), s / 60, s % 60) });
                }
                if show_link {
                    buttons.push(core::discord::PresenceButton { label: open_label(&t).into(), url: link });
                }
            } else if show_listen || show_link {
                // their links can't say where to start: one button, named for what it does
                buttons.push(core::discord::PresenceButton { label: open_label(&t).into(), url: link });
            }
        }
        let image = (show_art && !matches!(t.src, Src::Local(_)) && !t.art.is_empty()).then(|| big_art(&t));
        let _ = core::discord::presence_set(
            t.title.clone(),
            if t.artist.is_empty() { "Unknown".into() } else { t.artist.clone() },
            show_time.then_some(start),
            if show_time { end } else { None },
            image,
            Some(buttons),
        )
        .await;
    });
}

// ---------- Last.fm ----------
fn lfm_wanted(prefs: &Value) -> bool {
    on(prefs, "lfmOn") && set(prefs, "scrobble") && core::lastfm::lastfm_status()["connected"].as_bool() == Some(true)
}

/// Its artist and title as Last.fm knows songs (as the web app's: "Artist - Title" uploads split, the
/// bracketed parts left out, Spotify's first artist)
fn lfm_params(t: &Track) -> Vec<(String, String)> {
    let (artist, title) = crate::versions::guess_names(t);
    let mut p = vec![
        ("artist".to_string(), if artist.is_empty() { "Unknown".into() } else { artist }),
        ("track".to_string(), if title.is_empty() { if t.title.is_empty() { "Unknown".into() } else { t.title.clone() } } else { title }),
    ];
    if t.ms > 0 {
        p.push(("duration".into(), (t.ms / 1000).to_string()));
    }
    p
}

pub fn lastfm_now_playing(rt: &tokio::runtime::Handle, prefs: &Value, t: &Track) {
    if !lfm_wanted(prefs) {
        return;
    }
    let params = lfm_params(t);
    rt.spawn(async move { let _ = core::lastfm::lastfm_call("track.updateNowPlaying".into(), params).await; });
}

/// Called once a play counts (30 s, or half the track), with when it started
pub fn lastfm_scrobble(rt: &tokio::runtime::Handle, prefs: &Value, t: &Track, started_secs: u64) {
    if !lfm_wanted(prefs) {
        return;
    }
    let mut params = lfm_params(t);
    params.push(("timestamp".into(), started_secs.to_string()));
    rt.spawn(async move { let _ = core::lastfm::lastfm_call("track.scrobble".into(), params).await; });
}

// ---------- the OBS widget ----------
pub fn widget_start(prefs: &Value) -> Option<String> {
    if !set(prefs, "widget") {
        return None;
    }
    // (bots on other devices: alerts from them too, with the alerts' key, as the web app's)
    let remote = set(prefs, "botRemote") && set(prefs, "alertsOn");
    core::widget::widget_start(prefs["widgetPort"].as_u64().map(|p| p as u16).unwrap_or(7878), remote, prefs["alertKey"].as_str().unwrap_or("")).ok()
}

/// The alerts' settings, for the widget (as alertsCfg in js/widget.js)
/// The collage's palettes (as js/widget.js COLLAGE_PALETTES): "alert" is each alert's own colours
const PALETTES: [(&str, &[&str]); 9] = [
    ("rainbow", &["#ff4d4d", "#ff9f1c", "#ffe74c", "#3ddc84", "#3a86ff", "#8338ec"]),
    ("neon", &["#ff00e5", "#00f0ff", "#39ff14", "#fff000", "#ff3131"]),
    ("sunset", &["#ff6b6b", "#ff9e57", "#ffd166", "#c94b7d", "#6a4c93"]),
    ("ocean", &["#0077b6", "#00b4d8", "#48cae4", "#90e0ef", "#023e8a"]),
    ("forest", &["#2d6a4f", "#40916c", "#74c69d", "#b7e4c7", "#95d5b2"]),
    ("candy", &["#ff8fab", "#ffc2d1", "#cdb4db", "#a2d2ff", "#bde0fe"]),
    ("fire", &["#d00000", "#e85d04", "#faa307", "#ffba08", "#9d0208"]),
    ("streamer", &["#ff0033", "#9146ff", "#53fc18", "#ff5500", "#1ed760"]),
    ("mono", &["#f5f5f5", "#bdbdbd", "#757575", "#424242"]),
];

/// An alert's collage colours: a preset, your own, or none (its own two)
fn alert_palette(e: &Value) -> Option<Value> {
    let id = e["cpal"].as_str().filter(|p| !p.is_empty() && *p != "alert")?;
    if id == "custom" {
        let own: Vec<Value> = e["cpalOwn"].as_array().cloned().unwrap_or_default().into_iter().take(6).collect();
        return Some(Value::Array(if own.is_empty() { vec![json!("#ff4d6d"), json!("#ffb703"), json!("#4cc9f0")] } else { own }));
    }
    PALETTES.iter().find(|(p, _)| *p == id).map(|(_, c)| json!(c))
}

pub fn alerts_cfg(prefs: &Value) -> Value {
    if !set(prefs, "alertsOn") {
        return json!({ "on": false });
    }
    // the built-in kinds and your own (custom alerts, by their trigger names)
    let custom: Vec<(String, String)> = prefs["alertCustom"].as_array().map(|a| a.iter().filter_map(|c| Some((c["id"].as_str()?.to_string(), c["name"].as_str().unwrap_or("").to_string()))).collect()).unwrap_or_default();
    let all: Vec<String> = core::widget::ALERT_KINDS.iter().map(|k| k.to_string()).chain(custom.iter().map(|c| c.0.clone())).collect();
    let mut order: Vec<String> = prefs["alertOrder"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).filter(|k| all.iter().any(|x| x == k)).map(String::from).collect()).unwrap_or_default();
    for k in &all {
        if !order.contains(k) {
            order.push(k.clone());
        }
    }
    // each one's own settings: words, colour, length, animation, icon or picture
    let mut kinds = serde_json::Map::new();
    for k in &all {
        let e = &prefs["alertEdit"][k];
        let mut c = serde_json::Map::new();
        for f in ["head", "main", "color", "anim", "icon"] {
            if let Some(v) = e[f].as_str().filter(|v| !v.is_empty()) {
                c.insert(f.into(), json!(v));
            }
        }
        // its second colour, picture's crop, collage icons and their edits, and whether it's musical
        if e["color"].as_str().is_some_and(|v| !v.is_empty()) {
            if let Some(v) = e["color2"].as_str().filter(|v| !v.is_empty()) { c.insert("color2".into(), json!(v)); }
        }
        for f in ["crop", "cicons", "cstyle"] {
            if !e[f].is_null() && e[f] != json!("") { c.insert(f.into(), e[f].clone()); }
        }
        if e["music"].as_bool() == Some(true) { c.insert("music".into(), json!(true)); }
        if let Some(p) = alert_palette(e) { c.insert("colors".into(), p); }
        if e["secs"].as_str() == Some("sound") {
            c.insert("secs".into(), json!("sound"));
        } else if let Some(s) = e["secs"].as_u64() {
            c.insert("secs".into(), json!(s));
        }
        if let Some(d) = e["sdelay"].as_u64() { c.insert("sdelay".into(), json!(d)); }
        if let Some(at) = prefs["alertImages"][k]["at"].as_u64() {
            c.insert("image".into(), json!(at.to_string()));
        }
        // its collage pictures (by place, so a gap stays a gap), and its sound's level and length
        if let Some(cp) = prefs["alertCollage"][k].as_array().filter(|a| a.iter().any(|x| !x.is_null())) {
            c.insert("cpics".into(), Value::Array(cp.iter().map(|p| json!(p["at"].as_u64().map(|a| a.to_string()).unwrap_or_default())).collect()));
        }
        let lv = &prefs["alertLevels"][k];
        if lv.is_object() && prefs["alertSounds"][k].is_object() {
            c.insert("gain".into(), json!(lv["auto"].as_f64().unwrap_or(0.0) + lv["adj"].as_f64().unwrap_or(0.0)));
            c.insert("sndMs".into(), lv["ms"].clone());
        }
        if let Some((_, name)) = custom.iter().find(|x| &x.0 == k) {
            c.insert("name".into(), json!(e["name"].as_str().filter(|v| !v.is_empty()).unwrap_or(name)));
        }
        if !c.is_empty() {
            kinds.insert(k.clone(), Value::Object(c));
        }
    }
    let off: Vec<&str> = prefs["alertOff"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).collect()).unwrap_or_default();
    let sounds: serde_json::Map<String, Value> = prefs["alertSounds"]
        .as_object()
        .map(|m| m.iter().filter_map(|(k, v)| v["at"].as_u64().map(|at| (k.clone(), json!(at.to_string())))).collect())
        .unwrap_or_default();
    json!({
        "on": true,
        "order": order.into_iter().filter(|k| !off.contains(&k.as_str())).collect::<Vec<_>>(),
        "custom": custom.iter().map(|c| c.0.clone()).collect::<Vec<_>>(),
        "kinds": kinds,
        "anim": prefs["alertAnim"].as_str().unwrap_or("slide"),
        "transition": prefs["alertTrans"].as_str().unwrap_or("seamless"),
        "collage": { "size": prefs["alertCSize"].as_str().unwrap_or("medium"), "dir": prefs["alertCDir"].as_str().unwrap_or("diag"), "angle": prefs["alertCAngle"].as_f64().unwrap_or(0.0) },
        "accent": if set(prefs, "alertOwnCol") { prefs["alertCol1"].as_str().unwrap_or("#8e9bff") } else { "" },
        "accent2": if set(prefs, "alertOwnCol") { prefs["alertCol2"].as_str().or(prefs["alertCol1"].as_str()).unwrap_or("#b38cff") } else { "" },
        "dir": prefs["alertDir"].as_str().unwrap_or("115deg"),
        "icon_scale": prefs["alertIcon"].as_f64().unwrap_or(1.0).clamp(0.6, 2.0),
        "secs": prefs["alertSecs"].as_u64().unwrap_or(5),
        "volume": prefs["alertVol"].as_f64().unwrap_or(0.8),
        "sounds": sounds,
        "sb": { "on": on(prefs, "sbOn"), "host": prefs["sbHost"].as_str().unwrap_or("127.0.0.1:8080") },
    })
}

// Liking a song someone requested: the card says so for a few seconds, with a word for them (each
// line once before any repeats), as the web app's widgetLiked
const LIKE_LINES: [&str; 12] = [
    "Good choice, {user}!", "{user} has taste.", "Certified banger, {user}.", "Nice one, {user}!",
    "{user} gets it.", "This one's a keeper, {user}.", "Top-tier pick, {user}.", "{user} cooked with this one.",
    "Straight into the likes, thanks {user}!", "Great call, {user}.", "{user} knows what's up.", "Taste confirmed, {user}.",
];
const LIKE_NOTE_MS: u128 = 8000;
static LIKE_NOTE: std::sync::Mutex<Option<(String, std::time::Instant)>> = std::sync::Mutex::new(None);
static LIKE_ORDER: std::sync::Mutex<Vec<usize>> = std::sync::Mutex::new(Vec::new());

/// A requested song liked: an alert for whoever asked for it (Liked Requests), or with alerts off (or
/// that one switched off), the card's note for them (for LIKE_NOTE_MS)
pub fn like_banter(prefs: &Value, by: &str) {
    let mut order = LIKE_ORDER.lock().unwrap();
    if order.is_empty() {
        let mut all: Vec<usize> = (0..LIKE_LINES.len()).collect();
        crate::queue::shuffle_in_place(&mut all);
        *order = all;
    }
    let line = LIKE_LINES[order.pop().unwrap_or(0)].replace("{user}", by);
    let alert_off = prefs["alertOff"].as_array().is_some_and(|l| l.iter().any(|x| x == "liked"));
    if set(prefs, "widget") && set(prefs, "alertsOn") && !alert_off {
        let mut f = serde_json::Map::new();
        f.insert("type".into(), json!("liked"));
        f.insert("user".into(), json!(by));
        f.insert("message".into(), json!(line));
        if core::widget::alert_push(&f).is_ok() { return; }
    }
    *LIKE_NOTE.lock().unwrap() = Some((format!("\u{2665} Liked \u{b7} {line}"), std::time::Instant::now()));
}

fn like_note() -> String {
    LIKE_NOTE.lock().unwrap().as_ref().filter(|(_, at)| at.elapsed().as_millis() < LIKE_NOTE_MS).map(|(s, _)| s.clone()).unwrap_or_default()
}

/// What the widget shows: the track, where it's up to, its colours, and the look set in Settings
/// by: who asked for it in chat ("" for none)
/// The widget's Hide This Track: the card hidden while this track (its key) plays
pub static WIDGET_HIDDEN: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

pub fn widget_update(prefs: &Value, t: Option<&Track>, by: &str, pos_ms: u64, dur_ms: u64, playing: bool, accent: (&str, &str)) {
    if !set(prefs, "widget") {
        return;
    }
    // (hidden until the next track)
    let t = t.filter(|t| WIDGET_HIDDEN.lock().map(|h| h.is_empty() || *h != t.key).unwrap_or(true));
    let look = json!({
        "accent": accent.0, "accent2": accent.1,
        "design": prefs["widgetDesign"].as_str().unwrap_or("card"),
        "rounded": on(prefs, "widgetRounded"),
        "align": if prefs["widgetAlign"].as_str() == Some("right") { "right" } else { "left" },
        "text_scale": prefs["widgetText"].as_f64().unwrap_or(1.0),
        "alerts_cfg": alerts_cfg(prefs),
    });
    let mut state = match t {
        None => json!({ "playing": false }),
        Some(t) => json!({
            "playing": playing,
            "title": t.title, "artist": t.artist, "artwork": big_art(t), "url": t.web_link(), "platform": t.plat(),
            "requested_by": if on(prefs, "wRequester") { by } else { "" },
            "like_note": if by.is_empty() { String::new() } else { like_note() },
            "duration_ms": if dur_ms > 0 { dur_ms } else { t.ms }, "position_ms": pos_ms,
            "updated_at": core::now_secs() * 1000 + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.subsec_millis() as u64).unwrap_or(0)),
            "anim": {
                "fade": on(prefs, "wFade"), "scroll": on(prefs, "wScroll"), "spin": on(prefs, "wSpin"),
                "color": prefs["wColor"].as_f64().unwrap_or(1.2),
            },
        }),
    };
    if let (Some(o), Some(l)) = (state.as_object_mut(), look.as_object()) {
        o.extend(l.clone());
    }
    core::widget::widget_update(state);
}

/// Two colours from a cover (its upper-left and lower-right halves), bright enough to tint the
/// widget: the same idea as the web app's accent colours
/// The cover's two colours, as the web app's artPalette: the cover seen at 32 x 32, its vivid pixels
/// sorted into twelve hues, the strongest hue, and a clearly different one (two hues away, at least
/// 15% as strong) if there is one, else "" (apply_look then uses a darker shade of the first). Tuned
/// to the theme in apply_look (look::tune_art)
pub fn accents(px: &slint::SharedPixelBuffer<slint::Rgba8Pixel>) -> (String, String) {
    let (w, h) = (px.width() as usize, px.height() as usize);
    let data = px.as_slice();
    if w == 0 || h == 0 {
        return ("#8e9bff".into(), String::new());
    }
    // [r, g, b, weight] for each twelfth of the hue circle
    let mut buckets = [[0f32; 4]; 12];
    for y in 0..32 {
        for x in 0..32 {
            let p = data[(y * h / 32) * w + x * w / 32];
            let (r, g, b) = (p.r as f32 / 255.0, p.g as f32 / 255.0, p.b as f32 / 255.0);
            let (max, min) = (r.max(g).max(b), r.min(g).min(b));
            let l = (max + min) / 2.0;
            let d = max - min;
            if d == 0.0 {
                continue;
            }
            let s = if l > 0.5 { d / (2.0 - max - min) } else { d / (max + min) };
            let sixths = if max == r { (g - b) / d + if g < b { 6.0 } else { 0.0 } } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
            let hue = sixths / 6.0;
            // vivid and mid-bright counts; greys, near-blacks and near-whites don't
            let wt = s * (1.0 - (l - 0.5).abs() * 1.8).max(0.0);
            if wt < 0.05 {
                continue;
            }
            let k = ((hue * 12.0) as usize) % 12;
            buckets[k][0] += p.r as f32 * wt;
            buckets[k][1] += p.g as f32 * wt;
            buckets[k][2] += p.b as f32 * wt;
            buckets[k][3] += wt;
        }
    }
    let mut order: Vec<usize> = (0..12).filter(|&k| buckets[k][3] > 0.0).collect();
    order.sort_by(|a, b| buckets[*b][3].total_cmp(&buckets[*a][3]));
    let Some(&first) = order.first() else { return ("#8e9bff".into(), String::new()) };
    let apart = |a: usize, b: usize| (a as i32 - b as i32).abs().min(12 - (a as i32 - b as i32).abs());
    let second = order.iter().copied().find(|&k| apart(k, first) >= 2 && buckets[k][3] >= buckets[first][3] * 0.15);
    let hex = |k: usize| {
        let s = buckets[k];
        format!("#{:02x}{:02x}{:02x}", (s[0] / s[3]).round() as u8, (s[1] / s[3]).round() as u8, (s[2] / s[3]).round() as u8)
    };
    (hex(first), second.map(hex).unwrap_or_default())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn alerts_follow_the_web_apps_settings() {
        let prefs = json!({ "alertsOn": true, "alertOrder": ["bits", "nonsense"], "alertOff": ["ban"], "alertSounds": { "follow": { "name": "a.mp3", "at": 5 } } });
        let c = alerts_cfg(&prefs);
        let order: Vec<&str> = c["order"].as_array().unwrap().iter().map(|x| x.as_str().unwrap()).collect();
        assert_eq!(order[0], "bits");
        assert!(!order.contains(&"ban") && !order.contains(&"nonsense") && order.contains(&"watchstreak"));
        assert_eq!(c["sounds"]["follow"], "5");
        assert_eq!(alerts_cfg(&json!({}))["on"], false);
        let c = alerts_cfg(&json!({ "alertsOn": true, "alertCustom": [{ "id": "hydrate", "name": "Hydrate" }], "alertEdit": { "ban": { "color": "#ff0000", "secs": 8 } }, "alertImages": { "ban": { "at": 7 } } }));
        assert_eq!(c["custom"][0], "hydrate");
        assert_eq!((c["kinds"]["ban"]["color"].as_str(), c["kinds"]["ban"]["secs"].as_u64(), c["kinds"]["ban"]["image"].as_str()), (Some("#ff0000"), Some(8), Some("7")));
        assert_eq!(c["kinds"]["hydrate"]["name"], "Hydrate");
        assert!(c["order"].as_array().unwrap().iter().any(|k| k == "hydrate"));
    }

    #[test]
    fn a_red_cover_gives_red_accents() {
        let mut px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(20, 20);
        for p in px.make_mut_slice() {
            *p = slint::Rgba8Pixel { r: 120, g: 10, b: 10, a: 255 };
        }
        let (a, b) = accents(&px);
        assert_eq!((a.as_str(), b.as_str()), ("#780a0a", ""));
        // half red, half blue: the two, the bigger first
        let mut px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(20, 20);
        for (i, p) in px.make_mut_slice().iter_mut().enumerate() {
            *p = if i % 20 < 12 { slint::Rgba8Pixel { r: 200, g: 30, b: 30, a: 255 } } else { slint::Rgba8Pixel { r: 30, g: 30, b: 200, a: 255 } };
        }
        assert_eq!(accents(&px), ("#c81e1e".to_string(), "#1e1ec8".to_string()));
        // grey: nothing to take
        let mut px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(8, 8);
        for p in px.make_mut_slice() { *p = slint::Rgba8Pixel { r: 90, g: 90, b: 90, a: 255 }; }
        assert_eq!(accents(&px).1, "");
    }
}

#[cfg(test)]
mod alert_tests {
    use super::*;

    #[test]
    fn the_widget_hears_each_alerts_own_settings() {
        let prefs = json!({
            "alertsOn": true, "alertTrans": "collage", "alertCAngle": 30, "alertCDir": "ltr",
            "alertEdit": { "follow": { "color": "#2ecc71", "color2": "#3b82f6", "cpal": "sunset", "cicons": ["rocket"], "cstyle": { "icon:rocket": { "bg": "#ff8800" } }, "music": true, "secs": "sound", "sdelay": 500, "crop": { "x": 70 } } },
            "alertSounds": { "follow": { "name": "ding.wav", "at": 1 } },
            "alertLevels": { "follow": { "auto": -1.0, "adj": 2.0, "ms": 1200 } },
            "alertCollage": { "follow": [{ "name": "a.png", "at": 5 }, null] },
        });
        let c = alerts_cfg(&prefs);
        let f = &c["kinds"]["follow"];
        assert_eq!(f["color2"], "#3b82f6");
        assert_eq!(f["colors"][0], "#ff6b6b");
        assert_eq!(f["cicons"][0], "rocket");
        assert_eq!(f["cstyle"]["icon:rocket"]["bg"], "#ff8800");
        assert_eq!((f["music"].clone(), f["secs"].clone(), f["sdelay"].clone()), (json!(true), json!("sound"), json!(500)));
        assert_eq!((f["gain"].as_f64(), f["sndMs"].as_u64()), (Some(1.0), Some(1200)));
        assert_eq!(f["cpics"], json!(["5", ""]));
        assert_eq!(c["collage"]["angle"], 30.0);
        assert_eq!(c["collage"]["dir"], "ltr");
    }
}

#[cfg(test)]
mod banter_tests {
    #[test]
    fn each_line_once_before_any_repeats() {
        let mut seen = std::collections::HashSet::new();
        for _ in 0..super::LIKE_LINES.len() {
            super::like_banter(&serde_json::json!({}), "amy");
            seen.insert(super::like_note());
        }
        assert_eq!(seen.len(), super::LIKE_LINES.len());
        assert!(seen.iter().all(|s| s.contains("amy") && s.starts_with('\u{2665}')));
    }
}
