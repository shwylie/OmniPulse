// Synced lyrics from LRCLIB.

// ---------- lyrics (from LRCLIB, a free public lyrics database) ----------
const lyricsCache = new Map();
let lyricsMode = false;

// The Lyrics button: opens Now Playing with the lyrics, or hides them again
function setLyricsMode(on) {
  lyricsMode = on;
  $("lyricsBtn").classList.toggle("on", on);
  $("npView").classList.toggle("lyricsMode", on);
  if (on) {
    if (view !== "now") showView("now");
    else ensureLyrics();
  } else {
    $("npLyrics").hidden = true;
  }
}
let lyricsFor = "";
let lyricsLines = null;
let lyricsActive = -1;

// SoundCloud titles are often "Artist - Title (Remix)"; split that up for the search
function guessNames(t) {
  let title = (t.title || "").trim();
  let artist = artistName(t).trim();
  if (srcOf(t) === "spotify") {
    // Spotify's tags are clean: "A, B" is a list of artists, and " - Radio Edit" is a version name
    artist = artist.split(",")[0].trim();
    title = title.replace(/\s+[-\u2013\u2014]\s+[^-\u2013\u2014]*$/, "").trim() || title;
  } else {
    const m = title.match(/^(.+?)\s+[-\u2013\u2014]\s+(.+)$/);
    if (m) { artist = m[1].trim(); title = m[2].trim(); }
  }
  title = title.replace(/\s*[\(\[][^\)\]]*[\)\]]\s*/g, " ").replace(/\s+/g, " ").trim() || title;
  return { artist, title };
}

function parseLrc(text) {
  const out = [];
  text.split("\n").forEach((line) => {
    const m = line.match(/^\[(\d+):(\d+(?:\.\d+)?)\](.*)$/);
    if (m) out.push({ t: Number(m[1]) * 60 + Number(m[2]), text: m[3].trim() });
  });
  return out;
}

function renderLyrics(data) {
  const box = $("npLyrics");
  box.replaceChildren();
  lyricsLines = null;
  lyricsActive = -1;
  if (data && data.syncedLyrics) {
    lyricsLines = parseLrc(data.syncedLyrics);
    lyricsLines.forEach((line, i) => {
      const p = el("p", "lyLine", line.text || "\u00a0");
      p.dataset.i = i;
      box.append(p);
    });
  } else if (data && data.plainLyrics) {
    box.append(el("p", "lyPlain", data.plainLyrics));
  } else {
    box.append(el("p", "lyNote", "No lyrics found for this track."));
    return;
  }
  box.append(el("p", "lyNote", "Lyrics from LRCLIB."));
}

function highlightLyric(time) {
  if (!lyricsLines || !lyricsLines.length) return;
  let i = -1;
  for (let k = 0; k < lyricsLines.length; k++) {
    if (lyricsLines[k].t <= time + 0.2) i = k; else break;
  }
  if (i === lyricsActive) return;
  const box = $("npLyrics");
  const prev = box.querySelector(".lyLine.on");
  if (prev) prev.classList.remove("on");
  lyricsActive = i;
  if (i < 0) return;
  const cur = box.querySelector('.lyLine[data-i="' + i + '"]');
  if (cur) {
    cur.classList.add("on");
    box.scrollTop = cur.offsetTop - box.offsetTop - box.clientHeight / 2 + cur.clientHeight / 2;
  }
}

async function ensureLyrics() {
  const t = curTrack();
  const box = $("npLyrics");
  if (!lyricsMode) { box.hidden = true; return; }
  if (!t) { box.hidden = true; lyricsFor = ""; lyricsLines = null; return; }
  box.hidden = false;
  if (lyricsFor === t.id) return;
  lyricsFor = t.id;
  box.replaceChildren(el("p", "lyNote", "Looking for lyrics..."));
  let data = lyricsCache.get(t.id);
  if (data === undefined) {
    const { artist, title } = guessNames(t);
    const enc = encodeURIComponent;
    const dur = Math.round((t.duration || 0) / 1000);
    data = null;
    try {
      data = await invoke("lyrics_get", {
        url: "https://lrclib.net/api/get?artist_name=" + enc(artist) + "&track_name=" + enc(title) + (dur ? "&duration=" + dur : ""),
      });
    } catch (e) {
      try {
        const list = await invoke("lyrics_get", { url: "https://lrclib.net/api/search?q=" + enc(artist + " " + title) });
        data = Array.isArray(list) && list.length ? list[0] : null;
      } catch (e2) {
        data = null;
      }
    }
    lyricsCache.set(t.id, data);
  }
  if (lyricsFor !== t.id) return;
  renderLyrics(data);
  highlightLyric(curPosMs() / 1000);
}

function lyricsInit() {
  $("lyricsBtn").addEventListener("click", () => setLyricsMode(!lyricsMode));
}
