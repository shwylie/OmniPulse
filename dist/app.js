const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const audio = new Audio();
const win = window.__TAURI__.window.getCurrentWindow();

// ---------- preferences (stored in this app's browser profile) ----------
const DEFAULTS = { theme: "dark", accent: "#8e9bff", density: "comfortable", sink: "default", volume: 0.8, queue: false, repeat: "off", shuffle: false, artAccent: false, css: "", presence: true, scrobble: true, closeToTray: false, loudMatch: "normal", skipSilence: 8, recLang: "en" };
let prefs = { ...DEFAULTS };
try { prefs = { ...DEFAULTS, ...JSON.parse(localStorage.getItem("sc.prefs") || "{}") }; } catch (e) {}
const savePrefs = () => { try { localStorage.setItem("sc.prefs", JSON.stringify(prefs)); } catch (e) {} };

// Pick dark or light text so it stays readable on any accent color
function onAccent(hex) {
  const n = parseInt(hex.slice(1), 16);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  return 0.299 * r + 0.587 * g + 0.114 * b > 150 ? "#111318" : "#ffffff";
}

// Settings > Look > Player Style: a whole look for the player bar (painted further down)
// min: the window's narrowest for it (the others put everything on one row, so they need more room)
const PLAYER_STYLES = {
  tab: { note: "What's playing on a tab above the controls, as wide as it needs.", min: 820 },
  bar: { note: "Everything on one full-width bar: what's playing on the left, the controls in the middle.", min: 1000 },
  float: { note: "One bar too, floating above the bottom of the window with rounded ends.", min: 1040 },
  minimal: { note: "A slim row in the panel colour, with the progress as a thin line along its top.", min: 1000 },
};
const playerStyle = () => (PLAYER_STYLES[prefs.playerStyle] ? prefs.playerStyle : "tab");

let playerMinNow = 0;
function playerMinWidth(w) {
  if (w === playerMinNow) return;
  playerMinNow = w;
  const api = window.__TAURI__ && window.__TAURI__.window;
  const Size = (window.__TAURI__ && window.__TAURI__.dpi && window.__TAURI__.dpi.LogicalSize) || (api && api.LogicalSize);
  if (!api || !Size) return;
  win.setMinSize(new Size(w, 520)).catch(() => {});
  // narrower than that right now: as wide as it needs
  if (innerWidth < w) win.setSize(new Size(w, Math.max(520, innerHeight))).catch(() => {});
}

function applyPrefs() {
  const root = document.documentElement;
  root.dataset.tab = prefs.tabAlign === "right" ? "right" : "left";
  // Settings > Look > Player Style (and the window's narrowest, so it always fits)
  const ps = playerStyle();
  root.dataset.player = ps;
  root.classList.toggle("noTab", ps !== "tab");
  playerMinWidth(PLAYER_STYLES[ps].min);
  root.classList.toggle("square", prefs.appRounded === false);   // Settings > App > Rounded Corners
  applyTheme();   // light or dark, and its tint (js/appearance.js)
  root.dataset.density = prefs.density;
  if (prefs.artAccent && artColors && !artPaused) setAccentColors(artColors[0], artColors[1]);
  else setAccentColors(prefs.accent, prefs.accent2 || prefs.accent);
  applyUserCss();
}
applyPrefs();

// ---------- helpers ----------
// A line with an account name in it: the name in a span of its own, which Streamer Mode hides
function nameNote(box, before, name, after) {
  box.replaceChildren(document.createTextNode(before), el("span", "sensitive", name), document.createTextNode(after || ""));
}

// ---------- Streamer Mode: account names, keys, addresses and ports, and plugin settings, hidden ----------
// The title bar's button does both Streamer Modes, a click at a time: off, then private info hidden
// (it pulses), then copyrighted music left out too (it glows as well), then both off again.
function streamerPaint() {
  const on = !!prefs.streamerMode;
  document.body.classList.toggle("streamer", on);
  const b = $("streamerBtn");
  const safe = typeof streamSafe === "function" && streamSafe();
  b.classList.toggle("on", on || safe);
  b.classList.toggle("safe", safe);
  b.setAttribute("aria-pressed", on || safe ? "true" : "false");
  b.title = safe ? "Streamer Mode, Extended: private info hidden, and only music that's safe on stream. Click to turn it off."
    : on ? "Streamer Mode: private info hidden. Click again for Extended (only music that's safe on stream)."
    : "Streamer Mode: off. Click to hide private info; again for Extended (only music that's safe on stream).";
  $("smPrivate").checked = on;
  // Extended only with Streamer Mode, and what counts as safe only with Extended
  $("smSafeRow").hidden = !on;
  $("clearedBox").hidden = !on || !safe;
  stateNotes();
  markSensitive(document);
}

// What a bot's steps show that's yours alone: an address, a port, a key, an IP
function markSensitive(root) {
  root.querySelectorAll(".botHelp code, .botRemote code").forEach((c) => {
    if (/:\/\/|\bkey=|\b\d{1,3}(\.\d{1,3}){3}\b|^\d{2,5}$|THIS-PC/.test(c.textContent)) c.classList.add("sensitive");
  });
}
// "a", "a and b", "a, b and c"
const andList = (l) => l.length < 2 ? l.join("") : l.slice(0, -1).join(", ") + " and " + l[l.length - 1];
const SVGNS = "http://www.w3.org/2000/svg";
// rule: "evenodd" for shapes with holes cut out (like Spotify's mark)
function icon(d, rule) {
  const s = document.createElementNS(SVGNS, "svg");
  s.setAttribute("viewBox", "0 0 24 24");
  const p = document.createElementNS(SVGNS, "path");
  p.setAttribute("d", d);
  if (rule) p.setAttribute("fill-rule", rule);
  s.append(p);
  return s;
}
// (a track from a plugin that's gone shows with a plain plugin icon)
const platInfo = (key) => PLATFORMS[key] || { name: "Plugin", icon: "M20.5 11H19V7c0-1.1-.9-2-2-2h-4V3.5C13 2.12 11.88 1 10.5 1S8 2.12 8 3.5V5H4c-1.1 0-1.99.9-1.99 2v3.8H3.5c1.49 0 2.7 1.21 2.7 2.7s-1.21 2.7-2.7 2.7H2V20c0 1.1.9 2 2 2h3.8v-1.5c0-1.49 1.21-2.7 2.7-2.7 1.49 0 2.7 1.21 2.7 2.7V22H17c1.1 0 2-.9 2-2v-4h1.5c1.38 0 2.5-1.12 2.5-2.5S21.88 11 20.5 11z" };
const platIcon = (key) => icon(platInfo(key).icon, platInfo(key).rule);
function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}
// Status and error messages slide out in a strip above the player bar, then go again.
// "Loading..." style messages stay until they're replaced or cleared; say("") clears.
// say(msg, { stay: true, actions: [[label, fn], ...] }): stays until it's dismissed (a click on it, or
// one of its buttons), and the passing messages of what plays next don't replace it.
let statusTimer = null;
let statusStays = false;
function say(msg, opts) {
  const bar = $("statusBar");
  msg = msg ? String(msg) : "";
  opts = opts || {};
  if (statusStays && !opts.stay && !opts.force) return;
  clearTimeout(statusTimer);
  statusStays = !!opts.stay && !!msg;
  bar.classList.toggle("stays", statusStays);
  $("statusActions").replaceChildren(...(msg && opts.actions ? opts.actions : []).map(([label, fn]) => {
    const b = el("button", "soft", label);
    b.type = "button";
    b.addEventListener("click", (e) => { e.stopPropagation(); say("", { force: true }); fn(); });
    return b;
  }));
  if (!msg) { bar.classList.remove("show"); return; }
  const busy = /(\.\.\.|\u2026)$/.test(msg);
  const bad = !busy && (!!opts.bad || /^(could not|couldn't|can't|error|stream error|scrobble failed)|failed|problem|expired|isn't available/i.test(msg));
  $("statusText").textContent = msg;
  bar.classList.toggle("busy", busy);
  bar.classList.toggle("bad", bad);
  bar.classList.add("show");
  if (!statusStays) statusTimer = setTimeout(() => bar.classList.remove("show"), busy ? 30000 : bad ? 7000 : 4000);
}
// With YouTube Music on, SoundCloud lists include Go+ tracks too: they play from YouTube Music
// (once: SoundCloud's next-page links already carry it)
const api = (path) => invoke("api_get", { path: ytOn() ? path.replace(/access=playable(?=&|$)/, "access=playable,preview") : path });

const ICON_PLAY = "M8 5v14l11-7z";
const ICON_ADD = "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z";
const ICON_NEXT = "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z";
const ICON_HEART = "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z";
const ICON_HEART_OUT = "M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z";
const ICON_OPEN = "M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z";
const ICON_PLIST = "M14 10H2v2h12v-2zm0-4H2v2h12V6zm4 8v-4h-2v4h-4v2h4v4h2v-4h4v-2h-4zM2 16h8v-2H2v2z";
const ICON_CLOSE = "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z";

// The seek bar, and the thin bar that stands in for it when the window is narrow
function paintProgress(frac) {
  const f = Math.max(0, Math.min(1, frac || 0));
  $("seek").value = String(Math.round(f * 1000));
  if (miniDrag === null) {
    $("miniBar").style.width = (f * 100).toFixed(2) + "%";
    $("miniKnob").style.left = (f * 100).toFixed(2) + "%";
  }
}

// The progress line: click or drag along it to move through the track (the seek bar, which it stands
// in for, does the moving). While dragging it follows the pointer, and it moves there when let go.
let miniDrag = null;
function miniAt(e) {
  const r = $("miniProg").getBoundingClientRect();
  return Math.max(0, Math.min(1, (e.clientX - r.left) / Math.max(1, r.width)));
}
function miniShow(f) {
  $("miniBar").style.width = (f * 100).toFixed(2) + "%";
  $("miniKnob").style.left = (f * 100).toFixed(2) + "%";
}
function miniTip(f) {
  const t = curTrack(), tip = $("miniTime");
  tip.style.left = (f * 100).toFixed(2) + "%";
  tip.textContent = t ? fmt(f * viewDur(t, curDurMs() || t.duration || 0)) : "";
}
function miniSeek(f) {
  $("seek").value = String(Math.round(f * 1000));
  $("seek").dispatchEvent(new Event("input"));
  $("seek").dispatchEvent(new Event("change"));
}
$("miniProg").addEventListener("pointerdown", (e) => {
  if (e.button !== 0 || !curTrack()) return;
  $("miniProg").setPointerCapture(e.pointerId);
  $("miniProg").classList.add("drag");
  miniDrag = miniAt(e);
  miniShow(miniDrag);
  miniTip(miniDrag);
});
$("miniProg").addEventListener("pointermove", (e) => {
  const f = miniAt(e);
  miniTip(f);
  if (miniDrag !== null) { miniDrag = f; miniShow(f); }
});
$("miniProg").addEventListener("pointerup", () => {
  if (miniDrag === null) return;
  const f = miniDrag;
  miniDrag = null;
  $("miniProg").classList.remove("drag");
  miniSeek(f);
});
$("miniProg").addEventListener("keydown", (e) => {
  const d = curDurMs();
  if (!d || (e.key !== "ArrowLeft" && e.key !== "ArrowRight")) return;
  e.preventDefault();
  miniSeek(Math.max(0, Math.min(1, Number($("seek").value) / 1000 + (e.key === "ArrowRight" ? 5000 : -5000) / d)));
});

function fmt(ms) {
  const s = isFinite(ms) ? Math.max(0, Math.floor(ms / 1000)) : 0;
  return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0");
}
function artUrl(t, size) {
  const u = t.artwork_url || (t.user && t.user.avatar_url);
  return u ? u.replace("-large", "-" + size) : "";
}
// Round-robin: the first of each list, then the second of each, and so on
function interleave(lists) {
  const out = [];
  const longest = Math.max(0, ...lists.map((l) => l.length));
  for (let i = 0; i < longest; i++) lists.forEach((l) => { if (i < l.length) out.push(l[i]); });
  return out;
}
function shuffleInPlace(a) {
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}
// next_href is a full URL; the Rust side only wants the path and query
function toPath(href) {
  try {
    const u = new URL(href);
    return u.hostname === "api.soundcloud.com" ? u.pathname + u.search : "";
  } catch (e) { return ""; }
}

// ---------- platforms ----------
const PLATFORMS = {
  soundcloud: { name: "SoundCloud", icon: "M19.35 10.04C18.67 6.59 15.64 4 12 4 9.11 4 6.6 5.64 5.35 8.04 2.34 8.36 0 10.91 0 14c0 3.31 2.69 6 6 6h13c2.76 0 5-2.24 5-5 0-2.64-2.05-4.78-4.65-4.96z" },
  spotify: { name: "Spotify", icon: "M12 2a10 10 0 1 0 0 20 10 10 0 1 0 0-20zM6.6 9.1c3.5-1.1 7.9-.8 11 1l-.9 1.5c-2.7-1.6-6.6-1.9-9.6-.9zM7.3 12.6c2.9-.8 6.3-.5 8.8 1l-.8 1.3c-2.2-1.3-5.2-1.6-7.6-.9zM8 15.8c2.2-.5 4.5-.3 6.4.8l-.7 1.1c-1.6-.9-3.6-1.1-5.4-.7z", rule: "evenodd" },
  xpl: { name: "Playlist on this PC", icon: "M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z" },
  local: { name: "Local files", icon: "M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z" },
  youtube: { name: "YouTube Music", icon: "M5 3h14a3 3 0 0 1 3 3v9a3 3 0 0 1-3 3H5a3 3 0 0 1-3-3V6a3 3 0 0 1 3-3zm0 2.2a.8.8 0 0 0-.8.8v9c0 .44.36.8.8.8h14a.8.8 0 0 0 .8-.8V6a.8.8 0 0 0-.8-.8zM8 20h8v1.8H8z", rule: "evenodd" },   // a screen, with nothing in it: not a play button
  apple: { name: "Apple Music", icon: "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" },
  amazon: { name: "Amazon Music", icon: "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" },
};
const srcOf = (t) => (t && t.source) || "soundcloud";
// Likes are saved to SoundCloud, your Spotify library, or (YouTube Music) this PC; local files can only be favourited
const canLike = (t) => !!t && (srcOf(t) === "soundcloud" || srcOf(t) === "youtube" || (srcOf(t) === "spotify" && !!t.uri));
// "Open on SoundCloud", "Open in Spotify", "Open in YouTube Music"
const openLabel = (t) => (srcOf(t) === "spotify" ? "Open in Spotify" : srcOf(t) === "youtube" ? "Open in YouTube Music"
  : srcOf(t).startsWith("plug:") ? "Open on " + platInfo(srcOf(t)).name : "Open on SoundCloud");

function platformBadge(t) {
  const key = srcOf(t);
  const p = platInfo(key);
  const b = el("span", "plat " + key);
  b.title = "Playing from " + p.name;
  b.setAttribute("aria-label", p.name);
  b.append(platIcon(key));
  return b;
}

// The bigger platform icon left of a track's title, artist and album: it lights up in the platform's
// own colour on hover, and opens the uploader's page (who put it there), whatever the credits say
function platformButton(t) {
  const b = platformBadge(t);
  b.classList.add("platBig");
  const up = t ? artistOfTrack(t).name : "";
  b.title = platInfo(srcOf(t)).name + (up ? ". Click to open " + up + "'s page" : "");
  b.setAttribute("role", "button");
  return b;
}

// A track's artists: "A, B & C" (or "A x B", "A feat. B", "A ft. B", "A (feat. B)") is three, each a
// link to their own page; one is the usual link. The line keeps its own commas, ampersands, x's and
// feat.s. An X that ends a name ("Lil Nas X & ...") isn't one: it's followed by another separator, or
// nothing. "ft" inside a name ("Daft Punk", "Swift") isn't either: it has to start a word after
// another name ("Ft Lauderdale" at the start is a name).
const ARTIST_SEP = /(\s*,\s*|\s+&\s+|\s+[x×]\s+(?![&,]|$)|(?:\s+\(?|\s*\()(?:feat\.?|ft\.?|featuring)\s+)/i;
// a bracketed feat.'s closing bracket isn't part of the name (a name's own brackets are)
const artistPart = (part) => {
  const p = part.trim();
  return p.endsWith(")") && !p.includes("(") ? { name: p.slice(0, -1).trim(), close: ")" } : { name: p, close: "" };
};
// Names with "&" in them that are one artist, not two ("Pete & Bas", "Above & Beyond"): what Spotify and
// YouTube Music credit as one artist (they list several with commas, so an "&" inside one is part of the
// name), from this track and every one seen so far, a few well known ones, and "X & the Y" bands
const ARTIST_WHOLE = new Set(["pete & bas", "above & beyond", "chase & status", "simon & garfunkel", "hall & oates", "mumford & sons",
  "belle & sebastian", "brooks & dunn", "camo & krooked", "dimitri vegas & like mike", "drumsound & bassline smith", "krept & konan",
  "nico & vinz", "angus & julia stone", "matt & kim", "sam & dave", "ike & tina turner", "iron & wine", "macklemore & ryan lewis",
  "sunnery james & ryan marciano", "aly & fila", "bob & earl", "zion & lennox", "wisin & yandel"]);
function artistLearn(t) {
  const src = srcOf(t);
  const names = src === "spotify" ? (t.spArtists || []).map((a) => a.name)
    : src === "youtube" ? String((t.user && t.user.username) || "").split(/\s*,\s*/) : [];
  names.forEach((n) => { if (n && n.includes("&")) ARTIST_WHOLE.add(n.toLowerCase().trim()); });
}
// The artist line in pieces: [name, separator, name, ...], with whole names kept together
function artistSplit(t) {
  artistLearn(t);
  const parts = String(artistName(t) || "").split(ARTIST_SEP);
  for (let i = 1; i < parts.length - 1; i += 2) {
    if (!/^\s+&\s+$/.test(parts[i])) continue;
    const a = parts[i - 1].trim(), b = parts[i + 1].trim();
    const whole = (a + " & " + b).toLowerCase();
    if (ARTIST_WHOLE.has(whole) || /^the\s/i.test(b)) {
      parts.splice(i - 1, 3, a + " & " + b);
      i -= 2;
    }
  }
  return parts;
}
const artistNames = (t) => artistSplit(t).filter((x, i) => i % 2 === 0 && x.trim()).map((x) => artistPart(x).name).filter(Boolean);

function fillArtists(box, t) {
  box.replaceChildren();
  if (!t) return;
  const names = artistNames(t);
  if (names.length < 2) {
    const s = el("span", "artistPart", artistName(t));
    s.title = uploaderNote(t);
    box.append(s);
    return;
  }
  artistSplit(t).forEach((part, i) => {
    if (i % 2) { box.append(part); return; }   // the comma, " & " or " feat. " between them
    const { name: n, close } = artistPart(part);
    if (!n) return;
    const s = el("span", "artistPart", n);
    s.dataset.name = n;
    s.title = "Open " + n + "'s page";
    box.append(s);
    if (close) box.append(close);
  });
}

// The page for one of a track's artists (by name, with its Spotify id when there is one)
function openArtistNamed(t, name) {
  const sp = (t.spArtists || []).find((a) => a.name === name);
  openArtist(artistFrom({ name, sp: sp && sp.id ? { id: sp.id, name } : undefined }));
}

// A click in an artist line: the artist clicked, or the track's artist
function openArtistClicked(t, target) {
  const part = target && target.closest && target.closest(".artistPart[data-name]");
  if (part) openArtistNamed(t, part.dataset.name);
  else openArtistFor(t);
}

// ---------- state ----------
let viewTracks = [];      // tracks shown in the main list
let nextHref = "";        // path for "Load more"
let view = "likes";       // likes | search | playlist | artist | discover | rediscover | stats | now
let back = "likes";       // list view to return to from Now Playing
let query = "";
let playlists = [];
let currentPlaylist = null;
let queue = [];           // tracks that will play, in order
let qi = -1;              // index of the current track in the queue

const curTrack = () => (qi >= 0 ? queue[qi] : null);

// Whether each track is liked. Filled from the likes list and from the
// "user_favorite" flag when the API provides it; updated when you like or unlike.
const likeState = new Map();
const isLiked = (t) => !!t && (likeState.get(t.id) === true || (t.dupes || []).some((d) => likeState.get(d.id) === true));
let userName = "";
let meId = null;
let currentArtist = null;
let discoverSrc = "";

// ---------- views ----------
function showView(v) {
  if (v !== view && typeof pageFilterClear === "function") pageFilterClear();
  // each page opens on the filter last picked there
  if (!restoring && v !== view) listFilter = (prefs.pageFilters && prefs.pageFilters[v]) || "all";
  view = v;
  const isNow = v === "now";
  if (!isNow) back = v;
  $("listView").hidden = isNow || v === "stats" || (v === "search" && !query);
  $("searchPane").hidden = v !== "search";
  $("searchPane").classList.toggle("empty", v === "search" && !query);
  $("searchForm").style.visibility = v === "search" ? "hidden" : "";
  renderListChips();
  $("navRediscover").classList.toggle("active", v === "rediscover");
  navTrack(v);
  $("npView").hidden = !isNow;
  $("statsView").hidden = v !== "stats";
  $("navStats").classList.toggle("active", v === "stats");
  if (v === "stats") renderStats();
  if (isNow) ensureLyrics();
  else if (lyricsMode) setLyricsMode(false);
  $("navDiscover").classList.toggle("active", v === "discover");
  $("navStreamer").classList.toggle("active", v === "streamer");
  $("feed").hidden = v !== "discover" && v !== "streamer";
  if (v !== "search") $("searchShelves").hidden = true;
  $("list").hidden = v === "discover" || v === "streamer";
  if (!isNow) $("langChip").hidden = v !== "discover";
  $("navLikes").classList.toggle("active", v === "likes");
  $("navFavourites").classList.toggle("active", v === "favourites");
  $("navSets").classList.toggle("active", v === "sets");
  $("navArtists").classList.toggle("active", v === "artists");
  $("artistsPage").hidden = v !== "artists";
  document.querySelectorAll(".plItem").forEach((b) => {
    b.classList.toggle("active", v === "playlist" && currentPlaylist && String(b.dataset.id) === String(currentPlaylist.id));
  });
}

function renderHero() {
  const v = view === "now" ? back : view;
  let title = "Library";
  let sub = v === "likes" ? "Everything you've liked on each platform, plus your music files, in one list." : "";
  if (v === "rediscover") {
    title = "Rediscover";
    sub = "";   // set below, in two parts
  } else if (v === "sets") {
    title = "Sets & Podcasts";
    sub = "Long sets, mixes and podcasts: anything over 15 minutes.";
  } else if (v === "artists") {
    title = "Artists";
    sub = "Everyone you follow on " + ARTIST_PLATS.filter((p) => activePlats().includes(p)).map((p) => PLATFORMS[p].name).join(", ") +
      ". Right-click an artist to follow them on your other platforms too.";
  } else if (v === "favourites") {
    title = "Favourites";
    sub = "Starred tracks from every platform, kept on this PC.";
  } else if (v === "discover") {
    title = "Discover";
    sub = "Mixes and stations made from what you like.";
  } else if (v === "streamer") {
    title = "Streamer Mode";
    sub = "Only music that's safe on stream: openly licensed, or by artists and labels cleared for streaming. Autoplay adds from these collections too.";
  } else if (v === "search") {
    title = 'Results for "' + query + '"';
  } else if (v === "playlist" && currentPlaylist) {
    title = currentPlaylist.title || "Playlist";
    if (currentPlaylist.source === "mix") sub = currentPlaylist.desc || "";
    else if (isXpl(currentPlaylist)) sub = "On this PC. Holds tracks from any platform.";
    else if (currentPlaylist.user && currentPlaylist.user.username) sub = "By " + currentPlaylist.user.username + ". ";
  } else if (v === "artist" && currentArtist) {
    title = currentArtist.name || currentArtist.username || "Artist";
  }
  const n = viewTracks.length;
  $("viewTitle").textContent = title;
  // pages made from what the platforms send: fetched again with the refresh button
  $("refreshBtn").hidden = !["discover", "rediscover", "sets", "artists", "streamer"].includes(v);
  $("viewSub").textContent = sub.trim();
  if (v === "artist" && currentArtist && currentArtist.on) $("viewSub").replaceChildren(...artistSubline(currentArtist));
  if (v === "rediscover") {
    $("viewSub").replaceChildren(el("span", "subLead", "Fall in love all over again"),
      el("span", "", " with the likes you haven't heard in a while."));
  }
  const isArtist = v === "artist" && !!currentArtist;
  $("playAll").hidden = n === 0;
  $("shuffleAll").hidden = n === 0;
  $("artistsBatch").hidden = v !== "artists";
  $("followBtn").hidden = !isArtist || !followPlats(currentArtist).length ||
    (meId !== null && currentArtist.on && currentArtist.on.soundcloud && String(currentArtist.on.soundcloud.id) === String(meId));
  if ($("followBtn").hidden) $("followMoreBtn").hidden = true; else paintFollow();   // "Also follow on ...", now Follow shows
  $("plDelete").hidden = !(v === "playlist" && isOwnPlaylist(currentPlaylist));
  $("plShare").hidden = !(v === "playlist" && currentPlaylist && currentPlaylist.source !== "local");
  const av = $("heroAvatar");
  const src = v === "artist" ? artistImage(currentArtist) : "";
  if (src) { av.src = src; av.hidden = false; } else { av.removeAttribute("src"); av.hidden = true; }
  paintHeroCover(v);
  paintHeroActions();
}

// ---------- cover art for playlists, albums and Favourites ----------
// An album shows its own cover. A playlist (and Favourites) shows up to four different covers from
// its tracks in one square, split diagonally: two halves, a "Y" of three, or an "X" of four.
const COVER_WEDGES = {
  1: ["polygon(0 0, 100% 0, 100% 100%, 0 100%)"],
  2: ["polygon(0 0, 100% 0, 0 100%)", "polygon(100% 0, 100% 100%, 0 100%)"],
  3: ["polygon(0 0, 100% 0, 50% 50%)", "polygon(0 0, 50% 50%, 50% 100%, 0 100%)", "polygon(100% 0, 100% 100%, 50% 100%, 50% 50%)"],
  4: ["polygon(0 0, 100% 0, 50% 50%)", "polygon(100% 0, 100% 100%, 50% 50%)", "polygon(0 100%, 100% 100%, 50% 50%)", "polygon(0 0, 0 100%, 50% 50%)"],
};

const isAlbumPage = (p) => !!p && (!!p.album || !!p.spAlbumId || !!p.ytAlbumId || isScAlbum(p));

function heroCovers(v) {
  if (v === "playlist" && isAlbumPage(currentPlaylist)) {
    const own = currentPlaylist.artwork_url ? artUrl(currentPlaylist, "t500x500") : "";
    const first = viewTracks.map((t) => artUrl(t, "t500x500")).find(Boolean);
    return [own || first].filter(Boolean);
  }
  if (v !== "playlist" && v !== "favourites") return [];
  const seen = new Set();
  for (const t of viewTracks) {
    const u = artUrl(t, "t500x500");
    if (u && !seen.has(u)) seen.add(u);
    if (seen.size === 4) break;
  }
  return [...seen];
}

function paintHeroCover(v) {
  const box = $("heroCover");
  const urls = heroCovers(v);
  const key = urls.join("|");
  box.hidden = !urls.length;
  if (box.dataset.key === key) return;   // unchanged: don't reload the images
  box.dataset.key = key;
  box.replaceChildren(...urls.map((u, i) => {
    const img = el("img");
    img.alt = "";
    img.src = u;
    img.style.clipPath = COVER_WEDGES[urls.length][i];
    img.onerror = () => { img.removeAttribute("src"); };
    return img;
  }));
}

// Go+ tracks are served to this app as 30-second previews only
function isPreview(t) { return !!t && (t.access === "preview" || t.policy === "SNIP"); }
// ---------- Streamer Mode: Stream-Safe Music ----------
// On, only tracks under an open licence are shown and played: SoundCloud's Creative Commons licences
// ("cc-by", "cc-by-nc-sa" and the rest) and "no-rights-reserved". Nothing else says how it's licensed
// (Spotify, YouTube Music, your files, plugins without a licence), so none of it counts as open.
const streamSafe = () => !!prefs.streamSafe;
const openLicence = (t) => !!t && /^(cc-|cc0$|no-rights-reserved$)/i.test(String(t.license || ""));
const safeOk = (t) => !streamSafe() || streamOk(t);   // (js/streamsafe.js: open licences, and what's cleared for streaming)

function safePaint() {
  const on = streamSafe();
  document.body.classList.toggle("streamSafe", on);
  $("smSafe").checked = on;
  streamerPaint();
}

// The button's next step: off -> private info -> and safe music -> off
function streamerStep() {
  const priv = !!prefs.streamerMode, safe = streamSafe();
  if (!priv && !safe) {
    prefs.streamerMode = true;
    savePrefs();
    streamerPaint();
    say("Streamer Mode: account names, keys, addresses and plugin settings are hidden. Click again to leave out copyrighted music too.", { force: true });
  } else if (priv && !safe) {
    safeSet(true);
  } else {
    prefs.streamerMode = false;
    savePrefs();
    safeSet(false);
    say("Streamer Mode off.", { force: true });
  }
}

function safeSet(on) {
  prefs.streamSafe = !!on;
  savePrefs();
  safePaint();
  streamerPageSync(!!on);   // the Streamer Mode page in place of the others (js/streamerpage.js)
  autoplayPaint();
  render();
  renderQueue();
  say(on ? "Streamer Mode: only music that's safe on stream shows and plays" + (prefs.streamerMode ? ", and private info is hidden." : ".") : "Stream-Safe Music off.", { force: true });
  streamLookSoon();
  // what's playing now, if it isn't open, goes
  const cur = curTrack();
  if (on && cur && !streamOk(cur)) safeSkip(cur);
}

// Not open, with Stream-Safe Music on: on to the next one that is (or stop)
function safeSkip(t) {
  audio.pause();
  if (typeof spotifyPause === "function") spotifyPause();
  const next = queue.findIndex((x, k) => k > qi && streamOk(x));
  if (next > 0) { qi = next; playCurrent(); return; }
  if (autoplayActive()) { qi = queue.length - 1; autoplayAdvance(); return; }
  say('Stream-Safe Music: "' + (t.title || "That track") + "\" isn't safe on stream, and nothing after it in the queue is.", { force: true });
}

// Go+ previews and blocked tracks are never shown.
// Go+ tracks are shown when YouTube Music (or Spotify Premium) can play them in full, unless you've
// hidden one that had no full version anywhere.
const spFullOn = () => typeof spotifyPremium !== "undefined" && spotifyPremium && prefs.spOn !== false;
const noVersionHidden = (t) => !!t && !!prefs.noVersion && !!prefs.noVersion[String(t.id)];
function isUnplayable(t) {
  return noVersionHidden(t) || (isPreview(t) && !ytOn() && !spFullOn()) || (!!t && t.access === "blocked") || (!!t && /\bpreview\b/i.test(t.title || ""));
}

function openOnSoundCloud(t) {
  if (t && t.permalink_url) invoke("open_url", { url: t.permalink_url }).catch((e) => say(String(e)));
}

// ---------- track list ----------
function render() {
  $("list").replaceChildren(...viewTracks.map((t, i) => trackRow(t, i, curTrack())));
  afterRender();
}

// Infinite scroll: add only the rows from "from" onwards instead of rebuilding the list
function renderAppend(from) {
  const ul = $("list");
  ul.querySelectorAll(".albumTile").forEach((x) => x.remove());   // re-spaced by injectAlbums
  const playing = curTrack();
  ul.append(...viewTracks.slice(from).map((t, k) => trackRow(t, from + k, playing)));
  afterRender();
}

// ---------- versions of a song, beneath its row (artist pages: js/artists.js groupVersions) ----------
const versionsOpen = new Set();   // the songs whose versions are open, by id
let versionPool = [];             // the version rows' tracks (their rows say which by data-v)

function versionsPanel(i) {
  const t = viewTracks[i];
  const li = el("li", "versionsPanel");
  li.dataset.p = i;
  const ul = el("ul", "versionsList");
  const playing = curTrack();
  t._versions.forEach((v) => {
    const row = trackRow(v, i, playing);
    delete row.dataset.i;
    row.draggable = false;
    row.dataset.v = versionPool.push(v) - 1;
    row.dataset.p = i;
    row.querySelector(".num .n").textContent = "";
    ul.append(row);
  });
  const more = el("div", "versionsMore");
  const b = el("button", "soft", "More versions");
  b.title = "Search everywhere for this song";
  b.addEventListener("click", () => openSearch(((guessNames(t).artist || artistName(t)) + " " + versionBaseTitle(t)).trim()));
  more.append(b);
  li.append(ul, more);
  return li;
}
// The song's own name, for More versions' search ("Song", not "Song (Radio Edit)")
const versionBaseTitle = (t) => versionStrip(versionTitle(t)).replace(/\s+/g, " ").trim();

function versionsPaint() {
  const ul = $("list");
  ul.querySelectorAll(".versionsPanel").forEach((x) => x.remove());
  versionPool = [];
  if (view !== "artist") return;
  ul.querySelectorAll("li[data-i]").forEach((li) => {
    const t = viewTracks[Number(li.dataset.i)];
    if (t && t._versions && versionsOpen.has(String(t.id))) li.after(versionsPanel(Number(li.dataset.i)));
  });
}

function versionsToggle(i) {
  const t = viewTracks[i];
  if (!t || !t._versions) return;
  const id = String(t.id);
  if (versionsOpen.has(id)) versionsOpen.delete(id); else versionsOpen.add(id);
  const row = $("list").querySelector('li[data-i="' + i + '"]');
  if (row) { const b = row.querySelector(".versionsBtn"); if (b) b.classList.toggle("open", versionsOpen.has(id)); }
  versionsPaint();
}

// A version, played in the page's list in the song's place
function playVersion(p, v) {
  const list = viewTracks.slice();
  list[p] = v;
  startList(list, p, false);
}

function afterRender() {
  versionsPaint();
  $("tableHead").hidden = viewTracks.length === 0;
  paintTrackHead();
  $("more").hidden = !nextHref;
  renderHero();
  refreshLikeUI();
  renderPlatChips();
  injectAlbums();
  paintHeroActions();
  spotifyLikeCheck(viewTracks);
  streamLookSoon();   // labels and licences, for Stream-Safe Music (js/streamsafe.js)
  if (pageQuery) pageFilterApply();   // the search bar's filter on this page (js/pagefind.js)
}

// The row under the title: play / shuffle and page buttons, with the view options on the right.
// Shown whenever any of them is, so the language filter stays reachable when it hid everything.
function paintHeroActions() {
  $("luckyBtn").hidden = view !== "discover";
  $("libSort").hidden = view !== "likes" || albumsOnly();
  $("favOnlyChip").hidden = !favOn() || ["favourites", "discover", "stats", "now", "artists"].includes(view) || (view === "search" && !query);
  paintSetsChip();
  const hl = $("hideLikesChip");
  hl.hidden = view !== "search" || !query;
  hl.classList.toggle("on", !!prefs.hideLikes);
  hl.setAttribute("aria-pressed", prefs.hideLikes ? "true" : "false");
  hl.title = prefs.hideLikes ? "Your likes are hidden from these results. Click to show them." : "Leave tracks you've liked out of the results";
  $("heroActions").hidden = !["playAll", "followBtn", "plDelete", "plShare", "luckyBtn", "libSort", "langChip", "favOnlyChip", "setsLikedChip", "hideLikesChip", "artistsBatch"].some((id) => !$(id).hidden);
}

function trackRow(t, i, playing) {
  applyFix(t);   // your own title and artist, whichever copy this is (js/titles.js)
  const li = el("li");
  li.dataset.i = i;
  li.draggable = true;   // onto a playlist or the queue (js/dnd.js)
  if (!isFav(t)) li.classList.add("notFav");   // hidden while "Favourites only" is on
  if (!streamOk(t)) li.classList.add("notSafe");   // hidden while Stream-Safe Music is on
  if (playing && t.id === playing.id) li.classList.add("playing");

  const num = el("div", "num");
  num.append(el("span", "n", String(i + 1)), icon(ICON_PLAY));

  const img = el("img");
  img.loading = "lazy";
  img.alt = "";
  const src = artUrl(t, "t120x120");
  if (src) img.src = src;
  img.onerror = () => img.removeAttribute("src");

  const meta = el("div", "meta");
  const artistLine = el("div", "artist");
  const whoEl = el("span", "artistLink");
  fillArtists(whoEl, t);
  artistLine.append(whoEl);
  if (srcOf(t) === "soundcloud" && isPreview(t)) {
    const tag = el("span", "goPlus", "Go+");
    tag.title = "A Go+ track: the full song plays from YouTube Music";
    artistLine.insertBefore(tag, whoEl);
  }
  meta.append(el("div", "title", t.title || "Untitled"), artistLine);
  // other versions of it (artist pages): opened beneath the row
  if (t._versions && t._versions.length) {
    const vb = el("button", "versionsBtn" + (versionsOpen.has(String(t.id)) ? " open" : ""), t._versions.length + (t._versions.length === 1 ? " more version" : " more versions"));
    vb.dataset.act = "versions";
    vb.title = "Its other versions: remixes, edits and the like";
    artistLine.append(vb);
  }
  const cell = el("div", "trackCell");
  cell.append(img, platformButton(t), meta);

  const acts = el("div", "acts");
  const bNext = el("button");
  bNext.dataset.act = "next";
  bNext.title = "Play next";
  bNext.setAttribute("aria-label", "Play next");
  bNext.append(icon(ICON_NEXT));
  const bAdd = el("button");
  bAdd.dataset.act = "add";
  bAdd.title = "Add to queue";
  bAdd.setAttribute("aria-label", "Add to queue");
  bAdd.append(icon(ICON_ADD));
  const bLike = el("button");
  bLike.dataset.act = "like";
  bLike.setAttribute("aria-label", "Like");
  setHeart(bLike, isLiked(t));
  const bOpen = el("button");
  bOpen.dataset.act = "open";
  bOpen.title = openLabel(t);
  bOpen.setAttribute("aria-label", bOpen.title);
  bOpen.append(icon(ICON_OPEN));
  const bList = el("button");
  bList.dataset.act = "plist";
  bList.title = "Add to playlist";
  bList.setAttribute("aria-label", "Add to playlist");
  bList.append(icon(ICON_PLIST));
  // Liking is SoundCloud-only; every track can go in a playlist (on this PC, if not SoundCloud)
  const bFav = el("button");
  bFav.dataset.act = "fav";
  setStar(bFav, isFav(t));
  acts.append(canLike(t) ? bLike : el("span", "actGap"));   // keeps the stars lined up
  acts.append(bFav, bNext, bAdd, bList);
  if (view === "playlist" && isOwnPlaylist(currentPlaylist)) {
    const bRm = el("button");
    bRm.dataset.act = "unlist";
    bRm.title = "Remove from this playlist";
    bRm.setAttribute("aria-label", bRm.title);
    bRm.append(icon(ICON_CLOSE));
    acts.append(bRm);
  } else if (t.permalink_url) acts.append(bOpen);

  li.append(num, cell, acts, el("div", "time", fmt(viewDur(t, t.duration || 0)) + (trimOf(t) ? "*" : "")));
  return li;
}

function markPlaying() {
  const playing = curTrack();
  document.querySelectorAll("#list li").forEach((li) => {
    const t = viewTracks[Number(li.dataset.i)];
    li.classList.toggle("playing", !!(playing && t && t.id === playing.id));
  });
}

$("list").addEventListener("click", (e) => {
  const li = e.target.closest("li");
  if (!li || li.classList.contains("versionsPanel")) return;
  // (a version's row, beneath its song's: its own track)
  const isVersion = li.dataset.v !== undefined;
  const i = Number(isVersion ? li.dataset.p : li.dataset.i);
  const t = isVersion ? versionPool[Number(li.dataset.v)] : viewTracks[i];
  const btn = e.target.closest("button[data-act]");
  if (btn) {
    if (btn.dataset.act === "versions") versionsToggle(i);
    else if (btn.dataset.act === "like") toggleLike(t);
    else if (btn.dataset.act === "fav") toggleFav(t);
    else if (btn.dataset.act === "next") playNext(t);
    else if (btn.dataset.act === "plist") openPlaylistMenu(t, btn);
    else if (btn.dataset.act === "unlist") removeFromPlaylist(t);
    else if (btn.dataset.act === "open") openOnSoundCloud(t);
    else addToQueue(t);
    return;
  }
  if (e.target.closest(".platBig")) {
    openArtistFor(t);   // the uploader
    return;
  }
  if (e.target.closest(".artistLink")) {
    openArtistClicked(t, e.target);
    return;
  }
  if (isVersion) playVersion(i, t); else playFromPage(i);
});

// ---------- loading ----------
const LIKES_PATH = "/me/likes/tracks?limit=50&access=playable&linked_partitioning=true";

function asList(data) {
  const items = Array.isArray(data) ? data : (data && data.collection) || [];
  return items.map((x) => x.track || x).filter((t) => t && t.id && !isUnplayable(t)).map(tidyTrack);
}

let loadToken = 0;   // lets a newer request win over a slower, older one

// "paths" is one API path, or a list of alternatives: the first one that returns tracks is used
async function loadTracks(paths, append, fallback) {
  const tok = ++loadToken;
  say("Loading...");
  const options = Array.isArray(paths) ? paths : [paths];
  let data = null;
  let used = options[0];
  let err = null;
  for (const p of options) {
    try {
      const d = await api(p);
      if (!data) { data = d; used = p; }
      if (asList(d).length) { data = d; used = p; err = null; break; }
    } catch (e) {
      err = e;
    }
  }
  if (tok !== loadToken) return;
  const shown = viewTracks.length;

  if (data) {
    let got = asList(data);
    if (view === "discover") got = recFilter(got);
    if (hideMixesIn(view)) got = got.filter((t) => !isMix(t));
    if (LIST_FILTERS[view]) got = applyListFilter(got);
    const fromLikes = used.startsWith("/me/likes/tracks");
    got.forEach((t) => {
      if (fromLikes) likeState.set(t.id, true);
      else if (typeof t.user_favorite === "boolean") likeState.set(t.id, t.user_favorite);
    });
    viewTracks = append ? viewTracks.concat(got) : got;
    nextHref = !Array.isArray(data) && data.next_href ? toPath(data.next_href) : "";
    say(viewTracks.length ? "" : "Nothing found.");
  } else if (fallback && fallback.length) {
    viewTracks = fallback.filter((t) => t && t.id && !isUnplayable(t)).map(tidyTrack);
    nextHref = "";
    say("");
  } else {
    say(String(err));
    if (!append) viewTracks = [];
  }
  if (append && data) renderAppend(shown); else render();
  if (!append) setTimeout(nearBottomCheck, 50);
}

function openLikes() {
  currentPlaylist = null;
  showView("likes");
  const plats = activePlats("likes");
  if (!plats.length) { showNothing(); return; }
  if (albumsOnly()) return;
  openLibrary(plats);
}

// Your whole library, platforms taking turns, from the likes in memory (cached or fresh)
function libraryList(plats) {
  return mergeDupes(interleave([
    plats.includes("soundcloud") ? likedTracks.filter((t) => isLikedTrack(t) && !isUnplayable(t)) : [],
    plats.includes("spotify") ? spLikedCache || [] : [],
    plats.includes("local") ? localTracks : [],
    plats.includes("youtube") ? ytLikedTracks() : [],
    ...plats.filter((p) => p.startsWith("plug:")).map((p) => pluginLibrary.get(p) || []),
  ]));
}

// Shows the list without losing your place (keeps as many rows loaded and the scroll position)
// The Library's order (its sort chips): by artist or title, either way, or neither (as you liked them)
const libSortOf = () => Object.assign({ by: "added", dir: 1 }, prefs.libSort);
const sortText = (s) => String(s || "").replace(/^the\s+/i, "").trim();
function libSorted(list) {
  const { by, dir } = libSortOf();
  if (by === "added") return list;
  const cmp = (a, b) => sortText(a).localeCompare(sortText(b), undefined, { sensitivity: "base", numeric: true });
  return list.slice().sort((x, y) => dir * (by === "artist"
    ? cmp(artistName(x), artistName(y)) || cmp(x.title, y.title)
    : cmp(x.title, y.title) || cmp(artistName(x), artistName(y))));
}

function libSortPaint() {
  const { by, dir } = libSortOf();
  document.querySelectorAll("#libSort button").forEach((b) => {
    const on = b.dataset.by === by;
    b.classList.toggle("on", on);
    b.setAttribute("aria-pressed", on ? "true" : "false");
    const arrow = b.querySelector(".sortArrow");
    if (arrow) arrow.textContent = on ? (dir > 0 ? " \u2191" : " \u2193") : "";
    b.title = on ? (dir > 0 ? "A to Z: click for Z to A" : "Z to A: click to go back to the order you liked them") : "By " + b.dataset.by;
  });
}

// a chip, clicked through: A to Z, Z to A, then off (as you liked them)
function libSortPick(by) {
  const cur = libSortOf();
  prefs.libSort = cur.by !== by ? { by, dir: 1 } : cur.dir > 0 ? { by, dir: -1 } : { by: "added", dir: 1 };
  savePrefs();
  libSortPaint();
  if (view === "likes") showLibrary(libraryList(activePlats("likes")), false);
}

function showLibrary(list, keepPlace) {
  const top = $("listView").scrollTop;
  const shown = viewTracks.length;
  setLocalPool(libSorted(applyListFilter(list)));
  if (keepPlace && shown > viewTracks.length) {
    localPos = Math.min(localPool.length, shown);
    viewTracks = localPool.slice(0, localPos);
    nextHref = localPos < localPool.length ? "local:" : "";
  }
  render();
  if (keepPlace) $("listView").scrollTop = top;
  else setTimeout(nearBottomCheck, 50);
}

async function openLibrary(plats) {
  const tok = ++loadToken;
  plats.filter((p) => p.startsWith("plug:") && !pluginLibrary.has(p)).forEach((p) =>
    pluginLibraryOf(p.slice(5)).then(() => { if (view === "likes" && tok === loadToken) showLibrary(libraryList(plats), true); }).catch(() => {}));
  const online = plats.filter((p) => p !== "local");
  // what this PC already has (only reading the saved copies, nothing downloaded yet)
  await Promise.all([
    plats.includes("soundcloud") && prefs.scOn !== false ? likesFromCache() : null,
    plats.includes("spotify") ? spLikedFromCache() : null,
  ]);
  if (tok !== loadToken || view !== "likes") return;
  const have = (plats.includes("soundcloud") && likedTracks.length) || (plats.includes("spotify") && spLikedCache && spLikedCache.length);
  if (!have && online.length) {
    // the very first time: page by page as before, while the full lists are saved for next time
    loadMerged(plats.map(likesFeed), applyListFilter);
    if (plats.includes("soundcloud")) ensureLikes();   // saves the copy for next time
    if (plats.includes("spotify")) spLiked().catch(() => null);
    return;
  }
  const before = libraryList(plats);
  showLibrary(before, false);
  if (!online.length) { say(viewTracks.length ? "" : "Nothing here yet."); return; }
  // then catch up, while the page stays usable
  say("Checking for updates...");
  await Promise.all([
    plats.includes("soundcloud") ? ensureLikes(true).catch(() => null) : null,
    plats.includes("spotify") ? spLiked(true).catch(() => null) : null,
  ]);
  if (tok !== loadToken || view !== "likes") return;
  const after = libraryList(plats);
  const same = after.length === before.length && after.every((t, i) => t.id === before[i].id && t.artwork_url === before[i].artwork_url);
  if (!same) showLibrary(after, true);
  say(same ? "" : "Library updated.");
}

function showNothing() {
  ++loadToken;
  viewTracks = [];
  nextHref = "";
  render();
  say(NOTHING_ON);
}

function openSearch(q) {
  query = q;
  $("bigQ").value = q;
  currentPlaylist = null;
  showView("search");
  const plats = activePlats("search");
  if (!plats.length) { showNothing(); return; }
  searchMixes = [];
  tracksExpanded = false;
  paintSearchShelves(q);
  if (albumsOnly()) return;
  const genre = genreOf(q);
  const tok = loadToken;
  // To leave your likes out, the app needs to know them first (both lists are cached)
  const ready = prefs.hideLikes ? loadLikesFor(plats) : Promise.resolve();
  ready.then(() => {
    if (tok !== loadToken || view !== "search" || query !== q) return;   // moved on meanwhile
    // a genre is searched as a genre on each platform; sets & podcasts get their own shelf under All
    loadMerged(plats.map((id) => (genre && id !== "local" ? genreFeed(id, genre) : searchFeed(id, q))), (list) => {
      searchSetsFound(list);
      return list.filter((t) => (listFilter === "mixes" ? isMix(t) : !isMix(t)) && !(prefs.hideLikes && isLikedTrack(t)));
    });
  });
}

// Your SoundCloud and Spotify likes, so liked tracks can be recognised in other lists
function loadLikesFor(plats) {
  if (prefs.hideLikes) say("Loading...");
  return Promise.all([
    plats.includes("soundcloud") ? ensureLikes() : null,
    plats.includes("spotify") ? spLiked().catch(() => null) : null,
  ]);
}

function openPlaylist(p) {
  currentPlaylist = p;
  showView("playlist");
  if (p.source === "mix") { openMix(p); return; }   // a Discover mix or station
  if (isXpl(p)) { ++loadToken; $("listView").scrollTop = 0; setLocalPool(xplTracks(xplFind(p.id) || { tracks: [] })); render(); say(viewTracks.length ? "" : "Nothing in here yet. Add tracks with the playlist button on any track, or drag them onto this playlist."); return; }
  if (p.source === "spotify" || p.source === "local" || p.source === "youtube") { loadPlatformPlaylist(p); return; }
  openCachedList("list:sc:" + p.id,
    () => collectPages("/playlists/" + p.id + "/tracks?access=playable&limit=200&linked_partitioning=true", 10),
    "Nothing in here yet.");
}

// ---------- discover ----------
// Discover is a feed of shelves (js/feed.js); each platform's picker (js/discover.js) supplies
// "More of what you like" and the genre mixes
const DISCOVER_PICKERS = { soundcloud: (src) => scDiscover(src), spotify: (src) => spDiscover(src), youtube: (src) => ytDiscover(src) };

function openDiscover(src, fresh) {
  if (!activePlats("discover").length) { openLikes(); return; }
  return openFeed(fresh);
}

$("navDiscover").addEventListener("click", () => openDiscover());
$("navLikes").addEventListener("click", openLikes);
// The Search tab is its own page: empty, with one big search box in the middle
function openSearchPage() {
  query = "";
  currentPlaylist = null;
  currentArtist = null;
  showView("search");
  viewTracks = [];
  nextHref = "";
  render();
  say("");
  $("bigQ").value = "";
  $("bigQ").focus();
}
// The magnifying glass in the search bar opens the Search page (or searches, if something is typed)
$("searchGo").addEventListener("click", () => {
  const q = $("q").value.trim();
  if (q) openSearch(q); else openSearchPage();
});
$("bigSearchForm").addEventListener("submit", (e) => {
  e.preventDefault();
  const q = $("bigQ").value.trim();
  if (q) openSearch(q);
});
$("hideLikesChip").addEventListener("click", () => {
  prefs.hideLikes = !prefs.hideLikes;
  savePrefs();
  if (view === "search" && query) openSearch(query);
});
$("searchForm").addEventListener("submit", (e) => {
  e.preventDefault();
  const q = $("q").value.trim();
  if (!q) return;
  pageFilterClear();
  openSearch(q);
});
// Infinite scroll: load the next page when the end of the list comes near
let loadingMore = false;
async function loadMore() {
  if (tracksCollapsed()) return;   // Search's Tracks shows its first few until Show all
  if (!nextHref || loadingMore) return;
  if (nextHref === "local:") { pageLocal(); return; }   // lists held in the app, shown 60 at a time
  loadingMore = true;
  $("more").disabled = true;
  $("more").textContent = "Loading more...";
  try {
    if (nextHref === "merged:") await pageMerged(); else await loadTracks(nextHref, true);
  } finally {
    loadingMore = false;
    $("more").disabled = false;
    $("more").textContent = "Load more";
  }
  // still near the bottom (a short page)? keep going
  const r = document.querySelector(".moreRow").getBoundingClientRect();
  const box = $("listView").getBoundingClientRect();
  if (nextHref && r.top - box.bottom < 600) loadMore();
}
// A filter that leaves the first page short (e.g. Likes > Mixes) still fills the screen
function nearBottomCheck() {
  const r = document.querySelector(".moreRow").getBoundingClientRect();
  const box = $("listView").getBoundingClientRect();
  if (nextHref && !loadingMore && r.top - box.bottom < 600) loadMore();
}
$("more").addEventListener("click", loadMore);
new IntersectionObserver((entries) => {
  if (entries.some((e) => e.isIntersecting)) loadMore();
}, { root: $("listView"), rootMargin: "600px" }).observe(document.querySelector(".moreRow"));
$("playAll").addEventListener("click", () => startList(viewTracks, 0, false));
$("shuffleAll").addEventListener("click", async () => {
  const from = view;
  const list = await shuffleSource();
  if (view === from && list.length) startList(list, 0, true);
});

// ---------- playlists (sidebar) ----------
function renderPlaylists(note) {
  const box = $("plList");
  box.replaceChildren();
  if (note) box.append(el("div", "plNote", note));
  // playlists first, then albums from your files under their own heading
  const item = (p, into) => {
    const b = el("button", "plItem");
    b.dataset.id = p.id;
    b.draggable = true;   // onto another one: the two combined (js/dnd.js)
    b.title = p.title || "";
    b.append(platformBadge({ source: p.source || "soundcloud" }), el("span", "plName", p.title || "Untitled playlist"));
    into.append(b);
  };
  // your albums get their own section below, which scrolls separately
  const albums = playlists.filter((p) => p.group === "albums");
  playlists.filter((p) => p.group !== "albums").forEach((p) => item(p, box));
  const abox = $("albumList");
  abox.replaceChildren();
  albums.forEach((p) => item(p, abox));
  $("albumSection").hidden = !albums.length;
  if (!playlists.length && !note) box.append(el("div", "plNote", "No playlists yet."));
  showView(view);
}

$("sideLists").addEventListener("click", (e) => {
  const b = e.target.closest(".plItem");
  if (!b) return;
  const p = playlists.find((x) => String(x.id) === b.dataset.id);
  if (p) openPlaylist(p);
});

// Your SoundCloud playlists and the ones you liked (refreshSidebar shows them with the others)
async function scPlaylists() {
  const list = [];
  let failed = 0;
  for (const path of ["/me/playlists?limit=50&show_tracks=false", "/me/likes/playlists?limit=50"]) {
    try {
      const data = await api(path);
      const items = Array.isArray(data) ? data : (data.collection || []);
      items.forEach((p) => { if (p && p.id && !list.some((x) => x.id === p.id)) list.push(p); });
    } catch (e) {
      failed++;
    }
  }
  if (failed === 2) throw new Error("Could not load your SoundCloud playlists.");
  return list;
}

// After creating or deleting one
const loadPlaylists = () => refreshSidebar();

// ---------- queue ----------
// The queue is: what's playing, then what you queued (and chat requests), then the rest of the list
// you started (a playlist, an album, your Library...), then autoplay's picks. Each part shows on its own.
let queueListName = "";

function listNameHere() {
  const v = view === "now" ? back : view;
  if (v === "search" && query) return 'Search: "' + query + '"';
  return ($("viewTitle").textContent || "").trim() || "your list";
}

const isQueuedByYou = (t) => !!t && (t.queuedByYou || !!t.requestedBy);
// the queue came from your Library or a playlist: shuffled, its recent plays are spaced out (spaceRecent)
let queueSpaced = false;

function startList(list, i, shuffle, name) {
  if (!list.length) return;
  queueListName = name || listNameHere();
  if (!prefs.queueSets && view !== "sets") {
    const picked = shuffle ? null : list[i];
    const kept = list.filter((t) => !isMix(t) || t === picked);
    if (kept.length) { list = kept; i = picked ? list.indexOf(picked) : 0; }
  }
  // your Library and playlists: shuffled, recent plays are spaced out instead of left out (and played
  // in order, they play as they are). With repeat on, nothing is held back: you asked to hear it again.
  queueSpaced = view === "likes" || view === "playlist";
  const holdBack = repeatMode === "off";
  // Avoid Recent Plays: what you heard lately is left out of what plays from a list (not the track you
  // picked, and not from a search); a list that's nearly all recent plays as it is
  if (holdBack && !queueSpaced && avoidRecent() && view !== "search" && list.length > 1) {
    const picked = shuffle ? null : list[i];
    const rest = withoutRecent(list.filter((t) => t !== picked), 3);
    if (rest.length < list.length - (picked ? 1 : 0)) {
      list = picked ? list.filter((t) => t === picked || rest.includes(t)) : rest;
      i = picked ? list.indexOf(picked) : 0;
    }
  }
  if (streamSafe()) {
    const picked = list[i];
    list = list.filter(streamOk);
    if (!list.length) { say("Stream-Safe Music is on, and nothing here is safe on stream.", { force: true }); return; }
    i = Math.max(0, list.indexOf(picked));
  }
  queue = list.slice();
  if (shuffle) { queue = shuffledOrder(queue); if (queueSpaced && holdBack) queue = spaceRecent(queue); i = 0; }   // fair: long-unheard tracks first (js/shuffle.js)
  qi = i;
  if (shuffleOn && !shuffle) shuffleUpcoming();
  playCurrent();
}

const queuedByYou = (t) => Object.assign({}, t, { queuedByYou: true, autoplay: undefined });

function playNext(t) {
  if (!t) return;
  if (!safeOk(t)) { say("Stream-Safe Music is on, and that track isn't safe on stream.", { force: true }); return; }
  if (qi < 0) { startList([t], 0, false); return; }
  queue.splice(qi + 1, 0, queuedByYou(t));
  renderQueue();
}

// Where "Add to queue" puts a track: after what's playing and anything else you've queued, ahead of
// the rest of the list that's playing (and of autoplay's picks)
function queuedEnd() {
  let i = qi + 1;
  while (i < queue.length && isQueuedByYou(queue[i])) i++;
  return i;
}

function addToQueue(t) {
  if (!t) return;
  if (!safeOk(t)) { say("Stream-Safe Music is on, and that track isn't safe on stream.", { force: true }); return; }
  if (qi < 0) { startList([t], 0, false); return; }
  queue.splice(queuedEnd(), 0, queuedByYou(t));
  renderQueue();
}

function queueRow(t, idx, removable) {
  applyFix(t);
  const li = el("li", "qRow" + (idx === qi ? " cur" : "") + (streamOk(t) ? "" : " notSafe"));
  li.dataset.q = idx;
  li.draggable = true;   // onto a playlist (what's playing is copied, not moved)
  const img = el("img");
  img.alt = "";
  img.loading = "lazy";
  const src = artUrl(t, "t120x120");
  if (src) img.src = src;
  const meta = el("div", "meta");
  const qa = el("div", "artist");
  const who = el("span", "artistLink");
  fillArtists(who, t);   // each artist lights up and opens their page; dragging still takes the track
  qa.append(platformBadge(t), who);
  if (t.requestedBy) qa.append(el("span", "reqBy", "requested by " + t.requestedBy));
  meta.append(el("div", "title", t.title || "Untitled"), qa);
  li.append(img, meta);
  if (removable) {
    const b = el("button");
    b.dataset.remove = idx;
    b.title = "Remove from queue";
    b.setAttribute("aria-label", "Remove from queue");
    b.append(icon(ICON_CLOSE));
    li.append(b);
  } else {
    li.append(el("span"));
  }
  return li;
}

const QUEUE_SHOWN = 100;

function renderQueue() {
  hostPush();   // listeners see the queue coming up (js/together.js)
  const nowBox = $("qNow");
  const nextBox = $("qNext");
  const autoBox = $("qAuto");
  nowBox.replaceChildren();
  nextBox.replaceChildren();
  autoBox.replaceChildren();
  const t = curTrack();
  if (t && t.standInFor) {
    // listening along: the Jam's track, greyed, with what plays in its place (js/together.js)
    const li = queueRow(t.standInFor, qi, false);
    li.classList.add("qUnavail");
    li.querySelector(".meta").append(el("div", "qNote", "Not available on your end \u00b7 playing " + (t.title || "a song of yours")));
    nowBox.append(li);
  } else if (t) nowBox.append(queueRow(t, qi, false));
  else nowBox.append(el("li", "qEmpty", "Nothing playing."));
  // what you queued, then the rest of the list, then autoplay's picks; each run gets its own heading
  const split = qi >= 0 ? autoplayStart() : 0;
  const next = qi >= 0 ? queue.slice(qi + 1, split) : [];
  const auto = qi >= 0 ? queue.slice(split) : [];
  if (!next.length) {
    nextBox.append(el("li", "qGroup", "Next in queue"), el("li", "qEmpty", "Nothing queued. Add tracks with + or drag them here."));
  }
  // A long queue (all your likes, say) only draws the next stretch of it
  let kind = null;
  let shown = 0;
  next.forEach((tr, k) => {
    const mine = isQueuedByYou(tr);
    if (mine !== kind) {
      kind = mine;
      nextBox.append(el("li", "qGroup" + (mine ? " qMine" : " qList"), mine ? "Next in queue" : "Next from " + (queueListName || "your list")));
    }
    if (shown++ < QUEUE_SHOWN) {
      const li = queueRow(tr, qi + 1 + k, together.role !== "listener");
      if (together.role === "listener" && !canPlayHere(tr)) {
        li.classList.add("qUnavail");
        li.querySelector(".meta").append(el("div", "qNote", "Not available on your end: a song of yours plays in its place"));
      }
      nextBox.append(li);
    }
  });
  if (next.length > QUEUE_SHOWN) nextBox.append(el("li", "qEmpty", "...and " + (next.length - QUEUE_SHOWN) + " more."));
  auto.forEach((tr, k) => autoBox.append(queueRow(tr, split + k, true)));
  autoBox.hidden = !auto.length || !autoplayOn();
  autoplayPaint();
  reqQueueBtn();
  sessionSave();
}

$("queue").addEventListener("click", (e) => {
  const rm = e.target.closest("button[data-remove]");
  if (rm) {
    const idx = Number(rm.dataset.remove);
    if (idx > qi) { queue.splice(idx, 1); renderQueue(); }
    return;
  }
  const li = e.target.closest(".qRow");
  if (!li) return;
  const idx = Number(li.dataset.q);
  if (e.target.closest(".artistLink") && queue[idx]) { openArtistClicked(queue[idx], e.target); return; }
  if (idx !== qi) { qi = idx; playCurrent(); }
});

// right-click a queue row: the same menu as anywhere else (what's playing gets the player's)
$("queue").addEventListener("contextmenu", (e) => {
  const li = e.target.closest(".qRow");
  if (!li) return;
  e.preventDefault();
  e.stopPropagation();
  const idx = Number(li.dataset.q);
  if (queue[idx]) openTrackMenu(queue[idx], e.clientX, e.clientY, idx === qi, idx);
});

$("clearQueue").addEventListener("click", () => {
  queueListName = "";
  queue = qi >= 0 ? [queue[qi]] : [];
  qi = queue.length ? 0 : -1;
  renderQueue();
});

function setQueueOpen(open) {
  prefs.queue = open;
  savePrefs();
  $("queue").hidden = !open;
  $("app").classList.toggle("queueOpen", open);
  $("queueBtn").classList.toggle("on", open);
}
$("queueBtn").addEventListener("click", () => setQueueOpen(!prefs.queue));

// ---------- now playing ----------
// The bar's tab: as wide as the track's title and artist need, sliding there from the width before
function tabFit() {
  const n = $("nowToggle");
  if (!n) return;
  // (only the Tab style's track info is a tab that sizes itself)
  if (playerStyle() !== "tab") { n.style.width = ""; return; }
  const was = n.getBoundingClientRect().width;
  n.style.transition = "none";
  n.style.width = "auto";
  const bar = n.parentElement.getBoundingClientRect();
  // never narrower than the panel above it: the sidebar (on the left), or the queue (on the right,
  // while it's open), so their edges line up and a short title doesn't wrap
  const right = document.documentElement.dataset.tab === "right";
  const panel = right ? $("queue") : document.querySelector(".sidebar");
  const pr = panel && !panel.hidden && panel.offsetParent ? panel.getBoundingClientRect() : null;
  const floor = pr && pr.width ? (right ? bar.right - pr.left : pr.right - bar.left) : 0;
  const want = Math.min(Math.max(n.getBoundingClientRect().width, floor), bar.width);
  n.style.width = was + "px";
  void n.offsetWidth;
  n.style.transition = "";
  n.style.width = Math.ceil(want) + "px";
}
addEventListener("resize", () => requestAnimationFrame(tabFit));
// (and when the sidebar or the queue changes width: opened, closed, narrowed)
if (window.ResizeObserver) { const ro = new ResizeObserver(() => requestAnimationFrame(tabFit)); [document.querySelector(".sidebar"), $("queue")].forEach((x) => x && ro.observe(x)); }
// (and whenever what it shows changes: an album name, say, comes in after the track has started)
if (document.fonts && document.fonts.ready) document.fonts.ready.then(() => requestAnimationFrame(tabFit));
new MutationObserver(() => requestAnimationFrame(tabFit)).observe($("nowToggle").querySelector(".nowText"), { childList: true, subtree: true, characterData: true });

function fillNowPlaying(t) {
  requestAnimationFrame(tabFit);
  applyFix(t);
  $("nowTitle").textContent = t ? t.title || "Untitled" : "Nothing playing";
  fillArtists($("nowArtist"), t);
  $("nowAlbum").textContent = t ? albumShown(t) : "";
  $("plAddBtn").hidden = !t;
  $("nowToggle").draggable = !!t;
  $("moreBtn").hidden = !t;
  $("nowPlat").replaceChildren(...(t ? [platformButton(t)] : []));
  const small = t ? artUrl(t, "t120x120") : "";
  fadeImg($("nowArt"), small);

  $("npTitle").textContent = t ? t.title || "Untitled" : "Nothing playing";
  if (t && trimOf(t)) { $("npTitle").append(trimMark(t)); $("nowTitle").append(trimMark(t)); }
  fillArtists($("npArtist"), t);
  $("npPlat").replaceChildren(...(t ? [platformButton(t)] : []));
  const bits = [];
  if (t && t.genre) bits.push(t.genre);
  const year = t && t.created_at ? String(t.created_at).slice(0, 4) : "";
  if (/^\d{4}$/.test(year)) bits.push(year);
  if (t && t.playback_count) bits.push(Number(t.playback_count).toLocaleString() + " plays");
  $("npMeta").textContent = bits.join(", ");
  const album = t ? albumShown(t) : "";
  $("npAlbum").textContent = album;
  $("npAlbum").hidden = !album;
  $("npDesc").textContent = t && t.description ? t.description.slice(0, 600) : "";
  const big = t ? artUrl(t, "t500x500") : "";
  if (big) { fadeImg($("npArt"), big); $("npArt").hidden = false; } else { $("npArt").hidden = true; }
  $("npOpen").hidden = !(t && t.permalink_url);
  $("npOpen").textContent = t ? openLabel(t) : "";
  $("dur").textContent = fmt(t ? viewDur(t, t.duration || 0) : 0);

  if (t && "mediaSession" in navigator) {
    navigator.mediaSession.metadata = new MediaMetadata({
      title: t.title || "",
      artist: artistName(t),
      artwork: big ? [{ src: big }] : [],
    });
  }
}

function openCurrentArtist(e) {
  e.stopPropagation();
  const t = curTrack();
  if (t) openArtistClicked(t, e.target);
}
$("nowArtist").addEventListener("click", openCurrentArtist);
$("npArtist").addEventListener("click", openCurrentArtist);
// the platform icon: the uploader's page
["nowPlat", "npPlat"].forEach((id) => $(id).addEventListener("click", (e) => {
  if (!e.target.closest(".platBig")) return;
  e.stopPropagation();
  const t = curTrack();
  if (t) openArtistFor(t);
}));

function toggleNowView() { showView(view === "now" ? back : "now"); }
$("nowToggle").addEventListener("click", toggleNowView);
$("nowToggle").addEventListener("keydown", (e) => {
  if (e.target !== $("nowToggle")) return;
  if (e.key === "Enter" || e.key === " ") { e.preventDefault(); toggleNowView(); }
});
$("npOpen").addEventListener("click", () => {
  const t = curTrack();
  if (t && t.permalink_url) invoke("open_url", { url: t.permalink_url }).catch((e) => say(String(e)));
});

// ---------- like / unlike ----------
function setHeart(btn, on) {
  btn.classList.toggle("on", on);
  btn.replaceChildren(icon(on ? ICON_HEART : ICON_HEART_OUT));
  btn.title = on ? "Remove from Likes" : "Add to Likes";
  if (btn.id === "likeBtn") btn.setAttribute("aria-label", btn.title);
}

function refreshLikeUI() {
  taskbarSync();
  const t = curTrack();
  const lb = $("likeBtn");
  lb.hidden = !canLike(t);
  if (t) setHeart(lb, isLiked(t));
  const nb = $("npLike");
  nb.hidden = !canLike(t);
  if (t) {
    nb.textContent = isLiked(t) ? "Liked" : "Like";
    nb.classList.toggle("on", isLiked(t));
  }
  document.querySelectorAll("#list li").forEach((li) => {
    const t2 = viewTracks[Number(li.dataset.i)];
    const b = li.querySelector('button[data-act="like"]');
    if (t2 && b && b.classList.contains("on") !== isLiked(t2)) setHeart(b, isLiked(t2));
  });
  refreshFavUI();
}

async function setLikedOn(t, on) {
  if (srcOf(t) === "spotify") await spotifySetLiked(t, on);
  else if (srcOf(t) === "youtube") ytSetLiked(t, on);
  else await invoke("set_like", { id: String(t.id), like: on });
  ytMirrorLike(t, on);   // and on YouTube, with your account connected and that asked for (js/youtube.js)
}

// Liking likes this copy of the song; unliking unlikes it wherever it's liked (the same song on
// other platforms shows as one row, liked if any of them is)
async function toggleLike(t) {
  if (!canLike(t)) return;
  const next = !isLiked(t);
  const targets = next ? [t] : [t].concat(t.dupes || []).filter((x) => canLike(x) && likeState.get(x.id) === true);
  targets.forEach((x) => likeState.set(x.id, next));   // update right away, undo if the platform refuses
  refreshLikeUI();
  for (const x of targets) {
    try {
      await setLikedOn(x, next);
    } catch (e) {
      likeState.set(x.id, !next);
      refreshLikeUI();
      say("Could not " + (next ? "like" : "unlike") + " this track: " + e);
    }
  }
  if (next && likeState.get(t.id) === true) widgetLiked(t);   // a request: the widget cheers them on
}

$("likeBtn").addEventListener("click", (e) => { e.stopPropagation(); toggleLike(curTrack()); });

// The track menu for what's playing: the "..." button, or right-click the now-playing area
$("moreBtn").addEventListener("click", (e) => {
  const t = curTrack();
  if (!t) return;
  e.stopPropagation();
  const r = $("moreBtn").getBoundingClientRect();
  openTrackMenu(t, r.left, r.top - 4, true);
});
$("nowToggle").addEventListener("contextmenu", (e) => {
  e.preventDefault();
  const t = curTrack();
  if (t) openTrackMenu(t, e.clientX, e.clientY, true);
});
$("plAddBtn").addEventListener("click", (e) => {
  e.stopPropagation();
  const t = curTrack();
  if (t) openPlaylistMenu(t, $("plAddBtn"));
});
$("npLike").addEventListener("click", () => toggleLike(curTrack()));

// A track picked on a page: from a playlist (or Library, Favourites and the like) the list becomes the
// queue; from Search or an artist's page it only takes the place of what's playing, and the queue
// you had carries on after it
function playFromPage(i) {
  const t = viewTracks[i];
  if (!t) return;
  if ((view === "search" || view === "artist") && queue.length && qi >= 0) {
    queue[qi] = Object.assign({}, t, { autoplay: undefined, requestedBy: undefined });
    playCurrent();
    return;
  }
  if (view === "search" || view === "artist") { startList([t], 0, false); return; }
  startList(viewTracks, i, false);
}

// ---------- playback ----------
async function playCurrent() {
  const t = curTrack();
  if (!t) return;
  if (closestNote && closestNote.t !== t) { closestNote = null; say("", { force: true }); }
  if (!safeOk(t)) { safeSkip(t); return; }
  fadeTrackChange(fadeState.last, t);   // the old one fades out, this one in (js/fade.js)
  fadeState.last = t;
  nativeOutFlush();   // the last track's tail isn't heard over the new one
  fillNowPlaying(t);
  refreshLikeUI();
  trackStarted(t);
  markPlaying();
  renderQueue();
  say("");
  const startAt = trimStartFor(t, takeResume(t));   // a resumed position, or where it's set to start
  try {
    if (onSpotifyEngine(t)) {
      // a stream still loading for the track before (SoundCloud, YouTube...) mustn't start under this one
      streamToken++;
      if (hls) { hls.destroy(); hls = null; }
      audio.pause();
      await spotifyStart(t, startAt);
    } else {
      spotifyPause();
      await startStream(t);
      if (startAt && curTrack() === t) audio.currentTime = startAt / 1000;
    }
  } catch (e) {
    if (e && e.name === "AbortError") return;   // skipped to another track
    // listening along: what can't play here comes from YouTube Music, or waits for the next one
    if (together.role === "listener" && curTrack() === t) { listenerFallback(t, String(e && e.message ? e.message : e).replace(/^Error: /, "")); return; }
    say("Could not play this track: " + e);
  }
}

// Full tracks arrive as HLS, which the browser engine can't play without hls.js
let hls = null;
let streamToken = 0;   // ignore results for tracks we've already skipped past
// and whatever still starts the <audio> element late, under a Spotify track, is stopped
audio.addEventListener("playing", () => { if (onSpotifyEngine(curTrack())) audio.pause(); });

const streamName = (name) => String(name).replace(/_url$/, "");

// Loads one stream and resolves once it is actually playing
function tryStream(url, isHls, auth) {
  return new Promise((resolve, reject) => {
    let done = false;
    const finish = (err) => {
      if (done) return;
      done = true;
      clearTimeout(timer);
      audio.removeEventListener("playing", onPlaying);
      audio.removeEventListener("error", onError);
      if (err) reject(err); else resolve();
    };
    const onPlaying = () => finish(null);
    const onError = () => finish(new Error("unsupported or broken stream" +
      (audio.error ? " (code " + audio.error.code + (audio.error.message ? ", " + audio.error.message : "") + ")" : "")));
    const timer = setTimeout(() => finish(new Error("timed out")), 15000);
    audio.addEventListener("playing", onPlaying);
    audio.addEventListener("error", onError);

    if (hls) { hls.destroy(); hls = null; }
    if (isHls) {
      if (window.Hls && window.Hls.isSupported()) {
        hls = new window.Hls({
          xhrSetup: (xhr, u) => {
            if (auth && new URL(u).hostname === "api.soundcloud.com") xhr.setRequestHeader("Authorization", auth);
          },
        });
        hls.on(window.Hls.Events.ERROR, (_e, data) => {
          if (!data.fatal) return;
          if (done) say("Stream error (" + data.type + "): " + data.details);
          else finish(new Error(data.details));
        });
        hls.loadSource(url);
        hls.attachMedia(audio);
      } else if (audio.canPlayType("application/vnd.apple.mpegurl")) {
        audio.src = url;
      } else {
        finish(new Error("HLS support is missing (run npm install)"));
        return;
      }
    } else {
      audio.src = url;
    }
    audio.play().catch((e) => { if (e && e.name === "AbortError") return; finish(e); });
  });
}

// Fallback for MP3 streams the player won't take as HLS: an MP3 split into pieces
// can simply be joined back together, so fetch every piece and play the whole file.
let joinedBlobUrl = null;

async function joinedMp3Url(m3u8Url, auth, depth) {
  const headers = auth && new URL(m3u8Url).hostname === "api.soundcloud.com" ? { Authorization: auth } : {};
  const res = await fetch(m3u8Url, { headers });
  if (!res.ok) throw new Error("playlist request failed (" + res.status + ")");
  const text = await res.text();
  if (/#EXT-X-KEY:METHOD=(?!NONE)/.test(text)) throw new Error("the stream is encrypted");
  const base = res.url || m3u8Url;
  const parts = text.split("\n").map((l) => l.trim())
    .filter((l) => l && !l.startsWith("#"))
    .map((l) => new URL(l, base).href);
  if (!parts.length) throw new Error("the playlist is empty");
  if (/\.m3u8(\?|$)/i.test(parts[0]) && (depth || 0) < 2) return joinedMp3Url(parts[0], auth, (depth || 0) + 1);
  const pieces = new Array(parts.length);
  let next = 0;
  const worker = async () => {
    while (next < parts.length) {
      const i = next++;
      const r = await fetch(parts[i]);
      if (!r.ok) throw new Error("piece " + (i + 1) + " failed (" + r.status + ")");
      pieces[i] = await r.arrayBuffer();
    }
  };
  await Promise.all([worker(), worker(), worker(), worker()]);
  return URL.createObjectURL(new Blob(pieces, { type: "audio/mpeg" }));
}

// SoundCloud offers each track in a few formats; try them in order until one plays
async function startStream(t) {
  const mine = ++streamToken;
  if (srcOf(t) === "local") {
    const live = liveLocal(t);   // a restored queue's files get this run's address
    if (!live.url) throw new Error("this file isn't in your music folders any more");
    t.url = live.url;
    await tryStream(t.url, false, null);
    if (mine === streamToken) say("");
    return;
  }
  // Spotify without Premium, or a song removed from Spotify: the same song from YouTube Music
  if (srcOf(t) === "spotify") {
    const note = t.spGone ? "This song was removed from Spotify: playing it from YouTube Music." : "Playing from YouTube Music (this Spotify account isn't Premium).";
    if (await playViaYouTube(t, mine, note)) return;
    if (mine !== streamToken) return;
    if (t.spGone) {
      // listening along: a song of yours plays in its place (js/together.js)
      if (together.role === "listener") { playStandIn(t, Math.max(0, (t.duration || 0) - hostPos()) || hostLenMs(t)); return; }
      // nothing to play it from: say so, and carry on with the queue
      say('"' + (t.title || "This song") + '" was removed from Spotify' + (ytOn() ? " and isn't on YouTube Music either" : " (and YouTube Music is off)") + ", so it was skipped.");
      setTimeout(() => { if (curTrack() === t) { if (qi < queue.length - 1) step(1); else if (autoplayActive()) autoplayAdvance(); } }, 1500);
      return;
    }
    throw new Error(ytOn() ? "this song couldn't be found on YouTube Music" : "without Spotify Premium, Spotify tracks play from YouTube Music, which is switched off");
  }
  if (isPluginTrack(t)) {
    const url = await pluginPlayUrl(t);
    if (mine !== streamToken) return;
    await tryStream(url, false, null);
    if (mine === streamToken) say("");
    return;
  }
  if (srcOf(t) === "youtube") {
    const url = await ytPlay(t);
    if (mine !== streamToken) return;
    await tryStream(url, false, null);
    if (mine === streamToken) say("");
    return;
  }
  // Go+: the full song from YouTube Music (or Spotify), never SoundCloud's 30 seconds
  if (isPreview(t)) { await playFullElsewhere(t, mine, "Go+ track"); return; }
  const info = await invoke("stream_url", { id: String(t.id) });
  if (mine !== streamToken) return;
  const failures = [];
  for (const c of info.candidates || []) {
    // only the preview left: the full song from elsewhere, or nothing
    if (c.name === "preview") { await playFullElsewhere(t, mine, "SoundCloud only had a preview"); return; }
    let resolved;
    try {
      resolved = await invoke("stream_resolve", { url: c.url });
    } catch (e) {
      failures.push(streamName(c.name) + ": " + e);
      continue;
    }
    if (mine !== streamToken) return;
    const isHls = String(c.name).startsWith("hls") || /\.m3u8(\?|$)/i.test(resolved.url) || /\/hls\b/i.test(resolved.url);
    try {
      await tryStream(resolved.url, isHls, resolved.auth);
      if (mine !== streamToken) return;
      // a "full" stream that's only 30 seconds of a longer song is a preview, whatever it's called
      const full = t.full_duration || t.duration || 0;
      const got = isFinite(audio.duration) ? audio.duration * 1000 : 0;
      if (got > 0 && got < 35000 && full > 60000) { audio.pause(); await playFullElsewhere(t, mine, "SoundCloud only sent a 30-second preview"); return; }
      say("");
      return;
    } catch (e) {
      if (mine !== streamToken) return;
      failures.push(streamName(c.name) + ": " + (e && e.message ? e.message : e));
      if (isHls && /mp3/i.test(c.name)) {
        try {
          say("Loading the whole track...");
          const blobUrl = await joinedMp3Url(resolved.url, resolved.auth);
          if (mine !== streamToken) { URL.revokeObjectURL(blobUrl); return; }
          if (joinedBlobUrl) URL.revokeObjectURL(joinedBlobUrl);
          joinedBlobUrl = blobUrl;
          await tryStream(blobUrl, false, null);
          if (mine !== streamToken) return;
          say("");
          return;
        } catch (e2) {
          if (mine !== streamToken) return;
          failures.push(streamName(c.name) + " joined: " + (e2 && e2.message ? e2.message : e2));
        }
      }
    }
  }
  throw new Error("none of SoundCloud's formats would play (" + failures.join("; ") + ")");
}

// ---------- Go+ and other preview-only tracks ----------
// The full song from YouTube Music (its best match, picked by itself) or, with Spotify Premium, from
// Spotify. With neither, the track is skipped, and a note stays up saying so and offering to hide it.
const spVersions = new Map();   // SoundCloud id -> the Spotify track found for it (or null)

async function spotifyVersion(t) {
  if (!spFullOn()) return null;
  const key = String(t.id);
  if (spVersions.has(key)) return spVersions.get(key);
  const n = guessNames(t);
  let found = [];
  try { found = await spotifySearchTracks((n.artist || artistName(t)) + " " + matchTitle(t)); } catch (e) { return null; }
  const best = found.map((c) => ({ c, score: ytMatchScore(t, c) })).sort((a, b) => b.score - a.score).find((x) => x.score >= 0.5 && artistHit(t, x.c));
  const sp = best ? best.c : null;
  spVersions.set(key, sp);
  return sp;
}

async function playFullElsewhere(t, mine, why) {
  if (await playViaYouTube(t, mine, why + ": playing the full song from YouTube Music.", true)) return;
  if (mine !== streamToken) return;
  const sp = await spotifyVersion(t);
  if (mine !== streamToken) return;
  if (sp && curTrack() === t) {
    // the Spotify version takes its place in the queue
    queue[qi] = sp;
    setTimeout(() => playCurrent().then(() => { if (curTrack() === sp) say(why + ": playing the full song from Spotify."); }), 0);
    return;
  }
  // nothing close enough: the closest there is, so it keeps playing; a note says so while it plays
  const near = ytOn() ? ytClosest(t) : null;
  if (near && curTrack() === t) {
    const ok = await playYouTubeVersion(t, near, mine, "").catch(() => false);
    if (mine !== streamToken) return;
    if (ok) { closestShow(t, near, why); return; }
  }
  noVersionSkip(t, why);
}

// ---------- the closest version, playing ----------
// While it plays, a note says which it is, with Choose a version and Skip. Heard to the end, it's
// kept for that track from then on, and its artist is shown as that version's.
let closestNote = null;   // { t, near }
function closestShow(t, near, why) {
  closestNote = { t, near };
  say(why + ": no exact match, so it's playing the closest, \"" + near.title + "\" by " + artistName(near) + ", from YouTube Music. Wrong one?", {
    stay: true,
    actions: [["Choose a version", () => { closestNote = null; openFixDialog(t); }], ["Skip", () => { closestNote = null; step(1); }]],
  });
}

// It played to the end: that version it is
function closestAccept() {
  const n = closestNote;
  closestNote = null;
  if (!n || curTrack() !== n.t) return;
  const t = n.t;
  ytChooseMatch(t, n.near);
  const theirs = artistName(n.near);
  if (theirs && theirs.toLowerCase() !== String(artistName(t)).toLowerCase()) {
    const old = (prefs.trackFixes || {})[t.id];
    prefs.trackFixes = Object.assign({}, prefs.trackFixes, { [t.id]: { title: (old && old.title) || (t._auto ? t._auto.title : t.title), artist: theirs } });
    savePrefs();
    forEachCopy(t.id, (x) => {
      if (!x._auto) x._auto = { title: x.title, artist: x.displayArtist || null };
      x.displayArtist = theirs;
    });
    refreshAfterFix(t.id);
  }
  say("", { force: true });
  say('"' + (t.title || "That track") + "\" plays that version from now on" + (theirs ? ", by " + theirs : "") + ".");
}

function noVersionSkip(t, why) {
  audio.pause();
  const where = [ytOn() ? "YouTube Music" : "", spFullOn() ? "Spotify" : ""].filter(Boolean);
  const actions = [["Hide it", () => noVersionHide(t)]];
  if (ytOn()) actions.push(["Choose a version", () => openFixDialog(t)]);
  say("Skipped \"" + (t.title || "this track") + "\" (" + why + "): " +
    (where.length ? "no full version turned up on " + where.join(" or ") : "nothing you use can play the full song") +
    (ytLastMiss && ytLastMiss.id === t.id && ytLastMiss.closest ? " (the closest was " + ytLastMiss.closest + ")" : "") + ".",
    { stay: true, bad: true, actions });
  setTimeout(() => { if (curTrack() === t) { if (qi < queue.length - 1) step(1); else if (autoplayActive()) autoplayAdvance(); } }, 300);
}

// Hidden from every list, until it's shown again (Settings > Library > Hidden & Not Recommended)
function noVersionHide(t) {
  prefs.noVersion = Object.assign({}, prefs.noVersion, { [String(t.id)]: (t.title || "Untitled") + " \u2014 " + artistName(t) });
  savePrefs();
  if (typeof paintNotRecommended === "function") paintNotRecommended();
  viewTracks = viewTracks.filter((x) => x !== t && String(x.id) !== String(t.id));
  render();
  say("Hidden. Settings > Library > Hidden & Not Recommended shows it again.");
}

function toggle() {
  const cur = curTrack();
  if (onSpotifyEngine(cur)) { spotifyToggle(); return; }
  if (!audio.src) {
    if (cur) playCurrent();   // the queue from last time
    else if (viewTracks.length) startList(viewTracks, 0, false);
    return;
  }
  if (audio.paused) audio.play(); else audio.pause();
}

function step(d) {
  if (!queue.length) return;
  // Previous restarts the track if it has been playing for a few seconds
  const cur = curTrack();
  const onSpotify = onSpotifyEngine(cur);
  // (back to where the track's set to start)
  if (d < 0 && onSpotify && spotifyPos() > trimStart(cur) + 3000) { spotifySeek(spState.dur ? trimStart(cur) / spState.dur : 0); return; }
  if (d < 0 && !onSpotify && audio.currentTime * 1000 > trimStart(cur) + 3000) { audio.currentTime = trimStart(cur) / 1000; return; }
  if (d > 0 && qi === queue.length - 1 && autoplayActive()) { autoplayAdvance(); return; }
  qi = (qi + d + queue.length) % queue.length;
  playCurrent();
}

audio.addEventListener("play", () => { $("icoPlay").classList.add("off"); $("icoPause").classList.remove("off"); });
audio.addEventListener("pause", () => { $("icoPlay").classList.remove("off"); $("icoPause").classList.add("off"); });
audio.addEventListener("ended", onEnded);
audio.addEventListener("error", () => {
  say("The player could not decode this stream" + (audio.error ? " (code " + audio.error.code + ")" : "") + ".");
});
// positions and lengths as shown: within the part of the track that plays (js/trims.js)
audio.addEventListener("timeupdate", () => {
  const t = curTrack();
  $("cur").textContent = fmt(viewPos(t, audio.currentTime * 1000));
  const d = viewDur(t, curDurMs());
  if (d) paintProgress(viewPos(t, audio.currentTime * 1000) / d);
});
audio.addEventListener("loadedmetadata", () => {
  if (isFinite(audio.duration) && audio.duration) $("dur").textContent = fmt(viewDur(curTrack(), audio.duration * 1000));   // streams of unknown length report Infinity
});

$("seek").addEventListener("input", () => {
  const cur = curTrack();
  const frac = Number($("seek").value) / 1000;
  if (onSpotifyEngine(cur)) { if (spState.dur) spotifySeek(rawAt(cur, frac, spState.dur) / spState.dur); return; }
  const d = curDurMs();
  if (d) audio.currentTime = rawAt(cur, frac, d) / 1000;
});

audio.volume = appVolume();
$("vol").value = String(Math.round(prefs.volume * 100));

// (the volume slider is always in the player bar; an old "hidden" setting is let go)
delete prefs.hideVolume;
setLevel(appVolume());
castVolPaint();
$("vol").addEventListener("input", () => {
  prefs.volume = Number($("vol").value) / 100;
  setLevel(prefs.volume);
  savePrefs();
});
$("mute").addEventListener("click", () => {
  audio.muted = !audio.muted;
  normMute(audio.muted);
  $("icoVol").classList.toggle("off", audio.muted);
  $("icoMute").classList.toggle("off", !audio.muted);
});

$("toggle").addEventListener("click", toggle);
// Skipping by hand (buttons, keys, the remote): listening along, it goes to the Jam instead (js/together.js)
function skipPress(d) { step(d); }
$("prev").addEventListener("click", () => skipPress(-1));
$("next").addEventListener("click", () => skipPress(1));

// Media keys, the tray menu and the taskbar buttons call this from the Rust side
window.__mediaAction = (a) => {
  if (a === "play") toggle();
  else if (a === "next") skipPress(1);
  else if (a === "prev") skipPress(-1);
  else if (a === "shuffle") toggleShuffle();
  else if (a === "like") toggleLike(curTrack());
};

const dlg = $("settings");
document.addEventListener("keydown", (e) => {
  if (dlg.open) return;
  if (e.target.closest("input, select, textarea, button, [role='button']")) return;
  if (e.ctrlKey || e.altKey || e.metaKey) return;
  if (e.code === "Space") { e.preventDefault(); toggle(); }
  else if (e.shiftKey && e.code === "ArrowRight") skipPress(1);
  else if (e.shiftKey && e.code === "ArrowLeft") skipPress(-1);
  else if (e.key === "/") { e.preventDefault(); $("q").focus(); }
  else if (e.key === "q") setQueueOpen(!prefs.queue);
  else if (e.key === "l") toggleLike(curTrack());
});

// ---------- settings: appearance ----------
const ACCENTS = ["#8e9bff", "#1ed760", "#fa586a", "#ff9f43", "#4cc9f0", "#c77dff"];

function buildSwatches() {
  const box = $("swatches");
  box.replaceChildren();
  ACCENTS.forEach((c) => {
    const b = el("button", "swatch");
    b.style.background = c;
    b.dataset.c = c;
    b.setAttribute("aria-label", "Accent " + c);
    box.append(b);
  });
  const custom = el("input");
  custom.type = "color";
  custom.id = "accentCustom";
  custom.setAttribute("aria-label", "Custom accent color");
  box.append(custom);
}
buildSwatches();

function syncAppearance() {
  document.querySelectorAll("#densitySeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === prefs.density));
  document.querySelectorAll("#tabAlignSeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === (prefs.tabAlign === "right" ? "right" : "left")));
  document.querySelectorAll(".swatch").forEach((b) => b.classList.toggle("on", b.dataset.c === prefs.accent));
  $("accentCustom").value = prefs.accent;
  syncAccent2();
}

function setPref(key, value) {
  prefs[key] = value;
  savePrefs();
  applyPrefs();
  syncAppearance();
}

// ---------- Settings > Look > Player Style: a whole look for the player bar ----------
function playerStylePaint() {
  const ps = playerStyle();
  $("playerStyleSel").value = ps;
  $("playerStyleNote").textContent = PLAYER_STYLES[ps].note + (ps === "tab" ? "" : " Needs the window at least " + PLAYER_STYLES[ps].min + " px wide.");
  $("tabAlignRow").hidden = ps !== "tab";
}
playerStylePaint();
$("playerStyleSel").addEventListener("change", () => {
  setPref("playerStyle", $("playerStyleSel").value);
  playerStylePaint();
  requestAnimationFrame(tabFit);
});

$("roundedOn").checked = prefs.appRounded !== false;
$("roundedOn").addEventListener("change", () => setPref("appRounded", $("roundedOn").checked));
$("densitySeg").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) setPref("density", b.dataset.v); });
$("tabAlignSeg").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) { setPref("tabAlign", b.dataset.v); requestAnimationFrame(tabFit); } });
$("swatches").addEventListener("click", (e) => { const b = e.target.closest(".swatch"); if (b) setPref("accent", b.dataset.c); });
$("swatches").addEventListener("input", (e) => { if (e.target.id === "accentCustom") setPref("accent", e.target.value); });

// ---------- settings: audio output ----------
// The sound comes out of the audio chain (js/audio.js), so that's what's moved; the <audio> it reads
// from is moved too, for when the chain isn't running
async function applySink(id) {
  if (audio.setSinkId) await audio.setSinkId(id || "default").catch(() => {});
  if (norm.ctx) await mainSink(id);
  else if (!audio.setSinkId) throw new Error("This engine cannot choose an output device.");
}

async function refreshOutputs() {
  const sel = $("outputSel");
  sel.replaceChildren();
  let outs = [];
  try {
    outs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audiooutput");
  } catch (e) {}
  if (!outs.length) sel.append(new Option("System default", "default"));
  outs.forEach((d, i) => {
    const name = d.label || (d.deviceId === "default" ? "System default" : "Output " + (i + 1));
    sel.append(new Option(name, d.deviceId));
  });
  if (![...sel.options].some((o) => o.value === prefs.sink)) sel.selectedIndex = 0;
  else sel.value = prefs.sink;
  $("namesBtn").hidden = outs.length === 0 || outs.some((d) => d.label);
}

// Switches the PC's output (Settings, or the output button's menu)
async function setOutput(id) {
  try {
    await applySink(id);
    prefs.sink = id;
    savePrefs();
    $("outputNote").textContent = "";
  } catch (e) {
    $("outputNote").textContent = "Couldn't play on that device: " + (e && e.message ? e.message : e) + " Windows may be holding it for another app, or it may not take this sound format (try its settings in Windows' Sound panel).";
  }
}
$("outputSel").addEventListener("change", () => { if (cast.on) castStop(); setOutput($("outputSel").value); });

// Browsers only reveal device names after a microphone permission is granted.
// We open the mic for an instant and close it right away. Nothing is recorded.
$("namesBtn").addEventListener("click", async () => {
  try {
    const s = await navigator.mediaDevices.getUserMedia({ audio: true });
    s.getTracks().forEach((t) => t.stop());
    $("outputNote").textContent = "";
  } catch (e) {
    $("outputNote").textContent = e && e.name === "NotAllowedError"
      ? "Access is still blocked from an earlier choice. Close OmniPulse, delete the folder %LOCALAPPDATA%\\dev.scclient.app\\EBWebView, then reopen it."
      : "Could not read device names (no microphone found?). Devices are listed as Output 1, 2, and so on.";
  }
  refreshOutputs();
});

if (navigator.mediaDevices && navigator.mediaDevices.addEventListener) {
  navigator.mediaDevices.addEventListener("devicechange", () => { if (dlg.open) refreshOutputs(); });
}
if (prefs.sink !== "default") applySink(prefs.sink).catch(() => {});

// ---------- settings: performance ----------
$("gpuToggle").addEventListener("change", async () => {
  try {
    await invoke("set_gpu", { enabled: $("gpuToggle").checked });
    $("gpuNote").hidden = false;
  } catch (e) {
    say(String(e));
  }
});
$("restartBtn").addEventListener("click", () => invoke("restart_app"));

// ---------- settings dialog ----------
async function openSettings(focusId) {
  syncAppearance();
  try { $("gpuToggle").checked = (await invoke("get_settings")).gpu; } catch (e) {}
  $("gpuNote").hidden = true;
  extrasSyncSettings();
  await refreshOutputs();
  dlg.showModal();
  if (focusId) $(focusId).scrollIntoView({ block: "start" });
}
$("openSettings").addEventListener("click", () => openSettings());
$("outputBtn").addEventListener("click", (e) => { e.stopPropagation(); outputMenu(); });
$("closeSettings").addEventListener("click", () => dlg.close());
dlg.addEventListener("click", (e) => { if (e.target === dlg) dlg.close(); });

// ---------- sign in / out ----------
async function refreshAuth() {
  const ok = await invoke("signed_in");
  $("scAuth").textContent = ok ? "Sign out" : "Sign in";
  updateTitle(ok);
  renderPlatChips();   // SoundCloud is available while signed in
  return ok;
}

function startSession() {
  openLikes();
  refreshSidebar();
  loadUser();
}

async function signIn() {
  say("Finish signing in in your browser...");
  try {
    await invoke("login");
    await refreshAuth();
    startSession();
  } catch (e) {
    say(String(e));
  }
}

$("scAuth").addEventListener("click", async () => {
  if (await invoke("signed_in")) {
    await invoke("logout");
    userName = "";
    audio.pause();
    queue = [];
    qi = -1;
    viewTracks = [];
    nextHref = "";
    playlists = [];
    currentPlaylist = null;
    fillNowPlaying(null);
    renderQueue();
    renderPlaylists();
    showView("likes");
    render();
    await refreshAuth();
    say("Signed out.");
    return;
  }
  signIn();
});

// ---------- title bar ----------
let lastSignedIn = false;

function updateTitle(signedIn) {
  if (signedIn && userName) nameNote($("scNote"), "Signed in as ", userName, (scTier ? " \u00b7 " + scTier : "") + ".");
  else $("scNote").textContent = signedIn ? "Signed in" + (scTier ? " \u00b7 " + scTier : "") + "." : "Not signed in.";
  const text = $("titleText");
  const prompt = $("titlePrompt");
  lastSignedIn = signedIn;
  const custom = (prefs.appName || "").trim();
  let title;
  if (signedIn || custom || !scOn()) {
    title = custom || "OmniPulse";
    text.textContent = title;
    text.hidden = false;
    prompt.hidden = true;
  } else {
    title = "Sign in to SoundCloud";
    text.hidden = true;
    prompt.hidden = false;
  }
  document.title = title;
  win.setTitle(title).catch(() => {});
}

// SoundCloud's plan names, from /me (its subscriptions, or the older "plan" field)
let scTier = "";
function soundcloudTier(me) {
  const subs = (me && me.subscriptions) || [];
  const ids = subs.map((s) => ((s.product && (s.product.id || s.product.name)) || "").toLowerCase());
  if (ids.some((x) => /high.?tier|go\+|go plus/.test(x))) return "Go+";
  if (ids.some((x) => /mid.?tier|\bgo\b/.test(x))) return "Go";
  if (ids.some((x) => /unlimited|artist.?pro|creator/.test(x))) return "Artist Pro";
  if (ids.some((x) => /\bpro\b|creator.?pro/.test(x))) return "Artist";
  const plan = String((me && me.plan) || "").trim();
  if (plan && !/^free$/i.test(plan)) return plan;
  return "Free";
}

async function loadUser() {
  try {
    const me = await api("/me");
    userName = me.username || me.full_name || "";
    meId = me.id;
    scTier = soundcloudTier(me);
  } catch (e) {
    userName = "";
  }
  updateTitle(true);
}

$("titlePrompt").addEventListener("click", signIn);
$("statusBar").addEventListener("click", () => say("", { force: true }));
$("winMin").addEventListener("click", () => win.minimize());
$("winMax").addEventListener("click", () => win.toggleMaximize());
$("winClose").addEventListener("click", () => { if (prefs.closeToTray) win.hide(); else win.close(); });
// ...and closing it any other way (the taskbar's Close window, Alt+F4): the same choice
win.onCloseRequested((e) => { if (prefs.closeToTray) { e.preventDefault(); win.hide(); } }).catch(() => {});

async function syncMaximized() {
  try {
    const m = await win.isMaximized();
    $("icoMax").classList.toggle("off", m);
    $("icoRestore").classList.toggle("off", !m);
    $("winMax").setAttribute("aria-label", m ? "Restore" : "Maximize");
  } catch (e) {}
}
win.onResized(syncMaximized).catch(() => {});
syncMaximized();

// ---------- start ----------
(async () => {
  setQueueOpen(prefs.queue);
  fillNowPlaying(null);
  renderQueue();
  renderPlaylists();
  render();
  $("spotifySection").hidden = !FEATURES.spotify;
  extrasInit();
  sessionRestore();
  if (FEATURES.spotify) spotifyInit();
  const signedIn = await refreshAuth();
  scApply();
  if (signedIn && scOn()) startSession();
  else {
    loadUser();
    openLikes();   // whatever else is active; more appears as Spotify and your files finish loading
    if (!signedIn && !activePlats().length) say("Sign in to SoundCloud (or set up Spotify or local music) to load your likes, playlists and search.");
  }
})();
