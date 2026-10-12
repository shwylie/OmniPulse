// HLS: a playlist of short segments (SoundCloud's full-length streams). The segments are fetched in
// order and read as one stream: MPEG-TS segments are unwrapped to their AAC or MP3 (see ts.rs),
// fragmented-MP4 ones are read after their init segment. A seek starts again at the segment holding
// that time and skips into it. Encrypted playlists aren't played (as in the web app).

use crate::source::{Decoded, Source};
use crate::ts::Demux;
use std::io::Read;
use std::time::Duration;
use symphonia::core::io::ReadOnlySource;

#[derive(Clone, Debug)]
pub struct Segment {
    pub url: String,
    pub ms: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Ts,
    Fmp4,
    /// anything else: the segments as they are, one after another
    Raw,
}

#[derive(Clone, Debug)]
pub struct Playlist {
    pub segments: Vec<Segment>,
    pub init: Option<String>,
    pub encrypted: bool,
}

fn join(base: &str, rel: &str) -> String {
    reqwest::Url::parse(base).and_then(|b| b.join(rel)).map(|u| u.to_string()).unwrap_or_else(|_| rel.to_string())
}

/// A media playlist's segments (the first variant of a master playlist is followed by `open`)
pub fn parse(base: &str, text: &str) -> Playlist {
    let mut segments = Vec::new();
    let (mut init, mut encrypted, mut dur) = (None, false, 0u64);
    for line in text.lines().map(str::trim) {
        if let Some(d) = line.strip_prefix("#EXTINF:") {
            dur = (d.split(',').next().unwrap_or("0").trim().parse::<f64>().unwrap_or(0.0) * 1000.0).round() as u64;
        } else if let Some(m) = line.strip_prefix("#EXT-X-MAP:") {
            if let Some(uri) = m.split("URI=\"").nth(1).and_then(|r| r.split('"').next()) {
                init = Some(join(base, uri));
            }
        } else if let Some(k) = line.strip_prefix("#EXT-X-KEY:") {
            if !k.contains("METHOD=NONE") {
                encrypted = true;
            }
        } else if !line.is_empty() && !line.starts_with('#') {
            segments.push(Segment { url: join(base, line), ms: dur });
            dur = 0;
        }
    }
    Playlist { segments, init, encrypted }
}

/// Segments fetched ahead of the one being read (SoundCloud's are about ten seconds each): a slow one,
/// or the next song fetched alongside, never makes the sound wait
const SEGS_AHEAD: usize = 8;

/// The segments from one of them on, read as one stream (fetched in order by a fetcher of its own,
/// a few ahead; it stops when this is let go, a seek making another)
struct SegmentReader {
    got: std::sync::Mutex<std::sync::mpsc::Receiver<Result<Vec<u8>, String>>>,
    kind: Kind,
    demux: Demux,
    buf: Vec<u8>,
    pos: usize,
}

impl SegmentReader {
    fn new(client: &reqwest::blocking::Client, segs: &[Segment], from: usize, kind: Kind, init: Vec<u8>) -> Self {
        let (tx, got) = std::sync::mpsc::sync_channel(SEGS_AHEAD);
        let (client, urls): (_, Vec<String>) = (client.clone(), segs.iter().skip(from).map(|s| s.url.clone()).collect());
        let _ = std::thread::Builder::new().name("segments ahead".into()).spawn(move || {
            for url in urls {
                let r = client.get(&url).send().map_err(|e| e.to_string()).and_then(|res| {
                    if !res.status().is_success() { return Err(format!("a segment answered {}", res.status())); }
                    res.bytes().map(|b| b.to_vec()).map_err(|e| e.to_string())
                });
                let failed = r.is_err();
                // (read no more: let go, or a seek)
                if tx.send(r).is_err() || failed { return; }
            }
        });
        Self { got: std::sync::Mutex::new(got), kind, demux: Demux::default(), buf: init, pos: 0 }
    }
}

impl Read for SegmentReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        while self.pos >= self.buf.len() {
            // (all of them read: the fetcher's done, and gone)
            let Ok(next) = self.got.get_mut().unwrap().recv() else { return Ok(0) };
            let data = next.map_err(std::io::Error::other)?;
            self.buf.clear();
            self.pos = 0;
            match self.kind {
                Kind::Ts => self.demux.segment(&data, &mut self.buf).map_err(std::io::Error::other)?,
                _ => self.buf = data,
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

pub struct HlsSource {
    client: reqwest::blocking::Client,
    list: Playlist,
    kind: Kind,
    ext: &'static str,
    init_bytes: Vec<u8>,
    inner: Decoded,
    rate: u32,
    total_ms: u64,
}

impl HlsSource {
    pub fn open(url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder().timeout(Duration::from_secs(30)).build().map_err(|e| e.to_string())?;
        let get = |u: &str| -> Result<String, String> {
            let r = client.get(u).send().map_err(|e| format!("Couldn't reach the stream: {e}"))?;
            if !r.status().is_success() {
                return Err(format!("The stream answered {}.", r.status()));
            }
            r.text().map_err(|e| e.to_string())
        };
        let mut base = url.to_string();
        let mut text = get(url)?;
        // a master playlist: its first variant
        if text.contains("#EXT-X-STREAM-INF") {
            let variant = text.lines().skip_while(|l| !l.starts_with("#EXT-X-STREAM-INF")).nth(1).ok_or("The playlist has no streams.")?.trim().to_string();
            base = join(url, &variant);
            text = get(&base)?;
        }
        let list = parse(&base, &text);
        if list.encrypted {
            return Err("This stream is encrypted, so it can't be played here.".into());
        }
        if list.segments.is_empty() {
            return Err("The playlist has no segments.".into());
        }
        let total_ms = list.segments.iter().map(|s| s.ms).sum();
        // what the segments are, from the first one
        let first = client.get(&list.segments[0].url).send().and_then(|r| r.bytes()).map_err(|e| e.to_string())?;
        let init_bytes = match &list.init {
            Some(u) => client.get(u).send().and_then(|r| r.bytes()).map_err(|e| e.to_string())?.to_vec(),
            None => Vec::new(),
        };
        let looks_mp4 = |b: &[u8]| b.len() > 8 && matches!(&b[4..8], b"ftyp" | b"styp" | b"moof" | b"sidx");
        let (kind, ext) = if first.first() == Some(&0x47) {
            let mut d = Demux::default();
            let mut sink = Vec::new();
            d.segment(&first, &mut sink)?;
            (Kind::Ts, d.codec.map(|c| c.ext()).unwrap_or("aac"))
        } else if list.init.is_some() || looks_mp4(&first) {
            (Kind::Fmp4, "mp4")
        } else {
            let e = list.segments[0].url.split('?').next().unwrap_or("").rsplit('.').next().unwrap_or("");
            (Kind::Raw, match e { "mp3" => "mp3", "aac" => "aac", "ogg" | "opus" => "ogg", _ => "" })
        };
        let inner = Self::reader_from(&client, &list, kind, ext, &init_bytes, 0)?;
        let rate = inner.rate();
        Ok(Self { client, list, kind, ext, init_bytes, inner, rate, total_ms })
    }

    fn reader_from(client: &reqwest::blocking::Client, list: &Playlist, kind: Kind, ext: &str, init: &[u8], from: usize) -> Result<Decoded, String> {
        let r = SegmentReader::new(client, &list.segments, from, kind, init.to_vec());
        Decoded::open(Box::new(ReadOnlySource::new(r)), ext)
    }
}

impl Source for HlsSource {
    fn rate(&self) -> u32 {
        self.rate
    }
    fn duration_ms(&self) -> Option<u64> {
        Some(self.total_ms)
    }
    fn read(&mut self, out: &mut Vec<f32>) -> Result<bool, String> {
        self.inner.read(out)
    }
    fn seek(&mut self, ms: u64) -> Result<u64, String> {
        // the segment holding that time, then decode up to it
        let (mut k, mut at) = (0usize, 0u64);
        while k + 1 < self.list.segments.len() && at + self.list.segments[k].ms <= ms {
            at += self.list.segments[k].ms;
            k += 1;
        }
        self.inner = Self::reader_from(&self.client, &self.list, self.kind, self.ext, &self.init_bytes, k)?;
        let mut skip = ((ms - at) * self.rate as u64 / 1000) as usize * 2;
        let mut scratch = Vec::new();
        while skip > 0 {
            scratch.clear();
            if !self.inner.read(&mut scratch)? {
                break;
            }
            if scratch.len() > skip {
                // the rest of this piece is kept for the next read
                self.inner.carry(scratch.split_off(skip));
                break;
            }
            skip -= scratch.len();
        }
        Ok(ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_playlists() {
        let p = parse("https://cdn.example.com/a/b/playlist.m3u8", "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:10\n#EXTINF:9.98,\nseg0.ts\n#EXTINF:10.0,\nhttps://other.example.com/seg1.ts?x=1\n#EXT-X-ENDLIST\n");
        assert_eq!(p.segments.len(), 2);
        assert_eq!(p.segments[0].url, "https://cdn.example.com/a/b/seg0.ts");
        assert_eq!(p.segments[0].ms, 9980);
        assert_eq!(p.segments[1].url, "https://other.example.com/seg1.ts?x=1");
        assert!(!p.encrypted && p.init.is_none());
        let e = parse("https://x/y.m3u8", "#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXT-X-MAP:URI=\"init.mp4\"\n#EXTINF:5,\ns.m4s\n");
        assert!(e.encrypted);
        assert_eq!(e.init.as_deref(), Some("https://x/init.mp4"));
    }
}
