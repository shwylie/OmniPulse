// One OmniPulse at a time (as the web app's single-instance plugin): the first one listens on a port of
// this PC's own; a second one started (from an omnipulse:// link, or the Start menu again) hands what it
// was started with to the first, which comes to the front, and then closes.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::time::Duration;

const PORT: u16 = 47_821;
const HELLO: &str = "omnipulse-native 1";

/// The first one: what later ones hand over ("show", or an omnipulse:// link) goes to `on`, and Ok.
/// Another OmniPulse is running already: what this one was started with is handed to it, and Err (close).
pub fn claim(arg: Option<String>, on: impl Fn(String) + Send + 'static) -> Result<(), ()> {
    match TcpListener::bind(("127.0.0.1", PORT)) {
        Ok(listener) => {
            std::thread::Builder::new().name("op-instance".into()).spawn(move || {
                for conn in listener.incoming().flatten() {
                    let _ = conn.set_read_timeout(Some(Duration::from_secs(2)));
                    let Ok(mut back) = conn.try_clone() else { continue };
                    let mut lines = BufReader::new(conn).lines();
                    // (only another OmniPulse: each says hello first)
                    if lines.next().and_then(|l| l.ok()).as_deref() != Some(HELLO) { continue; }
                    if writeln!(back, "{HELLO}").is_err() { continue; }
                    if let Some(Ok(msg)) = lines.next() {
                        let msg = msg.trim().to_string();
                        if msg == "show" || msg.starts_with("omnipulse://") { on(msg); }
                    }
                }
            }).map_err(|_| ())?;
            Ok(())
        }
        Err(_) => {
            // (something else has the port, or doesn't answer as OmniPulse: carry on as the only one)
            let Ok(s) = TcpStream::connect_timeout(&std::net::SocketAddr::from(([127, 0, 0, 1], PORT)), Duration::from_secs(2)) else { return Ok(()) };
            let _ = s.set_read_timeout(Some(Duration::from_secs(2)));
            let Ok(mut w) = s.try_clone() else { return Ok(()) };
            if writeln!(w, "{HELLO}").is_err() { return Ok(()); }
            let mut reply = String::new();
            if BufReader::new(s).read_line(&mut reply).is_err() || reply.trim() != HELLO { return Ok(()); }
            let msg = arg.filter(|a| a.starts_with("omnipulse://")).unwrap_or_else(|| "show".into());
            if writeln!(w, "{msg}").is_ok() { Err(()) } else { Ok(()) }
        }
    }
}

/// Restarting (Hardware Acceleration's Restart Now): waits, up to ten seconds, for the one before to close
pub fn wait_for_the_last() {
    for _ in 0..50 {
        if TcpListener::bind(("127.0.0.1", PORT)).is_ok() { return; }
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_second_one_hands_over() {
        let (tx, rx) = std::sync::mpsc::channel();
        assert!(super::claim(None, move |m| { let _ = tx.send(m); }).is_ok());
        assert!(super::claim(Some("omnipulse://playlist/abcd1234?e=k".into()), |_| {}).is_err());
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(), "omnipulse://playlist/abcd1234?e=k");
        assert!(super::claim(None, |_| {}).is_err());
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap(), "show");
    }
}
