// Settings panels (each a switch's Settings button): the web app's sections for Song Requests, the
// Stream Widget and its alerts, the Stream Deck's remote, OmniPulse Offset and Avoid Recent Plays, as
// fields for the settings dialog (the plugins' one): the same prefs, the same defaults.
//
// A field is as a plugin's setting is described ({ key, label, note, type }), with:
//   select: "options" (the values) and "labels" (what's shown for them)
//   info:   an address, "value" shown and "copy" copied (a key shown hidden in Streamer Mode)
//   note:   instructions (its label is the text)
//   button: pressed at once (not on Save): "value" is its label

use serde_json::{json, Map, Value};

/// A widget's look: what each of its outputs can have of its own (the rest, its address and port, are the first's)
pub fn look_keys(kind: &str) -> Vec<&'static str> {
    match kind {
        "widget" => vec!["widgetDesign", "widgetTheme", "widgetAlign", "widgetRounded", "widgetText", "wEnter", "wFade", "wScroll", "wSpin", "wUpNext", "wRequester", "wColor", "widgetOwnCol", "widgetCol1", "widgetCol2"],
        "lyricsw" => keys("lyricsw").into_iter().map(|(k, _)| k).collect(),
        _ => Vec::new(),
    }
}

/// An output's page: "widget@2" is the Stream Widget's second output (its kind, and its number)
pub fn output_of(name: &str) -> Option<(&str, usize)> {
    let (kind, n) = name.split_once('@')?;
    Some((kind, n.parse().ok().filter(|n| *n >= 2)?))
}

/// The other outputs, listed on a widget's page: each one's address, its look, Remove; then Add
fn outputs_list(prefs: &Value, kind: &str, base: &str) -> Vec<Value> {
    let mut f = vec![note("More Outputs")];
    let outs = crate::integrations::outputs_of(prefs, kind);
    if outs.is_empty() {
        f.push(note("Add another output for another scene, or for a second copy on screen at once. Each one has its own address and its own look."));
    }
    for i in 0..outs.len() {
        let n = i + 2;
        let url = format!("{base}{n}");
        // (one card: its address, copied; its look; taken away)
        let mut o = info(&format!("Output {n}"), &url, &url, "A second look, playing the same song. Add this address as another Browser source in OBS.");
        o["type"] = json!("output");
        o["key"] = json!(format!("out:{kind}:{n}"));
        f.push(o);
    }
    f.push(button(&format!("outAdd:{kind}"), "Another Output", "Add", "Starts out looking like this one."));
    f
}

/// Whether the Stream Suite's running: the Stream Widget is then its Now Playing
pub static SUITE_ON: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn title(name: &str) -> String {
    if name == "widget" && SUITE_ON.load(std::sync::atomic::Ordering::Relaxed) { return "Now Playing".into(); }
    if let Some((kind, n)) = output_of(name) { return format!("{}, Output {n}", title(kind)); }
    if name == "socials" { return "Social Plug".into(); }
    if crate::docksets::page_of(name).is_some() { return crate::docksets::title(name).into(); }
    if let Some(id) = name.strip_prefix("social:") {
        return crate::socials::list(&crate::SHARED.with(|s| s.borrow().as_ref().map(|sh| sh.borrow().web.prefs.clone())).unwrap_or_default()).iter().find(|m| m["id"] == id).map(crate::socials::name_of).unwrap_or_else(|| "Platform".into());
    }
    if name == "alertlist" { return "Alerts and Their Order".into(); }
    if let Some(k) = name.strip_prefix("alert:") { return format!("Alert: {}", crate::alerts::name(&Value::Null, k)); }
    match name {
        "requests" => "Song Requests",
        "replies" => "Chat Replies",
        "limits" => "Limit Requests",
        "smart" => "Seamless Mix (WIP)",
        "youtube" => "YouTube Music",
        "discord" => "Discord Status",
        "together" => "Listen Along & Jams",
        "report" => "Report a Bug or Suggest a Feature",
        "lyrics" => "Lyrics",
        "widget" => "Stream Widget",
        "lyricsw" => "Lyrics Widget",
        "alerts" => "Alerts",
        "remote" => "Stream Deck & Remote",
        "offset" => "OmniPulse Offset",
        "hidden" => "Hidden & Not Recommended",
        "plugins" => "Add a Plugin",
        "safe" => "What Counts as Safe",
        _ => "Avoid Recent Plays",
    }.to_string()
}

/// The panel a switch's Settings button opens (by the switch's pref)
pub fn of_switch(key: &str) -> Option<&'static str> {
    Some(match key { "streamSafe" => "safe", "req" => "requests", "smartOn" => "smart", "presence" => "discord", "jamsOn" => "together", "lyricsSources" => "lyrics", "widget" => "widget", "lyricsWidget" => "lyricsw", "alertsOn" => "alerts", "remoteOn" => "remote", "out2On" => "offset", "socials" => "socials", "hideOn" => "hidden", "plugAdd" => "plugins", _ => return None })
}

/// What a panel edits, and each one's default (the web app's)
fn keys(name: &str) -> Vec<(&'static str, Value)> {
    match name {
        "limits" => vec![("reqMax", json!(10)), ("reqPerUser", json!(2))],
        "youtube" => vec![("ytMirror", json!(false))],
        "discord" => vec![("presArt", json!(true)), ("presTime", json!(true)), ("presListen", json!(true)), ("presAsk", json!(false)), ("presLink", json!(true))],
        "together" => vec![("togetherName", json!(""))],
        "lyricsw" => vec![("lyricsTheme", json!("custom")), ("lyricsLines", json!(3)), ("lyricsBefore", json!(false)), ("lyricsScroll", json!("smooth")), ("lyricsScrollMs", json!(600)), ("lyricsScrollPlace", json!("top")), ("lyricsFadeEdges", json!(false)), ("lyricsSize", json!(48)), ("lyricsFont", json!("sans")), ("lyricsAlign", json!("center")), ("lyricsPlace", json!("")), ("lyricsStyle", json!("karaoke")), ("lyricsColor", json!("#ffffff")), ("lyricsTextGrad", json!(false)), ("lyricsColor2", json!("#9ad1ff")), ("lyricsOwnHi", json!(false)), ("lyricsHi", json!("#8e9bff")), ("lyricsHiGrad", json!(false)), ("lyricsHi2", json!("#ff8ad8")), ("lyricsGradDir", json!("90deg")), ("lyricsDim", json!(0.45)), ("lyricsTransform", json!("none")), ("lyricsShadow", json!(true)), ("lyricsOutline", json!(false)), ("lyricsOutlineSize", json!(4)), ("lyricsOutlineColor", json!("#000000")), ("lyricsOutlineOpacity", json!(1.0)), ("lyricsBox", json!(false)), ("lyricsBoxColor", json!("#0a0a0e")), ("lyricsBoxGrad", json!(false)), ("lyricsBoxColor2", json!("#2a1450")), ("lyricsBoxDir", json!("135deg")), ("lyricsBoxOpacity", json!(0.62)), ("lyricsTitle", json!(true)), ("lyricsHideNone", json!(true)), ("lyricsOffset", json!(0)), ("lyricsHidePaused", json!(true))],
        "smart" => vec![("shareRatings", json!(true)), ("smartBlend", json!(true)), ("smartKeys", json!(true)), ("smartMax", json!(10)), ("smartTape", json!(false)), ("smartRoll", json!(false)), ("smartVoices", json!(true))],
        "requests" => vec![("reqBot", json!("streamerbot")), ("reqRadioEdit", json!(false)), ("reqNoAi", json!(false)), ("reqLimits", json!(true)), ("reqMax", json!(10)), ("reqPerUser", json!(2)), ("reqPort", json!(7879)), ("botRemote", json!(false))],
        "socials" => vec![("socialStyle", json!("card")), ("socialAlign", json!("left")), ("socialSecs", json!(8)), ("socialAll", json!(false)), ("socialRounded", json!(true)), ("socialColoured", json!(false)), ("socialMotion", json!("rise"))],
        "widget" => vec![("widgetPort", json!(7878)), ("widgetDesign", json!("card")), ("widgetTheme", json!("colours")), ("wEnter", json!("fade")), ("widgetAlign", json!("left")), ("widgetRounded", json!(true)), ("widgetText", json!(1)), ("wFade", json!(true)), ("wScroll", json!(true)), ("wSpin", json!(true)), ("wUpNext", json!("soon")), ("wRequester", json!(true)), ("wColor", json!(1.2)), ("alertInWidget", json!(false)), ("widgetOwnCol", json!(false)), ("widgetCol1", json!("#8e9bff")), ("widgetCol2", json!("#b48eff"))],
        "report" => vec![("reportKind", json!("bug")), ("reportTitle", json!("")), ("reportText", json!("")), ("reportContact", json!("")), ("reportInfo", json!(true))],
        "alerts" => vec![("alertsOn", json!(false)), ("alertInWidget", json!(false)), ("alertGif", json!("")), ("alertTheme", json!("widget")), ("alertBot", json!("streamerbot")), ("sbHost", json!("127.0.0.1:8080")), ("sbOn", json!(true)), ("alertTrans", json!("seamless")), ("alertCSize", json!("medium")), ("alertCDir", json!("diag")), ("alertCAngle", json!(0)), ("alertAnim", json!("slide")), ("alertOwnCol", json!(false)), ("alertCol1", json!("#8e9bff")), ("alertCol2", json!("#b38cff")), ("alertDir", json!("115deg")), ("alertIcon", json!(1)), ("alertSecs", json!(5)), ("alertVol", json!(0.8)), ("botRemote", json!(false))],
        "remote" => vec![("remotePort", json!(7880))],
        "offset" => vec![("out2Offset", json!(0)), ("out2Volume", json!(1))],
        "hidden" => vec![],
        "plugins" => vec![("plugSource", json!(""))],
        "dockchat" | "dockfeed" | "multichat" => vec![],
        _ => vec![("recentWindow", json!("week"))],
    }
}

/// The lyrics widget's fonts: the system's (on every Windows PC), then some from Google Fonts (web)
/// The Lyrics Widget's themes: their look, set all at once when one's picked (anything of it changed after:
/// Custom)
pub const LYRICS_THEME_KEYS: [&str; 17] = ["lyricsFont", "lyricsTransform", "lyricsColor", "lyricsTextGrad", "lyricsOwnHi", "lyricsHi", "lyricsHiGrad", "lyricsHi2", "lyricsDim", "lyricsShadow",
    "lyricsOutline", "lyricsOutlineSize", "lyricsOutlineColor", "lyricsBox", "lyricsBoxColor", "lyricsBoxOpacity", "lyricsStyle"];

pub fn lyrics_theme(name: &str) -> Vec<(&'static str, Value)> {
    let v = match name {
        "neon" => json!({ "lyricsFont": "montserrat", "lyricsTransform": "none", "lyricsColor": "#e8e2ff", "lyricsTextGrad": false, "lyricsOwnHi": true, "lyricsHi": "#ff3df2", "lyricsHiGrad": true, "lyricsHi2": "#3df2ff", "lyricsDim": 0.45, "lyricsShadow": true, "lyricsOutline": false, "lyricsOutlineSize": 2, "lyricsOutlineColor": "#000000", "lyricsBox": false, "lyricsBoxColor": "#000000", "lyricsBoxOpacity": 0.4, "lyricsStyle": "karaoke" }),
        "clean" => json!({ "lyricsFont": "inter", "lyricsTransform": "none", "lyricsColor": "#ffffff", "lyricsTextGrad": false, "lyricsOwnHi": true, "lyricsHi": "#ffffff", "lyricsHiGrad": false, "lyricsHi2": "#ffffff", "lyricsDim": 0.4, "lyricsShadow": true, "lyricsOutline": false, "lyricsOutlineSize": 2, "lyricsOutlineColor": "#000000", "lyricsBox": false, "lyricsBoxColor": "#000000", "lyricsBoxOpacity": 0.4, "lyricsStyle": "highlight" }),
        "outline" => json!({ "lyricsFont": "anton", "lyricsTransform": "uppercase", "lyricsColor": "#ffffff", "lyricsTextGrad": false, "lyricsOwnHi": true, "lyricsHi": "#ffd24a", "lyricsHiGrad": false, "lyricsHi2": "#ffd24a", "lyricsDim": 0.6, "lyricsShadow": false, "lyricsOutline": true, "lyricsOutlineSize": 5, "lyricsOutlineColor": "#000000", "lyricsBox": false, "lyricsBoxColor": "#000000", "lyricsBoxOpacity": 0.4, "lyricsStyle": "karaoke" }),
        "glass" => json!({ "lyricsFont": "poppins", "lyricsTransform": "none", "lyricsColor": "#ffffff", "lyricsTextGrad": false, "lyricsOwnHi": false, "lyricsHi": "#8e9bff", "lyricsHiGrad": false, "lyricsHi2": "#b48eff", "lyricsDim": 0.5, "lyricsShadow": false, "lyricsOutline": false, "lyricsOutlineSize": 2, "lyricsOutlineColor": "#000000", "lyricsBox": true, "lyricsBoxColor": "#1a1d2e", "lyricsBoxOpacity": 0.55, "lyricsStyle": "karaoke" }),
        "retro" => json!({ "lyricsFont": "mono", "lyricsTransform": "uppercase", "lyricsColor": "#b6ffbe", "lyricsTextGrad": false, "lyricsOwnHi": true, "lyricsHi": "#3cff6e", "lyricsHiGrad": false, "lyricsHi2": "#3cff6e", "lyricsDim": 0.45, "lyricsShadow": false, "lyricsOutline": false, "lyricsOutlineSize": 2, "lyricsOutlineColor": "#000000", "lyricsBox": true, "lyricsBoxColor": "#000000", "lyricsBoxOpacity": 0.75, "lyricsStyle": "highlight" }),
        "minimal" => json!({ "lyricsFont": "sans", "lyricsTransform": "none", "lyricsColor": "#ffffff", "lyricsTextGrad": false, "lyricsOwnHi": false, "lyricsHi": "#ffffff", "lyricsHiGrad": false, "lyricsHi2": "#ffffff", "lyricsDim": 0.3, "lyricsShadow": true, "lyricsOutline": false, "lyricsOutlineSize": 2, "lyricsOutlineColor": "#000000", "lyricsBox": false, "lyricsBoxColor": "#000000", "lyricsBoxOpacity": 0.4, "lyricsStyle": "plain" }),
        _ => return Vec::new(),
    };
    LYRICS_THEME_KEYS.iter().filter_map(|k| v.get(*k).map(|x| (*k, x.clone()))).collect()
}

const FONTS: [(&str, &str); 43] = [
    ("sans", "Clean (system)"), ("segoe", "Segoe UI"), ("arial", "Arial"), ("verdana", "Verdana"), ("tahoma", "Tahoma"), ("trebuchet", "Trebuchet MS"),
    ("calibri", "Calibri"), ("bahnschrift", "Bahnschrift"), ("franklin", "Franklin Gothic"), ("bold", "Impact"), ("black", "Arial Black"),
    ("serif", "Georgia"), ("times", "Times New Roman"), ("palatino", "Palatino"), ("cambria", "Cambria"), ("gabriola", "Gabriola"),
    ("mono", "Consolas"), ("courier", "Courier New"), ("comic", "Comic Sans MS"), ("script", "Segoe Script"), ("print", "Segoe Print"),
    ("montserrat", "Montserrat (web)"), ("poppins", "Poppins (web)"), ("inter", "Inter (web)"), ("roboto", "Roboto (web)"), ("nunito", "Nunito (web)"),
    ("rubik", "Rubik (web)"), ("fredoka", "Fredoka (web)"), ("oswald", "Oswald (web)"), ("bebas", "Bebas Neue (web)"), ("anton", "Anton (web)"),
    ("russo", "Russo One (web)"), ("righteous", "Righteous (web)"), ("orbitron", "Orbitron (web)"), ("bangers", "Bangers (web)"), ("luckiest", "Luckiest Guy (web)"),
    ("marker", "Permanent Marker (web)"), ("caveat", "Caveat (web)"), ("pacifico", "Pacifico (web)"), ("lobster", "Lobster (web)"), ("dancing", "Dancing Script (web)"),
    ("playfair", "Playfair Display (web)"), ("pixel", "Press Start 2P (web)"),
];

/// A lyrics font as the preview draws it: a system one's family, or (true) a Google Font's, to be fetched
pub fn lyrics_family(key: &str) -> (&'static str, bool) {
    let sys = match key {
        "segoe" => "Segoe UI", "arial" => "Arial", "verdana" => "Verdana", "tahoma" => "Tahoma", "trebuchet" => "Trebuchet MS", "calibri" => "Calibri",
        "bahnschrift" => "Bahnschrift", "franklin" => "Franklin Gothic Medium", "bold" => "Impact", "black" => "Arial Black", "serif" => "Georgia",
        "times" => "Times New Roman", "palatino" => "Palatino Linotype", "cambria" => "Cambria", "gabriola" => "Gabriola", "mono" => "Consolas",
        "courier" => "Courier New", "comic" => "Comic Sans MS", "script" => "Segoe Script", "print" => "Segoe Print", _ => "",
    };
    if !sys.is_empty() || key == "sans" { return (sys, false); }
    // (the web ones: their names, as in the list, without "(web)")
    FONTS.iter().find(|(k, _)| *k == key).map_or(("", false), |(_, n)| (n.trim_end_matches(" (web)"), true))
}

/// Lyrics' sources: asked in this order, the first with timed lyrics used; each switchable, moved up or
/// down (in the panel's values: "order", the ids, and "on:<id>" each)
fn lyrics_fields(vals: &Map<String, Value>) -> Vec<Value> {
    let order: Vec<String> = vals.get("order").and_then(|o| o.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    let n = order.len();
    let mut f = vec![note("OmniPulse asks these sources for lyrics in this order, and uses the first one with timed lyrics. In Now Playing, you can switch a song's lyrics to another source.")];
    for (i, id) in order.iter().enumerate() {
        let (mut acts, mut labels) = (Vec::new(), Vec::new());
        if i > 0 { acts.push(format!("lyUp:{id}")); labels.push("\u{2191}".to_string()); }
        if i + 1 < n { acts.push(format!("lyDown:{id}")); labels.push("\u{2193}".to_string()); }
        f.push(json!({ "key": format!("on:{id}"), "label": crate::player::lyrics_name(id), "note": "", "type": "row", "actions": acts, "labels": labels }));
    }
    f
}

/// Lyrics' sources as they're set (all of them, the ones switched off too)
pub fn lyrics_values(prefs: &Value) -> Map<String, Value> {
    let mut m = Map::new();
    let all = crate::player::lyrics_all(prefs);
    m.insert("order".into(), json!(all.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>()));
    for (id, on) in all { m.insert(format!("on:{id}"), json!(on)); }
    m
}

/// Lyrics' sources saved: prefs.lyricsOrder, as the web app keeps it ([{ id, on }])
pub fn lyrics_save(vals: &Map<String, Value>) -> Value {
    let order: Vec<String> = vals.get("order").and_then(|o| o.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    json!(order.iter().map(|id| json!({ "id": id, "on": vals.get(&format!("on:{id}")).and_then(|v| v.as_bool()) != Some(false) })).collect::<Vec<_>>())
}

/// Moved up (-1) or down (1) in the panel's values
pub fn lyrics_move(vals: &mut Map<String, Value>, id: &str, by: i32) {
    let mut order: Vec<String> = vals.get("order").and_then(|o| o.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    if let Some(i) = order.iter().position(|x| x == id) {
        let j = (i as i32 + by).clamp(0, order.len() as i32 - 1) as usize;
        order.swap(i, j);
    }
    vals.insert("order".into(), json!(order));
}

/// The values a panel starts with: what's set, else the default
pub fn values(name: &str, prefs: &Value) -> Map<String, Value> {
    if name == "lyrics" { return lyrics_values(prefs); }
    // (Chat Replies: each one's own words, empty for the default)
    if name == "replies" {
        return crate::requests::MSG_KEYS.iter().map(|(k, _)| (format!("msg:{k}"), json!(prefs["reqMsgs"][*k].as_str().unwrap_or("")))).collect();
    }
    if name == "alertlist" { return crate::alerts::list_values(prefs); }
    if crate::docksets::page_of(name).is_some() { return crate::docksets::values(prefs, name); }
    if name == "safe" { return crate::streamsafe::panel_values(prefs); }
    if let Some(k) = name.strip_prefix("alert:") { return crate::alerts::edit_values(prefs, k); }
    if let Some(id) = name.strip_prefix("social:") { return crate::socials::edit_values(prefs, id); }
    if name == "socials" {
        let mut m = crate::socials::list_values(prefs);
        for (k, d) in keys("socials") { m.insert(k.to_string(), if prefs[k].is_null() { d } else { prefs[k].clone() }); }
        return m;
    }
    if let Some((kind, n)) = output_of(name) {
        let over = crate::integrations::outputs_of(prefs, kind).get(n - 2).cloned().unwrap_or_default();
        let merged = crate::integrations::merged(prefs, &over);
        let all = values(kind, &merged);
        return look_keys(kind).into_iter().filter_map(|k| all.get(k).map(|v| (k.to_string(), v.clone()))).collect();
    }
    let mut m = Map::new();
    if name == "requests" {
        let bot = &prefs["pluginSettings"][crate::botpage::BOT];
        for (k, d) in bot_req_keys() { m.insert(format!("bot:{k}"), if bot[k].is_null() { d } else { bot[k].clone() }); }
    }
    for (k, d) in keys(name) {
        let v = if prefs[k].is_null() { d } else { prefs[k].clone() };
        m.insert(k.to_string(), v);
    }
    // (the alerts' bot follows the requests' one until it's picked, as the web app's)
    if name == "alerts" && prefs["alertBot"].is_null() {
        let b = match prefs["reqBot"].as_str() { Some("mixitup") => "mixitup", Some("other") => "generic", _ => "streamerbot" };
        m.insert("alertBot".into(), json!(b));
    }
    m
}

pub fn keys_of(name: &str) -> Vec<&'static str> { keys(name).into_iter().map(|(k, _)| k).collect() }

/// Two values the same (1 and 1.0 alike)
pub fn same(a: &Value, b: &Value) -> bool {
    match (a.as_f64(), b.as_f64()) { (Some(x), Some(y)) => (x - y).abs() < 1e-9, _ => a == b }
}

/// Streamer.bot's reply, posted on the platform the command came from (its %commandSource%): a Twitch
/// Send Message sub-action answers on Twitch whatever chat asked
const SB_REPLY_CODE: &str = r#"using System;

public class CPHInline
{
    public bool Execute()
    {
        // what OmniPulse answered (Fetch URL's variable), and the chat the command came from
        if (!CPH.TryGetArg("reply", out string reply) || string.IsNullOrWhiteSpace(reply)) return true;
        CPH.TryGetArg("commandSource", out string source);
        switch ((source ?? "").ToLowerInvariant())
        {
            case "youtube": CPH.SendYouTubeMessage(reply); break;
            case "kick": CPH.SendKickMessage(reply); break;
            case "trovo": CPH.SendTrovoMessage(reply); break;
            default: CPH.SendMessage(reply); break;
        }
        return true;
    }
}
"#;

/// The bots Song Requests knows, as (id, name, whether it's online (a cloud bot: it reaches the PC over the
/// internet), its !sr, !song and !wrongsong, its steps). In the commands, {base} is the address up to the
/// port and {key} the secret key. Each written its own way, as its documentation has it; where that isn't
/// known for sure, the general address, with its own variables for USERNAME and SONG.
/// The chatbot's own request settings (its manifest's keys and defaults): on the Song Requests page while it's on
pub fn bot_req_keys() -> Vec<(&'static str, Value)> {
    vec![("requests", json!(true)), ("srCommands", json!("!sr, !request")), ("songCommands", json!("!song")), ("wrongCommands", json!("!wrongsong")), ("srWho", json!("everyone")), ("srCooldown", json!(0)), ("srCost", json!(0))]
}

const REQ_BOTS: &[(&str, &str, bool, &str, &str, &str, &str)] = &[
    ("streamerbot", "Streamer.bot", false, "{base}/request?key={key}&user=%user%&q=%rawInputUrlEncoded%", "{base}/now", "{base}/wrongsong?key={key}&user=%user%&mod=%isModerator%",
        "1. Make an action with a Command trigger for !sr.\n2. Add Core > Network > Fetch URL with the request address, and name its variable reply.\n3. Add Core > C# > Execute C# Code with the Reply Code below, then click Compile. It answers in the chat that asked.\n4. Do the same for !song and !wrongsong, with their addresses."),
    ("mixitup", "Mix It Up", false, "{base}/request?key={key}&user=$username&q=$allargs", "{base}/now", "{base}/wrongsong?key={key}&user=$username&mod=$userroles",
        "1. Make a Chat Command, !sr.\n2. Add a Web Request action with the request address, its response as plain text.\n3. Add a Chat action that sends $webrequestresult.\n4. The same for !song and !wrongsong, with their addresses."),
    ("nightbot", "Nightbot", true, "!commands add !sr $(urlfetch {base}/request?key={key}&q=$(querystring))", "!commands add !song $(urlfetch {base}/now)", "!commands add !wrongsong $(urlfetch {base}/wrongsong?key={key})",
        "Paste each one into your chat, as yourself. (Nightbot says who asked, and whether they're a moderator, by itself.)"),
    ("streamelements", "StreamElements", true, "!command add !sr $(customapi {base}/request?key={key}&user=$(user)&q=$(queryescape ${1:}))", "!command add !song $(customapi {base}/now)", "!command add !wrongsong $(customapi {base}/wrongsong?key={key}&user=$(user))",
        "Paste each one into your chat, as yourself."),
    ("kickbot", "KickBot", true, "{{request(\"{base}/request?key={key}&format=json&user={{sender.username}}&q={{param}}\").message}}", "{{request(\"{base}/now?format=json\").message}}", "{{request(\"{base}/wrongsong?key={key}&format=json&user={{sender.username}}\").message}}",
        "1. In KickBot's dashboard, make a custom command, !sr, and paste the first as its response.\n2. The same for !song and !wrongsong.\nIf a command answers with the variable names themselves, KickBot isn't filling them in inside a request: say so, and we'll find another way."),
];

fn tog(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "toggle" }) }
fn num(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "number" }) }
fn text(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "text" }) }
fn sel(key: &str, label: &str, note: &str, opts: &[(Value, &str)]) -> Value {
    json!({ "key": key, "label": label, "note": note, "type": "select", "options": opts.iter().map(|o| o.0.clone()).collect::<Vec<_>>(), "labels": opts.iter().map(|o| o.1).collect::<Vec<_>>() })
}
/// A widget's layout: picked from sketches of each (the page draws them, in the options' order)
fn design(key: &str, label: &str) -> Value {
    let mut v = sel(key, label, "", &[(json!("card"), "Card"), (json!("bar"), "Slim Bar"), (json!("square"), "Square"), (json!("text"), "Text Only"), (json!("pill"), "Pill"), (json!("vinyl"), "Vinyl"), (json!("poster"), "Poster"), (json!("ticker"), "Ticker"),
        (json!("split"), "Split"), (json!("lower"), "Lower Third"), (json!("cassette"), "Cassette"), (json!("equaliser"), "Equaliser"), (json!("nownext"), "Now and Next"), (json!("polaroid"), "Polaroid"), (json!("mini"), "Mini"), (json!("vertical"), "Vertical")]);
    v["type"] = json!("designpick");
    v
}
/// A widget's theme: picked from sketches of each, in the options' order
fn themepick(key: &str, label: &str) -> Value {
    let mut v = sel(key, label, "", &[(json!("colours"), "Colours"), (json!("solid"), "Dark"), (json!("paper"), "Light"), (json!("neon"), "Neon"), (json!("frosted"), "Frosted Glass"), (json!("clear"), "No Panel")]);
    v["type"] = json!("themepick");
    v
}
fn info(label: &str, value: &str, copy: &str, note: &str) -> Value { json!({ "key": "", "label": label, "note": note, "type": "info", "value": value, "copy": copy }) }
/// A box for a few lines (a report's words)
fn area(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "area" }) }
fn note(t: &str) -> Value { json!({ "key": "", "label": t, "note": "", "type": "note" }) }
fn button(key: &str, label: &str, press: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "button", "value": press }) }
fn pct(from: f64, to: f64, step: f64) -> Vec<(Value, String)> {
    let mut out = Vec::new();
    let mut v = from;
    while v <= to + 1e-9 { let r = (v * 100.0).round() / 100.0; out.push((json!(r), format!("{}%", (r * 100.0).round()))); v += step; }
    out
}
/// − and + through the options (a size, a count), as the web app's steppers
fn stepper(key: &str, label: &str, note: &str, opts: Vec<(Value, String)>) -> Value {
    let mut v = sel_owned(key, label, note, opts);
    v["type"] = json!("stepper");
    v
}
fn counts(from: u64, to: u64) -> Vec<(Value, String)> { (from..=to).map(|n| (json!(n), n.to_string())).collect() }
/// A slider from `min` to `max` by `step` (`unit`: "%" for 0 to 1 shown as a percentage, or what's shown
/// after the number); `with_box`: a box to type it in too
fn range(key: &str, label: &str, note: &str, min: f64, max: f64, step: f64, unit: &str, with_box: bool) -> Value {
    json!({ "key": key, "label": label, "note": note, "type": if with_box { "rangenum" } else { "range" }, "min": min, "max": max, "step": step, "unit": unit })
}
/// A colour: its swatch, its code, and Windows' colour picker
fn color(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "color" }) }
/// A switch with its own settings beside it (`press`: what Configure does)
fn toggle_cfg(key: &str, label: &str, note: &str, press: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "togglecfg", "press": press }) }

fn sel_owned(key: &str, label: &str, note: &str, opts: Vec<(Value, String)>) -> Value {
    let o: Vec<(Value, &str)> = opts.iter().map(|(v, l)| (v.clone(), l.as_str())).collect();
    sel(key, label, note, &o)
}

/// Bots elsewhere (the web app's botRemoteBlock): the switch, and with it on, how
fn remote_bots(vals: &Map<String, Value>, what: &str, port: u64, path: &str, lan: &str) -> Vec<Value> {
    let mut out = vec![tog("botRemote", "Bots on Other Devices", &format!("When this is off, only programs on this PC can use these addresses. When it's on, other devices can too{}", if what == "alerts" { ", with the key in their address." } else { " (with the key)." }))];
    if vals["botRemote"].as_bool() == Some(true) {
        out.push(note(&format!("1. For a bot on another computer at home, use http://{lan}:{port}{path} instead of 127.0.0.1.\n2. For an online bot (one outside your network), forward port {port} (TCP) on your router to this PC, {lan}. Then give the bot the same address with your public IP address instead of {lan}.\nCareful: your public IP address points to your home. Don't show it on stream or in screenshots, and don't share it. Keep Streamer Mode on while you set this up. With it and the key, anyone could {}.", if what == "alerts" { "send alerts to your stream" } else { "request songs" })));
    }
    out
}

/// Mix It Up's events, each with its alert's type and Mix It Up's special identifiers for it (as its
/// documentation lists them: Twitch's Channel Followed, Subscribed, Resubscribed, Subscriptions Gifted, Bits
/// Cheered, Channel Raided, Hype Train; and Donation Received)
const MIXITUP_EVENTS: [(&str, &str); 8] = [
    ("Channel Followed", "follow&user=$username"),
    ("Channel Subscribed", "sub&user=$username&amount=$usersubdurationmonths"),
    ("Channel Resubscribed", "sub&user=$username&amount=$usersubmonths&message=$message"),
    ("Mass Subscriptions Gifted", "gift&user=$username&amount=$subsgiftedamount"),
    ("Bits Cheered", "bits&user=$username&amount=$bitsamount&message=$messagenocheermotes"),
    ("Donation Received", "donation&user=$username&amount=$donationamount&message=$donationmessage"),
    ("Channel Raided", "raid&user=$username&amount=$raidviewercount"),
    ("Hype Train Level Up", "hypetrain&amount=$hypetrainlevel"),
];

/// Any other bot's events (Generic): each alert's own address, its type in, and a word in capitals
/// for each of the bot's variables to put in its place
const GENERIC_EVENTS: [(&str, &str); 11] = [
    ("Follows", "follow&user=NAME"),
    ("Subscriptions", "sub&user=NAME&amount=MONTHS&message=MESSAGE"),
    ("Gifted Subs", "gift&user=NAME&amount=COUNT"),
    ("Bits", "bits&user=NAME&amount=BITS&message=MESSAGE"),
    ("Kicks", "kicks&user=NAME&amount=KICKS&message=MESSAGE"),
    ("Donations", "donation&user=NAME&amount=AMOUNT&message=MESSAGE"),
    ("Raids & Hosts", "raid&user=NAME&amount=VIEWERS"),
    ("Hype Trains", "hypetrain&amount=LEVEL"),
    ("Polls", "poll&user=QUESTION&message=MESSAGE"),
    ("Bans", "ban&user=NAME"),
    ("Watch Streaks", "watchstreak&user=NAME&amount=STREAMS"),
];

/// A panel's fields, for the values as they are in it (prefs: for the keys, Streamer Mode, the LAN address)
pub fn fields(name: &str, vals: &Map<String, Value>, prefs: &Value, lan: &str) -> Vec<Value> {
    // the Chat Dock, the Activity Feed, the Multichat Overlay
    if crate::docksets::page_of(name).is_some() { return crate::docksets::fields(name, vals, prefs["widgetPort"].as_u64().unwrap_or(7878)); }
    // the Social Plug: its look, then its platforms in turn (each one's own page from there)
    if name == "socials" {
        let port = prefs["widgetPort"].as_u64().unwrap_or(7878);
        let url = format!("http://127.0.0.1:{port}/socials");
        // (your platforms first: the preview above shows them as they're added)
        let mut f = vec![note("Your Platforms")];
        f.extend(crate::socials::list_rows(prefs));
        f.push(note("How It Looks"));
        f.push(sel("socialStyle", "Style", "Card and Pill sit on a dark panel. Glass is frosted, Neon glows in each platform's colour, Comic is a white speech bubble, and Minimal is just the icon and words over your stream.", &[(json!("card"), "Card"), (json!("pill"), "Pill"), (json!("glass"), "Glass"), (json!("neon"), "Neon"), (json!("comic"), "Comic"), (json!("minimal"), "Minimal")]));
        if matches!(vals["socialStyle"].as_str().unwrap_or("card"), "card" | "pill") {
            f.push(tog("socialColoured", "In Each Platform's Colours", "The whole card in the platform's colours, not just its icon."));
        }
        f.push(sel("socialAlign", "Alignment", "", &[(json!("left"), "Left"), (json!("centre"), "Centre"), (json!("right"), "Right")]));
        f.push(tog("socialRounded", "Rounded Corners", ""));
        f.push(tog("socialAll", "All at Once", "Shows every platform side by side. When this is off, they show one at a time, in turn."));
        if vals["socialAll"].as_bool() == Some(true) {
            f.push(sel("socialMotion", "Animation", "How they come in, one after another, before a slow glow runs along them.", &[(json!("rise"), "Rise"), (json!("slide"), "Slide"), (json!("flip"), "Flip"), (json!("pop"), "Pop"), (json!("none"), "None")]));
        }
        if vals["socialAll"].as_bool() != Some(true) {
            f.push(sel("socialSecs", "Each One For", "", &[(json!(5), "5 s"), (json!(8), "8 s"), (json!(12), "12 s"), (json!(20), "20 s"), (json!(30), "30 s")]));
            f.push(sel("socialMotion", "Animation", "How each one comes in and goes out.", &[(json!("rise"), "Rise"), (json!("slide"), "Slide"), (json!("flip"), "Flip"), (json!("pop"), "Pop"), (json!("type"), "Typewriter"), (json!("none"), "None")]));
        }
        f.push(note("In OBS"));
        f.push(info("Address", &url, &url, "Add this address as a Browser source in OBS. About 920 × 180 fits one at a time; give it more width for All at Once."));
        return f;
    }
    if let Some(id) = name.strip_prefix("social:") {
        let m = crate::socials::list(prefs).into_iter().find(|m| m["id"] == id).unwrap_or_default();
        let opts: Vec<(Value, &str)> = crate::socials::PLATFORMS.iter().map(|p| (json!(p.0), p.1)).collect();
        let other = vals["platform"] == "other";
        let mut f = vec![sel("platform", "Platform", "", &opts)];
        if other { f.push(text("name", "Name", "What it's called, shown under your handle.")); }
        f.push(text("handle", "Your Handle", "As you want it shown, like @yourname or yourname.tv."));
        f.push(note("Icon and Colours"));
        match crate::socials::icon_of(&m) {
            Some(file) => {
                f.push(button(&format!("socIcon:{id}"), &format!("Your Icon: {file}"), "Change", ""));
                f.push(button(&format!("socIconClear:{id}"), "", if other { "Use the First Letter" } else { "Use the Platform Icon" }, ""));
            }
            None => f.push(button(&format!("socIcon:{id}"), "Your Own Icon", "Choose...", if other { "Without one, it shows the first letter of its name." } else { "In place of the platform's icon (a PNG with a clear background works best)." })),
        }
        if !other { f.push(tog("ownCol", "Your Own Colours", "When this is off, it uses the platform's colours.")); }
        if other || vals["ownCol"].as_bool() == Some(true) {
            f.push(color("col1", "Colour", ""));
            f.push(color("col2", "Second Colour", "The two make its gradient."));
        }
        f.push(button(&format!("socDel:{id}"), "", "Remove This Platform", ""));
        return f;
    }
    // (an output's page: its address, then the first's page cut down to its look, its sections' headings kept
    // only where something's left under them)
    if let Some((kind, n)) = output_of(name) {
        let port = prefs["widgetPort"].as_u64().unwrap_or(7878);
        let url = if kind == "lyricsw" { format!("http://127.0.0.1:{port}/lyrics/{n}") } else { format!("http://127.0.0.1:{port}/{n}") };
        let keys = look_keys(kind);
        // (the first's whole page, its own look over it: the page reads more than its look)
        let mut full = values(kind, prefs);
        for (k, v) in vals { full.insert(k.clone(), v.clone()); }
        let all = fields(kind, &full, prefs, lan);
        let mut out = vec![info("Address", &url, &url, "Add this address as a Browser source in OBS. It plays the same song as the first, in its own look.")];
        let mut head: Option<Value> = None;
        for f in all {
            let k = f["key"].as_str().unwrap_or("");
            if f["type"] == "note" && f["label"].as_str().map(|l| l.chars().count() <= 32).unwrap_or(false) { head = Some(f); continue; }
            if !k.is_empty() && keys.contains(&k) {
                if let Some(h) = head.take() { out.push(h); }
                out.push(f);
            }
        }
        return out;
    }
    let mut f = fields_of(name, vals, prefs, lan);
    // (the Stream Widget's and its alerts' pages: a pop-out preview first, as the Lyrics Widget's has)
    if matches!(name, "widget" | "alerts" | "alertlist") || name.starts_with("alert:") {
        f.insert(0, button("previewOpen", "Preview", "Pop Out Preview", "Opens a window that shows how it will look. It updates as you change these, before you save."));
    }
    f
}

fn fields_of(name: &str, vals: &Map<String, Value>, prefs: &Value, lan: &str) -> Vec<Value> {
    if name == "alertlist" { return crate::alerts::list_fields(prefs); }
    if let Some(k) = name.strip_prefix("alert:") { return crate::alerts::edit_fields(prefs, k, vals); }
    let streamer = prefs["streamerMode"].as_bool() == Some(true);
    let hide = |key: &str| if streamer { "\u{2022}".repeat(8) } else { key.to_string() };
    let n = |k: &str, d: u64| vals.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let s = |k: &str, d: &str| vals.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    match name {
        // YouTube Music's own: liking there too, and its helpers (how they are, set up or removed)
        "youtube" => {
            let (state, setup, remove) = crate::helpers_state();
            let mut f = vec![tog("ytMirror", "Like Other Tracks There Too", "When your YouTube account is connected, liking a SoundCloud or Spotify track also likes it on YouTube.")];
            if !setup.is_empty() { f.push(button("ytHelpersSetup", "Playback Helpers", setup, &state)); }
            else if remove { f.push(button("ytHelpersRemove", "Playback Helpers", "Remove", &format!("{state} Removed, YouTube Music songs won't play until they're set up again."))); }
            else { f.push(note(&format!("Playback Helpers: {state}"))); }
            f
        }
        "requests" => {
            let port = n("reqPort", 7879);
            let key = prefs["reqKey"].as_str().unwrap_or("").to_string();
            // (the songs it plays, its limits and its replies: the same whichever bot takes the requests)
            let songs = |f: &mut Vec<Value>| {
                f.extend([
                    note("Which Songs"),
                    tog("reqRadioEdit", "Prefer Radio Edits", "Plays a long song's radio edit, if there is one."),
                    tog("reqNoAi", "Keep Out AI Music", "Requests never play AI-made music: artists on Soul Over AI's list of confirmed AI artists (souloverai.com, CC BY 4.0, updated daily), and anything whose title or artist says it's AI, like Suno, Udio or an \"AI cover\". Only whole artist names count, so a few may slip through."),
                    note("Limits"),
                    tog("reqLimits", "Limit Requests", "The most songs waiting, and the most per viewer. When this is off, chat can request as many songs as it likes."),
                    stepper("reqMax", "Most Waiting", "How many requests can wait at once.", counts(1, 50)),
                    stepper("reqPerUser", "Most per Viewer", "How many of one viewer's can wait at once.", counts(1, 10)),
                    button("reqLimitsReset", "", "Reset to Defaults", "On, 10 waiting, 2 per viewer."),
                    button("reqClear", "Waiting Now", "Clear Requests", "Takes the requested songs out of the queue."),
                    note("Chat"),
                    button("repliesOpen", "Chat Replies", "Configure", "What's posted back in chat, in your own words."),
                ]);
            };
            // the OmniPulse Chatbot on: it takes the requests itself, set up here as fully as another bot would be
            // (installed, and switched on)
            if crate::plugin_runs(prefs, crate::botpage::BOT) && crate::bot_manifest().is_some() {
                // (its commands, who and how often: in its own page; here, what's the app's)
                let mut f = Vec::new();
                songs(&mut f);
                return f;
            }
            let bot = s("reqBot", "streamerbot");
            // (a bot no longer listed: the first one)
            let (_, _, cloud, rq, now, wrong, steps) = REQ_BOTS.iter().copied().find(|b| b.0 == bot).unwrap_or(REQ_BOTS[0]);
            // (an online bot reaches it over the internet: your public IP in the address, kept off stream)
            let base = if cloud { format!("http://YOUR-PUBLIC-IP:{port}") } else { format!("http://127.0.0.1:{port}") };
            let fill = |t: &str, k: &str| t.replace("{base}", &base).replace("{key}", k);
            let opts: Vec<(Value, &str)> = REQ_BOTS.iter().map(|b| (json!(b.0), b.1)).collect();
            let chat = rq.starts_with('!');
            let mut f = vec![
                note("Your Bot"),
                sel("reqBot", "Chat Bot", "The bot that answers !sr in your chat. Or turn on the OmniPulse Stream Suite in Plugins, and its chatbot does this for you.", &opts),
                note(steps),
                info(if chat { "Song Request (!sr)" } else { "Request Address (!sr)" }, &fill(rq, &hide(&key)), &fill(rq, &key), "Adds the song a viewer asks for to the queue."),
                info(if chat { "Now Playing (!song)" } else { "Now Playing Address (!song)" }, &fill(now, &hide(&key)), &fill(now, &key), "Says what's playing."),
                info(if chat { "Wrong Song (!wrongsong)" } else { "Wrong Song Address (!wrongsong)" }, &fill(wrong, &hide(&key)), &fill(wrong, &key), "Takes back the viewer's last request. For a moderator, it takes back the last request anyone made."),
            ];
            if bot == "streamerbot" {
                f.push(info("Reply Code (for Execute C# Code)", "Copy it, then paste it into the sub-action", SB_REPLY_CODE, ""));
            }
            // (an online bot: reaching this PC from the internet, the only setup it needs besides the above)
            if cloud {
                f.push(note(&format!("To let it reach this PC:\n1. Turn on Bots on Other Devices (below).\n2. On your router, forward port {port} (TCP) to this PC, {lan}.\n3. In each line above, replace YOUR-PUBLIC-IP with your public IP address (search \"what is my IP\").\nKeep your public IP off stream and out of screenshots. Turn on Streamer Mode while you set this up.")));
            }
            songs(&mut f);
            f.extend([
                note("Connection"),
                num("reqPort", "Port", "1024 to 65535."),
                button("reqNewKey", "Secret Key", "New Key", "A new key: your bot needs the new addresses after."),
            ]);
            if cloud { f.push(remote_bots(vals, "requests", port, "", lan).remove(0)); } else { f.extend(remote_bots(vals, "requests", port, &format!("/request?key={}&user=NAME&q=SONG", hide(&key)), lan)); }
            f
        }
        "widget" => {
            let port = n("widgetPort", 7878);
            let alerts_on = prefs["alertsOn"].as_bool() == Some(true);
            // (in OBS first, then how it looks, the preview above showing it; what it shows; its alerts)
            let mut f = vec![
                note("In OBS"),
                info("Address", &format!("http://127.0.0.1:{port}/"), &format!("http://127.0.0.1:{port}/"), "Add this address as a Browser source in OBS."),
                // (its layout: where the cover and words sit; its theme: its panel, colours and animations. Any
                // theme goes with any layout)
                note("Layout"),
                design("widgetDesign", "Where the Cover and Words Sit"),
                sel("widgetAlign", "Alignment", "", &[(json!("left"), "Left"), (json!("right"), "Right")]),
                tog("widgetRounded", "Rounded Corners", ""),
                stepper("widgetText", "Text Size", "", pct(0.7, 1.6, 0.1)),
                note("Theme"),
                themepick("widgetTheme", "Panel and Colours"),
                tog("widgetOwnCol", "Own Colours", "When this is off, it uses your app theme's accent colours."),
            ];
            if vals["widgetOwnCol"].as_bool() == Some(true) {
                f.push(color("widgetCol1", "Accent Colour", ""));
                f.push(color("widgetCol2", "Second Colour", ""));
            }
            f.extend([
                sel("wColor", "Colour Change", "How quickly it changes to new colours.", &[(json!(0), "Instant"), (json!(0.5), "Fast"), (json!(1.2), "Normal"), (json!(2.5), "Slow")]),
                sel("wEnter", "New Song", "How the next song's name comes in.", &[(json!("fade"), "Fade"), (json!("slide"), "Slide In"), (json!("pop"), "Pop"), (json!("flicker"), "Flicker"), (json!("none"), "None")]),
                tog("wScroll", "Scroll Long Text", "Long names slide along so you can read all of them."),
            ]);
            if s("widgetDesign", "card") == "vinyl" { f.push(tog("wSpin", "Spin the Record", "")); }
            if s("widgetDesign", "card") == "nownext" { f.push(sel("wUpNext", "The Next Song", "Near the End opens it out about 30 seconds before this song ends, to say what's coming.", &[(json!("soon"), "Near the End"), (json!("always"), "Always")])); }
            f.extend([
                note("What It Shows"),
                tog("wRequester", "Who Requested It", "Their name beside the artist, for a requested song."),
                // Alerts: on the widget, so here: a switch, with Configure beside it
                note("Alerts"),
                toggle_cfg("alertsOn", "Alerts", "Follows, subs, bits, raids and more. Click the gear for how they look.", "alertsOpen"),
                tog("alertInWidget", "Show Alerts in This Widget", "They take Now Playing's place for a moment, then it comes back. When this is off, alerts have their own Browser source (on the Alerts page), so you can place them anywhere."),
                num("widgetPort", "Port", "1024 to 65535. Every widget's address uses it."),
            ]);
            f.push(button("widgetHide", "Hide This Track", "Hide", "Until the next track."));
            // (more copies of it, each in a look of its own: another scene, or two on screen at once)
            f.extend(outputs_list(prefs, "widget", &format!("http://127.0.0.1:{port}/")));
            let _ = alerts_on;
            f
        }
        "alerts" => {
            let bot = s("alertBot", "streamerbot");
            let port = prefs["widgetPort"].as_u64().unwrap_or(7878);
            let key = prefs["alertKey"].as_str().unwrap_or("").to_string();
            let vars = if bot == "mixitup" { "type=follow&user=$username" } else { "type=follow&user=NAME&amount=&message=" };
            let part = |k: &str| if vals["botRemote"].as_bool() == Some(true) && !k.is_empty() { format!("key={k}&") } else { String::new() };
            let url = |k: &str| format!("http://127.0.0.1:{port}/alert?{}{vars}", part(k));
            let types = "follow, sub, gift, bits, kicks, donation, raid, hypetrain, poll, ban, watchstreak, liked, game";
            // (the alerts and how they look first; how your bot sends them after, in `setup`)
            let in_widget = vals["alertInWidget"].as_bool() == Some(true);
            let theme = s("alertTheme", "widget");
            let mut look: Vec<Value> = vec![
                tog("alertsOn", "Alerts", "Follows, subs, bits, raids and more, on stream."),
                tog("alertInWidget", "Show Them in the Stream Widget", "They take Now Playing's place for a moment. When this is off, they have a Browser source of their own, below, so you can put them anywhere."),
            ];
            if !in_widget { look.push(info("Alerts Address", &format!("http://127.0.0.1:{port}/alerts"), &format!("http://127.0.0.1:{port}/alerts"), "Add this as a Browser source in OBS, where you'd like your alerts.")); }
            look.extend([
                button("alList", "Alerts and Their Order", "Open", "Which alerts show and which goes first, and each one's words, picture, GIF and sound."),
                button("alSlImport", "Import from Streamlabs", "Import", "Copies your Streamlabs Alert Box into these alerts: each one's picture, sound, message, how long it shows and its colours."),
                note("How They Look"),
                sel("alertTheme", "Theme", "The preview above plays made-up alerts in it. GIF, GIF Pop-Out and Meme are made for GIFs.", &[(json!("widget"), "Widget"), (json!("glass"), "Glass"), (json!("neon"), "Neon"), (json!("banner"), "Banner"), (json!("split"), "Split"), (json!("comic"), "Comic"), (json!("minimal"), "Minimal"), (json!("gif"), "GIF"), (json!("gifpop"), "GIF Pop-Out"), (json!("meme"), "Meme")]),
            ]);
            if matches!(theme.as_str(), "gif" | "gifpop" | "meme") {
                look.push(text("alertGif", "Their GIF", "A GIF's address, for the alerts without a picture of their own (each alert's own is in Alerts and Their Order). On GIPHY or Tenor, right-click a GIF and choose Copy Image Address."));
                if theme == "gifpop" { look.push(note("GIF Pop-Out's GIF stands taller than the alert: leave some room above it in OBS.")); }
            }
            let mut f = vec![sel("alertBot", "Your Bot", "", &[(json!("streamerbot"), "Streamer.bot"), (json!("mixitup"), "Mix It Up"), (json!("generic"), "Generic")])];
            if bot == "streamerbot" {
                f.push(text("sbHost", "Streamer.bot", "The address and port of Streamer.bot's WebSocket server."));
                f.push(tog("sbOn", "Connect to Streamer.bot", ""));
                f.push(note("1. In Streamer.bot, open Servers/Clients > WebSocket Server. Tick Auto Start, then click Start Server.\n2. Enter its address and port here (127.0.0.1:8080 unless you changed it), and turn it on.\n3. That's it. Follows, subs, bits, raids and the rest come through on their own."));
            } else if bot == "mixitup" {
                // one address for each, ready to paste: its event, its type and Mix It Up's own special
                // identifiers already in (its documented ones for each event)
                f.push(note("In Mix It Up, open each event below, add a Web Request action, and paste its address (Copy). Nothing in it needs changing."));
                let at = |k: &str, rest: &str| format!("http://127.0.0.1:{port}/alert?{}type={rest}", part(k));
                for (event, rest) in MIXITUP_EVENTS {
                    f.push(info(event, &at(&hide(&key), rest), &at(&key, rest), ""));
                }
                // (your own alerts: their trigger, the viewer's name)
                for k in crate::alerts::order(prefs) {
                    if crate::alerts::KINDS.iter().any(|x| x.0 == k) { continue; }
                    let rest = format!("{k}&user=$username");
                    f.push(info(&format!("{} (yours)", crate::alerts::name(prefs, &k)), &at(&hide(&key), &rest), &at(&key, &rest), "Any event or command you like."));
                }
                f.push(note("Polls, bans and watch streaks have no Mix It Up event to send them from."));
            } else if bot == "generic" {
                // (as Mix It Up's: one address for each, its type in; only the bot's own variables to put in)
                f.push(note("Have your bot open the matching address below when something happens. Replace the words in capitals with your bot's own variables: NAME is the viewer's name. Fill in MESSAGE and the amounts if your bot has them, or leave them empty."));
                let at = |k: &str, rest: &str| format!("http://127.0.0.1:{port}/alert?{}type={rest}", part(k));
                for (event, rest) in GENERIC_EVENTS {
                    f.push(info(event, &at(&hide(&key), rest), &at(&key, rest), ""));
                }
                for k in crate::alerts::order(prefs) {
                    if crate::alerts::KINDS.iter().any(|x| x.0 == k) { continue; }
                    let rest = format!("{k}&user=NAME");
                    f.push(info(&format!("{} (yours)", crate::alerts::name(prefs, &k)), &at(&hide(&key), &rest), &at(&key, &rest), "Any event or command you like."));
                }
                f.extend(remote_bots(vals, "alerts", port, &format!("/alert?{}{vars}", part(&hide(&key))), lan));
            } else {
                f.push(info("Web Request", &url(&hide(&key)), &url(&key), "Your bot opens this address when something happens."));
                f.push(note(&if bot == "mixitup" {
                    format!("1. In Mix It Up, open the event you want an alert for (a follow, a sub, bits...) and add a Web Request action.\n2. Paste the address above, with type= set to that event: {types}.\n3. Add Mix It Up's special identifiers: user=$username, plus the amount and message for that event. Mix It Up's list of special identifiers has the right names, such as bits or months.")
                } else {
                    format!("1. When something happens, have your bot open the address above.\n2. Replace NAME with your bot's variable for the viewer's name. Fill in amount and message if your bot has them, or leave them empty.\n3. type says what happened: {types}, or one of your own alerts' triggers.")
                }));
                if bot != "mixitup" { f.extend(remote_bots(vals, "alerts", port, &format!("/alert?{}{vars}", part(&hide(&key))), lan)); }
            }
            let setup = f;
            let mut f = std::mem::take(&mut look);
            // (how they come in: from Now Playing, a transition, in the widget; on their own, their animation)
            let anims = [(json!("slide"), "Slide"), (json!("pop"), "Pop"), (json!("fade"), "Fade"), (json!("bounce"), "Bounce"), (json!("flip"), "Flip"), (json!("zoom"), "Zoom"), (json!("spin"), "Spin"), (json!("drop"), "Drop"), (json!("rise"), "Rise"), (json!("unfold"), "Unfold"), (json!("swing"), "Swing"), (json!("glitch"), "Glitch"), (json!("elastic"), "Elastic")];
            f.push(note("How They Come In"));
            if in_widget {
                f.push(sel("alertTrans", "Transition", "From Now Playing to the alert, and back. The preview shows it.", &[(json!("seamless"), "Seamless"), (json!("crossfade"), "Crossfade"), (json!("push"), "Slide Across"), (json!("flip"), "Flip"), (json!("wipe"), "Wipe"), (json!("zoom"), "Zoom Through"), (json!("rise"), "Rise"), (json!("collage"), "Collage"), (json!("overlay"), "Over the Top")]));
            }
            if !in_widget || s("alertTrans", "seamless") == "overlay" {
                f.push(sel("alertAnim", "Animation", "The preview shows it.", &anims));
            }
            if in_widget && s("alertTrans", "seamless") == "collage" {
                f.push(sel("alertCSize", "Collage Tiles", "", &[(json!("small"), "Small"), (json!("medium"), "Medium"), (json!("large"), "Large")]));
                f.push(sel("alertCDir", "Sweep", "", &[(json!("diag"), "Corner to Corner"), (json!("diag-up"), "Corner to Corner, Rising"), (json!("ltr"), "Left to Right"), (json!("rtl"), "Right to Left"), (json!("ttb"), "Top to Bottom"), (json!("btt"), "Bottom to Top"), (json!("centre"), "From the Middle"), (json!("random"), "Scattered")]));
                f.push(range("alertCAngle", "Angle", "The tiles' tilt, in degrees.", -90.0, 90.0, 5.0, "°", true));
            }
            f.push(note("Colours"));
            f.push(tog("alertOwnCol", "Their Own Colours", "When this is off, alerts use the album art's colours, like the widget does."));
            if vals["alertOwnCol"].as_bool() == Some(true) {
                f.push(color("alertCol1", "Accent Colour", ""));
                f.push(color("alertCol2", "Second Colour", ""));
                f.push(sel("alertDir", "Gradient", "", &[(json!("115deg"), "Diagonal"), (json!("90deg"), "Left to Right"), (json!("180deg"), "Top to Bottom"), (json!("65deg"), "Diagonal, Rising"), (json!("glow"), "Glow")]));
            }
            f.push(note("Time and Sound"));
            if !matches!(theme.as_str(), "meme") { f.push(stepper("alertIcon", if matches!(theme.as_str(), "gif" | "gifpop") { "GIF Size" } else { "Icon Size" }, "", pct(0.6, 2.0, 0.2))); }
            f.push(sel("alertSecs", "Show For", "Each alert can have its own time too, in Alerts and Their Order.", &[(json!(3), "3 s"), (json!(5), "5 s"), (json!(8), "8 s"), (json!(12), "12 s")]));
            f.push(range("alertVol", "Sound Volume", "", 0.0, 1.0, 0.01, "%", false));
            // (the Stream Suite sends its own: no other bot to set up, so none of this)
            if !crate::SHARED.with(|s| s.borrow().as_ref().is_some_and(|sh| crate::plugin_runs(&sh.borrow().web.prefs, crate::botpage::BOT))) {
                f.push(note("Where They Come From"));
                f.extend(setup);
            }
            f
        }
        // a bug or an idea, sent to the developer through the relay; anything needing pictures, to Discord
        "report" => {
            let bug = s("reportKind", "bug") == "bug";
            vec![
                sel("reportKind", "What It Is", "", &[(json!("bug"), "Something's Wrong (a Bug)"), (json!("feature"), "An Idea (a Feature)")]),
                text("reportTitle", "In a Few Words", if bug { "Like: the queue skips a song after a crossfade." } else { "Like: a sleep timer in the player." }),
                area("reportText", if bug { "What Happened" } else { "Your Idea" }, if bug { "What you did, what you expected, and what happened instead. The more steps, the easier it is to fix." } else { "What you'd like, and what it'd help you do." }),
                text("reportContact", "Your Discord or Email (If You Like)", "So the developer can ask you about it. Leave it empty to stay anonymous."),
                tog("reportInfo", "Include My App's Details", "Your OmniPulse version, Windows version, player style and which plugins are on. Nothing else: no account names, songs or files."),
                button("reportSend", "Send It", "Send", "Sent to OmniPulse's developer."),
                note(if bug { "For a complicated bug, or one you'd need to show a picture or video of, message @shwylie on Discord instead: you can share screenshots there." } else { "Want to talk it through, or show a picture of what you mean? Message @shwylie on Discord." }),
                info("Discord", "@shwylie", "shwylie", "Click Copy, then add them on Discord."),
            ]
        }
        // Discord Status: what it shows (as the web app's)
        "discord" => vec![
            tog("presArt", "Cover Art", "The cover of what's playing."),
            tog("presTime", "Time", "How far through the song you are."),
            tog("presListen", "Listen Along Button", "Friends can listen along with you from it."),
            tog("presAsk", "Ask to Jam Button", "Friends can ask you to start a Jam from it."),
            tog("presLink", "Open Track Button", "A button that opens the song on its platform."),
        ],
        // Listen Along & Jams: the name the others see
        "together" => vec![
            text("togetherName", "Your Name", "The name others see when you listen along or Jam. Leave it empty to use your account's name."),
        ],
        // Lyrics: where they come from, in order (as the web app's)
        "lyrics" => lyrics_fields(vals),
        "replies" => {
            let mut f = vec![note("What your bot posts back for each one. Leave it empty to use the default shown under it.\nYou can use: {title} {artist} {user} {position} {platform} {query} {count} {by}")];
            for (k, label) in crate::requests::MSG_KEYS { f.push(text(&format!("msg:{k}"), label, crate::requests::default_msg(k))); }
            f.push(button("repliesReset", "", "Reset to Defaults", "Every reply back to its default (Save keeps it)."));
            f
        }
        // the lyrics widget's look (an OBS Browser source at /lyrics on the widget's port)
        "lyricsw" => {
            // (the Stream Widget's port: its setting, not this page's)
            let port = prefs["widgetPort"].as_u64().unwrap_or(7878);
            let url = format!("http://127.0.0.1:{port}/lyrics");
            let dirs = [(json!("90deg"), "Left to right"), (json!("180deg"), "Top to bottom"), (json!("135deg"), "Diagonal"), (json!("270deg"), "Right to left"), (json!("0deg"), "Bottom to top")];
            let on = |k: &str| vals[k].as_bool() == Some(true);
            let mut f = vec![
                note("In OBS"),
                info("Address", &url, &url, "Add this address as a Browser source in OBS. About 1280 × 400 fits 3 lines. It uses the Stream Widget's port."),
                button("previewOpen", "Preview", "Pop Out Preview", "Opens a window that shows how it will look. It updates as you change these, before you save."),
                note("How It Looks"),
                sel("lyricsTheme", "Theme", "Sets the font, colours, outline and box at once. Change any of them after and it's Custom.", &[(json!("custom"), "Custom"), (json!("neon"), "Neon Karaoke"), (json!("clean"), "Clean White"), (json!("outline"), "Bold Outline"), (json!("glass"), "Glass Box"), (json!("retro"), "Retro"), (json!("minimal"), "Minimal")]),
                sel("lyricsStyle", "Style", "Karaoke fills each line in as it's sung. Highlight lights up the whole line at once. Plain only makes it brighter.", &[(json!("karaoke"), "Karaoke"), (json!("highlight"), "Highlight"), (json!("plain"), "Plain")]),
                sel_owned("lyricsFont", "Font", "Those marked (web) come from Google Fonts.", FONTS.iter().map(|(k, n)| (json!(k), n.to_string())).collect()),
                stepper("lyricsSize", "Text Size", "In pixels.", (16..=120).step_by(4).map(|x| (json!(x), format!("{x}px"))).collect()),
                sel("lyricsTransform", "Text Transform", "", &[(json!("none"), "None"), (json!("uppercase"), "Uppercase"), (json!("lowercase"), "Lowercase"), (json!("capitalize"), "Start Case")]),
                sel("lyricsAlign", "Alignment", "", &[(json!("center"), "Centre"), (json!("left"), "Left"), (json!("right"), "Right")]),
                sel("lyricsPlace", "Position", "Where the lines sit in the source.", &[(json!(""), "Middle"), (json!("top"), "Top"), (json!("bottom"), "Bottom")]),
                note("Colours"),
                color("lyricsColor", "Text Colour", ""),
                tog("lyricsTextGrad", "Text Gradient", "From the text colour to a second one."),
            ];
            if on("lyricsTextGrad") { f.push(color("lyricsColor2", "Text Gradient's Second Colour", "")); }
            f.push(tog("lyricsOwnHi", "Own Highlight Colour", "When this is off, it uses the app's accent colour (from the album art, if that's on)."));
            if on("lyricsOwnHi") {
                f.push(color("lyricsHi", "Highlight Colour", "The words being sung."));
                f.push(tog("lyricsHiGrad", "Highlight Gradient", "From the highlight colour to a second one."));
                if on("lyricsHiGrad") { f.push(color("lyricsHi2", "Highlight Gradient's Second Colour", "")); }
            }
            if on("lyricsTextGrad") || (on("lyricsOwnHi") && on("lyricsHiGrad")) { f.push(sel("lyricsGradDir", "Gradient Direction", "For the text's and the highlight's.", &dirs)); }
            f.push(range("lyricsDim", "Other Lines", "How bright the lines not being sung are.", 0.0, 1.0, 0.05, "%", false));
            f.push(tog("lyricsShadow", "Shadow", "Keeps it readable over the game."));
            f.push(tog("lyricsOutline", "Outline", "A line round each letter, as OBS's text outline."));
            if on("lyricsOutline") {
                f.push(stepper("lyricsOutlineSize", "Outline Size", "", (1..=20).map(|x| (json!(x), format!("{x}px"))).collect()));
                f.push(color("lyricsOutlineColor", "Outline Colour", ""));
                f.push(range("lyricsOutlineOpacity", "Outline Opacity", "", 0.0, 1.0, 0.05, "%", false));
            }
            f.push(tog("lyricsBox", "Background Box", "A box behind the lines."));
            if on("lyricsBox") {
                f.push(color("lyricsBoxColor", "Box Colour", ""));
                f.push(tog("lyricsBoxGrad", "Box Gradient", "From the box colour to a second one."));
                if on("lyricsBoxGrad") { f.push(color("lyricsBoxColor2", "Box Gradient's Second Colour", "")); f.push(sel("lyricsBoxDir", "Box Gradient Direction", "", &dirs)); }
                f.push(range("lyricsBoxOpacity", "Box Opacity", "", 0.0, 1.0, 0.05, "%", false));
            }
            f.extend([
                note("Lines"),
                stepper("lyricsLines", "Lines Shown", "The one being sung and the next ones.", counts(1, 9)),
                tog("lyricsBefore", "Show the Line Before", "The line just sung, above the one being sung."),
                note("Scrolling"),
                sel("lyricsScroll", "Scrolling", "How the lines move on as each one's sung.", &[(json!("smooth"), "Smooth (slides up)"), (json!("jump"), "Jump (straight to the next line)")]),
                stepper("lyricsScrollMs", "Scroll Speed", "How long each slide takes.", (100..=1500).step_by(100).map(|x| (json!(x), format!("{:.1} s", x as f64 / 1000.0))).collect()),
                sel("lyricsScrollPlace", "Sung Line Sits At", "", &[(json!("top"), "At the top, with the next lines below"), (json!("middle"), "In the middle, with sung lines above")]),
                tog("lyricsFadeEdges", "Fade the Edges", "Lines dissolve in at the bottom and out at the top, instead of being cut off."),
                note("What Shows, and When"),
                tog("lyricsTitle", "Track Info", "Shows the song and artist above the lines. When this is off, only the lyrics show."),
                tog("lyricsHideNone", "Hide When There Are No Lyrics", "Shows nothing for songs without timed lyrics, not even the track info."),
                tog("lyricsHidePaused", "Hide While Paused", ""),
                note("The lines fade in a moment before their first words are sung, out after the last, and out over a long instrumental part."),
                note("Timing"),
                num("lyricsOffset", "Timing", "In milliseconds, from -5000 to 5000. Use a positive number to show lyrics later, or a negative one to show them earlier. Match it to your stream delay, or to a song whose lyrics run early."),
            ]);
            f.extend(outputs_list(prefs, "lyricsw", &format!("http://127.0.0.1:{port}/lyrics/")));
            f
        }
        // Smart Transitions' own page
        "smart" => vec![
            note("Seamless Mix is still being built, so some transitions may not sound right yet. Rating them on your Stream Deck (Mix Good or Mix Bad) helps."),
            sel("smartMax", "Longest Fade", "The most the two songs play over each other. Each pair gets as long as the shorter of the song playing's outro and the next one's intro, up to this.", &[(json!(6), "6 s"), (json!(8), "8 s"), (json!(10), "10 s"), (json!(12), "12 s"), (json!(16), "16 s"), (json!(20), "20 s")]),
            tog("smartBlend", "Beatmatched Blends", "Songs with a clear beat, close in tempo (within 4%) and in matching keys, blend on the beat. The next song plays at the current song's tempo without changing pitch, starts on a phrase, and takes over the bass on a bar line. When this is off, songs always fade."),
            tog("smartKeys", "Match Keys", "When two songs' keys clash, the next song comes in shifted to the current song's key. Both then move to the next song's own key during the blend, so they stay in key the whole time. When this is off, keys aren't shifted and clashes are kept short."),
            tog("smartVoices", "Vocal Handoff", "Stops two songs' vocals from overlapping. The current song's voice drops out as the next song's voice comes in. It works like a karaoke switch, so it can also quiet other sounds in the centre of the mix. Bass and low drums aren't affected."),
            tog("smartRoll", "Rolls", "For songs too different to blend, the current song's last bar loops, halving each time (a bar, a half, a quarter, an eighth) to build up. The next song then lands on its first beat at its own tempo. It turns itself off if most of your ratings for it (at least 5) are bad."),
            tog("smartTape", "Tape Stops", "For songs too different to blend, the current song slows to a stop and the next one spins up. It only starts once most of your ratings for it (at least 5) are good."),
            tog("shareRatings", "Send My Ratings to the Developer", "Each time you open OmniPulse, your new Mix Good and Mix Bad ratings go to OmniPulse's developer, to help tune Seamless Mix. Each one has the two songs' names and how they were mixed. Nothing about your accounts is sent."),
        ],
        // Limit Requests' own page (as the web app's, under it)
        "limits" => vec![
            stepper("reqMax", "Most Waiting", "How many requests can wait at once.", counts(1, 50)),
            stepper("reqPerUser", "Most per Viewer", "How many of one viewer's can wait at once.", counts(1, 10)),
            button("reqClear", "Waiting Now", "Remove All", "Takes the requested songs out of the queue."),
            button("reqLimitsReset", "", "Reset to Defaults", "10 waiting, 2 per viewer."),
        ],
        "remote" => {
            let port = n("remotePort", 7880);
            let key = prefs["remoteKey"].as_str().unwrap_or("").to_string();
            let url = |k: &str| format!("http://127.0.0.1:{port}/control?key={k}&action=toggle");
            vec![
                info("Address", &url(&hide(&key)), &url(&key), "The Stream Deck plugin finds it by itself; other devices use this address (action: toggle, next, previous, like...)."),
                num("remotePort", "Port", "1024 to 65535."),
                button("remoteNewKey", "Secret Key", "New Key", "A new key: anything using the old address needs the new one."),
                // (the plugin itself: the app's own, given to Stream Deck; kept up to date as the app starts)
                {
                    let have = crate::streamdeck::installed();
                    let ours = crate::streamdeck::version();
                    let (label, note) = match (crate::streamdeck::present(), have.as_deref()) {
                        (false, _) => ("Install", "Stream Deck isn't installed on this PC. Install it first, from elgato.com."),
                        (true, None) => ("Install", "Adds OmniPulse's keys to Stream Deck. Stream Deck asks you to confirm."),
                        (true, Some(v)) if crate::streamdeck::newer(ours, v) => ("Update", "A newer version came with the app. It updates by itself as OmniPulse starts, or here now."),
                        (true, Some(_)) => ("Install Again", "Up to date. It's kept up to date as the app updates."),
                    };
                    button("sdInstall", "Stream Deck Plugin", label, note)
                },
            ]
        }
        "hidden" => crate::hide::fields(prefs),
        "safe" => crate::streamsafe::panel_fields(prefs),
        "plugins" => plugin_fields(prefs),
        "offset" => vec![
            num("out2Offset", "Offset", "In milliseconds, from -5000 to 5000. How much later OmniPulse Offset plays than what you hear. Use a negative number for earlier."),
            range("out2Volume", "Volume", "", 0.0, 1.0, 0.01, "%", false),
            button("out2Restart", "Not Being Heard?", "Restart OmniPulse Offset", "Starts OmniPulse Offset again, fresh, without stopping your music."),
        ],
        _ => vec![
            sel("recentWindow", "Time Frame", "What counts as heard lately.", &[(json!("day"), "Past day"), (json!("week"), "Past week"), (json!("month"), "Past month")]),
        ],
    }
}

// ---------- Plugins: adding one (the web app's js/plugins.js), and the ones you've added ----------
thread_local! {
    /// the plugin read from what was given, waiting to be added (or why it couldn't be read)
    pub static PLUG_PENDING: std::cell::RefCell<Option<Result<omnipulse_core::plugins::Package, String>>> = const { std::cell::RefCell::new(None) };
}

/// What a plugin will and won't be able to do (the web app's plugAccess)
pub fn plug_access(m: &omnipulse_core::plugins::Manifest) -> (Vec<String>, Vec<String>, Vec<String>) {
    let (mut can, mut cannot, mut warn) = (Vec::new(), Vec::new(), Vec::new());
    let caps: std::collections::HashSet<&str> = m.capabilities.iter().map(|s| s.as_str()).collect();
    let local: Vec<&String> = m.hosts.iter().filter(|h| *h == "127.0.0.1" || *h == "localhost").collect();
    let mut bare: Vec<String> = Vec::new();
    for h in m.hosts.iter().filter(|h| !local.contains(h)) {
        let b = h.trim_start_matches("*.").to_string();
        if !bare.contains(&b) { bare.push(b); }
    }
    let web: Vec<String> = bare.iter().map(|h| if m.hosts.iter().any(|x| *x == format!("*.{h}")) { format!("{h} (any part of it)") } else { h.clone() }).collect();
    if caps.contains("search") || caps.is_empty() { can.push("Show its songs in your search results, so it sees what you search for".into()); }
    if caps.contains("play") || caps.is_empty() { can.push("Play its songs, from its own sites, and know when you play one".into()); }
    if caps.contains("library") { can.push("Add its saved songs to your Library".into()); }
    if caps.contains("sockets") { can.push("Keep live connections open to its sites (a chat, its events)".into()); }
    if caps.contains("app") { warn.push("Lets the plugin use the app: add songs to your queue as requests, show alerts, read what's playing, open its sign-in pages in your browser, and keep its own settings. Only allow this for plugins you trust.".into()); }
    if !web.is_empty() { can.push(format!("Connect to {}", web.join(", "))); }
    if !local.is_empty() { warn.push(format!("It can connect to programs running on this PC ({}). Only allow this if you know what it talks to.", local.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))); }
    let sets = m.settings.as_array().cloned().unwrap_or_default();
    if sets.iter().any(|x| x["type"] == "password") { can.push("Saves a password or key you give it on this PC. The plugin can send it to the sites above.".into()); }
    else if !sets.is_empty() { can.push("Keep its own settings, on this PC".into()); }
    cannot.push(if m.hosts.is_empty() { "Connect to the internet".into() } else { "Connect to any other site".into() });
    cannot.extend(["See your accounts or sign-ins (SoundCloud, Spotify, YouTube Music, Last.fm, Discord)", "Open, read or change your files", "See your playlists, Library, listening history or settings", "Change anything else in the app"].map(String::from));
    (can, cannot, warn)
}

fn plugin_fields(_prefs: &Value) -> Vec<Value> {
    let mut f = vec![
        note("Music sources made by others. Each is walled off and only reaches the sites it lists."),
        text("plugSource", "Add a Plugin", "The plugin's manifest.json, as a web address or a file on this PC."),
        button("plugFile", "", "Choose File...", ""),
        button("plugCheck", "", "Read It", "Shows what it would be able to do, before anything's added."),
    ];
    PLUG_PENDING.with(|p| match &*p.borrow() {
        Some(Ok(pkg)) => {
            let m = &pkg.manifest;
            let have = omnipulse_core::plugins::list().into_iter().find(|x| x.manifest.id == m.id);
            let (can, cannot, warn) = plug_access(m);
            let kb = (pkg.code.len() / 1024).max(1);
            let about: Vec<String> = [(!m.version.is_empty()).then(|| format!("Version {}", m.version)), (!m.author.is_empty()).then(|| format!("by {}", m.author)), Some(format!("{kb} KB"))].into_iter().flatten().collect();
            f.push(note(&format!("Add {}?\n{}{}", m.name, about.join(" \u{b7} "), if m.description.is_empty() { String::new() } else { format!("\n{}", m.description) })));
            f.push(note(&format!("It will be able to:\n{}", can.iter().map(|c| format!("\u{2022} {c}")).collect::<Vec<_>>().join("\n"))));
            f.push(note(&format!("It won't be able to:\n{}", cannot.iter().map(|c| format!("\u{2022} {c}")).collect::<Vec<_>>().join("\n"))));
            for w in warn { f.push(note(&format!("\u{26a0} {w}"))); }
            f.push(note(&format!("It runs walled off from the rest of OmniPulse, and you can turn it off or remove it any time here.{}", have.map(|h| format!(" Adding it replaces the {} version you have.", if h.manifest.version.is_empty() { "installed".to_string() } else { h.manifest.version.clone() })).unwrap_or_default())));
            f.push(button("plugInstall", "", if omnipulse_core::plugins::list().iter().any(|x| x.manifest.id == m.id) { "Update Plugin" } else { "Add Plugin" }, ""));
        }
        Some(Err(e)) => f.push(note(&format!("\u{26a0} {e}"))),
        None => {}
    });
    // each one's files (its manifest.json and main.js), to read what it does
    let all = omnipulse_core::plugins::list();
    if !all.is_empty() { f.push(note("Their files:")); }
    for p in &all { f.push(button(&format!("plugFiles:{}", p.manifest.id), &p.manifest.name, "Show Files", "Opens the plugin's folder, with manifest.json and main.js.")); }
    // the ones you've added (those that come with OmniPulse stay)
    let added: Vec<omnipulse_core::plugins::Package> = omnipulse_core::plugins::list().into_iter().filter(|p| !p.bundled).collect();
    if !added.is_empty() { f.push(note("Added by you:")); }
    for p in added {
        f.push(button(&format!("plugRemove:{}", p.manifest.id), &p.manifest.name, "Remove", &p.manifest.description));
    }
    f
}

/// A value as typed in, for its key: numbers within their range (else left as it was)
pub fn typed(key: &str, text: &str) -> Option<Value> {
    let t = text.trim();
    let range = |lo: f64, hi: f64| t.parse::<f64>().ok().filter(|n| *n >= lo && *n <= hi).map(|n| json!(n.round() as i64));
    match key {
        "reqPort" | "widgetPort" | "remotePort" => range(1024.0, 65535.0),
        "reqMax" => range(1.0, 50.0),
        "reqPerUser" => range(1.0, 10.0),
        "out2Offset" => range(-5000.0, 5000.0),
        "alertCAngle" => range(-90.0, 90.0),
        _ => Some(json!(t)),
    }
}

/// A new secret key (24 hex digits, as the web app makes them)
pub fn new_key() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut out = String::new();
    for i in 0..3u64 {
        let mut h = std::collections::hash_map::RandomState::new().build_hasher();
        h.write_u64(i);
        h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
        out.push_str(&format!("{:016x}", h.finish()));
    }
    out.truncate(24);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn an_output_has_its_own_look() {
        let prefs = json!({ "widgetDesign": "card", "widgetOutputs": [{ "widgetDesign": "vinyl" }] });
        let v = values("widget@2", &prefs);
        assert_eq!(v["widgetDesign"], "vinyl");
        assert_eq!(title("widget@2"), "Stream Widget, Output 2");
        let f = fields("widget@2", &v, &prefs, "");
        assert!(f[0]["value"].as_str().unwrap().ends_with("/2"));
        assert!(f.iter().any(|x| x["key"] == "widgetDesign") && !f.iter().any(|x| x["key"] == "widgetPort"));
        assert!(fields("widget", &values("widget", &prefs), &prefs, "").iter().any(|x| x["type"] == "output"));
    }

    #[test]
    fn panels_start_from_prefs_and_defaults() {
        // (the chatbot off: another bot's lines to paste)
        let prefs = json!({ "reqBot": "mixitup", "reqKey": "abc", "widgetText": 1.2, "plugOn_chatbot": false });
        let v = values("requests", &prefs);
        assert_eq!(v["reqBot"], "mixitup");
        assert_eq!(v["reqMax"], 10);
        let f = fields("requests", &v, &prefs, "192.168.1.2");
        let addr = f.iter().find(|x| x["type"] == "info").unwrap();
        assert!(addr["copy"].as_str().unwrap().contains("key=abc&user=$username&q=$allargs"));
        assert!(same(&json!(1), &json!(1.0)));
        assert_eq!(typed("reqMax", "70"), None);
        assert_eq!(typed("reqMax", "12"), Some(json!(12)));
        assert_eq!(new_key().len(), 24);
        // Streamer Mode: the key hidden where it's shown, copied whole
        let f = fields("requests", &v, &json!({ "reqKey": "abc", "streamerMode": true, "plugOn_chatbot": false }), "");
        let addr = f.iter().find(|x| x["type"] == "info").unwrap();
        assert!(!addr["value"].as_str().unwrap().contains("abc") && addr["copy"].as_str().unwrap().contains("abc"));
        // (a bot no longer listed: the first one's lines)
        let f = fields("requests", &values("requests", &json!({ "reqBot": "moobot" })), &json!({ "reqBot": "moobot", "plugOn_chatbot": false }), "");
        assert!(f.iter().any(|x| x["type"] == "info" && x["copy"].as_str().unwrap_or("").contains("%rawInputUrlEncoded%")));
        // (the chatbot's own settings among its values, its defaults where it has none)
        assert_eq!(values("requests", &prefs)["bot:songCommands"], "!song");
        // the alerts' bot follows the requests' one until it's picked
        assert_eq!(values("alerts", &prefs)["alertBot"], "mixitup");
    }

    #[test]
    fn any_other_bot_gets_an_address_for_each_alert() {
        let prefs = json!({ "alertBot": "generic", "alertKey": "k" });
        let f = fields("alerts", &values("alerts", &prefs), &prefs, "");
        let addrs: Vec<&str> = f.iter().filter(|x| x["type"] == "info").filter_map(|x| x["copy"].as_str()).collect();
        // (the alerts on their own, as they start: their own address too)
        assert_eq!(addrs.len(), 12);
        assert!(addrs.contains(&"http://127.0.0.1:7878/alerts"));
        assert!(addrs.contains(&"http://127.0.0.1:7878/alert?type=raid&user=NAME&amount=VIEWERS"));
    }
}
