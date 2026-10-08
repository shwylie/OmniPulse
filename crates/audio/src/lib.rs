// OmniPulse's playback engine, for the native app (the web app plays through WebView2 instead).

pub mod dsp;
pub mod engine;
pub mod source;

pub use engine::{render, Engine, Event};
pub use source::{Decoded, Source, Tone};

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
}
