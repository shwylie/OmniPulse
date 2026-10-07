// Whole-track loudness and silences, so every track can play at the same loudness and long
// silences can be skipped.
//
// A track is decoded (symphonia) and measured the way streaming services do it: integrated
// loudness in LUFS (EBU R128 / ITU BS.1770, via the ebur128 crate) plus its sample peak. Along the
// way, stretches of silence of 2 s or more are noted. Results are kept in <data dir>/loudness.json.
//
// Local files are read from disk. SoundCloud tracks are downloaded: progressive MP3, or HLS
// (MPEG-TS, fragmented MP4 or packed audio segments). Tracks longer than LONG_MS (sets, podcasts)
// are only measured over their first MEASURE_MS, which is a good guide to the whole and saves
// downloading hours of audio. Spotify needs none of this: librespot applies Spotify's own
// whole-track values (see spotify_engine::spe_set_normalisation).

use crate::{data_dir, local, soundcloud};
use futures::stream::{self, StreamExt, TryStreamExt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Cursor;
use std::sync::Mutex;
use symphonia::core::{
    audio::SampleBuffer,
    codecs::{DecoderOptions, CODEC_TYPE_NULL},
    errors::Error as SymError,
    formats::FormatOptions,
    io::{MediaSource, MediaSourceStream},
    meta::MetadataOptions,
    probe::Hint,
};

const VERSION: &str = "v1";
const LONG_MS: u64 = 15 * 60 * 1000; // longer than this: only the start is measured
const MEASURE_MS: u64 = 10 * 60 * 1000; // ...namely the first ten minutes
const PROGRESSIVE_MAX_BYTES: u64 = 10 * 1024 * 1024; // ~10 minutes at 128 kbps (YouTube serves at most this per request)
const MAX_BYTES: usize = 120 * 1024 * 1024; // never hold more than this in memory
const WINDOW_MS: u64 = 50; // silence is judged in windows this long
const SILENT_RMS_DB: f64 = -55.0; // a window is silent below this level...
const SILENT_PEAK_DB: f64 = -40.0; // ...with no peak above this
const MIN_SILENCE_MS: u64 = 2000; // shorter silences aren't noted

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct Analysis {
    /// Integrated loudness, LUFS
    pub lufs: f64,
    /// Highest sample, dBFS
    pub peak_db: f64,
    /// How long the track is (from the stream's own length when only part was measured)
    pub duration_ms: u64,
    /// How much of it was measured: silences after this point aren't known
    pub analysed_ms: u64,
    /// Silent stretches of MIN_SILENCE_MS or more, [from, to] in ms
    pub silences: Vec<[u64; 2]>,
}

// ---------- measuring ----------

struct Meter {
    r128: ebur128::EbuR128,
    channels: usize,
    rate: u64,
    frames: u64,
    win_frames: u64,
    win_sq: f64,
    win_peak: f32,
    win_n: u64,
    silent_from: Option<u64>, // frame where the current silent stretch began
    silences: Vec<[u64; 2]>,
    peak: f32,
}

impl Meter {
    fn new(channels: usize, rate: u32) -> Result<Meter, String> {
        let r128 = ebur128::EbuR128::new(channels as u32, rate, ebur128::Mode::I)
            .map_err(|e| format!("loudness meter: {e:?}"))?;
        Ok(Meter {
            r128,
            channels,
            rate: rate as u64,
            frames: 0,
            win_frames: (rate as u64 * WINDOW_MS / 1000).max(1),
            win_sq: 0.0,
            win_peak: 0.0,
            win_n: 0,
            silent_from: None,
            silences: Vec::new(),
            peak: 0.0,
        })
    }

    fn ms(&self, frames: u64) -> u64 {
        frames * 1000 / self.rate
    }

    fn close_silence(&mut self, end_frame: u64) {
        if let Some(from) = self.silent_from.take() {
            let (a, b) = (self.ms(from), self.ms(end_frame));
            if b - a >= MIN_SILENCE_MS {
                self.silences.push([a, b]);
            }
        }
    }

    fn end_window(&mut self) {
        let rms = (self.win_sq / (self.win_n.max(1) * self.channels as u64) as f64).sqrt();
        let silent = db(rms) < SILENT_RMS_DB && db(self.win_peak as f64) < SILENT_PEAK_DB;
        let start = self.frames - self.win_n;
        if silent {
            self.silent_from.get_or_insert(start);
        } else {
            self.close_silence(start);
        }
        self.win_sq = 0.0;
        self.win_peak = 0.0;
        self.win_n = 0;
    }

    /// Interleaved samples, `channels` per frame
    fn push(&mut self, samples: &[f32]) -> Result<(), String> {
        self.r128.add_frames_f32(samples).map_err(|e| format!("loudness meter: {e:?}"))?;
        for frame in samples.chunks_exact(self.channels) {
            for &s in frame {
                let a = s.abs();
                self.win_sq += (s as f64) * (s as f64);
                if a > self.win_peak {
                    self.win_peak = a;
                }
                if a > self.peak {
                    self.peak = a;
                }
            }
            self.frames += 1;
            self.win_n += 1;
            if self.win_n == self.win_frames {
                self.end_window();
            }
        }
        Ok(())
    }

    fn finish(mut self, duration_ms: Option<u64>) -> Result<Analysis, String> {
        if self.win_n > 0 {
            self.end_window();
        }
        let analysed_ms = self.ms(self.frames);
        let duration_ms = duration_ms.filter(|d| *d > analysed_ms).unwrap_or(analysed_ms);
        // silence that runs to the end of what was measured: to the end of the track if that's
        // all of it, otherwise only as far as we know
        self.close_silence(self.frames);
        let lufs = self.r128.loudness_global().map_err(|e| format!("loudness meter: {e:?}"))?;
        Ok(Analysis {
            // all silence (or too short to measure) comes out as -inf: call it very quiet
            lufs: if lufs.is_finite() { round1(lufs) } else { -70.0 },
            peak_db: if self.peak > 0.0 { round1(db(self.peak as f64)) } else { -100.0 },
            duration_ms,
            analysed_ms,
            silences: self.silences,
        })
    }
}

fn db(x: f64) -> f64 {
    20.0 * x.max(1e-10).log10()
}

fn round1(x: f64) -> f64 {
    (x * 10.0).round() / 10.0
}

/// Decodes `source` and measures it (blocking: run it on a blocking thread)
pub fn measure(source: Box<dyn MediaSource>, ext: Option<&str>, duration_ms: Option<u64>) -> Result<Analysis, String> {
    let mut hint = Hint::new();
    if let Some(e) = ext {
        hint.with_extension(e);
    }
    let mss = MediaSourceStream::new(source, Default::default());
    let probed = symphonia::default::get_probe()
        .format(&hint, mss, &FormatOptions::default(), &MetadataOptions::default())
        .map_err(|e| format!("unreadable audio: {e}"))?;
    let mut format = probed.format;
    let track = format
        .tracks()
        .iter()
        .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
        .ok_or("no audio in it")?;
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(&track.codec_params, &DecoderOptions::default())
        .map_err(|e| format!("can't decode it: {e}"))?;

    let mut meter: Option<Meter> = None;
    let mut buf: Option<SampleBuffer<f32>> = None;
    loop {
        let packet = match format.next_packet() {
            Ok(p) => p,
            Err(SymError::IoError(_)) | Err(SymError::ResetRequired) => break, // the end (or a cut-off download)
            Err(e) => return Err(format!("reading the audio failed: {e}")),
        };
        if packet.track_id() != track_id {
            continue;
        }
        let decoded = match decoder.decode(&packet) {
            Ok(d) => d,
            Err(SymError::DecodeError(_)) => continue, // a damaged frame: skip it
            Err(SymError::IoError(_)) => break,
            Err(e) => return Err(format!("decoding failed: {e}")),
        };
        let spec = *decoded.spec();
        if decoded.frames() == 0 {
            continue;
        }
        if meter.is_none() {
            meter = Some(Meter::new(spec.channels.count(), spec.rate)?);
        }
        let need = decoded.capacity() as u64;
        if buf.as_ref().is_none_or(|b| (b.capacity() as u64) < need * spec.channels.count() as u64) {
            buf = Some(SampleBuffer::new(need, spec));
        }
        let b = buf.as_mut().unwrap();
        b.copy_interleaved_ref(decoded);
        meter.as_mut().unwrap().push(b.samples())?;
    }
    meter.ok_or("there was no audio to measure")?.finish(duration_ms)
}

// ---------- the cache ----------

static CACHE: Mutex<Option<HashMap<String, Analysis>>> = Mutex::new(None);

fn cache_file() -> Option<std::path::PathBuf> {
    data_dir().ok().map(|d| d.join("loudness.json"))
}

fn cached(key: &str) -> Option<Analysis> {
    let mut g = CACHE.lock().ok()?;
    if g.is_none() {
        let loaded = cache_file()
            .and_then(|f| std::fs::read_to_string(f).ok())
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        *g = Some(loaded);
    }
    g.as_ref()?.get(key).cloned()
}

/// Drops the remembered measurements whose keys start with this
pub fn forget(prefix: &str) {
    let _ = cached(""); // makes sure the file has been read
    let Ok(mut g) = CACHE.lock() else { return };
    if let Some(map) = g.as_mut() {
        map.retain(|k, _| !k.starts_with(prefix));
        if let (Some(f), Ok(text)) = (cache_file(), serde_json::to_string(&*map)) {
            let _ = std::fs::write(f, text);
        }
    }
}

fn remember(key: String, a: &Analysis) {
    let Ok(mut g) = CACHE.lock() else { return };
    let map = g.get_or_insert_with(HashMap::new);
    map.insert(key, a.clone());
    if let (Some(f), Ok(text)) = (cache_file(), serde_json::to_string(&*map)) {
        let _ = std::fs::write(f, text);
    }
}

// ---------- local files ----------

pub async fn analyse_local(id: &str) -> Result<Analysis, String> {
    let e = local::local_entry(id).ok_or("this file isn't in your music folders any more")?;
    let key = format!("{VERSION}:local:{}:{}:{}", e.id, e.size, e.mtime);
    if let Some(a) = cached(&key) {
        return Ok(a);
    }
    let path = std::path::PathBuf::from(&e.path);
    let ext = path.extension().and_then(|x| x.to_str()).map(|x| x.to_lowercase());
    let a = tokio::task::spawn_blocking(move || {
        let file = std::fs::File::open(&path).map_err(|e| format!("can't open the file: {e}"))?;
        measure(Box::new(file), ext.as_deref(), None)
    })
    .await
    .map_err(|e| e.to_string())??;
    remember(key, &a);
    Ok(a)
}

// ---------- SoundCloud ----------

pub async fn analyse_soundcloud(id: &str) -> Result<Analysis, String> {
    let key = format!("{VERSION}:sc:{id}");
    if let Some(a) = cached(&key) {
        return Ok(a);
    }
    let info = soundcloud::stream_url(id.to_string()).await?;
    let mut failures = Vec::new();
    for c in info["candidates"].as_array().cloned().unwrap_or_default() {
        let name = c["name"].as_str().unwrap_or("").to_string();
        if name == "preview" {
            continue; // a 30-second preview says nothing about the track
        }
        let resolved = match soundcloud::stream_resolve(c["url"].as_str().unwrap_or("").to_string()).await {
            Ok(r) => r,
            Err(e) => {
                failures.push(format!("{name}: {e}"));
                continue;
            }
        };
        let url = resolved["url"].as_str().unwrap_or("").to_string();
        let auth = resolved["auth"].as_str().map(String::from);
        match fetch_audio(&url, auth.as_deref()).await {
            Ok(f) => {
                let res = tokio::task::spawn_blocking(move || {
                    measure(Box::new(Cursor::new(f.bytes)), f.ext, f.duration_ms)
                })
                .await
                .map_err(|e| e.to_string())?;
                match res {
                    Ok(a) => {
                        remember(key, &a);
                        return Ok(a);
                    }
                    Err(e) => failures.push(format!("{name}: {e}")),
                }
            }
            Err(e) => failures.push(format!("{name}: {e}")),
        }
    }
    Err(format!("couldn't measure this track ({})", failures.join("; ")))
}

// ---------- YouTube ----------

pub async fn analyse_youtube(id: &str) -> Result<Analysis, String> {
    let key = format!("{VERSION}:yt:{id}");
    if let Some(a) = cached(&key) {
        return Ok(a);
    }
    let url = crate::youtube::stream_url(id, false).await?;
    let f = fetch_audio(&url, None).await?;
    let a = tokio::task::spawn_blocking(move || measure(Box::new(Cursor::new(f.bytes)), f.ext, f.duration_ms))
        .await
        .map_err(|e| e.to_string())??;
    remember(key, &a);
    Ok(a)
}

pub struct Fetched {
    pub bytes: Vec<u8>,
    pub ext: Option<&'static str>,
    pub duration_ms: Option<u64>,
}

fn request(client: &reqwest::Client, url: &str, auth: Option<&str>) -> reqwest::RequestBuilder {
    let mut rb = client.get(url);
    if let (Some(a), Ok(u)) = (auth, reqwest::Url::parse(url)) {
        if u.host_str() == Some("api.soundcloud.com") {
            rb = rb.header("Authorization", a);
        }
    }
    rb
}

/// Downloads a stream (progressive file or HLS playlist) as one decodable piece of audio
pub async fn fetch_audio(url: &str, auth: Option<&str>) -> Result<Fetched, String> {
    let client = reqwest::Client::new();
    let mut url = url.to_string();
    for _ in 0..3 {
        // a playlist is small, a file is fetched in part: ask for the start only either way
        let res = request(&client, &url, auth)
            .header("Range", format!("bytes=0-{}", PROGRESSIVE_MAX_BYTES - 1))
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if !res.status().is_success() {
            return Err(format!("the stream request failed ({})", res.status()));
        }
        let base = res.url().clone();
        let bytes = res.bytes().await.map_err(|e| e.to_string())?.to_vec();
        if !bytes.starts_with(b"#EXTM3U") {
            let ext = sniff(&bytes);
            return Ok(Fetched { bytes, ext, duration_ms: None });
        }
        let pl = parse_m3u8(&String::from_utf8_lossy(&bytes), &base)?;
        if let Some(v) = pl.variants.first() {
            url = v.clone(); // a list of qualities: take the first and read again
            continue;
        }
        return fetch_segments(&client, pl, auth).await;
    }
    Err("the playlist points to itself".into())
}

#[derive(Default, Debug)]
pub struct Playlist {
    pub variants: Vec<String>,
    pub map: Option<String>,
    pub segments: Vec<(f64, String)>,
}

pub fn parse_m3u8(text: &str, base: &reqwest::Url) -> Result<Playlist, String> {
    let abs = |u: &str| base.join(u).map(|x| x.to_string()).map_err(|e| e.to_string());
    let mut pl = Playlist::default();
    let mut dur: Option<f64> = None;
    let mut variant_next = false;
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if let Some(rest) = line.strip_prefix("#EXT-X-KEY:") {
            if !rest.contains("METHOD=NONE") {
                return Err("the stream is encrypted".into());
            }
        } else if let Some(rest) = line.strip_prefix("#EXT-X-MAP:") {
            if let Some(uri) = attr(rest, "URI") {
                pl.map = Some(abs(&uri)?);
            }
        } else if let Some(rest) = line.strip_prefix("#EXTINF:") {
            dur = rest.split(',').next().and_then(|d| d.trim().parse().ok());
        } else if line.starts_with("#EXT-X-STREAM-INF") {
            variant_next = true;
        } else if !line.starts_with('#') {
            if variant_next {
                pl.variants.push(abs(line)?);
                variant_next = false;
            } else {
                pl.segments.push((dur.take().unwrap_or(0.0), abs(line)?));
            }
        }
    }
    if pl.variants.is_empty() && pl.segments.is_empty() {
        return Err("the playlist is empty".into());
    }
    Ok(pl)
}

fn attr(list: &str, name: &str) -> Option<String> {
    let at = list.find(&format!("{name}="))? + name.len() + 1;
    let rest = &list[at..];
    if let Some(q) = rest.strip_prefix('"') {
        q.split('"').next().map(String::from)
    } else {
        rest.split(',').next().map(String::from)
    }
}

async fn fetch_segments(client: &reqwest::Client, pl: Playlist, auth: Option<&str>) -> Result<Fetched, String> {
    let total_ms = (pl.segments.iter().map(|(d, _)| d).sum::<f64>() * 1000.0) as u64;
    // a set or podcast: only its first stretch
    let mut wanted = Vec::new();
    let mut ms = 0.0;
    for (d, u) in &pl.segments {
        if total_ms > LONG_MS && ms >= MEASURE_MS as f64 {
            break;
        }
        wanted.push(u.clone());
        ms += d * 1000.0;
    }
    let get = |u: String| async move {
        let r = request(client, &u, auth).send().await.map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("a piece of the stream failed ({})", r.status()));
        }
        r.bytes().await.map(|b| b.to_vec()).map_err(|e| e.to_string())
    };
    let mut parts: Vec<Vec<u8>> = Vec::new();
    if let Some(m) = &pl.map {
        parts.push(get(m.clone()).await?);
    }
    let segs: Vec<Vec<u8>> = stream::iter(wanted).map(get).buffered(4).try_collect().await?;
    if segs.iter().map(Vec::len).sum::<usize>() > MAX_BYTES {
        return Err("the stream is too big to measure".into());
    }
    let duration_ms = (total_ms > 0).then_some(total_ms);
    if pl.map.is_some() {
        // fragmented MP4: the init section, then the fragments
        parts.extend(segs);
        return Ok(Fetched { bytes: parts.concat(), ext: Some("mp4"), duration_ms });
    }
    let first = segs.first().ok_or("the playlist is empty")?;
    if first.len() >= 189 && first[0] == 0x47 && first[188] == 0x47 {
        let (bytes, ext) = ts_audio(&segs.concat()).ok_or("no audio found in the MPEG-TS stream")?;
        return Ok(Fetched { bytes, ext: Some(ext), duration_ms });
    }
    // packed audio (AAC or MP3), each piece usually starting with an ID3 timestamp
    let bytes: Vec<u8> = segs.iter().flat_map(|s| strip_id3(s).iter().copied()).collect();
    let ext = sniff(&bytes);
    Ok(Fetched { bytes, ext, duration_ms })
}

fn strip_id3(b: &[u8]) -> &[u8] {
    if b.len() >= 10 && b.starts_with(b"ID3") {
        let size = ((b[6] as usize & 0x7f) << 21) | ((b[7] as usize & 0x7f) << 14) | ((b[8] as usize & 0x7f) << 7) | (b[9] as usize & 0x7f);
        let footer = if b[5] & 0x10 != 0 { 10 } else { 0 };
        let end = (10 + size + footer).min(b.len());
        return &b[end..];
    }
    b
}

fn sniff(b: &[u8]) -> Option<&'static str> {
    let b = strip_id3(b);
    if b.len() >= 12 && &b[4..8] == b"ftyp" {
        Some("mp4")
    } else if b.starts_with(b"OggS") {
        Some("ogg")
    } else if b.starts_with(b"fLaC") {
        Some("flac")
    } else if b.starts_with(b"RIFF") {
        Some("wav")
    } else if b.len() >= 2 && b[0] == 0xFF && b[1] & 0xF6 == 0xF0 {
        Some("aac") // ADTS: sync word and layer 00
    } else if b.len() >= 2 && b[0] == 0xFF && b[1] & 0xE0 == 0xE0 {
        Some("mp3")
    } else {
        None
    }
}

/// Pulls the audio elementary stream out of MPEG-TS: (the stream, "aac" or "mp3")
pub fn ts_audio(ts: &[u8]) -> Option<(Vec<u8>, &'static str)> {
    let mut pmt_pid: Option<u16> = None;
    let mut audio: Option<(u16, &'static str)> = None;
    let mut out = Vec::new();
    for p in ts.as_chunks::<188>().0 {
        if p[0] != 0x47 {
            continue;
        }
        let pusi = p[1] & 0x40 != 0;
        let pid = (((p[1] & 0x1f) as u16) << 8) | p[2] as u16;
        let afc = (p[3] >> 4) & 3;
        if afc == 0 || afc == 2 {
            continue; // no payload
        }
        let mut at = 4;
        if afc == 3 {
            at += 1 + p[4] as usize;
        }
        if at >= 188 {
            continue;
        }
        let payload = &p[at..];
        if pid == 0 && pusi && pmt_pid.is_none() {
            let s = payload.get(1 + payload[0] as usize..)?; // past the pointer field
            let len = ((((*s.get(1)? & 0x0f) as usize) << 8) | *s.get(2)? as usize).min(s.len().saturating_sub(3));
            let end = (3 + len).saturating_sub(4); // the CRC is the last 4 bytes
            let mut i = 8;
            while i + 4 <= end {
                let program = ((s[i] as u16) << 8) | s[i + 1] as u16;
                if program != 0 {
                    pmt_pid = Some((((s[i + 2] & 0x1f) as u16) << 8) | s[i + 3] as u16);
                    break;
                }
                i += 4;
            }
        } else if Some(pid) == pmt_pid && pusi && audio.is_none() {
            let s = payload.get(1 + payload[0] as usize..)?;
            let len = ((((*s.get(1)? & 0x0f) as usize) << 8) | *s.get(2)? as usize).min(s.len().saturating_sub(3));
            let end = (3 + len).saturating_sub(4); // the CRC is the last 4 bytes
            let info_len = (((*s.get(10)? & 0x0f) as usize) << 8) | *s.get(11)? as usize;
            let mut i = 12 + info_len;
            while i + 5 <= end {
                let kind = s[i];
                let es_pid = (((s[i + 1] & 0x1f) as u16) << 8) | s[i + 2] as u16;
                let es_info = (((s[i + 3] & 0x0f) as usize) << 8) | s[i + 4] as usize;
                match kind {
                    0x0F => audio = Some((es_pid, "aac")),
                    0x03 | 0x04 => audio = Some((es_pid, "mp3")),
                    _ => {}
                }
                if audio.is_some() {
                    break;
                }
                i += 5 + es_info;
            }
        } else if let Some((apid, _)) = audio {
            if pid != apid {
                continue;
            }
            if pusi {
                // a PES header: 00 00 01, stream id, length (2), flags (2), header length, header
                if payload.len() >= 9 && payload[..3] == [0, 0, 1] {
                    let skip = 9 + payload[8] as usize;
                    if skip < payload.len() {
                        out.extend_from_slice(&payload[skip..]);
                    }
                }
            } else {
                out.extend_from_slice(payload);
            }
        }
    }
    let (_, ext) = audio?;
    (!out.is_empty()).then_some((out, ext))
}

/// For the app: measures a track ("local" or "soundcloud") and returns the Analysis as JSON
pub async fn analyse(source: &str, id: &str) -> Result<Value, String> {
    let a = match source {
        "local" => analyse_local(id).await?,
        "soundcloud" => {
            if id.is_empty() || !id.chars().all(|c| c.is_ascii_digit()) {
                return Err("Bad track id".into());
            }
            analyse_soundcloud(id).await?
        }
        "youtube" => analyse_youtube(id).await?,
        _ => return Err("only local files, SoundCloud and YouTube tracks are measured".into()),
    };
    serde_json::to_value(a).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 16-bit stereo WAV made of (seconds, amplitude) parts: a 1 kHz tone, or silence at 0
    fn wav(parts: &[(f64, f64)]) -> Vec<u8> {
        let rate = 44100u32;
        let mut pcm: Vec<i16> = Vec::new();
        let mut n = 0u64;
        for &(secs, amp) in parts {
            for _ in 0..(secs * rate as f64) as u64 {
                let v = (amp * (2.0 * std::f64::consts::PI * 1000.0 * n as f64 / rate as f64).sin() * 32767.0) as i16;
                pcm.push(v);
                pcm.push(v);
                n += 1;
            }
        }
        let data_len = (pcm.len() * 2) as u32;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data_len).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 4).to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&data_len.to_le_bytes());
        for s in pcm {
            out.extend_from_slice(&s.to_le_bytes());
        }
        out
    }

    fn m(parts: &[(f64, f64)]) -> Analysis {
        measure(Box::new(Cursor::new(wav(parts))), Some("wav"), None).unwrap()
    }

    #[test]
    fn loudness_follows_the_level() {
        let loud = m(&[(10.0, 0.5)]);
        let quiet = m(&[(10.0, 0.05)]);
        // ten times quieter is 20 dB, whatever the absolute calibration
        assert!((loud.lufs - quiet.lufs - 20.0).abs() < 0.3, "{loud:?} {quiet:?}");
        // a 1 kHz sine at half scale in both channels: about -6 dBFS peak, -6 LUFS (BS.1770 sums the channels)
        assert!((loud.peak_db + 6.0).abs() < 0.2, "{loud:?}");
        assert!((loud.lufs + 6.0).abs() < 1.0, "{loud:?}");
        assert_eq!(loud.duration_ms, 10_000);
        assert!(loud.silences.is_empty());
    }

    #[test]
    fn silence_doesnt_pull_the_loudness_down() {
        // the gating in R128 ignores silence, so padding a track with silence doesn't change it
        let plain = m(&[(8.0, 0.3)]);
        let padded = m(&[(8.0, 0.3), (30.0, 0.0)]);
        assert!((plain.lufs - padded.lufs).abs() < 0.2, "{plain:?} {padded:?}");
    }

    #[test]
    fn finds_silences_at_the_start_middle_and_end() {
        let a = m(&[(3.0, 0.0), (4.0, 0.3), (1.0, 0.0), (5.0, 0.0001), (4.0, 0.3), (12.0, 0.0)]);
        // the 1 s gap followed by near-silent noise at -80 dB counts as one 6 s silence
        assert_eq!(a.silences.len(), 3, "{a:?}");
        let near = |x: u64, y: u64| x.abs_diff(y) <= WINDOW_MS;
        assert!(near(a.silences[0][0], 0) && near(a.silences[0][1], 3000), "{a:?}");
        assert!(near(a.silences[1][0], 7000) && near(a.silences[1][1], 13000), "{a:?}");
        assert!(near(a.silences[2][0], 17000) && near(a.silences[2][1], 29000), "{a:?}");
        assert_eq!(a.duration_ms, 29_000);
    }

    #[test]
    fn short_pauses_and_quiet_music_arent_silence() {
        let a = m(&[(4.0, 0.3), (1.5, 0.0), (4.0, 0.3), (5.0, 0.003)]); // -50 dB tone: quiet, not silent
        assert!(a.silences.is_empty(), "{a:?}");
    }

    #[test]
    fn reads_hls_playlists() {
        let base = reqwest::Url::parse("https://cdn.example/a/b/playlist.m3u8?x=1").unwrap();
        let pl = parse_m3u8(
            "#EXTM3U\n#EXT-X-TARGETDURATION:10\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:9.98,\nseg1.m4s\n#EXTINF:4.5,\n/abs/seg2.m4s?t=2\n#EXT-X-ENDLIST\n",
            &base,
        )
        .unwrap();
        assert_eq!(pl.map.as_deref(), Some("https://cdn.example/a/b/init.mp4"));
        assert_eq!(pl.segments, vec![(9.98, "https://cdn.example/a/b/seg1.m4s".into()), (4.5, "https://cdn.example/abs/seg2.m4s?t=2".into())]);
        let master = parse_m3u8("#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=160000\nhi/index.m3u8\n", &base).unwrap();
        assert_eq!(master.variants, vec!["https://cdn.example/a/b/hi/index.m3u8".to_string()]);
        let enc = parse_m3u8("#EXTM3U\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:10,\ns.ts\n", &base);
        assert!(enc.unwrap_err().contains("encrypted"));
    }

    // MPEG-TS packets: PAT, PMT (one AAC stream on PID 0x101), then the audio in two PES packets
    fn ts_packet(pid: u16, pusi: bool, payload: &[u8]) -> Vec<u8> {
        let mut p = vec![0x47, ((pusi as u8) << 6) | (pid >> 8) as u8, pid as u8];
        let room = 184;
        if payload.len() < room {
            // pad with an adaptation field of stuffing bytes
            let stuff = room - payload.len() - 1;
            p.push(0x30);
            p.push(stuff as u8);
            if stuff > 0 {
                p.push(0x00);
                p.extend(std::iter::repeat_n(0xFF, stuff - 1));
            }
        } else {
            p.push(0x10);
        }
        p.extend_from_slice(payload);
        assert_eq!(p.len(), 188);
        p
    }

    #[test]
    fn pulls_audio_out_of_mpeg_ts() {
        let pat = [0x00, 0x00, 0xB0, 0x0D, 0x00, 0x01, 0xC1, 0x00, 0x00, 0x00, 0x01, 0xF0, 0x00, 0, 0, 0, 0];
        let pmt = [
            0x00, 0x02, 0xB0, 0x12, 0x00, 0x01, 0xC1, 0x00, 0x00, 0xE1, 0x01, 0xF0, 0x00, 0x0F, 0xE1, 0x01, 0xF0, 0x00, 0, 0, 0, 0,
        ];
        let audio: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
        let mut pes = vec![0, 0, 1, 0xC0, 0, 0, 0x80, 0x80, 5, 0x21, 0, 1, 0, 1];
        pes.extend_from_slice(&audio[..170]);
        let mut ts = Vec::new();
        ts.extend(ts_packet(0, true, &pat));
        ts.extend(ts_packet(0x1000, true, &pmt));
        ts.extend(ts_packet(0x1000, true, &pmt)); // repeated tables are fine
        ts.extend(ts_packet(0x101, true, &pes));
        ts.extend(ts_packet(0x200, true, &[0xAA; 50])); // another stream, ignored
        ts.extend(ts_packet(0x101, false, &audio[170..]));
        let (got, ext) = ts_audio(&ts).unwrap();
        assert_eq!(ext, "aac");
        assert_eq!(got, audio);
    }

    #[test]
    fn strips_id3_and_recognises_formats() {
        let mut b = b"ID3\x04\x00\x00\x00\x00\x00\x03abc".to_vec();
        b.extend_from_slice(&[0xFF, 0xF1, 0x50, 0x80]);
        assert_eq!(strip_id3(&b), &[0xFF, 0xF1, 0x50, 0x80]);
        assert_eq!(sniff(&b), Some("aac"));
        assert_eq!(sniff(&[0xFF, 0xFB, 0x90, 0x64]), Some("mp3"));
        assert_eq!(sniff(b"\0\0\0\x18ftypM4A \0\0\0\0"), Some("mp4"));
    }
}
