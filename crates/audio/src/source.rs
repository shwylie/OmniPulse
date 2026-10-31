// Where the sound comes from: anything symphonia reads (MP3, AAC/M4A, FLAC, ALAC, Ogg Vorbis, WAV,
// AIFF), from a file or any other seekable reader. Each gives interleaved stereo f32 at its own rate;
// mono is doubled and anything over two channels keeps its front pair.

use std::fs::File;
use std::path::Path;
use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{Decoder, DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error as SymError;
use symphonia::core::formats::{FormatOptions, FormatReader, SeekMode, SeekTo};
use symphonia::core::io::{MediaSource, MediaSourceStream};
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

pub trait Source: Send {
    /// The source's sample rate
    fn rate(&self) -> u32;
    /// Its length, if known
    fn duration_ms(&self) -> Option<u64>;
    /// The next piece of sound, appended to `out` (interleaved stereo); false at the end
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String>;
    /// Goes to a position; returns where it landed
    fn seek(&mut self, ms: u64) -> Result<u64, String>;
    /// Where in the track the sound it last gave is, when that isn't simply how much it has given (a
    /// stretched one's, crate::stretch)
    fn position_ms(&self) -> Option<u64> {
        None
    }
}

pub struct Decoded {
    format: Box<dyn FormatReader>,
    decoder: Box<dyn Decoder>,
    track: u32,
    rate: u32,
    duration_ms: Option<u64>,
    buf: Option<SampleBuffer<f32>>,
    /// sound already decoded, handed out first (what's left after skipping into a block)
    carry: Vec<f32>,
}

impl Decoded {
    pub fn open_file(path: &Path) -> Result<Self, String> {
        let f = File::open(path).map_err(|e| format!("Couldn't open {}: {e}", path.display()))?;
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_string();
        Self::open(Box::new(f), &ext)
    }

    /// Any seekable stream; `ext` (like "mp3") helps find its format
    pub fn open(src: Box<dyn MediaSource>, ext: &str) -> Result<Self, String> {
        let mss = MediaSourceStream::new(src, Default::default());
        let mut hint = Hint::new();
        if !ext.is_empty() {
            hint.with_extension(ext);
        }
        let probed = symphonia::default::get_probe()
            .format(&hint, mss, &FormatOptions { enable_gapless: true, ..Default::default() }, &MetadataOptions::default())
            .map_err(|e| format!("Not a format this can play: {e}"))?;
        let format = probed.format;
        let (track_id, params) = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .map(|t| (t.id, t.codec_params.clone()))
            .ok_or("There's no sound in it.")?;
        let rate = params.sample_rate.ok_or("Its sample rate isn't known.")?;
        let duration_ms = match (params.n_frames, params.time_base) {
            (Some(n), Some(tb)) => {
                let t = tb.calc_time(n);
                Some(t.seconds * 1000 + (t.frac * 1000.0) as u64)
            }
            (Some(n), None) => Some(n * 1000 / rate as u64),
            _ => None,
        };
        let decoder = symphonia::default::get_codecs()
            .make(&params, &DecoderOptions::default())
            .map_err(|e| format!("Can't decode it: {e}"))?;
        Ok(Self { format, decoder, track: track_id, rate, duration_ms, buf: None, carry: Vec::new() })
    }
}

impl Decoded {
    /// Hands this out before decoding any more
    pub fn carry(&mut self, rest: Vec<f32>) {
        self.carry = rest;
    }
}

impl Source for Decoded {
    fn rate(&self) -> u32 {
        self.rate
    }

    fn duration_ms(&self) -> Option<u64> {
        self.duration_ms
    }

    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if !self.carry.is_empty() {
            out.append(&mut self.carry);
            return Ok(true);
        }
        loop {
            let packet = match self.format.next_packet() {
                Ok(p) => p,
                Err(SymError::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(false),
                Err(SymError::ResetRequired) => return Ok(false),
                Err(e) => return Err(e.to_string()),
            };
            if packet.track_id() != self.track {
                continue;
            }
            let decoded = match self.decoder.decode(&packet) {
                Ok(d) => d,
                Err(SymError::DecodeError(_)) => continue, // a damaged packet: skip it
                Err(e) => return Err(e.to_string()),
            };
            let spec = *decoded.spec();
            let ch = spec.channels.count();
            let frames = decoded.frames();
            if frames == 0 {
                continue;
            }
            let buf = self.buf.get_or_insert_with(|| SampleBuffer::new(decoded.capacity() as u64, spec));
            if buf.capacity() < decoded.capacity() * ch {
                *buf = SampleBuffer::new(decoded.capacity() as u64, spec);
            }
            buf.copy_interleaved_ref(decoded);
            let s = &buf.samples()[..frames * ch];
            out.reserve(frames * 2);
            match ch {
                1 => s.iter().for_each(|&v| out.extend_from_slice(&[v, v])),
                2 => out.extend_from_slice(s),
                _ => s.chunks_exact(ch).for_each(|f| out.extend_from_slice(&[f[0], f[1]])),
            }
            return Ok(true);
        }
    }

    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        let to = SeekTo::Time { time: Time::new(ms / 1000, (ms % 1000) as f64 / 1000.0), track_id: Some(self.track) };
        let got = self.format.seek(SeekMode::Accurate, to).map_err(|e| format!("Couldn't seek: {e}"))?;
        self.decoder.reset();
        let tb = self.format.tracks().iter().find(|t| t.id == self.track).and_then(|t| t.codec_params.time_base);
        let Some(tb) = tb else { return Ok(ms) };
        // (it lands on the start of the block holding that time, up to a block early (an m4a's: 23 ms; a
        // YouTube song's next one in a blend came in that late against its beats): decoded on to it)
        let secs = |ts: u64| { let t = tb.calc_time(ts); t.seconds as f64 + t.frac };
        let mut skip = (((secs(got.required_ts) - secs(got.actual_ts)).max(0.0) * self.rate as f64).round() as usize) * 2;
        let mut scratch = Vec::new();
        while skip > 0 {
            scratch.clear();
            if !self.read(&mut scratch)? { break; }
            if scratch.len() > skip { self.carry = scratch.split_off(skip); break; }
            skip -= scratch.len();
        }
        let t = tb.calc_time(got.required_ts);
        Ok(t.seconds * 1000 + (t.frac * 1000.0).round() as u64)
    }
}

/// A tone, for tests and the test player
pub struct Tone {
    pub rate: u32,
    pub freq: f32,
    pub amp: f32,
    pub len_frames: u64,
    pos: u64,
}

impl Tone {
    pub fn new(rate: u32, freq: f32, amp: f32, secs: f32) -> Self {
        Self { rate, freq, amp, len_frames: (rate as f32 * secs) as u64, pos: 0 }
    }
}

impl Source for Tone {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn duration_ms(&self) -> Option<u64> {
        Some(self.len_frames * 1000 / self.rate as u64)
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        if self.pos >= self.len_frames {
            return Ok(false);
        }
        let n = 1024.min(self.len_frames - self.pos);
        for i in 0..n {
            let t = (self.pos + i) as f32 / self.rate as f32;
            let v = self.amp * (2.0 * std::f32::consts::PI * self.freq * t).sin();
            out.extend_from_slice(&[v, v]);
        }
        self.pos += n;
        Ok(true)
    }
    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        self.pos = (ms * self.rate as u64 / 1000).min(self.len_frames);
        Ok(ms)
    }
}

/// Several sources heard together (tones, for tests: each gives as much at a time)
pub struct Sum(pub Vec<Tone>);

impl Source for Sum {
    fn rate(&self) -> u32 {
        self.0.first().map(|t| t.rate).unwrap_or(44_100)
    }
    fn duration_ms(&self) -> Option<u64> {
        self.0.iter().filter_map(|t| t.duration_ms()).max()
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        let (from, mut more, mut part) = (out.len(), false, Vec::new());
        for t in &mut self.0 {
            part.clear();
            more |= t.read(&mut part)?;
            if out.len() < from + part.len() { out.resize(from + part.len(), 0.0); }
            for (o, v) in out[from..].iter_mut().zip(&part) { *o += v; }
        }
        Ok(more)
    }
    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        for t in &mut self.0 { t.seek(ms)?; }
        Ok(ms)
    }
}
