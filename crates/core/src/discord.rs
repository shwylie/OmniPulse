// Discord activity, spoken directly to the local Discord client.
// The name shown on your profile ("Listening to ...") is the name of the Discord
// application you create, so name that application whatever you like (for example "Music").

use crate::load_config;
use serde::Deserialize;
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    sync::Mutex,
    time::Duration,
};

pub static DISCORD: Mutex<Option<fs::File>> = Mutex::new(None);

pub fn discord_frame(op: u32, payload: &str) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&op.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload.as_bytes());
    out
}

pub fn discord_read(f: &mut fs::File) -> Result<Value, String> {
    let mut head = [0u8; 8];
    f.read_exact(&mut head).map_err(|e| e.to_string())?;
    let len = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
    let mut body = vec![0u8; len.min(1 << 20)];
    f.read_exact(&mut body).map_err(|e| e.to_string())?;
    Ok(serde_json::from_slice(&body).unwrap_or(Value::Null))
}

pub fn discord_connect(client_id: &str) -> Result<fs::File, String> {
    for i in 0..10 {
        let path = format!(r"\\.\pipe\discord-ipc-{i}");
        if let Ok(mut f) = fs::OpenOptions::new().read(true).write(true).open(&path) {
            let hello = serde_json::json!({ "v": 1, "client_id": client_id }).to_string();
            f.write_all(&discord_frame(0, &hello)).map_err(|e| e.to_string())?;
            let reply = discord_read(&mut f)?;
            if reply["evt"] == "READY" {
                return Ok(f);
            }
            let msg = reply["message"]
                .as_str()
                .or_else(|| reply["data"]["message"].as_str())
                .unwrap_or("no reason given")
                .to_string();
            return Err(format!("Discord refused the connection ({msg}). Check discord_client_id in config.json."));
        }
    }
    Err("Discord isn't running, or it's running as a different Windows user or as administrator.".to_string())
}

// Runs a Discord exchange on its own thread so a silent Discord can't hang anything
pub fn with_timeout<T: Send + 'static>(
    secs: u64,
    what: &str,
    f: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(Duration::from_secs(secs)) {
        Ok(r) => r,
        Err(_) => Err(format!("Discord didn't answer within {secs} seconds ({what}).")),
    }
}

pub fn discord_exchange(f: &mut fs::File, activity: &Option<Value>) -> Result<Value, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .to_string();
    let payload = serde_json::json!({
        "cmd": "SET_ACTIVITY",
        "args": { "pid": std::process::id(), "activity": activity },
        "nonce": nonce
    })
    .to_string();
    f.write_all(&discord_frame(1, &payload)).map_err(|e| e.to_string())?;
    discord_read(f)
}

pub fn discord_send(client_id: &str, activity: Option<Value>) -> Result<(), String> {
    // Take the open connection out, so nothing else waits on it while we talk to Discord
    let existing = DISCORD.lock().map_err(|_| "Discord lock failed".to_string())?.take();
    let id = client_id.to_string();
    let (file, reply) = with_timeout(4, "sending the activity", move || {
        if let Some(mut f) = existing {
            if let Ok(reply) = discord_exchange(&mut f, &activity) {
                return Ok((f, reply));
            }
        }
        let mut f = discord_connect(&id)?;
        let reply = discord_exchange(&mut f, &activity)?;
        Ok((f, reply))
    })?;
    if let Ok(mut g) = DISCORD.lock() {
        *g = Some(file);
    }
    if reply["evt"] == "ERROR" {
        let msg = reply["data"]["message"].as_str().unwrap_or("no reason given");
        return Err(format!("Discord rejected the activity: {msg}"));
    }
    Ok(())
}

pub fn discord_id() -> String {
    load_config().map(|c| c.discord_client_id).unwrap_or_default()
}

#[derive(Deserialize)]
pub struct PresenceButton {
    pub label: String,
    pub url: String,
}

// Returns a note when Discord only accepted a simpler version of the activity
pub async fn presence_set(
    details: String,
    state: String,
    start: Option<i64>,
    end: Option<i64>,
    image: Option<String>,
    buttons: Option<Vec<PresenceButton>>,
) -> Result<String, String> {
    let id = discord_id();
    if id.trim().is_empty() {
        return Err("Add discord_client_id to config.json (see the README).".to_string());
    }
    let r = tokio::task::spawn_blocking(move || -> Result<String, String> {
        let fix = |s: String| -> String {
            let t: String = s.chars().take(120).collect();
            if t.trim().is_empty() {
                "Unknown".to_string()
            } else if t.chars().count() < 2 {
                format!("{t}\u{200B}")
            } else {
                t
            }
        };
        // type 2 = "Listening to"
        let mut full = serde_json::json!({ "type": 2, "details": fix(details), "state": fix(state) });
        let mut ts = serde_json::Map::new();
        if let Some(s) = start {
            ts.insert("start".to_string(), s.into());
        }
        if let Some(e) = end {
            ts.insert("end".to_string(), e.into());
        }
        if !ts.is_empty() {
            full["timestamps"] = Value::Object(ts);
        }
        if let Some(img) = image {
            if !img.is_empty() {
                full["assets"] = serde_json::json!({ "large_image": img });
            }
        }
        // Buttons other people can click (Discord allows two; labels up to 32 characters)
        let ok_url = |u: &str| {
            u.len() <= 512
                && (u.starts_with("https://open.spotify.com/") || u.starts_with("https://soundcloud.com/") || u.starts_with("https://www.soundcloud.com/") || u.starts_with("https://music.youtube.com/"))
        };
        let list: Vec<Value> = buttons
            .unwrap_or_default()
            .into_iter()
            .filter(|b| ok_url(&b.url) && !b.label.trim().is_empty())
            .take(2)
            .map(|b| serde_json::json!({ "label": b.label.chars().take(32).collect::<String>(), "url": b.url }))
            .collect();
        if !list.is_empty() {
            full["buttons"] = Value::Array(list);
        }

        // Try the full activity, then simpler versions if Discord rejects a part of it
        match discord_send(&id, Some(full.clone())) {
            Ok(()) => return Ok(String::new()),
            Err(e) if !e.starts_with("Discord rejected") => return Err(e),
            Err(_) => {}
        }
        let had_link = full.get("buttons").is_some();
        let mut no_link = full;
        if let Some(o) = no_link.as_object_mut() {
            o.remove("buttons");
        }
        if had_link {
            match discord_send(&id, Some(no_link.clone())) {
                Ok(()) => return Ok("Showing without the buttons (Discord refused them).".to_string()),
                Err(e) if !e.starts_with("Discord rejected") => return Err(e),
                Err(_) => {}
            }
        }
        let mut no_art = no_link;
        if let Some(o) = no_art.as_object_mut() {
            o.remove("assets");
        }
        match discord_send(&id, Some(no_art.clone())) {
            Ok(()) => return Ok("Showing without artwork (Discord refused the cover image).".to_string()),
            Err(e) if !e.starts_with("Discord rejected") => return Err(e),
            Err(_) => {}
        }
        let mut plain = no_art;
        if let Some(o) = plain.as_object_mut() {
            o.remove("type");
            o.remove("timestamps");
        }
        discord_send(&id, Some(plain))
            .map(|_| "Showing as \"Playing\" (Discord refused the \"Listening to\" style).".to_string())
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}

pub async fn presence_clear() -> Result<(), String> {
    let id = discord_id();
    if id.trim().is_empty() {
        return Ok(());
    }
    let r = tokio::task::spawn_blocking(move || discord_send(&id, None))
        .await
        .map_err(|e| e.to_string())?;
    r
}

// Settings > Discord > Test: shows exactly what Discord answers
pub async fn presence_test() -> Result<String, String> {
    let id = discord_id();
    if id.trim().is_empty() {
        return Err("discord_client_id is missing from config.json.".to_string());
    }
    let r = tokio::task::spawn_blocking(move || -> Result<String, String> {
        if let Ok(mut g) = DISCORD.lock() {
            *g = None; // start from a fresh connection
        }
        let test = Some(serde_json::json!({
            "type": 2,
            "details": "Connection test",
            "state": "If you can see this, it works"
        }));
        let (file, reply) = with_timeout(5, "connecting and sending a test", move || {
            let mut f = discord_connect(&id)?;
            let reply = discord_exchange(&mut f, &test)?;
            Ok((f, reply))
        })?;
        if let Ok(mut g) = DISCORD.lock() {
            *g = Some(file);
        }
        let text: String = reply.to_string().chars().take(400).collect();
        Ok(format!("Connected. Discord replied: {text}"))
    })
    .await
    .map_err(|e| e.to_string())?;
    r
}
