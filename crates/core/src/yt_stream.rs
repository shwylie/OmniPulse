// Finds a direct audio stream URL for a search, using yt-dlp from the system PATH:
//   yt-dlp "ytsearch1:<query>" --print "%(available_at|0)s %(url)s" -f "ba[ext=m4a]/ba"
// (the best M4A audio of the top YouTube result, or the best audio of any kind).
//
// The query is passed as one argument with no shell in between, so it can't run commands, and it
// always starts with "ytsearch1:", so yt-dlp can't mistake it for an option. YouTube's stream
// URLs expire after a few hours: resolve them shortly before playing.

use std::ffi::OsStr;
use std::process::Stdio;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(30);

// Runs `program` (yt-dlp, or a stand-in in tests) for the query
async fn resolve_with(program: impl AsRef<OsStr>, query: &str) -> Result<String, String> {
    let query = query.trim();
    if query.is_empty() {
        return Err("Nothing to search for.".into());
    }
    run(program, format!("ytsearch1:{query}"), query, "ba[ext=m4a]/ba").await
}

// YouTube's player needs a JavaScript runtime (yt-dlp issue #15012): our own Deno (tools.rs) if it's
// set up, otherwise yt-dlp finds Deno on the PATH by itself, or is told to use Node if that's all there is.
fn js_runtime_args() -> Vec<String> {
    if let Some(deno) = crate::tools::deno_path() {
        vec!["--js-runtimes".into(), format!("deno:{}", deno.display())]
    } else if !on_path("deno") && on_path("node") {
        vec!["--js-runtimes".into(), "node".into()]
    } else {
        Vec::new()
    }
}

/// Whether a program is on the PATH
pub fn on_path(name: &str) -> bool {
    let exts: &[&str] = if cfg!(windows) { &[".exe", ".cmd", ".bat"] } else { &[""] };
    std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).any(|dir| exts.iter().any(|e| dir.join(format!("{name}{e}")).is_file())))
        .unwrap_or(false)
}

// YouTube asking yt-dlp to sign in "to confirm you're not a bot" (it's had too many requests from this
// connection): YouTube left alone for a while, every yt-dlp lookup turned down at once meanwhile, since
// asking again keeps it asking (2026-11-03: twenty songs in a minute, each failing and moving on to the
// next, each another request). 5 minutes, twice as long each time it asks again, up to 30; back to 5 once
// a lookup works.
// With PO tokens set up (crate::pot), YouTube asking first moves lookups onto its web client with a token
// for TOKENS_SECS, the song asked about again at once; it's only left alone if it asks even then.
const BOT_FIRST_SECS: u64 = 300;
const BOT_MOST_SECS: u64 = 1800;
const TOKENS_SECS: u64 = 6 * 3600;
// (until when it's left alone, how long next time, and until when tokens are used; each test's own, as
// they run side by side)
type Bot = (u64, u64, u64);
// Kept through a restart (<data>/state/youtube-pause.json): YouTube goes on asking for hours once it's started,
// and a restart that forgot would ask it again at once, each ask making it longer.
#[cfg(not(test))]
static BOT: std::sync::LazyLock<std::sync::Mutex<Bot>> = std::sync::LazyLock::new(|| std::sync::Mutex::new(bot_kept(bot_file().and_then(|p| std::fs::read_to_string(p).ok()).as_deref(), crate::now_secs())));
#[cfg(not(test))]
fn bot_state<T>(f: impl FnOnce(&mut Bot) -> T) -> T {
    let mut g = BOT.lock().unwrap_or_else(|e| e.into_inner());
    let was = *g;
    let r = f(&mut g);
    if *g != was {
        if let Some(p) = bot_file() {
            if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
            let _ = std::fs::write(p, serde_json::json!({ "until": g.0, "step": g.1, "tokens": g.2, "at": crate::now_secs() }).to_string());
        }
    }
    r
}

#[cfg(not(test))]
fn bot_file() -> Option<std::path::PathBuf> {
    crate::data_dir().ok().map(|d| d.join("state").join("youtube-pause.json"))
}

/// The pause as it was kept, read at `now`: what's still to come of it; how long the next one is, back to the
/// first's once YouTube's been left alone for six hours since
#[cfg_attr(test, allow(dead_code))]
fn bot_kept(text: Option<&str>, now: u64) -> Bot {
    let v: serde_json::Value = text.and_then(|t| serde_json::from_str(t).ok()).unwrap_or(serde_json::Value::Null);
    let n = |k: &str| v[k].as_u64().unwrap_or(0);
    let fresh = now.saturating_sub(n("at")) < 6 * 3600;
    (n("until").min(now + BOT_MOST_SECS), if fresh { n("step").clamp(BOT_FIRST_SECS, BOT_MOST_SECS) } else { BOT_FIRST_SECS }, n("tokens").min(now + TOKENS_SECS))
}

// How many lookups there have been lately (each one's time, the last hour's kept): said in the log when YouTube
// asks for its bot check, to see what led to it
static LOOKUPS: std::sync::Mutex<Vec<u64>> = std::sync::Mutex::new(Vec::new());

fn lookup_noted() {
    let now = crate::now_secs();
    if let Ok(mut l) = LOOKUPS.lock() { l.retain(|t| now.saturating_sub(*t) < 3600); l.push(now); }
}

/// (in the last ten minutes, in the last hour)
fn lookups_lately() -> (usize, usize) {
    let now = crate::now_secs();
    LOOKUPS.lock().map(|l| (l.iter().filter(|t| now.saturating_sub(**t) < 600).count(), l.iter().filter(|t| now.saturating_sub(**t) < 3600).count())).unwrap_or((0, 0))
}
#[cfg(test)]
thread_local! {
    static BOT: std::cell::RefCell<Bot> = const { std::cell::RefCell::new((0, BOT_FIRST_SECS, 0)) };
    /// (whether a test has tokens set up)
    static TEST_TOKENS: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
#[cfg(test)]
fn bot_state<T>(f: impl FnOnce(&mut Bot) -> T) -> T { BOT.with(|b| f(&mut b.borrow_mut())) }

/// PO tokens set up (a test's: as it says)
fn tokens_ready() -> bool {
    #[cfg(test)]
    { TEST_TOKENS.with(|t| t.get()) }
    #[cfg(not(test))]
    { crate::pot::ready() }
}

/// Their setup started, if they aren't (never in a test)
fn tokens_set_up() {
    #[cfg(not(test))]
    crate::pot::start_setup();
}

/// Lookups are using tokens, since YouTube asked
fn tokens_on() -> bool {
    tokens_ready() && bot_state(|b| b.2) > crate::now_secs()
}

/// What makes yt-dlp's lookup a token's: its plugin, its script, and YouTube's web client (which sends one)
fn tokens_args() -> Vec<String> {
    let mut a = crate::pot::ytdlp_args();
    a.extend(["--extractor-args".into(), "youtube:player_client=mweb".into()]);
    a
}

/// How long YouTube is still being left alone (seconds), after it asked for its bot check
pub fn bot_wait() -> Option<u64> {
    let (until, now) = (bot_state(|b| b.0), crate::now_secs());
    (until > now).then(|| until - now)
}

fn bot_said(wait: u64) -> String {
    format!("YouTube asked OmniPulse to prove it isn't a bot (too many requests), so YouTube is paused for {} min", wait.div_ceil(60).max(1))
}

/// yt-dlp's error is YouTube's bot check
/// A helper's error line, as a person reads it: "ERROR: [youtube] abc: Video unavailable" is "Video unavailable"
fn plain(why: &str) -> String {
    let s = why.trim().trim_start_matches("ERROR:").trim();
    let s = s.strip_prefix('[').and_then(|r| r.split_once(']')).map(|(_, r)| r.trim()).unwrap_or(s);
    let s = s.split_once(": ").filter(|(id, _)| id.len() <= 16 && !id.contains(' ')).map(|(_, r)| r).unwrap_or(s);
    s.to_string()
}

fn bot_check(why: &str) -> bool {
    let w = why.to_lowercase();
    w.contains("not a bot") || w.contains("confirm you")
}

/// YouTube's bot check met: left alone from now, for longer each time
fn bot_hold() -> u64 {
    let step = bot_state(|b| { let step = b.1; b.0 = crate::now_secs() + step; b.1 = (step * 2).min(BOT_MOST_SECS); step });
    let (ten, hour) = lookups_lately();
    crate::diag::note(&format!("youtube: its bot check; leaving it alone for {step} s ({ten} lookups in the last 10 min, {hour} in the last hour)"));
    step
}

/// Why a lookup didn't work: YouTube's bot check, or anything else (said)
enum Fail { Bot, Other(String) }

/// yt-dlp found no format it could use: as when YouTube gave a request only its newer streaming (SABR)
fn sabr(why: &str) -> bool {
    why.contains("Requested format is not available")
}

async fn run(program: impl AsRef<OsStr>, target: String, label: &str, format: &str) -> Result<String, String> {
    if let Some(wait) = bot_wait() { return Err(bot_said(wait) + "."); }
    let tokens = tokens_on();
    let first = run_once(program.as_ref(), &target, label, format, tokens).await;
    let r = match first {
        // (YouTube asking, tokens set up: on to them for a while, and this one asked again with one)
        Err(Fail::Bot) if !tokens && tokens_ready() => {
            bot_state(|b| b.2 = crate::now_secs() + TOKENS_SECS);
            crate::diag::note("youtube: its bot check; lookups use PO tokens for a while");
            run_once(program.as_ref(), &target, label, format, true).await
        }
        r => r,
    };
    // (with a token, its web client's formats now and then come without their addresses, YouTube trying its
    // newer streaming on that request (SABR): asked once more, as the next request mostly isn't)
    let r = match r {
        Err(Fail::Other(e)) if tokens_on() && sabr(&e) => run_once(program.as_ref(), &target, label, format, true).await,
        r => r,
    };
    match r {
        Ok(url) => { bot_state(|b| b.1 = BOT_FIRST_SECS); Ok(url) }
        Err(Fail::Bot) => {
            // (no tokens yet: set up now, for next time)
            if !tokens_ready() { tokens_set_up(); }
            Err(bot_said(bot_hold()) + ".")
        }
        Err(Fail::Other(e)) => Err(e),
    }
}

async fn run_once(program: &OsStr, target: &str, label: &str, format: &str, tokens: bool) -> Result<String, Fail> {
    let query = label;
    if target.contains("youtube.com/") { lookup_noted(); }
    let mut cmd = tokio::process::Command::new(program);
    cmd.arg(target)
        // (its address, and when YouTube lets it be used: an address used before then is refused, 403)
        .args(["--print", "%(available_at|0)s %(url)s", "-f", format, "--no-playlist", "--no-warnings"])
        // (not the video's streaming playlist: a request to YouTube each lookup for formats that are never played)
        .args(if tokens { tokens_args() } else { vec!["--extractor-args".into(), "youtube:skip=hls".into()] })
        .args(js_runtime_args())
        .args(crate::tools::cache_args())
        .envs(crate::tools::cache_env())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);   // a timed-out yt-dlp doesn't keep running
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);   // CREATE_NO_WINDOW: no console window flashing up

    let child = cmd.spawn().map_err(|e| Fail::Other(
        if e.kind() == std::io::ErrorKind::NotFound {
            "YouTube Music isn't set up yet: Settings > Integrations > YouTube Music > Set Up.".to_string()
        } else {
            format!("Couldn't start YouTube Music's helper: {e}")
        }
    ))?;
    let out = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| Fail::Other(format!("YouTube Music took longer than {} seconds and was stopped.", TIMEOUT.as_secs())))?
        .map_err(|e| Fail::Other(format!("YouTube Music failed: {e}")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // yt-dlp prints "ERROR: ..." lines; the last one is usually the useful one
        let why = stderr.lines().rev().find(|l| l.contains("ERROR")).or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
            .unwrap_or("no details").trim();
        if bot_check(why) { return Err(Fail::Bot); }
        let code = out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into());
        return Err(Fail::Other(format!("YouTube Music couldn't find a stream for \"{query}\": {}", plain(why))));
    }
    // a line per format: when it can be used, then its address; with an audio-only format there's one,
    // but the first is taken to be safe
    let stdout = String::from_utf8_lossy(&out.stdout);
    let (at, url) = stdout
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            // ("<when> <address>"; or the address alone, from an older yt-dlp)
            let (a, u) = l.split_once(' ').filter(|(a, _)| !a.starts_with("http")).unwrap_or(("0", l));
            let u = u.trim();
            (u.starts_with("http://") || u.starts_with("https://")).then(|| (a.parse::<u64>().unwrap_or(0), u.to_string()))
        })
        .next()
        .ok_or_else(|| Fail::Other(format!("YouTube Music found nothing to play for \"{query}\".")))?;
    // YouTube hands out its streams a few seconds before they work (yt-dlp waits for them too): waited
    // for here, so whatever opens it next isn't refused (never more than 15 s)
    let wait = at.saturating_sub(crate::now_secs()).min(15);
    if wait > 0 { tokio::time::sleep(Duration::from_secs(wait) + Duration::from_millis(300)).await; }
    Ok(url)
}

/// The audio of one YouTube video (the best M4A, or the best audio of any kind)
pub async fn resolve_video(video_id: &str) -> Result<String, String> {
    // always inside a URL, so an id can never be read as an option
    run(crate::tools::ytdlp_program(), format!("https://music.youtube.com/watch?v={video_id}"), video_id, "ba[ext=m4a]/ba").await
}

/// A SoundCloud track's own audio without signing in to SoundCloud (yt-dlp's public access): a
/// plain download if there is one, else its HLS stream. Only soundcloud.com track links are taken.
pub async fn resolve_soundcloud(url: &str) -> Result<String, String> {
    if !soundcloud_link(url) {
        return Err("Only a SoundCloud track's link can be played this way.".into());
    }
    run(crate::tools::ytdlp_program(), url.to_string(), url, "ba[protocol=https]/ba[protocol=http]/ba").await
}

fn soundcloud_link(url: &str) -> bool {
    let rest = url.strip_prefix("https://soundcloud.com/").or_else(|| url.strip_prefix("https://m.soundcloud.com/"));
    rest.is_some_and(|r| !r.is_empty() && r.len() < 300 && r.chars().all(|c| c.is_ascii_alphanumeric() || "-_/.%".contains(c)) && r.contains('/'))
}

/// What's installed: yt-dlp's version (if it runs), the JavaScript runtimes, whether they're our
/// built-in copies, and how setting them up is going
/// A program on the PATH (the native app's YouTube Music's Helpers)
pub fn on_path_public(name: &str) -> bool { on_path(name) }

pub async fn status() -> serde_json::Value {
    let mut cmd = tokio::process::Command::new(crate::tools::ytdlp_program());
    cmd.arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).kill_on_drop(true);
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);
    let version = match cmd.spawn() {
        Ok(child) => tokio::time::timeout(Duration::from_secs(15), child.wait_with_output())
            .await
            .ok()
            .and_then(|r| r.ok())
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string()),
        Err(_) => None,
    };
    serde_json::json!({
        "ytdlp": version,
        "deno": crate::tools::deno_path().is_some() || on_path("deno"),
        "node": on_path("node"),
        "builtIn": crate::tools::ytdlp_path().is_some() && crate::tools::deno_path().is_some(),
        "diskUse": crate::tools::disk_use(),
        "setup": crate::tools::progress(),
    })
}

pub async fn resolve_yt_stream(query: String) -> Result<String, String> {
    resolve_with("yt-dlp", &query).await
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // A stand-in for yt-dlp: a batch file that behaves the way we tell it
    fn fake(name: &str, body: &str) -> PathBuf {
        let dir = std::env::temp_dir().join("sc-client-yt-tests");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.bat"));
        std::fs::write(&path, format!("@echo off\r\n{body}\r\n")).unwrap();
        path
    }

    #[tokio::test]
    async fn returns_the_trimmed_url() {
        let p = fake("ok", "echo https://rr1.googlevideo.com/videoplayback?id=abc   \r\necho.");
        assert_eq!(resolve_with(&p, "Daft Punk - Get Lucky").await.unwrap(), "https://rr1.googlevideo.com/videoplayback?id=abc");
    }

    #[tokio::test]
    async fn passes_the_search_and_format_flags() {
        let p = fake("args", "echo https://x/?args=%*");
        let url = resolve_with(&p, "Daft Punk - Get Lucky").await.unwrap();
        assert!(url.contains("ytsearch1:Daft Punk - Get Lucky"), "{url}");
        assert!(url.contains("--print") && url.contains("ba[ext=m4a]/ba"), "{url}");
    }

    #[tokio::test]
    async fn it_waits_till_youtube_lets_the_stream_be_used() {
        // (YouTube's streams are refused, 403, till their available_at: waited for, so it's never opened too soon)
        let at = crate::now_secs() + 2;
        let p = fake("later", &format!("echo {at} https://rr1.googlevideo.com/videoplayback?id=abc"));
        let t = std::time::Instant::now();
        assert_eq!(resolve_with(&p, "x").await.unwrap(), "https://rr1.googlevideo.com/videoplayback?id=abc");
        assert!(crate::now_secs() >= at && t.elapsed() >= std::time::Duration::from_millis(1000), "{:?}", t.elapsed());
    }

    #[tokio::test]
    async fn a_failing_yt_dlp_gives_its_error() {
        let p = fake("fail", "echo WARNING: something 1>&2\r\necho ERROR: [youtube] abc: Video unavailable 1>&2\r\nexit /b 1");
        let e = resolve_with(&p, "x").await.unwrap_err();
        assert_eq!(e, "YouTube Music couldn't find a stream for \"x\": Video unavailable");
        assert!(bot_wait().is_none());
    }

    #[test]
    fn the_pause_is_kept_through_a_restart() {
        let now = 1_000_000;
        // (a pause still running, its next one twice as long, tokens still on: all as they were)
        let kept = serde_json::json!({ "until": now + 200, "step": 600, "tokens": now + 3600, "at": now - 100 }).to_string();
        assert_eq!(bot_kept(Some(&kept), now), (now + 200, 600, now + 3600));
        // (long over: nothing left of it, and the next one the first's length again)
        assert_eq!(bot_kept(Some(&kept), now + 7 * 3600), (now + 200, BOT_FIRST_SECS, now + 3600));
        assert!(now + 200 < now + 7 * 3600);
        // (nothing kept, or a file that isn't what it should be: as new; a time far ahead (the clock changed): no
        // longer than the longest pause)
        assert_eq!(bot_kept(None, now), (0, BOT_FIRST_SECS, 0));
        assert_eq!(bot_kept(Some("not json"), now), (0, BOT_FIRST_SECS, 0));
        let far = serde_json::json!({ "until": now + 999_999, "step": 5, "tokens": 0, "at": now }).to_string();
        assert_eq!(bot_kept(Some(&far), now), (now + BOT_MOST_SECS, BOT_FIRST_SECS, 0));
    }

    #[tokio::test]
    async fn youtubes_bot_check_leaves_youtube_alone_for_a_while() {
        let p = fake("bot", "echo ERROR: [youtube] abc: Sign in to confirm you're not a bot. This helps protect our community. 1>&2\r\nexit /b 1");
        let e = resolve_with(&p, "x").await.unwrap_err();
        assert!(e.contains("isn't a bot") && e.contains("5 min"), "{e}");
        let w = bot_wait().expect("left alone");
        assert!((290..=300).contains(&w), "{w}");
        // (meanwhile nothing's asked: even one that would work is turned down at once)
        let ok = fake("ok-later", "echo https://rr1.googlevideo.com/videoplayback?id=abc");
        assert!(resolve_with(&ok, "x").await.unwrap_err().contains("paused"));
        // (asked again after: twice as long)
        bot_state(|b| b.0 = 0);
        assert!(resolve_with(&p, "x").await.unwrap_err().contains("10 min"));
        // (a lookup that works: back to 5 next time)
        bot_state(|b| b.0 = 0);
        assert!(resolve_with(&ok, "x").await.is_ok());
        assert!(resolve_with(&p, "x").await.unwrap_err().contains("5 min"));
    }

    #[tokio::test]
    async fn with_tokens_set_up_youtube_asking_moves_lookups_onto_them() {
        TEST_TOKENS.with(|t| t.set(true));
        // (YouTube asks unless it's its web client, with a token)
        let p = fake("tokens", "echo %* | findstr /c:\"player_client=mweb\" >nul && (echo https://rr1.googlevideo.com/videoplayback?id=tok & exit /b 0)\r\necho ERROR: [youtube] abc: Sign in to confirm you're not a bot. 1>&2\r\nexit /b 1");
        // (asked: this one again with a token, at once, and it works; YouTube isn't left alone)
        assert_eq!(resolve_with(&p, "x").await.unwrap(), "https://rr1.googlevideo.com/videoplayback?id=tok");
        assert!(bot_wait().is_none() && tokens_on());
        // (the next one with a token from the start)
        assert_eq!(resolve_with(&p, "y").await.unwrap(), "https://rr1.googlevideo.com/videoplayback?id=tok");
        // (asking even with a token: left alone then)
        let always = fake("tokens-bot", "echo ERROR: [youtube] abc: Sign in to confirm you're not a bot. 1>&2\r\nexit /b 1");
        assert!(resolve_with(&always, "z").await.unwrap_err().contains("paused"));
        assert!(bot_wait().is_some());
    }

    /// (for real: PO tokens set up in OmniPulse's own folder, as the app does it, then one lookup with a token)
    /// cargo test -p omnipulse-core --features desktop --lib tokens_for_real -- --ignored --nocapture
    #[tokio::test]
    #[ignore]
    async fn tokens_for_real() {
        let t0 = std::time::Instant::now();
        if !crate::pot::ready() { crate::pot::setup().await.expect("set up"); }
        assert!(crate::pot::ready());
        println!("set up in {:.0} s", t0.elapsed().as_secs_f64());
        // (a few songs, each with a token, and how each went: YouTube's newer streaming, or its bot check even
        // with one, can come up on any request)
        let mut worked = 0;
        for id in ["dQw4w9WgXcQ", "fG_TXsA2yjk", "t9w8W1EOlA0"] {
            let t1 = std::time::Instant::now();
            let r = run_once(crate::tools::ytdlp_program().as_ref(), &format!("https://music.youtube.com/watch?v={id}"), id, "ba[ext=m4a]/ba", true).await;
            let said = match &r { Ok(u) => { worked += 1; format!("worked: {}...", &u[..50.min(u.len())]) }, Err(Fail::Bot) => "YouTube asked even with a token".into(), Err(Fail::Other(e)) => e.clone() };
            println!("{id} in {:.0} s: {said}", t1.elapsed().as_secs_f64());
        }
        assert!(worked > 0, "none worked with a token");
    }

    #[tokio::test]
    async fn no_url_is_an_error() {
        let p = fake("empty", "echo.");
        assert!(resolve_with(&p, "x").await.unwrap_err().contains("found nothing"));
    }

    #[tokio::test]
    async fn missing_yt_dlp_is_explained() {
        let e = resolve_with("yt-dlp-definitely-not-installed", "x").await.unwrap_err();
        assert!(e.contains("isn't set up yet"), "{e}");
    }

    #[tokio::test]
    async fn empty_query_is_refused() {
        assert!(resolve_with("yt-dlp", "   ").await.is_err());
    }

    #[test]
    fn soundcloud_links_only() {
        assert!(super::soundcloud_link("https://soundcloud.com/artist/a-song"));
        assert!(!super::soundcloud_link("https://soundcloud.com/artist"));
        assert!(!super::soundcloud_link("https://soundcloud.com/a/b?x=--exec"));
        assert!(!super::soundcloud_link("https://evil.com/soundcloud.com/a/b"));
        assert!(!super::soundcloud_link("--exec=calc"));
    }
}
