// Measures a file the way OmniPulse does:  cargo run -p omnipulse-core --example measure -- <file> [--ts]
// With --ts, an AAC file (.m4a) is first repackaged as MPEG-TS (ADTS frames in PES packets), the way
// HLS streams deliver it, to check that path too.

use omnipulse_core::loudness::{measure, ts_audio};
use std::io::Cursor;
use symphonia::core::{codecs::CODEC_TYPE_NULL, formats::FormatOptions, io::MediaSourceStream, meta::MetadataOptions, probe::Hint};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let path = args.first().expect("usage: measure <file> [--ts]");
    let ext = std::path::Path::new(path).extension().and_then(|e| e.to_str()).map(str::to_lowercase);
    let start = std::time::Instant::now();
    let result = if args.iter().any(|a| a == "--ts") {
        let ts = to_ts(path);
        let (es, kind) = ts_audio(&ts).expect("no audio found in the TS");
        println!("TS: {} bytes -> {} bytes of {kind}", ts.len(), es.len());
        measure(Box::new(Cursor::new(es)), Some(kind), None)
    } else {
        measure(Box::new(std::fs::File::open(path).expect("can't open")), ext.as_deref(), None)
    };
    println!("{}", serde_json::to_string_pretty(&result).unwrap());
    println!("took {:?}", start.elapsed());
}

// AAC access units from an MP4, each given an ADTS header, in PES packets, in TS packets
fn to_ts(path: &str) -> Vec<u8> {
    let file = std::fs::File::open(path).expect("can't open");
    let mss = MediaSourceStream::new(Box::new(file), Default::default());
    let mut hint = Hint::new();
    hint.with_extension("m4a");
    let mut format = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .expect("not an MP4")
        .format;
    let track = format.tracks().iter().find(|t| t.codec_params.codec != CODEC_TYPE_NULL).unwrap().clone();
    let rate = track.codec_params.sample_rate.unwrap();
    let chans = track.codec_params.channels.map(|c| c.count()).unwrap_or(2) as u8;
    let sfi = [96000, 88200, 64000, 48000, 44100, 32000, 24000, 22050, 16000, 12000, 11025, 8000]
        .iter()
        .position(|r| *r == rate)
        .unwrap() as u8;
    let mut es = Vec::new();
    while let Ok(p) = format.next_packet() {
        let len = p.data.len() + 7;
        es.extend_from_slice(&[
            0xFF,
            0xF1,
            (1 << 6) | (sfi << 2) | (chans >> 2), // AAC LC
            ((chans & 3) << 6) | (len >> 11) as u8,
            (len >> 3) as u8,
            (((len & 7) as u8) << 5) | 0x1F,
            0xFC,
        ]);
        es.extend_from_slice(&p.data);
    }
    let pat = [0x00, 0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00, 0, 0, 0, 0];
    let pmt = [0x00, 0x02, 0xB0, 0x12, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x01, 0xF0, 0x00, 0x0F, 0xE1, 0x01, 0xF0, 0x00, 0, 0, 0, 0];
    let mut ts = Vec::new();
    for (n, chunk) in es.chunks(4000).enumerate() {
        if n % 20 == 0 {
            ts.extend(packet(0, true, &pat));
            ts.extend(packet(0x1000, true, &pmt));
        }
        let mut pes = vec![0, 0, 1, 0xC0, 0, 0, 0x80, 0x00, 0];
        pes.extend_from_slice(chunk);
        for (i, piece) in pes.chunks(184).enumerate() {
            ts.extend(packet(0x101, i == 0, piece));
        }
    }
    ts
}

fn packet(pid: u16, pusi: bool, payload: &[u8]) -> Vec<u8> {
    let mut p = vec![0x47, ((pusi as u8) << 6) | (pid >> 8) as u8, pid as u8];
    if payload.len() < 184 {
        let stuff = 184 - payload.len() - 1;
        p.push(0x30);
        p.push(stuff as u8);
        if stuff > 0 {
            p.push(0);
            p.extend(std::iter::repeat_n(0xFF, stuff - 1));
        }
    } else {
        p.push(0x10);
    }
    p.extend_from_slice(payload);
    p
}
