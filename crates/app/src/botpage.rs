// The chatbot's own settings page (Settings > Streaming > OmniPulse Chatbot): what goes on it, worked out
// from the chatbot's settings (as saved, with the page's changes over them) and what the chatbot says is going
// on (asked every couple of seconds while the page is open). The first time it's a path of steps; after
// that its card, its services, what it does, and what it's done lately. See docs/ease-of-use-pass.md.

use serde_json::{json, Map, Value};

pub const BOT: &str = "chatbot";

/// OmniPulse's own apps, so you only sign in: their Client IDs (empty: not registered yet, so you make your own).
/// Kick's secret isn't here: the relay keeps it and finishes Kick's sign-ins (jam-server, /api/kick/*)
pub const TWITCH_ID: &str = "122hyd1hxg8qpi1nfzwc4oa85lolgd";
pub const KICK_ID: &str = "01M5NZG55J6HQ93XPKVQ8CHRY8";

/// Your own app wanted instead (or OmniPulse's not there yet)
fn own_app(cur: &Map<String, Value>, plat: &str) -> bool {
    let built = if plat == "kick" { KICK_ID } else { TWITCH_ID };
    built.is_empty() || on(cur, if plat == "kick" { "kickOwnApp" } else { "twitchOwnApp" }, false)
}

/// What a sign-in's given besides the page's values: OmniPulse's apps, and where to finish a Kick sign-in
pub fn sign_in_extras(vals: &mut Map<String, Value>, relay: &str) {
    // (Twitch's: its Authorize page, finished by the relay; your own public app: by device code, as before)
    if !own_app(vals, "twitch") { vals.insert("twitchClientId".into(), json!(TWITCH_ID)); vals.insert("twitchRelay".into(), json!(relay)); } else { vals.insert("twitchRelay".into(), json!("")); }
    if !own_app(vals, "kick") {
        vals.insert("kickClientId".into(), json!(KICK_ID));
        vals.insert("kickClientSecret".into(), json!(""));
        vals.insert("kickRelay".into(), json!(relay));
    } else {
        vals.insert("kickRelay".into(), json!(""));
    }
}

thread_local! {
    /// Tips and Donations' platforms clicked open
    pub static TIPS_OPEN: std::cell::RefCell<std::collections::HashSet<String>> = std::cell::RefCell::new(std::collections::HashSet::new());
    /// The webcam overlay's setup: the step it's on
    pub static CAM_STEP: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The webcam overlay's setup, step by step
pub const CAM_STEPS: [&str; 6] = ["Your Camera", "Frame", "Look", "What's On It", "Arrange", "Add to OBS"];

/// A setting that changes only how widgets look or behave: taken by the running Suite as it's saved, no
/// restart. Anything to do with an account, a connection, a sign-in, commands or actions isn't
pub fn live_key(k: &str) -> bool {
    const LIVE: [&str; 25] = ["goal", "timer", "suite", "panel", "cam", "hud", "rain", "wheel", "hype", "skip", "sponsor", "gif", "tune", "leader", "latest", "raid", "theme", "ab", "train", "widgetsOff", "social", "fun", "voteskip", "credits", "vis"];
    const NEVER: [&str; 12] = ["token", "login", "client", "secret", "relay", "refresh", "kick", "userid", "channel", "botname", "obs", "url"];
    let low = k.to_lowercase();
    LIVE.iter().any(|p| k.starts_with(p)) && !NEVER.iter().any(|n| low.contains(n))
}

#[cfg(test)]
mod live_tests {
    #[test]
    fn looks_are_live_and_accounts_restart() {
        for k in ["goalStyle", "suiteTheme", "panelTheme", "camBorder", "rainDir", "wheelColors", "hypeShow", "timerLabel", "widgetsOff"] { assert!(super::live_key(k), "{k}"); }
        for k in ["twitchToken", "kickToken", "botLogin", "kofiToken", "streamlabsToken", "commands", "actions", "obsOn", "kofiUrl"] { assert!(!super::live_key(k), "{k}"); }
    }
}

/// One line of a sheet or a step, as the page draws it (BotField)
#[derive(Clone)]
pub struct Field { pub key: String, pub kind: &'static str, pub label: String, pub note: String, pub value: String, pub extra: String, pub n: i32, pub cols: Vec<String> }

fn f(kind: &'static str, key: &str, label: &str, note: &str, value: &str, extra: &str) -> Field {
    Field { key: key.into(), kind, label: label.into(), note: note.into(), value: value.into(), extra: extra.into(), n: 0, cols: Vec::new() }
}

/// A widget's address in OBS, copied with its icon (the widget server's port, as set)
fn addr(path: &str, note: &str) -> Field {
    let port = crate::SHARED.with(|s| s.borrow().as_ref().and_then(|sh| sh.try_borrow().ok().map(|a| a.web.prefs["widgetPort"].as_u64().unwrap_or(7878)))).unwrap_or(7878);
    // (the raid's clip plays only from localhost: Twitch's player checks where it's opened)
    let host = if path == "/raid" { "localhost" } else { "127.0.0.1" };
    f("addr", "", "Address", note, &format!("http://{host}:{port}{path}"), "")
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
/// The Suite's widgets' theme, as it's set now (one for all of them)
fn theme_of(cur: &Map<String, Value>) -> String { if s(cur, "suiteTheme").is_empty() { "dark".into() } else { s(cur, "suiteTheme").to_string() } }

/// The wheel's prizes as the widget has them (up to 16; "x2" on the end of one, twice as likely), each with
/// where its slice starts, in degrees from the top, as big as its chance
pub fn wheel_slices(list: &str) -> Vec<(String, f32)> {
    let list = if list.trim().is_empty() { "A shoutout | 100 points | Pick the next song | Nothing! | Hydrate" } else { list };
    let items: Vec<(String, u32)> = list.split('|').map(str::trim).filter(|p| !p.is_empty()).take(16).map(|p| {
        let w = p.rsplit_once([' ', '\t']).and_then(|(name, end)| end.strip_prefix(['x', 'X', '\u{d7}']).and_then(|k| k.parse::<u32>().ok()).filter(|k| (1..=99).contains(k)).map(|k| (name.trim().to_string(), k.min(20))));
        w.unwrap_or((p.to_string(), 1))
    }).collect();
    let total = items.iter().map(|x| x.1).sum::<u32>().max(1) as f32;
    let mut at = 0.0;
    items.into_iter().map(|(name, w)| { let start = at; at += w as f32 / total * 360.0; (name, start) }).collect()
}

/// A label in Title Case, as the whole app has them: each word's first letter a capital, but the short joining
/// words ("a", "of", "to" and the like) unless first or last, or after a colon. A word already with a capital or
/// a digit in it is left as it is; a full sentence (ending in . or ?) is left alone
pub fn title_case(label: &str) -> String {
    const SMALL: [&str; 17] = ["a", "an", "the", "and", "but", "or", "nor", "for", "so", "as", "at", "by", "in", "of", "on", "per", "to"];
    if label.ends_with('.') || label.ends_with('?') { return label.to_string(); }
    let words: Vec<&str> = label.split(' ').collect();
    let last = words.len().saturating_sub(1);
    let mut after_colon = false;
    words.iter().enumerate().map(|(i, w)| {
        let start = i == 0 || i == last || after_colon;
        after_colon = w.ends_with(':');
        let bare = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        if w.chars().any(|c| c.is_uppercase() || c.is_ascii_digit()) || (!start && SMALL.contains(&bare.as_str())) { return w.to_string(); }
        let mut out = String::with_capacity(w.len());
        let mut done = false;
        for c in w.chars() { if !done && c.is_alphabetic() { out.extend(c.to_uppercase()); done = true; } else { out.push(c); } }
        out
    }).collect::<Vec<_>>().join(" ")
}

/// Its choice, on each widget's sheet (one setting, shared)
fn theme_field(cur: &Map<String, Value>) -> Field {
    f("chips", "suiteTheme", "Theme", "For all of the Suite's widgets. Sticker is made for a GIF on top.", &theme_of(cur), "dark=Dark|glass=Glass|neon=Neon|comic=Comic|minimal=Minimal|pastel=Pastel|y2k=Y2K|pixel=Pixel|edge=Edge Light|sticker=Sticker")
}

/// A list's items, as kept ("a | b | c"); an empty one kept, so a row just added shows
pub fn list_items(v: &str) -> Vec<String> {
    if v.trim().is_empty() { return Vec::new(); }
    v.split('|').map(|x| x.trim().to_string()).collect()
}

/// A channel panel as you've written it: its title, its words (**bold**, *italics*, __underline__; lines starting "- " or "1. " a list), and a
/// picture of your own for its icon ("" for none), its icon (a panelicon key; "" for the one its title picks),
/// and its picture made into an icon (its address; "" to show the picture as it is)
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Panel { pub title: String, pub body: String, pub img: String, pub icon: String, pub mask: String }

/// The most panels made
pub const PANELS_MAX: usize = 12;
const PANELS_FIRST: &str = "About Me | My Music | PC Specs | Sponsors | Donate";

/// Your panels: as kept (panelItems), else as they were before it (panelList's "Title: words | ...", " / " a
/// new line, and panelOwnIcons in their order)
pub fn panels(cur: &Map<String, Value>) -> Vec<Panel> {
    if let Some(a) = serde_json::from_str::<Value>(s(cur, "panelItems")).ok().and_then(|v| v.as_array().cloned()) {
        let t = |p: &Value, k: &str| p[k].as_str().unwrap_or("").to_string();
        // (words typed in the app's own boxes once kept Windows' line ends: lines as the page and plugin read them)
        return a.iter().take(PANELS_MAX).map(|p| Panel { title: t(p, "title"), body: t(p, "body").replace("\r\n", "\n").replace('\r', "\n"), img: t(p, "img"), icon: t(p, "icon"), mask: t(p, "mask") }).collect();
    }
    let list = if s(cur, "panelList").is_empty() { PANELS_FIRST } else { s(cur, "panelList") };
    let icons: Vec<String> = s(cur, "panelOwnIcons").split('|').map(|x| x.trim().to_string()).collect();
    list.split('|').map(str::trim).filter(|x| !x.is_empty()).take(PANELS_MAX).enumerate().map(|(i, p)| {
        let (t, b) = p.split_once(':').map(|(a, b)| (a.trim(), b.trim())).unwrap_or((p, ""));
        Panel { title: t.into(), body: b.replace(" / ", "\n"), img: icons.get(i).cloned().unwrap_or_default(), ..Default::default() }
    }).collect()
}

/// Your panels as kept (panelItems)
pub fn panels_json(list: &[Panel]) -> String {
    Value::Array(list.iter().map(|p| serde_json::json!({ "title": p.title, "body": p.body, "img": p.img, "icon": p.icon, "mask": p.mask })).collect()).to_string()
}

/// A panel's words as plain text (its marks taken out, its lists' bullets and numbers kept)
pub fn rich_plain(text: &str) -> String {
    text.lines().map(|l| {
        let (mark, rest) = list_mark(l);
        let words = rest.replace("**", "").replace("__", "").replace('*', "");
        match mark { Some(m) if m.starts_with('-') || m.starts_with('\u{2022}') || m.starts_with('*') => format!("\u{2022} {words}"), Some(m) => format!("{} {words}", m.trim()), None => words }
    }).collect::<Vec<_>>().join("\n")
}

/// A panel's words as the app shows them formatted (Slint's markdown): its lines paragraphs, lists' lines a list
/// each, underline as <u>, anything else markdown reads as its own (# [ ` < and the like) kept as written
pub fn rich_md(text: &str) -> String {
    let inline = |s: &str| {
        let mut out = String::new();
        let mut u = false;
        let mut rest = s;
        while let Some(c) = rest.chars().next() {
            if rest.starts_with("**") { out.push_str("**"); rest = &rest[2..]; continue; }
            if rest.starts_with("__") { out.push_str(if u { "</u>" } else { "<u>" }); u = !u; rest = &rest[2..]; continue; }
            if c != '*' && "\\`#[]<>!~|_&".contains(c) { out.push('\\'); }
            out.push(c);
            rest = &rest[c.len_utf8()..];
        }
        if u { out.push_str("</u>"); }
        out
    };
    let mut out = String::new();
    let mut last: Option<u8> = None;
    for line in text.replace('\r', "").lines() {
        if line.trim().is_empty() { continue; }
        let (mark, rest) = list_mark(line);
        // (0 a paragraph, 1 a bulleted list's line, 2 a numbered one's)
        let kind = match mark { None => 0, Some(m) if m.starts_with(|c: char| c.is_ascii_digit()) => 2, _ => 1 };
        // (a paragraph from the line before, or another list: a blank line between)
        if let Some(was) = last { out.push_str(if kind > 0 && kind == was { "\n" } else { "\n\n" }); }
        match mark {
            Some(m) if m.starts_with(|c: char| c.is_ascii_digit()) => { out.push_str(m.trim_end()); out.push(' '); }
            Some(_) => out.push_str("- "),
            None => {}
        }
        out.push_str(&inline(rest.trim()));
        last = Some(kind);
    }
    out
}

/// A line's list mark, if it starts with one ("- ", "\u{2022} ", "* ", "1. ", "2) "), and the rest of it
fn list_mark(line: &str) -> (Option<&str>, &str) {
    for b in ["- ", "\u{2022} ", "* "] { if let Some(r) = line.strip_prefix(b) { return (Some(&line[..b.len()]), r); } }
    let digits = line.bytes().take_while(|c| c.is_ascii_digit()).count();
    if (1..=3).contains(&digits) && (line[digits..].starts_with(". ") || line[digits..].starts_with(") ")) { return (Some(&line[..digits + 2]), &line[digits + 2..]); }
    (None, line)
}

/// The webcam overlay's parts: (id, name, on at first, place, size)
pub const CAM_PARTS: [(&str, &str, bool, &str, &str); 12] = [("name", "Name Tag", true, "below", "medium"), ("live", "LIVE Badge", true, "corner", "small"), ("now", "Now Playing", false, "above", "medium"), ("alerts", "Alerts", true, "above", "large"), ("follow", "Latest Follow", false, "left", "medium"), ("sub", "Latest Sub", false, "left", "medium"), ("goals", "Goals", false, "left", "medium"), ("hype", "Hype Train", false, "above", "medium"), ("chat", "Chat", false, "left", "medium"), ("socials", "Social Plug", false, "below", "medium"), ("clock", "Clock", false, "in-top", "small"), ("sticker", "A GIF or Picture", false, "corner", "medium")];

/// The sides a part can go on round the camera, and the places inside it, in the order they're laid out
pub const CAM_SIDES: [(&str, &str); 6] = [("above", "Above"), ("below", "Below"), ("left", "Left"), ("right", "Right"), ("in-top", "Inside, Top"), ("in-bottom", "Inside, Bottom")];

/// A webcam overlay part where it's put: its id, its name, what it shows (the name tag's words), its box (pixels on a 1920 × 1080 canvas), and the
/// place it ended up in
#[derive(Clone, Debug, PartialEq)]
pub struct CamPart { pub id: String, pub name: String, pub label: String, pub x: f32, pub y: f32, pub w: f32, pub h: f32, pub place: String }

/// The webcam overlay laid out: the camera first, then each part that's on; the parts moved to the other side
/// for want of room (name, the side they went to); and the parts that still overlap (two names; the camera's
/// "your camera"; a part off the canvas's edge, its name and "")
#[derive(Clone, Debug, Default)]
pub struct CamArranged { pub parts: Vec<CamPart>, pub moved: Vec<(String, String)>, pub overlaps: Vec<(String, String)> }

/// The webcam overlay laid out as cam.html lays it out (the two kept the same), on a 1920 × 1080 canvas. Parts
/// sharing a side are stacked (one after another, out from the camera) or side by side, as that side's flow is
/// set ("auto" picks stacked if there's room for it); a side without the room for its parts has them moved to the
/// other side if there's room there; each side's parts are kept on the canvas together
pub fn cam_arrange(cur: &Map<String, Value>) -> CamArranged {
    let num = |k: &str, d: f32| cur.get(k).and_then(|v| v.as_f64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))).map(|v| v as f32).unwrap_or(d);
    let or = |k: &str, d: &str| if s(cur, k).is_empty() { d.to_string() } else { s(cur, k).to_string() };
    let (bx, by, bw, bh) = match or("camSpot", "br").as_str() {
        "bl" => (2.0, 62.0, 26.0, 34.0), "tr" => (72.0, 4.0, 26.0, 34.0), "tl" => (2.0, 4.0, 26.0, 34.0),
        "custom" => (num("camX", 72.0), num("camY", 62.0), num("camW", 26.0), num("camH", 34.0)),
        _ => (72.0, 62.0, 26.0, 34.0),
    };
    let parts: Vec<(String, String, String, String)> = CAM_PARTS.iter().filter(|p| on(cur, &format!("camP_{}", p.0), p.2))
        .map(|(id, name, _, place, size)| (id.to_string(), name.to_string(), or(&format!("camP_{id}Place"), place), or(&format!("camP_{id}Size"), size))).collect();
    let flows: Vec<(String, String)> = CAM_SIDES.iter().map(|(k, _)| (k.to_string(), or(&format!("camFlow_{k}"), "auto"))).collect();
    let mut a = cam_arrange_in(1920.0, 1080.0, (bx, by, bw, bh), &parts, &flows);
    for p in a.parts.iter_mut().filter(|p| p.id == "name") { p.label = or("camNameText", "Your Name"); }
    a
}

/// The layout itself (see cam_arrange): the canvas's size, the camera's box (percent of it), the parts on
/// (id, name, place, size), and each side's flow ("row", "column" or "auto")
pub fn cam_arrange_in(cw: f32, ch: f32, cam: (f32, f32, f32, f32), parts: &[(String, String, String, String)], flows: &[(String, String)]) -> CamArranged {
    let (bx, by, bw, bh) = (cam.0 / 100.0 * cw, cam.1 / 100.0 * ch, cam.2 / 100.0 * cw, cam.3 / 100.0 * ch);
    let gap = 10.0f32;
    // (each part's size: from the camera's width, its shape as its widget's)
    let sized: Vec<(String, String, String, f32, f32)> = parts.iter().map(|(id, name, place, size)| {
        let k = match size.as_str() { "small" => 0.6, "large" => 1.1, _ => 0.85 };
        let ratio = match id.as_str() { "now" | "alerts" => 3.6, "follow" | "sub" | "hype" => 4.6, "goals" => 1.8, "chat" => 0.7, "socials" => 4.2, "name" => 4.4, "live" | "clock" => 3.2, _ => 1.0 };
        let (mut w, mut h) = (bw * k, bw * k / ratio);
        if id == "chat" { h = bh; w = h * ratio; }
        if place == "corner" { w *= 0.45; h *= 0.45; }
        let place = if CAM_SIDES.iter().any(|s| s.0 == place) || place == "corner" { place.clone() } else { "below".to_string() };
        (id.clone(), name.clone(), place, w, h)
    }).collect();
    let flow_set = |side: &str| flows.iter().find(|f| f.0 == side).map_or("auto", |f| f.1.as_str()).to_string();
    // (the room a side's parts take out from the camera: above and below, stacked their heights one after
    // another, side by side the tallest; left and right, stacked (one under another) the widest, side by side
    // their widths one after another)
    let need = |side: &str, items: &[usize], flow: &str| -> f32 {
        let deep: Vec<f32> = items.iter().map(|&i| if side == "left" || side == "right" { sized[i].3 } else { sized[i].4 }).collect();
        let all = deep.iter().sum::<f32>() + gap * (deep.len() as f32 - 1.0).max(0.0);
        let most = deep.iter().cloned().fold(0.0, f32::max);
        if (side == "left" || side == "right") == (flow == "column") { most } else { all }
    };
    let room = |side: &str| match side { "above" => by - gap, "below" => ch - by - bh - gap, "left" => bx - gap, "right" => cw - bx - bw - gap, _ => bh - gap * 2.0 };
    // (auto: stacked if that fits, else side by side; inside the camera, side by side if they fit across it)
    let resolve = |side: &str, items: &[usize]| -> String {
        let set = flow_set(side);
        if set == "row" || set == "column" { return set; }
        match side {
            "left" | "right" => { let tall: f32 = items.iter().map(|&i| sized[i].4).sum::<f32>() + gap * (items.len() as f32 - 1.0).max(0.0); if tall <= ch { "column" } else { "row" }.into() }
            "in-top" | "in-bottom" => { let wide: f32 = items.iter().map(|&i| sized[i].3).sum::<f32>() + gap * (items.len() as f32 + 1.0); if wide <= bw { "row" } else { "column" }.into() }
            _ => if need(side, items, "column") <= room(side) { "column".into() } else { "row".into() },
        }
    };
    let mut groups: Vec<(String, Vec<usize>)> = CAM_SIDES.iter().map(|(k, _)| (k.to_string(), (0..sized.len()).filter(|&i| sized[i].2 == *k).collect())).collect();
    let mut moved = Vec::new();
    // (a side without room for its parts: them on the other side, if they'd fit there with what's there already)
    for (a, b) in [("below", "above"), ("above", "below"), ("right", "left"), ("left", "right")] {
        let ia = groups.iter().position(|g| g.0 == a).unwrap_or(0);
        let ib = groups.iter().position(|g| g.0 == b).unwrap_or(0);
        if groups[ia].1.is_empty() { continue; }
        let here = need(a, &groups[ia].1, &resolve(a, &groups[ia].1));
        if here <= room(a) { continue; }
        let mut both = groups[ib].1.clone();
        both.extend(groups[ia].1.iter().cloned());
        let there = need(b, &both, &resolve(b, &both));
        if there <= room(b) || room(b) > room(a) {
            for &i in &groups[ia].1 { moved.push((sized[i].1.clone(), b.to_string())); }
            let mut items = std::mem::take(&mut groups[ia].1);
            groups[ib].1.append(&mut items);
        }
    }
    let mut out = vec![CamPart { id: "cam".into(), name: "Your camera".into(), label: "Your camera".into(), x: bx, y: by, w: bw, h: bh, place: String::new() }];
    let mut place_of = vec![String::new(); sized.len()];
    let mut boxes = vec![(0f32, 0f32, 0f32, 0f32); sized.len()];
    for (side, items) in &groups {
        if items.is_empty() { continue; }
        let flow = resolve(side, items);
        let n = items.len() as f32;
        let sum_w: f32 = items.iter().map(|&i| sized[i].3).sum::<f32>() + gap * (n - 1.0);
        let sum_h: f32 = items.iter().map(|&i| sized[i].4).sum::<f32>() + gap * (n - 1.0);
        let (cx, cy) = (bx + bw / 2.0, by + bh / 2.0);
        let mut at = 0.0f32;
        for &i in items {
            let (w, h) = (sized[i].3, sized[i].4);
            let (x, y) = match (side.as_str(), flow.as_str()) {
                ("above", "column") => { let y = by - gap - at - h; at += h + gap; (cx - w / 2.0, y) }
                ("above", _) => { let x = cx - sum_w / 2.0 + at; at += w + gap; (x, by - gap - h) }
                ("below", "column") => { let y = by + bh + gap + at; at += h + gap; (cx - w / 2.0, y) }
                ("below", _) => { let x = cx - sum_w / 2.0 + at; at += w + gap; (x, by + bh + gap) }
                ("left", "column") => { let y = cy - sum_h / 2.0 + at; at += h + gap; (bx - gap - w, y) }
                ("left", _) => { let x = bx - gap - at - w; at += w + gap; (x, cy - h / 2.0) }
                ("right", "column") => { let y = cy - sum_h / 2.0 + at; at += h + gap; (bx + bw + gap, y) }
                ("right", _) => { let x = bx + bw + gap + at; at += w + gap; (x, cy - h / 2.0) }
                ("in-top", "column") => { let y = by + gap + at; at += h + gap; (cx - w / 2.0, y) }
                ("in-top", _) => { let x = cx - sum_w / 2.0 + at; at += w + gap; (x, by + gap) }
                ("in-bottom", "column") => { let y = by + bh - gap - at - h; at += h + gap; (cx - w / 2.0, y) }
                (_, _) => { let x = cx - sum_w / 2.0 + at; at += w + gap; (x, by + bh - gap - h) }
            };
            boxes[i] = (x, y, w, h);
            place_of[i] = side.clone();
        }
        // (the side's parts kept on the canvas together, so they don't slide onto each other)
        let (x0, y0) = items.iter().fold((f32::MAX, f32::MAX), |a, &i| (a.0.min(boxes[i].0), a.1.min(boxes[i].1)));
        let (x1, y1) = items.iter().fold((f32::MIN, f32::MIN), |a, &i| (a.0.max(boxes[i].0 + boxes[i].2), a.1.max(boxes[i].1 + boxes[i].3)));
        let dx = if x0 < 0.0 { -x0 } else if x1 > cw { (cw - x1).max(-x0) } else { 0.0 };
        let dy = if y0 < 0.0 { -y0 } else if y1 > ch { (ch - y1).max(-y0) } else { 0.0 };
        for &i in items { boxes[i].0 += dx; boxes[i].1 += dy; }
    }
    // (on the corner: each in the first of its spots that's free, else the first: on the camera's top right
    // corner, then just inside it, inside its bottom right, top left and bottom left; more than one, each further
    // in along the edge)
    let free = |b: (f32, f32, f32, f32), boxes: &[(f32, f32, f32, f32)]| boxes.iter().all(|o| {
        o.2 <= 0.0 || (b.0 + b.2).min(o.0 + o.2) - b.0.max(o.0) <= 2.0 || (b.1 + b.3).min(o.1 + o.3) - b.1.max(o.1) <= 2.0
    });
    let mut at = 0.0f32;
    for i in (0..sized.len()).filter(|&i| sized[i].2 == "corner") {
        let (w, h) = (sized[i].3, sized[i].4);
        let spots = [(bx + bw - w * 0.6 - at, by - h * 0.45), (bx + bw - gap - w - at, by + gap), (bx + bw - gap - w - at, by + bh - gap - h), (bx + gap + at, by + gap), (bx + gap + at, by + bh - gap - h)];
        let fit = |(x, y): (f32, f32)| (x.clamp(0.0, (cw - w).max(0.0)), y.clamp(0.0, (ch - h).max(0.0)), w, h);
        boxes[i] = spots.iter().map(|&s| fit(s)).find(|&b| free(b, &boxes)).unwrap_or_else(|| fit(spots[0]));
        at += w + gap;
        place_of[i] = "corner".into();
    }
    for (i, p) in sized.iter().enumerate() {
        out.push(CamPart { id: p.0.clone(), name: p.1.clone(), label: p.1.clone(), x: boxes[i].0, y: boxes[i].1, w: boxes[i].2, h: boxes[i].3, place: place_of[i].clone() });
    }
    // (what still overlaps: two parts over each other by more than a sliver; a part outside the camera over it)
    let over = |a: &CamPart, b: &CamPart| {
        let (ix, iy) = ((a.x + a.w).min(b.x + b.w) - a.x.max(b.x), (a.y + a.h).min(b.y + b.h) - a.y.max(b.y));
        ix > 2.0 && iy > 2.0 && ix * iy > 0.04 * (a.w * a.h).min(b.w * b.h)
    };
    let mut overlaps = Vec::new();
    for i in 1..out.len() {
        // (and one partly off the canvas: with "" for what it's over)
        let p = &out[i];
        if p.x < -2.0 || p.y < -2.0 || p.x + p.w > cw + 2.0 || p.y + p.h > ch + 2.0 { overlaps.push((p.name.clone(), String::new())); }
        let inside = out[i].place.starts_with("in-") || out[i].place == "corner";
        if !inside && over(&out[0], &out[i]) { overlaps.push((out[i].name.clone(), "your camera".to_string())); }
        for j in i + 1..out.len() { if over(&out[i], &out[j]) { overlaps.push((out[i].name.clone(), out[j].name.clone())); } }
    }
    CamArranged { parts: out, moved, overlaps }
}

/// The widgets' colours, as hex: a palette's, your own, or none (your accent)
pub fn suite_colours(cur: &Map<String, Value>) -> Option<(String, String)> {
    colours_of(cur, s(cur, "suiteColors"), "suiteC1", "suiteC2")
}

/// A colour typed in as hex (#rrggbb), if it is one
pub fn hex_of(cur: &Map<String, Value>, k: &str) -> Option<String> {
    let v = s(cur, k).trim().to_string();
    (v.len() == 7 && v.starts_with('#') && v[1..].chars().all(|c| c.is_ascii_hexdigit())).then_some(v)
}

/// The panels' colours: the Suite widgets' (the default), a palette, or their own two (None: the app's)
pub fn panel_colours(cur: &Map<String, Value>) -> Option<(String, String)> {
    match s(cur, "panelColors") {
        "" | "suite" => suite_colours(cur),
        k => colours_of(cur, k, "panelC1", "panelC2"),
    }
}

/// The visualiser's colours in OBS: a palette, its own two, or None (your theme's, as the app's)
pub fn vis_colours(cur: &Map<String, Value>) -> Option<(String, String)> {
    colours_of(cur, s(cur, "visColors"), "visC1", "visC2")
}

/// A palette by name, or "custom": the two typed in (c1, c2 their keys); "accent" or anything else: None
fn colours_of(cur: &Map<String, Value>, which: &str, c1: &str, c2: &str) -> Option<(String, String)> {
    let hex = |k: &str, d: &str| hex_of(cur, k).unwrap_or_else(|| d.to_string());
    let (a, b) = match which {
        "sunset" => ("#ff8a5c", "#ff4f8b"), "ocean" => ("#3ec7ff", "#5a6bff"), "candy" => ("#ff8fd8", "#b48eff"), "mint" => ("#5ef2b8", "#3ab0ff"),
        "fire" => ("#ffb03a", "#ff3d3d"), "gold" => ("#ffd76e", "#ff9f43"), "mono" => ("#f2f2f2", "#9a9aa6"),
        "custom" => return Some((hex(c1, "#8e9bff"), hex(c2, "#ff7ac8"))),
        _ => return None,
    };
    Some((a.into(), b.into()))
}

/// The fonts offered for a theme (any other Google Font can be typed in)
pub const FONTS: [&str; 14] = ["Inter", "Poppins", "Montserrat", "Rubik", "Fredoka", "Baloo 2", "Bebas Neue", "Bangers", "Orbitron", "Press Start 2P", "Pacifico", "Permanent Marker", "Space Grotesk", "Righteous"];

/// Each theme's own font, as picked (themeFonts): none, the theme's own
pub fn theme_font(cur: &Map<String, Value>, theme: &str) -> String {
    serde_json::from_str::<Value>(s(cur, "themeFonts")).ok().and_then(|m| m[theme].as_str().map(String::from)).unwrap_or_default()
}

/// A Suite widget's sheet, with its look's choices after its theme: the colours, and a GIF with it
pub fn sheet(id: &str, cur: &Map<String, Value>) -> (String, Vec<Field>) {
    let (title, mut v) = sheet_of(id, cur);
    // (Colours, wherever it is: "Yours" is the app theme's own two, and Your Own shows its two boxes)
    for x in v.iter_mut().filter(|x| x.key == "suiteColors") {
        x.extra = x.extra.replace("accent=Yours", "accent=Your Theme's");
        x.note = "Your Theme's means the two colours of your app theme (App & Look), not the album art.".into();
    }
    if s(cur, "suiteColors") == "custom" && !v.iter().any(|x| x.key == "suiteC1") {
        if let Some(at) = v.iter().position(|x| x.key == "suiteColors") {
            v.insert(at + 1, f("text", "suiteC2", "Second colour", "", s(cur, "suiteC2"), "#ff7ac8"));
            v.insert(at + 1, f("text", "suiteC1", "First colour", "As hex, like #8e9bff. Your theme's colours and the album art's don't change it.", s(cur, "suiteC1"), "#8e9bff"));
        }
    }
    // (a font for the theme picked here: kept for that theme, any Google Font)
    if let Some(at) = v.iter().position(|x| matches!(x.key.as_str(), "suiteTheme" | "panelTheme" | "camTheme")) {
        let theme = if v[at].value.is_empty() { "dark".to_string() } else { v[at].value.clone() };
        let now = theme_font(cur, &theme);
        // (each option "theme:font", as "|" is what's between options)
        let opts = std::iter::once(format!("{theme}:=Theme's Own")).chain(FONTS.iter().map(|n| format!("{theme}:{n}={n}"))).collect::<Vec<_>>().join("|");
        let picked = if now.is_empty() || FONTS.contains(&now.as_str()) { format!("{theme}:{now}") } else { String::new() };
        let fonts = vec![
            f("chips", "fontPick", "Font", "For this theme. Each theme keeps its own.", &picked, &opts),
            f("textenter", "fontCustom", "Or any Google Font", "Type its name as fonts.google.com shows it, like Lobster, then press Enter or click Use It.", if picked.is_empty() { &now } else { "" }, "Lobster"),
        ];
        let after = v.iter().position(|x| x.key == "suiteColors").filter(|c| *c > at).map(|c| c + if s(cur, "suiteColors") == "custom" { 3 } else { 1 }).unwrap_or(at + 1);
        v.splice(after..after, fonts);
    }
    let gif = match id { "goals" => "gifGoals", "timer" => "gifTimer", "latest" => "gifLatest", "raid" => "gifRaid", "hype" => "gifHype", "voteskip" => "gifVoteskip", "wheel" => "gifWheel", "leaderboard" => "gifLeaderboard", "tune" => "gifTune", _ => "" };
    if let Some(at) = v.iter().position(|x| x.key == "suiteTheme") {
        let pal = if s(cur, "suiteColors").is_empty() { "accent" } else { s(cur, "suiteColors") };
        let mut more = vec![f("chips", "suiteColors", "Colours", "For all of the Suite's widgets. Yours follows the app's accent (or the album art's).", pal, "accent=Yours|sunset=Sunset|ocean=Ocean|candy=Candy|mint=Mint|fire=Fire|gold=Gold|mono=Mono|custom=Your Own")];
        if pal == "custom" {
            more.push(f("text", "suiteC1", "First colour", "As hex, like #8e9bff.", s(cur, "suiteC1"), "#8e9bff"));
            more.push(f("text", "suiteC2", "Second colour", "", s(cur, "suiteC2"), "#ff7ac8"));
        }
        if !gif.is_empty() {
            more.push(f("text", gif, "A GIF with it", "A GIF's address (from GIPHY or Tenor: right-click the GIF, Copy Image Address), shown with this widget. Leave it empty for none.", s(cur, gif), "https://…/your.gif"));
            if !s(cur, gif).trim().is_empty() {
                more.push(f("chips", "gifPlace", "Where it goes", "", if s(cur, "gifPlace").is_empty() { "top" } else { s(cur, "gifPlace") }, "top=On Top|left=Left|right=Right|corner=In the Corner"));
                more.push(f("chips", "gifSize", "GIF size", "", if s(cur, "gifSize").is_empty() { "medium" } else { s(cur, "gifSize") }, "small=Small|medium=Medium|large=Large"));
            }
        }
        v.splice(at + 1..at + 1, more);
    }
    // (a widget's page, the same order on each: In OBS (its address, and its Try buttons) at the top, under
    // its preview; then its own settings; then the look every widget shares; then its Timing and Motion)
    if matches!(id, "goals" | "timer" | "latest" | "raid" | "hype" | "voteskip" | "wheel" | "leaderboard" | "tune" | "emoterain" | "credits" | "vis") {
        let obs = |x: &Field| x.kind == "addr" || (x.kind == "link" && x.label.starts_with("Try"));
        let (mut top, rest): (Vec<Field>, Vec<Field>) = std::mem::take(&mut v).into_iter().partition(|x| obs(x));
        // (its address first, then its Try buttons)
        top.sort_by_key(|x| x.kind != "addr");
        let (prev, rest): (Vec<Field>, Vec<Field>) = rest.into_iter().partition(|x| x.kind == "suiteprev");
        v = prev;
        if !top.is_empty() { v.push(f("head", "", "In OBS", "", "", "")); v.extend(top); }
        if !rest.is_empty() && rest.first().is_some_and(|x| x.kind != "head") { v.push(f("head", "", "Settings", "", "", "")); }
        v.extend(rest);
        let shared = |k: &str| matches!(k, "suiteTheme" | "suiteColors" | "suiteC1" | "suiteC2" | "fontPick" | "fontCustom" | "gifPlace" | "gifSize") || k.starts_with("gif");
        let (look, own): (Vec<Field>, Vec<Field>) = std::mem::take(&mut v).into_iter().partition(|x| shared(&x.key));
        v = own;
        if !look.is_empty() {
            v.push(f("head", "", "Look, for every widget", "", "", ""));
            v.push(f("note", "", "The theme, colours and font are shared by all the Suite's widgets, so they match on stream.", "", "", ""));
            v.extend(look);
        }
    }
    // (its own Timing and Motion, last: how it comes in and goes out, and how fast)
    let motion = match id { "goals" | "timer" | "latest" | "raid" | "hype" | "voteskip" | "wheel" | "leaderboard" | "tune" => id, _ => "" };
    if !motion.is_empty() {
        let (a, sp) = (format!("{motion}Anim"), format!("{motion}Speed"));
        v.push(f("head", "", "Timing and Motion", "", "", ""));
        v.push(f("chips", &a, "How it comes in and goes out", "As the Suite's is the Suite's Widget Animation, for every widget.", s(cur, &a), "=As the Suite's|rise=Rise|slide=Slide|flip=Flip|pop=Pop|none=Still"));
        v.push(f("chips", &sp, "How fast", "", if s(cur, &sp).is_empty() { "normal" } else { s(cur, &sp) }, "slow=Slow|normal=Normal|fast=Fast"));
    }
    // (actions side by side: two or more in a row (not web links), one card of buttons, "act=Label|..."; their
    // notes, if any, as its note)
    let mut grouped: Vec<Field> = Vec::with_capacity(v.len());
    let act = |x: &Field| x.kind == "link" && !x.extra.starts_with("url:") && !x.extra.starts_with("http") && !x.extra.is_empty();
    let mut i = 0;
    while i < v.len() {
        let mut j = i;
        while j < v.len() && act(&v[j]) { j += 1; }
        if j - i >= 2 {
            let run: Vec<Field> = v[i..j].to_vec();
            let opts = run.iter().map(|x| format!("{}={}", x.extra.replace(['|', '='], " "), x.label)).collect::<Vec<_>>().join("|");
            let notes = run.iter().filter(|x| !x.note.is_empty()).map(|x| x.note.clone()).collect::<Vec<_>>().join(" ");
            grouped.push(f("buttons", "", "", &notes, "", &opts));
            i = j;
        } else { grouped.push(v[i].clone()); i += 1; }
    }
    let mut v = grouped;
    // (the preview: the colours as hex in its note, and whether there's a GIF)
    let col = suite_colours(cur).map(|(a, b)| format!("{a},{b}")).unwrap_or_default();
    let has_gif = !gif.is_empty() && !s(cur, gif).trim().is_empty();
    for x in v.iter_mut().filter(|x| x.kind == "suiteprev") { x.note = col.clone(); x.n = if has_gif { 1 } else { 0 }; }
    looks_shown(cur, &mut v);
    (title, v)
}

/// Colours and themes as you'd pick them by eye: a palette's choices each with its two colours beside it, your own
/// colours as a swatch to click (Windows' colour picker), the panels' and the webcam overlay's themes as a grid of
/// small previews. (Colours left "" are the app theme's own, filled in as the page is drawn)
fn looks_shown(cur: &Map<String, Value>, v: &mut [Field]) {
    for x in v.iter_mut() {
        match x.key.as_str() {
            "panelColors" | "suiteColors" | "visColors" if x.kind == "chips" => {
                x.kind = "palettes";
                let own = |a: &str, b: &str| (hex_of(cur, a).unwrap_or_default(), hex_of(cur, b).unwrap_or_default());
                x.cols = x.extra.split('|').filter_map(|o| o.split_once('=')).flat_map(|(id, _)| {
                    let (a, b) = match id {
                        "suite" => suite_colours(cur).unwrap_or_default(),
                        "custom" => match x.key.as_str() { "panelColors" => own("panelC1", "panelC2"), "visColors" => own("visC1", "visC2"), _ => own("suiteC1", "suiteC2") },
                        k => colours_of(cur, k, "", "").unwrap_or_default(),
                    };
                    [a, b]
                }).collect();
            }
            "panelLogoColours" if x.kind == "chips" => {
                x.kind = "palettes";
                let (a, b) = panel_colours(cur).unwrap_or_default();
                x.cols = vec![a, b, "#00e5ff".into(), "#ff2bd0".into(), "#ffffff".into(), "#ffffff".into()];
            }
            "panelTheme" if x.kind == "chips" => x.kind = "panelthemes",
            "camTheme" if x.kind == "chips" => x.kind = "camthemes",
            // (your own colours: a swatch; the words' and the background's can go back to the theme's own)
            "panelC1" | "panelC2" | "suiteC1" | "suiteC2" | "visC1" | "visC2" | "panelInk" | "panelBg" if x.kind == "text" => {
                x.kind = "colour";
                x.n = i32::from(matches!(x.key.as_str(), "panelInk" | "panelBg"));
                x.note = x.note.replace("As hex, like #8e9bff. ", "").replace("As hex, like #8e9bff.", "").replace("As hex, like #ffffff. ", "").replace("Leave it empty for the theme's own.", "").trim().to_string();
            }
            _ => {}
        }
    }
}

fn sheet_of(id: &str, cur: &Map<String, Value>) -> (String, Vec<Field>) {
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
            // (Twitch Shared Chat: a partner's chatters in your chat)
            v.push(f("head", "", "Shared Chat", "", "", ""));
            v.push(f("toggle", "sharedCommands", "Their chatters can use your commands", "When you share chat with another streamer, their viewers' messages show in your Chat Dock, marked with their channel. With this off, they can't use your song requests, actions, games or points. Auto-Ban applies to them either way.", if on(cur, "sharedCommands", false) { "true" } else { "false" }, ""));
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
        "youtube" => ("YouTube".into(), vec![
            f("toggle", "ytOn", "Read your YouTube live chat", "YouTube chat shows in your Chat Dock and the Multichat Overlay, its commands start your actions, and Super Chats, new members and gifted memberships become alerts.", if on(cur, "ytOn", false) { "true" } else { "false" }, ""),
            f("text", "ytChannel", "Your channel", "Your @handle, like @yourname, or your channel's link. OmniPulse finds your stream once you're live. A stream's own link works too.", s(cur, "ytChannel"), "@yourname"),
            f("note", "", "No sign-in needed: it reads your chat as YouTube's own chat window does. For now it's read only, so the Suite doesn't post replies in YouTube's chat.", "", "", ""),
        ]),
        "meld" => ("Meld Studio".into(), vec![
            f("toggle", "meldOn", "Connect to Meld Studio", "Your actions can switch scenes, show or hide layers, turn effects on or off, record clips, and more. Changing scenes or going live in Meld can start actions too.", if on(cur, "meldOn", false) { "true" } else { "false" }, ""),
            step(1, "Open Meld Studio on this PC. Nothing needs setting up in it: OmniPulse finds it.", "", "", ""),
            f("text", "meldAddress", "Address", "Leave this as it is if Meld runs on this PC. For another PC, turn on Allow remote connections in Meld's Settings > Advanced, and put that PC's address here.", s(cur, "meldAddress"), "127.0.0.1:13376"),
        ]),
        "slobs" => ("Streamlabs Desktop".into(), vec![
            f("toggle", "slobsOn", "Connect to Streamlabs Desktop", "Your actions can switch scenes, show or hide sources, mute, and start or stop the stream. Changing scenes or going live in Streamlabs can start actions too.", if on(cur, "slobsOn", false) { "true" } else { "false" }, ""),
            step(1, "In Streamlabs Desktop, open Settings, then Remote Control.", "", "", ""),
            step(2, "Click the QR code, then Show Details. Copy the API Token and paste it below.", "", "", ""),
            f("password", "slobsToken", "API Token", "", s(cur, "slobsToken"), "Paste the API Token here"),
            f("text", "slobsAddress", "Address", "Leave this as it is if Streamlabs runs on this PC.", s(cur, "slobsAddress"), "127.0.0.1:59650"),
        ]),
        "streamlabs" => ("Streamlabs".into(), vec![
            step(1, "Open your Streamlabs API settings and go to API Tokens.", "", "https://streamlabs.com/dashboard#/settings/api-settings", "Open Streamlabs' API settings"),
            step(2, "Copy your Socket API Token and paste it below.", "", "", ""),
            f("password", "streamlabsToken", "Socket API Token", "Donations then show as alerts, and can start your actions. Leave it empty to turn this off.", s(cur, "streamlabsToken"), "Paste your Socket API Token here"),
        ]),
        "software" => ("Streaming Software".into(), {
            let app = if s(cur, "streamApp").is_empty() { if on(cur, "meldOn", false) { "meld" } else if on(cur, "slobsOn", false) { "slobs" } else { "obs" } } else { s(cur, "streamApp") };
            let mut v = vec![
                f("note", "", "The app you stream with. Your actions can switch its scenes, show or hide sources, and more, and its scene changes can start actions. You can connect more than one.", "", "", ""),
                f("chips", "streamApp", "Your app", "", app, "obs=OBS|meld=Meld Studio|slobs=Streamlabs Desktop"),
            ];
            v.extend(sheet_of(app, cur).1);
            v
        }),
        "hud" => ("Game HUD Frames (WIP)".into(), {
            let frames = crate::hud::frames(s(cur, "hudFrames"));
            let sel = n(cur, "hudSel").parse::<f64>().unwrap_or(-1.0) as i64;
            let shot = !s(cur, "hudShotPath").is_empty();
            let mut v = Vec::new();
            if !shot {
                // (nothing yet: how it's done, and the two ways to start)
                v.push(f("note", "", "Frames round parts of your game's HUD (the minimap, health, ammo), on stream, in your colours. Here's how:", "", "", ""));
                v.push(step(1, "Take a screenshot of your game, at the resolution you stream it. Win+Shift+S (or the Print Screen key) works.", "", "", ""));
                v.push(step(2, "Paste it here, or choose the file if you saved one.", "", "", ""));
                v.push(step(3, "Drag a box round each part of the HUD you'd like framed, and it wraps round what's inside. For an odd shape, or one frame round several parts, click its corners yourself instead.", "", "", ""));
                v.push(f("link", "", "Paste a Screenshot", "Paste brings in what you just took with Win+Shift+S or Print Screen.", "", "hud:paste"));
                v.push(f("link", "", "Choose a Screenshot", "Choose opens a PNG or JPG of your game from your PC.", "", "hud:shot"));
                return ("Game HUD Frames (WIP)".into(), v);
            }
            v.push(f("hudedit", "", "", "", "", ""));
            // (each frame, to pick it here as well as on the picture)
            if !frames.is_empty() {
                let opts = frames.iter().enumerate().map(|(i, fr)| { let l = fr["label"].as_str().filter(|l| !l.is_empty()).map(|l| format!(": {l}")).unwrap_or_default(); format!("{i}=Frame {}{}", i + 1, l.replace(['|', '='], " ")) }).collect::<Vec<_>>().join("|");
                v.push(f("chips", "hudPick", "Your frames", "Click one here or on the screenshot to change it.", &sel.to_string(), &opts));
            }
            if let Some(fr) = usize::try_from(sel).ok().and_then(|i| frames.get(i)) {
                let g = |k: &str, d: &str| fr[k].as_str().map(String::from).unwrap_or_else(|| d.to_string());
                v.push(f("head", "", &format!("Frame {}: How It Looks", sel + 1), "", "", ""));
                v.push(f("chips", "hudF_style", "Style", "", &g("style", "brackets"), "brackets=Corner Brackets|glow=Glow|neon=Neon|double=Double Line|tech=Tech|soft=Soft Gradient|ribbon=Ribbon|dashed=Dashed|dots=Corner Dots|glass=Glass"));
                v.push(f("chips", "hudF_anim", "Animation", "A Light Running Round and Scan Line draw the eye most. Still is the calmest.", &fr["anim"].as_str().map(String::from).unwrap_or_else(|| if fr["pulse"].as_bool() == Some(true) { "pulse".into() } else { "none".into() }), "none=Still|pulse=Slow Pulse|breathe=Breathing Glow|run=A Light Running Round|march=Marching Dashes|spin=Spinning Segments|scan=Scan Line|draw=Draws Itself|flicker=Flicker"));
                v.push(f("chips", "hudF_thick", "Line", "", &fr["thick"].as_f64().unwrap_or(1.0).to_string(), "0.6=Thin|1=Normal|1.6=Thick|2.4=Heavy"));
                v.push(f("toggle", "hudF_round", "Rounded Corners", "", if fr["round"].as_bool() == Some(true) { "true" } else { "false" }, ""));
                v.push(f("toggle", "hudF_fill", "A Faint Tint Inside", "", if fr["fill"].as_bool() == Some(true) { "true" } else { "false" }, ""));
                v.push(f("chips", "hudF_shape", "Shape", "Traced follows the part's own outline in your screenshot, like a slanted ability slot. The others are drawn in its box.", &g("shape", "rect"), &crate::hud::SHAPES.iter().map(|(k, n)| format!("{k}={n}")).collect::<Vec<_>>().join("|")));
                v.push(f("text", "hudF_label", "Label", "Words above it, like MAP. Leave it empty for none.", &g("label", ""), "MAP"));
                v.push(f("chips", "hudF_labelStyle", "Label Style", "", &g("labelStyle", "plain"), "plain=Plain|glow=Glowing|tag=Tag|pill=Pill|cut=Cut Corner"));
                v.push(f("chips", "hudF_pad", "Room round it", "", &fr["pad"].as_f64().unwrap_or(6.0).to_string(), "0=None|6=A Little|14=More|24=Lots"));
                v.push(f("head", "", &format!("Frame {}: Where It Is", sel + 1), "", "", ""));
                v.push(f("buttons", "", "", "Move it a pixel at a time. On the screenshot, the arrow keys do the same (with Shift, 10 at a time).", "", "hudn:left=← Left|hudn:right=→ Right|hudn:up=↑ Up|hudn:down=↓ Down"));
                v.push(f("buttons", "", "", "Make it bigger or smaller, from its middle.", "", "hudn:wider=Wider|hudn:narrower=Narrower|hudn:taller=Taller|hudn:shorter=Shorter"));
                v.push(f("buttons", "", "", "Fit It Again wraps it round the part inside it again, after you've moved it. Same Look for All gives every frame this one's style, animation and line.", "", "hud:snap=Fit It Again|hud:same=Same Look for All|hud:del=Take It Away|hud:none=Done"));
            }
            v.push(f("head", "", "Look", "", "", ""));
            v.push(f("chips", "suiteColors", "Colours", "Shared with the Suite's widgets.", if s(cur, "suiteColors").is_empty() { "accent" } else { s(cur, "suiteColors") }, "accent=Yours|sunset=Sunset|ocean=Ocean|candy=Candy|mint=Mint|fire=Fire|gold=Gold|mono=Mono|custom=Your Own"));
            v.push(f("head", "", "In OBS", "", "", ""));
            v.push(addr("/hud", "Add it as a Browser source the size of your canvas, above your game. It lines up while the game runs at the screenshot's resolution and HUD scale; if you change either, take a new screenshot."));
            v.push(f("link", "", "Paste a new screenshot", "Your frames stay; move them if the HUD's moved.", "", "hud:paste"));
            v.push(f("link", "", "Choose a new screenshot", "", "", "hud:shot"));
            v
        }),
        "sponsors" => ("Sponsor Plug".into(), {
            let or = |k: &str, d: &'static str| if s(cur, k).is_empty() { d.to_string() } else { s(cur, k).to_string() };
            // (its preview: the first sponsor (or a made-up one), its name in look, its line in spot)
            let first = list_items(s(cur, "sponsorList")).into_iter().find(|x| !x.trim().is_empty()).unwrap_or_else(|| "Acme Energy: Code SHWYLIE for 10% off".into());
            let (name, line) = first.split_once(':').map(|(a, b)| (a.trim().to_string(), b.trim().to_string())).unwrap_or((first.trim().to_string(), String::new()));
            vec![
                // (its logo's letter in extra: the first of its name)
                f("suiteprev", &line, &name, "", "sponsors", &name.chars().next().map(|c| c.to_uppercase().to_string()).unwrap_or_else(|| "S".into())),
                f("note", "", "Your sponsors, one at a time (or all together), each with its logo, its name and a line: a code, a link or a deal.", "", "", ""),
                f("list", "sponsorList", "Your sponsors", "One each, its name, a colon, then its line, like Acme Energy: Code SHWYLIE for 10% off.", s(cur, "sponsorList"), ""),
                f("list", "sponsorLogos", "Their logos", "A logo's address for each, in the same order (a PNG with a see-through background is best). Leave one empty for its first letter.", s(cur, "sponsorLogos"), ""),
                f("text", "sponsorTag", "Heading", "Shown small above each one. Leave it empty for none.", s(cur, "sponsorTag"), "Sponsored by"),
                f("head", "", "How it looks", "", "", ""),
                f("chips", "sponsorStyle", "Style", "", &or("sponsorStyle", "card"), "card=Card|pill=Pill|glass=Glass|neon=Neon|comic=Comic|minimal=Minimal"),
                f("toggle", "sponsorColoured", "In your colours", "The card in the Suite's colours (Widgets' Colours).", if on(cur, "sponsorColoured", false) { "true" } else { "false" }, ""),
                f("chips", "sponsorAlign", "Alignment", "", &or("sponsorAlign", "left"), "left=Left|centre=Centre|right=Right"),
                f("chips", "sponsorMotion", "How each comes in", "", &or("sponsorMotion", "rise"), "rise=Rise|slide=Slide|flip=Flip|pop=Pop|type=Typewriter|none=None"),
                f("toggle", "sponsorAll", "All together", "Side by side, not one at a time.", if on(cur, "sponsorAll", false) { "true" } else { "false" }, ""),
                f("chips", "sponsorSecs", "Each one for", "", &or("sponsorSecs", "10"), "5=5 s|10=10 s|15=15 s|30=30 s|60=1 min"),
                addr("/sponsors", "About 920 × 180."),
            ]
        }),
        "panels" => ("Panels".into(), {
            let or = |k: &str, d: &'static str| if s(cur, k).is_empty() { d.to_string() } else { s(cur, k).to_string() };
            // (your saved templates: their names, to load or delete)
            let tpls: Vec<String> = serde_json::from_str::<Value>(s(cur, "panelTemplates")).ok().and_then(|v| v.as_array().cloned()).unwrap_or_default().iter().filter_map(|t| t["name"].as_str().map(String::from)).collect();
            let chips = tpls.iter().map(|n| format!("{}={}", n.replace(['|', '='], " "), n.replace(['|', '='], " "))).collect::<Vec<_>>().join("|");
            // (the preview and Save are the page's right half: these are its left)
            let mut v = vec![
                f("note", "", "Make your channel's panels here, watch them change on the right, then click Open in Your Browser. Each panel there has a Save as PNG button: add them on Twitch (your channel, About, Edit Panels) or Kick.", "", "", ""),
                f("head", "", "1. Your panels", "", "", ""),
                f("note", "", "Give each panel a title, and any words to go under it. Click a panel's words, or Edit Words, to write them. Format them as in any word processor: Ctrl+B for bold, Ctrl+I for italics, and the list buttons for a list. Each panel's icon is picked from its title. Click the icon to choose another, or to use a picture of your own.", "", "", ""),
            ];
            // (each panel its own card: its title, its words with their buttons, its icon, moved or taken out)
            let list = panels(cur);
            for (i, p) in list.iter().enumerate() {
                // (its icon chosen, and its picture made into an icon, in its note: "key|address")
                let mut card = f("panelcard", &format!("pnl:{i}"), &p.title, &format!("{}|{}", p.icon, p.mask), &p.body, &p.img);
                card.n = i as i32;
                v.push(card);
            }
            if list.len() < PANELS_MAX { v.push(f("buttons", "", "", "", "", "pnl:add=+ Add a Panel")); }
            v.extend([
                f("toggle", "panelExtended", "Always put the words under the title", "Each panel shows its title with all its words under it, as tall as they need (up to Twitch's 600). When this is off, a few words show beside the title instead, and a panel with more words, or a list, grows to fit them anyway.", if on(cur, "panelExtended", false) { "true" } else { "false" }, ""),
                f("toggle", "panelPowered", "Add a Powered by OmniPulse panel", "One more at the end, in the same look, with OmniPulse's logo, so viewers can find the app your stream runs on.", if on(cur, "panelPowered", true) { "true" } else { "false" }, ""),
                f("chips", "panelLogoColours", "Logo colours", "", if s(cur, "panelLogoColours").is_empty() { "theme" } else { s(cur, "panelLogoColours") }, "theme=Your Colours|omnipulse=OmniPulse's Own|plain=Plain"),
                f("head", "", "2. How they look", "", "", ""),
                f("chips", "panelTheme", "Theme", "", &or("panelTheme", "dark"), "dark=Dark|glass=Glass|neon=Neon|comic=Comic|pastel=Pastel|y2k=Y2K|pixel=Pixel|gradient=Gradient|minimal=Minimal|synthwave=Synthwave|paper=Paper Note|bauhaus=Bauhaus|holo=Holo Foil|terminal=Terminal|watercolour=Watercolour|cyber=Cyber|galaxy=Galaxy|ticket=Ticket"),
                f("chips", "panelColors", "Colours", "The theme's two colours. Same as Your Widgets uses the Suite widgets' colours.", if s(cur, "panelColors").is_empty() { "suite" } else { s(cur, "panelColors") }, "suite=Same as Your Widgets|accent=OmniPulse's Look|sunset=Sunset|ocean=Ocean|candy=Candy|mint=Mint|fire=Fire|gold=Gold|mono=Mono|custom=Your Own"),
            ]);
            // (the logo's colours: only with its panel on)
            if !on(cur, "panelPowered", true) { v.retain(|x| x.key != "panelLogoColours"); }
            if s(cur, "panelColors") == "custom" {
                v.push(f("text", "panelC1", "First colour", "As hex, like #8e9bff.", s(cur, "panelC1"), "#8e9bff"));
                v.push(f("text", "panelC2", "Second colour", "", s(cur, "panelC2"), "#ff7ac8"));
            }
            v.extend([
                f("text", "panelInk", "Words colour", "As hex, like #ffffff. Leave it empty for the theme's own.", s(cur, "panelInk"), "The theme's own"),
                f("text", "panelBg", "Background colour", "Replaces a plain background (Dark, Neon, Comic, Y2K, Pastel, Pixel, Paper Note, Bauhaus, Terminal, Watercolour, Galaxy, Ticket). Leave it empty for the theme's own.", s(cur, "panelBg"), "The theme's own"),
                f("chips", "panelShape", "Corners", "", &or("panelShape", "rounded"), "rounded=Rounded|square=Square|pill=Round Ends"),
                f("chips", "panelHeight", "Size", "Header is Twitch's usual 320 × 100. Square (320 × 320) suits a badge or a partner's logo.", &or("panelHeight", "medium"), "short=Short|medium=Header|tall=Tall|square=Square"),
                f("toggle", "panelIcons", "Icons", "An icon beside each title.", if on(cur, "panelIcons", true) { "true" } else { "false" }, ""),
                f("chips", "panelScale", "Sharpness", "Sharp saves them at twice the size, so words and logos stay crisp on high-density screens. Twitch shows them the same size either way.", &or("panelScale", "2"), "2=Sharp|1=Standard"),
                f("head", "", "3. Templates", "", "", ""),
                f("text", "panelTplName", "Template name", "Save this look and these panels under a name, to come back and change them later.", s(cur, "panelTplName"), "My panels"),
                f("link", "", "Save as a template", "A template with the same name is replaced.", "", "tpl:save"),
            ]);
            if !tpls.is_empty() {
                v.push(f("chips", "panelTplLoad", "Load a template", "Brings back the look and the panels you saved.", "", &chips));
                v.push(f("chips", "panelTplDel", "Delete a template", "", "", &chips));
            }
            v
        }),
        "cam" => ("Webcam Overlay".into(), {
            let or = |k: &str, d: &'static str| if s(cur, k).is_empty() { d.to_string() } else { s(cur, k).to_string() };
            let spot = or("camSpot", "br");
            let step = CAM_STEP.with(|c| c.get()).min(CAM_STEPS.len() - 1);
            let mut wiz = f("wizard", "cam", "", "", &step.to_string(), &CAM_STEPS.join("|"));
            wiz.n = step as i32;
            let mut v = vec![wiz];
            match step {
                // (1: where your camera is on your canvas)
                0 => {
                    v.push(f("note", "", "First, where your camera sits on your stream. Pick its corner, or choose Your Own Spot to set it exactly.", "", "", ""));
                    v.push(f("chips", "camSpot", "Where it is", "Put your camera there in OBS too, or choose Your Own Spot and match it.", &spot, "br=Bottom Right|bl=Bottom Left|tr=Top Right|tl=Top Left|custom=Your Own Spot"));
                    if spot == "custom" {
                        v.push(f("number", "camX", "Across (%)", "Where the left edge sits: 0 is the left of the canvas, 100 the right.", &n(cur, "camX"), "72"));
                        v.push(f("number", "camY", "Down (%)", "Where the top edge sits: 0 is the top of the canvas, 100 the bottom.", &n(cur, "camY"), "62"));
                        v.push(f("number", "camW", "Width (%)", "Of the canvas's width.", &n(cur, "camW"), "26"));
                        v.push(f("number", "camH", "Height (%)", "Of the canvas's height.", &n(cur, "camH"), "34"));
                    }
                }
                // (2: the frame round it)
                1 => {
                    v.push(f("note", "", "Now the frame that goes round your camera.", "", "", ""));
                    v.push(f("chips", "camShape", "Shape", "", &or("camShape", "rounded"), "rounded=Rounded|square=Square|circle=Circle|hex=Hexagon|arch=Arch"));
                    v.push(f("chips", "camBorder", "Border", "", &or("camBorder", "glow"), "thin=Thin|thick=Thick|double=Double|glow=Glow|gradient=Gradient|brackets=Corner Brackets|edge=Edge Light"));
                }
                // (3: its look)
                2 => {
                    v.push(f("note", "", "Pick a look for the name tag and the clock, and the colours for it all.", "", "", ""));
                    v.push(f("chips", "camTheme", "Theme", "For the name tag and the clock.", &or("camTheme", "dark"), "dark=Dark|glass=Glass|neon=Neon|comic=Comic|pastel=Pastel|gradient=Gradient|minimal=Minimal"));
                    v.push(f("chips", "suiteColors", "Colours", "Shared with the Suite's widgets.", if s(cur, "suiteColors").is_empty() { "accent" } else { s(cur, "suiteColors") }, "accent=Yours|sunset=Sunset|ocean=Ocean|candy=Candy|mint=Mint|fire=Fire|gold=Gold|mono=Mono|custom=Your Own"));
                }
                // (4: what's on it)
                3 => {
                    v.push(f("note", "", "Switch on what you'd like round your camera. The live ones are your widgets, as you've set them up on their own pages. You'll arrange them next.", "", "", ""));
                    for (id, name, dflt, _, _) in CAM_PARTS {
                        let k = format!("camP_{id}");
                        let is = on(cur, &k, dflt);
                        v.push(f("toggle", &k, name, "", if is { "true" } else { "false" }, ""));
                        if !is { continue; }
                        if id == "name" { v.push(f("text", "camNameText", "Name tag text", "Your name, or anything you like.", s(cur, "camNameText"), if s(cur, "login").is_empty() { "Your Name" } else { s(cur, "login") })); }
                        if id == "sticker" { v.push(f("text", "camStickerImg", "Sticker address", "A GIF's or picture's address (on GIPHY or Tenor: right-click it, Copy Image Address).", s(cur, "camStickerImg"), "https://…/your.gif")); }
                    }
                }
                // (5: where each part goes, and how the parts sharing a side sit; overlaps found and said)
                4 => {
                    let a = cam_arrange(cur);
                    let on_parts: Vec<_> = CAM_PARTS.iter().filter(|p| on(cur, &format!("camP_{}", p.0), p.2)).collect();
                    if on_parts.is_empty() {
                        v.push(f("note", "", "Nothing's switched on yet. Go back a step to choose what goes round your camera.", "", "", ""));
                    } else {
                        let names = |l: &[String]| match l.len() { 0 => String::new(), 1 => l[0].clone(), n => format!("{} and {}", l[..n - 1].join(", "), l[n - 1]) };
                        if a.overlaps.is_empty() {
                            v.push(f("status", "", "Nothing overlaps. Parts on the same side are lined up one after another.", "ok", "", ""));
                        } else {
                            let mut said: Vec<String> = a.overlaps.iter().map(|(x, y)| if y.is_empty() { format!("{x} goes off the edge of your canvas") } else { format!("{x} is over {y}") }).collect();
                            said.dedup();
                            v.push(f("status", "", &format!("{}. Try another place, a smaller size, or Side by Side instead of Stacked.", names(&said)), "warn", "", ""));
                        }
                        for side in CAM_SIDES.iter().map(|s| s.0) {
                            let went: Vec<String> = a.moved.iter().filter(|m| m.1 == side).map(|m| m.0.clone()).collect();
                            if !went.is_empty() {
                                let from = match side { "above" => "below", "below" => "above", "left" => "on the left of", _ => "on the right of" };
                                let to = match side { "above" | "below" => side.to_string(), s => format!("on the {s} of") };
                                v.push(f("status", "", &format!("There isn't room for {} {from} your camera, so {} shown {to} it instead.", names(&went), if went.len() == 1 { "it's" } else { "they're" }), "ok", "", ""));
                            }
                        }
                        // (each side with two or more parts: stacked or side by side)
                        for (side, side_name) in CAM_SIDES {
                            let here = a.parts.iter().skip(1).filter(|p| p.place == side).count();
                            if here < 2 { continue; }
                            let k = format!("camFlow_{side}");
                            v.push(f("chips", &k, &format!("{side_name} your camera"), "How the parts there sit together.", &or(&k, "auto"), "auto=Automatic|column=Stacked|row=Side by Side"));
                        }
                        for (id, name, _, place, size) in on_parts {
                            let k = format!("camP_{id}");
                            v.push(f("head", "", name, "", "", ""));
                            v.push(f("chips", &format!("{k}Place"), "Where", "", &or(&format!("{k}Place"), place), "above=Above|below=Below|left=Left|right=Right|in-top=Inside, Top|in-bottom=Inside, Bottom|corner=On the Corner"));
                            v.push(f("chips", &format!("{k}Size"), "Size", "", &or(&format!("{k}Size"), size), "small=Small|medium=Medium|large=Large"));
                        }
                    }
                }
                // (6: into OBS)
                _ => {
                    v.push(f("note", "", "Last, add it to OBS. Add a Browser source the size of your canvas (1920 × 1080), above your camera, with the address below. It's see-through: the frame goes round your camera, and the parts come with it, live ones included, so it's all one source.", "", "", ""));
                    v.push(addr("/cam/main", ""));
                    v.push(f("note", "", "Change anything later by clicking a step above.", "", "", ""));
                }
            }
            let last = CAM_STEPS.len() - 1;
            let back = if step > 0 { format!("cam:step:{}=Back", step - 1) } else { "=".into() };
            let next = if step < last { format!("cam:step:{}=Next: {}", step + 1, CAM_STEPS[step + 1]) } else { "=".into() };
            v.push(f("wiznav", "", &format!("Step {} of {}", step + 1, CAM_STEPS.len()), "", "", &format!("{back}|{next}")));
            v
        }),
        "tips" => ("Tips and Donations".into(), {
            let mut v = vec![
                f("note", "", "Tips and donations from Streamlabs, StreamElements, Ko-fi, Throne or Fourthwall show as alerts, and can start your actions. Click the ones you use to set them up.", "", "", ""),
            ];
            // (each a row of its own: clicked open for its steps, shut again with another click)
            for (id, name, set) in [("streamlabs", "Streamlabs", !s(cur, "streamlabsToken").is_empty()), ("streamelements", "StreamElements", !s(cur, "streamelementsToken").is_empty()), ("kofi", "Ko-fi", !s(cur, "kofiToken").is_empty()), ("throne", "Throne", on(cur, "throneOn", false)), ("fourthwall", "Fourthwall", !s(cur, "fourthwallSecret").is_empty())] {
                let open = TIPS_OPEN.with(|o| o.borrow().contains(id));
                v.push(f("fold", id, name, if set { "Set up" } else { "Not set up" }, if set { "ok" } else { "off" }, if open { "open" } else { "shut" }));
                if open { v.extend(sheet_of(id, cur).1); }
            }
            v
        }),
        // (Ko-fi: its webhook to the relay, a hook of your own; Ko-fi's token checked by the chatbot)
        "kofi" => ("Ko-fi".into(), vec![
            step(1, "Open Ko-fi's webhooks page (More, then API, then Webhooks).", "", "https://ko-fi.com/manage/webhooks", "Open Ko-fi's webhooks page"),
            step(2, "Copy your webhook address below and paste it into Webhook URL there. Click Update.", "", "", ""),
            f("addr", "", "Your webhook address", "It's yours alone. Don't show it on stream.", s(cur, "kofiUrl"), ""),
            step(3, "On the same page, open Advanced, copy your Verification Token and paste it below.", "", "", ""),
            f("password", "kofiToken", "Verification Token", "Tips, memberships and shop orders then show as alerts, and can start your actions. Leave it empty to turn this off.", s(cur, "kofiToken"), "Paste your Verification Token here"),
            f("note", "", "To check it works, save this, then click Send Single Donation Test on Ko-fi's page.", "", "", ""),
        ]),
        // (Throne: its webhook to the relay, which checks Throne's signature)
        "throne" => ("Throne".into(), vec![
            step(1, "On Throne, open your webhook settings (Throne's help page shows where) and switch webhooks on.", "", "https://help.throne.com/en/articles/15935990-how-do-i-set-up-webhook-integration", "How to find Throne's webhook settings"),
            step(2, "Copy your webhook address below and paste it in as a subscriber URL there. Save it.", "", "", ""),
            f("addr", "", "Your webhook address", "It's yours alone. Don't show it on stream.", s(cur, "throneUrl"), ""),
            f("toggle", "throneOn", "Show Throne gifts", "Gifts, contributions and fully funded gifts then show as alerts, and can start your actions.", if on(cur, "throneOn", false) { "true" } else { "false" }, ""),
            f("note", "", "Throne signs each one, and only signed ones from Throne are shown.", "", "", ""),
        ]),
        // (Fourthwall: its webhook to the relay; its signature checked by the chatbot, with your shop's secret)
        "fourthwall" => ("Fourthwall".into(), vec![
            step(1, "In your Fourthwall dashboard, open Settings, then For Developers, and add a webhook.", "", "https://docs.fourthwall.com/webhooks/getting-started", "Fourthwall's webhook guide"),
            step(2, "Copy your webhook address below and paste it in there. Pick the events Donation, Order placed, Gift purchase and Subscription purchased.", "", "", ""),
            f("addr", "", "Your webhook address", "It's yours alone. Don't show it on stream.", s(cur, "fourthwallUrl"), ""),
            step(3, "On the same page, copy your shop's secret key and paste it below.", "", "", ""),
            f("password", "fourthwallSecret", "Secret key", "Donations, orders, gifts and memberships then show as alerts, and can start your actions. Leave it empty to turn this off.", s(cur, "fourthwallSecret"), "Paste your secret key here"),
            f("note", "", "To check it works, save this, then send a test from Fourthwall's webhook page.", "", "", ""),
        ]),
        "streamelements" => ("StreamElements".into(), vec![
            step(1, "Open your StreamElements channels page and click Show Secrets.", "", "https://streamelements.com/dashboard/account/channels", "Open StreamElements' channels page"),
            step(2, "Copy your JWT Token and paste it below.", "", "", ""),
            f("password", "streamelementsToken", "JWT Token", "Tips then show as alerts, and can start your actions. Leave it empty to turn this off.", s(cur, "streamelementsToken"), "Paste your JWT Token here"),
        ]),
        // (all here while the chatbot's on: it takes the requests itself, so there are no addresses to set up)
        "requests" => ("Song Requests".into(), {
            let mut v = vec![
                f("toggle", "requests", "Song requests from chat", "Viewers can ask for songs in your Twitch and Kick chat.", if on(cur, "requests", true) { "true" } else { "false" }, ""),
                f("head", "", "Commands", "", "", ""),
                f("text", "srCommands", "Request", "What viewers type, then a song. Separate them with commas.", s(cur, "srCommands"), "!sr, !request"),
                f("text", "songCommands", "Now playing", "Says what's playing.", s(cur, "songCommands"), "!song"),
                f("text", "wrongCommands", "Wrong song", "Takes back the viewer's last request. For a moderator, the last one anyone made.", s(cur, "wrongCommands"), "!wrongsong"),
                f("head", "", "Who and how often", "", "", ""),
                f("who", "srWho", "Who can request", "", if s(cur, "srWho").is_empty() { "everyone" } else { s(cur, "srWho") }, ""),
                f("number", "srCooldown", "Wait between requests", "Seconds before the same viewer can request again. 0 for no wait.", &n(cur, "srCooldown"), "0"),
            ];
            if on(cur, "pointsOn", false) { v.push(f("number", "srCost", "Points per request", "Taken when the song is added. 0 for free.", &n(cur, "srCost"), "0")); }
            v.push(f("link", "", "Which songs, limits and replies", "Radio edits, keeping out AI music, how many can wait, and what's said back in chat.", "", "row-more:req"));
            v
        }),
        // (the Suite's widgets: where each goes in OBS, then how it's set; Try It sends a pretend one)
        "goals" => ("Goals".into(), {
            let style = if s(cur, "goalStyle").is_empty() { "bars" } else { s(cur, "goalStyle") };
            // (your goals first, what they count to; then how they look; then when they start again)
            // (the preview: your goals that are on, by name and target, as "name=target|…" in its key)
            let num = |k: &str, d: i64| n(cur, k).trim().parse::<f64>().map_or(d, |x| x as i64).max(1);
            let named = |k: &str, d: &str| if s(cur, k).trim().is_empty() { d.to_string() } else { s(cur, k).trim().replace(['|', '='], " ") };
            let mut shown: Vec<String> = [("Follow", "Followers", 10), ("Sub", "Subs", 5), ("Bits", "Bits", 1000), ("Dono", "Donations", 50)].iter()
                .filter(|(k, _, _)| on(cur, &format!("goal{k}On"), true))
                .map(|(k, name, d)| format!("{}={}", named(&format!("goal{k}Label"), name), num(&format!("goal{k}Target"), *d))).collect();
            if on(cur, "goalCommonOn", false) { shown.push(format!("{}={}", named("goalCommonLabel", "Community goal"), num("goalCommonTarget", 100))); }
            let mut v = vec![f("suiteprev", &shown.join("|"), style, "", "goals", &theme_of(cur))];
            v.push(addr("/goals", "All your goals in one source, about 920 × 520. To show one goal on its own, use the copy button on that goal's row."));
            v.push(f("link", "", "Try it", "Adds one of each, to see it fill.", "", "call:suiteTest:goals"));
            // (each goal one row: shown or not, its name, its target; key: its prefix, label: what it counts,
            // value: on, extra: its name, note: its target, n: its default target)
            v.push(f("head", "", "Your goals", "", "", ""));
            for (k, name, dflt) in [("Follow", "Followers", 10), ("Sub", "Subs", 5), ("Bits", "Bits", 1000), ("Dono", "Donations", 50)] {
                let mut row = f("goalrow", &format!("goal{k}"), name, &n(cur, &format!("goal{k}Target")), if on(cur, &format!("goal{k}On"), true) { "true" } else { "false" }, s(cur, &format!("goal{k}Label")));
                row.n = dflt;
                v.push(row);
            }
            v.push(f("text", "goalCurrency", "Donation currency", "Shown before amounts, like $ or £.", s(cur, "goalCurrency"), "$"));
            // (the Common Goal: one goal everything counts toward)
            v.push(f("head", "", "Common goal", "", "", ""));
            v.push(f("toggle", "goalCommonOn", "Show it", "One goal that follows, subs, bits, donations and raids all count toward, from Twitch, Kick and YouTube.", if on(cur, "goalCommonOn", false) { "true" } else { "false" }, ""));
            if on(cur, "goalCommonOn", false) {
                v.push(f("text", "goalCommonLabel", "Goal name", "", s(cur, "goalCommonLabel"), "Community goal"));
                v.push(f("number", "goalCommonTarget", "Target (points)", "", &n(cur, "goalCommonTarget"), "100"));
                v.push(f("number", "goalPtsFollow", "Points for a follow", "", &n(cur, "goalPtsFollow"), "1"));
                v.push(f("number", "goalPtsSub", "Points for a sub", "A gift of 5 subs counts 5 times.", &n(cur, "goalPtsSub"), "5"));
                v.push(f("number", "goalPtsBits", "Points for every 100 bits", "", &n(cur, "goalPtsBits"), "1"));
                v.push(f("number", "goalPtsDono", "Points for every 1 donated", "", &n(cur, "goalPtsDono"), "1"));
                v.push(f("number", "goalPtsRaid", "Points for each raider", "0 leaves raids out.", &n(cur, "goalPtsRaid"), "0"));
            }
            v.push(f("head", "", "How they look", "", "", ""));
            v.push(f("chips", "goalStyle", "Look", "Liquid fills a shape. Jar drops sweets in, a few for each step. Hourglass pours sand, and turns over when it's full to start the next round.", style, "bars=Bars|rings=Rings|liquid=Liquid|jar=Jar|hourglass=Hourglass"));
            // (the shape, for the liquid and the jar; the drops, for the jar)
            let shape = if s(cur, "goalShape").is_empty() { "jar" } else { s(cur, "goalShape") };
            if style == "liquid" || style == "jar" {
                v.push(f("chips", "goalShape", "Shape", "", shape, "jar=Jar|potion=Potion|heart=Heart|cup=Cup|star=Star|ghost=Ghost|bubble=Bubble"));
            }
            if style == "jar" {
                v.push(f("chips", "goalDrops", "What drops in", "", if s(cur, "goalDrops").is_empty() { "mixed" } else { s(cur, "goalDrops") }, "mixed=A mix|heart=Hearts|star=Stars|candy=Sweets|gem=Gems|coin=Coins"));
            }
            if style == "liquid" {
                v.push(f("text", "goalImage", "Your own shape", "A picture's address (a PNG with a see-through background). The liquid fills its outline. Leave it empty for the shape above.", s(cur, "goalImage"), "https://…/shape.png"));
            }
            v.push(f("toggle", "goalPercent", "Show a percentage", "In place of the count, like 7 / 10.", if on(cur, "goalPercent", false) { "true" } else { "false" }, ""));
            v.push(theme_field(cur));
            v.push(f("head", "", "Starting again", "", "", ""));
            v.push(f("reset", "goalReset", "Start again", "", if s(cur, "goalReset").is_empty() { "stream" } else { s(cur, "goalReset") }, ""));
            v.push(f("link", "", "Start again now", "Puts every goal back to 0.", "", "call:suiteTest:goalsreset"));
            v
        }),
        "timer" => ("Timer".into(), {
            let mut v = vec![
                // (the preview: its title, its starting minutes, and which way it counts, as "a|b|c" in its key)
                f("suiteprev", &format!("{}|{}|{}", s(cur, "timerLabel").replace('|', "/"), n(cur, "timerStart").trim().parse::<f64>().map_or(60, |m| m.max(0.0) as i64), if on(cur, "timerStopwatch", false) { "up" } else { "down" }), "", "", "timer", &theme_of(cur)),
                theme_field(cur),
                addr("/timer", "About 720 × 280. Moderators can control it with the chat command: start, pause, add 5, remove 5, reset."),
                f("toggle", "timerStopwatch", "Count up", "A stopwatch. When this is off, it counts down.", if on(cur, "timerStopwatch", false) { "true" } else { "false" }, ""),
                f("number", "timerStart", "Starting time (minutes)", "Where the countdown starts, and goes back to on reset.", &n(cur, "timerStart"), "60"),
                f("text", "timerLabel", "Title", "Shown above the timer. Leave it empty for none.", s(cur, "timerLabel"), "Subathon"),
                f("text", "timerCommand", "Chat command", "", s(cur, "timerCommand"), "!timer"),
                f("link", "", "Start or resume", "", "", "call:suiteTimer:start"),
                f("link", "", "Pause", "", "", "call:suiteTimer:pause"),
                f("link", "", "Add 5 minutes", "", "", "call:suiteTimer:add:5"),
                f("link", "", "Reset", "", "", "call:suiteTimer:reset"),
                f("head", "", "Subathon", "", "", ""),
                f("toggle", "timerSubathon", "Add time as things happen", "Each sub, bits, donation and follow adds the time set here.", if on(cur, "timerSubathon", false) { "true" } else { "false" }, ""),
            ];
            if on(cur, "timerSubathon", false) {
                v.extend([
                    f("number", "timerSub1", "Minutes per tier 1 sub", "Gifted subs count each one.", &n(cur, "timerSub1"), "5"),
                    f("number", "timerSub2", "Minutes per tier 2 sub", "", &n(cur, "timerSub2"), "10"),
                    f("number", "timerSub3", "Minutes per tier 3 sub", "", &n(cur, "timerSub3"), "25"),
                    f("number", "timerBits", "Minutes per 100 bits", "", &n(cur, "timerBits"), "1"),
                    f("number", "timerDono", "Minutes per 1 donated", "In your donations' currency.", &n(cur, "timerDono"), "1"),
                    f("number", "timerFollow", "Seconds per follow", "0 for none.", &n(cur, "timerFollow"), "0"),
                    f("number", "timerCap", "Most added (hours)", "0 for no limit.", &n(cur, "timerCap"), "0"),
                ]);
            }
            v
        }),
        "latest" => ("Latest Follow and Sub".into(), vec![
            f("suiteprev", "", if s(cur, "latestLook").is_empty() { "card" } else { s(cur, "latestLook") }, "", "latest", &theme_of(cur)),
            f("chips", "latestLook", "Look", "Just the Name shows only who it was: no panel, icon or label. It suits an overlay that already says Latest Follower.", if s(cur, "latestLook").is_empty() { "card" } else { s(cur, "latestLook") }, "card=Card|text=Just the Name"),
            theme_field(cur),
            addr("/latest-follow", "Who followed last. About 840 × 180."),
            addr("/latest-sub", "Who subscribed last, with their months or gifts. About 840 × 180."),
            f("link", "", "Try a follow", "", "", "call:suiteTest:follow"),
            f("link", "", "Try a sub", "", "", "call:suiteTest:sub"),
        ]),
        "raid" => ("Raid Shoutout".into(), vec![
            f("suiteprev", "", "", "", "raid", &theme_of(cur)),
            theme_field(cur),
            f("note", "", "Your latest raider is remembered. Press Raid Shoutout on your Stream Deck (or Show Shoutout here) to put them on screen, with one of their clips and a follow them animation.", "", "", ""),
            addr("/raid", "About 1120 × 840. Open it as localhost (as here), or Twitch won't play the clip."),
            f("number", "raidMin", "Smallest raid", "Raids with fewer viewers than this aren't remembered for a shoutout.", &n(cur, "raidMin"), "1"),
            f("number", "raidSecs", "Shown for (seconds)", "Longer if their clip is.", &n(cur, "raidSecs"), "20"),
            f("toggle", "raidChat", "Shout out in chat too", "Said when you show the shoutout.", if on(cur, "raidChat", true) { "true" } else { "false" }, ""),
            f("text", "raidMessage", "Shoutout message", "{user} is the raider, {login} their channel name, {viewers} how many came.", s(cur, "raidMessage"), "Thanks for the raid, {user}!"),
            f("toggle", "raidClip", "Play one of their clips", "A random clip of theirs from the last 60 days (Twitch only).", if on(cur, "raidClip", false) { "true" } else { "false" }, ""),
            f("link", "", "Show shoutout", "Your latest raider, now.", "", "call:suiteShoutout"),
            f("link", "", "Try it", "A pretend raider, to see how it looks.", "", "call:suiteTest:raid"),
        ]),
        // (moderation: Auto-Ban, then where the other rules are)
        "moderation" => ("Moderation".into(), {
            let what = if s(cur, "abDo").is_empty() { "ban" } else { s(cur, "abDo") };
            let mut v = vec![
                f("head", "", "Auto-Ban", "", "", ""),
                f("toggle", "abOn", "Take down spam at once", "A message with one of the terms below is deleted, and its sender dealt with as you choose. You and your moderators are never affected.", if on(cur, "abOn", false) { "true" } else { "false" }, ""),
                f("text", "abTerms", "Your terms", "With | between them, like: buy free views | stream.ai. Shorter catches more: the address alone catches every message with it.", s(cur, "abTerms"), ""),
                f("toggle", "abPack", "Common spam too", "Phrases viewer-bot and follower sellers use, like buy followers and best viewers on.", if on(cur, "abPack", true) { "true" } else { "false" }, ""),
                f("toggle", "abLoose", "Match loosely", "Capitals, spaces, dots and symbols don't count, nor numbers for letters, so stream . ai and STR3AM.ai are both caught.", if on(cur, "abLoose", true) { "true" } else { "false" }, ""),
                f("chips", "abDo", "What happens", "", what, "ban=Ban|timeout=Time Out|delete=Delete the Message"),
            ];
            if what == "timeout" { v.push(f("number", "abSecs", "Time-out length (seconds)", "600 is 10 minutes.", &n(cur, "abSecs"), "600")); }
            if what != "delete" { v.push(f("text", "abReason", "Reason", "Shown to them, and in your ban list.", s(cur, "abReason"), "Spam (auto-ban)")); }
            v.extend([
                f("chips", "abSpare", "Never affects", "You and your moderators always. VIPs and subscribers too, if you choose.", if s(cur, "abSpare").is_empty() { "mods" } else { s(cur, "abSpare") }, "mods=Moderators|vips=VIPs Too|subs=Subscribers Too"),
                f("toggle", "abFirstOnly", "First-time chatters only", "Most spam is someone's first message. With this on, regulars are never checked.", if on(cur, "abFirstOnly", false) { "true" } else { "false" }, ""),
                f("note", "", "Each one taken down is said in your Chat Dock with the term that caught it, and kept in the Activity Feed.", "", "", ""),
                f("head", "", "Other rules", "", "", ""),
                f("note", "", "Links, too many capitals or emotes, too long, or words of your own: make an action with A Rule Broken (in Chat), then choose what happens, like deleting the message or a time-out.", "", "", ""),
                f("link", "", "Open Actions", "", "", "actions"),
            ]);
            v
        }),
        // (the fun's: chat playing along)
        "hype" => ("Hype Train".into(), {
            let mode = if s(cur, "hypeMode").is_empty() { "train" } else { s(cur, "hypeMode") };
            let train = mode == "train";
            let mut v = vec![
                f("suiteprev", "", mode, "", "hype", &theme_of(cur)),
                theme_field(cur),
                f("link", "", "Try it", if train { "Starts a test train." } else { "Adds a level's worth of hype." }, "", "call:suiteTest:hype"),
                f("chips", "hypeMode", "It works as", if train { "A hype train for every platform at once: subs, gifts, bits, Kicks, donations and Super Chats from Twitch, Kick and YouTube all keep it going. Twitch's own hype train starts it too." } else { "A meter that's always there: chat, follows, subs, bits, donations and raids fill it, and it cools slowly while things are quiet." }, mode, "train=A Hype Train|meter=An Always-On Meter"),
            ];
            if train {
                v.push(f("chips", "hypeShow", "On stream", "", if s(cur, "hypeShow").is_empty() { "train" } else { s(cur, "hypeShow") }, "train=Only During a Train|always=All the Time"));
                v.push(f("chips", "hypeStart", "A train starts with", "", if s(cur, "hypeStart").is_empty() { "any" } else { s(cur, "hypeStart") }, "any=Enough on Any Platform|twitch=Twitch's Own Only"));
                if s(cur, "hypeStart") != "twitch" { v.push(f("number", "hypeStartPts", "Points to start one", "Within 5 minutes. A sub is 25, 100 bits or Kicks 10, each 1 donated 10.", &n(cur, "hypeStartPts"), "50")); }
                v.push(f("number", "hypeSecs", "How long it waits", "Seconds without anything coming in before it ends. Twitch's waits 5 minutes (300).", &n(cur, "hypeSecs"), "300"));
                v.push(f("number", "hypeLevel", "Points per level", "", &n(cur, "hypeLevel"), "100"));
            } else {
                v.push(f("number", "hypeLevel", "Points per level", "A follow is 10, a sub 25, 100 bits 10, a raid 1 per viewer.", &n(cur, "hypeLevel"), "100"));
                v.push(f("number", "hypeChatPts", "Per chatter", "For each viewer who chats, at most once every 10 seconds.", &n(cur, "hypeChatPts"), "2"));
                v.push(f("number", "hypeCool", "Cooling", "How fast it empties while quiet, in percent of a level every 5 seconds.", &n(cur, "hypeCool"), "2"));
            }
            v.push(f("toggle", "hypeRace", "Platforms race", if train { "With hype from more than one of Twitch, Kick and YouTube, each gets a lane and its own train, racing the others." } else { "With hype from more than one of Twitch, Kick and YouTube, they fight over the meter: two pull it like a tug of war, three take squares from each other." }, if on(cur, "hypeRace", true) { "true" } else { "false" }, ""));
            v.push(f("text", "hypeLabel", "Title", "Shown on the meter.", s(cur, "hypeLabel"), if train { "Hype Train" } else { "Hype" }));
            v.push(f("head", "", "In chat", "", "", ""));
            v.push(f("toggle", "hypeChat", "Say it in chat", if train { "When a train starts, at each level, and when it ends." } else { "At each level." }, if on(cur, "hypeChat", true) { "true" } else { "false" }, ""));
            if on(cur, "hypeChat", true) {
                if train { v.push(f("text", "hypeStartMessage", "When it starts", "", s(cur, "hypeStartMessage"), "🚂 A hype train's started! Subs, bits, Kicks and donations on any platform keep it going.")); }
                v.push(f("text", "hypeMessage", "Each level", "{level} is the level reached.", s(cur, "hypeMessage"), "Hype level {level}! 🔥"));
                if train { v.push(f("text", "hypeEndMessage", "When it ends", "{level} is the level it got to, {top} who gave the most.", s(cur, "hypeEndMessage"), "The hype train ended at level {level}. Thank you!{top}")); }
            }
            v.push(f("note", "", "Actions can run on these: Fun > A Hype Level Reached, A Hype Train Starting and A Hype Train Ending.", "", "", ""));
            v.push(addr("/hype", "About 1040 × 260."));
            v
        }),
        "voteskip" => ("Vote Skip".into(), vec![
            f("suiteprev", "", "", "", "voteskip", &theme_of(cur)),
            theme_field(cur),
            f("toggle", "skipOn", "Let chat vote to skip", "Off until you switch it on, so it doesn't take a command you already use.", if on(cur, "skipOn", false) { "true" } else { "false" }, ""),
            f("text", "skipCommand", "Chat command", "", s(cur, "skipCommand"), "!skip"),
            f("number", "skipVotes", "Votes needed", "Each viewer once a song. A new song starts the count again.", &n(cur, "skipVotes"), "5"),
            addr("/voteskip", "About 1040 × 260. It only shows while there are votes."),
            f("link", "", "Try it", "Two test votes, to see it.", "", "call:suiteTest:voteskip"),
        ]),
        "wheel" => ("Spin the Wheel".into(), { let mut v = vec![
            // (the preview: each prize and where its slice starts, in degrees, as "name=start|…" in its key;
            // its colours in its look)
            f("suiteprev", &wheel_slices(s(cur, "wheelPrizes")).iter().map(|(n, a)| format!("{}={a}", n.replace(['|', '='], " "))).collect::<Vec<_>>().join("|"), if s(cur, "wheelColors").is_empty() { "theme" } else { s(cur, "wheelColors") }, "", "wheel", &theme_of(cur)),
            theme_field(cur),
            f("link", "", "Try it", "A free spin, to see it on the preview's widget in OBS.", "", "call:suiteTest:wheel"),
            f("chips", "wheelColors", "Colours", "", if s(cur, "wheelColors").is_empty() { "theme" } else { s(cur, "wheelColors") }, "theme=Your Colours|alternate=Two Colours|rainbow=Rainbow|pastel=Pastel"),
            f("chips", "wheelSecs", "How long it spins", "", if s(cur, "wheelSecs").is_empty() { "6" } else { s(cur, "wheelSecs") }, "3=3 s|6=6 s|9=9 s|12=12 s"),
            f("toggle", "wheelHide", "Only show it while it spins", "Hidden the rest of the time, and for a few seconds after it lands.", if on(cur, "wheelHide", false) { "true" } else { "false" }, ""),
            f("head", "", "Prizes", "", "", ""),
            f("list", "wheelPrizes", "Prizes", "2 to 16. Add x2 or x3 to the end of one to make it twice or three times as likely, like Nothing! x3.", s(cur, "wheelPrizes"), "A shoutout | 100 points | Pick the next song | Nothing! | Hydrate"),
            f("head", "", "Who can spin", "", "", ""),
            f("toggle", "wheelOn", "Chat can spin it", "With its command. Actions can spin it too, with the Spin the Wheel step: from a Channel Points reward, say.", if on(cur, "wheelOn", false) { "true" } else { "false" }, ""),
        ];
            if on(cur, "wheelOn", false) {
                v.push(f("text", "wheelCommand", "Command", "", s(cur, "wheelCommand"), "!spin"));
                v.push(f("number", "wheelCooldown", "Wait between spins", "Seconds before the same viewer can spin again. 0 for no wait.", &n(cur, "wheelCooldown"), "0"));
                if on(cur, "pointsOn", false) { v.push(f("number", "wheelCost", "Points for a spin", "0 for free.", &n(cur, "wheelCost"), "0")); }
            }
            v.push(f("head", "", "When it lands", "", "", ""));
            v.push(f("toggle", "wheelChat", "Say it in chat", "", if on(cur, "wheelChat", true) { "true" } else { "false" }, ""));
            if on(cur, "wheelChat", true) { v.push(f("text", "wheelMessage", "Message", "{user} is who spun it, {prize} what it landed on.", s(cur, "wheelMessage"), "{user} spun the wheel: {prize}!")); }
            v.push(f("note", "", "To make something happen for a prize, add an action with Fun > The Wheel Landing as its trigger. %message% is the prize.", "", "", ""));
            v.push(addr("/wheel", "About 1040 × 1120."));
            v
        }),
        // (the GPU scenes in OBS: its address made from what's picked)
        "vis" => {
            let pick = |k: &str, d: &'static str| -> String { let v = s(cur, k); if v.is_empty() { d.to_string() } else { v.to_string() } };
            let (scene, scale, fps) = (Some(pick("visScene", "bars")).filter(|s| s != "depths" && s != "phosphor").unwrap_or_else(|| "smoke".into()), pick("visScale", "1"), pick("visFps", "60"));
            // (one address: what it shows is kept here, and OBS's picks it up when it's saved)
            let path = "/vis".to_string();
            let mut v = vec![
                f("note", "", "The app's visualisers on stream, drawn by OBS. The GPU scenes are still being built, so they may change, and some may not look right yet.", "", "", ""),
                addr(&path, "Add it as a Browser source the size of your canvas, like 1920 × 1080. What you pick below shows in OBS once you click Save. The app's own styles lay over your stream as they are; a GPU scene fills its source."),
                f("chips", "visScene", "Style", "The app's own styles, drawn see-through so they lay straight over your stream.", &scene, "bars=Bars|mirror=Mirror|peaks=Peaks|blocks=Blocks|dots=Dots|wave=Wave|solid=Solid Wave|centre=Centre|halo=Halo"),
            ];
            let flat = !matches!(scene.as_str(), "smoke" | "swarm" | "ink");
            if flat {
                let pal = if s(cur, "visColors").is_empty() { "accent" } else { s(cur, "visColors") };
                v.push(f("chips", "visColors", "Colours", "Your Theme's means the two colours of your app theme (App & Look), never the album art's.", pal, "accent=Your Theme's|sunset=Sunset|ocean=Ocean|candy=Candy|mint=Mint|fire=Fire|gold=Gold|mono=Mono|custom=Your Own"));
                if pal == "custom" {
                    v.push(f("text", "visC1", "First colour", "As hex, like #8e9bff.", s(cur, "visC1"), "#8e9bff"));
                    v.push(f("text", "visC2", "Second colour", "", s(cur, "visC2"), "#ff7ac8"));
                }
                v.push(f("chips", "visTrail", "Motion blur", "Leaves a fading trail behind what moves.", if s(cur, "visTrail").is_empty() { "off" } else { s(cur, "visTrail") }, "off=Off|short=Short|long=Long"));
            }
            v.extend([
                f("chips", "visScene", "Or a GPU scene (WIP)", "Fuller pictures, drawn on your graphics card.", &scene, "smoke=Smoke|swarm=Swarm|ink=Ink"),
                f("chips", "visScale", "Detail", "Lower draws it smaller and stretches it to fit, for less work on your graphics card.", &scale, "0.5=Low|0.75=Medium|1=Full"),
                f("chips", "visFps", "Frame rate", "", &fps, "30=30 a second|60=60 a second"),
                f("toggle", "visOffset", "Keep in time with OmniPulse Offset", "Holds it back by the Offset's delay, so it lands with what viewers hear. This only applies while OmniPulse Offset is on.", if on(cur, "visOffset", true) { "true" } else { "false" }, ""),
                f("note", "", "It rests while nothing's playing, and the app only reads the music while OBS has it open.", "", "", ""),
            ]);
            ("Visualiser".into(), v)
        }
        "credits" => {
            let mut v = vec![
                f("suiteprev", "", "", "", "credits", &theme_of(cur)),
                theme_field(cur),
                f("note", "", "Rolls a thank-you to everyone who followed, subscribed, gifted, cheered, raided, donated or chatted since you went live, and the songs you played. Roll it at the end of your stream.", "", "", ""),
                addr("/credits", "Add it as a Browser source the size of your canvas, like 1920 × 1080. It's see-through until it rolls."),
                f("link", "", "Try it", "Rolls them now. With no one in them yet, it uses made-up names.", "", "call:suiteTest:credits"),
                f("text", "creditsTitle", "Title", "Shown first.", s(cur, "creditsTitle"), "Thanks for watching!"),
                f("text", "creditsEnd", "Last line", "Shown at the end. Leave it empty for none.", if cur.get("creditsEnd").is_none() { "See you next stream!" } else { s(cur, "creditsEnd") }, ""),
                f("chips", "creditsSpeed", "Speed", "", if s(cur, "creditsSpeed").is_empty() { "normal" } else { s(cur, "creditsSpeed") }, "slow=Slow|normal=Normal|fast=Fast"),
                f("toggle", "creditsLoop", "Roll again when it ends", "Keeps rolling, for an ending screen.", if on(cur, "creditsLoop", false) { "true" } else { "false" }, ""),
                f("text", "creditsCommand", "Chat command", "You and your moderators roll the credits with it. So can an action, with the Roll the Credits step.", s(cur, "creditsCommand"), "!credits"),
                f("head", "", "What they list", "", "", ""),
            ];
            for p in ["Followers", "Subscribers", "Gifted Subs", "Cheers", "Raiders", "Donations", "Chatters", "Songs Played"] {
                let k = format!("credits_{}", p.replace(' ', ""));
                v.push(f("toggle", &k, p, "", if on(cur, &k, true) { "true" } else { "false" }, ""));
            }
            ("Stream Credits".into(), v)
        }
        "leaderboard" => ("Leaderboard".into(), vec![
            f("suiteprev", "", "", "", "leaderboard", &theme_of(cur)),
            theme_field(cur),
            f("note", "", "The viewers with the most points. Points need to be on (Chatbot > Points and ranks).", "", "", ""),
            f("number", "boardCount", "How many", "3 to 10.", &n(cur, "boardCount"), "5"),
            addr("/leaderboard", "About 840 × 760."),
        ]),
        "tune" => ("Name That Tune".into(), vec![
            f("suiteprev", "", "", "", "tune", &theme_of(cur)),
            theme_field(cur),
            f("note", "", "Chat guesses the song you're playing. While a round's on, the Stream Widget hides the song's name and artist, and this widget shows a little more of the name as time goes. The first viewer to type the name or the artist wins.", "", "", ""),
            f("toggle", "tuneOn", "Start rounds from chat", "You and your moderators can start a round with the chat command. Actions can start one too, with the Start Name That Tune step (from a reward, say).", if on(cur, "tuneOn", false) { "true" } else { "false" }, ""),
            f("text", "tuneCommand", "Command", "", s(cur, "tuneCommand"), "!tune"),
            f("toggle", "tuneAnyone", "Anyone can start a round", "", if on(cur, "tuneAnyone", false) { "true" } else { "false" }, ""),
            f("number", "tuneSecs", "Seconds to guess", "15 to 300. A new song ends the round too.", &n(cur, "tuneSecs"), "60"),
            f("number", "tunePoints", "Points for winning", "With Points on. Actions can run on a win: Fun > Name That Tune Won.", &n(cur, "tunePoints"), "100"),
            f("text", "tuneMessage", "Round message", "Said when a round starts. {points} is the points, {pointsname} what they're called.", s(cur, "tuneMessage"), "🎵 Name That Tune! First to type the song's name or its artist wins {points} {pointsname}."),
            addr("/tune", "About 1040 × 360."),
            f("link", "", "Try it", "A round on what's playing now.", "", "call:suiteTest:tune"),
        ]),
        "emoterain" => ("Emote Rain".into(), {
            let or = |k: &str, d: &'static str| if s(cur, k).is_empty() { d.to_string() } else { s(cur, k).to_string() };
            let (dir, from, src) = (or("rainDir", "down"), or("rainFrom", "all"), or("rainSource", "chat"));
            // (its look: which way, then the size and whether they spin, after "|")
            let mut prev = f("suiteprev", &from, &format!("{dir}|{}|{}", or("rainSize", "medium"), if on(cur, "rainSpin", true) { "spin" } else { "still" }), "", "emoterain", &theme_of(cur));
            prev.key = from.clone();
            let mut v = vec![
                prev,
                f("link", "", "Try it", "A burst, as it's set.", "", "call:suiteTest:emoterain"),
                f("head", "", "What rains", "", "", ""),
                f("chips", "rainSource", "Emotes", "Chat's are Twitch's, Kick's, 7TV's, BetterTTV's and FrankerFaceZ's, as your viewers use them.", &src, "chat=Chat's Emotes|mine=Your Own Pictures|both=Both"),
            ];
            if src != "chat" { v.push(f("text", "rainImages", "Your pictures", "Their addresses (PNG or GIF), with | between them. Up to 20.", s(cur, "rainImages"), "https://…/heart.png | https://…/star.gif")); }
            if src != "mine" { v.push(f("toggle", "rainChat", "Each emote said rains", "One for each emote in a chat message. When this is off, only bursts rain.", if on(cur, "rainChat", true) { "true" } else { "false" }, "")); }
            v.extend([
                f("head", "", "How it rains", "", "", ""),
                f("chips", "rainDir", "Which way", "Fountain throws them up from the spot and lets them fall. Burst throws them out all round it. Float drifts them up like bubbles.", &dir, "down=Down|up=Up|left=To the Left|right=To the Right|fountain=Fountain|burst=Burst|float=Float"),
                f("chips", "rainFrom", "From where", "A spot, such as where your camera is, or all over.", &from, "all=All Over|bl=Bottom Left|br=Bottom Right|tl=Top Left|tr=Top Right|b=Bottom Middle|c=Middle|custom=Your Own Spot"),
            ]);
            if from == "custom" {
                v.push(f("number", "rainX", "Across (%)", "0 is the left edge, 100 the right.", &n(cur, "rainX"), "85"));
                v.push(f("number", "rainY", "Down (%)", "0 is the top, 100 the bottom.", &n(cur, "rainY"), "85"));
            }
            v.extend([
                f("chips", "rainSize", "Size", "", &or("rainSize", "medium"), "small=Small|medium=Medium|large=Large"),
                f("chips", "rainSpeed", "Speed", "", &or("rainSpeed", "normal"), "slow=Slow|normal=Normal|fast=Fast"),
                f("chips", "rainAmount", "A burst's emotes", "", &or("rainAmount", "30"), "15=15|30=30|60=60|100=100"),
                f("toggle", "rainSpin", "They spin", "", if on(cur, "rainSpin", true) { "true" } else { "false" }, ""),
                f("head", "", "What pours a burst", "", "", ""),
                f("toggle", "rainOnSub", "A sub or gift", "", if on(cur, "rainOnSub", true) { "true" } else { "false" }, ""),
                f("toggle", "rainOnRaid", "A raid", "", if on(cur, "rainOnRaid", true) { "true" } else { "false" }, ""),
                f("toggle", "rainOnBits", "A cheer of 100 bits or more", "", if on(cur, "rainOnBits", true) { "true" } else { "false" }, ""),
                f("toggle", "rainOnDono", "A donation", "", if on(cur, "rainOnDono", true) { "true" } else { "false" }, ""),
                f("toggle", "rainOnFollow", "A follow", "", if on(cur, "rainOnFollow", false) { "true" } else { "false" }, ""),
                f("note", "", "A new hype level pours one too, and so does the Make It Rain Emotes step in your actions (from a Channel Points reward, say).", "", "", ""),
                addr("/emotes", "Make it as big as your canvas (1920 × 1080), on top. It's see-through."),
            ]);
            v
        }),
        "import" => ("Import from Another Bot".into(), vec![
            f("note", "", "Your commands, timers and actions from another bot become the Suite's actions. Nothing in the other bot changes.", "", "", ""),
            f("link", "", "From a backup file", "Streamer.bot (an export, a .sb file or its backup zip) or Mix It Up (a .miubackup or .miucommand).", "", "import:file"),
            f("link", "", "Paste from Streamer.bot", "In Streamer.bot, click Export and copy the code it gives you. Then click this.", "", "import:paste"),
        ]),
        "points" => ("Points".into(), {
            let channel = s(cur, "pointsCurrency") == "channel";
            let mut v = vec![
                f("toggle", "pointsOn", "Points and ranks", "A currency viewers spend on song requests, the wheel and your actions, and ranks for your regulars.", if on(cur, "pointsOn", false) { "true" } else { "false" }, ""),
                f("chips", "pointsCurrency", "What viewers spend", if channel { "Twitch and Kick don't let any app see or take channel points, so paid things become rewards viewers redeem. If one can't be done, their points are given back." } else { "The chatbot's own points: viewers earn them by chatting." }, if channel { "channel" } else { "app" }, "app=The Chatbot's Points|channel=Channel Points"),
            ];
            if channel {
                v.push(f("note", "", "Request a Song and Spin the Wheel become channel points rewards, at the prices set on their pages (Song Requests, and the wheel's). Click below to make them, and again after changing a price. Games that bet points (gamble, duels, heists) are off.", "", "", ""));
                v.push(f("buttons", "", "", "OmniPulse makes its own rewards, because Twitch only lets the app that made a reward give its points back.", "", "pts:rewards=Make the Rewards"));
            } else {
                v.extend([
                    f("text", "pointsName", "Name", "What points are called in chat.", s(cur, "pointsName"), "points"),
                    f("number", "pointsAmount", "Points each time", "Given to each viewer who chatted since the last time.", &n(cur, "pointsAmount"), "10"),
                    f("number", "pointsEvery", "How often (minutes)", "", &n(cur, "pointsEvery"), "10"),
                    f("toggle", "pointsOffline", "Also while you're offline", "When this is off, viewers only earn points while you're live.", if on(cur, "pointsOffline", false) { "true" } else { "false" }, ""),
                ]);
            }
            let by = s(cur, "pointsRankBy");
            let by = if by.is_empty() { if channel { "spent" } else { "points" } } else { by };
            v.push(f("chips", "pointsRankBy", "Ranks count", "Channel points spent counts what viewers spend on any of your rewards from now on.", by, "points=The Chatbot's Points|spent=Channel Points Spent|both=Both, Added Up"));
            v.push(f("text", "pointsRanks", "Ranks", "Each rank is a number and a name, separated by commas.", s(cur, "pointsRanks"), "0 Newcomer, 100 Regular, 500 Veteran"));
            v
        }),
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
        ("import", "Coming from another bot?".into(), if on(cur, "imported", false) { "Your commands and actions came across.".into() } else { "Optional. Bring your commands and actions from Streamer.bot or Mix It Up.".into() }, on(cur, "imported", false), true),
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
        "import" => sheet("import", cur).1,
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
    // (your name as Twitch shows it, once the chatbot's asked; the lower-case login till then)
    let tw = st["twitch"]["name"].as_str().filter(|n| !n.is_empty() && n.eq_ignore_ascii_case(s(cur, "login"))).unwrap_or(s(cur, "login"));
    let kk = s(cur, "kickLogin");
    let wait = if running { "Connecting..." } else { "The chatbot is stopped" };
    vec![
        ("twitch", "Twitch".into(), light(!tw.is_empty(), "twitch"), if tw.is_empty() { "Not signed in".into() } else if up("twitch") { tw.to_string() } else { format!("{tw}: {}", wait.to_lowercase()) }, if s(cur, "botLogin").is_empty() { "No bot account".into() } else { format!("Bot: {}", s(cur, "botLogin")) }),
        ("kick", "Kick".into(), light(on(cur, "kickOn", false) && !kk.is_empty(), "kick"), if !on(cur, "kickOn", false) { "Off".into() } else if kk.is_empty() { "Not signed in".into() } else if up("kick") { kk.to_string() } else { format!("{kk}: {}", wait.to_lowercase()) }, if s(cur, "kickBotLogin").is_empty() { "No bot account".into() } else { format!("Bot: {}", s(cur, "kickBotLogin")) }),
        ("youtube", "YouTube".into(), light(on(cur, "ytOn", false) && !s(cur, "ytChannel").is_empty(), "youtube"), if !on(cur, "ytOn", false) { "Off".into() } else if s(cur, "ytChannel").is_empty() { "Needs your channel".into() } else if up("youtube") { "Reading your live chat".into() } else { "Waiting for you to go live".into() }, "Chat, Super Chats, members".into()),
        software(cur, st),
        tips(cur, st),
    ]
}

/// Your streaming software, in one tile: OBS, Meld Studio or Streamlabs Desktop, whichever are on
fn software(cur: &Map<String, Value>, st: &Value) -> (&'static str, String, &'static str, String, String) {
    let up = |k: &str| st[k]["up"].as_bool().unwrap_or(false);
    let apps = [("obs", "OBS", on(cur, "obsOn", false)), ("meld", "Meld Studio", on(cur, "meldOn", false)), ("slobs", "Streamlabs Desktop", on(cur, "slobsOn", false) && !s(cur, "slobsToken").is_empty())];
    let set: Vec<_> = apps.iter().filter(|a| a.2).collect();
    if set.is_empty() { return ("software", "Streaming Software".into(), "off", "Not connected".into(), "OBS, Meld Studio or Streamlabs Desktop".into()); }
    let ok = set.iter().any(|a| up(a.0));
    let line = set.iter().map(|a| format!("{}: {}", a.1, if up(a.0) { "connected" } else { "waiting for it to open" })).collect::<Vec<_>>().join(", ");
    ("software", "Streaming Software".into(), if ok { "ok" } else { "warn" }, line, "Scenes, sources and more".into())
}

/// Tips and donations, in one tile: Streamlabs' and StreamElements'
fn tips(cur: &Map<String, Value>, st: &Value) -> (&'static str, String, &'static str, String, String) {
    let up = |k: &str| st[k]["up"].as_bool().unwrap_or(false);
    let apps = [("streamlabs", "Streamlabs", !s(cur, "streamlabsToken").is_empty()), ("streamelements", "StreamElements", !s(cur, "streamelementsToken").is_empty()), ("kofi", "Ko-fi", !s(cur, "kofiToken").is_empty() && !s(cur, "kofiUrl").is_empty()), ("throne", "Throne", on(cur, "throneOn", false) && !s(cur, "throneUrl").is_empty()), ("fourthwall", "Fourthwall", !s(cur, "fourthwallSecret").is_empty() && !s(cur, "fourthwallUrl").is_empty())];
    let set: Vec<_> = apps.iter().filter(|a| a.2).collect();
    if set.is_empty() { return ("tips", "Tips and Donations".into(), "off", "Not set up".into(), "Streamlabs, StreamElements, Ko-fi, Throne or Fourthwall".into()); }
    let line = set.iter().map(|a| format!("{}: {}", a.1, if up(a.0) { "listening" } else { "not connected" })).collect::<Vec<_>>().join(", ");
    ("tips", "Tips and Donations".into(), if set.iter().all(|a| up(a.0)) { "ok" } else { "warn" }, line, "As alerts, and for your actions".into())
}

/// What it does: (id, name, note, on, has a sheet)
pub fn feats(cur: &Map<String, Value>, st: &Value) -> Vec<(&'static str, String, String, bool, bool)> {
    let (all, live) = (st["actions"]["all"].as_u64(), st["actions"]["on"].as_u64());
    let name = if s(cur, "pointsName").is_empty() { "points".to_string() } else { s(cur, "pointsName").to_string() };
    vec![
        ("requests", "Song requests".into(), {
            let or = |k: &str, d: &str| if s(cur, k).is_empty() { d.to_string() } else { s(cur, k).to_string() };
            format!("{}, {}, {}", or("srCommands", "!sr, !request"), or("songCommands", "!song"), or("wrongCommands", "!wrongsong"))
        }, on(cur, "requests", true), true),
        ("alerts", "Alerts".into(), "Follows, subs, bits and raids, on stream".into(), on(cur, "alerts", true), true),
        ("points", "Points and ranks".into(), format!("{} {name} every {} minutes", if n(cur, "pointsAmount").is_empty() { "10".into() } else { n(cur, "pointsAmount") }, if n(cur, "pointsEvery").is_empty() { "10".into() } else { n(cur, "pointsEvery") }), on(cur, "pointsOn", false), true),
        // (moderation: Auto-Ban first; the rules, for links, caps and emote spam, are Actions' A Rule Broken)
        ("moderation", "Moderation".into(), if on(cur, "abOn", false) {
            let own = s(cur, "abTerms").split(['|', '\n']).filter(|x| !x.trim().is_empty()).count();
            format!("Auto-Ban on: {own} of your terms{}", if on(cur, "abPack", true) { ", and common spam" } else { "" })
        } else { "Auto-Ban spam and anything you name".into() }, on(cur, "abOn", false), true),
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
    match k { "feat:requests" => "requests", "feat:alerts" => "alerts", "feat:points" => "pointsOn", "feat:moderation" => "abOn", x => x }
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
        // (channel points as the currency: its rewards made on Twitch and Kick)
        "pts:rewards" => "makePointRewards",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webcam_parts_sharing_a_side_dont_overlap() {
        let p = |id: &str, place: &str, size: &str| (id.to_string(), id.to_string(), place.to_string(), size.to_string());
        // (the camera bottom right: no room under it, so the name tag goes above, with the rest stacked there)
        let parts = [p("name", "below", "medium"), p("live", "corner", "small"), p("now", "above", "medium"), p("alerts", "above", "large"), p("hype", "above", "medium"), p("follow", "left", "medium"), p("sub", "left", "medium")];
        let a = cam_arrange_in(1920.0, 1080.0, (72.0, 62.0, 26.0, 34.0), &parts, &[]);
        assert!(a.overlaps.is_empty(), "{:?}", a.overlaps);
        assert_eq!(a.moved, vec![("name".to_string(), "above".to_string())]);
        assert!(a.parts.iter().all(|q| q.x >= 0.0 && q.y >= 0.0 && q.x + q.w <= 1920.1 && q.y + q.h <= 1080.1));
        // (side by side above, as asked: one row, not one over another)
        let a = cam_arrange_in(1920.0, 1080.0, (72.0, 62.0, 26.0, 34.0), &parts[2..4], &[("above".into(), "row".into())]);
        assert!((a.parts[1].y + a.parts[1].h - (a.parts[2].y + a.parts[2].h)).abs() < 0.5);
        assert!(a.overlaps.is_empty(), "{:?}", a.overlaps);
        // (the corner badge kept off the clock inside the top, with something above the camera too)
        let a = cam_arrange_in(1920.0, 1080.0, (72.0, 62.0, 26.0, 34.0), &[p("live", "corner", "small"), p("alerts", "above", "large"), p("clock", "in-top", "small")], &[]);
        assert!(a.overlaps.is_empty(), "{:?}", a.overlaps);
        // (too many for anywhere: the overlap's found)
        let big: Vec<_> = (0..6).map(|_| p("chat", "left", "large")).collect();
        let a = cam_arrange_in(1920.0, 1080.0, (40.0, 30.0, 26.0, 34.0), &big, &[("left".into(), "column".into())]);
        assert!(!a.overlaps.is_empty());
    }

    #[test]
    fn panel_words_show_formatted() {
        let md = rich_md("Hi, I'm **Sam** and *you* __rock__\r\n- one\n- two #3\n\n1. first\n2. second\nThe end <3");
        assert_eq!(md, "Hi, I'm **Sam** and *you* <u>rock</u>\n\n- one\n- two \\#3\n\n1. first\n2. second\n\nThe end \\<3");
        let st = slint::StyledText::from_markdown(&md);
        assert!(st.is_ok(), "{:?}", st.err());
    }

    #[test]
    fn panels_kept_before_their_list_came_in() {
        let mut cur = Map::new();
        cur.insert("panelList".into(), json!("About Me: Music / Good vibes | Rules"));
        cur.insert("panelOwnIcons".into(), json!(" | /overlay/r.png"));
        let p = panels(&cur);
        assert_eq!(p, vec![Panel { title: "About Me".into(), body: "Music\nGood vibes".into(), ..Default::default() }, Panel { title: "Rules".into(), img: "/overlay/r.png".into(), ..Default::default() }]);
        cur.insert("panelItems".into(), json!(panels_json(&p)));
        cur.insert("panelList".into(), json!("Something else"));
        assert_eq!(panels(&cur), p);
        assert_eq!(rich_plain("**Hi** *there*\n- one\n2. two"), "Hi there\n\u{2022} one\n2. two");
    }

    #[test]
    fn labels_in_title_case() {
        assert_eq!(title_case("Wait between requests"), "Wait Between Requests");
        assert_eq!(title_case("Points for every 100 bits"), "Points for Every 100 Bits");
        assert_eq!(title_case("Start or resume"), "Start or Resume");
        assert_eq!(title_case("Who it's for"), "Who It's For");
        assert_eq!(title_case("A GIF with it"), "A GIF With It");
        assert_eq!(title_case("Shown for (seconds)"), "Shown for (Seconds)");
        assert_eq!(title_case("Leave it empty."), "Leave it empty.");
    }

    #[test]
    fn a_prize_marked_x3_gets_three_times_the_slice() {
        let w = wheel_slices("Nothing! x3 | A shoutout");
        assert_eq!(w, vec![("Nothing!".to_string(), 0.0), ("A shoutout".to_string(), 270.0)]);
        assert_eq!(wheel_slices("").len(), 5);
    }

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
        // (and Shared Chat's switch: whether a partner's chatters use your commands)
        assert_eq!(keys("twitch", &m(json!({}))), vec!["twitch:channel", "twitch:bot", "twitchOwnApp", "sharedCommands"]);
        assert_eq!(keys("kick", &m(json!({}))), vec!["kickOn", "kickChannel", "kick:channel", "kick:bot", "kickOwnApp"]);
        assert!(keys("twitch", &m(json!({ "twitchOwnApp": true }))).contains(&"twitchClientId".to_string()));
        // (and a sign-in's given them, Kick's to be finished by the relay)
        let mut v = m(json!({}));
        sign_in_extras(&mut v, "https://relay");
        assert_eq!((v["twitchClientId"].as_str(), v["twitchRelay"].as_str(), v["kickClientId"].as_str(), v["kickClientSecret"].as_str(), v["kickRelay"].as_str()), (Some(TWITCH_ID), Some("https://relay"), Some(KICK_ID), Some(""), Some("https://relay")));
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
        let light = |id: &str| t.iter().find(|x| x.0 == id).map(|x| x.2).unwrap_or("");
        assert_eq!((light("twitch"), light("kick"), light("software"), light("youtube"), light("tips")), ("ok", "off", "warn", "off", "off"));
        assert_eq!(t[0].3, "Me");
        assert!(method("link:kick:bot") == Some("signInKickBotCopy") && method("out:twitch:channel") == Some("signOutChannel"));
    }
}
