// Finds a direct audio stream URL for a search, using yt-dlp from the system PATH:
//   yt-dlp "ytsearch1:<query>" -g -f "ba[ext=m4a]/ba"
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
    let mut cmd = tokio::process::Command::new(program.as_ref());
    cmd.arg(format!("ytsearch1:{query}"))
        .args(["-g", "-f", "ba[ext=m4a]/ba", "--no-playlist", "--no-warnings"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);   // a timed-out yt-dlp doesn't keep running
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000);   // CREATE_NO_WINDOW: no console window flashing up

    let child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            "yt-dlp isn't installed, or isn't on your PATH. Install it from https://github.com/yt-dlp/yt-dlp and restart the app.".to_string()
        } else {
            format!("Couldn't start yt-dlp: {e}")
        }
    })?;
    let out = tokio::time::timeout(TIMEOUT, child.wait_with_output())
        .await
        .map_err(|_| format!("yt-dlp took longer than {} seconds and was stopped.", TIMEOUT.as_secs()))?
        .map_err(|e| format!("yt-dlp failed: {e}"))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        // yt-dlp prints "ERROR: ..." lines; the last one is usually the useful one
        let why = stderr.lines().rev().find(|l| l.contains("ERROR")).or_else(|| stderr.lines().rev().find(|l| !l.trim().is_empty()))
            .unwrap_or("no details").trim();
        let code = out.status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into());
        return Err(format!("yt-dlp couldn't resolve \"{query}\" (exit {code}): {why}"));
    }
    // -g prints one URL per line; with an audio-only format there's one, but take the first to be safe
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("http://") || l.starts_with("https://"))
        .map(str::to_string)
        .ok_or_else(|| format!("yt-dlp found nothing to play for \"{query}\"."))
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
        assert!(url.contains("-g -f") && url.contains("ba[ext=m4a]/ba"), "{url}");
    }

    #[tokio::test]
    async fn a_failing_yt_dlp_gives_its_error() {
        let p = fake("fail", "echo WARNING: something 1>&2\r\necho ERROR: [youtube] abc: Sign in to confirm you're not a bot 1>&2\r\nexit /b 1");
        let e = resolve_with(&p, "x").await.unwrap_err();
        assert!(e.contains("exit 1") && e.contains("Sign in to confirm"), "{e}");
    }

    #[tokio::test]
    async fn no_url_is_an_error() {
        let p = fake("empty", "echo.");
        assert!(resolve_with(&p, "x").await.unwrap_err().contains("found nothing"));
    }

    #[tokio::test]
    async fn missing_yt_dlp_is_explained() {
        let e = resolve_with("yt-dlp-definitely-not-installed", "x").await.unwrap_err();
        assert!(e.contains("isn't installed"), "{e}");
    }

    #[tokio::test]
    async fn empty_query_is_refused() {
        assert!(resolve_with("yt-dlp", "   ").await.is_err());
    }
}
