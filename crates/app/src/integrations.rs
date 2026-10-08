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
pub fn presence(rt: &tokio::runtime::Handle, prefs: &Value, t: Option<&Track>, pos_ms: u64, playing: bool) {
    if !set(prefs, "presence") {
        return;
    }
    let Some(t) = t.filter(|_| playing).cloned() else {
        rt.spawn(async { let _ = core::discord::presence_clear().await; });
        return;
    };
    let (show_time, show_art, show_link, show_listen) = (on(prefs, "presTime"), on(prefs, "presArt"), on(prefs, "presLink"), on(prefs, "presListen"));
    rt.spawn(async move {
        let now = core::now_secs() as i64;
        let start = now - (pos_ms / 1000) as i64;
        let end = (t.ms > 0).then(|| start + (t.ms / 1000) as i64);
        let link = t.web_link();
        let mut buttons = Vec::new();
        if !link.is_empty() {
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

fn lfm_params(t: &Track) -> Vec<(String, String)> {
    let mut p = vec![
        ("artist".to_string(), if t.artist.is_empty() { "Unknown".into() } else { t.artist.split(", ").next().unwrap_or(&t.artist).to_string() }),
        ("track".to_string(), t.title.clone()),
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
    core::widget::widget_start(prefs["widgetPort"].as_u64().map(|p| p as u16).unwrap_or(7878)).ok()
}

/// The alerts' settings, for the widget (as alertsCfg in js/widget.js)
pub fn alerts_cfg(prefs: &Value) -> Value {
    if !set(prefs, "alertsOn") {
        return json!({ "on": false });
    }
    let all = core::widget::ALERT_KINDS;
    let mut order: Vec<String> = prefs["alertOrder"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).filter(|k| all.contains(k)).map(String::from).collect()).unwrap_or_default();
    for k in all {
        if !order.iter().any(|o| o == k) {
            order.push(k.to_string());
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
        "anim": prefs["alertAnim"].as_str().unwrap_or("slide"),
        "secs": prefs["alertSecs"].as_u64().unwrap_or(5),
        "volume": prefs["alertVol"].as_f64().unwrap_or(0.8),
        "sounds": sounds,
        "sb": { "on": on(prefs, "sbOn"), "host": prefs["sbHost"].as_str().unwrap_or("127.0.0.1:8080") },
    })
}

/// What the widget shows: the track, where it's up to, its colours, and the look set in Settings
pub fn widget_update(prefs: &Value, t: Option<&Track>, pos_ms: u64, dur_ms: u64, playing: bool, accent: (&str, &str)) {
    if !set(prefs, "widget") {
        return;
    }
    let look = json!({
        "accent": accent.0, "accent2": accent.1,
        "design": prefs["widgetDesign"].as_str().unwrap_or("card"),
        "rounded": on(prefs, "widgetRounded"),
        "text_scale": prefs["widgetText"].as_f64().unwrap_or(1.0),
        "alerts_cfg": alerts_cfg(prefs),
    });
    let mut state = match t {
        None => json!({ "playing": false }),
        Some(t) => json!({
            "playing": playing,
            "title": t.title, "artist": t.artist, "artwork": big_art(t), "url": t.web_link(), "platform": t.plat(),
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
pub fn accents(px: &slint::SharedPixelBuffer<slint::Rgba8Pixel>) -> (String, String) {
    let (w, h) = (px.width() as usize, px.height() as usize);
    let data = px.as_slice();
    let mut sums = [[0f32; 4]; 2];
    let step = (w * h / 4000).max(1);
    for (i, p) in data.iter().enumerate().step_by(step) {
        let (x, y) = (i % w, i / w);
        let half = usize::from(x * h + y * w > w * h);
        let (r, g, b) = (p.r as f32, p.g as f32, p.b as f32);
        let max = r.max(g).max(b);
        let sat = if max > 0.0 { (max - r.min(g).min(b)) / max } else { 0.0 };
        // vivid, not too dark, pixels count most
        let weight = 0.15 + sat * (max / 255.0);
        sums[half][0] += r * weight;
        sums[half][1] += g * weight;
        sums[half][2] += b * weight;
        sums[half][3] += weight;
    }
    let hex = |s: [f32; 4]| {
        if s[3] <= 0.0 {
            return "#8e9bff".to_string();
        }
        let (r, g, b) = (s[0] / s[3], s[1] / s[3], s[2] / s[3]);
        // lifted to a usable brightness, keeping the hue
        let m = r.max(g).max(b).max(1.0);
        let k = (200.0 / m).clamp(1.0, 3.0);
        format!("#{:02x}{:02x}{:02x}", (r * k).min(255.0) as u8, (g * k).min(255.0) as u8, (b * k).min(255.0) as u8)
    };
    (hex(sums[0]), hex(sums[1]))
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
    }

    #[test]
    fn a_red_cover_gives_red_accents() {
        let mut px = slint::SharedPixelBuffer::<slint::Rgba8Pixel>::new(20, 20);
        for p in px.make_mut_slice() {
            *p = slint::Rgba8Pixel { r: 120, g: 10, b: 10, a: 255 };
        }
        let (a, b) = accents(&px);
        assert!(a.starts_with("#c8") && a == b, "{a} {b}");
    }
}
