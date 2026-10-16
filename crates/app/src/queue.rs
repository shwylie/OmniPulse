// The queue: what's playing, Next Up (yours) and then Autoplay's picks, which are kept apart so
// anything you add goes before them. Shuffling that doesn't keep serving the same songs, autoplay
// taking turns between platforms, and the saved session (as in js/queue, shuffle, autoplay, session).

use crate::track::Track;
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug)]
pub struct QItem {
    pub t: Track,
    /// added by autoplay (not by you)
    pub auto: bool,
    /// who asked for it in chat ("" for you)
    pub by: String,
    /// put there by you (Add to Queue, Play Next), not the list it's playing from: what you add next
    /// goes after these, ahead of the rest of the list (the web app's queuedByYou)
    pub queued: bool,
}

impl QItem {
    pub fn mine(t: Track) -> QItem {
        QItem { t, auto: false, by: String::new(), queued: false }
    }
    /// added by you (Add to Queue, Play Next)
    pub fn queued(t: Track) -> QItem {
        QItem { t, auto: false, by: String::new(), queued: true }
    }
    pub fn auto(t: Track) -> QItem {
        QItem { t, auto: true, by: String::new(), queued: false }
    }
}

#[derive(Default)]
pub struct Queue {
    pub items: Vec<QItem>,
    pub qi: Option<usize>,
    /// the list it was started from (a playlist's, a page's name), for its heading in the queue
    pub name: String,
}

impl Queue {
    pub fn current(&self) -> Option<&Track> {
        self.qi.and_then(|i| self.items.get(i)).map(|q| &q.t)
    }

    /// Where "Add to Queue" goes: after everything you queued, before autoplay's picks
    pub fn autoplay_start(&self) -> usize {
        let from = self.qi.map_or(0, |i| i + 1);
        (from..self.items.len()).find(|&i| self.items[i].auto).unwrap_or(self.items.len())
    }

    /// A list to play from `start` (a playlist, the Library...): the queue becomes it
    pub fn set_list(&mut self, list: Vec<Track>, start: usize) {
        self.items = list.into_iter().skip(start).take(1000).map(QItem::mine).collect();
        self.qi = if self.items.is_empty() { None } else { Some(0) };
    }

    /// One track (from search, or a shelf): it takes the current one's place, the rest stays
    pub fn replace_current(&mut self, t: Track) {
        match self.qi {
            Some(i) if i < self.items.len() => self.items[i] = QItem::mine(t),
            _ => {
                self.items.push(QItem::mine(t));
                self.qi = Some(self.items.len() - 1);
            }
        }
    }

    /// The same song further on in the list that's playing (or among autoplay's picks): taken out, as
    /// it's queued now (what you queued, and chat requests, stay where they are)
    fn take_from_list(&mut self, key: &str) {
        let from = self.qi.map_or(0, |i| i + 1);
        let mut i = from;
        while i < self.items.len() {
            let q = &self.items[i];
            if q.t.key == key && !q.queued && q.by.is_empty() { self.items.remove(i); } else { i += 1; }
        }
    }

    pub fn play_next(&mut self, t: Track) {
        self.take_from_list(&t.key);
        let at = self.qi.map_or(0, |i| i + 1).min(self.items.len());
        self.items.insert(at, QItem::queued(t));
        if self.qi.is_none() {
            self.qi = Some(0);
        }
    }

    /// Where Add to Queue puts a track (as the web app's queuedEnd): after what's playing and anything
    /// else you've queued, ahead of the rest of the list that's playing (and of autoplay's picks)
    pub fn queued_end(&self) -> usize {
        let mut i = self.qi.map_or(0, |q| q + 1).min(self.items.len());
        while i < self.items.len() && (self.items[i].queued || !self.items[i].by.is_empty()) { i += 1; }
        i
    }

    pub fn add(&mut self, t: Track) {
        self.take_from_list(&t.key);
        let at = self.queued_end();
        self.items.insert(at, QItem::queued(t));
        if self.qi.is_none() {
            self.qi = Some(0);
        }
    }

    /// A chat request: queued like Add to Queue (after what you've queued and the other requests, ahead of
    /// the rest of the list; coming up further on in it: moved up), with who asked; its place in the queue
    /// (0: playing)
    pub fn add_request(&mut self, t: Track, by: &str) -> usize {
        self.take_from_list(&t.key);
        let at = self.queued_end();
        self.items.insert(at, QItem { t, auto: false, by: by.to_string(), queued: false });
        if self.qi.is_none() {
            self.qi = Some(0);
        }
        at - self.qi.unwrap_or(0)
    }

    /// The queue's Clear: only what's playing kept
    pub fn clear(&mut self) {
        match self.qi.and_then(|i| (i < self.items.len()).then(|| self.items.remove(i))) {
            Some(q) => { self.items = vec![q]; self.qi = Some(0); }
            None => { self.items.clear(); self.qi = None; }
        }
    }

    /// Clear requests: the chat requests still to play taken out
    pub fn clear_requests(&mut self) -> usize {
        let from = self.qi.map_or(0, |i| i + 1);
        let before = self.items.len();
        let mut k = 0;
        self.items.retain(|q| { let keep = k < from || q.by.is_empty(); k += 1; keep });
        before - self.items.len()
    }

    /// Chat requests still to play
    pub fn pending_requests(&self) -> impl Iterator<Item = &QItem> {
        self.items.iter().skip(self.qi.map_or(0, |i| i + 1)).filter(|q| !q.by.is_empty())
    }

    /// Moves a queued track to play next
    pub fn move_next(&mut self, i: usize) {
        let Some(cur) = self.qi else { return };
        if i <= cur || i >= self.items.len() {
            return;
        }
        let mut q = self.items.remove(i);
        q.auto = false;
        self.items.insert(cur + 1, q);
    }

    pub fn remove(&mut self, i: usize) {
        if i >= self.items.len() || Some(i) == self.qi {
            return;
        }
        self.items.remove(i);
        if let Some(cur) = self.qi {
            if i < cur {
                self.qi = Some(cur - 1);
            }
        }
    }

    /// Autoplay switched off: its unplayed picks go
    pub fn drop_autoplay(&mut self) {
        let cur = self.qi.unwrap_or(0);
        let mut k = 0;
        self.items.retain(|q| {
            let keep = k <= cur || !q.auto;
            k += 1;
            keep
        });
    }

    /// What's still to come, shuffled fairly (the current track stays where it is)
    /// recent: (the Avoid Recent Plays time frame, now), for a queue from your Library or a playlist,
    /// whose recent plays are spaced out (space_recent)
    pub fn shuffle_rest(&mut self, last_played: &HashMap<String, u64>, recent: Option<(u64, u64)>) {
        let from = self.qi.map_or(0, |i| i + 1);
        let start = self.autoplay_start();
        if start <= from + 1 {
            return;
        }
        // (only the rest of the list: what you queued and chat requests stay where they are, ahead of it)
        let at: Vec<usize> = (from..start).filter(|&i| !self.items[i].queued && self.items[i].by.is_empty()).collect();
        if at.len() < 2 { return; }
        let mine: Vec<Track> = at.iter().map(|&i| self.items[i].t.clone()).collect();
        let mut shuffled = fair_shuffle(mine, last_played);
        if let Some((window, now)) = recent {
            shuffled = space_recent(shuffled, last_played, window, now);
        }
        for (&i, t) in at.iter().zip(shuffled) {
            self.items[i] = QItem::mine(t);
        }
    }

    /// Up next, for the queue panel: (index in the queue, the track, the heading above it if any). As
    /// the web app's: what you queued ("Next in Queue"), the rest of the list ("Next From <its name>"),
    /// then autoplay's picks, each run with its own heading
    pub fn panel(&self) -> Vec<(usize, &QItem, String)> {
        let Some(cur) = self.qi else { return Vec::new() };
        let mut out = Vec::new();
        let mut last = String::new();
        let from = format!("Next From {}", if self.name.is_empty() { "Your List" } else { &self.name });
        for (i, q) in self.items.iter().enumerate().skip(cur) {
            let head = if i == cur { "Now Playing".to_string() } else if q.auto { "Autoplay".to_string() } else if q.queued || !q.by.is_empty() { "Next in Queue".to_string() } else { from.clone() };
            out.push((i, q, if head != last { head.clone() } else { String::new() }));
            last = head;
        }
        out
    }

    // ---------- the session: saved as you listen, restored (paused) at start ----------
    pub fn to_session(&self, pos_ms: u64) -> Value {
        // (1,000 kept: the ones around what's playing, a hundred behind it, so it's always among them;
        // the first 1,000 of a big list left it out once you were past them, and nothing came back)
        let from = self.qi.unwrap_or(0).saturating_sub(100);
        json!({
            "qi": self.qi.map(|q| q - from),
            "posMs": pos_ms,
            "savedAt": omnipulse_core::now_secs() * 1000,
            "listName": self.name,
            "queue": self.items.iter().skip(from).take(1000).map(|q| {
                let mut s = q.t.to_snapshot();
                if q.auto { s["autoplay"] = json!(true); }
                if !q.by.is_empty() { s["requestedBy"] = json!(q.by); }
                if q.queued { s["queuedByYou"] = json!(true); }
                s
            }).collect::<Vec<_>>(),
        })
    }

    pub fn from_session(v: &Value) -> Option<(Queue, u64)> {
        let saved_qi = v["qi"].as_u64()? as usize;
        // (one that can't be read back is left out, and what was playing kept in its place: those
        // before it no longer shift it onto another song, or past the end)
        let mut qi = saved_qi;
        let mut items = Vec::new();
        for (i, s) in v["queue"].as_array()?.iter().enumerate() {
            match Track::from_snapshot(s) {
                Some(t) => items.push(QItem { t, auto: s["autoplay"].as_bool().unwrap_or(false), by: s["requestedBy"].as_str().unwrap_or("").to_string(), queued: s["queuedByYou"].as_bool().unwrap_or(false) }),
                None if i < saved_qi => qi -= 1,
                None if i == saved_qi => return None,
                None => {}
            }
        }
        if qi >= items.len() {
            return None;
        }
        Some((Queue { items, qi: Some(qi), name: v["listName"].as_str().unwrap_or("").to_string() }, v["posMs"].as_u64().unwrap_or(0)))
    }
}

fn session_file() -> Option<std::path::PathBuf> {
    let d = omnipulse_core::data_dir().ok()?.join("state");
    std::fs::create_dir_all(&d).ok()?;
    Some(d.join("native-session.json"))
}

pub fn save_session(v: &Value) {
    if let Some(p) = session_file() {
        let tmp = p.with_extension("tmp");
        if std::fs::write(&tmp, v.to_string()).is_ok() {
            let _ = std::fs::rename(tmp, p);
        }
    }
}

pub fn load_session() -> Option<(Queue, u64)> {
    let text = std::fs::read_to_string(session_file()?).ok()?;
    Queue::from_session(&serde_json::from_str(&text).ok()?)
}

// ---------- shuffling ----------
pub fn shuffle_in_place<T>(v: &mut [T]) {
    // a small xorshift, seeded by the clock: no dependency for something this simple
    let mut x = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos() as u64).unwrap_or(7) | 1;
    for i in (1..v.len()).rev() {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.swap(i, (x % (i as u64 + 1)) as usize);
    }
}

/// Tracks never heard come first, then the rest from longest ago to most recent, in four groups
/// shuffled within themselves, so a song just heard goes to the back
pub fn fair_shuffle(list: Vec<Track>, last_played: &HashMap<String, u64>) -> Vec<Track> {
    let (mut never, mut heard): (Vec<Track>, Vec<Track>) = list.into_iter().partition(|t| !last_played.contains_key(&t.key));
    heard.sort_by_key(|t| last_played.get(&t.key).copied().unwrap_or(0));
    shuffle_in_place(&mut never);
    let size = heard.len().div_ceil(4).max(1);
    for g in heard.chunks_mut(size) {
        shuffle_in_place(g);
    }
    never.extend(heard);
    never
}

/// Takes turns between platforms: A, B, A, B... each one's own order kept; once only one has tracks
/// left the list stops there, so the same platform never plays twice in a row, unless it's the only
/// one in the list (the web app's alternatePlatforms)
pub fn alternate_platforms(list: Vec<Track>) -> Vec<Track> {
    let mut by: Vec<(&'static str, std::collections::VecDeque<Track>)> = Vec::new();
    for t in list {
        let p = t.plat();
        match by.iter_mut().find(|x| x.0 == p) { Some(x) => x.1.push_back(t), None => by.push((p, std::collections::VecDeque::from([t]))) }
    }
    if by.len() < 2 { return by.into_iter().flat_map(|x| x.1).collect(); }
    let rounds = by.iter().map(|x| x.1.len()).min().unwrap_or(0);
    let mut out = Vec::new();
    for _ in 0..rounds { for x in by.iter_mut() { out.extend(x.1.pop_front()); } }
    out
}

/// Shuffling your Library or a playlist (as the web app's spaceRecent): what you've heard within the
/// time frame comes up by how much of the list it is: 10% of it, once every 10 songs; 50%, once every
/// 2. The rest play in between, in the order given.
pub fn space_recent(list: Vec<Track>, last_played: &HashMap<String, u64>, window_ms: u64, now_ms: u64) -> Vec<Track> {
    let len = list.len();
    let (mut recent, mut fresh): (std::collections::VecDeque<Track>, std::collections::VecDeque<Track>) =
        list.into_iter().partition(|t| last_played.get(&t.key).is_some_and(|&ts| now_ms.saturating_sub(ts) < window_ms));
    if recent.is_empty() || fresh.is_empty() {
        return recent.into_iter().chain(fresh).collect();
    }
    let every = ((len as f64 / recent.len() as f64).round() as usize).max(1);
    let mut out = Vec::with_capacity(len);
    let mut k = 1;
    while !recent.is_empty() || !fresh.is_empty() {
        let take_recent = (k % every == 0 && !recent.is_empty()) || fresh.is_empty();
        out.extend(if take_recent { recent.pop_front() } else { fresh.pop_front() });
        k += 1;
    }
    out
}

// ---------- autoplay ----------
pub const AUTOPLAY_BATCH: usize = 10;

/// Takes turns between the platforms' lists (each list's own order kept), starting with one other
/// than `last_plat`. A platform isn't used twice in a row unless it's the only one with anything to
/// add; at most two per artist; nothing already `taken`; only what `ok` allows.
pub fn take_turns(mut lists: Vec<(&'static str, Vec<Track>)>, last_plat: &str, taken: &HashSet<String>, ok: impl Fn(&Track) -> bool) -> Vec<Track> {
    lists.retain(|(_, l)| !l.is_empty());
    let several = lists.len() > 1;
    if several && lists[0].0 == last_plat {
        let first = lists.remove(0);
        lists.push(first);
    }
    let mut taken = taken.clone();
    let mut per_artist: HashMap<String, usize> = HashMap::new();
    let mut lists: Vec<(&str, std::collections::VecDeque<Track>)> = lists.into_iter().map(|(p, l)| (p, l.into())).collect();
    let mut out = Vec::new();
    while out.len() < AUTOPLAY_BATCH && !lists.is_empty() {
        let mut k = 0;
        while k < lists.len() && out.len() < AUTOPLAY_BATCH {
            let pick = loop {
                match lists[k].1.pop_front() {
                    None => break None,
                    Some(t) => {
                        let a = t.artist.to_lowercase();
                        if !taken.contains(&t.key) && per_artist.get(&a).copied().unwrap_or(0) < 2 && ok(&t) {
                            break Some(t);
                        }
                    }
                }
            };
            match pick {
                None => {
                    lists.remove(k);
                    // one platform left: it would play twice in a row
                    if several && lists.len() == 1 {
                        return out;
                    }
                }
                Some(t) => {
                    taken.insert(t.key.clone());
                    *per_artist.entry(t.artist.to_lowercase()).or_insert(0) += 1;
                    out.push(t);
                    k += 1;
                }
            }
        }
        // once only one platform has tracks left, stop rather than play it twice in a row
        if several && lists.len() == 1 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::track::Src;

    #[test]
    fn a_request_goes_ahead_of_the_list() {
        let t = |k: &str| Track { key: k.into(), title: k.into(), ..Default::default() };
        let mut q = Queue::default();
        q.set_list((0..10).map(|i| t(&format!("l{i}"))).collect(), 0);
        q.add(t("mine"));
        assert_eq!(q.add_request(t("req1"), "viewer"), 2);
        assert_eq!(q.add_request(t("l7"), "other"), 3);
        let keys: Vec<&str> = q.items.iter().map(|x| x.t.key.as_str()).collect();
        assert_eq!(&keys[..5], &["l0", "mine", "req1", "l7", "l1"]);
        assert_eq!(keys.iter().filter(|k| **k == "l7").count(), 1);
        // (and what you queue after goes after the requests)
        q.add(t("mine2"));
        assert_eq!(q.items[4].t.key, "mine2");
    }

    #[test]
    fn a_shuffle_leaves_what_you_queued_ahead_of_the_list() {
        let t = |k: &str| Track { key: k.into(), title: k.into(), ..Default::default() };
        let mut q = Queue::default();
        q.set_list((0..20).map(|i| t(&format!("l{i}"))).collect(), 0);
        q.add(t("mine1"));
        q.add(t("mine2"));
        q.shuffle_rest(&HashMap::new(), None);
        let keys: Vec<&str> = q.items.iter().map(|x| x.t.key.as_str()).collect();
        assert_eq!(&keys[..3], &["l0", "mine1", "mine2"]);
        assert!(q.items[1].queued && q.items[2].queued && !q.items[3].queued);
        assert_eq!(keys.len(), 22);
    }

    #[test]
    fn queuing_a_song_takes_it_out_of_the_list() {
        let mut q = Queue::default();
        q.set_list(vec![t("a", "x", true), t("b", "x", true), t("c", "x", true), t("d", "x", true)], 0);
        q.add(t("c", "x", true));
        let keys: Vec<&str> = q.items.iter().map(|i| i.t.key.as_str()).collect();
        assert_eq!(keys, ["a", "c", "b", "d"]);
        q.play_next(t("d", "x", true));
        let keys: Vec<&str> = q.items.iter().map(|i| i.t.key.as_str()).collect();
        assert_eq!(keys, ["a", "d", "c", "b"]);
    }

    #[test]
    fn platforms_take_turns() {
        let list = vec![t("a1", "x", true), t("a2", "x", true), t("a3", "x", true), t("b1", "y", false), t("b2", "y", false)];
        let keys: Vec<String> = alternate_platforms(list).into_iter().map(|x| x.key).collect();
        assert_eq!(keys, ["a1", "b1", "a2", "b2"]);
    }

    fn t(key: &str, artist: &str, sc: bool) -> Track {
        Track { src: if sc { Src::SoundCloud(1) } else { Src::YouTube(key.into()) }, key: key.into(), title: key.into(), artist: artist.into(), album: String::new(), ms: 200_000, art: String::new(), genre: String::new(), link: String::new(), go_plus: false, ..Default::default() }
    }

    #[test]
    fn a_long_queue_comes_back_where_it_was() {
        // 1,500 tracks, playing the 1,201st: it's kept (it was cut off before, past the first 1,000)
        let mut q = Queue::default();
        q.items = (0..1500).map(|i| QItem::mine(t(&format!("{}", 1000 + i), "x", true))).collect();
        q.qi = Some(1200);
        let (back, _) = Queue::from_session(&q.to_session(5000)).expect("restored");
        assert_eq!(back.current().map(|t| t.key.as_str()), Some("2200"));
        assert_eq!(back.items.len(), 400);
    }

    #[test]
    fn your_tracks_go_before_autoplay() {
        let mut q = Queue::default();
        q.set_list(vec![t("a", "x", true), t("b", "x", true)], 0);
        q.items.push(QItem::auto(t("auto1", "y", true)));
        // (as the web app's: after what's playing and what you've queued, ahead of the rest of the list)
        q.add(t("mine", "z", true));
        let keys: Vec<&str> = q.items.iter().map(|i| i.t.key.as_str()).collect();
        assert_eq!(keys, ["a", "mine", "b", "auto1"]);
        q.play_next(t("next", "z", true));
        assert_eq!(q.items[1].t.key, "next");
        q.add(t("mine2", "z", true));
        let keys: Vec<&str> = q.items.iter().map(|i| i.t.key.as_str()).collect();
        assert_eq!(keys, ["a", "next", "mine", "mine2", "b", "auto1"]);
        q.items.remove(3);
        let heads: Vec<String> = q.panel().into_iter().map(|p| p.2).collect();
        assert_eq!(heads, ["Now Playing", "Next in Queue", "", "Next From Your List", "Autoplay"]);
        q.drop_autoplay();
        assert!(q.items.iter().all(|i| !i.auto));
        q.remove(1);
        assert_eq!(q.items[1].t.key, "mine");
    }

    #[test]
    fn never_heard_comes_first_and_the_latest_last() {
        let lp: HashMap<String, u64> = [("old".to_string(), 1), ("new".to_string(), 100), ("mid".into(), 50), ("mid2".into(), 60)].into();
        let out = fair_shuffle(vec![t("new", "a", true), t("never", "a", true), t("old", "a", true), t("mid", "a", true), t("mid2", "a", true)], &lp);
        assert_eq!(out[0].key, "never");
        assert_eq!(out.last().unwrap().key, "new");
    }

    #[test]
    fn autoplay_takes_turns_and_limits_artists() {
        let sc = vec![t("s1", "a", true), t("s2", "a", true), t("s3", "a", true), t("s4", "b", true)];
        let yt = vec![t("y1", "c", false), t("y2", "d", false)];
        let out = take_turns(vec![("soundcloud", sc), ("youtube", yt)], "soundcloud", &HashSet::new(), |_| true);
        let keys: Vec<&str> = out.iter().map(|x| x.key.as_str()).collect();
        // youtube first (the last track was SoundCloud), turns after, and only one platform left stops it
        assert_eq!(keys, ["y1", "s1", "y2", "s2"]);
        let taken: HashSet<String> = ["s1".to_string()].into();
        let out = take_turns(vec![("soundcloud", vec![t("s1", "a", true), t("s2", "a", true), t("s3", "a", true)])], "", &taken, |_| true);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn the_session_comes_back() {
        let mut q = Queue::default();
        q.set_list(vec![t("1", "a", true), t("2", "b", true)], 0);
        q.items[1].auto = true;
        q.qi = Some(1);
        let v = q.to_session(42_000);
        let (back, pos) = Queue::from_session(&v).unwrap();
        assert_eq!((back.qi, pos, back.items.len(), back.items[1].auto), (Some(1), 42_000, 2, true));
    }

    #[test]
    fn recent_plays_are_spaced_by_their_share() {
        let now = 100 * 86_400_000u64;
        let list: Vec<Track> = (0..100).map(|i| t(&format!("k{i}"), "A", true)).collect();
        for (share, every) in [(10, 10), (50, 2)] {
            let lp: HashMap<String, u64> = (0..share).map(|i| (format!("k{i}"), now - 1000)).collect();
            let out = space_recent(list.clone(), &lp, 7 * 86_400_000, now);
            let at: Vec<usize> = out.iter().enumerate().filter(|(_, x)| lp.contains_key(&x.key)).map(|(k, _)| k + 1).collect();
            assert_eq!(out.len(), 100);
            assert!(at.iter().take(5).enumerate().all(|(n, &p)| p == (n + 1) * every), "{share}%: {at:?}");
        }
    }

}
