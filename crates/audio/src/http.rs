// Sound from a web address, read like a file: fetched in pieces with range requests, so playback starts
// after the first piece and seeking only fetches what's needed. For progressive streams (SoundCloud's
// MP3s, YouTube's M4A from yt-dlp, plugins' files, the app's own local server). Servers that don't
// take ranges are read straight through (and can't be seeked).

use std::io::{Read, Seek, SeekFrom};
use std::time::Duration;
use symphonia::core::io::MediaSource;

const PIECE: u64 = 256 * 1024;

/// A fresh address for the same sound, for when the one in use is turned down partway (YouTube's
/// addresses can stop being accepted while a song plays)
pub type Refresh = Box<dyn FnMut() -> Result<String, String> + Send + Sync>;

/// Pieces fetched ahead of where it's read (about 8 MB: minutes of most streams), so the sound never
/// waits on the network while there's any: a slow piece, or the next song being fetched alongside
/// (to measure it), is ridden out
const AHEAD: u64 = 32;

/// What's fetched, by piece (PIECE bytes from piece * PIECE), shared with the fetcher that runs ahead
struct Pieces {
    have: std::collections::HashMap<u64, Vec<u8>>,
    /// the one being fetched ahead right now
    busy: Option<u64>,
    /// the piece being read (the fetcher keeps AHEAD pieces past it)
    want: u64,
    url: String,
    refreshed: u32,
    /// the fetcher gave up (reading fetches what it needs itself, as it did before)
    failed: bool,
    stop: bool,
}

type Ahead = std::sync::Arc<(std::sync::Mutex<Pieces>, std::sync::Condvar)>;

pub struct HttpStream {
    client: reqwest::blocking::Client,
    len: Option<u64>,
    ranges: bool,
    pos: u64,
    /// a server without ranges: the one response, read in order
    whole: Option<reqwest::blocking::Response>,
    ahead: Ahead,
    refresh: std::sync::Arc<std::sync::Mutex<Option<Refresh>>>,
}

/// One piece, fetched (a fresh address for it, if the one in use is turned down: a few times at most)
fn fetch_piece(client: &reqwest::blocking::Client, ahead: &Ahead, refresh: &std::sync::Mutex<Option<Refresh>>, k: u64, len: Option<u64>) -> std::io::Result<Vec<u8>> {
    let at = k * PIECE;
    let end = (at + PIECE - 1).min(len.unwrap_or(u64::MAX).saturating_sub(1));
    let url = ahead.0.lock().unwrap().url.clone();
    let mut res = client.get(&url).header("Range", format!("bytes={at}-{end}")).send().map_err(std::io::Error::other)?;
    // turned down partway: a fresh address for the same sound, and on from where it was
    if matches!(res.status().as_u16(), 403 | 410) {
        let can = ahead.0.lock().unwrap().refreshed < 3;
        if can {
            if let Some(f) = refresh.lock().unwrap().as_mut() {
                let fresh = f().map_err(std::io::Error::other)?;
                { let mut p = ahead.0.lock().unwrap(); p.url = fresh.clone(); p.refreshed += 1; }
                res = client.get(&fresh).header("Range", format!("bytes={at}-{end}")).send().map_err(std::io::Error::other)?;
            }
        }
    }
    if !res.status().is_success() {
        return Err(std::io::Error::other(format!("the stream answered {}", res.status())));
    }
    Ok(res.bytes().map_err(std::io::Error::other)?.to_vec())
}

impl HttpStream {
    pub fn open(url: &str) -> Result<Self, String> {
        Self::open_with(url, None)
    }

    pub fn open_with(url: &str, mut refresh: Option<Refresh>) -> Result<Self, String> {
        let mut url = url.to_string();
        let mut refreshed = 0;
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|e| e.to_string())?;
        // the first piece tells us the length and whether ranges work
        let mut res = client
            .get(&url)
            .header("Range", format!("bytes=0-{}", PIECE - 1))
            .send()
            .map_err(|e| format!("Couldn't reach the stream: {e}"))?;
        // (turned down: a fresh address, once)
        if matches!(res.status().as_u16(), 403 | 410) {
            if let Some(f) = refresh.as_mut() {
                url = f()?;
                refreshed += 1;
                res = client.get(&url).header("Range", format!("bytes=0-{}", PIECE - 1)).send().map_err(|e| format!("Couldn't reach the stream: {e}"))?;
            }
        }
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
        let ahead: Ahead = std::sync::Arc::new((std::sync::Mutex::new(Pieces { have: Default::default(), busy: None, want: 0, url, refreshed, failed: false, stop: false }), std::sync::Condvar::new()));
        let refresh = std::sync::Arc::new(std::sync::Mutex::new(refresh));
        let mut s = Self { client, len, ranges, pos: 0, whole: None, ahead, refresh };
        if ranges {
            let first = res.bytes().map_err(|e| e.to_string())?.to_vec();
            s.ahead.0.lock().unwrap().have.insert(0, first);
            // the fetcher: the pieces after the one being read, in order, kept AHEAD in front
            if let Some(len) = len {
                let last = len.saturating_sub(1) / PIECE;
                let (client, ahead, refresh) = (s.client.clone(), s.ahead.clone(), s.refresh.clone());
                std::thread::Builder::new().name("stream ahead".into()).spawn(move || loop {
                    let k = {
                        let (m, cv) = &*ahead;
                        let mut p = m.lock().unwrap();
                        loop {
                            if p.stop || p.failed { return; }
                            let to = (p.want + AHEAD).min(last);
                            if let Some(k) = (p.want..=to).find(|k| !p.have.contains_key(k)) { p.busy = Some(k); break k; }
                            p = cv.wait_timeout(p, Duration::from_millis(500)).unwrap().0;
                        }
                    };
                    let got = fetch_piece(&client, &ahead, &refresh, k, Some(len));
                    let (m, cv) = &*ahead;
                    let mut p = m.lock().unwrap();
                    p.busy = None;
                    match got {
                        Ok(b) => {
                            p.have.insert(k, b);
                            // (only what's near: a piece or two behind, and what's ahead)
                            let want = p.want;
                            p.have.retain(|&i, _| i + 2 >= want && i <= want + AHEAD + 2);
                        }
                        Err(_) => p.failed = true,
                    }
                    cv.notify_all();
                }).map_err(|e| e.to_string())?;
            }
        } else {
            s.whole = Some(res);
        }
        Ok(s)
    }

    /// The piece `k`: fetched ahead already, coming (waited for), or fetched now
    fn piece(&self, k: u64) -> std::io::Result<Vec<u8>> {
        {
            let (m, cv) = &*self.ahead;
            let mut p = m.lock().unwrap();
            p.want = k;
            cv.notify_all();
            loop {
                if let Some(b) = p.have.get(&k) { return Ok(b.clone()); }
                if p.busy != Some(k) { break; }
                let (q, t) = cv.wait_timeout(p, Duration::from_secs(30)).unwrap();
                p = q;
                if t.timed_out() { break; }
            }
        }
        let b = fetch_piece(&self.client, &self.ahead, &self.refresh, k, self.len)?;
        self.ahead.0.lock().unwrap().have.insert(k, b.clone());
        Ok(b)
    }
}

impl Drop for HttpStream {
    fn drop(&mut self) {
        let (m, cv) = &*self.ahead;
        m.lock().unwrap().stop = true;
        cv.notify_all();
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
        let k = self.pos / PIECE;
        let b = self.piece(k)?;
        let from = (self.pos - k * PIECE) as usize;
        if from >= b.len() {
            return Ok(0);
        }
        let n = out.len().min(b.len() - from);
        out[..n].copy_from_slice(&b[from..from + n]);
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

/// The same, with a way to a fresh address if this one's turned down partway
pub fn open_url_refreshing(url: &str, ext: &str, refresh: Refresh) -> Result<crate::Decoded, String> {
    crate::Decoded::open(Box::new(HttpStream::open_with(url, Some(refresh))?), ext)
}
