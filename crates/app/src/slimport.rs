//! Streamlabs' Alert Box, imported: its settings as its own widget page reads them (api/v5/widget/config, with
//! the token from the Alert Box's link), each alert type's picture, sound, message, duration and colours
//! mapped onto OmniPulse's alerts.

use serde_json::Value;

/// Streamlabs' alert types (their settings' prefixes), as ours; a raid takes the host's if it has none of its own
pub const MAP: [(&str, &str); 6] = [("follow", "follow"), ("sub", "sub"), ("bits", "bits"), ("donation", "donation"), ("raid", "raid"), ("kick_kicks", "kicks")];

/// The token in an Alert Box link (streamlabs.com/alert-box/v3/<token>)
pub fn token(link: &str) -> Option<String> {
    let rest = link.split("alert-box/v3/").nth(1)?;
    let t: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-').collect();
    (t.len() >= 8).then_some(t)
}

/// Where the settings are read from
pub fn config_url(token: &str) -> String {
    format!("https://streamlabs.com/api/v5/widget/config?widget=alert_box&token={token}")
}

/// One alert type, as imported
#[derive(Debug, Default, PartialEq)]
pub struct One {
    pub kind: &'static str,
    pub on: bool,
    pub image: Option<String>,
    pub sound: Option<String>,
    /// its message, with our placeholders
    pub main: Option<String>,
    pub secs: Option<u64>,
    pub colours: Option<(String, String)>,
}

/// Its message with ours: {name} and {gifter} are {user}; {amount} and {count} are {amount}; the rest gone
pub fn template(t: &str) -> String {
    let mut out = String::new();
    let mut rest = t;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let Some(b) = rest[a..].find('}') else { out.push_str(&rest[a..]); rest = ""; break };
        let word = &rest[a + 1..a + b];
        out.push_str(match word { "name" | "gifter" => "{user}", "amount" | "count" | "months" => "{amount}", _ => "" });
        rest = &rest[a + b + 1..];
    }
    out.push_str(rest);
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A picture's or sound's address, if it's one of Streamlabs' own (nothing else is fetched)
fn media(v: &Value) -> Option<String> {
    let u = v.as_str()?.trim();
    // (Streamlabs' own gallery's: given from its site's root)
    let full = u.starts_with('/').then(|| format!("https://streamlabs.com{u}"));
    let u = full.as_deref().unwrap_or(u);
    let host = u.strip_prefix("https://")?.split('/').next()?;
    (host.ends_with(".streamlabs.com") || host == "streamlabs.com" || host.ends_with(".twitchalerts.com")).then(|| u.to_string())
}

fn hex(v: &Value) -> Option<String> {
    let s = v.as_str()?.trim();
    (s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_lowercase())
}

/// Each alert type in the settings (`settings` of what config_url gives)
pub fn read(s: &Value) -> Vec<One> {
    MAP.iter().filter_map(|&(p, kind)| {
        let g = |k: &str| &s[format!("{p}_{k}")];
        if g("enabled").is_null() && g("message_template").is_null() { return None; }
        let host = |k: &str| if p == "raid" { media(&s[format!("host_{k}")]) } else { None };
        let main = g("message_template").as_str().map(template).filter(|t| !t.is_empty());
        let secs = g("alert_duration").as_f64().filter(|ms| *ms > 0.0).map(|ms| ((ms / 1000.0).round() as u64).clamp(1, 60));
        // (its highlight colour (the name's) first, as ours is the accent, and its words' second)
        let colours = hex(g("font_color2")).zip(hex(g("font_color")));
        Some(One { kind, on: g("enabled").as_bool() != Some(false), image: media(g("image_href")).or_else(|| host("image_href")), sound: media(g("sound_href")).or_else(|| host("sound_href")), main, secs, colours })
    }).collect()
}

/// A file name's extension for a download (from its address), if it's one alerts take
pub fn ext_of(url: &str, image: bool) -> Option<String> {
    let path = url.split(['?', '#']).next().unwrap_or("");
    let ext = path.rsplit('.').next().unwrap_or("").to_lowercase();
    let ok: &[&str] = if image { &omnipulse_core::widget::IMAGE_EXTS } else { &["mp3", "wav", "ogg", "m4a", "aac", "flac"] };
    (ok.contains(&ext.as_str()) || (image && omnipulse_core::widget::VIDEO_EXTS.contains(&ext.as_str()))).then_some(ext)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_token_comes_from_the_link() {
        assert_eq!(token("https://streamlabs.com/alert-box/v3/ABCdef1234567890").as_deref(), Some("ABCdef1234567890"));
        assert_eq!(token("https://streamlabs.com/alert-box/v3/ABCdef1234567890?x=1").as_deref(), Some("ABCdef1234567890"));
        assert_eq!(token("https://streamlabs.com/widgets/chat-box/v1/abc"), None);
    }

    #[test]
    fn messages_take_our_placeholders() {
        assert_eq!(template("{name} is now following!"), "{user} is now following!");
        assert_eq!(template("{name} donated {amount}!"), "{user} donated {amount}!");
        assert_eq!(template("{gifter} gifted {count} subs to {streamer}"), "{user} gifted {amount} subs to");
    }

    #[test]
    fn each_alert_is_read_and_a_raid_falls_back_to_the_host() {
        let s = json!({
            "follow_enabled": true, "follow_message_template": "{name} is now following!", "follow_image_href": "https://cdn.streamlabs.com/a.gif",
            "follow_sound_href": "https://uploads.twitchalerts.com/b.mp3", "follow_alert_duration": 9000, "follow_font_color": "#FFFFFF", "follow_font_color2": "#32C3A2",
            "bits_enabled": false, "bits_message_template": "{name} cheered {amount}!", "bits_image_href": "https://example.com/x.gif",
            "raid_enabled": true, "raid_message_template": "{name} is raiding with {count}!", "host_image_href": "https://cdn.streamlabs.com/h.png",
        });
        let l = read(&s);
        assert_eq!(l.len(), 3);
        assert_eq!(l[0], One { kind: "follow", on: true, image: Some("https://cdn.streamlabs.com/a.gif".into()), sound: Some("https://uploads.twitchalerts.com/b.mp3".into()), main: Some("{user} is now following!".into()), secs: Some(9), colours: Some(("#32c3a2".into(), "#ffffff".into())) });
        // (a picture from anywhere else isn't fetched)
        assert_eq!((l[1].kind, l[1].on, l[1].image.clone()), ("bits", false, None));
        assert_eq!((l[2].kind, l[2].image.as_deref()), ("raid", Some("https://cdn.streamlabs.com/h.png")));
        assert_eq!(ext_of("https://cdn.streamlabs.com/a.GIF?v=2", true).as_deref(), Some("gif"));
        // (Streamlabs' animations are videos: taken as pictures)
        assert_eq!(ext_of("https://cdn.streamlabs.com/a.webm", true).as_deref(), Some("webm"));
        assert_eq!(ext_of("https://cdn.streamlabs.com/a.webm", false), None);
        assert_eq!(media(&json!("/sounds/gallery/default.ogg")).as_deref(), Some("https://streamlabs.com/sounds/gallery/default.ogg"));
    }
}
