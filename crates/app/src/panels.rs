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

pub fn title(name: &str) -> String {
    if name == "alertlist" { return "Alerts and Their Order".into(); }
    if let Some(k) = name.strip_prefix("alert:") { return format!("Alert: {}", crate::alerts::name(&Value::Null, k)); }
    match name {
        "requests" => "Song Requests",
        "replies" => "Chat Replies",
        "limits" => "Limit Requests",
        "mix" => "Seamless Mix (WIP)",
        "smart" => "Smart Transitions (WIP)",
        "discord" => "Discord Status",
        "together" => "Listen Along & Jams",
        "lyrics" => "Lyrics",
        "widget" => "Stream Widget",
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
    Some(match key { "streamSafe" => "safe", "req" => "requests", "reqLimits" => "limits", "mixOn" => "mix", "smartOn" => "smart", "presence" => "discord", "jamsOn" => "together", "lyricsSources" => "lyrics", "widget" => "widget", "alertsOn" => "alerts", "remoteOn" => "remote", "out2On" => "offset", "avoidRecent" => "recent", "hideOn" => "hidden", "plugAdd" => "plugins", _ => return None })
}

/// What a panel edits, and each one's default (the web app's)
fn keys(name: &str) -> Vec<(&'static str, Value)> {
    match name {
        "limits" => vec![("reqMax", json!(10)), ("reqPerUser", json!(2))],
        "discord" => vec![("presArt", json!(true)), ("presTime", json!(true)), ("presListen", json!(true)), ("presAsk", json!(false)), ("presLink", json!(true))],
        "together" => vec![("togetherName", json!(""))],
        "smart" => vec![("smartBlend", json!(true)), ("smartMax", json!(10)), ("smartTape", json!(false)), ("smartRoll", json!(false)), ("smartVoices", json!(true))],
        "mix" => vec![("mixWords", json!(true)), ("mixDrop", json!(true)), ("mixMelody", json!(true)), ("mixTape", json!(true)), ("mixBars", json!(0)), ("mixBands", json!(false)), ("mixKeys", json!(true)), ("mixLoops", json!(true)), ("mixForce", json!("auto"))],
        "requests" => vec![("reqBot", json!("streamerbot")), ("reqRadioEdit", json!(false)), ("reqMax", json!(10)), ("reqPerUser", json!(2)), ("reqPort", json!(7879)), ("botRemote", json!(false))],
        "widget" => vec![("widgetPort", json!(7878)), ("widgetDesign", json!("card")), ("widgetAlign", json!("left")), ("widgetRounded", json!(true)), ("widgetText", json!(1)), ("wFade", json!(true)), ("wScroll", json!(true)), ("wSpin", json!(true)), ("wRequester", json!(true)), ("wColor", json!(1.2))],
        "alerts" => vec![("alertBot", json!("streamerbot")), ("sbHost", json!("127.0.0.1:8080")), ("sbOn", json!(true)), ("alertTrans", json!("seamless")), ("alertCSize", json!("medium")), ("alertCDir", json!("diag")), ("alertCAngle", json!(0)), ("alertAnim", json!("slide")), ("alertOwnCol", json!(false)), ("alertCol1", json!("#8e9bff")), ("alertCol2", json!("#b38cff")), ("alertDir", json!("115deg")), ("alertIcon", json!(1)), ("alertSecs", json!(5)), ("alertVol", json!(0.8)), ("botRemote", json!(false))],
        "remote" => vec![("remotePort", json!(7880))],
        "offset" => vec![("out2Offset", json!(0)), ("out2Volume", json!(1))],
        "hidden" => vec![],
        "plugins" => vec![("plugSource", json!(""))],
        _ => vec![("recentWindow", json!("week"))],
    }
}

/// Lyrics' sources: asked in this order, the first with timed lyrics used; each switchable, moved up or
/// down (in the panel's values: "order", the ids, and "on:<id>" each)
fn lyrics_fields(vals: &Map<String, Value>) -> Vec<Value> {
    let order: Vec<String> = vals.get("order").and_then(|o| o.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    let n = order.len();
    let mut f = vec![note("Where lyrics come from, asked in this order: the first with timed lyrics is used. In Now Playing, a song's lyrics can switch to another of these.")];
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
    if name == "safe" { return crate::streamsafe::panel_values(prefs); }
    if let Some(k) = name.strip_prefix("alert:") { return crate::alerts::edit_values(prefs, k); }
    let mut m = Map::new();
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

fn tog(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "toggle" }) }
fn num(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "number" }) }
fn text(key: &str, label: &str, note: &str) -> Value { json!({ "key": key, "label": label, "note": note, "type": "text" }) }
fn sel(key: &str, label: &str, note: &str, opts: &[(Value, &str)]) -> Value {
    json!({ "key": key, "label": label, "note": note, "type": "select", "options": opts.iter().map(|o| o.0.clone()).collect::<Vec<_>>(), "labels": opts.iter().map(|o| o.1).collect::<Vec<_>>() })
}
fn info(label: &str, value: &str, copy: &str, note: &str) -> Value { json!({ "key": "", "label": label, "note": note, "type": "info", "value": value, "copy": copy }) }
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
    let mut out = vec![tog("botRemote", "Bots on Other Devices", &format!("Off: only programs on this PC can reach these addresses. On: other devices can too{}", if what == "alerts" { " (with the key in their address)." } else { " (with the key)." }))];
    if vals["botRemote"].as_bool() == Some(true) {
        out.push(note(&format!("1. A bot on another computer at home: give it http://{lan}:{port}{path} in place of 127.0.0.1.\n2. An online bot (one that isn't on your network): on your router, forward port {port} (TCP) to this PC, {lan}. Then give the bot the same address with your public IP address in place of {lan}.\nCareful: your public IP address points to your home. Don't show it on stream or in screenshots, and don't share it; keep Streamer Mode on while you set this up. With it and the key, anyone could {}.", if what == "alerts" { "send alerts to your stream" } else { "request songs" })));
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
    if name == "alertlist" { return crate::alerts::list_fields(prefs); }
    if let Some(k) = name.strip_prefix("alert:") { return crate::alerts::edit_fields(prefs, k, vals); }
    let streamer = prefs["streamerMode"].as_bool() == Some(true);
    let hide = |key: &str| if streamer { "\u{2022}".repeat(8) } else { key.to_string() };
    let n = |k: &str, d: u64| vals.get(k).and_then(|v| v.as_u64()).unwrap_or(d);
    let s = |k: &str, d: &str| vals.get(k).and_then(|v| v.as_str()).unwrap_or(d).to_string();
    match name {
        "requests" => {
            let bot = s("reqBot", "streamerbot");
            let port = n("reqPort", 7879);
            let key = prefs["reqKey"].as_str().unwrap_or("").to_string();
            let (u, q, m) = match bot.as_str() { "mixitup" => ("$username", "$allargs", "$userroles"), "other" => ("USERNAME", "SONG", "MOD"), _ => ("%user%", "%rawInputUrlEncoded%", "%isModerator%") };
            let base = format!("http://127.0.0.1:{port}");
            let req = |k: &str| format!("{base}/request?key={k}&user={u}&q={q}");
            let wrong = |k: &str| format!("{base}/wrongsong?key={k}&user={u}&mod={m}");
            let steps = match bot.as_str() {
                "mixitup" => "1. Create a Chat Command such as !sr.\n2. Add a Web Request action, paste the request address, and set the response to plain text.\n3. Add a Chat action that sends $webrequestresult.\n4. For !song: the same again with the now playing address.\n5. For !wrongsong: the same again with the wrong song address ($userroles tells it who's a moderator). A viewer takes back their last request; a moderator, or you, takes back the last one anyone made.\nIf $webrequestresult doesn't post anything in your version, check Mix It Up's list of special identifiers for the web request result.",
                "other" => "Any bot that can call a web address and post the reply in chat will work:\n1. Call /request with key (required, from here), q (what was asked for: search words or a SoundCloud or Spotify link) and user (who asked; used for the per-viewer limit and \"requested by\").\n2. Text in q and user must be URL-encoded (spaces as %20). Most bots have an encoded version of the message variable.\n3. The reply is one line of plain text, ready to post in chat. Add &format=json to get JSON instead: ok, message, title, artist, url and position.\n4. Call /now for the current song (no key needed). It also supports &format=json.\n5. Call /wrongsong with key, user and mod (true for a moderator, or a list of their roles with Moderator or Streamer in it) to take back that viewer's last request; a moderator's takes back the last one anyone made.",
                _ => "1. Create an Action called Song request, with a Command trigger such as !sr.\n2. Add the sub-action Core > Network > Fetch URL, paste the request address, and set its variable name to reply.\n3. Add the sub-action Twitch > Chat > Send Message to Channel with the message %reply%.\n4. For !song: the same again with the now playing address.\n5. For !wrongsong: the same again with the wrong song address. A viewer takes back their last request; a moderator takes back the last one anyone made.\nIf your version names things slightly differently, look for the sub-action that fetches a URL and saves the reply in a variable.",
            };
            let mut f = vec![
                sel("reqBot", "Chat Bot", "", &[(json!("streamerbot"), "Streamer.bot"), (json!("mixitup"), "Mix It Up"), (json!("other"), "Generic")]),
                info(match bot.as_str() { "mixitup" => "Web Request Address (special identifiers already filled in)", "other" => "Request Address (replace USERNAME and SONG with your bot's variables)", _ => "Fetch URL Address (variables already filled in)" }, &req(&hide(&key)), &req(&key), ""),
                info("Now Playing Address (!song)", &format!("{base}/now"), &format!("{base}/now"), ""),
                info("Wrong Song Address (!wrongsong)", &wrong(&hide(&key)), &wrong(&key), ""),
                note(steps),
                tog("reqRadioEdit", "Prefer Radio Edits", "Plays a long song's radio edit, if there is one."),
                button("reqClear", "Waiting Now", "Clear Requests", "Takes the requested songs out of the queue."),
                button("repliesOpen", "Chat Replies", "Configure", "What your bot posts back in chat, in your own words."),
                num("reqPort", "Port", "1024 to 65535."),
                button("reqNewKey", "Secret Key", "New Key", "A new key: your bot needs the new addresses after."),
            ];
            f.extend(remote_bots(vals, "requests", port, &format!("/request?key={}&user=USERNAME&q=SONG", hide(&key)), lan));
            f
        }
        "widget" => {
            let port = n("widgetPort", 7878);
            let alerts_on = prefs["alertsOn"].as_bool() == Some(true);
            let f = vec![
                info("Address", &format!("http://127.0.0.1:{port}/"), &format!("http://127.0.0.1:{port}/"), "In OBS: a Browser source with this address."),
                num("widgetPort", "Port", "1024 to 65535."),
                sel("widgetDesign", "Design", "", &[(json!("card"), "Card (460 × 130)"), (json!("bar"), "Slim bar (560 × 64)"), (json!("square"), "Square (300 × 380)"), (json!("minimal"), "Minimal (460 × 110)"), (json!("pill"), "Pill (480 × 80)"), (json!("vinyl"), "Vinyl (460 × 150)"), (json!("poster"), "Poster (480 × 270)"), (json!("ticker"), "Ticker (640 × 48)")]),
                sel("widgetAlign", "Alignment", "", &[(json!("left"), "Left"), (json!("right"), "Right")]),
                tog("widgetRounded", "Rounded Corners", ""),
                stepper("widgetText", "Text Size", "", pct(0.7, 1.6, 0.1)),
                tog("wFade", "Fade Between Tracks", ""),
                tog("wScroll", "Scroll Long Text", ""),
                // (only the Vinyl design has a record to spin, as the web app's)
            ];
            let mut f = f;
            if s("widgetDesign", "card") == "vinyl" { f.push(tog("wSpin", "Spin the Record", "")); }
            f.extend([
                tog("wRequester", "Show Who Requested It", ""),
                sel("wColor", "Colour Change", "", &[(json!(0), "Instant"), (json!(0.5), "Fast"), (json!(1.2), "Normal"), (json!(2.5), "Slow")]),
                button("widgetHide", "Hide This Track", "Hide", "Until the next track."),
                // Alerts: on the widget, so here (Settings > Streaming > Stream Widget > Alerts): a switch, as
                // every other, with Configure beside it
                toggle_cfg("alertsOn", "Alerts", "On the widget: follows, subs, bits, raids and more, from your bot.", "alertsOpen"),
            ]);
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
            let types = "follow, sub, gift, bits, kicks, donation, raid, hypetrain, poll, ban, watchstreak, liked";
            let mut f = vec![sel("alertBot", "Your Bot", "", &[(json!("streamerbot"), "Streamer.bot"), (json!("mixitup"), "Mix It Up"), (json!("generic"), "Generic")])];
            if bot == "streamerbot" {
                f.push(text("sbHost", "Streamer.bot", "Its WebSocket Server's address and port."));
                f.push(tog("sbOn", "Connect to Streamer.bot", ""));
                f.push(note("1. In Streamer.bot: Servers/Clients > WebSocket Server. Tick Auto Start, then Start Server.\n2. Put its address and port here (127.0.0.1:8080 unless you've changed it), and switch it on.\n3. That's all: follows, subs, bits, raids and the rest come through by themselves."));
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
                f.push(note("Have your bot open each address below when that happens, with its own variables in place of the words in capitals (NAME: the viewer's name; MESSAGE, and the amounts, where it has them, or left empty)."));
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
                    format!("1. In Mix It Up, open the event to alert on (a follow, a sub, bits...) and add a Web Request action.\n2. Paste the address above, with type= set to that event: {types}.\n3. Put Mix It Up's special identifiers in: user=$username, and the amount and message for that event (its list of special identifiers has the right names, such as the bits or months).")
                } else {
                    format!("1. When something happens, have your bot open the address above.\n2. Swap NAME for your bot's name for the viewer, and fill in amount and message if it has them (or leave them empty).\n3. type says what happened: {types}, or one of your own alerts' triggers.")
                }));
                if bot != "mixitup" { f.extend(remote_bots(vals, "alerts", port, &format!("/alert?{}{vars}", part(&hide(&key))), lan)); }
            }
            f.extend([
                sel("alertTrans", "Transition", "", &[(json!("seamless"), "Seamless"), (json!("crossfade"), "Crossfade"), (json!("push"), "Slide Across"), (json!("flip"), "Flip"), (json!("wipe"), "Wipe"), (json!("collage"), "Collage"), (json!("overlay"), "Over the Top")]),
            ]);
            if s("alertTrans", "seamless") == "collage" {
                f.push(sel("alertCSize", "Collage Tiles", "", &[(json!("small"), "Small"), (json!("medium"), "Medium"), (json!("large"), "Large")]));
                f.push(sel("alertCDir", "Sweep", "", &[(json!("diag"), "Corner to Corner"), (json!("diag-up"), "Corner to Corner, Rising"), (json!("ltr"), "Left to Right"), (json!("rtl"), "Right to Left"), (json!("ttb"), "Top to Bottom"), (json!("btt"), "Bottom to Top"), (json!("centre"), "From the Middle"), (json!("random"), "Scattered")]));
                f.push(range("alertCAngle", "Angle", "The tiles' tilt, in degrees.", -90.0, 90.0, 5.0, "°", true));
            }
            if s("alertTrans", "seamless") == "overlay" {
                f.push(sel("alertAnim", "Animation", "", &[(json!("slide"), "Slide"), (json!("pop"), "Pop"), (json!("fade"), "Fade"), (json!("bounce"), "Bounce"), (json!("flip"), "Flip")]));
            }
            f.push(tog("alertOwnCol", "Their Own Colours", "Off: the alerts take the album art's colours, as the widget does."));
            if vals["alertOwnCol"].as_bool() == Some(true) {
                f.push(color("alertCol1", "Accent Colour", ""));
                f.push(color("alertCol2", "Second Colour", ""));
                f.push(sel("alertDir", "Gradient", "", &[(json!("115deg"), "Diagonal"), (json!("90deg"), "Left to Right"), (json!("180deg"), "Top to Bottom"), (json!("65deg"), "Diagonal, Rising"), (json!("glow"), "Glow")]));
            }
            f.push(stepper("alertIcon", "Icon Size", "", pct(0.6, 2.0, 0.2)));
            f.push(sel("alertSecs", "Show For", "", &[(json!(3), "3 s"), (json!(5), "5 s"), (json!(8), "8 s"), (json!(12), "12 s")]));
            f.push(range("alertVol", "Sound Volume", "", 0.0, 1.0, 0.01, "%", false));
            f.push(button("alList", "Alerts and Their Order", "Open", "Which alerts show and which goes first, and each one's words, picture and sound."));
            f
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
            text("togetherName", "Your Name", "What others see when you listen along or Jam. Empty: your account's name."),
        ],
        // Lyrics: where they come from, in order (as the web app's)
        "lyrics" => lyrics_fields(vals),
        "replies" => {
            let mut f = vec![note("What your bot posts back for each. Empty: the default, shown under it.\nPlaceholders: {title} {artist} {user} {position} {platform} {query} {count} {by}")];
            for (k, label) in crate::requests::MSG_KEYS { f.push(text(&format!("msg:{k}"), label, crate::requests::default_msg(k))); }
            f.push(button("repliesReset", "", "Reset to Defaults", "Every reply back to its default (Save keeps it)."));
            f
        }
        // Seamless Mix's own page: its styles, its blend, and one forced for testing
        "mix" => vec![
            note("Seamless Mix is WIP: it's still being worked on, so some transitions may not sound right yet. Rating them (Stream Deck: Mix Good / Mix Bad) helps."),
            note("Styles: each one, when two songs suit it, in place of the usual blend (never the same one twice in a row)."),
            tog("mixWords", "Wordplay", "The song playing sings a word (\"heat\", \"one more time\"), it's looped on the beat, and the next song comes in singing the same one. Only with songs whose timed lyrics match what plays."),
            tog("mixDrop", "Drop Swap", "The song playing builds to its final drop, and the next song's drop lands there instead, on the beat."),
            tog("mixMelody", "Melody Play", "A bar of the song playing's melody loops while the next one comes in under it, when the next one's melody starts with the same notes."),
            tog("mixTape", "Tape Stops", "Two songs too far apart in tempo or key to blend: the one playing slows to a halt and the next spins up. Off: they change over the usual way instead."),
            note("The blend: on the beat, at one tempo, the next song sweeping in from the top as the one playing gives way below it."),
            sel("mixBars", "Blend Length", "How long the two play together.", &[(json!(0), "Auto (as the songs suit)"), (json!(4), "4 bars"), (json!(8), "8 bars"), (json!(16), "16 bars")]),
            tog("mixBands", "EQ Band Handoff", "A band at a time (highs, then voice and bass), as a DJ's EQ does, instead of one sweep: quicker and more sudden."),
            tog("mixKeys", "Match Keys", "A song shifted a semitone or two during the blend, so the two keys go together (back to its own key by the end of it)."),
            tog("mixLoops", "Loops", "A short intro or outro looped, so neither song is cut short to fit the blend."),
            note("Testing"),
            sel("mixForce", "Force a Style", "Every transition this style, to hear it. A pair that can't be mixed that way isn't mixed at all.", &[(json!("auto"), "Off (as the songs suit)"), (json!("beat"), "On the beat (no tape stops)"), (json!("wordplay"), "Wordplay"), (json!("drop"), "Drop Swap"), (json!("melody"), "Melody Play"), (json!("tape"), "Tape Stop")]),
        ],
        // Smart Transitions' own page
        "smart" => vec![
            note("Smart Transitions are WIP: still being built, so some may not sound right yet. Rating them (Stream Deck: Mix Good / Mix Bad) helps."),
            sel("smartMax", "Longest Fade", "The most the two songs play over each other. Each pair gets as long as the shorter of the song playing's outro and the next one's intro, up to this.", &[(json!(6), "6 s"), (json!(8), "8 s"), (json!(10), "10 s"), (json!(12), "12 s"), (json!(16), "16 s"), (json!(20), "20 s")]),
            tog("smartBlend", "Beatmatched Blends", "Two songs with a clear beat, within 4% of each other's tempo and in keys that go together, blend on the beat: the next one at the playing one's tempo (its pitch kept), on a phrase, the bass swapped on a bar line. Off: always a fade."),
            tog("smartVoices", "Vocal Handoff", "The two songs' vocals are never sung over each other: the one playing's voice drops out as the next one's comes in (on a blend's bar line, or halfway through a fade). It takes out what's in the middle of the mix, as a karaoke switch does, so it can thin other centred sounds too; the bass and drums' low end stay."),
            tog("smartRoll", "Rolls", "Songs too far apart to blend: the one playing's last bar before a phrase line loops, halving (a bar, a half, a quarter, an eighth) as a build, and the next one lands on its downbeat at its own tempo. Stops by itself if your ratings turn it down (5 rated, most of them bad)."),
            tog("smartTape", "Tape Stops", "Songs too far apart to blend: the one playing slows to a halt and the next spins up. Only once your ratings back them (at least 5 rated, most of them good)."),
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
            ]
        }
        "hidden" => crate::hide::fields(prefs),
        "safe" => crate::streamsafe::panel_fields(prefs),
        "plugins" => plugin_fields(prefs),
        "offset" => vec![
            num("out2Offset", "Offset", "In ms, -5000 to 5000: how much later (or, below 0, earlier) OmniPulse Offset plays than you hear."),
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
    if !web.is_empty() { can.push(format!("Connect to {}", web.join(", "))); }
    if !local.is_empty() { warn.push(format!("It can connect to programs running on this PC ({}). Only allow this if you know what it talks to.", local.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", "))); }
    let sets = m.settings.as_array().cloned().unwrap_or_default();
    if sets.iter().any(|x| x["type"] == "password") { can.push("Keep a password or key you give it, on this PC; it can send that to the sites above".into()); }
    else if !sets.is_empty() { can.push("Keep its own settings, on this PC".into()); }
    cannot.push(if m.hosts.is_empty() { "Connect to the internet".into() } else { "Connect to any other site".into() });
    cannot.extend(["See your accounts or sign-ins (SoundCloud, Spotify, YouTube Music, Last.fm, Discord)", "Open, read or change your files", "See your playlists, Library, listening history or settings", "Change anything else in the app"].map(String::from));
    (can, cannot, warn)
}

fn plugin_fields(_prefs: &Value) -> Vec<Value> {
    let mut f = vec![
        note("Music sources made by others. Each is walled off and only reaches the sites it lists."),
        text("plugSource", "Add a Plugin", "Its manifest.json: a web address, or a file on this PC."),
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
    for p in &all { f.push(button(&format!("plugFiles:{}", p.manifest.id), &p.manifest.name, "Show Its Files", "Opens the folder with its manifest.json and main.js.")); }
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
    fn panels_start_from_prefs_and_defaults() {
        let prefs = json!({ "reqBot": "mixitup", "reqKey": "abc", "widgetText": 1.2 });
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
        let f = fields("requests", &v, &json!({ "reqKey": "abc", "streamerMode": true }), "");
        let addr = f.iter().find(|x| x["type"] == "info").unwrap();
        assert!(!addr["value"].as_str().unwrap().contains("abc") && addr["copy"].as_str().unwrap().contains("abc"));
        // the alerts' bot follows the requests' one until it's picked
        assert_eq!(values("alerts", &prefs)["alertBot"], "mixitup");
    }

    #[test]
    fn any_other_bot_gets_an_address_for_each_alert() {
        let prefs = json!({ "alertBot": "generic", "alertKey": "k" });
        let f = fields("alerts", &values("alerts", &prefs), &prefs, "");
        let addrs: Vec<&str> = f.iter().filter(|x| x["type"] == "info").filter_map(|x| x["copy"].as_str()).collect();
        assert_eq!(addrs.len(), 11);
        assert!(addrs.contains(&"http://127.0.0.1:7878/alert?type=raid&user=NAME&amount=VIEWERS"));
    }
}
