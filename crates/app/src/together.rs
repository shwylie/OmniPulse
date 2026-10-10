// Listen Along and Jams, hosting (as the web app's js/together.js): a room on the relay (jam-server/),
// what's playing sent to it as it changes, and what comes back (who's in, songs a Jam member added,
// someone asking for a Jam). Listeners' apps play the same songs through their own accounts.
//
// Joining someone else's room comes with this app's own installer (the omnipulse:// links go to the
// web app until then).

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

/// A room this app is hosting
#[derive(Clone)]
pub struct Room {
    /// "listen" (the Discord status's Listen along / Ask to Jam) or "jam" (invite-only)
    pub kind: String,
    /// the link friends open (the relay's page, which opens OmniPulse)
    pub join_url: String,
    /// what goes to the room (dropping it closes the connection)
    pub tx: mpsc::UnboundedSender<String>,
}

/// A new room on the relay: { code, hostKey, inviteKey, joinUrl }
pub async fn create(relay: &str, kind: &str, host: &str) -> Result<Value, String> {
    let r = reqwest::Client::new()
        .post(format!("{relay}/api/rooms"))
        .json(&json!({ "kind": kind, "host": host }))
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !r.status().is_success() {
        return Err(format!("the relay answered {}", r.status().as_u16()));
    }
    r.json().await.map_err(|e| e.to_string())
}

/// Connects to a room made by create, as its host. Everything the room says goes to \`on\` (and
/// { type: "open" } each time it's connected); a dropped connection is made again a few seconds later,
/// until the Room's sender is dropped.
pub fn connect(rt: &tokio::runtime::Handle, relay: &str, kind: &str, made: &Value, on: impl Fn(Value) + Send + Sync + 'static) -> Room {
    let code = made["code"].as_str().unwrap_or("").to_string();
    let key = made["hostKey"].as_str().unwrap_or("").to_string();
    let url = format!("{}/ws/{code}?role=host&key={}", relay.replacen("http", "ws", 1), crate::sources::urlencode(&key));
    let (tx, mut rx) = mpsc::unbounded_channel::<String>();
    rt.spawn(async move {
        loop {
            if let Ok((ws, _)) = tokio_tungstenite::connect_async(url.as_str()).await {
                let (mut write, mut read) = ws.split();
                on(json!({ "type": "open" }));
                loop {
                    tokio::select! {
                        out = rx.recv() => match out {
                            Some(m) => { if write.send(Message::Text(m.into())).await.is_err() { break; } }
                            // the room's gone from this app: closed
                            None => { let _ = write.close().await; return; }
                        },
                        got = read.next() => match got {
                            Some(Ok(Message::Text(t))) => { if let Ok(v) = serde_json::from_str::<Value>(t.as_str()) { on(v); } }
                            Some(Ok(_)) => {}
                            _ => break,
                        },
                    }
                }
            }
            // dropped (or couldn't connect): again shortly, unless it's been let go meanwhile
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(4)) => {}
                out = rx.recv() => { if out.is_none() { return; } }
            }
        }
    });
    Room { kind: kind.to_string(), join_url: made["joinUrl"].as_str().unwrap_or("").to_string(), tx }
}

/// The ring that empties as an ask runs out (left: 1 full, 0 empty), as a path in a 20 x 20 box
pub fn ring_path(left: f32) -> String {
    let a = (left.clamp(0.0, 0.9999) * std::f32::consts::TAU) as f64;
    if a <= 0.0 {
        return String::new();
    }
    let (x, y) = (10.0 + 7.0 * a.sin(), 11.0 - 7.0 * a.cos());
    format!("M 10 4 A 7 7 0 {} 1 {x:.2} {y:.2}", if a > std::f64::consts::PI { 1 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_empties() {
        assert!(ring_path(0.0).is_empty());
        assert!(ring_path(0.25).starts_with("M 10 4 A 7 7 0 0 1 17.00 11.00"));
        assert!(ring_path(0.75).contains(" 0 1 1 "));
    }
}
