// The Chat Dock's, the Activity Feed's and the Multichat Overlay's settings, on pages of their own (Settings >
// Streaming > Suite, each one's gear). Kept in prefs.dockSet by page ("chat", "activity", "multichat"), handed
// to the widget server for the pages to follow, and taken back when they're changed on the page itself (its
// gear in OBS), so the two never differ.

use serde_json::{json, Map, Value};

/// A panel's page: dockchat, dockfeed, multichat
pub fn page_of(panel: &str) -> Option<&'static str> {
    Some(match panel { "dockchat" => "chat", "dockfeed" => "activity", "multichat" => "multichat", _ => return None })
}

pub fn title(panel: &str) -> &'static str {
    match panel { "dockchat" => "Chat Dock", "dockfeed" => "Activity Feed", _ => "Multichat Overlay" }
}

/// Each page's settings as the page starts them
fn defaults(page: &str) -> Map<String, Value> {
    let v = match page {
        "chat" => json!({ "twitch": true, "kick": true, "time": false, "badges": true, "hover": false, "confirm": true, "secs": "600", "size": "13", "to": "all", "bttv": true, "ffz": true, "stv": true,
            "modPile": true, "lkPile": true, "pileMin": "3", "modLock": true, "modModes": true, "modShield": true, "modAb": true, "lkShield": true, "lkFollow": true, "lkSlow": true, "lkEmote": false, "lkSub": false }),
        "activity" => json!({ "hidden": ["firstwords"], "newest": true, "compact": false, "platform": true, "msg": true, "size": "13", "from": "all" }),
        _ => json!({ "from": "all", "cmds": false, "bot": true, "badges": true, "marks": true, "style": "plain", "size": 20, "bg": "0", "shadow": true, "dir": "bottom", "fade": 0, "max": 30, "bttv": true, "ffz": true, "stv": true, "name": "beside", "anim": "rise", "events": false, "pronouns": false, "first": true }),
    };
    v.as_object().cloned().unwrap_or_default()
}

/// The feed's kinds of event, as its page lists them: (id, name, group); read from the page itself, so a kind
/// added there shows here too
pub fn kinds() -> Vec<(String, String, String)> {
    static K: std::sync::OnceLock<Vec<(String, String, String)>> = std::sync::OnceLock::new();
    K.get_or_init(|| {
        let page = include_str!("../../core/widget/dock-activity.html");
        let rx = regex::Regex::new(r##"^\s*\["([a-z]+)", "([^"]+)", "[^"]*", "#[0-9a-fA-F]+", "([A-Za-z]+)""##).unwrap();
        page.lines().filter_map(|l| rx.captures(l).map(|c| (c[1].to_string(), c[2].to_string(), c[3].to_string()))).collect()
    }).clone()
}

/// What's kept for a page: its defaults, with what's been changed over them
pub fn kept(prefs: &Value, page: &str) -> Map<String, Value> {
    let mut m = defaults(page);
    if let Some(o) = prefs["dockSet"][page].as_object() { for (k, v) in o { if m.contains_key(k) { m.insert(k.clone(), v.clone()); } } }
    m
}

/// Its settings page's values: the feed's hidden kinds as a switch for each ("show:follow")
pub fn values(prefs: &Value, panel: &str) -> Map<String, Value> {
    let Some(page) = page_of(panel) else { return Map::new() };
    let mut m = kept(prefs, page);
    if page == "activity" {
        let hidden: Vec<String> = m.remove("hidden").and_then(|h| h.as_array().cloned()).unwrap_or_default().iter().filter_map(|x| x.as_str().map(String::from)).collect();
        for (k, _, _) in kinds() { m.insert(format!("show:{k}"), json!(!hidden.contains(&k))); }
    }
    m
}

/// The page's values, as kept: back to the page's own keys (the switches as the feed's hidden list)
pub fn saved(prefs: &Value, panel: &str, vals: &Map<String, Value>) -> Value {
    let mut all = prefs["dockSet"].as_object().cloned().unwrap_or_default();
    let Some(page) = page_of(panel) else { return Value::Object(all) };
    let mut m = defaults(page);
    for (k, v) in vals { if m.contains_key(k) { m.insert(k.clone(), v.clone()); } }
    if page == "activity" {
        let hidden: Vec<String> = kinds().into_iter().filter(|(k, _, _)| vals.get(&format!("show:{k}")).and_then(|v| v.as_bool()) == Some(false)).map(|(k, _, _)| k).collect();
        m.insert("hidden".into(), json!(hidden));
    }
    all.insert(page.into(), Value::Object(m));
    Value::Object(all)
}

/// A change made on the page itself (its gear): kept, as the page sent it (only the keys it has)
pub fn from_page(prefs: &Value, page: &str, opt: &Value) -> Value {
    let mut all = prefs["dockSet"].as_object().cloned().unwrap_or_default();
    let base = page.trim_end_matches(|c: char| c.is_ascii_digit());
    if !matches!(base, "chat" | "activity" | "multichat") { return Value::Object(all); }
    let mut m = defaults(base);
    if let Some(o) = opt.as_object() { for (k, v) in o { if m.contains_key(k) { m.insert(k.clone(), v.clone()); } } }
    all.insert(page.into(), Value::Object(m));
    Value::Object(all)
}

fn tog(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "toggle" }) }
fn note(t: &str) -> Value { json!({ "key": "", "label": t, "note": "", "type": "note" }) }
fn info(label: &str, value: &str, n: &str) -> Value { json!({ "key": "", "label": label, "note": n, "type": "info", "value": value, "copy": value }) }
fn sel(key: &str, label: &str, n: &str, opts: &[(Value, &str)]) -> Value {
    json!({ "key": key, "label": label, "note": n, "type": "select", "options": opts.iter().map(|o| o.0.clone()).collect::<Vec<_>>(), "labels": opts.iter().map(|o| o.1).collect::<Vec<_>>() })
}

/// Its page: what you'd change most first, what it shows, how it looks, then where it goes in OBS
pub fn fields(panel: &str, vals: &Map<String, Value>, port: u64) -> Vec<Value> {
    let base = format!("http://127.0.0.1:{port}");
    match panel {
        "dockchat" => vec![
            note("What It Shows"),
            tog("twitch", "Twitch Chat", ""),
            tog("kick", "Kick Chat", ""),
            tog("badges", "Badges", "Subscriber, moderator and VIP badges next to names."),
            tog("time", "Times", "When each message was sent."),
            sel("size", "Text Size", "", &[(json!("12"), "Small"), (json!("13"), "Normal"), (json!("15"), "Large"), (json!("17"), "Larger")]),
            note("Emotes"),
            tog("stv", "7TV Emotes", "Your channel's and 7TV's global ones, on Twitch and Kick."),
            tog("bttv", "BetterTTV Emotes", "Your channel's and BetterTTV's global ones."),
            tog("ffz", "FrankerFaceZ Emotes", "Your channel's and FrankerFaceZ's global ones."),
            note("Moderating"),
            sel("secs", "Time-Out Length", "How long the time-out button times someone out for.", &[(json!("60"), "1 minute"), (json!("300"), "5 minutes"), (json!("600"), "10 minutes"), (json!("3600"), "1 hour"), (json!("86400"), "1 day")]),
            tog("confirm", "Ask Before Banning", "A ban asks you to click again first, so a slip doesn't ban anyone."),
            note("Moderation (the dock's shield)"),
            tog("modPile", "Pile-Up", "The switch is in the panel. While it's on, the same message from several accounts folds into one card above chat, to ban them all at once. It's off until you switch it on, as a friendly raid's message repeats too."),
            tog("modLock", "Raid Lockdown", "One button for a hate raid: it turns on what you choose below, and turns them off again."),
            tog("modModes", "Chat Modes", "Slow, followers only, subscribers only, emotes only and unique chat, each a click. Twitch's."),
            tog("modShield", "Shield Mode and Clear Chat", "Twitch's Shield Mode, and clearing chat (asked twice)."),
            tog("modAb", "Auto-Ban", "Your banned words and phrases: add one in a moment, or take one off. Each message's shield button puts its words in."),
            note("Raid Lockdown Turns On"),
            tog("lkPile", "Pile-Up", ""),
            tog("lkShield", "Shield Mode", ""),
            tog("lkFollow", "Followers Only", "For as long as its own choice in the dock says (10 minutes at first)."),
            tog("lkSlow", "Slow", ""),
            tog("lkEmote", "Emotes Only", ""),
            tog("lkSub", "Subscribers Only", ""),
            tog("hover", "Buttons Only on Hover", "Delete, time-out and ban show when you point at a message. Off, they're always there."),
            note("Chatting Back"),
            sel("to", "Your Messages Go To", "Where what you type in the dock is sent.", &[(json!("all"), "Twitch and Kick"), (json!("twitch"), "Twitch"), (json!("kick"), "Kick")]),
            note("In OBS"),
            info("Address", &format!("{base}/dock/chat"), "In OBS, open Docks > Custom Browser Docks, give it a name and paste this address. Changes here show there within a few seconds."),
            info("Moderation Dock", &format!("{base}/dock/mod"), "Moderation on its own: Pile-Up, Raid Lockdown, chat modes, Shield Mode and Auto-Ban, as a dock beside your chat. These settings are its too."),
        ],
        "dockfeed" => {
            let mut f = vec![
                note("What It Shows"),
                sel("from", "Platforms", "", &[(json!("all"), "Twitch and Kick"), (json!("twitch"), "Twitch"), (json!("kick"), "Kick")]),
                tog("msg", "Their Message", "What they wrote with a sub, cheer or reward."),
                tog("platform", "Platform", "A small Twitch or Kick mark on each."),
                note("How It Looks"),
                tog("newest", "Newest at the Top", ""),
                tog("compact", "Compact", "Smaller rows, so more fit."),
                sel("size", "Text Size", "", &[(json!("12"), "Small"), (json!("13"), "Normal"), (json!("15"), "Large")]),
            ];
            // (the kinds of event: a switch for each, in their groups)
            let all = kinds();
            let mut groups: Vec<String> = Vec::new();
            for (_, _, g) in &all { if !groups.contains(g) { groups.push(g.clone()); } }
            for g in groups {
                f.push(note(&format!("Events: {g}")));
                for (k, name, _) in all.iter().filter(|x| &x.2 == &g) { f.push(tog(&format!("show:{k}"), name, "")); }
            }
            f.push(note("In OBS"));
            f.push(info("Address", &format!("{base}/dock/activity"), "In OBS, open Docks > Custom Browser Docks, give it a name and paste this address. Changes here show there within a few seconds."));
            let _ = vals;
            f
        }
        _ => {
            let plain = vals.get("style").and_then(|v| v.as_str()).unwrap_or("plain") == "plain";
            let mut f = vec![
                note("In OBS"),
                info("Address", &format!("{base}/chat"), "Add it as a Browser source. Changes here show there within a few seconds. For another with its own settings, add /2 to the end of the address, then change those in OBS: right-click it, choose Interact, and click its gear."),
                note("How It Looks"),
                sel("style", "Theme", "The preview above shows it with made-up chat.", &[(json!("plain"), "Plain"), (json!("bubbles"), "Bubbles"), (json!("glass"), "Glass"), (json!("neon"), "Neon"), (json!("cards"), "Cards"), (json!("terminal"), "Terminal"), (json!("comic"), "Comic"), (json!("minimal"), "Minimal"), (json!("edge"), "Edge Light"), (json!("y2k"), "Y2K Window"), (json!("pastel"), "Pastel"), (json!("pixel"), "Pixel")]),
                sel("name", "Names", "", &[(json!("beside"), "Beside the Message"), (json!("above"), "Above It")]),
                sel("anim", "How Messages Come In", "", &[(json!("rise"), "Rise"), (json!("slide"), "Slide"), (json!("pop"), "Pop"), (json!("fade"), "Fade")]),
                sel("size", "Text Size", "", &[(json!(16), "Small"), (json!(20), "Normal"), (json!(26), "Large"), (json!(34), "Huge")]),
            ];
            // (only Plain has a background of your choosing: the others bring their own)
            if plain { f.push(sel("bg", "Background", "Behind each message. None lets your stream show through.", &[(json!("0"), "None"), (json!("0.35"), "Faint"), (json!("0.6"), "Half"), (json!("0.85"), "Solid")])); }
            f.extend([
                tog("shadow", "Shadow", "Makes the words easier to read over a busy picture."),
                sel("dir", "Newest Messages", "", &[(json!("bottom"), "At the Bottom"), (json!("top"), "At the Top")]),
                note("What It Shows"),
                sel("from", "Chat From", "", &[(json!("all"), "Every Platform"), (json!("twitch"), "Twitch"), (json!("kick"), "Kick"), (json!("youtube"), "YouTube")]),
                tog("badges", "Badges", "Subscriber, moderator and VIP badges next to names."),
                tog("marks", "Platform Marks", "A small Twitch or Kick mark before each name."),
                tog("events", "Follows, Subs and Raids", "Shown in with chat, as cards of their own: follows, subs, gifts, cheers, Kicks, donations and raids."),
                tog("first", "First-Time Chatters Stand Out", "A NEW tag and a glow on someone's first message in your channel, so you can say hello. Twitch tells OmniPulse who they are."),
                tog("pronouns", "Pronouns", "Twitch viewers' pronouns after their names, as they've set them at pronouns.alejo.io."),
                tog("bot", "The Bot's Own Messages", ""),
                tog("cmds", "Commands", "Messages starting with !, like !sr. Most streamers leave them out."),
                tog("stv", "7TV Emotes", "Your channel's and 7TV's global ones, on Twitch and Kick."),
                tog("bttv", "BetterTTV Emotes", "Your channel's and BetterTTV's global ones."),
                tog("ffz", "FrankerFaceZ Emotes", "Your channel's and FrankerFaceZ's global ones."),
                note("How Long They Stay"),
                sel("fade", "Fade After", "", &[(json!(0), "Never"), (json!(15), "15 seconds"), (json!(30), "30 seconds"), (json!(60), "1 minute"), (json!(120), "2 minutes")]),
                sel("max", "At Most", "How many show at once.", &[(json!(10), "10"), (json!(20), "20"), (json!(30), "30"), (json!(50), "50"), (json!(100), "100")]),
            ]);
            let _ = vals;
            f
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_feeds_kinds_are_read_from_its_page() {
        let k = kinds();
        assert!(k.len() > 20, "{k:?}");
        assert!(k.iter().any(|x| x.0 == "follow" && x.1 == "Follows" && x.2 == "Community"));
    }
    #[test]
    fn a_feeds_switches_are_its_hidden_list() {
        let prefs = json!({ "dockSet": { "activity": { "hidden": ["firstwords", "raid"] } } });
        let mut v = values(&prefs, "dockfeed");
        assert_eq!(v["show:raid"], json!(false));
        assert_eq!(v["show:follow"], json!(true));
        v.insert("show:raid".into(), json!(true));
        v.insert("show:bits".into(), json!(false));
        let all = saved(&prefs, "dockfeed", &v);
        assert_eq!(all["activity"]["hidden"], json!(["bits", "firstwords"]));
    }
    #[test]
    fn a_pages_own_change_is_kept_by_its_name() {
        let all = from_page(&json!({}), "multichat2", &json!({ "size": 30, "nonsense": 1 }));
        assert_eq!(all["multichat2"]["size"], json!(30));
        assert!(all["multichat2"].get("nonsense").is_none());
    }
}
