// Mod View: for moderators. A channel you moderate, watched in OmniPulse: its stream, its chat with
// moderation on each message, and its chat modes, Shield Mode and AutoMod queue. Mod View plugin
// (plugins/modsuite) signs you in, reads the chat and does what you ask; this keeps what it sends (the chat,
// and its state: who you're signed in as, the channels you moderate, the one you're watching, its modes and
// held messages) for the page (/mod), and hands what the page asks to the app.

use serde_json::{json, Value};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicU64, Ordering},
    sync::Mutex,
};

/// The page: stream top left, controls bottom left, chat on the right
pub const MOD_HTML: &str = include_str!("../widget/mod.html");

const CHAT_KEPT: usize = 400;

static CHAT: Mutex<VecDeque<Value>> = Mutex::new(VecDeque::new());
static CHAT_N: AtomicU64 = AtomicU64::new(0);
/// The plugin's state (its own fields), and the app's (the Jam's: "jam")
static STATE: Mutex<Option<Value>> = Mutex::new(None);
static JAM: Mutex<Option<Value>> = Mutex::new(None);
static STATE_N: AtomicU64 = AtomicU64::new(0);
/// The app's look as it is now (its accents as they're showing, the album art's among them), for its own page
static LOOK: Mutex<Option<Value>> = Mutex::new(None);

/// The app's look now: { accent, accent2 }
pub fn look_set(v: Value) {
    let mut l = LOOK.lock().unwrap();
    if l.as_ref() != Some(&v) { *l = Some(v); STATE_N.fetch_add(1, Ordering::Relaxed); }
}

/// Told what the page asks: { do: ..., ... }; its answer (Ok: a line to show, or nothing) comes back by `notice`
pub type ModHandler = std::sync::Arc<dyn Fn(Value) + Send + Sync>;
static ON_MOD: Mutex<Option<ModHandler>> = Mutex::new(None);
pub fn set_handler(f: ModHandler) {
    if let Ok(mut h) = ON_MOD.lock() { *h = Some(f); }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// A chat message (as the Chat Dock's: platform, id, user, login, userId, color, badges, text, emotes...), or a notice
pub fn chat_push(mut v: Value) {
    let Some(o) = v.as_object_mut() else { return };
    o.insert("n".into(), json!(CHAT_N.fetch_add(1, Ordering::SeqCst) + 1));
    o.insert("ts".into(), json!(now_ms()));
    if let Some(t) = o.get("text").and_then(|t| t.as_str()) { let t: String = t.chars().take(600).collect(); o.insert("text".into(), json!(t)); }
    if let (Some(p), Some(id)) = (o.get("platform").and_then(|x| x.as_str()), o.get("userId").and_then(|x| x.as_str())) { crate::badges::seen(p, id); }
    let mut q = CHAT.lock().unwrap();
    q.push_back(v);
    while q.len() > CHAT_KEPT { q.pop_front(); }
}

/// Something the page should say (what a button did, or why it couldn't)
pub fn notice(text: &str) {
    chat_push(json!({ "notice": text.chars().take(300).collect::<String>() }));
}

/// Watching another channel: the chat so far let go, so the new one starts clean
pub fn chat_clear() {
    CHAT.lock().unwrap().clear();
}

/// Messages taken down: one (its id), all of a viewer's (their login), or the whole chat (neither)
pub fn chat_gone(platform: &str, msg_id: &str, login: &str) {
    let mut q = CHAT.lock().unwrap();
    for m in q.iter_mut().filter(|m| m["platform"] == platform && m.get("notice").is_none()) {
        let hit = if !msg_id.is_empty() { m["id"] == msg_id } else if !login.is_empty() { m["login"].as_str().map(|l| l.eq_ignore_ascii_case(login)).unwrap_or(false) } else { true };
        if hit { m["gone"] = json!(true); }
    }
}

pub fn chat_json(after: u64) -> Value {
    let q = CHAT.lock().unwrap();
    let items: Vec<&Value> = q.iter().filter(|m| m["n"].as_u64().unwrap_or(0) > after).collect();
    let gone: Vec<&Value> = q.iter().filter(|m| m["gone"] == true).map(|m| &m["n"]).collect();
    json!({ "items": items, "gone": gone, "last": CHAT_N.load(Ordering::SeqCst) })
}

/// The plugin's state, as it last sent it
pub fn state_set(v: Value) {
    *STATE.lock().unwrap() = Some(v);
    STATE_N.fetch_add(1, Ordering::Relaxed);
}

/// The Jam's, as the app has it (None: not in one)
pub fn jam_set(v: Option<Value>) {
    *JAM.lock().unwrap() = v;
    STATE_N.fetch_add(1, Ordering::Relaxed);
}

/// For the page: the plugin's state (none yet: the plugin isn't running), the Jam's, and a count that goes up
/// with each change
pub fn state_json() -> Value {
    let st = STATE.lock().unwrap().clone();
    json!({ "n": STATE_N.load(Ordering::Relaxed), "running": st.is_some(), "state": st.unwrap_or(Value::Null), "jam": JAM.lock().unwrap().clone().unwrap_or(Value::Null), "look": LOOK.lock().unwrap().clone().unwrap_or(Value::Null) })
}

/// The plugin stopped: its state forgotten
pub fn stopped() {
    *STATE.lock().unwrap() = None;
    STATE_N.fetch_add(1, Ordering::Relaxed);
}

/// What the page asks (a page of the widgets' own server only: see docks::trusted)
pub fn mod_do(cmd: Value) -> Result<String, String> {
    if !cmd["do"].is_string() { return Err("ask it to do something".into()); }
    match ON_MOD.lock().ok().and_then(|h| h.clone()) {
        Some(h) => { h(cmd); Ok(String::new()) }
        None => Err("OmniPulse isn't ready for that yet".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mod_chat_is_kept_cleared_and_taken_down() {
        chat_clear();
        chat_push(json!({ "platform": "twitch", "id": "a", "login": "bad", "text": "spam" }));
        chat_push(json!({ "platform": "twitch", "id": "b", "login": "good", "text": "hi" }));
        chat_gone("twitch", "", "BAD");
        let j = chat_json(0);
        let items = j["items"].as_array().unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(j["gone"].as_array().unwrap().len(), 1);
        chat_clear();
        assert_eq!(chat_json(0)["items"].as_array().unwrap().len(), 0);
        state_set(json!({ "watching": { "platform": "twitch", "channel": "x" } }));
        assert_eq!(state_json()["state"]["watching"]["channel"], "x");
        assert_eq!(state_json()["running"], true);
    }
}
