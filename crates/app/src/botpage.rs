// The chatbot's own settings page (Settings > Streaming > OmniPulse Chatbot): what goes on it, worked out
// from the chatbot's settings (as saved, with the page's changes over them) and what the chatbot says is going
// on (asked every couple of seconds while the page is open). The first time it's a path of steps; after
// that its card, its services, what it does, and what it's done lately. See docs/ease-of-use-pass.md.

use serde_json::{json, Map, Value};

pub const BOT: &str = "chatbot";

/// One line of a sheet or a step, as the page draws it (BotField)
pub struct Field { pub key: String, pub kind: &'static str, pub label: String, pub note: String, pub value: String, pub extra: String, pub n: i32 }

fn f(kind: &'static str, key: &str, label: &str, note: &str, value: &str, extra: &str) -> Field {
    Field { key: key.into(), kind, label: label.into(), note: note.into(), value: value.into(), extra: extra.into(), n: 0 }
}

/// What's set: the settings as saved, with the page's own changes (what differs from when it opened) over them
pub fn current(saved: &Map<String, Value>, edit: &Map<String, Value>, orig: &Map<String, Value>) -> Map<String, Value> {
    let mut cur = saved.clone();
    for (k, v) in edit { if orig.get(k) != Some(v) { cur.insert(k.clone(), v.clone()); } }
    cur
}

fn s<'a>(cur: &'a Map<String, Value>, k: &str) -> &'a str { cur.get(k).and_then(|v| v.as_str()).unwrap_or("").trim() }
fn on(cur: &Map<String, Value>, k: &str, dflt: bool) -> bool { match cur.get(k) { Some(Value::Bool(b)) => *b, Some(Value::String(x)) => x == "true", _ => dflt } }
fn n(cur: &Map<String, Value>, k: &str) -> String { cur.get(k).map(|v| match v { Value::String(x) => x.clone(), Value::Null => String::new(), x => x.to_string() }).unwrap_or_default() }

fn skipped(cur: &Map<String, Value>) -> Vec<String> {
    cur.get("setupSkip").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

/// A service's (or a feature's) sheet: its title, and its lines
pub fn sheet(id: &str, cur: &Map<String, Value>) -> (String, Vec<Field>) {
    let step = |k: i32, label: &str, value: &str, url: &str, tip: &str| { let mut x = f("step", "", label, tip, value, url); x.n = k; x };
    let account = |key: &str, label: &str, who: &str, bot: bool, plat: &str| f("account", key, label, if bot { "bot" } else { "channel" }, who, plat);
    match id {
        "twitch" => ("Twitch".into(), vec![
            f("head", "", "Your own Twitch app (once, about three minutes)", "", "", ""),
            step(1, "Open Twitch's developer console, and register an application (any name).", "", "https://dev.twitch.tv/console/apps/create", "Open Twitch's developer console"),
            step(2, "Its OAuth Redirect URL:", "http://localhost", "", ""),
            step(3, "Its Category: Chat Bot. Its Client Type: Public. Then Create.", "", "", ""),
            step(4, "Manage it, and copy its Client ID into here:", "", "", ""),
            f("text", "twitchClientId", "Client ID", "", s(cur, "twitchClientId"), "Twitch's long string of letters and numbers"),
            f("head", "", "Accounts", "", "", ""),
            account("twitch:channel", "Your channel", s(cur, "login"), false, "twitch"),
            account("twitch:bot", "Bot account", s(cur, "botLogin"), true, "twitch"),
            f("note", "", "Each one: sign in here, or copy its link and open it in a private window (or another browser) to sign in as a different account there.", "", "", ""),
        ]),
        "kick" => ("Kick".into(), vec![
            f("toggle", "kickOn", "Use Kick", "Your Kick chat too: requests, your actions, and alerts.", if on(cur, "kickOn", false) { "true" } else { "false" }, ""),
            f("head", "", "Your own Kick app (once, about three minutes)", "", "", ""),
            step(1, "Open Kick's developer settings, and create an app.", "", "https://kick.com/settings/developer", "Open Kick's developer settings"),
            step(2, "Its Redirect URL:", "http://localhost:8476/kick", "", ""),
            step(3, "Tick all of its scopes, then create it.", "", "", ""),
            step(4, "Copy its Client ID and Client Secret into here:", "", "", ""),
            f("text", "kickClientId", "Client ID", "", s(cur, "kickClientId"), "Kick's Client ID"),
            f("password", "kickClientSecret", "Client Secret", "", s(cur, "kickClientSecret"), "Kick's Client Secret"),
            f("text", "kickChannel", "Your Kick channel", "As in kick.com/name: filled in when you sign in.", s(cur, "kickChannel"), "yourname"),
            f("head", "", "Accounts", "", "", ""),
            account("kick:channel", "Your channel", s(cur, "kickLogin"), false, "kick"),
            account("kick:bot", "Bot account", s(cur, "kickBotLogin"), true, "kick"),
            f("note", "", "Each one: sign in here, or copy its link and open it in a private window on this PC to sign in as a different account there.", "", "", ""),
        ]),
        "obs" => ("OBS".into(), vec![
            f("toggle", "obsOn", "Connect to OBS", "Your actions can switch scenes, show and hide sources, and more.", if on(cur, "obsOn", false) { "true" } else { "false" }, ""),
            step(1, "In OBS: Tools > WebSocket Server Settings, and tick Enable WebSocket server.", "", "", ""),
            step(2, "Click Show Connect Info, and copy its password into here:", "", "", ""),
            f("password", "obsPassword", "Password", "", s(cur, "obsPassword"), "OBS's WebSocket password"),
            f("text", "obsAddress", "Address", "Where OBS is: this PC, and its port.", s(cur, "obsAddress"), "127.0.0.1:4455"),
        ]),
        "streamlabs" => ("Streamlabs".into(), vec![
            step(1, "Open Streamlabs' API settings, then API Tokens.", "", "https://streamlabs.com/dashboard#/settings/api-settings", "Open Streamlabs' API settings"),
            step(2, "Copy your Socket API Token into here:", "", "", ""),
            f("password", "streamlabsToken", "Socket API Token", "Donations then come in as A Donation, and as alerts. Empty: none.", s(cur, "streamlabsToken"), "Streamlabs' Socket API Token"),
        ]),
        "streamelements" => ("StreamElements".into(), vec![
            step(1, "Open your StreamElements channels page, and Show Secrets.", "", "https://streamelements.com/dashboard/account/channels", "Open StreamElements' channels page"),
            step(2, "Copy your JWT Token into here:", "", "", ""),
            f("password", "streamelementsToken", "JWT Token", "Tips then come in as A Donation, and as alerts. Empty: none.", s(cur, "streamelementsToken"), "StreamElements' JWT Token"),
        ]),
        "requests" => ("Song Requests".into(), vec![
            f("toggle", "requests", "Song requests from chat", "Your viewers ask for songs; Settings > Streaming > Song Requests has its limits and replies.", if on(cur, "requests", true) { "true" } else { "false" }, ""),
            f("text", "srCommands", "Request commands", "With commas between. !song says what's playing; !wrongsong takes back a viewer's last request.", s(cur, "srCommands"), "!sr, !request"),
        ]),
        "points" => ("Points".into(), vec![
            f("toggle", "pointsOn", "Points", "A currency of your own: viewers get some for chatting, to spend in your actions and games.", if on(cur, "pointsOn", false) { "true" } else { "false" }, ""),
            f("text", "pointsName", "Their name", "What they're called in chat (%pointsname%).", s(cur, "pointsName"), "points"),
            f("number", "pointsAmount", "How many each time", "For each viewer who's chatted since the last time.", &n(cur, "pointsAmount"), "10"),
            f("number", "pointsEvery", "Every (minutes)", "", &n(cur, "pointsEvery"), "10"),
            f("text", "pointsRanks", "Ranks", "Points, then the rank's name, with commas between: %rank% is a viewer's.", s(cur, "pointsRanks"), "0 Newcomer, 100 Regular, 500 Veteran"),
            f("toggle", "pointsOffline", "While you're offline too", "Off: only while you're live.", if on(cur, "pointsOffline", false) { "true" } else { "false" }, ""),
        ]),
        _ => (String::new(), Vec::new()),
    }
}

/// The setup path's steps: (id, title, note, done, optional); the step's own lines from sheet()
pub fn steps(cur: &Map<String, Value>) -> Vec<(&'static str, String, String, bool, bool)> {
    let tw = s(cur, "login");
    let kk = s(cur, "kickLogin");
    vec![
        ("twitch", "Your Twitch channel".into(), if tw.is_empty() { "Where your chat is. Streaming on Kick only? Skip it.".into() } else { format!("Signed in as {tw}") }, !tw.is_empty(), true),
        ("twitchbot", "A bot account for Twitch".into(), if s(cur, "botLogin").is_empty() { "Optional: another account that posts the replies.".into() } else { format!("{} posts the replies", s(cur, "botLogin")) }, !s(cur, "botLogin").is_empty(), true),
        ("kick", "Kick, too?".into(), if kk.is_empty() { "Optional: your Kick chat as well.".into() } else { format!("Signed in as {kk}") }, !kk.is_empty(), true),
        ("kickbot", "A bot account for Kick".into(), if s(cur, "kickBotLogin").is_empty() { "Optional: another Kick account that posts the replies.".into() } else { format!("{} posts the replies", s(cur, "kickBotLogin")) }, !s(cur, "kickBotLogin").is_empty(), true),
        ("obs", "OBS".into(), if on(cur, "obsOn", false) { "Connected to OBS's WebSocket server.".into() } else { "Optional: scenes and sources, from your actions.".into() }, on(cur, "obsOn", false), true),
        ("features", "Pick what it does".into(), "Song requests, alerts, points. Games, timers and the rest: in Actions, after.".into(), on(cur, "setupDone", false), false),
    ]
}

/// A setup step's own lines: its sheet's, the bot ones only their account
pub fn step_fields(id: &str, cur: &Map<String, Value>) -> Vec<Field> {
    match id {
        "twitch" => sheet("twitch", cur).1.into_iter().filter(|x| x.key != "twitch:bot").collect(),
        "kick" => sheet("kick", cur).1.into_iter().filter(|x| x.key != "kick:bot" && x.key != "kickOn").collect(),
        "twitchbot" | "kickbot" => {
            let plat = if id == "kickbot" { "kick" } else { "twitch" };
            let mut v = vec![f("note", "", "A second account (one you've made for the bot) posts the replies, so they don't come from your channel. Copy its sign-in link and open it in a private window, signed in as the bot there: your own account stays signed in in your browser.", "", "", "")];
            v.extend(sheet(plat, cur).1.into_iter().filter(|x| x.key == format!("{plat}:bot")));
            v
        }
        "obs" => sheet("obs", cur).1,
        "features" => vec![
            f("toggle", "requests", "Song requests", "!sr and a song, !song, !wrongsong.", if on(cur, "requests", true) { "true" } else { "false" }, ""),
            f("toggle", "alerts", "Alerts", "Follows, subs, bits, raids and more, on the Stream Widget.", if on(cur, "alerts", true) { "true" } else { "false" }, ""),
            f("toggle", "pointsOn", "Points", "A currency of your own, for chatting.", if on(cur, "pointsOn", false) { "true" } else { "false" }, ""),
        ],
        _ => Vec::new(),
    }
}

/// Which step is being done: the one asked for, else the first neither done nor skipped
pub fn step_now(cur: &Map<String, Value>, focus: &str) -> String {
    let skip = skipped(cur);
    let all = steps(cur);
    if all.iter().any(|x| x.0 == focus) { return focus.into(); }
    all.iter().find(|x| !x.3 && !skip.iter().any(|k| k == x.0)).map(|x| x.0.to_string()).unwrap_or_else(|| "features".into())
}

pub fn step_state(cur: &Map<String, Value>, id: &str, done: bool, now: &str) -> &'static str {
    if id == now { "now" } else if done { "done" } else if skipped(cur).iter().any(|k| k == id) { "skipped" } else { "todo" }
}

/// The services' tiles: (id, name, light, line1, line2), from what the chatbot says (or, not running, from what's set)
pub fn tiles(cur: &Map<String, Value>, st: &Value, running: bool) -> Vec<(&'static str, String, &'static str, String, String)> {
    let up = |k: &str| st[k]["up"].as_bool().unwrap_or(false);
    let light = |set: bool, k: &str| if !set { "off" } else if up(k) { "ok" } else { "warn" };
    let tw = s(cur, "login");
    let kk = s(cur, "kickLogin");
    let wait = if running { "Connecting..." } else { "The chatbot's stopped" };
    vec![
        ("twitch", "Twitch".into(), light(!tw.is_empty(), "twitch"), if tw.is_empty() { "Not signed in".into() } else if up("twitch") { tw.to_string() } else { format!("{tw}: {}", wait.to_lowercase()) }, if s(cur, "botLogin").is_empty() { "No bot account".into() } else { format!("Bot: {}", s(cur, "botLogin")) }),
        ("kick", "Kick".into(), light(on(cur, "kickOn", false) && !kk.is_empty(), "kick"), if !on(cur, "kickOn", false) { "Off".into() } else if kk.is_empty() { "Not signed in".into() } else if up("kick") { kk.to_string() } else { format!("{kk}: {}", wait.to_lowercase()) }, if s(cur, "kickBotLogin").is_empty() { "No bot account".into() } else { format!("Bot: {}", s(cur, "kickBotLogin")) }),
        ("obs", "OBS".into(), light(on(cur, "obsOn", false), "obs"), if !on(cur, "obsOn", false) { "Off".into() } else if up("obs") { "Connected".into() } else { "Waiting for OBS".into() }, "Scenes, sources, recording".into()),
        ("streamlabs", "Streamlabs".into(), light(!s(cur, "streamlabsToken").is_empty(), "streamlabs"), if s(cur, "streamlabsToken").is_empty() { "Not set up".into() } else if up("streamlabs") { "Listening".into() } else { "Not connected".into() }, "Donations".into()),
        ("streamelements", "StreamElements".into(), light(!s(cur, "streamelementsToken").is_empty(), "streamelements"), if s(cur, "streamelementsToken").is_empty() { "Not set up".into() } else if up("streamelements") { "Listening".into() } else { "Not connected".into() }, "Tips".into()),
    ]
}

/// What it does: (id, name, note, on, has a sheet)
pub fn feats(cur: &Map<String, Value>, st: &Value) -> Vec<(&'static str, String, String, bool, bool)> {
    let (all, live) = (st["actions"]["all"].as_u64(), st["actions"]["on"].as_u64());
    let name = if s(cur, "pointsName").is_empty() { "points".to_string() } else { s(cur, "pointsName").to_string() };
    vec![
        ("requests", "Song requests".into(), if s(cur, "srCommands").is_empty() { "!sr, !song, !wrongsong".into() } else { format!("{}, !song, !wrongsong", s(cur, "srCommands")) }, on(cur, "requests", true), true),
        ("alerts", "Alerts".into(), "Follows, subs, bits and raids, on the Stream Widget".into(), on(cur, "alerts", true), false),
        ("points", "Points and ranks".into(), format!("{} {name} every {} minutes", if n(cur, "pointsAmount").is_empty() { "10".into() } else { n(cur, "pointsAmount") }, if n(cur, "pointsEvery").is_empty() { "10".into() } else { n(cur, "pointsEvery") }), on(cur, "pointsOn", false), true),
        ("games", "Games".into(), "Slots, gamble, duel, heist, trivia: from Actions' templates".into(), false, false),
        ("actions", "Actions".into(), match (all, live) { (Some(a), Some(l)) => format!("{a} made, {l} on"), _ => "Commands, timers, events".into() }, true, false),
    ]
}

/// The card's name and line, and its counts
pub fn card(cur: &Map<String, Value>, st: &Value, running: bool) -> (String, String, String) {
    let name = ["botLogin", "login", "kickBotLogin", "kickLogin"].iter().map(|k| s(cur, k)).find(|x| !x.is_empty()).unwrap_or("Your chatbot").to_string();
    let mut on_ = Vec::new();
    if st["twitch"]["up"].as_bool() == Some(true) { on_.push("Twitch"); }
    if st["kick"]["up"].as_bool() == Some(true) { on_.push("Kick"); }
    let sub = if !running { "Stopped: start it to answer your chat.".to_string() }
        else if on_.is_empty() { "Running, but not in a chat yet: sign in below.".to_string() }
        else { format!("Answering in your {} chat{}", on_.join(" and "), if on_.len() > 1 { "s" } else { "" }) };
    let c = &st["counts"];
    let stats = if running { format!("Since it started: {} replies, {} requests, {} alerts", c["replies"].as_u64().unwrap_or(0), c["requests"].as_u64().unwrap_or(0), c["alerts"].as_u64().unwrap_or(0)) } else { String::new() };
    (name, sub, stats)
}

/// A page key as the settings have it (a feature's switch: its setting's own name)
pub fn key_of(k: &str) -> &str {
    match k { "feat:requests" => "requests", "feat:alerts" => "alerts", "feat:points" => "pointsOn", x => x }
}

/// A value as its field's type keeps it
pub fn value_of(manifest: &Value, key: &str, v: &str) -> Value {
    let kind = manifest.as_array().into_iter().flatten().find(|x| x["key"] == key).and_then(|x| x["type"].as_str()).unwrap_or("text");
    match kind {
        "toggle" => json!(v == "true"),
        "number" => v.trim().parse::<f64>().map(|x| json!(x)).unwrap_or(Value::Null),
        _ => json!(v),
    }
}

/// An account's icons: the plugin's method for each (in: sign in here; link: its link copied; out: signed out)
pub fn method(act: &str) -> Option<&'static str> {
    Some(match act {
        "in:twitch:channel" => "signInChannel", "link:twitch:channel" => "signInChannelCopy", "out:twitch:channel" => "signOutChannel",
        "in:twitch:bot" => "signInBot", "link:twitch:bot" => "signInBotCopy", "out:twitch:bot" => "signOutBot",
        "in:kick:channel" => "signInKick", "link:kick:channel" => "signInKickCopy", "out:kick:channel" => "signOutKickChannel",
        "in:kick:bot" => "signInKickBot", "link:kick:bot" => "signInKickBotCopy", "out:kick:bot" => "signOutKickBot",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(v: Value) -> Map<String, Value> { v.as_object().unwrap().clone() }

    #[test]
    fn the_path_goes_on_from_what_is_done_and_skipped() {
        let cur = m(json!({ "login": "Me", "setupSkip": ["twitchbot"] }));
        assert_eq!(step_now(&cur, ""), "kick");
        assert_eq!(step_now(&cur, "twitch"), "twitch");
        let st = steps(&cur);
        assert_eq!(step_state(&cur, "twitch", st[0].3, "kick"), "done");
        assert_eq!(step_state(&cur, "twitchbot", st[1].3, "kick"), "skipped");
        assert_eq!(step_state(&cur, "obs", st[4].3, "kick"), "todo");
    }

    #[test]
    fn the_page_shows_its_changes_over_what_is_saved() {
        let saved = m(json!({ "login": "Me", "requests": true, "token": "new" }));
        let orig = m(json!({ "login": "", "requests": true, "token": "" }));
        let edit = m(json!({ "login": "", "requests": false, "token": "" }));
        let cur = current(&saved, &edit, &orig);
        assert_eq!((cur["login"].as_str(), cur["requests"].as_bool(), cur["token"].as_str()), (Some("Me"), Some(false), Some("new")));
    }

    #[test]
    fn tiles_light_as_the_chatbot_says() {
        let cur = m(json!({ "login": "Me", "kickOn": true, "kickLogin": "", "obsOn": true }));
        let t = tiles(&cur, &json!({ "twitch": { "up": true }, "obs": { "up": false } }), true);
        assert_eq!((t[0].2, t[1].2, t[2].2, t[3].2), ("ok", "off", "warn", "off"));
        assert_eq!(t[0].3, "Me");
        assert!(method("link:kick:bot") == Some("signInKickBotCopy") && method("out:twitch:channel") == Some("signOutChannel"));
    }
}
