// A page loading (Discover; the Library, which Rediscover is made from): each platform it asks, ticked
// as it answers, so the page can show which one it's still waiting on.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

struct Job {
    /// the pages it's for
    pages: &'static [&'static str],
    /// (platform, its name, answered)
    steps: Vec<(&'static str, &'static str, bool)>,
}

static JOB: Mutex<Option<Job>> = Mutex::new(None);
/// changed each time: what's shown is made again only then
static CHANGED: AtomicU64 = AtomicU64::new(0);

/// A load begun, for `pages`, asking these platforms (platform, its name)
pub fn begin(pages: &'static [&'static str], steps: &[(&'static str, &'static str)]) {
    if steps.is_empty() { return end(pages); }
    *JOB.lock().unwrap() = Some(Job { pages, steps: steps.iter().map(|(p, n)| (*p, *n, false)).collect() });
    CHANGED.fetch_add(1, Ordering::Relaxed);
}

/// A platform answered
pub fn done(plat: &str) {
    if let Some(j) = JOB.lock().unwrap().as_mut() {
        for s in j.steps.iter_mut().filter(|s| s.0 == plat) { s.2 = true; }
    }
    CHANGED.fetch_add(1, Ordering::Relaxed);
}

/// The load over (its page shown)
pub fn end(pages: &'static [&'static str]) {
    let mut j = JOB.lock().unwrap();
    if j.as_ref().is_some_and(|x| x.pages == pages) { *j = None; }
    CHANGED.fetch_add(1, Ordering::Relaxed);
}

/// What to show on `page`: its steps, or none; and the change count, to tell when it's new
pub fn shown(page: &str) -> (u64, Vec<(&'static str, &'static str, bool)>) {
    let n = CHANGED.load(Ordering::Relaxed);
    let j = JOB.lock().unwrap();
    (n, j.as_ref().filter(|x| x.pages.contains(&page)).map(|x| x.steps.clone()).unwrap_or_default())
}
