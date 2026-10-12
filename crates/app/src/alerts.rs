// The widget's alerts, and their order (the web app's js/widget.js and js/alertmedia.js), as settings
// pages: the list (each switched on or off, opened, tried, moved up or down: the higher, the sooner when
// several are waiting; + a custom one, set off by a web request with its trigger), and an alert's own
// page (its name and words, colours, how long, animation, icon or picture, sound with its level, delay
// and whether the app plays it itself). The web app's prefs: alertOrder, alertOff, alertEdit,
// alertCustom, alertSounds, alertImages, alertLevels. (A picture's crop and a collage's own pictures are
// still set in the web app.)

use serde_json::{json, Map, Value};

/// [id, name in Settings, test values, heading, main line]
pub const KINDS: [(&str, &str, &str, &str, &str); 11] = [
    ("follow", "Follows", r#"{"user":"A new friend"}"#, "New Follower", "{user}"),
    ("sub", "Subscriptions", r#"{"user":"A loyal viewer","amount":"6","message":"Six months already!"}"#, "New Subscriber", "{user}"),
    ("gift", "Gifted Subs", r#"{"user":"A generous viewer","amount":"5"}"#, "Gifted Subs", "{user} gifted {amount} subs"),
    ("bits", "Bits", r#"{"user":"A cheerful viewer","amount":"500","message":"gg!"}"#, "{amount} Bits", "{user}"),
    ("kicks", "Kicks", r#"{"user":"A kind viewer","amount":"100","message":"Love it"}"#, "{amount} Kicks", "{user}"),
    ("donation", "Donations", r#"{"user":"A kind viewer","amount":"$10.00","message":"Love the stream"}"#, "Donation", "{user} · {amount}"),
    ("raid", "Raids & Hosts", r#"{"user":"A fellow streamer","amount":"42"}"#, "{amount} Raiders", "{user} is here!"),
    ("hypetrain", "Hype Trains", r#"{"amount":"3"}"#, "Hype Train", "Level {amount}"),
    ("poll", "Polls", r#"{"user":"Which track next?","message":"Vote now!"}"#, "Poll", "{user}"),
    ("ban", "Bans", r#"{"user":"A troll"}"#, "Banned", "{user}"),
    ("watchstreak", "Watch Streaks", r#"{"user":"A regular","amount":"10"}"#, "Watch Streak", "{user} · {amount} streams"),
];
const ICONS: [(&str, &str); 17] = [
    ("follow", "Heart"), ("sub", "Star"), ("gift", "Gift"), ("bits", "Gem"), ("kicks", "Lightning"), ("donation", "Money"),
    ("raid", "People"), ("hypetrain", "Train"), ("poll", "Chart"), ("ban", "No Entry"), ("watchstreak", "Flame"),
    ("bell", "Bell"), ("trophy", "Trophy"), ("hammer", "Hammer"), ("rocket", "Rocket"), ("music", "Music"), ("sparkle", "Sparkles"),
];
const PALETTES: [(&str, &str); 11] = [("alert", "Alert's Colours"), ("rainbow", "Rainbow"), ("neon", "Neon"), ("sunset", "Sunset"), ("ocean", "Ocean"), ("forest", "Forest"), ("candy", "Candy"), ("fire", "Fire"), ("streamer", "Streaming Sites"), ("mono", "Black & White"), ("custom", "Your Own")];
const COLLAGE_MAX: usize = 6;
const ANIMS: [(&str, &str); 5] = [("slide", "Slide"), ("pop", "Pop"), ("fade", "Fade"), ("bounce", "Bounce"), ("flip", "Flip")];

fn builtin(k: &str) -> Option<&'static (&'static str, &'static str, &'static str, &'static str, &'static str)> { KINDS.iter().find(|x| x.0 == k) }

fn custom(prefs: &Value) -> Vec<Value> {
    prefs["alertCustom"].as_array().cloned().unwrap_or_default().into_iter().filter(|c| c["id"].as_str().is_some_and(|s| !s.is_empty())).collect()
}

fn edits(prefs: &Value, k: &str) -> Map<String, Value> { prefs["alertEdit"][k].as_object().cloned().unwrap_or_default() }

pub fn name(prefs: &Value, k: &str) -> String {
    if let Some(n) = edits(prefs, k).get("name").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) { return n.to_string(); }
    if let Some(b) = builtin(k) { return b.1.to_string(); }
    custom(prefs).iter().find(|c| c["id"] == k).and_then(|c| c["name"].as_str().map(String::from)).unwrap_or_else(|| k.to_string())
}

/// Every alert, highest first: your order, then any new ones
pub fn order(prefs: &Value) -> Vec<String> {
    let all: Vec<String> = KINDS.iter().map(|k| k.0.to_string()).chain(custom(prefs).iter().filter_map(|c| c["id"].as_str().map(String::from))).collect();
    let mut out: Vec<String> = prefs["alertOrder"].as_array().into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).filter(|k| all.contains(k)).collect();
    for k in all { if !out.contains(&k) { out.push(k); } }
    out
}

fn off(prefs: &Value, k: &str) -> bool { prefs["alertOff"].as_array().is_some_and(|l| l.iter().any(|x| x == k)) }

// ---------- the list ----------
pub fn list_values(prefs: &Value) -> Map<String, Value> {
    order(prefs).into_iter().map(|k| (format!("on:{k}"), json!(!off(prefs, &k)))).collect()
}

pub fn list_fields(prefs: &Value) -> Vec<Value> {
    let ks = order(prefs);
    let mut f = vec![json!({ "key": "", "label": "Which alerts show, and which goes first when several are waiting (the higher, the sooner). Edit for each one's words, look, picture and sound; Test sends it to the widget.", "note": "", "type": "note" })];
    let n = ks.len();
    for (i, k) in ks.iter().enumerate() {
        let mut acts = vec![format!("alEdit:{k}"), format!("alTest:{k}")];
        let mut labels = vec!["Edit".to_string(), "Test".to_string()];
        if i > 0 { acts.push(format!("alUp:{k}")); labels.push("\u{2191}".into()); }
        if i + 1 < n { acts.push(format!("alDown:{k}")); labels.push("\u{2193}".into()); }
        let note = if builtin(k).is_none() { format!("Your own: set off by type={k}") } else { String::new() };
        f.push(json!({ "key": format!("on:{k}"), "label": name(prefs, k), "note": note, "type": "row", "actions": acts, "labels": labels }));
    }
    f.push(json!({ "key": "alNew", "label": "", "note": "One of your own, set off by a web request with its trigger.", "type": "button", "value": "New Alert" }));
    f
}

/// Saved: the ones switched off
pub fn list_save(prefs: &Value, vals: &Map<String, Value>) -> Vec<(&'static str, Value)> {
    let offs: Vec<Value> = order(prefs).into_iter().filter(|k| vals.get(&format!("on:{k}")).and_then(|v| v.as_bool()) == Some(false)).map(|k| json!(k)).collect();
    vec![("alertOff", json!(offs))]
}

/// Moved up (by -1) or down (by 1)
pub fn moved(prefs: &Value, k: &str, by: i32) -> Value {
    let mut ks = order(prefs);
    if let Some(i) = ks.iter().position(|x| x == k) {
        let j = (i as i32 + by).clamp(0, ks.len() as i32 - 1) as usize;
        ks.swap(i, j);
    }
    json!(ks)
}

/// A new custom alert's trigger, and the list with it
pub fn added(prefs: &Value) -> (String, Value) {
    let have = |id: &str| builtin(id).is_some() || custom(prefs).iter().any(|c| c["id"] == id);
    let (mut n, mut id) = (1, "custom".to_string());
    while have(&id) { n += 1; id = format!("custom-{n}"); }
    let mut all = custom(prefs);
    all.push(json!({ "id": id, "name": format!("New Alert{}", if n > 1 { format!(" {n}") } else { String::new() }) }));
    (id, json!(all))
}

/// A custom alert gone, with its settings (its files are taken out by the caller)
pub fn deleted(prefs: &Value, k: &str) -> Vec<(&'static str, Value)> {
    let without = |key: &str| { let mut m = prefs[key].as_object().cloned().unwrap_or_default(); m.remove(k); Value::Object(m) };
    let list = |key: &str| json!(prefs[key].as_array().cloned().unwrap_or_default().into_iter().filter(|x| x != k).collect::<Vec<_>>());
    vec![
        ("alertCustom", json!(custom(prefs).into_iter().filter(|c| c["id"] != k).collect::<Vec<_>>())),
        ("alertEdit", without("alertEdit")), ("alertSounds", without("alertSounds")), ("alertImages", without("alertImages")), ("alertLevels", without("alertLevels")), ("alertCollage", without("alertCollage")),
        ("alertOrder", list("alertOrder")), ("alertOff", list("alertOff")),
    ]
}

/// What Test sends: its test values (a custom one: someone, 1)
pub fn test_alert(k: &str) -> Map<String, Value> {
    let sample: Value = builtin(k).and_then(|b| serde_json::from_str(b.2).ok()).unwrap_or_else(|| json!({ "user": "Someone", "amount": "1", "message": "" }));
    let mut m = Map::new();
    for (a, b) in [("type", json!(k)), ("user", json!("")), ("amount", json!("")), ("message", json!(""))] { m.insert(a.into(), b); }
    if let Some(o) = sample.as_object() { for (a, b) in o { m.insert(a.clone(), b.clone()); } }
    m.insert("test".into(), json!(true));
    m
}

// ---------- an alert's own page ----------
const EDIT_KEYS: [&str; 10] = ["name", "head", "main", "color", "color2", "secs", "anim", "icon", "sdelay", "music"];

pub fn edit_values(prefs: &Value, k: &str) -> Map<String, Value> {
    let e = edits(prefs, k);
    let mut m = Map::new();
    for key in EDIT_KEYS { m.insert(format!("e.{key}"), e.get(key).cloned().unwrap_or(json!(""))); }
    m.insert("own".into(), json!(e.get("color").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty())));
    if m["e.color"] == "" { m.insert("e.color".into(), json!("#8e9bff")); }
    if m["e.color2"] == "" { m.insert("e.color2".into(), json!("#b38cff")); }
    m.insert("e.music".into(), json!(e.get("music").and_then(|v| v.as_bool()).unwrap_or(false)));
    m.insert("adj".into(), json!(prefs["alertLevels"][k]["adj"].as_f64().unwrap_or(0.0)));
    // its picture's crop (the web app's: where the middle is, 0 to 100; zoom 1 to 4; fit or fill)
    let c = e.get("crop").cloned().unwrap_or(Value::Null);
    m.insert("crop.fit".into(), json!(c["fit"].as_str().unwrap_or("contain")));
    m.insert("crop.z".into(), json!(c["z"].as_f64().unwrap_or(1.0)));
    m.insert("crop.x".into(), json!(c["x"].as_f64().unwrap_or(50.0)));
    m.insert("crop.y".into(), json!(c["y"].as_f64().unwrap_or(50.0)));
    // its collage: the icons chosen, the tiles' colours
    let chosen: Vec<String> = e.get("cicons").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    for (id, _) in ICONS { m.insert(format!("ci:{id}"), json!(chosen.iter().any(|c| c == id))); }
    m.insert("cpal".into(), json!(e.get("cpal").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("alert")));
    let own: Vec<String> = e.get("cpalOwn").and_then(|v| v.as_array()).into_iter().flatten().filter_map(|x| x.as_str().map(String::from)).collect();
    m.insert("cpalOwn".into(), json!(if own.is_empty() { "#ff4d6d, #ffb703, #4cc9f0".to_string() } else { own.join(", ") }));
    if builtin(k).is_none() { m.insert("e.name".into(), json!(name(prefs, k))); }
    m
}

pub fn edit_fields(prefs: &Value, k: &str, vals: &Map<String, Value>) -> Vec<Value> {
    let b = builtin(k);
    let s = |key: &str| vals.get(key).and_then(|v| v.as_str()).unwrap_or("").to_string();
    let field = |key: &str, label: &str, note: &str, kind: &str| json!({ "key": key, "label": label, "note": note, "type": kind });
    let sel = |key: &str, label: &str, note: &str, opts: Vec<(Value, String)>| json!({ "key": key, "label": label, "note": note, "type": "select", "options": opts.iter().map(|o| o.0.clone()).collect::<Vec<_>>(), "labels": opts.iter().map(|o| o.1.clone()).collect::<Vec<_>>() });
    let button = |key: String, label: &str, press: &str, note: &str| json!({ "key": key, "label": label, "note": note, "type": "button", "value": press });
    let note = |t: &str| json!({ "key": "", "label": t, "note": "", "type": "note" });
    let mut f = Vec::new();
    match b {
        Some(b) => f.push(field("e.name", "Name", &format!("In Settings. Empty: {}.", b.1), "text")),
        None => {
            f.push(field("e.name", "Name", "", "text"));
            f.push(json!({ "key": "", "label": "Its Trigger", "note": "Your bot's web request sets it off with this type.", "type": "info", "value": format!("type={k}"), "copy": k }));
        }
    }
    let (head, main) = b.map(|b| (b.3, b.4)).unwrap_or(("{user}", "{message}"));
    f.push(field("e.head", "Heading", &format!("Empty: \"{head}\". {{user}}, {{amount}} and {{message}} are filled in."), "text"));
    f.push(field("e.main", "Main Line", &format!("Empty: \"{main}\"."), "text"));
    f.push(field("own", "Its Own Colours", "Off: the colours set for all the alerts.", "toggle"));
    if vals.get("own").and_then(|v| v.as_bool()) == Some(true) {
        f.push(field("e.color", "Colour", "As #rrggbb: where its gradient starts.", "text"));
        f.push(field("e.color2", "Second Colour", "As #rrggbb: where it ends.", "text"));
    }
    let has_sound = prefs["alertSounds"][k].is_object();
    let mut secs: Vec<(Value, String)> = vec![(json!(""), "As set for all".into())];
    if has_sound || s("e.secs") == "sound" { secs.push((json!("sound"), "As long as its sound".into())); }
    for n in [3, 5, 8, 12, 20] { secs.push((json!(n), format!("{n} s"))); }
    f.push(sel("e.secs", "Show For", "", secs));
    let collage = prefs["alertTrans"].as_str() == Some("collage");
    f.push(sel("e.anim", "Animation", if collage { "Not used with Collage: its tiles bring the alert in." } else { "" }, std::iter::once((json!(""), "As set for all".to_string())).chain(ANIMS.iter().map(|(v, l)| (json!(v), l.to_string()))).collect()));
    // the picture: an icon, or your own
    match prefs["alertImages"][k]["name"].as_str() {
        Some(file) => {
            f.push(note(&format!("Picture: {file}")));
            f.push(button(format!("alPic:{k}"), "", "Change Picture", ""));
            f.push(button(format!("alPicClear:{k}"), "", "Use an Icon", ""));
            let steps = |from: f64, to: f64, by: f64, show: &dyn Fn(f64) -> String| { let mut o = Vec::new(); let mut v = from; while v <= to + 1e-9 { o.push((json!((v * 100.0).round() / 100.0), show(v))); v += by; } o };
            f.push(sel("crop.fit", "Crop", "", vec![(json!("contain"), "Fit in the Tile".into()), (json!("cover"), "Fill the Tile".into())]));
            f.push(sel("crop.z", "Zoom", "", steps(1.0, 4.0, 0.25, &|v| format!("{v}×"))));
            f.push(sel("crop.x", "Across", "Which part is in the middle: 0 is its left edge, 100 its right.", steps(0.0, 100.0, 10.0, &|v| format!("{v}"))));
            f.push(sel("crop.y", "Down", "0 is its top, 100 its bottom.", steps(0.0, 100.0, 10.0, &|v| format!("{v}"))));
        }
        None => {
            let dflt = if b.is_some() { k } else { "bell" };
            let mut icons: Vec<(Value, String)> = vec![(json!(""), format!("{} (its own)", ICONS.iter().find(|i| i.0 == dflt).map(|i| i.1).unwrap_or("Bell")))];
            icons.extend(ICONS.iter().filter(|i| i.0 != dflt).map(|(v, l)| (json!(v), l.to_string())));
            f.push(sel("e.icon", "Icon", "", icons));
            f.push(button(format!("alPic:{k}"), "", "Image or GIF...", "Your own picture (a GIF, say) in place of the icon."));
        }
    }
    // its sound
    match prefs["alertSounds"][k]["name"].as_str() {
        Some(file) => {
            let lv = &prefs["alertLevels"][k];
            let secs = lv["ms"].as_u64().map(|ms| format!(", {:.1} s", ms as f64 / 1000.0)).unwrap_or_default();
            let auto = lv["auto"].as_f64().map(|a| format!(" Measured: {}{a} dB to match the others.", if a > 0.0 { "+" } else { "" })).unwrap_or_default();
            f.push(note(&format!("Sound: {file}{secs}.{auto}")));
            let mut adj = Vec::new();
            let mut v = -12.0;
            while v <= 12.0 { adj.push((json!(v), format!("{}{v} dB", if v > 0.0 { "+" } else { "" }))); v += 1.0; }
            f.push(sel("adj", "Its Level", "Yours, on top of what was measured.", adj));
            f.push(field("e.sdelay", "Delay", "In ms: how long after the alert shows its sound starts.", "number"));
            f.push(field("e.music", "Musical Alert", "The app plays its sound itself, pausing the music for it and fading the music back in after.", "toggle"));
            f.push(button(format!("alSnd:{k}"), "", "Change Sound", ""));
            f.push(button(format!("alSndClear:{k}"), "", "Remove Sound", ""));
        }
        None => f.push(button(format!("alSnd:{k}"), "Sound", "Add a Sound...", "Measured as it's added, so it plays about as loud as the others.")),
    }
    // its collage (with the Collage transition): its icons, its own pictures, its tiles' colours
    f.push(note(if prefs["alertTrans"].as_str() == Some("collage") { "Collage: its tiles show its own picture, then these (nothing chosen: its icon, with a couple that go with it)." } else { "Collage (with the Collage transition): its tiles show its own picture, then these." }));
    for (id, label) in ICONS { f.push(field(&format!("ci:{id}"), &format!("Icon: {label}"), "", "toggle")); }
    let pics = prefs["alertCollage"][k].as_array().cloned().unwrap_or_default();
    for i in 0..COLLAGE_MAX {
        match pics.get(i).and_then(|p| p["name"].as_str()) {
            Some(file) => f.push(button(format!("clrm:{i}:{k}"), &format!("Picture {}: {file}", i + 1), "Take It Out", "")),
            None => f.push(button(format!("cladd:{i}:{k}"), &format!("Picture {}", i + 1), "Add...", "")),
        }
    }
    f.push(sel("cpal", "Tile Colours", "", PALETTES.iter().map(|(v, l)| (json!(v), l.to_string())).collect()));
    if s("cpal") == "custom" { f.push(field("cpalOwn", "Your Colours", "Two to six, as #rrggbb, with commas between.", "text")); }
    f.push(note("Each tile's own position and background are set in the web app for now."));
    if b.is_none() { f.push(button(format!("alDelete:{k}"), "", "Delete This Alert", "")); }
    else if !edits(prefs, k).is_empty() { f.push(button(format!("alReset:{k}"), "", "Back to How It Was", "")); }
    f
}

/// Saved: its own settings (empty ones left out), its name (a custom one's), its level
pub fn edit_save(prefs: &Value, k: &str, vals: &Map<String, Value>) -> Vec<(&'static str, Value)> {
    let mut e = edits(prefs, k);
    let own = vals.get("own").and_then(|v| v.as_bool()) == Some(true);
    for key in EDIT_KEYS {
        let v = vals.get(&format!("e.{key}")).cloned().unwrap_or(json!(""));
        let empty = v.is_null() || v == "" || v == json!(false) || (key == "sdelay" && v.as_f64() == Some(0.0)) || (matches!(key, "color" | "color2") && !own);
        if empty || (key == "name" && builtin(k).is_none()) { e.remove(key); } else { e.insert(key.into(), v); }
    }
    // the crop: kept unless it's as it starts
    let n = |key: &str, d: f64| vals.get(key).and_then(|v| v.as_f64()).unwrap_or(d);
    let fit = vals.get("crop.fit").and_then(|v| v.as_str()).unwrap_or("contain").to_string();
    let (cx, cy, cz) = (n("crop.x", 50.0), n("crop.y", 50.0), n("crop.z", 1.0));
    if prefs["alertImages"][k].is_object() && (fit != "contain" || (cx - 50.0).abs() > 1e-9 || (cy - 50.0).abs() > 1e-9 || (cz - 1.0).abs() > 1e-9) {
        e.insert("crop".into(), json!({ "x": cx.round(), "y": cy.round(), "z": (cz * 100.0).round() / 100.0, "fit": fit }));
    } else { e.remove("crop"); }
    let chosen: Vec<&str> = ICONS.iter().map(|i| i.0).filter(|id| vals.get(&format!("ci:{id}")).and_then(|v| v.as_bool()) == Some(true)).collect();
    if chosen.is_empty() { e.remove("cicons"); } else { e.insert("cicons".into(), json!(chosen)); }
    let pal = vals.get("cpal").and_then(|v| v.as_str()).unwrap_or("alert").to_string();
    if pal == "alert" { e.remove("cpal"); } else { e.insert("cpal".into(), json!(pal)); }
    if pal == "custom" {
        let own: Vec<String> = vals.get("cpalOwn").and_then(|v| v.as_str()).unwrap_or("").split(',').map(|c| c.trim().to_string())
            .filter(|c| c.len() == 7 && c.starts_with('#') && c[1..].chars().all(|h| h.is_ascii_hexdigit())).take(COLLAGE_MAX).collect();
        if own.len() >= 2 { e.insert("cpalOwn".into(), json!(own)); }
    }
    let mut all = prefs["alertEdit"].as_object().cloned().unwrap_or_default();
    if e.is_empty() { all.remove(k); } else { all.insert(k.into(), Value::Object(e)); }
    let mut out = vec![("alertEdit", Value::Object(all))];
    if builtin(k).is_none() {
        let n = vals.get("e.name").and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
        out.push(("alertCustom", json!(custom(prefs).into_iter().map(|mut c| { if c["id"] == k && !n.is_empty() { c["name"] = json!(n); } c }).collect::<Vec<_>>())));
    }
    if prefs["alertLevels"][k].is_object() {
        let mut lv = prefs["alertLevels"].as_object().cloned().unwrap_or_default();
        if let Some(o) = lv.get_mut(k).and_then(|x| x.as_object_mut()) { o.insert("adj".into(), vals.get("adj").cloned().unwrap_or(json!(0))); }
        out.push(("alertLevels", Value::Object(lv)));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_list_its_order_and_an_alerts_own_settings() {
        let p = json!({ "alertOrder": ["raid", "follow"], "alertOff": ["bits"] });
        let o = order(&p);
        assert_eq!(&o[..3], ["raid", "follow", "sub"]);
        assert_eq!(list_values(&p)["on:bits"], false);
        assert_eq!(moved(&p, "follow", -1)[0], "follow");
        let (id, all) = added(&p);
        assert_eq!(id, "custom");
        let p2 = json!({ "alertCustom": all });
        assert!(order(&p2).contains(&"custom".to_string()));
        assert_eq!(name(&p2, "custom"), "New Alert");
        // an alert's page: the heading set, its colours only with its own on
        let mut v = edit_values(&p, "follow");
        v.insert("e.head".into(), json!("Thanks {user}!"));
        let saved = edit_save(&p, "follow", &v);
        assert_eq!(saved[0].1["follow"]["head"], "Thanks {user}!");
        assert!(saved[0].1["follow"].get("color").is_none());
        assert_eq!(test_alert("raid")["amount"], "42");
        // its collage: icons chosen, a palette of your own (the bad colour left out)
        let mut v = edit_values(&p, "raid");
        v.insert("ci:rocket".into(), json!(true));
        v.insert("cpal".into(), json!("custom"));
        v.insert("cpalOwn".into(), json!("#112233, nope, #445566"));
        let e = &edit_save(&p, "raid", &v)[0].1["raid"];
        assert_eq!(e["cicons"], json!(["rocket"]));
        assert_eq!(e["cpalOwn"], json!(["#112233", "#445566"]));
        assert!(e.get("crop").is_none());
    }
}
