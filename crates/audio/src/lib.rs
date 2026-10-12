// OmniPulse's playback engine, for the native app (the web app plays through WebView2 instead).

pub mod dsp;
pub mod engine;
pub mod hls;
pub mod http;
pub mod push;
pub mod source;
pub mod stretch;
pub mod ts;

pub use engine::{render, Engine, Event, Handoff, Loop, Perf, Ramp, Sweep, Tap};
pub use hls::HlsSource;
pub use http::{open_url, open_url_refreshing};
pub use push::{PushFeed, PushSource};
pub use source::{Decoded, Source, Tone};
pub use stretch::{Stretched, Tempo};

#[cfg(test)]
mod tests {
    use super::*;

    /// A WAV file's sound (16-bit PCM), for checking what came out
    fn write_wav(path: &std::path::Path, rate: u32, frames: &[f32]) {
        let mut b = Vec::new();
        let data = (frames.len() * 2) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 4).to_le_bytes());
        b.extend_from_slice(&4u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for s in frames {
            b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes());
        }
        std::fs::write(path, b).unwrap();
    }

    #[test]
    fn a_file_plays_through_the_chain_at_the_output_rate() {
        // a 44.1 kHz file (like most music), played on a 48 kHz device
        let dir = std::env::temp_dir().join(format!("op-audio-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("tone.wav");
        let tone: Vec<f32> = (0..44100).flat_map(|i| {
            let v = 0.25 * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 44100.0).sin();
            [v, v]
        }).collect();
        write_wav(&wav, 44100, &tone);
        let src = Decoded::open_file(&wav).unwrap();
        assert_eq!(src.rate(), 44100);
        assert_eq!(src.duration_ms(), Some(1000));
        let out = render(Box::new(src), 48000, &[0.0; 10], -6.0, 5.0).unwrap();
        // a second of sound at the new rate (give or take the converter's edges)
        let frames = out.len() / 2;
        assert!((frames as i64 - 48000).abs() < 1200, "{frames}");
        // -12 dBFS, turned down 6 dB, plus the limiter's make-up gain
        let (_, peak) = dsp::level(&out[out.len() / 4..out.len() * 3 / 4]);
        assert!((peak - (-12.04 - 6.0 + 0.855)).abs() < 0.3, "{peak}");
        // and seeking lands where asked
        let mut src = Decoded::open_file(&wav).unwrap();
        let at = src.seek(500).unwrap();
        assert!((at as i64 - 500).abs() < 30, "{at}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_web_address_plays_and_seeks_with_ranges() {
        // a small server for a 3 s WAV that answers range requests, like SoundCloud's or YouTube's
        let dir = std::env::temp_dir().join(format!("op-audio-http-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("t.wav");
        let tone: Vec<f32> = (0..3 * 44100).flat_map(|i| {
            let v = 0.25 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44100.0).sin();
            [v, v]
        }).collect();
        write_wav(&wav, 44100, &tone);
        let bytes = std::sync::Arc::new(std::fs::read(&wav).unwrap());
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        let ranged = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        {
            let (bytes, ranged) = (bytes.clone(), ranged.clone());
            std::thread::spawn(move || {
                for req in server.incoming_requests() {
                    let range = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.as_str().to_string());
                    let total = bytes.len();
                    match range.and_then(|r| r.strip_prefix("bytes=").map(String::from)) {
                        Some(r) => {
                            ranged.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                            let (a, b) = r.split_once('-').unwrap();
                            let a: usize = a.parse().unwrap();
                            let b: usize = b.parse::<usize>().unwrap_or(total - 1).min(total - 1);
                            let body = bytes[a..=b].to_vec();
                            let res = tiny_http::Response::from_data(body).with_status_code(206)
                                .with_header(tiny_http::Header::from_bytes("Content-Range", format!("bytes {a}-{b}/{total}")).unwrap());
                            let _ = req.respond(res);
                        }
                        None => { let _ = req.respond(tiny_http::Response::from_data(bytes.to_vec())); }
                    }
                }
            });
        }
        let url = format!("http://127.0.0.1:{port}/t.wav");
        let src = open_url(&url, "wav").unwrap();
        assert_eq!(src.duration_ms(), Some(3000));
        let out = render(Box::new(src), 48000, &[0.0; 10], 0.0, 10.0).unwrap();
        assert!((out.len() as i64 / 2 - 3 * 48000).abs() < 1500, "{}", out.len() / 2);
        // a seek near the end fetches only from there
        let mut src = open_url(&url, "wav").unwrap();
        let before = ranged.load(std::sync::atomic::Ordering::SeqCst);
        let at = src.seek(2500).unwrap();
        assert!((at as i64 - 2500).abs() < 30, "{at}");
        let mut piece = Vec::new();
        assert!(src.read(&mut piece).unwrap() && !piece.is_empty());
        assert!(ranged.load(std::sync::atomic::Ordering::SeqCst) - before <= 2);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_address_turned_down_partway_is_swapped_for_a_fresh_one() {
        // the old address answers its first piece, then 403s (as YouTube's can); a fresh one plays on
        let dir = std::env::temp_dir().join(format!("op-audio-403-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let wav = dir.join("t.wav");
        let tone: Vec<f32> = (0..4 * 44100).flat_map(|i| {
            let v = 0.25 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 44100.0).sin();
            [v, v]
        }).collect();
        write_wav(&wav, 44100, &tone);
        let bytes = std::sync::Arc::new(std::fs::read(&wav).unwrap());
        let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
        let port = server.server_addr().to_ip().unwrap().port();
        {
            let bytes = bytes.clone();
            std::thread::spawn(move || {
                for req in server.incoming_requests() {
                    let old = req.url().starts_with("/old");
                    let r = req.headers().iter().find(|h| h.field.equiv("Range")).map(|h| h.value.as_str().to_string()).unwrap_or_default();
                    let total = bytes.len();
                    let (a, b) = r.trim_start_matches("bytes=").split_once('-').unwrap();
                    let a: usize = a.parse().unwrap();
                    let b: usize = b.parse::<usize>().unwrap().min(total - 1);
                    if old && a > 0 { let _ = req.respond(tiny_http::Response::empty(403)); continue; }
                    let res = tiny_http::Response::from_data(bytes[a..=b].to_vec()).with_status_code(206)
                        .with_header(tiny_http::Header::from_bytes("Content-Range", format!("bytes {a}-{b}/{total}")).unwrap());
                    let _ = req.respond(res);
                }
            });
        }
        let asked = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let a2 = asked.clone();
        let fresh = format!("http://127.0.0.1:{port}/new.wav");
        let src = open_url_refreshing(&format!("http://127.0.0.1:{port}/old.wav"), "wav", Box::new(move || { a2.fetch_add(1, std::sync::atomic::Ordering::SeqCst); Ok(fresh.clone()) })).unwrap();
        let out = render(Box::new(src), 48000, &[0.0; 10], 0.0, 10.0).unwrap();
        assert!((out.len() as i64 / 2 - 4 * 48000).abs() < 1500, "{}", out.len() / 2);
        assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
