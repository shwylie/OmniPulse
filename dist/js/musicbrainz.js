// MusicBrainz (Settings > Integrations): an open catalogue of songs, searched with no account, so a
// fresh install has music to find, with album and cover. Its songs play from YouTube Music (yt-dlp),
// matched by title, artist and length. MusicBrainz allows about one request a second, so the Rust side
// paces and remembers searches (crates/core/src/musicbrainz.rs); nothing here asks it on a timer.

const mbOn = () => prefs.mbOn !== false;

async function mbSearch(q) {
  return invoke("mb_search", { query: String(q || "") });
}

function mbPaint() {
  $("mbOn").checked = mbOn();
  $("mbNote").textContent = !mbOn() ? "Off."
    : ytOn() ? "On: search finds its songs too, and they play from YouTube Music."
    : "Its songs play from YouTube Music: switch that on to use it.";
}

function mbInit() {
  mbPaint();
  $("mbOn").addEventListener("change", () => setTimeout(mbPaint, 0));   // (js/platforms.js switches it)
}
