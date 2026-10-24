// The chatbot's own settings page (Settings > Streaming > OmniPulse Chatbot): what goes on it, worked out
// from the chatbot's settings (as saved, with the page's changes over them) and what the chatbot says is going
// on (asked every couple of seconds while the page is open). The first time it's a path of steps; after
// that its card, its services, what it does, and what it's done lately. See docs/ease-of-use-pass.md.

use serde_json::{json, Map, Value};

pub const BOT: &str = "chatbot";

/// OmniPulse's own apps, so you only sign in: their Client IDs (empty: not registered yet, so you make your own).
/// Kick's secret isn't here: the relay keeps it and finishes Kick's sign-ins (jam-server, /api/kick/*)
pub const TWITCH_ID: &str = "qgs2bz59rmqfzzdni2qf1l8imbnx2c";
pub const KICK_ID: &str = "01M5NZG55J6HQ93XPKVQ8CHRY8";

/// Your own app wanted instead (or OmniPulse's not there yet)
fn own_app(cur: &Map<String, Value>, plat: &str) -> bool {
    let built = if plat == "kick" { KICK_ID } else { TWITCH_ID };
    built.is_empty() || on(cur, if plat == "kick" { "kickOwnApp" } else { "twitchOwnApp" }, false)
}

/// What a sign-in's given besides the page's values: OmniPulse's apps, and where to finish a Kick sign-in
pub fn sign_in_extras(vals: &mut Map<String, Value>, relay: &str) {
    if !own_app(vals, "twitch") { vals.insert("twitchClientId".into(), json!(TWITCH_ID)); }
    if !own_app(vals, "kick") {
        vals.insert("kickClientId".into(), json!(KICK_ID));
        vals.insert("kickClientSecret".into(), json!(""));
        vals.insert("kickRelay".into(), json!(relay));
    } else {
        vals.insert("kickRelay".into(), json!(""));
    }
}

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
        "twitch" => ("Twitch".into(), { let own = own_app(cur, "twitch"); let mut v = vec![
            f("head", "", "Make your own Twitch app (once, about 3 minutes)", "", "", ""),
            step(1, "Open the Twitch developer console and click Register Your Application. Give it any name.", "", "https://dev.twitch.tv/console/apps/create", "Open the Twitch developer console"),
            step(2, "Under OAuth Redirect URLs, add this address:", "http://localhost", "", ""),
            step(3, "Set Category to Chat Bot and Client Type to Public. Then click Create.", "", "", ""),
            step(4, "Click Manage on your new app. Copy its Client ID and paste it below.", "", "", ""),
            f("text", "twitchClientId", "Client ID", "", s(cur, "twitchClientId"), "Paste your Client ID here"),
            f("head", "", "Sign in", "", "", ""),
            account("twitch:channel", "Your channel", s(cur, "login"), false, "twitch"),
            account("twitch:bot", "Bot account", s(cur, "botLogin"), true, "twitch"),
            f("note", "", "To sign in as a different account, copy the link instead. Then open it in a private window or another browser.", "", "", ""),
        ];
            // (OmniPulse's own app: nothing to make, only the sign-ins)
            if !own { v.drain(0..6); }
            if !TWITCH_ID.is_empty() { v.push(f("toggle", "twitchOwnApp", "Use my own Twitch app", "For developers. Most people can leave this off.", if own { "true" } else { "false" }, "")); }
            v }),
        "kick" => ("Kick".into(), { let own = own_app(cur, "kick"); let mut v = vec![
            f("toggle", "kickOn", "Use Kick", "The chatbot also answers your Kick chat.", if on(cur, "kickOn", false) { "true" } else { "false" }, ""),
            f("head", "", "Make your own Kick app (once, about 3 minutes)", "", "", ""),
            step(1, "Open Kick's developer settings and click Create App.", "", "https://kick.com/settings/developer", "Open Kick's developer settings"),
            step(2, "For the Redirect URL, use this address:", "http://localhost:8476/kick", "", ""),
            step(3, "Tick every scope, then create the app.", "", "", ""),
            step(4, "Copy the app's Client ID and Client Secret, and paste them below.", "", "", ""),
            f("text", "kickClientId", "Client ID", "", s(cur, "kickClientId"), "Paste your Client ID here"),
            f("password", "kickClientSecret", "Client Secret", "", s(cur, "kickClientSecret"), "Paste your Client Secret here"),
            f("text", "kickChannel", "Your Kick channel", "The name in your kick.com address. It's filled in when you sign in.", s(cur, "kickChannel"), "yourname"),
            f("head", "", "Sign in", "", "", ""),
            account("kick:channel", "Your channel", s(cur, "kickLogin"), false, "kick"),
            account("kick:bot", "Bot account", s(cur, "kickBotLogin"), true, "kick"),
            f("note", "", "To sign in as a different account, copy the link instead. Then open it in a private window on this PC.", "", "", ""),
        ];
            if !own { v.drain(1..8); }
            if !KICK_ID.is_empty() { v.push(f("toggle", "kickOwnApp", "Use my own Kick app", "For developers. Most people can leave this off.", if own { "true" } else { "false" }, "")); }
            v }),
        "obs" => ("OBS".into(), vec![
            f("toggle", "obsOn", "Connect to OBS", "Your actions can switch scenes, show or hide sources, and more.", if on(cur, "obsOn", false) { "true" } else { "false" }, ""),
            step(1, "In OBS, open Tools, then WebSocket Server Settings. Tick Enable WebSocket server.", "", "", ""),
            step(2, "Click Show Connect Info. Copy the password and paste it below.", "", "", ""),
            f("password", "obsPassword", "Password", "", s(cur, "obsPassword"), "Paste the OBS password here"),
            f("text", "obsAddress", "Address", "Leave this as it is if OBS runs on this PC.", s(cur, "obsAddress"), "127.0.0.1:4455"),
        ]),
        "streamlabs" => ("Streamlabs".into(), vec![
            step(1, "Open your Streamlabs API settings and go to API Tokens.", "", "https://streamlabs.com/dashboard#/settings/api-settings", "Open Streamlabs' API settings"),
            step(2, "Copy your Socket API Token and paste it below.", "", "", ""),
            f("password", "streamlabsToken", "Socket API Token", "Donations then show as alerts, and can start your actions. Leave it empty to turn this off.", s(cur, "streamlabsToken"), "Paste your Socket API Token here"),
        ]),
        "streamelements" => ("StreamElements".into(), vec![
            step(1, "Open your StreamElements channels page and click Show Secrets.", "", "https://streamelements.com/dashboard/account/channels", "Open StreamElements' channels page"),
            step(2, "Copy your JWT Token and paste it below.", "", "", ""),
            f("password", "streamelementsToken", "JWT Token", "Tips then show as alerts, and can start your actions. Leave it empty to turn this off.", s(cur, "streamelementsToken"), "Paste your JWT Token here"),
        ]),
        "requests" => ("Song Requests".into(), vec![
            f("toggle", "requests", "Song requests from chat", "Viewers can ask for songs. Limits and replies are in Settings > Streaming > Song Requests.", if on(cur, "requests", true) { "true" } else { "false" }, ""),
            f("text", "srCommands", "Request commands", "Separate them with commas. !song says what's playing, and !wrongsong takes back a request.", s(cur, "srCommands"), "!sr, !request"),
        ]),
        "points" => ("Points".into(), vec![
            f("toggle", "pointsOn", "Points", "Your own currency. Viewers earn it by chatting, and spend it in your actions and games.", if on(cur, "pointsOn", false) { "true" } else { "false" }, ""),
            f("text", "pointsName", "Name", "What points are called in chat.", s(cur, "pointsName"), "points"),
            f("number", "pointsAmount", "Points each time", "Given to each viewer who chatted since the last time.", &n(cur, "pointsAmount"), "10"),
            f("number", "pointsEvery", "How often (minutes)", "", &n(cur, "pointsEvery"), "10"),
            f("text", "pointsRanks", "Ranks", "Each rank is a number of points and a name, separated by commas.", s(cur, "pointsRanks"), "0 Newcomer, 100 Regular, 500 Veteran"),
            f("toggle", "pointsOffline", "Also while you're offline", "When this is off, viewers only earn points while you're live.", if on(cur, "pointsOffline", false) { "true" } else { "false" }, ""),
        ]),
        _ => (String::new(), Vec::new()),
    }
}

/// The setup path's steps: (id, title, note, done, optional); the step's own lines from sheet()
pub fn steps(cur: &Map<String, Value>) -> Vec<(&'static str, String, String, bool, bool)> {
    let tw = s(cur, "login");
    let kk = s(cur, "kickLogin");
    vec![
        ("twitch", "Your Twitch channel".into(), if tw.is_empty() { "Connect the channel you stream on. Only stream on Kick? Skip this.".into() } else { format!("Signed in as {tw}") }, !tw.is_empty(), true),
        ("twitchbot", "A bot account for Twitch".into(), if s(cur, "botLogin").is_empty() { "Optional. A second account that posts the chatbot's replies.".into() } else { format!("{} posts the replies", s(cur, "botLogin")) }, !s(cur, "botLogin").is_empty(), true),
        ("kick", "Kick".into(), if kk.is_empty() { "Optional. Connect your Kick chat too.".into() } else { format!("Signed in as {kk}") }, !kk.is_empty(), true),
        ("kickbot", "A bot account for Kick".into(), if s(cur, "kickBotLogin").is_empty() { "Optional. A second Kick account that posts the replies.".into() } else { format!("{} posts the replies", s(cur, "kickBotLogin")) }, !s(cur, "kickBotLogin").is_empty(), true),
        ("obs", "OBS".into(), if on(cur, "obsOn", false) { "Connected to OBS.".into() } else { "Optional. Lets your actions change scenes and sources.".into() }, on(cur, "obsOn", false), true),
        ("features", "Choose what it does".into(), "Song requests, alerts and points. You can add games, timers and more in Actions later.".into(), on(cur, "setupDone", false), false),
    ]
}

/// A setup step's own lines: its sheet's, the bot ones only their account
pub fn step_fields(id: &str, cur: &Map<String, Value>) -> Vec<Field> {
    match id {
        "twitch" => sheet("twitch", cur).1.into_iter().filter(|x| x.key != "twitch:bot").collect(),
        "kick" => sheet("kick", cur).1.into_iter().filter(|x| x.key != "kick:bot" && x.key != "kickOn").collect(),
        "twitchbot" | "kickbot" => {
            let plat = if id == "kickbot" { "kick" } else { "twitch" };
            let mut v = vec![f("note", "", "Use a second account you've made for the bot, so replies don't come from your channel. Copy the sign-in link, open it in a private window, and sign in as the bot there. You stay signed in to your own account in your normal browser.", "", "", "")];
            v.extend(sheet(plat, cur).1.into_iter().filter(|x| x.key == format!("{plat}:bot")));
            v
        }
        "obs" => sheet("obs", cur).1,
        "features" => vec![
            f("toggle", "requests", "Song requests", "Viewers can ask for songs with !sr.", if on(cur, "requests", true) { "true" } else { "false" }, ""),
            f("toggle", "alerts", "Alerts", "Shows follows, subs, bits and raids on the Stream Widget.", if on(cur, "alerts", true) { "true" } else { "false" }, ""),
            f("toggle", "pointsOn", "Points", "Viewers earn points by chatting.", if on(cur, "pointsOn", false) { "true" } else { "false" }, ""),
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
    let wait = if running { "Connecting..." } else { "The chatbot is stopped" };
    vec![
        ("twitch", "Twitch".into(), light(!tw.is_empty(), "twitch"), if tw.is_empty() { "Not signed in".into() } else if up("twitch") { tw.to_string() } else { format!("{tw}: {}", wait.to_lowercase()) }, if s(cur, "botLogin").is_empty() { "No bot account".into() } else { format!("Bot: {}", s(cur, "botLogin")) }),
        ("kick", "Kick".into(), light(on(cur, "kickOn", false) && !kk.is_empty(), "kick"), if !on(cur, "kickOn", false) { "Off".into() } else if kk.is_empty() { "Not signed in".into() } else if up("kick") { kk.to_string() } else { format!("{kk}: {}", wait.to_lowercase()) }, if s(cur, "kickBotLogin").is_empty() { "No bot account".into() } else { format!("Bot: {}", s(cur, "kickBotLogin")) }),
        ("obs", "OBS".into(), light(on(cur, "obsOn", false), "obs"), if !on(cur, "obsOn", false) { "Off".into() } else if up("obs") { "Connected".into() } else { "Waiting for OBS to open".into() }, "Scenes and sources".into()),
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
        ("alerts", "Alerts".into(), "Follows, subs, bits and raids on the Stream Widget".into(), on(cur, "alerts", true), false),
        ("points", "Points and ranks".into(), format!("{} {name} every {} minutes", if n(cur, "pointsAmount").is_empty() { "10".into() } else { n(cur, "pointsAmount") }, if n(cur, "pointsEvery").is_empty() { "10".into() } else { n(cur, "pointsEvery") }), on(cur, "pointsOn", false), true),
        ("games", "Games".into(), "Slots, duels, heists and trivia. Add them from Actions.".into(), false, false),
        ("actions", "Actions".into(), match (all, live) { (Some(a), Some(l)) => format!("{a} made, {l} on"), _ => "Commands, timers, events".into() }, true, false),
    ]
}

/// The card's name and line, and its counts
pub fn card(cur: &Map<String, Value>, st: &Value, running: bool) -> (String, String, String) {
    let name = ["botLogin", "login", "kickBotLogin", "kickLogin"].iter().map(|k| s(cur, k)).find(|x| !x.is_empty()).unwrap_or("Your chatbot").to_string();
    let mut on_ = Vec::new();
    if st["twitch"]["up"].as_bool() == Some(true) { on_.push("Twitch"); }
    if st["kick"]["up"].as_bool() == Some(true) { on_.push("Kick"); }
    let sub = if !running { "Stopped. Start it to answer your chat.".to_string() }
        else if on_.is_empty() { "Running, but not connected to a chat yet. Sign in below.".to_string() }
        else { format!("Answering in your {} chat{}", on_.join(" and "), if on_.len() > 1 { "s" } else { "" }) };
    let c = &st["counts"];
    let stats = if running { format!("Since it started: {} replies, {} song requests and {} alerts", c["replies"].as_u64().unwrap_or(0), c["requests"].as_u64().unwrap_or(0), c["alerts"].as_u64().unwrap_or(0)) } else { String::new() };
    (name, sub, stats)
}

/// Some to try in its test chat, from what it answers: a request, then the actions' own commands, then points
pub fn examples(cur: &Map<String, Value>) -> Vec<String> {
    let mut out = Vec::new();
    if on(cur, "requests", true) {
        let sr = s(cur, "srCommands").split(',').map(|x| x.trim()).find(|x| !x.is_empty()).unwrap_or("!sr").to_string();
        out.push(format!("{sr} take on me"));
    }
    let actions = match cur.get("actions") { Some(Value::String(t)) => serde_json::from_str::<Value>(t).unwrap_or(Value::Null), Some(v) => v.clone(), None => Value::Null };
    for a in actions.as_array().into_iter().flatten() {
        if out.len() >= 3 { break; }
        if a["on"] == false || a["trigger"]["type"] != "command" { continue; }
        if let Some(c) = a["trigger"]["command"].as_str().map(str::trim).filter(|c| c.starts_with('!') && !out.iter().any(|x| x == c)) { out.push(c.to_string()); }
    }
    if out.len() < 3 && on(cur, "pointsOn", false) { out.push("!points".into()); }
    if out.len() < 3 && on(cur, "requests", true) { out.push("!song".into()); }
    out
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
    fn omnipulses_own_apps_need_only_a_sign_in() {
        // (nothing to make: no Client ID to paste, only the accounts and the switch for your own app)
        let keys = |id: &str, cur: &Map<String, Value>| sheet(id, cur).1.into_iter().map(|x| x.key).filter(|k| !k.is_empty()).collect::<Vec<_>>();
        assert_eq!(keys("twitch", &m(json!({}))), vec!["twitch:channel", "twitch:bot", "twitchOwnApp"]);
        assert_eq!(keys("kick", &m(json!({}))), vec!["kickOn", "kickChannel", "kick:channel", "kick:bot", "kickOwnApp"]);
        assert!(keys("twitch", &m(json!({ "twitchOwnApp": true }))).contains(&"twitchClientId".to_string()));
        // (and a sign-in's given them, Kick's to be finished by the relay)
        let mut v = m(json!({}));
        sign_in_extras(&mut v, "https://relay");
        assert_eq!((v["twitchClientId"].as_str(), v["kickClientId"].as_str(), v["kickClientSecret"].as_str(), v["kickRelay"].as_str()), (Some(TWITCH_ID), Some(KICK_ID), Some(""), Some("https://relay")));
        let mut own = m(json!({ "kickOwnApp": true, "kickClientId": "mine", "kickClientSecret": "s" }));
        sign_in_extras(&mut own, "https://relay");
        assert_eq!((own["kickClientId"].as_str(), own["kickRelay"].as_str()), (Some("mine"), Some("")));
    }

    #[test]
    fn examples_come_from_what_it_answers() {
        let cur = m(json!({ "requests": true, "srCommands": "!request, !sr", "actions": [{ "on": true, "trigger": { "type": "command", "command": "!hello" } }, { "on": false, "trigger": { "type": "command", "command": "!off" } }] }));
        assert_eq!(examples(&cur), vec!["!request take on me", "!hello", "!song"]);
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
