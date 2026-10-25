// OBS docks (View > Docks > Custom Browser Docks), served by the Stream Widget's server: the chat as the
// chatbot hears it (Twitch and Kick), each message with its moderation buttons, and an activity feed of
// what happened on stream (follows, subs, bits, raids, rewards...), each kind shown or hidden, each alert
// replayed. What the docks ask for (a ban, a message, an action run again) goes to the app, which hands it
// to the chatbot; what it answers comes back as a notice in the chat dock.

use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    sync::Mutex,
};

pub const CHAT_HTML: &str = include_str!("../widget/dock-chat.html");
pub const ACTIVITY_HTML: &str = include_str!("../widget/dock-activity.html");
/// The multichat, on stream: the same chat, see-through, for a Browser source
pub const MULTICHAT_HTML: &str = include_str!("../widget/multichat.html");

const CHAT_KEPT: usize = 300;
const ACTIVITY_KEPT: usize = 250;

static CHAT: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
static CHAT_N: AtomicU64 = AtomicU64::new(0);
/// The activity feed (kept in <data>/activity.json, so it's there after a restart): None until read
static ACTIVITY: Mutex<Option<VecDeque<Value>>> = Mutex::new(None);
static ACTIVITY_N: AtomicU64 = AtomicU64::new(0);

/// Told what a dock asks for that only the chatbot can do: { do: "delete" | "timeout" | "ban" | "unban" | "say" | "rerun", ... }
pub type DockHandler = std::sync::Arc<dyn Fn(Value) + Send + Sync>;
static ON_DOCK: Mutex<Option<DockHandler>> = Mutex::new(None);
pub fn set_dock_handler(f: DockHandler) {
    if let Ok(mut h) = ON_DOCK.lock() { *h = Some(f); }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A chat message (the chatbot's: platform, id, user, login, userId, color, badges, text, emotes, self), or a notice
pub fn chat_push(mut v: Value) {
    let Some(o) = v.as_object_mut() else { return };
    o.insert("n".into(), json!(CHAT_N.fetch_add(1, Ordering::SeqCst) + 1));
    o.insert("ts".into(), json!(now_ms()));
    if let Some(t) = o.get("text").and_then(|t| t.as_str()) { let t: String = t.chars().take(600).collect(); o.insert("text".into(), json!(t)); }
    let mut q = CHAT.lock().unwrap();
    q.push_back(v);
    while q.len() > CHAT_KEPT { q.pop_front(); }
}

/// Something the chat dock should say (what a moderation button did, or why it couldn't)
pub fn notice(text: &str) {
    chat_push(json!({ "notice": text.chars().take(300).collect::<String>() }));
}

/// Messages taken down: one (its id), all of a viewer's (their login), or the whole chat (neither)
pub fn chat_gone(platform: &str, msg_id: &str, login: &str) {
    let mut q = CHAT.lock().unwrap();
    for m in q.iter_mut().filter(|m| m["platform"] == platform && m.get("notice").is_none()) {
        let hit = if !msg_id.is_empty() { m["id"] == msg_id } else if !login.is_empty() { m["login"].as_str().map(|l| l.eq_ignore_ascii_case(login)).unwrap_or(false) } else { true };
        if hit { m["gone"] = json!(true); }
    }
}

/// The chat dock's poll: the messages after the last it has, and which of all it has are now gone
pub fn chat_json(after: u64) -> Value {
    let q = CHAT.lock().unwrap();
    let items: Vec<&Value> = q.iter().filter(|m| m["n"].as_u64().unwrap_or(0) > after).collect();
    let gone: Vec<&Value> = q.iter().filter(|m| m["gone"] == true).map(|m| &m["n"]).collect();
    json!({ "items": items, "gone": gone, "last": CHAT_N.load(Ordering::SeqCst) })
}

fn activity_file() -> Option<std::path::PathBuf> {
    crate::data_dir().ok().map(|d| d.join("activity.json"))
}

fn with_activity<T>(f: impl FnOnce(&mut VecDeque<Value>) -> T) -> T {
    let mut g = ACTIVITY.lock().unwrap();
    if g.is_none() {
        let list: VecDeque<Value> = activity_file().filter(|_| !cfg!(test)).and_then(|p| std::fs::read(p).ok()).and_then(|b| serde_json::from_slice::<Vec<Value>>(&b).ok()).unwrap_or_default().into();
        let top = list.iter().filter_map(|x| x["n"].as_u64()).max().unwrap_or(0);
        ACTIVITY_N.fetch_max(top, Ordering::SeqCst);
        *g = Some(list);
    }
    f(g.as_mut().unwrap())
}

fn activity_save(list: &VecDeque<Value>) {
    // (never a test's: the feed on this PC is the user's)
    if cfg!(test) || std::env::var_os("OMNIPULSE_SELFTEST").is_some() { return; }
    if let Some(p) = activity_file() {
        let _ = std::fs::write(p, serde_json::to_vec(&list.iter().collect::<Vec<_>>()).unwrap_or_default());
    }
}

/// Something that happened on stream: { type, platform, user, amount, message, ... }, with its alert (to
/// replay) if it had one, and from the chatbot (its actions can run again) or not
pub fn activity_push(mut v: Value) {
    if !v.is_object() { return; }
    with_activity(|list| {
        v["n"] = json!(ACTIVITY_N.fetch_add(1, Ordering::SeqCst) + 1);
        v["ts"] = json!(now_ms());
        list.push_back(v);
        while list.len() > ACTIVITY_KEPT { list.pop_front(); }
        activity_save(list);
    });
}

pub fn activity_json(after: u64) -> Value {
    with_activity(|list| json!({ "items": list.iter().filter(|x| x["n"].as_u64().unwrap_or(0) > after).collect::<Vec<_>>(), "last": ACTIVITY_N.load(Ordering::SeqCst) }))
}

/// The docks' and the Multichat Overlay's settings, as OmniPulse keeps them (its Suite page): each page's by
/// its name ("chat", "activity", "multichat", "multichat2"...), and a count that goes up with each change, so a
/// page knows when to take them again
static PAGE_SETTINGS: Mutex<Option<Value>> = Mutex::new(None);
static PAGE_SETTINGS_N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

pub fn page_settings_set(all: Value) {
    *PAGE_SETTINGS.lock().unwrap() = Some(all);
    PAGE_SETTINGS_N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// One page's: { n, opt } (opt null: none kept yet, the page's own are sent to be kept)
pub fn page_settings(page: &str) -> Value {
    let opt = PAGE_SETTINGS.lock().unwrap().as_ref().map(|a| a[page].clone()).unwrap_or(Value::Null);
    json!({ "n": PAGE_SETTINGS_N.load(std::sync::atomic::Ordering::Relaxed), "opt": opt })
}

/// Who may ask a dock to do something: a page of the widget's own server (OBS's dock), never another site
/// (any web page can send a request to this PC; its Origin says where it's from) or another name for this PC
pub fn trusted(req: &str, port: u16) -> bool {
    let header = |name: &str| req.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case(name)).map(|(_, v)| v.trim().to_string()));
    let ours = |h: &str| h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}");
    let host_ok = header("host").map(|h| ours(&h)).unwrap_or(false);
    let origin_ok = match header("origin") { None => true, Some(o) => o.strip_prefix("http://").map(ours).unwrap_or(false) };
    host_ok && origin_ok
}

/// What a dock asks: the feed's own (replay an alert, clear it) done here; the rest the chatbot's
pub fn dock_do(cmd: Value) -> Result<String, String> {
    match cmd["do"].as_str().unwrap_or("") {
        "replay" => {
            let n = cmd["n"].as_u64().unwrap_or(0);
            let alert = with_activity(|list| list.iter().find(|x| x["n"].as_u64() == Some(n)).map(|x| x["alert"].clone())).filter(|a| a.is_object());
            let Some(Value::Object(mut a)) = alert else { return Err("that one had no alert".into()) };
            a.insert("feed".into(), json!(false));
            crate::widget::alert_push(&a).map(|_| "Replayed.".into())
        }
        "clearfeed" => {
            with_activity(|list| { list.clear(); activity_save(list); });
            Ok("Cleared.".into())
        }
        // (its settings changed on the page itself, its gear in OBS: kept by OmniPulse, as its own page's)
        "delete" | "timeout" | "ban" | "unban" | "say" | "rerun" | "settings" => {
            let mut cmd = cmd;
            // (an action run again: the event as it was kept, from the feed)
            if cmd["do"] == "rerun" {
                let n = cmd["n"].as_u64().unwrap_or(0);
                let Some(ev) = with_activity(|list| list.iter().find(|x| x["n"].as_u64() == Some(n) && x["chatbot"] == true).cloned()) else { return Err("only the chatbot's events can run their actions again".into()) };
                cmd["event"] = ev;
            }
            match ON_DOCK.lock().ok().and_then(|h| h.clone()) {
                Some(h) => { h(cmd); Ok(String::new()) }
                None => Err("OmniPulse isn't ready for that yet".into()),
            }
        }
        other => Err(format!("a dock can't {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_is_kept_and_taken_down() {
        chat_push(json!({ "platform": "twitch", "id": "a", "login": "bad", "text": "spam" }));
        chat_push(json!({ "platform": "twitch", "id": "b", "login": "good", "text": "hi" }));
        chat_push(json!({ "platform": "kick", "id": "c", "login": "bad", "text": "spam too" }));
        let last = CHAT_N.load(Ordering::SeqCst);
        chat_gone("twitch", "", "BAD");
        let j = chat_json(last - 3);
        assert_eq!(j["items"].as_array().unwrap().len(), 3);
        let gone: Vec<u64> = j["gone"].as_array().unwrap().iter().filter_map(|x| x.as_u64()).collect();
        assert!(gone.contains(&(last - 2)) && !gone.contains(&(last - 1)) && !gone.contains(&last), "{j}");
        assert_eq!(chat_json(last)["items"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn only_the_docks_own_pages_are_trusted() {
        let r = |host: &str, origin: Option<&str>| format!("POST /dock/do HTTP/1.1\r\nHost: {host}\r\n{}Content-Type: application/json\r\n\r\n{{}}", origin.map(|o| format!("Origin: {o}\r\n")).unwrap_or_default());
        assert!(trusted(&r("127.0.0.1:7878", Some("http://127.0.0.1:7878")), 7878));
        assert!(trusted(&r("localhost:7878", None), 7878));
        assert!(!trusted(&r("127.0.0.1:7878", Some("https://evil.example")), 7878));
        assert!(!trusted(&r("evil.example:7878", None), 7878));
        assert!(!trusted(&r("127.0.0.1:7878", Some("null")), 7878));
    }

    #[test]
    fn a_feeds_alert_replays_and_others_go_to_the_chatbot() {
        let _ = with_activity(|l| l.len());
        activity_push(json!({ "type": "follow", "user": "Ann", "alert": { "type": "follow", "user": "Ann" }, "chatbot": true }));
        let n = ACTIVITY_N.load(Ordering::SeqCst);
        assert_eq!(dock_do(json!({ "do": "replay", "n": n })), Ok("Replayed.".to_string()));
        let asked = std::sync::Arc::new(Mutex::new(Vec::new()));
        let a2 = asked.clone();
        set_dock_handler(std::sync::Arc::new(move |v| a2.lock().unwrap().push(v)));
        assert!(dock_do(json!({ "do": "rerun", "n": n })).is_ok());
        assert_eq!(asked.lock().unwrap()[0]["event"]["user"], "Ann");
        assert!(dock_do(json!({ "do": "format" })).is_err());
    }
}
