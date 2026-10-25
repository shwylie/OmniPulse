// Restoring a chatbot's backup as the Actions page's actions: OmniPulse's own, Streamer.bot's (an export
// string or its .sb file, a backup zip, or its actions.json) and Mix It Up's (a .miubackup, or a command
// exported on its own). Each one's triggers and steps become ours (crate::flows), its variables ours;
// what has no match here is kept as a note in its action, never lost without a word.
//
// Streamer.bot's formats as its exports and backups have them (an export: base64 of "SBAE" and gzipped
// JSON; its type codes as github.com/PedroZamecki/streamerz reads them, MIT); Mix It Up's as its own code
// writes them (github.com/MixItUpBot/Desktop: a zip of its settings, .miu3, and its database, .db3, whose
// Commands table holds each command as JSON, its Quotes table the quotes).

use serde_json::{json, Map, Value};
use std::collections::HashMap;
use std::io::Read;

use crate::flows::{defaults, STEPS, TRIGGERS};

/// What a backup held, as ours: its actions, its quotes (and, from OmniPulse's own, its variables and
/// points), where it was from, and what couldn't be brought over
#[derive(Default, Debug)]
pub struct Restored {
    pub from: &'static str,
    pub actions: Vec<Value>,
    pub quotes: Vec<Value>,
    pub vars: Map<String, Value>,
    pub points: Map<String, Value>,
    pub missed: Vec<String>,
}

/// OmniPulse's own backup: everything the chatbot keeps of its own
pub fn backup(actions: &[Value], kept: &Value) -> Value {
    json!({
        "omnipulseChatbot": 1,
        "actions": actions,
        "quotes": kept["quotes"].as_array().cloned().unwrap_or_default(),
        "vars": kept["vars"].as_object().cloned().unwrap_or_default(),
        "points": kept["points"].as_object().cloned().unwrap_or_default(),
    })
}

/// A backup file, whichever it is
pub fn restore_file(path: &std::path::Path) -> Result<Restored, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("couldn't read it: {e}"))?;
    if bytes.starts_with(b"PK") { return restore_zip(&bytes); }
    restore_text(&String::from_utf8_lossy(&bytes))
}

/// A backup as text: one of ours, Streamer.bot's export string or JSON, a Mix It Up command
pub fn restore_text(text: &str) -> Result<Restored, String> {
    let t = text.trim().trim_start_matches('\u{feff}').trim();
    if let Ok(v) = serde_json::from_str::<Value>(t) { return restore_json(&v); }
    // (Streamer.bot's import code: base64 of "SBAE" then gzipped JSON)
    use base64::Engine;
    let squeezed: String = t.chars().filter(|c| !c.is_whitespace()).collect();
    if let Ok(raw) = base64::engine::general_purpose::STANDARD.decode(squeezed.as_bytes()) {
        let body = raw.strip_prefix(b"SBAE").unwrap_or(&raw);
        let mut out = String::new();
        if flate2::read::GzDecoder::new(body).read_to_string(&mut out).is_ok() {
            if let Ok(v) = serde_json::from_str::<Value>(out.trim_start_matches('\u{feff}')) { return restore_json(&v); }
        }
    }
    Err("that isn't a backup OmniPulse knows: one of its own, Streamer.bot's (an import code, a .sb file, a backup or its actions.json) or Mix It Up's (a .miubackup or a .miucommand)".into())
}

fn restore_json(v: &Value) -> Result<Restored, String> {
    if v["omnipulseChatbot"].is_number() {
        return Ok(Restored {
            from: "an OmniPulse backup",
            actions: v["actions"].as_array().cloned().unwrap_or_default(),
            quotes: v["quotes"].as_array().cloned().unwrap_or_default(),
            vars: v["vars"].as_object().cloned().unwrap_or_default(),
            points: v["points"].as_object().cloned().unwrap_or_default(),
            missed: Vec::new(),
        });
    }
    // Streamer.bot's export ({ meta, data: { actions, commands, timers } }), or its actions.json
    if v["data"]["actions"].is_array() { return Ok(streamerbot(&v["data"])); }
    if v["actions"].as_array().is_some_and(|a| a.iter().any(|x| x.get("subActions").is_some() || x.get("triggers").is_some())) { return Ok(streamerbot(v)); }
    // a Mix It Up command, exported on its own
    if v.get("Actions").is_some() && (v.get("$type").is_some() || v.get("Triggers").is_some()) {
        let mut r = Restored { from: "Mix It Up", ..Default::default() };
        let names = HashMap::new();
        mixitup_command(v, &names, &Value::Null, &mut r);
        return Ok(r);
    }
    Err("that file's JSON isn't a backup OmniPulse knows".into())
}

fn restore_zip(bytes: &[u8]) -> Result<Restored, String> {
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("couldn't open the backup: {e}"))?;
    let names: Vec<String> = (0..z.len()).filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string())).collect();
    let read = |z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, name: &str| -> Option<Vec<u8>> {
        let mut f = z.by_name(name).ok()?;
        let mut out = Vec::new();
        f.read_to_end(&mut out).ok()?;
        Some(out)
    };
    // Mix It Up's: its settings (.miu3) and its database (.db3)
    if let Some(db) = names.iter().find(|n| n.to_lowercase().ends_with(".db3")).cloned() {
        let settings = names.iter().find(|n| n.to_lowercase().ends_with(".miu3")).cloned().and_then(|n| read(&mut z, &n)).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).unwrap_or(Value::Null);
        let data = read(&mut z, &db).ok_or("couldn't read its database")?;
        return mixitup_db(&data, &settings);
    }
    // Streamer.bot's: its data folder's actions.json and commands.json (and its timers, wherever they are)
    let json_of = |z: &mut zip::ZipArchive<std::io::Cursor<&[u8]>>, file: &str| -> Value {
        names.iter().find(|n| n.replace('\\', "/").rsplit('/').next().is_some_and(|b| b.eq_ignore_ascii_case(file))).cloned()
            .and_then(|n| read(z, &n)).and_then(|b| serde_json::from_str::<Value>(String::from_utf8_lossy(&b).trim_start_matches('\u{feff}')).ok()).unwrap_or(Value::Null)
    };
    let actions = json_of(&mut z, "actions.json");
    if actions["actions"].is_array() {
        let commands = json_of(&mut z, "commands.json");
        let settings = json_of(&mut z, "settings.json");
        let timers = json_of(&mut z, "timers.json");
        let data = json!({
            "actions": actions["actions"],
            "commands": commands["commands"],
            "timers": timers["timers"].as_array().cloned().or_else(|| settings["timers"].as_array().cloned()).or_else(|| settings["timedActions"].as_array().cloned()).unwrap_or_default(),
        });
        return Ok(streamerbot(&data));
    }
    Err("that zip isn't a Streamer.bot or Mix It Up backup".into())
}

/// A trigger or a step: ours, with these fields changed from its defaults
fn ours(list: &'static [crate::flows::Kind], id: &str, set: Value) -> Value {
    let mut d = defaults(list, id);
    if let (Some(o), Some(s)) = (d.as_object_mut(), set.as_object()) { for (k, v) in s { o.insert(k.clone(), v.clone()); } }
    d
}

/// How many of what couldn't be brought over were triggers (the rest, steps kept as notes)
pub fn missed_triggers(r: &Restored) -> usize {
    r.missed.iter().filter(|m| m.starts_with("its trigger") || m.contains(": its event") || m.contains(" command") ).count()
}

/// What couldn't be brought over, kept in its action as a note
fn note(from: &str, what: &str, missed: &mut Vec<String>) -> Value {
    missed.push(what.to_string());
    ours(STEPS, "note", json!({ "text": format!("Not brought over from {from}: {what}") }))
}

fn s(v: &Value) -> String {
    match v { Value::String(x) => x.clone(), Value::Null => String::new(), x => x.to_string() }
}
fn n(v: &Value) -> f64 {
    v.as_f64().or_else(|| v.as_str().and_then(|x| x.trim().parse().ok())).unwrap_or(0.0)
}

/// Triggers the same but for their platform and their reward's id (a Twitch reward and a Kick one, a
/// follow on each): one, on any platform, matching either reward
fn merge_triggers(list: Vec<(Value, bool)>) -> Vec<(Value, bool)> {
    let bare = |t: &Value| { let mut x = t.clone(); if let Some(o) = x.as_object_mut() { o.remove("from"); o.remove("rewardid"); o.remove("reward"); } x };
    let mut out: Vec<(Value, bool)> = Vec::new();
    for (t, on) in list {
        let same = out.iter_mut().find(|(u, _)| bare(u) == bare(&t) && (u["type"] == "event" || u["type"] == "message") && (u["event"] != "reward" || (!s(&u["rewardid"]).is_empty() && !s(&t["rewardid"]).is_empty())));
        match same {
            Some((u, u_on)) => {
                if u["from"] != t["from"] { u["from"] = json!("any"); }
                let ids: Vec<String> = s(&u["rewardid"]).split(',').chain(s(&t["rewardid"]).split(',')).map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).fold(Vec::new(), |mut v, x| { if !v.contains(&x) { v.push(x); } v });
                if !ids.is_empty() { u["rewardid"] = json!(ids.join(", ")); }
                if s(&u["reward"]).is_empty() { u["reward"] = t["reward"].clone(); }
                *u_on = *u_on || on;
            }
            None => out.push((t, on)),
        }
    }
    out
}

/// An action for each of its triggers (ours have one each): "Name", then "Name 2"...; the same trigger on
/// Twitch and Kick, one
fn per_trigger(name: &str, on: bool, triggers: Vec<(Value, bool)>, steps: &[Value], out: &mut Vec<Value>) {
    let triggers = if triggers.is_empty() { vec![(defaults(TRIGGERS, "manual"), true)] } else { merge_triggers(triggers) };
    for (k, (t, t_on)) in triggers.into_iter().enumerate() {
        let name = if k == 0 { name.to_string() } else { format!("{name} {}", k + 1) };
        out.push(json!({ "name": name, "on": on && t_on, "trigger": t, "steps": steps }));
    }
}

// ---------- Streamer.bot ----------

/// Streamer.bot's variables, as ours (its %user%, %rawInput% and %input0% are ours already)
fn sb_vars(t: &str) -> String {
    let mut t = t.to_string();
    for (from, to) in [
        ("%userName%", "%user%"), ("%username%", "%user%"), ("%displayName%", "%user%"), ("%rawInputEscaped%", "%rawInput%"), ("%rawInputUrlEncoded%", "%rawInput%"),
        ("%bits%", "%amount%"), ("%viewers%", "%amount%"), ("%monthsSubscribed%", "%amount%"), ("%cumulative%", "%amount%"), ("%gifts%", "%amount%"),
        ("%totalSubsGifted%", "%amount%"), ("%rewardName%", "%reward%"), ("%targetUser%", "%input0%"), ("%targetUserName%", "%input0%"), ("%messageStripped%", "%message%"),
    ] { t = t.replace(from, to); }
    t
}

fn sb_name(code: u64, table: &[(u32, &str)]) -> String {
    let raw = table.iter().find(|x| x.0 as u64 == code).map(|x| x.1).unwrap_or("");
    if raw.is_empty() { return format!("a Streamer.bot item (type {code})"); }
    // ("ObsSetGdiText": "Obs Set Gdi Text")
    let mut out = String::new();
    for (i, c) in raw.chars().enumerate() { if i > 0 && c.is_uppercase() { out.push(' '); } out.push(c); }
    out
}

const SB_STEPS: &[(u32, &str)] = &[
    (1, "PlaySound"), (3, "WriteToFile"), (4, "RunAction"), (5, "ReadRandomLineFromFile"), (6, "RunExecutable"), (7, "PlaySoundFromFolder"),
    (10, "SendMessage"), (11, "RedemptionStatus"), (15, "SetChannelTitle"), (16, "SetChannelGame"), (17, "CreateStreamMarker"), (18, "CreateClip"),
    (19, "RunCommercial"), (20, "TimeoutUser"), (23, "Announce"), (29, "SendShoutout"), (30, "ObsSourceVisibility"), (33, "ObsStreaming"),
    (34, "ObsSetActiveScene"), (35, "ObsSourceFilterState"), (38, "ObsSetBrowserSourceUrl"), (39, "ObsSetGdiText"), (42, "ObsSourceMuteState"),
    (45, "ObsRecording"), (46, "ObsRaw"), (99, "ExecuteCode"), (120, "IfElse"), (121, "GetGlobal"), (122, "SetGlobal"), (123, "SetArgument"),
    (124, "Break"), (127, "Switch"), (128, "While"), (560, "ReplyToMessage"), (1000, "GetQuote"), (1002, "Delay"), (1003, "GetRandomNumber"),
    (1007, "FetchUrl"), (1009, "Comment"), (1011, "SetActionGroupState"), (1012, "KeyboardPress"), (1020, "LogEntry"), (1037, "AddQuote"),
    (4001, "YouTubeMessage"), (35001, "KickSendMessage"), (35002, "KickReplyToMessage"), (35031, "KickSetChannelCategory"), (99900, "Group"),
];
const SB_TRIGGERS: &[(u32, &str)] = &[
    (101, "TwitchFollow"), (102, "TwitchCheer"), (103, "TwitchSub"), (104, "TwitchReSub"), (105, "TwitchGiftSub"), (106, "TwitchGiftBomb"),
    (107, "TwitchRaid"), (108, "TwitchHypeTrainStart"), (112, "TwitchRewardRedemption"), (119, "TwitchWhisper"), (120, "TwitchFirstWord"),
    (125, "TwitchPollCreated"), (128, "TwitchPredictionCreated"), (133, "TwitchChatMessage"), (139, "TwitchAdRun"), (154, "TwitchStreamOnline"),
    (155, "TwitchStreamOffline"), (156, "TwitchShoutoutReceived"), (201, "StreamlabsDonation"), (401, "CommandTriggered"), (701, "TimedAction"),
    (703, "ProcessStarted"), (35024, "KickRewardRedemption"),
    (14004, "ObsSceneChanged"), (14005, "ObsStreamingStarted"), (14006, "ObsStreamingStopped"), (14007, "ObsRecordingStarted"), (14008, "ObsRecordingStopped"),
    (35010, "KickChatMessage"), (35011, "KickFollow"), (35014, "KickSubscription"),
];

fn sb_trigger(t: &Value, commands: &HashMap<String, Value>, timers: &HashMap<String, Value>, missed: &mut Vec<String>) -> Option<(Value, bool)> {
    let code = t["type"].as_u64().unwrap_or(0);
    let on = t["enabled"].as_bool() != Some(false);
    let ev = |e: &str, set: Value| { let mut x = json!({ "event": e }); if let (Some(o), Some(s)) = (x.as_object_mut(), set.as_object()) { for (k, v) in s { o.insert(k.clone(), v.clone()); } } ours(TRIGGERS, "event", x) };
    let made = match code {
        401 => {
            let c = commands.get(&s(&t["commandId"]))?;
            let names: Vec<String> = s(&c["command"]).split(['\r', '\n']).map(|x| x.trim().to_lowercase()).filter(|x| !x.is_empty()).collect();
            let first = names.first().cloned().unwrap_or_else(|| format!("!{}", s(&c["name"]).to_lowercase()));
            let trig = ours(TRIGGERS, "command", json!({ "command": first, "aliases": names.iter().skip(1).cloned().collect::<Vec<_>>().join(", "), "cooldown": n(&c["globalCooldown"]), "usercooldown": n(&c["userCooldown"]) }));
            return Some((trig, on && c["enabled"].as_bool() != Some(false)));
        }
        101 | 35011 => ev("follow", json!({})),
        102 => ev("bits", json!({ "min": n(&t["min"]) })),
        103 | 35014 => ev("sub", json!({})),
        104 => ev("resub", json!({})),
        105 | 106 => ev("gift", json!({})),
        107 => ev("raid", json!({ "min": n(&t["min"]) })),
        108 => ev("hypetrain", json!({})),
        // (Streamer.bot keeps a reward's id, and its name only sometimes: matched by the id)
        112 => ev("reward", json!({ "reward": s(&t["rewardTitle"]).trim().to_string(), "rewardid": s(&t["rewardId"]).trim().to_string(), "from": "twitch" })),
        35024 => ev("reward", json!({ "reward": s(&t["rewardTitle"]).trim().to_string(), "rewardid": s(&t["rewardId"]).trim().to_string(), "from": "kick" })),
        703 => {
            let name = s(&t["processName"]).trim().to_string();
            if name.is_empty() { missed.push("its trigger Process Started, with no program named".into()); return None; }
            ours(TRIGGERS, "process", json!({ "name": name }))
        }
        120 => ev("firstmsg", json!({})),
        125 => ev("poll", json!({})),
        139 => ev("adbreak", json!({})),
        154 => ev("online", json!({})),
        155 => ev("offline", json!({})),
        156 => ev("shoutoutin", json!({})),
        201 => ev("donation", json!({})),
        133 | 35010 => ours(TRIGGERS, "message", json!({ "match": "regex", "text": "." })),
        701 => {
            let timer = timers.get(&s(&t["timerId"])).cloned().unwrap_or(Value::Null);
            let secs = n(&timer["interval"]).max(60.0);
            let trig = ours(TRIGGERS, "timer", json!({ "minutes": (secs / 60.0).round(), "lines": n(&timer["lines"]) }));
            // (the timer's own switch too)
            return Some((trig, on && timer["enabled"].as_bool() != Some(false)));
        }
        14004 => ours(TRIGGERS, "obs", json!({ "when": "scene", "scene": s(&t["sceneName"]) })),
        14005 => ours(TRIGGERS, "obs", json!({ "when": "streamstart" })),
        14006 => ours(TRIGGERS, "obs", json!({ "when": "streamstop" })),
        14007 => ours(TRIGGERS, "obs", json!({ "when": "recordstart" })),
        14008 => ours(TRIGGERS, "obs", json!({ "when": "recordstop" })),
        _ => { missed.push(format!("its trigger {}", sb_name(code, SB_TRIGGERS))); return None; }
    };
    Some((made, on))
}

/// A web address's text, %-decoded
fn unescape(t: &str) -> String {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() + 0 && i + 2 < b.len() + 1 {
            if let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..(i + 3).min(b.len())]).unwrap_or(""), 16) { out.push(v); i += 3; continue; }
        }
        out.push(if b[i] == b'+' { b' ' } else { b[i] });
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// A bot's web request, as ours: to OmniPulse's own addresses (its song requests, what's playing, a request
/// taken back, an alert), the step that does it; to anywhere else, Fetch a Web Address. `vars`: the bot's
/// variables, as ours; `into`: what the answer's called
fn web_step(url: &str, into: &str, vars: &dyn Fn(&str) -> String) -> Value {
    let (place, query) = url.split_once('?').unwrap_or((url, ""));
    let local = ["//127.0.0.1:", "//localhost:"].iter().any(|h| place.contains(h));
    let path = place.rsplit('/').next().unwrap_or("");
    let param = |k: &str| query.split('&').find_map(|kv| kv.split_once('=').filter(|(a, _)| *a == k).map(|(_, v)| vars(&unescape(v)).replace("%rawInputUrlEncoded%", "%rawInput%")));
    if local {
        match path {
            "request" => return ours(STEPS, "request", json!({ "query": param("q").unwrap_or_else(|| "%rawInput%".into()), "reply": false })),
            "now" => return ours(STEPS, "nowplaying", json!({ "say": false })),
            "wrongsong" => return ours(STEPS, "wrongsong", json!({ "say": false })),
            "alert" => return ours(STEPS, "alert", json!({ "alert": param("type").unwrap_or_else(|| "follow".into()), "user": param("user").unwrap_or_default(), "amount": param("amount").unwrap_or_default(), "message": param("message").unwrap_or_default() })),
            _ => {}
        }
    }
    ours(STEPS, "fetch", json!({ "url": vars(url), "into": if into.is_empty() { "reply" } else { into } }))
}

/// Streamer.bot's steps, in order: groups opened up, an If/Else that only picks which platform answers
/// taken as Twitch's branch, a last one that only goes on if something's so as Only Go On If
fn sb_steps(list: &[Value], names: &HashMap<String, String>, missed: &mut Vec<String>) -> Vec<Value> {
    let mut subs: Vec<&Value> = list.iter().filter(|x| x["enabled"].as_bool() != Some(false)).collect();
    subs.sort_by_key(|x| x["index"].as_i64().unwrap_or(0));
    let mut out = Vec::new();
    for (i, st) in subs.iter().enumerate() {
        let code = st["type"].as_u64().unwrap_or(0);
        let inner = st["subActions"].as_array().cloned().unwrap_or_default();
        match code {
            // (a group: its steps, one of them at random if it says so)
            99900..=99902 => {
                let steps = sb_steps(&inner, names, missed);
                if st["random"].as_bool() == Some(true) && steps.len() > 1 && steps.iter().all(|x| x["type"] == "say") {
                    let lines: Vec<String> = steps.iter().map(|x| s(&x["text"]).replace('|', "/")).collect();
                    out.push(ours(STEPS, "random", json!({ "lines": lines.join(" | ") })));
                } else { out.extend(steps); }
            }
            120 => {
                let (yes, no) = (inner.iter().find(|x| x["type"] == 99901).cloned(), inner.iter().find(|x| x["type"] == 99902).cloned());
                let branch = |g: &Option<Value>| g.as_ref().and_then(|g| g["subActions"].as_array().cloned()).unwrap_or_default();
                let op = st["operation"].as_u64().unwrap_or(0);
                let input = s(&st["input"]);
                if input.contains("%userType%") && op <= 1 {
                    // (which platform answers: ours answer where they were asked already, so one branch does
                    // when both say the same; otherwise each platform's own, by %platform%)
                    let is_value = op == 0;
                    let (on_value, off_value) = (sb_steps(&branch(&yes), names, missed), sb_steps(&branch(&no), names, missed));
                    let (twitch, other) = if s(&st["value"]).eq_ignore_ascii_case("twitch") == is_value { (on_value, off_value) } else { (off_value, on_value) };
                    let said = |l: &Vec<Value>| l.iter().map(|x| format!("{}{}", x["type"], s(&x["text"]))).collect::<Vec<_>>();
                    // (one for both: where it came from, not Twitch's own chat)
                    if other.is_empty() || said(&twitch) == said(&other) { out.extend(twitch.into_iter().map(|mut x| { if let Some(o) = x.as_object_mut() { o.remove("where"); } x })); } else {
                        out.push(ours(STEPS, "ifblock", json!({ "left": "%platform%", "test": "is", "right": "twitch" })));
                        out.extend(twitch);
                        out.push(ours(STEPS, "else", json!({})));
                        out.extend(other);
                        out.push(ours(STEPS, "endif", json!({})));
                    }
                } else if i == subs.len() - 1 && branch(&no).is_empty() && op <= 1 {
                    out.push(ours(STEPS, "if", json!({ "left": sb_vars(&input), "test": if op == 0 { "is" } else { "isnt" }, "right": sb_vars(&s(&st["value"])) })));
                    out.extend(sb_steps(&branch(&yes), names, missed));
                } else if let Some((test, flip)) = sb_test(op) {
                    // (Doesn't Contain: Contains, its branches the other way round)
                    let (a, b) = if flip { (&no, &yes) } else { (&yes, &no) };
                    out.push(ours(STEPS, "ifblock", json!({ "left": sb_vars(&input), "test": test, "right": sb_vars(&s(&st["value"])) })));
                    out.extend(sb_steps(&branch(a), names, missed));
                    let other = sb_steps(&branch(b), names, missed);
                    if !other.is_empty() { out.push(ours(STEPS, "else", json!({}))); out.extend(other); }
                    out.push(ours(STEPS, "endif", json!({})));
                } else {
                    out.push(note("Streamer.bot", "an If/Else with a test OmniPulse doesn't have (its steps are in Streamer.bot)", missed));
                }
            }
            1007 => out.push(web_step(&s(&st["url"]), &s(&st["variableName"]), &|t: &str| sb_vars(t).replace("%rawInput%", "%rawInputUrlEncoded%"))),
            _ => if let Some(x) = sb_step(st, names, missed) { out.push(x); },
        }
    }
    out
}

/// Streamer.bot's If/Else tests, as ours: (our test, its branches swapped)
fn sb_test(op: u64) -> Option<(&'static str, bool)> {
    Some(match op { 0 => ("is", false), 1 => ("isnt", false), 2 => ("contains", false), 3 => ("contains", true), 6 => ("more", false), 8 => ("less", false), _ => return None })
}

fn sb_step(st: &Value, names: &HashMap<String, String>, missed: &mut Vec<String>) -> Option<Value> {
    let code = st["type"].as_u64().unwrap_or(0);
    let txt = |k: &str| sb_vars(&s(&st[k]));
    // (OBS's: Visible / Hidden / Toggle, Start / Stop / Toggle, Enabled / Disabled, Mute / Unmute / Toggle)
    let state = st["state"].as_u64().unwrap_or(0);
    let three = |a: &str, b: &str| if state == 0 { a.to_string() } else if state == 1 { b.to_string() } else { "toggle".to_string() };
    Some(match code {
        // (Twitch's and Kick's own: each to its own chat)
        10 => ours(STEPS, "say", json!({ "text": txt("text"), "reply": false, "where": "twitch" })),
        35001 => ours(STEPS, "say", json!({ "text": txt("text"), "reply": false, "where": "kick" })),
        4001 => ours(STEPS, "say", json!({ "text": txt("text"), "reply": false })),
        560 | 35002 => ours(STEPS, "say", json!({ "text": txt("text"), "reply": true })),
        23 => ours(STEPS, "announce", json!({ "text": txt("text") })),
        1002 => ours(STEPS, "wait", json!({ "seconds": (n(&st["value"]) / 1000.0 * 10.0).round() / 10.0 })),
        1 => {
            let v = n(&st["volume"]);
            ours(STEPS, "sound", json!({ "file": s(&st["soundFile"]), "volume": if v > 0.0 && v <= 1.0 { (v * 100.0).round() } else if v > 1.0 { v.min(100.0) } else { 80.0 } }))
        }
        4 => ours(STEPS, "run", json!({ "action": names.get(&s(&st["actionId"])).cloned().unwrap_or_default() })),
        15 => ours(STEPS, "title", json!({ "text": txt("title") })),
        16 => ours(STEPS, "game", json!({ "text": txt("game"), "where": "twitch" })),
        35031 => ours(STEPS, "game", json!({ "text": txt("categoryName"), "where": "kick" })),
        19 => { let d = n(&st["duration"]); ours(STEPS, "ad", json!({ "seconds": if d > 0.0 { d } else { 60.0 } })) }
        1011 => ours(STEPS, "groupstate", json!({ "group": s(&st["groupName"]), "what": match st["state"].as_u64().unwrap_or(0) { 0 => "on", 1 => "off", _ => "toggle" } })),
        1012 => {
            // (its held keys: Windows' hotkey flags, or .NET's)
            let m = st["modifiers"].as_u64().unwrap_or(0);
            let mut keys = Vec::new();
            if m & 2 != 0 || m & 0x20000 != 0 { keys.push("Ctrl".to_string()); }
            if m & 4 != 0 || m & 0x10000 != 0 { keys.push("Shift".to_string()); }
            if m & 1 != 0 || m & 0x40000 != 0 { keys.push("Alt".to_string()); }
            if m & 8 != 0 { keys.push("Win".to_string()); }
            keys.push(crate::procwatch::key_name(st["key"].as_u64().unwrap_or(0)));
            ours(STEPS, "keys", json!({ "keys": keys.join("+") }))
        }
        6 => ours(STEPS, "open", json!({ "target": s(&st["command"]), "args": s(&st["arguments"]) })),
        17 => ours(STEPS, "marker", json!({ "text": txt("description") })),
        18 => ours(STEPS, "clip", json!({})),
        20 => ours(STEPS, "timeout", json!({ "seconds": n(&st["duration"]).max(1.0), "reason": txt("reason") })),
        29 => ours(STEPS, "shoutout", json!({ "who": txt("userLogin") })),
        30 => ours(STEPS, "obssource", json!({ "scene": txt("sceneName"), "source": txt("sourceName"), "show": three("show", "hide") })),
        33 => ours(STEPS, "obsstream", json!({ "what": three("start", "stop") })),
        45 => ours(STEPS, "obsrecord", json!({ "what": three("start", "stop") })),
        34 => ours(STEPS, "obsscene", json!({ "scene": txt("sceneName") })),
        35 => ours(STEPS, "obsfilter", json!({ "source": txt("sourceName"), "filter": txt("filterName"), "what": if state == 1 { "off" } else { "on" } })),
        39 => ours(STEPS, "obstext", json!({ "source": txt("sourceName"), "text": txt("text") })),
        42 => ours(STEPS, "obsmute", json!({ "source": txt("sourceName"), "what": three("mute", "unmute") })),
        122 => ours(STEPS, "setvar", json!({ "name": s(&st["variableName"]), "value": txt("value"), "keep": st["persisted"].as_bool().unwrap_or(true), "per": s(&st["destination"]).to_lowercase().contains("user") })),
        123 => ours(STEPS, "setvar", json!({ "name": s(&st["variableName"]), "value": txt("value"), "keep": false })),
        1000 => ours(STEPS, "quote", json!({ "number": if s(&st["quoteType"]) == "1" { s(&st["quoteId"]) } else { String::new() } })),
        1037 => ours(STEPS, "quoteadd", json!({ "text": "%rawInput%" })),
        // (its comments: left out, nothing lost)
        1009 => return None,
        _ => note("Streamer.bot", &sb_name(code, SB_STEPS), missed),
    })
}

fn streamerbot(data: &Value) -> Restored {
    let mut r = Restored { from: "Streamer.bot", ..Default::default() };
    let by_id = |k: &str| -> HashMap<String, Value> { data[k].as_array().into_iter().flatten().map(|x| (s(&x["id"]), x.clone())).collect() };
    let (commands, timers) = (by_id("commands"), by_id("timers"));
    let actions: Vec<Value> = data["actions"].as_array().cloned().unwrap_or_default();
    let names: HashMap<String, String> = actions.iter().map(|a| (s(&a["id"]), s(&a["name"]))).collect();
    for a in &actions {
        // (its steps: subActions; before Streamer.bot 0.2, actions)
        let mut subs: Vec<Value> = a["subActions"].as_array().or_else(|| a["actions"].as_array()).cloned().unwrap_or_default();
        subs.retain(|x| x["parentId"].is_null() || s(&x["parentId"]).is_empty());
        let mut steps = sb_steps(&subs, &names, &mut r.missed);
        // (a random action, or a random group of its steps (before 0.2): one of them; its messages as one of several)
        let random_group = a["actionGroups"].as_array().is_some_and(|g| g.iter().any(|x| x["random"].as_bool() == Some(true)));
        if (a["randomAction"].as_bool() == Some(true) || random_group) && !steps.is_empty() && steps.iter().all(|x| x["type"] == "say") && (steps.len() > 1 || random_group) {
            let lines: Vec<String> = steps.iter().map(|x| s(&x["text"]).replace('|', "/")).collect();
            steps = vec![ours(STEPS, "random", json!({ "lines": lines.join(" | ") }))];
        }
        let before = r.missed.len();
        let triggers: Vec<(Value, bool)> = a["triggers"].as_array().into_iter().flatten().filter_map(|t| sb_trigger(t, &commands, &timers, &mut r.missed)).collect();
        for m in r.missed[before..].to_vec().iter().rev() {
            steps.insert(0, ours(STEPS, "note", json!({ "text": format!("Not brought over from Streamer.bot: {m}") })));
        }
        let from = r.actions.len();
        per_trigger(&s(&a["name"]), a["enabled"].as_bool() != Some(false), triggers, &steps, &mut r.actions);
        let group = s(&a["group"]).trim().to_string();
        if !group.is_empty() { for x in &mut r.actions[from..] { x["group"] = json!(group); } }
    }
    r
}

// ---------- Mix It Up ----------

/// Mix It Up's special identifiers, as our variables
fn miu_vars(t: &str) -> String {
    let mut out = t.to_string();
    for k in (1..=9).rev() { out = out.replace(&format!("$arg{k}text"), &format!("%input{}%", k - 1)); }
    for (from, to) in [
        ("$targetuserdisplayname", "%input0%"), ("$targetusername", "%input0%"), ("$userdisplayname", "%user%"), ("$username", "%user%"),
        ("$allargs", "%rawInput%"), ("$bitsamount", "%amount%"), ("$raidviewercount", "%amount%"), ("$usersubmonths", "%amount%"),
        ("$subsgiftedamount", "%amount%"), ("$donationamount", "%amount%"), ("$messagenocheermotes", "%message%"), ("$message", "%message%"),
    ] { out = out.replace(from, to); }
    out
}

/// Its class, from Newtonsoft's $type ("MixItUp.Base.Model.Actions.ChatActionModel, MixItUp.Base")
fn class(v: &Value) -> String {
    s(&v["$type"]).split(',').next().unwrap_or("").rsplit('.').next().unwrap_or("").to_string()
}

fn list(v: &Value) -> Vec<Value> {
    // (Newtonsoft's typed lists: { "$type": ..., "$values": [...] })
    if let Some(a) = v.as_array() { return a.clone(); }
    v["$values"].as_array().cloned().unwrap_or_default()
}

fn miu_steps(actions: &[Value], names: &HashMap<String, String>, r: &mut Restored) -> Vec<Value> {
    let mut out = Vec::new();
    for (i, a) in actions.iter().enumerate() {
        if a["Enabled"].as_bool() == Some(false) { continue; }
        let c = class(a);
        let t = |k: &str| miu_vars(&s(&a[k]));
        let kind = a["ActionType"].as_u64().unwrap_or(0);
        let step = match c.as_str() {
            "ChatActionModel" if a["IsWhisper"].as_bool() != Some(true) => ours(STEPS, "say", json!({ "text": t("ChatText"), "reply": false })),
            "WaitActionModel" => ours(STEPS, "wait", json!({ "seconds": n(&a["Amount"]) })),
            "SoundActionModel" if kind == 0 => ours(STEPS, "sound", json!({ "file": s(&a["FilePath"]), "volume": n(&a["VolumeScale"]).clamp(0.0, 100.0) })),
            "StreamingSoftwareActionModel" => match kind {
                0 => ours(STEPS, "obsscene", json!({ "scene": t("ItemName") })),
                1 => ours(STEPS, "obssource", json!({ "source": t("ItemName"), "scene": t("ParentName"), "show": if a["Visible"].as_bool() == Some(false) { "hide" } else { "show" } })),
                2 => ours(STEPS, "obstext", json!({ "source": t("ItemName"), "text": t("SourceText") })),
                5 => ours(STEPS, "obsstream", json!({ "what": "toggle" })),
                8 => ours(STEPS, "obsfilter", json!({ "source": t("ParentName"), "filter": t("ItemName"), "what": if a["Visible"].as_bool() == Some(false) { "off" } else { "on" } })),
                9 => ours(STEPS, "obsrecord", json!({ "what": "toggle" })),
                _ => note("Mix It Up", "an OBS step of a kind OmniPulse hasn't got", &mut r.missed),
            },
            "ConsumablesActionModel" if matches!(kind, 0 | 1 | 2 | 4) => {
                let amount = n(&a["Amount"]).abs();
                let who = if matches!(kind, 2 | 4) { t("Username") } else { "%user%".into() };
                ours(STEPS, "points", json!({ "who": who, "amount": if matches!(kind, 1 | 4) { -amount } else { amount } }))
            }
            "WebRequestActionModel" => web_step(&s(&a["Url"]), "reply", &|x: &str| miu_vars(x)),
            "CommandActionModel" if kind == 0 => ours(STEPS, "run", json!({ "action": names.get(&s(&a["CommandID"])).cloned().unwrap_or_default() })),
            "ModerationActionModel" if kind == 0 => ours(STEPS, "timeout", json!({ "seconds": n(&a["TimeoutAmount"]).max(1.0), "reason": t("ModerationReason") })),
            // (a condition, last: only go on if, then its own steps; anywhere else it'd stop the rest, so a note)
            "ConditionalActionModel" if i == actions.len() - 1 => {
                let first = list(&a["Clauses"]).into_iter().next().unwrap_or(Value::Null);
                let test = match first["ComparisionType"].as_u64().unwrap_or(0) { 0 => "is", 1 => "isnt", 2 | 3 => "more", 4 | 5 => "less", 6 => "contains", _ => "" };
                if test.is_empty() { note("Mix It Up", "a condition of a kind OmniPulse hasn't got", &mut r.missed) } else {
                    out.push(ours(STEPS, "if", json!({ "left": miu_vars(&s(&first["Value1"])), "test": test, "right": miu_vars(&s(&first["Value2"])) })));
                    out.extend(miu_steps(&list(&a["Actions"]), names, r));
                    continue;
                }
            }
            "RandomActionModel" if list(&a["Actions"]).iter().all(|x| class(x) == "ChatActionModel") && !list(&a["Actions"]).is_empty() => {
                let lines: Vec<String> = list(&a["Actions"]).iter().map(|x| miu_vars(&s(&x["ChatText"])).replace('|', "/")).collect();
                ours(STEPS, "random", json!({ "lines": lines.join(" | ") }))
            }
            "" => continue,
            other => note("Mix It Up", &other.trim_end_matches("ActionModel").to_string(), &mut r.missed),
        };
        out.push(step);
    }
    out
}

/// Mix It Up's events (its EventTypeEnum), as ours
fn miu_event(code: u64) -> Option<Value> {
    let ev = |e: &str| ours(TRIGGERS, "event", json!({ "event": e }));
    Some(match code {
        1 | 200 => ev("online"),
        2 | 201 => ev("offline"),
        4 | 203 => ev("raid"),
        10 | 210 => ev("follow"),
        20 | 220 => ev("sub"),
        21 | 221 => ev("resub"),
        22 | 23 | 222 | 223 => ev("gift"),
        60 => ev("firstmsg"),
        53 => ours(TRIGGERS, "message", json!({ "match": "regex", "text": "." })),
        251 => ev("adbreak"),
        270 => ev("bits"),
        271 => ev("reward"),
        280 => ev("hypetrain"),
        282 => ev("hypetrainend"),
        230 => ev("watchstreak"),
        231 => ev("shoutoutin"),
        272 => ev("charity"),
        1000 | 1100 => ev("donation"),
        _ => return None,
    })
}

fn mixitup_command(c: &Value, names: &HashMap<String, String>, settings: &Value, r: &mut Restored) {
    let name = s(&c["Name"]);
    let on = c["IsEnabled"].as_bool() != Some(false);
    let steps = miu_steps(&list(&c["Actions"]), names, r);
    let cooldown = n(&c["Requirements"]["Cooldown"]["Amount"]);
    let per_viewer = c["Requirements"]["Cooldown"]["Type"].as_u64() == Some(1);
    let trigger = match (c["Type"].as_u64().unwrap_or(0), class(c).as_str()) {
        (1, _) | (_, "ChatCommandModel") => {
            let bang = c["IncludeExclamation"].as_bool() != Some(false);
            let words: Vec<String> = list(&c["Triggers"]).iter().map(|x| { let w = s(x).trim().to_lowercase(); if bang && !w.starts_with('!') { format!("!{w}") } else { w } }).filter(|w| !w.is_empty()).collect();
            let Some(first) = words.first().cloned() else { r.missed.push(format!("{name}: a chat command without its words")); return };
            Some(ours(TRIGGERS, "command", json!({ "command": first, "aliases": words[1..].join(", "), "cooldown": if per_viewer { 0.0 } else { cooldown }, "usercooldown": if per_viewer { cooldown } else { 0.0 } })))
        }
        (2, _) | (_, "EventCommandModel") => {
            let code = c["EventType"].as_u64().unwrap_or(0);
            let t = miu_event(code);
            if t.is_none() { r.missed.push(format!("{name}: its event (Mix It Up's {code})")); }
            t
        }
        (3, _) | (_, "TimerCommandModel") => Some(ours(TRIGGERS, "timer", json!({
            "minutes": settings["TimerCommandsInterval"].as_f64().unwrap_or(10.0).max(1.0),
            "lines": settings["TimerCommandsMinimumMessages"].as_f64().unwrap_or(10.0),
        }))),
        (7, _) | (_, "TwitchChannelPointsCommandModel") => Some(ours(TRIGGERS, "event", json!({ "event": "reward", "reward": name }))),
        (4 | 0, _) => Some(defaults(TRIGGERS, "manual")),
        (t, cl) => { r.missed.push(format!("{name}: a {} command", if cl.is_empty() { format!("kind {t}") } else { cl.trim_end_matches("CommandModel").to_string() })); None }
    };
    if let Some(t) = trigger { per_trigger(&name, on, vec![(t, true)], &steps, &mut r.actions); }
}

fn mixitup_db(db: &[u8], settings: &Value) -> Result<Restored, String> {
    // (its database read from a copy of its own, as it is)
    let dir = std::env::temp_dir().join(format!("omnipulse-miu-{}", std::process::id()));
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("restore.db3");
    std::fs::write(&path, db).map_err(|e| format!("couldn't read its database: {e}"))?;
    let out = (|| -> Result<Restored, String> {
        let conn = rusqlite::Connection::open_with_flags(&path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).map_err(|e| format!("couldn't read its database: {e}"))?;
        let mut r = Restored { from: "Mix It Up", ..Default::default() };
        let mut cmds: Vec<Value> = Vec::new();
        {
            let mut q = conn.prepare("SELECT Data FROM Commands").map_err(|e| format!("its commands: {e}"))?;
            let rows = q.query_map([], |row| row.get::<_, String>(0)).map_err(|e| format!("its commands: {e}"))?;
            for d in rows.flatten() { if let Ok(v) = serde_json::from_str::<Value>(&d) { cmds.push(v); } }
        }
        let names: HashMap<String, String> = cmds.iter().map(|c| (s(&c["ID"]), s(&c["Name"]))).collect();
        for c in &cmds { mixitup_command(c, &names, settings, &mut r); }
        if let Ok(mut q) = conn.prepare("SELECT Quote, DateTime FROM Quotes ORDER BY ID") {
            if let Ok(rows) = q.query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1).unwrap_or_default()))) {
                for (text, date) in rows.flatten() { r.quotes.push(json!({ "text": text, "by": "", "date": date.chars().take(10).collect::<String>() })); }
            }
        }
        Ok(r)
    })();
    let _ = std::fs::remove_dir_all(&dir);
    out
}

/// Restored into what's kept: its actions over those of the same name from the same place (OmniPulse's own
/// backup: over any of the same name), one from elsewhere with its place after its name ("Song Request (Mix It
/// Up)"), the rest added; its quotes added (none twice), and, from OmniPulse's own, its variables and points
pub fn merge(actions: &mut Vec<Value>, kept: &mut Map<String, Value>, r: &Restored) -> (usize, usize) {
    let ours = r.from == "an OmniPulse backup";
    // (an earlier import's extra copies, "Name 2"..., one for each trigger it then had: gone, as this
    // import's are one action now)
    if !ours {
        let made: Vec<String> = r.actions.iter().map(|a| s(&a["name"]).to_lowercase()).collect();
        actions.retain(|x| {
            if s(&x["from"]) != r.from { return true; }
            let name = s(&x["name"]).to_lowercase();
            let Some((base, n)) = name.rsplit_once(' ') else { return true };
            !(n.parse::<u32>().is_ok_and(|n| n >= 2) && made.iter().any(|m| m == base) && !made.contains(&name))
        });
    }
    for a in &r.actions {
        let mut a = a.clone();
        if !ours && a.get("from").is_none() { a["from"] = json!(r.from); }
        let name = s(&a["name"]);
        let from = s(&a["from"]);
        let same = |x: &Value| s(&x["name"]).eq_ignore_ascii_case(&name);
        match actions.iter_mut().find(|x| same(x) && (ours || s(&x["from"]) == from)) {
            Some(x) => *x = a,
            None => {
                if actions.iter().any(|x| same(x)) { a["name"] = json!(format!("{name} ({})", r.from)); }
                let again = s(&a["name"]);
                match actions.iter_mut().find(|x| s(&x["name"]).eq_ignore_ascii_case(&again)) { Some(x) => *x = a, None => actions.push(a) }
            }
        }
    }
    let mut quotes = kept.get("quotes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let before = quotes.len();
    for q in &r.quotes { if !quotes.iter().any(|x| s(&x["text"]) == s(&q["text"])) { quotes.push(q.clone()); } }
    let added = quotes.len() - before;
    kept.insert("quotes".into(), json!(quotes));
    for (key, from) in [("vars", &r.vars), ("points", &r.points)] {
        if from.is_empty() { continue; }
        let mut m = kept.get(key).and_then(|v| v.as_object()).cloned().unwrap_or_default();
        for (k, v) in from { m.insert(k.clone(), v.clone()); }
        kept.insert(key.into(), Value::Object(m));
    }
    (r.actions.len(), added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sb_export() -> Value {
        json!({ "meta": { "name": "Test" }, "data": {
            "actions": [
                { "id": "a1", "name": "Hello", "enabled": true, "randomAction": false,
                  "triggers": [{ "type": 401, "commandId": "c1", "enabled": true }, { "type": 101, "enabled": true }],
                  "subActions": [
                    { "type": 10, "text": "Hi %userName%!", "index": 0, "enabled": true, "parentId": null },
                    { "type": 1002, "value": 2500, "index": 1, "enabled": true, "parentId": null },
                    { "type": 99, "index": 2, "enabled": true, "parentId": null },
                    { "type": 34, "sceneName": "Main", "index": 3, "enabled": true, "parentId": null },
                    { "type": 4, "actionId": "a2", "index": 4, "enabled": true, "parentId": null },
                    { "type": 1007, "url": "http://127.0.0.1:7879/request?key=k&user=%user%&q=%rawInputUrlEncoded%", "variableName": "reply", "index": 5, "enabled": true, "parentId": null },
                    { "type": 120, "input": "%userType%", "operation": 0, "value": "Twitch", "index": 6, "enabled": true, "parentId": null, "subActions": [
                        { "type": 99901, "subActions": [{ "type": 560, "text": "%reply%", "enabled": true }], "enabled": true },
                        { "type": 99902, "subActions": [{ "type": 35002, "text": "%reply%", "enabled": true }], "enabled": true },
                    ] },
                    { "type": 1007, "url": "https://example.com/api?q=%rawInputUrlEncoded%&u=%userName%", "variableName": "answer", "index": 7, "enabled": true, "parentId": null },
                  ] },
                { "id": "a2", "name": "Timer", "enabled": false, "triggers": [{ "type": 701, "timerId": "t1", "enabled": true }], "subActions": [] },
            ],
            "commands": [{ "id": "c1", "name": "hello", "enabled": true, "command": "!hello\r\n!hi", "globalCooldown": 10, "userCooldown": 30 }],
            "timers": [{ "id": "t1", "interval": 600, "lines": 4 }],
        }, "version": 23 })
    }

    #[test]
    fn a_streamer_bot_export_becomes_actions() {
        // (as its import code: base64 of "SBAE" and gzipped JSON)
        use base64::Engine;
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(sb_export().to_string().as_bytes()).unwrap();
        let code = base64::engine::general_purpose::STANDARD.encode([b"SBAE".to_vec(), gz.finish().unwrap()].concat());
        let r = restore_text(&code).unwrap();
        assert_eq!(r.from, "Streamer.bot");
        // (its two triggers: two actions)
        assert_eq!(r.actions.len(), 3, "{:#?}", r.actions);
        let a = &r.actions[0];
        assert_eq!(a["trigger"]["command"], json!("!hello"));
        assert_eq!(a["trigger"]["aliases"], json!("!hi"));
        assert_eq!(a["trigger"]["usercooldown"], json!(30.0));
        assert_eq!(r.actions[1]["name"], json!("Hello 2"));
        assert_eq!(r.actions[1]["trigger"]["event"], json!("follow"));
        let steps = a["steps"].as_array().unwrap();
        assert_eq!(steps[0]["text"], json!("Hi %user%!"));
        assert_eq!(steps[1]["seconds"], json!(2.5));
        assert_eq!(steps[2]["type"], json!("note"));
        assert_eq!(steps[3]["scene"], json!("Main"));
        assert_eq!(steps[4]["action"], json!("Timer"));
        assert_eq!((steps[5]["type"].clone(), steps[5]["query"].clone(), steps[5]["reply"].clone()), (json!("request"), json!("%rawInput%"), json!(false)));
        // (the platform's If/Else: Twitch's branch, a reply)
        assert_eq!((steps[6]["type"].clone(), steps[6]["text"].clone(), steps[6]["reply"].clone()), (json!("say"), json!("%reply%"), json!(true)));
        assert_eq!(steps.len(), 8, "{steps:#?}");
        assert_eq!((steps[7]["type"].clone(), steps[7]["url"].clone(), steps[7]["into"].clone()), (json!("fetch"), json!("https://example.com/api?q=%rawInputUrlEncoded%&u=%user%"), json!("answer")));
        assert_eq!(r.missed, vec!["Execute Code".to_string()]);
        // (the timer: every 10 minutes, off as it was)
        assert_eq!((r.actions[2]["trigger"]["minutes"].clone(), r.actions[2]["on"].clone()), (json!(10.0), json!(false)));
    }

    #[test]
    fn a_mix_it_up_command_becomes_an_action() {
        let c = json!({ "$type": "MixItUp.Base.Model.Commands.ChatCommandModel, MixItUp.Base", "ID": "x", "Name": "Lurk", "Type": 1, "IsEnabled": true,
            "IncludeExclamation": true, "Triggers": ["lurk", "brb"], "Requirements": { "Cooldown": { "Type": 1, "Amount": 20 } },
            "Actions": [
                { "$type": "MixItUp.Base.Model.Actions.ChatActionModel, MixItUp.Base", "ChatText": "$username is lurking ($arg1text)", "Enabled": true },
                { "$type": "MixItUp.Base.Model.Actions.WaitActionModel, MixItUp.Base", "Amount": "3", "Enabled": true },
                { "$type": "MixItUp.Base.Model.Actions.ConsumablesActionModel, MixItUp.Base", "ActionType": 1, "Amount": "50", "Enabled": true },
                { "$type": "MixItUp.Base.Model.Actions.TextToSpeechActionModel, MixItUp.Base", "Enabled": true },
            ] });
        let r = restore_text(&c.to_string()).unwrap();
        assert_eq!(r.actions.len(), 1);
        let a = &r.actions[0];
        assert_eq!((a["trigger"]["command"].clone(), a["trigger"]["aliases"].clone(), a["trigger"]["usercooldown"].clone()), (json!("!lurk"), json!("!brb"), json!(20.0)));
        let st = a["steps"].as_array().unwrap();
        assert_eq!(st[0]["text"], json!("%user% is lurking (%input0%)"));
        assert_eq!(st[1]["seconds"], json!(3.0));
        assert_eq!((st[2]["type"].clone(), st[2]["amount"].clone()), (json!("points"), json!(-50.0)));
        assert_eq!(st[3]["type"], json!("note"));
        assert_eq!(r.missed, vec!["TextToSpeech".to_string()]);
    }

    #[test]
    fn a_mix_it_up_backup_reads_its_database() {
        let dir = std::env::temp_dir().join(format!("op-miu-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("s.db3");
        {
            let c = rusqlite::Connection::open(&db).unwrap();
            c.execute_batch("CREATE TABLE Commands (ID TEXT, TypeID INTEGER, Data TEXT); CREATE TABLE Quotes (ID INTEGER, Quote TEXT, DateTime TEXT, GameName TEXT);").unwrap();
            let cmd = json!({ "$type": "MixItUp.Base.Model.Commands.EventCommandModel, MixItUp.Base", "ID": "e", "Name": "Follow Thanks", "Type": 2, "EventType": 210, "IsEnabled": true,
                "Actions": [{ "$type": "MixItUp.Base.Model.Actions.ChatActionModel, MixItUp.Base", "ChatText": "Thanks $username!" }] });
            c.execute("INSERT INTO Commands VALUES ('e', 2, ?1)", [cmd.to_string()]).unwrap();
            c.execute("INSERT INTO Quotes VALUES (1, 'It works on my machine', '2025-04-01T12:00:00+00:00', 'Coding')", []).unwrap();
        }
        let mut zbytes = Vec::new();
        {
            let mut z = zip::ZipWriter::new(std::io::Cursor::new(&mut zbytes));
            z.start_file("abc.db3", zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, &std::fs::read(&db).unwrap()).unwrap();
            z.start_file("abc.miu3", zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut z, b"{}").unwrap();
            z.finish().unwrap();
        }
        let _ = std::fs::remove_dir_all(&dir);
        let r = restore_zip(&zbytes).unwrap();
        assert_eq!(r.actions[0]["trigger"]["event"], json!("follow"));
        assert_eq!(r.actions[0]["steps"][0]["text"], json!("Thanks %user%!"));
        assert_eq!(r.quotes, vec![json!({ "text": "It works on my machine", "by": "", "date": "2025-04-01" })]);
    }

    /// Real exports and backups, from a folder given as OMNIPULSE_BOT_SAMPLES (each restored, what came over said)
    #[test]
    #[ignore]
    fn sample_backups_restore() {
        let Some(dir) = std::env::var_os("OMNIPULSE_BOT_SAMPLES") else { return };
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let r = restore_file(&e.path());
            match r {
                Ok(r) => println!("{}: {} from {}, {} quotes, missed {:?}; first {}", e.file_name().to_string_lossy(), r.actions.len(), r.from, r.quotes.len(), r.missed, r.actions.first().map(|a| a.to_string()).unwrap_or_default()),
                Err(err) => println!("{}: {err}", e.file_name().to_string_lossy()),
            }
        }
    }

    #[test]
    fn our_own_backup_comes_back_as_it_was() {
        let actions = vec![json!({ "name": "A", "on": true, "trigger": defaults(TRIGGERS, "manual"), "steps": [] })];
        let b = backup(&actions, &json!({ "quotes": [{ "text": "q" }], "points": { "viewer": 5 } }));
        let r = restore_text(&b.to_string()).unwrap();
        let mut now = vec![json!({ "name": "a", "on": false, "trigger": defaults(TRIGGERS, "manual"), "steps": [] })];
        let mut kept = Map::new();
        assert_eq!(merge(&mut now, &mut kept, &r), (1, 1));
        // (the one of the same name replaced, not added again)
        assert_eq!(now.len(), 1);
        assert_eq!(now[0]["on"], json!(true));
        assert_eq!(kept["points"]["viewer"], json!(5));
        // (another bot's of the same name: beside it, its place after its name; again: over that one)
        let other = Restored { from: "Mix It Up", actions: vec![json!({ "name": "A", "on": false, "trigger": defaults(TRIGGERS, "manual"), "steps": [] })], ..Default::default() };
        merge(&mut now, &mut kept, &other);
        merge(&mut now, &mut kept, &other);
        assert_eq!(now.iter().map(|a| s(&a["name"])).collect::<Vec<_>>(), vec!["A".to_string(), "A (Mix It Up)".to_string()]);
    }
    #[test]
    fn streamer_bot_games_rewards_and_desktop_steps_come_across() {
        let st = |t: u64, idx: i64, extra: Value| { let mut x = json!({ "type": t, "index": idx, "enabled": true, "parentId": null }); for (k, v) in extra.as_object().unwrap() { x[k] = v.clone(); } x };
        let branch = |t: u64, subs: Vec<Value>| json!({ "type": t, "subActions": subs });
        let data = json!({ "actions": [
            { "id": "a1", "name": "Category > Game", "group": "Category Switching", "enabled": true,
              "triggers": [{ "type": 703, "processName": "gamelauncher", "enabled": true }, { "type": 703, "processName": "", "enabled": true }],
              "subActions": [
                st(35031, 0, json!({ "categoryName": "Some Game" })),
                st(1011, 1, json!({ "groupName": "Game > Some", "state": 0 })),
                st(6, 2, json!({ "command": "https://example.com/inventory", "arguments": null })),
                st(120, 3, json!({ "input": "%path%", "operation": 2, "value": "Some Game", "subActions": [branch(99901, vec![st(16, 0, json!({ "game": "Some Game" }))]), branch(99902, vec![st(10, 0, json!({ "text": "Not that one" }))])] })),
              ] },
            { "id": "a2", "name": "Reward", "enabled": true, "triggers": [{ "type": 112, "rewardId": "abc-123", "enabled": true }, { "type": 35024, "rewardId": "01KICK", "enabled": true }],
              "subActions": [st(1012, 0, json!({ "key": 86, "modifiers": 0 })), st(19, 1, json!({ "duration": 90 }))] },
            { "id": "a3", "name": "Reply", "enabled": true, "triggers": [],
              "subActions": [st(120, 0, json!({ "input": "%userType%", "operation": 0, "value": "Twitch", "subActions": [branch(99901, vec![st(560, 0, json!({ "text": "Hi" }))]), branch(99902, vec![st(35002, 0, json!({ "text": "Hi" }))])] }))] },
        ] });
        let r = streamerbot(&data);
        let a = &r.actions[0];
        assert_eq!(a["group"], json!("Category Switching"));
        assert_eq!(a["trigger"]["type"], json!("process"));
        assert_eq!(a["trigger"]["name"], json!("gamelauncher"));
        let types: Vec<String> = a["steps"].as_array().unwrap().iter().map(|x| s(&x["type"])).collect();
        assert_eq!(types, vec!["note", "game", "groupstate", "open", "ifblock", "game", "else", "say", "endif"]);
        assert_eq!(a["steps"][1]["where"], json!("kick"));
        assert_eq!(a["steps"][2]["group"], json!("Game > Some"));
        assert_eq!(a["steps"][3]["target"], json!("https://example.com/inventory"));
        assert_eq!(a["steps"][4]["test"], json!("contains"));
        let b = &r.actions[1];
        // (its Twitch reward and its Kick one: one action, either reward)
        assert_eq!(b["trigger"]["rewardid"], json!("abc-123, 01KICK"));
        assert_eq!(b["trigger"]["from"], json!("any"));
        assert_eq!(r.actions[2]["name"], json!("Reply"));
        assert_eq!(b["steps"][0]["keys"], json!("V"));
        assert_eq!(b["steps"][1]["type"], json!("ad"));
        assert_eq!(b["steps"][1]["seconds"], json!(90.0));
        // (the same reply on either platform: one step, ours answering where it was asked)
        assert_eq!(r.actions[2]["steps"].as_array().unwrap().len(), 1);
    }
}
