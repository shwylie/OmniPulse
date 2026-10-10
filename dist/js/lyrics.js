// Lyrics, from several places in the order you choose (Settings > Integrations > Lyrics): LRCLIB,
// NetEase Cloud Music, KuGou and YouTube Music (crates/core/src/lyrics.rs). The first with timed
// lyrics wins; plain ones are shown when that's all there is. Now Playing can switch a song's lyrics
// to another place, and that choice is kept.

const LYRICS_PROVIDERS = { lrclib: "LRCLIB", netease: "NetEase Cloud Music", kugou: "KuGou", youtube: "YouTube Music" };
const LYRICS_PICK_KEY = "sc.lyricsPick";
// [{ id, on }] in order
function lyricsOrder() {
  const saved = Array.isArray(prefs.lyricsOrder) ? prefs.lyricsOrder.filter((x) => LYRICS_PROVIDERS[x.id]) : [];
  Object.keys(LYRICS_PROVIDERS).forEach((id) => { if (!saved.some((x) => x.id === id)) saved.push({ id, on: true }); });
  return saved;
}
function lyricsPicks() { try { return JSON.parse(localStorage.getItem(LYRICS_PICK_KEY) || "{}"); } catch (e) { return {}; } }

// One provider's lyrics for a track (or null)
async function lyricsFrom(t, provider) {
  const { artist, title } = guessNames(t);
  const m = ytMatchesLoad()[t.id];
  const videoId = (typeof sbVideoOf === "function" && sbVideoOf(t)) || (m && m.id) || "";
  try {
    return await invoke("lyrics_from", { provider, artist, title, secs: Math.round((t.duration || 0) / 1000), videoId });
  } catch (e) { return null; }
}

// The track's lyrics: the ones you chose for it, or the first timed ones in order (else the first plain)
async function findLyrics(t) {
  const chosen = lyricsPicks()[t.id];
  if (chosen && LYRICS_PROVIDERS[chosen]) { const d = await lyricsFrom(t, chosen); if (d) return d; }
  let plain = null;
  for (const p of lyricsOrder().filter((x) => x.on)) {
    const d = await lyricsFrom(t, p.id);
    if (d && d.synced) return d;
    if (d && !plain) plain = d;
  }
  return plain;
}

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
  if (data && data.synced) {
    lyricsLines = parseLrc(data.synced);
    lyricsLines.forEach((line, i) => {
      const p = el("p", "lyLine", line.text || "\u00a0");
      p.dataset.i = i;
      box.append(p);
    });
  } else if (data && data.plain) {
    box.append(el("p", "lyPlain", data.plain));
  } else {
    box.append(el("p", "lyNote", "No lyrics found for this track."));
  }
  box.append(lyricsSourceLine(data));
}

// "Lyrics from KuGou. Other sources": the other places, one click each, the choice kept for the song
function lyricsSourceLine(data) {
  const line = el("p", "lyNote");
  if (data) line.append(document.createTextNode("Lyrics from " + data.source + (data.note ? " (" + data.note.replace(/^Source:\s*/i, "") + ")" : "") + ". "));
  const others = lyricsOrder().map((x) => x.id).filter((id) => !data || LYRICS_PROVIDERS[id] !== data.source);
  const btn = el("button", "linkBtn", data ? "Other sources" : "Look elsewhere");
  btn.type = "button";
  btn.addEventListener("click", () => {
    const row = el("span", "lyOthers");
    others.forEach((id) => {
      const b = el("button", "linkBtn", LYRICS_PROVIDERS[id]);
      b.type = "button";
      b.addEventListener("click", async () => {
        const t = curTrack();
        if (!t) return;
        b.textContent = LYRICS_PROVIDERS[id] + "...";
        const d = await lyricsFrom(t, id);
        if (!d) { b.textContent = LYRICS_PROVIDERS[id] + ": none"; b.disabled = true; return; }
        const picks = lyricsPicks();
        picks[t.id] = id;
        try { localStorage.setItem(LYRICS_PICK_KEY, JSON.stringify(picks)); } catch (e) {}
        lyricsCache.set(t.id, d);
        if (curTrack() === t) { renderLyrics(d); highlightLyric(curPosMs() / 1000); }
      });
      row.append(b);
    });
    btn.replaceWith(row);
  });
  if (others.length) line.append(btn);
  return line;
}

// Settings: the providers in order, each with a switch and buttons to move it
function paintLyricsProviders() {
  const box = $("lyricsProviders");
  const order = lyricsOrder();
  const save = (o) => { prefs.lyricsOrder = o; savePrefs(); lyricsCache.clear(); lyricsFor = ""; paintLyricsProviders(); };
  box.replaceChildren(...order.map((p, i) => {
    const row = el("div", "lyProvider");
    const on = el("input");
    on.type = "checkbox";
    on.checked = p.on;
    on.title = "Use " + LYRICS_PROVIDERS[p.id];
    on.addEventListener("change", () => save(order.map((x) => (x.id === p.id ? { id: x.id, on: on.checked } : x))));
    const move = (d) => { const o = order.slice(); [o[i], o[i + d]] = [o[i + d], o[i]]; save(o); };
    const up = el("button", "ghost", "\u2191");
    up.type = "button"; up.title = "Ask it sooner"; up.disabled = i === 0; up.addEventListener("click", () => move(-1));
    const down = el("button", "ghost", "\u2193");
    down.type = "button"; down.title = "Ask it later"; down.disabled = i === order.length - 1; down.addEventListener("click", () => move(1));
    row.append(el("span", "lyNum", String(i + 1)), on, el("span", "lyName", LYRICS_PROVIDERS[p.id]), up, down);
    return row;
  }));
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
    const top = cur.offsetTop - box.offsetTop - box.clientHeight / 2 + cur.clientHeight / 2;
    box.scrollTo({ top, behavior: prefs.animOn === false ? "auto" : "smooth" });   // glides to the new line
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
    data = await findLyrics(t);
    lyricsCache.set(t.id, data);
  }
  if (lyricsFor !== t.id) return;
  renderLyrics(data);
  highlightLyric(curPosMs() / 1000);
}

function lyricsInit() {
  $("lyricsBtn").addEventListener("click", () => setLyricsMode(!lyricsMode));
  paintLyricsProviders();
}
