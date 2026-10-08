// Sound from a web address, read like a file: fetched in pieces with range requests, so playback starts
// after the first piece and seeking only fetches what's needed. For progressive streams (SoundCloud's
// MP3s, YouTube's M4A from yt-dlp, plugins' files, the app's own local server). Servers that don't
// take ranges are read straight through (and can't be seeked).

use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;
use symphonia::core::io::MediaSource;

const PIECE: u64 = 256 * 1024;

pub struct HttpStream {
    client: reqwest::blocking::Client,
    url: String,
    len: Option<u64>,
    ranges: bool,
    pos: u64,
    buf: Vec<u8>,
    buf_start: u64,
    /// a server without ranges: the one response, read in order
    whole: Option<reqwest::blocking::Response>,
}

impl HttpStream {
    pub fn open(url: &str) -> Result<Self, String> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        // the first piece tells us the length and whether ranges work
        let res = client
            .get(url)
            .header("Range", format!("bytes=0-{}", PIECE - 1))
            .send()
            .map_err(|e| format!("Couldn't reach the stream: {e}"))?;
        let status = res.status();
        if !status.is_success() {
            return Err(format!("The stream answered {status}."));
        }
        let ranges = status.as_u16() == 206;
        let len = if ranges {
            res.headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next())
                .and_then(|v| v.parse().ok())
        } else {
            res.content_length()
        };
        let mut s = Self { client, url: url.to_string(), len, ranges, pos: 0, buf: Vec::new(), buf_start: 0, whole: None };
        if ranges {
            s.buf = res.bytes().map_err(|e| e.to_string())?.to_vec();
        } else {
            s.whole = Some(res);
        }
        Ok(s)
    }

    fn fetch(&mut self, at: u64) -> std::io::Result<()> {
        let end = at + PIECE - 1;
        let res = self
            .client
            .get(&self.url)
            .header("Range", format!("bytes={at}-{end}"))
            .send()
            .map_err(std::io::Error::other)?;
        if !res.status().is_success() {
            return Err(std::io::Error::other(format!("the stream answered {}", res.status())));
        }
        self.buf = res.bytes().map_err(std::io::Error::other)?.to_vec();
        self.buf_start = at;
        Ok(())
    }
}

impl Read for HttpStream {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if let Some(r) = self.whole.as_mut() {
            let n = r.read(out)?;
            self.pos += n as u64;
            return Ok(n);
        }
        if self.len.is_some_and(|l| self.pos >= l) {
            return Ok(0);
        }
        let in_buf = self.pos >= self.buf_start && self.pos < self.buf_start + self.buf.len() as u64;
        if !in_buf {
            self.fetch(self.pos)?;
            if self.buf.is_empty() {
                return Ok(0);
            }
        }
        let from = (self.pos - self.buf_start) as usize;
        let n = out.len().min(self.buf.len() - from);
        out[..n].copy_from_slice(&self.buf[from..from + n]);
        self.pos += n as u64;
        Ok(n)
    }
}

impl Seek for HttpStream {
    fn seek(&mut self, to: SeekFrom) -> std::io::Result<u64> {
        let target = match to {
            SeekFrom::Start(p) => p as i64,
            SeekFrom::Current(d) => self.pos as i64 + d,
            SeekFrom::End(d) => self.len.ok_or_else(|| std::io::Error::other("length unknown"))? as i64 + d,
        };
        if target < 0 {
            return Err(std::io::Error::other("before the start"));
        }
        if self.whole.is_some() && target as u64 != self.pos {
            return Err(std::io::Error::other("this stream can't be seeked"));
        }
        self.pos = target as u64;
        Ok(self.pos)
    }
}

impl MediaSource for HttpStream {
    fn is_seekable(&self) -> bool {
        self.ranges && self.len.is_some()
    }
    fn byte_len(&self) -> Option<u64> {
        self.len
    }
}

/// A web address's sound, ready to play; `ext` (like "mp3" or "m4a") helps find its format
pub fn open_url(url: &str, ext: &str) -> Result<crate::Decoded, String> {
    crate::Decoded::open(Box::new(HttpStream::open(url)?), ext)
}
