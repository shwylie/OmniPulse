// A test player for the engine:
//   op-play <file> [--eq bass|vocal|treble|smile] [--volume 0.5] [--match -6] [--secs 10] [--seek 30]
//   op-play <file> --wav out.wav     renders it through the chain to a WAV, without playing it
//   op-play --devices                lists output devices
//   op-play --tone                   a quiet 440 Hz tone, for checking the output

use omnipulse_audio::{open_url, render, Decoded, Engine, Event, HlsSource, Source, Tone};
use std::time::{Duration, Instant};

fn preset(name: &str) -> [f32; 10] {
    match name {
        "bass" => [6., 5., 4., 2., 0., 0., 0., 0., 0., 0.],
        "vocal" => [-2., -2., -1., 1., 3., 3., 2., 1., 0., -1.],
        "treble" => [0., 0., 0., 0., 0., 1., 2., 4., 5., 6.],
        "smile" => [5., 4., 2., 0., -1., -1., 0., 2., 4., 5.],
        _ => [0.; 10],
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let opt = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    if args.iter().any(|a| a == "--devices") {
        Engine::devices().iter().for_each(|d| println!("{d}"));
        return;
    }
    let eq = preset(&opt("--eq").unwrap_or_default());
    let match_db: f32 = opt("--match").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let secs: f32 = opt("--secs").and_then(|v| v.parse().ok()).unwrap_or(f32::MAX);
    let open = || -> Box<dyn Source> {
        if args.iter().any(|a| a == "--tone") {
            return Box::new(Tone::new(48000, 440.0, 0.05, 3.0));
        }
        let path = args.iter().find(|a| !a.starts_with("--") && !args.iter().any(|b| b.starts_with("--") && opt(b).as_deref() == Some(a.as_str()))).expect("a file to play");
        // a web address: an HLS playlist, or a file on the web
        let opened: Result<Box<dyn Source>, String> = if path.starts_with("http") {
            if path.contains(".m3u8") || path.contains("playlist") { HlsSource::open(path).map(|h| Box::new(h) as Box<dyn Source>) }
            else { open_url(path, "").map(|d| Box::new(d) as Box<dyn Source>) }
        } else {
            Decoded::open_file(std::path::Path::new(path)).map(|d| Box::new(d) as Box<dyn Source>)
        };
        opened.unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(1) })
    };

    if let Some(out) = opt("--wav") {
        let t = Instant::now();
        let mut src = open();
        if let Some(at) = opt("--seek").and_then(|v| v.parse::<u64>().ok()) {
            src.seek(at * 1000).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(1) });
        }
        let pcm = render(src, 48000, &eq, match_db, secs.min(600.0)).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(1) });
        let secs_out = pcm.len() as f32 / 2.0 / 48000.0;
        let mut b = Vec::new();
        let data = (pcm.len() * 2) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] { b.extend_from_slice(&v.to_le_bytes()); }
        for v in [48000u32, 192000] { b.extend_from_slice(&v.to_le_bytes()); }
        for v in [4u16, 16] { b.extend_from_slice(&v.to_le_bytes()); }
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for s in &pcm { b.extend_from_slice(&((s.clamp(-1.0, 1.0) * 32767.0) as i16).to_le_bytes()); }
        std::fs::write(&out, b).expect("write");
        let el = t.elapsed().as_secs_f32();
        println!("rendered {secs_out:.1} s in {el:.2} s ({:.0}x real time) to {out}", secs_out / el.max(1e-6));
        return;
    }

    let engine = Engine::new(&opt("--device").unwrap_or_default()).unwrap_or_else(|e| { eprintln!("{e}"); std::process::exit(1) });
    let events = engine.events().unwrap();
    engine.set_volume(opt("--volume").and_then(|v| v.parse().ok()).unwrap_or(1.0));
    engine.set_eq(&eq);
    engine.load(open(), match_db);
    if let Some(s) = opt("--seek").and_then(|v| v.parse::<u64>().ok()) {
        engine.seek(s * 1000);
    }
    engine.play();
    println!("playing at {} Hz", engine.output_rate());
    let start = Instant::now();
    loop {
        match events.recv_timeout(Duration::from_millis(1000)) {
            Ok(Event::Ended) => { println!("ended at {} ms", engine.position_ms()); break; }
            Ok(Event::Error(e)) => { eprintln!("error: {e}"); break; }
            Err(_) => {
                let (rms, peak) = engine.level();
                println!("{:>6.1} s / {:.1} s   level {rms:.1} dB rms, {peak:.1} dB peak", engine.position_ms() as f32 / 1000.0, engine.duration_ms() as f32 / 1000.0);
            }
        }
        if start.elapsed().as_secs_f32() > secs {
            break;
        }
    }
}
