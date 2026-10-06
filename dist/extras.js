// Extra features, loaded before app.js: this file only declares things, and app.js calls extrasInit().

// ---------- shuffle and repeat ----------
let repeatMode = "off";   // off | all | one
let shuffleOn = false;
let shuffleBackup = null; // upcoming tracks in their original order, to undo shuffle

// Taskbar thumbnail buttons: keep play/pause, shuffle and like icons current
function taskbarSync() {
  const t = curTrack();
  const playing = !!t && (srcOf(t) === "spotify" ? spState.playing : !audio.paused);
  invoke("taskbar_state", {
    playing,
    shuffle: shuffleOn,
    liked: isLiked(t),
    canLike: !!t && srcOf(t) === "soundcloud",
  }).catch(() => {});
}

function paintModes() {
  $("shuffleBtn").classList.toggle("on", shuffleOn);
  $("shuffleBtn").title = shuffleOn ? "Shuffle on" : "Shuffle off";
  $("repeatBtn").classList.toggle("on", repeatMode !== "off");
  $("repeatBtn").dataset.mode = repeatMode;
  $("repeatBtn").title = repeatMode === "one" ? "Repeat this track" : repeatMode === "all" ? "Repeat the queue" : "Repeat off";
  taskbarSync();
}

// Shuffles everything after the current track
function shuffleUpcoming() {
  if (qi < 0) return;
  const rest = queue.slice(qi + 1);
  shuffleBackup = rest.slice();
  shuffleInPlace(rest);
  queue = queue.slice(0, qi + 1).concat(rest);
  renderQueue();
}

function unshuffleUpcoming() {
  if (qi < 0 || !shuffleBackup) { shuffleBackup = null; return; }
  const rest = queue.slice(qi + 1);
  const present = new Set(rest.map((t) => t.id));
  const restored = shuffleBackup.filter((t) => present.has(t.id));
  const known = new Set(shuffleBackup.map((t) => t.id));
  const added = rest.filter((t) => !known.has(t.id));
  queue = queue.slice(0, qi + 1).concat(restored, added);
  shuffleBackup = null;
  renderQueue();
}

function toggleShuffle() {
  shuffleOn = !shuffleOn;
  prefs.shuffle = shuffleOn;
  savePrefs();
  if (shuffleOn) shuffleUpcoming(); else unshuffleUpcoming();
  paintModes();
}

function cycleRepeat() {
  repeatMode = repeatMode === "off" ? "all" : repeatMode === "all" ? "one" : "off";
  prefs.repeat = repeatMode;
  savePrefs();
  paintModes();
}

function onEnded() {
  if (repeatMode === "one") { audio.currentTime = 0; audio.play(); return; }
  if (qi < queue.length - 1) { step(1); return; }
  if (repeatMode === "all" && queue.length) { qi = 0; playCurrent(); }
}

// ---------- queue drag and drop ----------
function queueDndInit() {
  const list = $("qNext");
  let from = -1;
  const clear = () => document.querySelectorAll(".qRow.dragging, .qRow.over")
    .forEach((x) => x.classList.remove("dragging", "over"));
  list.addEventListener("dragstart", (e) => {
    const li = e.target.closest(".qRow");
    if (!li) return;
    from = Number(li.dataset.q);
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", String(from));
    li.classList.add("dragging");
  });
  list.addEventListener("dragover", (e) => {
    const li = e.target.closest(".qRow");
    if (!li || from < 0) return;
    e.preventDefault();
    document.querySelectorAll(".qRow.over").forEach((x) => x.classList.remove("over"));
    li.classList.add("over");
  });
  list.addEventListener("drop", (e) => {
    e.preventDefault();
    const li = e.target.closest(".qRow");
    if (li && from > qi) {
      const to = Number(li.dataset.q);
      if (to > qi && to !== from) {
        const [moved] = queue.splice(from, 1);
        queue.splice(to, 0, moved);
        renderQueue();
      }
    }
    from = -1;
    clear();
  });
  list.addEventListener("dragend", () => { from = -1; clear(); });
}

// ---------- custom CSS ----------
function applyUserCss() {
  let st = document.getElementById("userCss");
  if (!st) {
    st = document.createElement("style");
    st.id = "userCss";
    document.head.append(st);
  }
  st.textContent = cssEnabled() ? prefs.css || "" : "";
}

// ---------- accent from album art ----------
let artToken = 0;

function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let h = 0, s = 0;
  const l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h /= 6;
  }
  return [h, s, l];
}

function hslToHex(h, s, l) {
  const f = (n) => {
    const k = (n + h * 12) % 12;
    const a = s * Math.min(l, 1 - l);
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255).toString(16).padStart(2, "0");
  };
  return "#" + f(0) + f(8) + f(4);
}

let artColors = null;   // colours taken from the current cover, when that option is on

// Current accent colours (the second one comes from the cover when "Match accent to album art" is on)
let accentNow = ["#8e9bff", "#8e9bff"];

function setAccentColors(a1, a2) {
  const root = document.documentElement;
  accentNow = [a1, a2 || a1];
  root.style.setProperty("--accent", a1);
  root.style.setProperty("--accent-2", a2 || a1);
  root.style.setProperty("--on-accent", onAccent(a1));
  if (widgetReady) widgetPush();
}

// Two colours from the cover: the strongest one, and a clearly different one if there is one
function artPalette(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
      try {
        const c = document.createElement("canvas");
        c.width = c.height = 32;
        const ctx = c.getContext("2d", { willReadFrequently: true });
        ctx.drawImage(img, 0, 0, 32, 32);
        const d = ctx.getImageData(0, 0, 32, 32).data;
        const buckets = Array.from({ length: 12 }, (_, k) => ({ r: 0, g: 0, b: 0, w: 0, k }));
        for (let i = 0; i < d.length; i += 4) {
          const [h, s, l] = rgbToHsl(d[i], d[i + 1], d[i + 2]);
          const w = s * Math.max(0, 1 - Math.abs(l - 0.5) * 1.8);
          if (w < 0.05) continue;
          const bk = buckets[Math.floor(h * 12) % 12];
          bk.r += d[i] * w; bk.g += d[i + 1] * w; bk.b += d[i + 2] * w; bk.w += w;
        }
        const order = buckets.filter((b) => b.w > 0).sort((a, b) => b.w - a.w);
        if (!order.length) { reject(new Error("no colour")); return; }
        const first = order[0];
        const apart = (a, b) => Math.min(Math.abs(a - b), 12 - Math.abs(a - b));
        const second = order.find((b) => apart(b.k, first.k) >= 2 && b.w >= first.w * 0.15);
        const theme = document.documentElement.dataset.theme;
        const lightTheme = theme === "light" || theme === "paper";
        const tune = (b, shade) => {
          let [h, s, l] = rgbToHsl(b.r / b.w, b.g / b.w, b.b / b.w);
          s = Math.max(s, 0.5);
          l = lightTheme ? Math.min(Math.max(l, 0.3), 0.45) : Math.min(Math.max(l, 0.58), 0.72);
          if (shade) l = lightTheme ? Math.min(0.55, l + 0.1) : Math.max(0.45, l - 0.13);
          return hslToHex(h, s, l);
        };
        resolve([tune(first), second ? tune(second) : tune(first, true)]);
      } catch (e) {
        reject(e);
      }
    };
    img.onerror = () => reject(new Error("cover could not be read"));
    img.src = url;
  });
}

async function updateArtAccent(t) {
  const tok = ++artToken;
  if (!prefs.artAccent) { artColors = null; applyPrefs(); return; }
  const url = t ? artUrl(t, "t120x120") : "";
  if (!url) { artColors = null; applyPrefs(); return; }
  try {
    const [c1, c2] = await artPalette(url);
    if (tok !== artToken || !prefs.artAccent) return;
    artColors = [c1, c2];
    setAccentColors(c1, c2);
  } catch (e) {
    if (tok === artToken) { artColors = null; applyPrefs(); }
  }
}

// ---------- listening stats (kept in this app's storage on your PC) ----------
const STATS_KEY = "sc.stats";
let session = null;   // what's being listened to right now

function loadStats() {
  try { return JSON.parse(localStorage.getItem(STATS_KEY) || "[]"); } catch (e) { return []; }
}
function saveStats(list) {
  try { localStorage.setItem(STATS_KEY, JSON.stringify(list.slice(-5000))); } catch (e) {}
}

function countPlay(t) {
  const list = loadStats();
  list.push({
    ts: Date.now(),
    id: t.id,
    title: t.title || "Untitled",
    artist: artistName(t),
    ms: t.duration || 0,
  });
  saveStats(list);
  if (view === "stats") renderStats();
}

function topBy(list, keyFn) {
  const counts = new Map();
  list.forEach((x) => { const k = keyFn(x); if (k) counts.set(k, (counts.get(k) || 0) + 1); });
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 10);
}

function statBlock(title, rows) {
  const box = el("div", "statBlock");
  box.append(el("h2", "", title));
  if (!rows.length) { box.append(el("p", "statsSub", "Nothing yet.")); return box; }
  const max = rows[0][1];
  const ol = el("ol");
  rows.forEach(([name, count]) => {
    const li = el("li", "statRow");
    const bar = el("div", "statBar");
    bar.style.width = Math.max(4, Math.round((count / max) * 100)) + "%";
    li.append(el("span", "statName", name), el("span", "statCount", String(count)), bar);
    ol.append(li);
  });
  box.append(ol);
  return box;
}

function renderStats() {
  const box = $("statsView");
  box.replaceChildren();
  const all = loadStats();
  const start = new Date();
  start.setDate(1);
  start.setHours(0, 0, 0, 0);
  const month = all.filter((x) => x.ts >= start.getTime());
  const hours = month.reduce((n, x) => n + (x.ms || 0), 0) / 3600000;
  box.append(el("h1", "", "Your listening"));
  box.append(el("p", "statsSub",
    month.length + (month.length === 1 ? " play" : " plays") + " this month, about " + hours.toFixed(1) +
    " hours. " + all.length + " plays recorded in total."));
  box.append(statBlock("Top artists this month", topBy(month, (x) => x.artist)));
  box.append(statBlock("Top tracks this month", topBy(month, (x) => x.title + (x.artist ? " (" + x.artist + ")" : ""))));
  box.append(statBlock("Top artists of all time", topBy(all, (x) => x.artist)));
  const clear = el("button", "soft", "Clear listening history");
  clear.addEventListener("click", () => {
    if (confirm("Clear all listening history? This can't be undone.")) {
      saveStats([]);
      renderStats();
    }
  });
  box.append(clear);
}

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
  const m = title.match(/^(.+?)\s+[-\u2013\u2014]\s+(.+)$/);
  if (m) { artist = m[1].trim(); title = m[2].trim(); }
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
  if (!t || srcOf(t) === "spotify") { box.hidden = true; lyricsFor = ""; lyricsLines = null; return; }
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
  highlightLyric(audio.currentTime);
}

// ---------- Discord activity ----------
let presenceTimer = null;

function presenceUpdate() {
  clearTimeout(presenceTimer);
  presenceTimer = setTimeout(sendPresence, 800);   // Discord limits how often this can change
}

let presenceLastMsg = "";

function presenceNote(msg, isError) {
  $("presNote").textContent = msg;
  if (isError && msg !== presenceLastMsg) say("Discord: " + msg);
  presenceLastMsg = msg;
}

async function sendPresence() {
  const t = curTrack();
  if (!prefs.presence || !t || srcOf(t) === "spotify" || audio.paused) {
    invoke("presence_clear").catch(() => {});
    return;
  }
  try {
    const now = Math.floor(Date.now() / 1000);
    const start = now - Math.floor(audio.currentTime || 0);
    const end = t.duration ? start + Math.floor(t.duration / 1000) : null;
    const note = await invoke("presence_set", {
      details: t.title || "Unknown",
      state: artistName(t) || "Unknown",
      start,
      end,
      image: srcOf(t) === "local" ? null : (artUrl(t, "t500x500") || null),
      url: t.sharing === "private" ? null : (t.permalink_url || null),
    });
    presenceNote(note || "Showing on your Discord profile, with a link to the track.", false);
  } catch (e) {
    presenceNote(String(e), true);
  }
}

// ---------- Last.fm ----------
let lfmConnected = false;
const lfmActive = () => lfmConnected && prefs.lfmOn !== false;

async function lfmRefresh() {
  try {
    const s = await invoke("lastfm_status");
    lfmConnected = !!s.connected;
    $("lastfmSection").hidden = !(lfmConnected && prefs.lfmOn !== false);
    $("lfmOn").checked = prefs.lfmOn !== false;
    $("lfmNote").textContent = lfmConnected
      ? "Signed in as " + (s.name || "your account") + "."
      : "Not signed in. Needs lastfm_api_key and lastfm_secret in config.json.";
    $("lfmConnect").textContent = lfmConnected ? "Sign out" : "Sign in";
  } catch (e) {}
}

async function lfmConnectClick() {
  if (lfmConnected) {
    await invoke("lastfm_disconnect");
    await lfmRefresh();
    return;
  }
  $("lfmNote").textContent = "Approve the app in your browser, then come back here...";
  try {
    await invoke("lastfm_connect");
  } catch (e) {
    $("lfmNote").textContent = String(e);
    return;
  }
  await lfmRefresh();
}

function lfmTrackParams(t) {
  const { artist, title } = guessNames(t);
  const p = [["artist", artist || "Unknown"], ["track", title || t.title || "Unknown"]];
  if (t.duration) p.push(["duration", String(Math.round(t.duration / 1000))]);
  return p;
}

function lfmNowPlaying(t) {
  if (!lfmActive() || !prefs.scrobble || srcOf(t) === "spotify") return;
  invoke("lastfm_call", { method: "track.updateNowPlaying", params: lfmTrackParams(t) }).catch(() => {});
}

function lfmScrobble(t, startedAt) {
  if (!lfmActive() || !prefs.scrobble || srcOf(t) === "spotify") return;
  const params = lfmTrackParams(t);
  params.push(["timestamp", String(Math.floor(startedAt / 1000))]);
  invoke("lastfm_call", { method: "track.scrobble", params }).catch((e) => say("Scrobble failed: " + e));
}

// Discover > Similar artists: Last.fm finds related artists, SoundCloud supplies their tracks
async function loadSimilar() {
  const tok = ++loadToken;
  if (prefs.lfmOn === false) {
    viewTracks = [];
    render();
    say("Last.fm is switched off in Settings, so Similar artists is unavailable.");
    return;
  }
  say("Finding similar artists...");
  try {
    const cur = curTrack();
    let seed = cur && srcOf(cur) === "soundcloud" ? guessNames(cur).artist : "";
    if (!seed) {
      const likes = asList(await api("/me/likes/tracks?limit=30&access=playable&linked_partitioning=true"));
      if (!likes.length) throw new Error("Play or like something first, and this will suggest similar artists.");
      seed = guessNames(shuffleInPlace(likes.slice())[0]).artist;
    }
    const res = await invoke("lastfm_call", { method: "artist.getSimilar", params: [["artist", seed], ["limit", "6"]] });
    const names = ((res.similarartists && res.similarartists.artist) || []).map((a) => a.name).slice(0, 6);
    if (!names.length) throw new Error("Last.fm doesn't know any artists similar to " + seed + ".");
    const lists = await Promise.all(names.map((n) =>
      api("/tracks?q=" + encodeURIComponent(n) + "&access=playable&limit=5&linked_partitioning=true").then(asList).catch(() => [])
    ));
    if (tok !== loadToken) return;
    const seen = new Set();
    const out = [];
    lists.flat().forEach((t) => { if (!seen.has(t.id)) { seen.add(t.id); out.push(t); } });
    viewTracks = recFilter(out.filter((t) => !isMix(t)));
    nextHref = "";
    discoverNote = "Similar to " + seed;
    render();
    say(out.length ? "" : "No matching tracks on SoundCloud.");
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    nextHref = "";
    render();
    say(String(e));
  }
}


// ---------- loudness leveling ----------
// Measures how loud the track is, slowly rides a gain toward a target level, then a
// compressor smooths what's left and a limiter stops peaks clipping. It's built from the
// browser engine's own audio nodes, so it costs very little.
const NORM_PRESETS = {
  gentle: { target: -18, maxBoost: 9, maxCut: 12, thr: -20, ratio: 3, knee: 12, attack: 0.02, release: 0.3 },
  strong: { target: -15, maxBoost: 12, maxCut: 15, thr: -26, ratio: 5, knee: 10, attack: 0.008, release: 0.2 },
};
const norm = { ctx: null, agc: null, comp: null, master: null, analyser: null, buf: null, pow: 0, gainDb: 0, fastUntil: 0 };

// Once the player is routed through the leveler it stays routed; "Off" simply sets it flat.
function normEnsure() {
  if (norm.ctx) return true;
  try {
    const ctx = new (window.AudioContext || window.webkitAudioContext)();
    const src = ctx.createMediaElementSource(audio);
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 2048;
    const agc = ctx.createGain();
    const comp = ctx.createDynamicsCompressor();
    const limiter = ctx.createDynamicsCompressor();
    limiter.threshold.value = -1.5;
    limiter.knee.value = 0;
    limiter.ratio.value = 20;
    limiter.attack.value = 0.002;
    limiter.release.value = 0.12;
    const master = ctx.createGain();
    src.connect(analyser);   // measure the track as it arrives
    // 10-band equalizer (flat unless switched on)
    const eq = EQ_FREQS.map((f, i) => {
      const b = ctx.createBiquadFilter();
      b.type = i === 0 ? "lowshelf" : i === EQ_FREQS.length - 1 ? "highshelf" : "peaking";
      b.frequency.value = f;
      b.Q.value = 1.1;
      b.gain.value = 0;
      return b;
    });
    let node = src;
    eq.forEach((b) => { node.connect(b); node = b; });
    node.connect(agc);
    agc.connect(comp);
    comp.connect(limiter);
    limiter.connect(master);
    master.connect(ctx.destination);
    Object.assign(norm, { ctx, agc, comp, master, analyser, buf: new Float32Array(analyser.fftSize) });
    norm.eq = eq;
    if (prefs.sink && prefs.sink !== "default" && ctx.setSinkId) ctx.setSinkId(prefs.sink).catch(() => {});
    normApply();
    return true;
  } catch (e) {
    say("Could not start loudness leveling: " + e);
    return false;
  }
}

function normApply() {
  if (!norm.ctx) return;
  const p = NORM_PRESETS[prefs.normalize];
  const now = norm.ctx.currentTime;
  const c = norm.comp;
  if (p) {
    c.threshold.setValueAtTime(p.thr, now);
    c.ratio.setValueAtTime(p.ratio, now);
    c.knee.setValueAtTime(p.knee, now);
    c.attack.setValueAtTime(p.attack, now);
    c.release.setValueAtTime(p.release, now);
  } else {
    c.threshold.setValueAtTime(0, now);
    c.ratio.setValueAtTime(1, now);
    norm.gainDb = 0;
    norm.agc.gain.setTargetAtTime(1, now, 0.1);
  }
  setLevel(prefs.volume);
  eqApply();
}

// The volume slider sits after the leveler, so leveling doesn't fight it
function setLevel(v) {
  if (norm.ctx) {
    audio.volume = 1;
    norm.master.gain.setTargetAtTime(audio.muted ? 0 : v, norm.ctx.currentTime, 0.02);
  } else {
    audio.volume = v;
  }
}

function normMute(muted) {
  if (norm.ctx) norm.master.gain.setTargetAtTime(muted ? 0 : prefs.volume, norm.ctx.currentTime, 0.02);
}

function normTick() {
  if (!norm.ctx || audio.paused) return;
  const p = NORM_PRESETS[prefs.normalize];
  if (!p) return;
  norm.analyser.getFloatTimeDomainData(norm.buf);
  let sum = 0;
  for (let i = 0; i < norm.buf.length; i++) sum += norm.buf[i] * norm.buf[i];
  const fast = performance.now() < norm.fastUntil;
  norm.pow = norm.pow * (fast ? 0.6 : 0.9) + (sum / norm.buf.length) * (fast ? 0.4 : 0.1);
  const db = 10 * Math.log10(norm.pow + 1e-12);
  if (db < -50) return;   // silence or a very quiet intro: leave the gain where it is
  const want = Math.max(-p.maxCut, Math.min(p.maxBoost, p.target - db));
  norm.gainDb += (want - norm.gainDb) * (fast ? 0.25 : 0.04);
  norm.agc.gain.setTargetAtTime(Math.pow(10, norm.gainDb / 20), norm.ctx.currentTime, 0.25);
}

// ---------- recommendation language ----------
// SoundCloud doesn't record a track's language, so this guesses from the title and tags:
// other alphabets, accented letters and common words. Tracks it can't place are kept.
const LANGS = {
  en: { words: "the and you your my me i'm im don't dont with for of in on we is it be feel never night baby heart all this that what when like just got love so up down", chars: "" },
  es: { words: "el la los las que y de con para por mi tu te amor corazón corazon noche quiero sin una eres como más mas bebé nada vida mía mia contigo", chars: "ñ¿¡áéíóú" },
  pt: { words: "não nao você voce meu minha com pra para uma amor coração coracao sem é eu te saudade quero vida também tambem", chars: "ãõçâêô" },
  fr: { words: "le la les de des et je tu pour avec sans mon ma toi moi amour coeur cœur nuit c'est est pas une", chars: "àèùçœëêîï" },
  de: { words: "der die das und ich nicht mit für fur dich mich liebe herz nacht ein eine ist du wir", chars: "äöüß" },
};
Object.values(LANGS).forEach((L) => { L.set = new Set(L.words.split(" ")); });
const OTHER_SCRIPT = /[\u0370-\u03ff\u0400-\u04ff\u0590-\u05ff\u0600-\u06ff\u0900-\u097f\u0e00-\u0e7f\u3040-\u30ff\u3400-\u9fff\uac00-\ud7af]/;

function detectLang(t) {
  const text = ((t.title || "") + " " + (t.tag_list || "")).toLowerCase();
  if (OTHER_SCRIPT.test(text)) return "other";
  const words = text.split(/[^a-z\u00c0-\u024f\u0153']+/).filter(Boolean);
  let best = "unknown";
  let bestScore = 0;
  for (const [code, L] of Object.entries(LANGS)) {
    let s = 0;
    words.forEach((w) => { if (L.set.has(w)) s += 1; });
    for (const ch of text) if (L.chars && L.chars.includes(ch)) s += 2;
    if (s > bestScore) { bestScore = s; best = code; }
  }
  if (bestScore === 0) return "unknown";
  return bestScore >= 2 ? best : best + "?";   // "?" = only a weak hint
}

let recHidden = 0;

function recFilter(list) {
  list = list.filter((t) => !isLikedTrack(t) && !hiddenRecs.has(String(t.id)));
  const want = prefs.recLang || "any";
  if (want === "any") return list;
  const kept = list.filter((t) => {
    const d = detectLang(t);
    return d === "unknown" || d.endsWith("?") || d === want;
  });
  recHidden += list.length - kept.length;
  return kept;
}


// ---------- tidy titles: drop promo tags like "FREE DL" ----------
const JUNK_RE = /\b(?:free\s*(?:dl|d\/l|download|dwnld)|free\s*=?\s*buy|buy\s*=\s*free|click\s*buy|out\s*now|premiere|exclusive|supported\s+by\b.*|played\s+by\b.*|download\s+(?:link|in\s+desc\w*)|link\s+in\s+(?:bio|desc\w*)|hypeddit|toneden)/i;
const JUNK_G = new RegExp(JUNK_RE.source, "gi");
const SEP_JUNK = /\b(?:free\s*(?:dl|d\/l|download|dwnld)|click\s*buy|buy\s*=\s*free|out\s*now|supported\s+by|played\s+by|link\s+in)\b/i;
const KEEP_RE = /\b(?:re-?mix|mix|edit|bootleg|flip|vip|feat\.?|ft\.?|prod\.?|rework|cover|version|instrumental|acapella|live|remaster\w*)\b/i;
const ALWAYS_JUNK_G = /\s*\b(?:free\s*(?:dl|d\/l|download|dwnld)|click\s*buy|buy\s*=\s*free|free\s*=\s*buy|out\s*now|link\s+in\s+(?:bio|desc\w*))\b[!.]*/gi;

function cleanTitle(title) {
  if (!title) return title;
  let t = String(title);
  // "Premiere: Artist - Title"
  t = t.replace(/^\s*(?:premiere|exclusive)\s*[:|\-\u2013\u2014]\s*/i, "");
  // bracketed tags: (FREE DL), [OUT NOW], {Premiere}; "(Exclusive Remix)" keeps "(Remix)"
  t = t.replace(/[\(\[\{\u3010]([^\)\]\}\u3011]*)[\)\]\}\u3011]/g, (m, inner) => {
    if (!JUNK_RE.test(inner)) return m;
    if (!KEEP_RE.test(inner)) return " ";
    const kept = inner.replace(JUNK_G, " ").replace(/\s{2,}/g, " ").trim();
    return kept ? m[0] + kept + m[m.length - 1] : " ";
  });
  // promo text after a separator: "Title - FREE DL", "Title | Supported by ..."
  t = t.replace(/\s+(?:\||\/\/|-{1,2}|\u2013|\u2014|~)\s+([^|\-\u2013\u2014~]+)$/, (m, rest) =>
    SEP_JUNK.test(rest) && !KEEP_RE.test(rest) ? "" : m);
  // **FREE DOWNLOAD**
  t = t.replace(/\*+([^*]*)\*+/g, (m, inner) => (JUNK_RE.test(inner) ? " " : m));
  // phrases that are never part of a title
  t = t.replace(ALWAYS_JUNK_G, " ");
  t = t.replace(/\s{2,}/g, " ").replace(/\s+([,\)\]])/g, "$1")
    .replace(/[\s\-\u2013\u2014|~:]+$/, "").replace(/^[\s\-\u2013\u2014|~:]+/, "").trim();
  return t || String(title);
}

function tidyTrack(t) {
  if (t && t.title) t.title = cleanTitle(t.title);
  if (t && t.title && !t.displayArtist) splitArtist(t);
  if (t) applyFix(t);
  return t;
}

// ---------- your own title / artist fixes (right-click > Edit title and artist) ----------
function applyFix(t) {
  const fx = (prefs.trackFixes || {})[t.id];
  if (!t._auto) t._auto = { title: t.title, artist: t.displayArtist || null };
  if (!fx) return;
  if (fx.title) t.title = fx.title;
  if (fx.artist) t.displayArtist = fx.artist;
}

// Every copy of a track the app is holding (lists, queue, likes) gets the change
function forEachCopy(id, fn) {
  const seen = new Set();
  [viewTracks, queue, likedTracks].forEach((list) => list.forEach((x) => {
    if (x && x.id === id && !seen.has(x)) { seen.add(x); fn(x); }
  }));
}

function refreshAfterFix(id) {
  render();
  renderQueue();
  const cur = curTrack();
  if (cur && cur.id === id) { fillNowPlaying(cur); presenceUpdate(); widgetPush(); }
}

let fixTarget = null;

// A guess for titles that are really file names, e.g. "thescript-forthefirsttime(8barzremix).mp3"
function guessFromFilename(t) {
  let s = (t.title || "").replace(/\.(mp3|wav|flac|m4a|aac|ogg|aiff?)$/i, "").replace(/_/g, " ").trim();
  const m = s.match(/^([^-]+?)\s*-\s*(.+)$/);
  return m ? { artist: m[1].trim(), title: m[2].trim() } : { artist: artistName(t), title: s };
}

function openFixDialog(t) {
  fixTarget = t;
  const fx = (prefs.trackFixes || {})[t.id];
  const looksLikeFile = /\.(mp3|wav|flac|m4a|aac|ogg|aiff?)$/i.test(t.title || "") || (!/\s/.test(t.title || "") && /[-_]/.test(t.title || ""));
  const guess = !fx && looksLikeFile ? guessFromFilename(t) : { title: t.title, artist: artistName(t) };
  $("fixTitle").value = guess.title || "";
  $("fixArtist").value = guess.artist || "";
  $("fixReset").hidden = !fx;
  $("fixDialog").showModal();
  $("fixTitle").focus();
  $("fixTitle").select();
}

function saveFix(e) {
  e.preventDefault();
  const t = fixTarget;
  if (!t) return;
  const title = $("fixTitle").value.trim();
  const artist = $("fixArtist").value.trim();
  prefs.trackFixes = prefs.trackFixes || {};
  prefs.trackFixes[t.id] = { title: title || t._auto.title, artist: artist || null };
  savePrefs();
  forEachCopy(t.id, (x) => {
    if (!x._auto) x._auto = { title: x.title, artist: x.displayArtist || null };
    x.title = title || x._auto.title;
    x.displayArtist = artist || x._auto.artist || undefined;
  });
  $("fixDialog").close();
  refreshAfterFix(t.id);
  say("Saved. It will show like this from now on.");
}

function resetFix() {
  const t = fixTarget;
  if (!t) return;
  if (prefs.trackFixes) delete prefs.trackFixes[t.id];
  savePrefs();
  forEachCopy(t.id, (x) => {
    if (x._auto) { x.title = x._auto.title; x.displayArtist = x._auto.artist || undefined; }
  });
  $("fixDialog").close();
  refreshAfterFix(t.id);
  say("Back to the original title and artist.");
}

// "Artist - Title": show the artist from the title instead of the uploader, and drop it from the title.
// "Song - Extended Mix" is left alone, since the part after the dash is just a version name.
const DESCRIPTOR_ONLY = /^[^()\[\]]*\b(?:mix|edit|remix|version|vip|bootleg|flip|rework|dub|instrumental|acapella|remaster(?:ed)?)\s*$/i;

function splitArtist(t) {
  const m = t.title.match(/^(.+?)\s+[-\u2013\u2014]\s+(.+)$/);
  if (!m) return;
  const left = m[1].trim();
  const right = m[2].trim();
  if (!left || !right || left.length > 80 || /[\(\[]/.test(left)) return;
  // "Song - Extended Mix" is a version name, but "Artist - Song - VIP" still has a song in it
  if (!/\s[-\u2013\u2014]\s/.test(right) && DESCRIPTOR_ONLY.test(right)) return;
  t.displayArtist = left;
  t.title = right;
}

// The artist shown for a track (from the title when there is one, otherwise the uploader)
function artistName(t) {
  return (t && (t.displayArtist || (t.user && t.user.username))) || "";
}

// Hover text on the artist: who actually uploaded it, when that's someone else
function uploaderNote(t) {
  const up = t && t.user && t.user.username;
  if (t && t.displayArtist && up && up.toLowerCase() !== t.displayArtist.toLowerCase()) {
    return "Uploaded by " + up + ". Click to open their page.";
  }
  return "Open artist page";
}

// ---------- mixes: anything longer than 15 minutes ----------
const MIX_MS = 15 * 60 * 1000;
const isMix = (t) => !!t && (t.duration || 0) > MIX_MS;
// Mixes stay out of these; they still show in Likes (under Mixes) and playlists
const hideMixesIn = (v) => v === "search" || v === "discover" || v === "artist";

// Follows next_href for up to "pages" pages and returns all the tracks
async function collectPages(path, pages) {
  let out = [];
  let next = path;
  for (let i = 0; i < pages && next; i++) {
    const data = await api(next);
    out = out.concat(asList(data));
    next = !Array.isArray(data) && data.next_href ? toPath(data.next_href) : "";
  }
  return out;
}

// ---------- your own playlists (saved to your SoundCloud account, private) ----------
const isOwnPlaylist = (p) => !!p && !!p.user && meId !== null && String(p.user.id) === String(meId);

async function playlistTrackIds(id) {
  const p = await api("/playlists/" + id + "?show_tracks=true");
  return ((p && p.tracks) || []).map((t) => t.id).filter(Boolean);
}

async function savePlaylistTracks(id, ids) {
  await invoke("playlist_write", {
    method: "PUT",
    path: "/playlists/" + id,
    body: { playlist: { tracks: ids.map((x) => ({ id: x })) } },
  });
}

async function createPlaylist(title, firstTrack) {
  const body = { playlist: { title, sharing: "private", tracks: firstTrack ? [{ id: firstTrack.id }] : [] } };
  const created = await invoke("playlist_write", { method: "POST", path: "/playlists", body });
  await loadPlaylists();
  return created;
}

async function addToPlaylist(p, t) {
  try {
    const ids = await playlistTrackIds(p.id);
    if (ids.includes(t.id)) { say("Already in " + (p.title || "that playlist") + "."); return; }
    ids.push(t.id);
    await savePlaylistTracks(p.id, ids);
    say("Added to " + (p.title || "playlist") + ".");
    if (view === "playlist" && currentPlaylist && currentPlaylist.id === p.id) openPlaylist(currentPlaylist);
  } catch (e) {
    say("Could not add to the playlist: " + e);
  }
}

async function removeFromPlaylist(t) {
  const p = currentPlaylist;
  if (!p || !t) return;
  try {
    const ids = (await playlistTrackIds(p.id)).filter((x) => x !== t.id);
    await savePlaylistTracks(p.id, ids);
    viewTracks = viewTracks.filter((x) => x.id !== t.id);
    render();
    say("Removed from " + (p.title || "the playlist") + ".");
  } catch (e) {
    say("Could not remove the track: " + e);
  }
}

async function deletePlaylist() {
  const p = currentPlaylist;
  if (!p || !confirm('Delete the playlist "' + (p.title || "Untitled") + '" from your SoundCloud account?')) return;
  try {
    await invoke("playlist_write", { method: "DELETE", path: "/playlists/" + p.id, body: null });
    await loadPlaylists();
    openLikes();
    say("Playlist deleted.");
  } catch (e) {
    say("Could not delete the playlist: " + e);
  }
}

let menuTrack = null;

function closeMenu() {
  $("plMenu").hidden = true;
  menuTrack = null;
}

function placeMenu(anchor) {
  const m = $("plMenu");
  const r = anchor.getBoundingClientRect();
  const w = m.offsetWidth;
  const h = m.offsetHeight;
  m.style.left = Math.max(8, Math.min(window.innerWidth - w - 8, r.right - w)) + "px";
  m.style.top = (r.bottom + h + 8 > window.innerHeight ? Math.max(8, r.top - h - 4) : r.bottom + 4) + "px";
}

function openPlaylistMenu(t, anchor) {
  menuTrack = t;
  const m = $("plMenu");
  m.replaceChildren(el("div", "menuHead", "Add to playlist"));
  const mine = playlists.filter(isOwnPlaylist);
  mine.forEach((p) => {
    const b = el("button", "menuItem", p.title || "Untitled");
    b.dataset.id = p.id;
    m.append(b);
  });
  if (!mine.length) m.append(el("div", "menuNote", "You have no playlists yet."));
  const nb = el("button", "menuItem menuNew", "New playlist...");
  nb.dataset.new = "1";
  m.append(nb);
  m.hidden = false;
  placeMenu(anchor);
  m.dataset.anchor = "";
  m._anchor = anchor;
}

function menuNewPlaylist() {
  const m = $("plMenu");
  const track = menuTrack;
  m.replaceChildren(el("div", "menuHead", "New playlist"));
  const f = el("form");
  const inp = el("input");
  inp.placeholder = "Playlist name";
  inp.maxLength = 100;
  f.append(inp);
  m.append(f);
  if (m._anchor) placeMenu(m._anchor);
  inp.focus();
  f.addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = inp.value.trim();
    if (!name) return;
    closeMenu();
    try {
      await createPlaylist(name, track);
      say('Created "' + name + '"' + (track ? " with that track in it." : "."));
    } catch (err) {
      say("Could not create the playlist: " + err);
    }
  });
}

function playlistsInit() {
  $("plMenu").addEventListener("click", (e) => {
    const b = e.target.closest(".menuItem");
    if (!b) return;
    if (b.dataset.action) { runMenuAction(b.dataset.action); return; }
    if (b.dataset.new) { menuNewPlaylist(); return; }
    const p = playlists.find((x) => String(x.id) === b.dataset.id);
    const t = menuTrack;
    closeMenu();
    if (p && t) addToPlaylist(p, t);
  });
  document.addEventListener("mousedown", (e) => {
    const m = $("plMenu");
    if (!m.hidden && !m.contains(e.target) && !e.target.closest('[data-act="plist"]')) closeMenu();
  });
  document.addEventListener("keydown", (e) => { if (e.key === "Escape") closeMenu(); });
  $("listView").addEventListener("scroll", () => { if (!$("plMenu").hidden) closeMenu(); });

  // Sidebar "+": name it, press Enter
  $("plNew").addEventListener("click", () => {
    const f = $("plNewForm");
    f.hidden = !f.hidden;
    if (!f.hidden) { $("plNewName").value = ""; $("plNewName").focus(); }
  });
  $("plNewName").addEventListener("keydown", (e) => { if (e.key === "Escape") $("plNewForm").hidden = true; });
  $("plNewForm").addEventListener("submit", async (e) => {
    e.preventDefault();
    const name = $("plNewName").value.trim();
    if (!name) return;
    $("plNewForm").hidden = true;
    try {
      const created = await createPlaylist(name, null);
      const p = (created && playlists.find((x) => x.id === created.id)) || created;
      if (p && p.id) openPlaylist(p);
      say('Created "' + name + '". Add tracks with the playlist button on any track.');
    } catch (err) {
      say("Could not create the playlist: " + err);
    }
  });
  $("plDelete").addEventListener("click", deletePlaylist);

  // Likes: All / Tracks / Mixes
  $("likesChips").addEventListener("click", (e) => {
    const b = e.target.closest(".chip");
    if (!b) return;
    likesFilter = b.dataset.f;
    openLikes();
  });
}


// ---------- now playing widget for OBS ----------
const WIDGET_NOTE = "A now-playing card for OBS or other streaming software. It only shows what's playing; it doesn't play audio.";

let widgetReady = false;
let widgetCleared = false;   // "Clear" hides the card until the next track starts

function widgetState() {
  const t = curTrack();
  if (!t || widgetCleared) return { playing: false };
  const spot = srcOf(t) === "spotify";
  return {
    playing: spot ? spState.playing : !audio.paused,
    title: t.title || "",
    artist: artistName(t),
    artwork: artUrl(t, "t500x500") || "",
    url: t.permalink_url || "",
    platform: srcOf(t),
    duration_ms: t.duration || (isFinite(audio.duration) ? audio.duration * 1000 : 0),
    position_ms: spot ? spotifyPos() : (audio.currentTime || 0) * 1000,
    updated_at: Date.now(),
    accent: accentNow[0],
    accent2: accentNow[1],
    design: prefs.widgetDesign || "card",
    rounded: prefs.widgetRounded !== false,
    text_scale: prefs.widgetText || 1,
    anim: {
      fade: prefs.wFade !== false,
      scroll: prefs.wScroll !== false,
      spin: prefs.wSpin !== false,
      color: prefs.wColor === undefined ? 1.2 : Number(prefs.wColor),
    },
  };
}

function widgetPush() {
  if (prefs.widget || prefs.req) invoke("widget_update", { state: widgetState() }).catch(() => {});
}

async function widgetApply() {
  const on = !!prefs.widget;
  $("widgetSection").hidden = !on;
  if (!on) {
    invoke("widget_stop").catch(() => {});
    $("widgetNote").textContent = WIDGET_NOTE;
    return;
  }
  try {
    const url = await invoke("widget_start", { port: Number(prefs.widgetPort) || 7878 });
    $("widgetUrl").value = url;
    $("widgetNote").textContent = WIDGET_NOTE;
    widgetPush();
  } catch (e) {
    $("widgetUrl").value = "";
    $("widgetNote").textContent = String(e);
  }
}

function widgetInit() {
  $("widgetToggle").checked = !!prefs.widget;
  $("widgetPort").value = prefs.widgetPort || 7878;
  $("widgetDesign").value = prefs.widgetDesign || "card";
  $("widgetRounded").checked = prefs.widgetRounded !== false;
  $("widgetDesign").addEventListener("change", () => {
    prefs.widgetDesign = $("widgetDesign").value;
    savePrefs();
    widgetPush();
  });
  $("widgetRounded").addEventListener("change", () => {
    prefs.widgetRounded = $("widgetRounded").checked;
    savePrefs();
    widgetPush();
  });
  $("widgetToggle").addEventListener("change", () => {
    prefs.widget = $("widgetToggle").checked;
    savePrefs();
    widgetApply();
  });
  $("widgetPort").addEventListener("change", () => {
    const p = Math.round(Number($("widgetPort").value));
    if (p >= 1024 && p <= 65535) { prefs.widgetPort = p; savePrefs(); widgetApply(); }
    else $("widgetPort").value = prefs.widgetPort || 7878;
  });
  $("widgetClear").addEventListener("click", () => {
    widgetCleared = true;
    widgetPush();
  });
  $("widgetCopy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("widgetUrl").value);
      $("widgetCopy").textContent = "Copied";
      setTimeout(() => { $("widgetCopy").textContent = "Copy"; }, 1500);
    } catch (e) {
      $("widgetUrl").select();
    }
  });
  audio.addEventListener("seeked", widgetPush);
  setInterval(() => { if ((prefs.widget || prefs.req) && !audio.paused) widgetPush(); }, 5000);
  const paintText = () => { $("widgetTextVal").textContent = Math.round((prefs.widgetText || 1) * 100) + "%"; };
  const stepText = (d) => {
    prefs.widgetText = Math.min(1.6, Math.max(0.7, Math.round(((prefs.widgetText || 1) + d) * 10) / 10));
    savePrefs();
    paintText();
    widgetPush();
  };
  $("widgetTextDown").addEventListener("click", () => stepText(-0.1));
  $("widgetTextUp").addEventListener("click", () => stepText(0.1));
  paintText();
  widgetReady = true;
  widgetApply();
}

// ---------- long titles scroll on hover ----------
function marqueeInit() {
  document.addEventListener("mouseover", (e) => {
    const box = e.target.closest(".title");
    if (!box || box.classList.contains("marqOn")) return;
    const over = box.scrollWidth - box.clientWidth;
    if (over <= 2) return;
    let span = box.firstElementChild;
    if (!span || !span.classList.contains("marqText")) {
      span = el("span", "marqText", box.textContent);
      box.replaceChildren(span);
    }
    span.style.setProperty("--shift", -(over + 6) + "px");
    span.style.setProperty("--dur", Math.max(4, over / 40 + 3) + "s");
    box.classList.add("marqOn");
    box.addEventListener("mouseleave", () => box.classList.remove("marqOn"), { once: true });
  });
}

// ---------- fades between tracks ----------
// Fade the old cover out, wait for the new one to load, fade it in
function fadeImg(img, src) {
  img._tok = (img._tok || 0) + 1;
  const tok = img._tok;
  if (!src) { img.removeAttribute("src"); img.classList.remove("artFade"); return; }
  if (img.getAttribute("src") === src) return;
  if (!img.getAttribute("src")) { img.src = src; return; }
  img.classList.add("artFade");
  const started = performance.now();
  const swap = () => {
    setTimeout(() => {
      if (tok !== img._tok) return;
      img.src = src;
      requestAnimationFrame(() => img.classList.remove("artFade"));
    }, Math.max(0, 250 - (performance.now() - started)));
  };
  const pre = new Image();
  pre.onload = swap;
  pre.onerror = swap;
  pre.src = src;
}

function fadeIn(node) {
  if (!node) return;
  node.classList.remove("fadeIn");
  void node.offsetWidth;   // restart the animation
  node.classList.add("fadeIn");
}


// ---------- second accent colour ----------
function buildSwatches2() {
  const box = $("swatches2");
  box.replaceChildren();
  const off = el("button", "swatch none");
  off.dataset.c = "";
  off.title = "One colour";
  off.setAttribute("aria-label", "No second accent");
  box.append(off);
  ACCENTS.forEach((c) => {
    const b = el("button", "swatch");
    b.style.background = c;
    b.dataset.c = c;
    b.setAttribute("aria-label", "Second accent " + c);
    box.append(b);
  });
  const custom = el("input");
  custom.type = "color";
  custom.id = "accent2Custom";
  custom.setAttribute("aria-label", "Custom second accent");
  box.append(custom);
}

// Marks the chosen second accent, and greys out both accent rows while album-art colours are on
function syncAccent2() {
  document.querySelectorAll("#swatches2 .swatch").forEach((b) => {
    b.classList.toggle("on", (b.dataset.c || "") === (prefs.accent2 || ""));
  });
  const custom = $("accent2Custom");
  if (custom) custom.value = prefs.accent2 || prefs.accent;
  const art = !!prefs.artAccent;
  ["accentRow", "accent2Row"].forEach((id) => {
    const row = $(id);
    row.classList.toggle("disabledRow", art);
    row.title = art ? "Turn off \"Match accent to album art\" to choose your own colours" : "";
    row.querySelectorAll("button, input").forEach((x) => { x.disabled = art; });
  });
}

function setAccent2(c) {
  prefs.accent2 = c || null;
  savePrefs();
  applyPrefs();
  syncAccent2();
}

// ---------- back button ----------
const navStack = [];
const pageCache = new Map();   // Discover / Rediscover lists, so going back shows the same thing

function useCachedPage() {
  const hit = curPage && pageCache.get(pageKey(curPage));
  if (!hit) return false;
  ++loadToken;   // cancel anything still loading
  viewTracks = hit.tracks.slice();
  nextHref = hit.nextHref;
  localPool = hit.pool || null;
  localPos = hit.pos || 0;
  render();
  say("");
  requestAnimationFrame(() => { $("listView").scrollTop = hit.scroll; });
  return true;
}
let curPage = null;
let restoring = false;

function pageSnapshot(v) {
  return {
    view: v, query, playlist: currentPlaylist, artist: currentArtist,
    discoverSrc, likesFilter, plat: curPlat(v),
  };
}

function pageKey(p) {
  return [p.view, p.query, p.playlist && p.playlist.id, p.artist && p.artist.id, p.discoverSrc, p.likesFilter, p.plat].join("|");
}

// Called from showView: remembers where you were before this page
function navTrack(v) {
  const now = pageSnapshot(v);
  if (curPage && pageKey(curPage) !== pageKey(now) && (curPage.view === "discover" || curPage.view === "rediscover")) {
    pageCache.set(pageKey(curPage), { tracks: viewTracks.slice(), nextHref, scroll: $("listView").scrollTop, pool: localPool, pos: localPos });
  }
  if (curPage && !restoring && pageKey(curPage) !== pageKey(now)) {
    navStack.push(curPage);
    if (navStack.length > 50) navStack.shift();
  }
  curPage = now;
  albumsOnPage(now);
  $("backBtn").disabled = navStack.length === 0;
  renderPlatChips();
}

function restorePage(p) {
  likesFilter = p.likesFilter || "all";
  switch (p.view) {
    case "likes": openLikes(); break;
    case "search": if (p.query) openSearch(p.query); else openSearchPage(); break;
    case "playlist": if (p.playlist) openPlaylist(p.playlist); else openLikes(); break;
    case "artist": if (p.artist) openArtist(p.artist); else openLikes(); break;
    case "discover": openDiscover(p.discoverSrc); break;
    case "rediscover": openRediscover(); break;
    case "stats": showView("stats"); break;
    case "now": showView("now"); break;
    default: openLikes();
  }
}

function goBack() {
  const p = navStack.pop();
  if (!p) return;
  restoring = true;
  try { restorePage(p); } finally { restoring = false; }
  $("backBtn").disabled = navStack.length === 0;
}

// ---------- Likes filter: All / Tracks / Mixes ----------
let likesFilter = "all";

function renderLikesChips() {
  const box = $("likesChips");
  box.replaceChildren();
  [["all", "All"], ["tracks", "Tracks"], ["mixes", "Mixes"]].forEach(([k, label]) => {
    const b = el("button", "chip" + (k === likesFilter ? " on" : ""), label);
    b.dataset.f = k;
    box.append(b);
  });
}

function applyLikesFilter(list) {
  if (likesFilter === "mixes") return list.filter(isMix);
  if (likesFilter === "tracks") return list.filter((t) => !isMix(t));
  return list;
}

// ---------- platforms: one switch on each page, showing only what's linked ----------
// To add a platform later: give it an entry here (and an icon in PLATFORMS in app.js),
// then teach loadPlatformList how to fetch its likes / search results.
const PLATFORM_LIST = [
  { id: "soundcloud", pages: ["likes", "search", "discover", "rediscover"], linked: () => prefs.scOn !== false },
  { id: "spotify", pages: ["likes", "search", "discover", "rediscover"], linked: () => FEATURES.spotify && spotifyPremium && prefs.spOn !== false },
  { id: "local", pages: ["likes", "search", "rediscover"], linked: () => prefs.localOn !== false && localTracks.length > 0 },
  { id: "youtube", pages: [], linked: () => false },
  { id: "apple", pages: [], linked: () => false },
  { id: "amazon", pages: [], linked: () => false },
];

function platformsFor(page) {
  return PLATFORM_LIST.filter((p) => p.pages.includes(page) && p.linked());
}

function curPlat(page) {
  const avail = platformsFor(page);
  const want = prefs.globalPlat || "soundcloud";
  if (avail.some((p) => p.id === want)) return want;
  return avail.length ? avail[0].id : "none";
}

// Platform icons in the title bar: one choice for the whole app.
// Pages that don't support the chosen platform (Discover is SoundCloud-only) use what they can.
function renderPlatChips() {
  $("navDiscover").hidden = !platformsFor("discover").length;
  // the sidebar's playlists follow the platform chosen here
  const linkedNow = PLATFORM_LIST.filter((p) => p.pages.length && p.linked());
  const chosen = linkedNow.some((p) => p.id === prefs.globalPlat) ? prefs.globalPlat : (linkedNow[0] ? linkedNow[0].id : "none");
  if (chosen !== sidebarPlat) {
    sidebarPlat = chosen;
    refreshSidebar();
  }
  const box = $("titlePlat");
  const plats = PLATFORM_LIST.filter((p) => p.pages.length && p.linked());
  box.hidden = plats.length < 2;
  if (plats.length < 2) return;
  const cur = plats.some((p) => p.id === prefs.globalPlat) ? prefs.globalPlat : plats[0].id;
  box.replaceChildren();
  plats.forEach((p) => {
    const b = el("button", p.id === cur ? "on" : "");
    b.dataset.p = p.id;
    b.title = PLATFORMS[p.id].name;
    b.setAttribute("aria-label", "Show " + PLATFORMS[p.id].name);
    b.append(icon(PLATFORMS[p.id].icon));
    box.append(b);
  });
}

// Likes / Search / Rediscover for platforms other than SoundCloud
async function loadPlatformList(page, plat, q) {
  const tok = ++loadToken;
  say("Loading...");
  try {
    let list = [];
    if (plat === "spotify") list = page === "search" ? await spotifySearchTracks(q) : await spotifyLikedTracks(500);
    else if (plat === "local") list = page === "search" ? localSearch(q) : localTracks.slice();
    if (tok !== loadToken) return;
    if (page === "likes") list = applyLikesFilter(list);
    if (page === "search") list = list.filter((t) => !isMix(t));
    setLocalPool(list);
    render();
    say(viewTracks.length ? "" : "Nothing found.");
    setTimeout(nearBottomCheck, 50);
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    nextHref = "";
    render();
    say(String(e));
  }
}

// Lists the app already holds are shown 60 at a time and grow as you scroll
let localPool = null;
let localPos = 0;

function setLocalPool(list) {
  localPool = list;
  localPos = Math.min(60, list.length);
  viewTracks = list.slice(0, localPos);
  nextHref = localPos < list.length ? "local:" : "";
}

function pageLocal() {
  if (!localPool) { nextHref = ""; return; }
  const more = localPool.slice(localPos, localPos + 60);
  localPos += more.length;
  viewTracks = viewTracks.concat(more);
  nextHref = localPos < localPool.length ? "local:" : "";
  render();
  setTimeout(nearBottomCheck, 50);
}

// ---------- local music ----------
let localTracks = [];
let localInfo = null;
let localScanTimer = null;
let localFolderList = [];

function localUrl(kind, id) {
  return "http://127.0.0.1:" + localInfo.port + "/" + kind + "?k=" + encodeURIComponent(localInfo.token) + "&id=" + encodeURIComponent(id);
}

function localTrack(e) {
  return tidyTrack({
    source: "local",
    id: "local:" + e.id,
    title: e.title,
    user: { username: e.artist || "Unknown artist" },
    album: e.album,
    duration: e.duration,
    artwork_url: localUrl("cover", e.id),
    url: localUrl("file", e.id),
  });
}

function localSearch(q) {
  const n = q.toLowerCase();
  return localTracks.filter((t) => (t.title + " " + artistName(t) + " " + (t.album || "")).toLowerCase().includes(n));
}

function paintLocal(status) {
  const on = prefs.localOn !== false;
  $("localBody").hidden = !on;
  $("localOn").checked = on;
  const box = $("localFolders");
  box.replaceChildren();
  localFolderList.forEach((f, i) => {
    const row = el("div", "folderRow");
    const rm = el("button", "", "\u00d7");
    rm.title = "Remove this folder";
    rm.setAttribute("aria-label", "Remove " + f);
    rm.dataset.i = i;
    row.append(el("span", "", f), rm);
    row.querySelector("span").title = f;
    box.append(row);
  });
  if (!on) {
    $("localNote").textContent = "Switched off. Your folders are kept for when you switch it back on.";
  } else if (!localFolderList.length) {
    $("localNote").textContent = "Add a folder to get started. Your files appear as a platform on Likes, Search and Rediscover.";
  } else if (status && status.scanning) {
    $("localNote").textContent = "Scanning your folders... " + status.count + " tracks so far.";
  } else {
    $("localNote").textContent = localTracks.length + " tracks from " + localFolderList.length +
      (localFolderList.length === 1 ? " folder." : " folders.");
  }
}

async function localLoad() {
  try {
    localFolderList = await invoke("local_folders");
    if (prefs.localOn === false || !localFolderList.length) {
      localTracks = [];
    } else {
      if (!localInfo) localInfo = await invoke("local_server");
      localTracks = (await invoke("local_tracks")).map(localTrack);
    }
  } catch (e) {
    $("localNote").textContent = String(e);
  }
  paintLocal();
  renderPlatChips();
  // With SoundCloud off, the first page may have opened before your files were ready
  if (view === "likes" && !viewTracks.length && curPlat("likes") === "local") openLikes();
  if (sidebarPlat === "local") refreshSidebar();   // albums come from your files
}

async function localRescan() {
  if (prefs.localOn === false || !localFolderList.length) return;
  try { await invoke("local_scan"); } catch (e) {}
  clearInterval(localScanTimer);
  localScanTimer = setInterval(async () => {
    const st = await invoke("local_status").catch(() => null);
    if (!st) return;
    paintLocal(st);
    if (!st.scanning) {
      clearInterval(localScanTimer);
      localScanTimer = null;
      localLoad();
    }
  }, 1500);
}

async function localSaveFolders(list) {
  try {
    localFolderList = await invoke("local_set_folders", { folders: list });
    paintLocal();
    await localLoad();
    localRescan();
  } catch (e) {
    $("localNote").textContent = String(e);
  }
}

async function localAddFolder() {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (dlg && dlg.open) {
    try {
      const picked = await dlg.open({ directory: true, multiple: false, title: "Choose a music folder" });
      if (picked) localSaveFolders(localFolderList.concat([String(picked)]));
      return;
    } catch (e) {
      // fall back to typing the path
    }
  }
  $("localAddForm").hidden = false;
  $("localAddPath").focus();
}

function v29Init() {
  // local music settings
  $("localOn").checked = prefs.localOn !== false;
  $("localOn").addEventListener("change", async () => {
    prefs.localOn = $("localOn").checked;
    savePrefs();
    await localLoad();
    if (prefs.localOn) localRescan();
  });
  $("localAdd").addEventListener("click", localAddFolder);
  $("localRescan").addEventListener("click", localRescan);
  $("localAddForm").addEventListener("submit", (e) => {
    e.preventDefault();
    const p = $("localAddPath").value.trim();
    if (!p) return;
    $("localAddForm").hidden = true;
    $("localAddPath").value = "";
    localSaveFolders(localFolderList.concat([p]));
  });
  $("localFolders").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-i]");
    if (!b) return;
    localSaveFolders(localFolderList.filter((_, i) => i !== Number(b.dataset.i)));
  });
  localLoad().then(() => localRescan());
}

// ---------- everything you've liked (for Discover and Rediscover) ----------
let likedTracks = [];
let likedIds = new Set();
let likesLoadedAt = 0;
let likesLoading = null;

function ensureLikes(force) {
  if (prefs.scOn === false) return Promise.resolve();
  if (likesLoading) return likesLoading;
  if (!force && likesLoadedAt && Date.now() - likesLoadedAt < 10 * 60000) return Promise.resolve();
  likesLoading = (async () => {
    try {
      const all = await collectPages("/me/likes/tracks?access=playable&limit=200&linked_partitioning=true", 25);
      likedTracks = all;
      likedIds = new Set(all.map((t) => t.id));
      all.forEach((t) => likeState.set(t.id, true));
      likesLoadedAt = Date.now();
    } catch (e) {
      // offline or signed out: Discover still works, it just can't skip your likes
    } finally {
      likesLoading = null;
    }
  })();
  return likesLoading;
}

function isLikedTrack(t) {
  const s = likeState.get(t.id);
  return s === true || (s !== false && likedIds.has(t.id));
}

// Rediscover: your own likes you haven't played in the last month
async function openRediscover(fresh) {
  currentPlaylist = null;
  currentArtist = null;
  showView("rediscover");
  if (!fresh && useCachedPage()) return;
  $("listView").scrollTop = 0;
  const tok = ++loadToken;
  say("Looking through your likes...");
  try {
    const plat = curPlat("rediscover");
    if (plat === "none") { viewTracks = []; nextHref = ""; render(); say("Nothing to show yet: switch on SoundCloud, Spotify or Local music in Settings > Integrations."); return; }
    let source;
    if (plat === "spotify") source = await spotifyLikedTracks(500);
    else if (plat === "local") source = localTracks;
    else { await ensureLikes(true); source = likedTracks.filter(isLikedTrack); }
    if (tok !== loadToken) return;
    const lastPlayed = new Map();
    loadStats().forEach((x) => lastPlayed.set(x.id, Math.max(lastPlayed.get(x.id) || 0, x.ts)));
    const monthAgo = Date.now() - 30 * 86400000;
    let pool = source.filter((t) =>
      !isMix(t) && !hiddenRecs.has(String(t.id)) && (lastPlayed.get(t.id) || 0) < monthAgo);
    shuffleInPlace(pool);
    pool.sort((a, b) => (lastPlayed.get(a.id) || 0) - (lastPlayed.get(b.id) || 0));   // never played first
    setLocalPool(pool);
    render();
    say(viewTracks.length ? "" : source.length ? "You've played all your likes recently." : "No likes found.");
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    render();
    say(String(e));
  }
}

// ---------- "Remove recommendation" ----------
let hiddenRecs = new Set();

function hideRecommendation(t) {
  hiddenRecs.add(String(t.id));
  prefs.hiddenRecs = [...hiddenRecs].slice(-3000);
  savePrefs();
  viewTracks = viewTracks.filter((x) => x.id !== t.id);
  render();
  say("Removed. It won't be recommended again.");
}

// ---------- right-click menu on tracks ----------
let ctxTrack = null;
let ctxAnchor = null;

function openTrackMenu(t, x, y) {
  ctxTrack = t;
  ctxAnchor = { getBoundingClientRect: () => ({ left: x, right: x, top: y, bottom: y }) };
  const m = $("plMenu");
  m.replaceChildren();
  const add = (label, action) => {
    const b = el("button", "menuItem", label);
    b.dataset.action = action;
    m.append(b);
  };
  const sep = () => m.append(el("div", "menuSep"));
  const sc = srcOf(t) === "soundcloud";
  add("Play", "play");
  add("Play next", "next");
  add("Add to queue", "queue");
  add("Add to playlist...", "playlist");
  sep();
  if (sc) add(isLiked(t) ? "Remove from Likes" : "Add to Likes", "like");
  if (sc && t.user && t.user.id) add("Go to artist", "artist");
  if (view === "playlist" && isOwnPlaylist(currentPlaylist)) add("Remove from this playlist", "unlist");
  if (view === "discover" || view === "rediscover") add("Remove recommendation", "hide");
  if (t.permalink_url) {
    sep();
    add(srcOf(t) === "spotify" ? "Open in Spotify" : "Open on SoundCloud", "open");
    add("Copy link", "copy");
  }
  if (sc || srcOf(t) === "local") {
    sep();
    add("Edit title and artist...", "fix");
  }
  m.hidden = false;
  const w = m.offsetWidth;
  const h = m.offsetHeight;
  m.style.left = Math.max(8, Math.min(window.innerWidth - w - 8, x)) + "px";
  m.style.top = Math.max(8, y + h + 8 > window.innerHeight ? y - h : y) + "px";
}

function runMenuAction(a) {
  const t = ctxTrack;
  closeMenu();
  if (!t) return;
  const i = viewTracks.indexOf(t);
  switch (a) {
    case "play": if (i >= 0) startList(viewTracks, i, false); else startList([t], 0, false); break;
    case "next": playNext(t); break;
    case "queue": addToQueue(t); break;
    case "playlist": openPlaylistMenu(t, ctxAnchor); break;
    case "like": toggleLike(t); break;
    case "artist": openArtist(t.user); break;
    case "unlist": removeFromPlaylist(t); break;
    case "hide": hideRecommendation(t); break;
    case "fix": openFixDialog(t); break;
    case "open": openOnSoundCloud(t); break;
    case "copy":
      navigator.clipboard.writeText(t.permalink_url)
        .then(() => say("Link copied."))
        .catch(() => say("Could not copy the link."));
      break;
  }
}

function v28Init() {
  $("fixForm").addEventListener("submit", saveFix);
  $("fixCancel").addEventListener("click", () => $("fixDialog").close());
  $("fixReset").addEventListener("click", resetFix);

  // Integrations can be switched off without signing out
  $("lfmOn").checked = prefs.lfmOn !== false;
  $("lfmOn").addEventListener("change", () => {
    prefs.lfmOn = $("lfmOn").checked;
    savePrefs();
    lfmRefresh();
  });
  $("spOn").checked = prefs.spOn !== false;
  $("spOn").addEventListener("change", () => {
    prefs.spOn = $("spOn").checked;
    savePrefs();
    if (typeof spotifyCheck === "function") spotifyCheck();
    renderPlatChips();
  });
}

function v26Init() {
  // second accent
  buildSwatches2();
  $("swatches2").addEventListener("click", (e) => {
    const b = e.target.closest(".swatch");
    if (b && !b.disabled) setAccent2(b.dataset.c);
  });
  $("swatches2").addEventListener("input", (e) => {
    if (e.target.id === "accent2Custom") setAccent2(e.target.value);
  });
  syncAccent2();

  // back, rediscover
  $("backBtn").addEventListener("click", goBack);
  $("navRediscover").addEventListener("click", () => openRediscover());
  $("refreshBtn").addEventListener("click", () => {
    if (view === "discover") openDiscover(discoverSrc, true);
    else if (view === "rediscover") openRediscover(true);
  });

  // removed recommendations
  hiddenRecs = new Set((prefs.hiddenRecs || []).map(String));

  // right-click: our menu on tracks, and no browser menu anywhere except text boxes
  $("list").addEventListener("contextmenu", (e) => {
    const li = e.target.closest("li");
    if (!li) return;
    e.preventDefault();
    const t = viewTracks[Number(li.dataset.i)];
    if (t) openTrackMenu(t, e.clientX, e.clientY);
  });
  document.addEventListener("contextmenu", (e) => {
    if (!e.target.closest("input, textarea")) e.preventDefault();
  });

  // fetch your likes quietly in the background
  setTimeout(() => ensureLikes(), 1500);
}


// ---------- chat song requests ----------
function reqKeyNew() {
  const a = new Uint8Array(12);
  crypto.getRandomValues(a);
  return Array.from(a, (b) => b.toString(16).padStart(2, "0")).join("");
}

const reqPendingList = () => queue.slice(qi + 1).filter((t) => t.requestedBy);

function reqPaint() {
  const rp = curPlat("search");
  $("reqPlat").textContent = "Requests are going to: " + (rp === "none" ? "nowhere (no platform is switched on)" : PLATFORMS[rp].name) +
    ". Change it with the platform icons in the title bar.";
  $("reqMaxVal").textContent = prefs.reqMax || 10;
  $("reqUserVal").textContent = prefs.reqPerUser || 2;
  const p = reqPendingList();
  $("reqPending").textContent = p.length
    ? p.map((t) => '"' + t.title + '" for ' + t.requestedBy).join(", ")
    : "None.";
}

async function reqApply() {
  const on = !!prefs.req;
  $("reqOn").checked = on;
  $("requestSection").hidden = !on;
  if (!on) {
    invoke("requests_stop").catch(() => {});
    return;
  }
  if (!prefs.reqKey) { prefs.reqKey = reqKeyNew(); savePrefs(); }
  const port = Number(prefs.reqPort) || 7879;
  $("reqPort").value = port;
  try {
    await invoke("requests_start", { port, key: prefs.reqKey });
    const base = "http://127.0.0.1:" + port;
    $("reqUrl").value = base + "/request?key=" + prefs.reqKey + "&user=USER&q=SONG";
    $("reqNowUrl").value = base + "/now";
    $("reqNote").textContent = "";
    reqRenderBot();
    widgetPush();
  } catch (e) {
    $("reqNote").textContent = String(e);
  }
  reqPaint();
}

// Called from the Rust side for each request; it answers with request_result
window.__songRequest = async (id, q, user) => {
  let result;
  try {
    result = await handleSongRequest(String(q || "").trim(), String(user || "").trim());
  } catch (e) {
    result = { ok: false, message: "Couldn't add that one: " + e };
  }
  invoke("request_result", { id, result }).catch(() => {});
};

async function findRequested(q) {
  if (/^https?:\/\/(www\.|m\.)?soundcloud\.com\//i.test(q)) {
    try {
      const found = asList([await api("/resolve?url=" + encodeURIComponent(q.split("?")[0]))]);
      if (found.length) return found;
    } catch (e) {
      // fall back to searching for the words in the link
    }
    q = q.split("?")[0].replace(/^https?:\/\/[^/]+\//, "").replace(/[\/\-_]+/g, " ");
  }
  return asList(await api("/tracks?q=" + encodeURIComponent(q) + "&access=playable&limit=10&linked_partitioning=true"));
}

// Spotify: a track link, or a search
async function findSpotifyRequested(q) {
  const m = q.match(/open\.spotify\.com\/(?:intl-[a-z-]+\/)?track\/([A-Za-z0-9]+)/i);
  if (m) {
    try {
      const x = await spApi("GET", "/tracks/" + m[1]);
      if (x && x.id) return [spTrack(x)];
    } catch (e) {
      // fall through to a normal search
    }
  }
  return spotifySearchTracks(q);
}

const REQ_MSG_DEFAULTS = {
  added: 'Added "{title}" by {artist} (#{position} in the queue).',
  playing: 'Added "{title}" by {artist}, playing now.',
  notFound: 'Couldn\'t find "{query}" on {platform}.',
  full: "The request queue is full. Try again in a bit!",
  perUser: "{user}, you already have {count} requests waiting.",
  duplicate: '"{title}" is already in the queue.',
  closed: "Song requests are closed right now.",
  busy: "Requests are set to {platform}, but something else is playing right now. Try again soon!",
  empty: "Tell me what to play, for example: artist - song",
};
const REQ_MSG_LABELS = {
  added: "Added", playing: "Added, playing now", notFound: "Not found", full: "Queue full",
  perUser: "Viewer limit", duplicate: "Already queued", closed: "Requests off", busy: "Other platform playing",
  empty: "Nothing asked for",
};

function reqMsg(key, vars) {
  const tpl = ((prefs.reqMsgs || {})[key] || "").trim() || REQ_MSG_DEFAULTS[key];
  return tpl.replace(/\{(\w+)\}/g, (m, k) => (vars && vars[k] !== undefined ? String(vars[k]) : m));
}

async function handleSongRequest(q, user) {
  if (!prefs.req) return { ok: false, message: reqMsg("closed", {}) };
  q = q.slice(0, 200);
  const who = user || "chat";
  const plat = curPlat("search");
  if (plat === "none") return { ok: false, message: reqMsg("closed", { user: who }) };
  const platName = PLATFORMS[plat].name;
  const base = { user: who, query: q, platform: platName };
  if (!q) return { ok: false, message: reqMsg("empty", base) };
  const pending = reqPendingList();
  if (pending.length >= (prefs.reqMax || 10)) return { ok: false, message: reqMsg("full", base) };
  const mine = pending.filter((t) => t.requestedBy.toLowerCase() === who.toLowerCase()).length;
  if (user && mine >= (prefs.reqPerUser || 2)) return { ok: false, message: reqMsg("perUser", Object.assign({ count: mine }, base)) };
  const cur = curTrack();
  if (cur && srcOf(cur) !== plat && (srcOf(cur) === "spotify" || plat === "spotify")) {
    return { ok: false, message: reqMsg("busy", base) };
  }
  let found;
  if (plat === "spotify") found = await findSpotifyRequested(q);
  else if (plat === "local") found = localSearch(q);
  else found = await findRequested(q);
  found = found.filter((t) => !isMix(t));
  if (!found.length) return { ok: false, message: reqMsg("notFound", base) };
  const t = found[0];
  const vars = Object.assign({ title: t.title, artist: artistName(t) }, base);
  if (queue.slice(Math.max(qi, 0)).some((x) => x.id === t.id)) return { ok: false, message: reqMsg("duplicate", vars) };
  t.requestedBy = who;
  addToQueue(t);
  reqPaint();
  reqQueueBtn();
  const pos = queue.length - 1 - qi;
  vars.position = pos;
  return {
    ok: true, title: t.title, artist: artistName(t), url: t.permalink_url || "", position: pos,
    message: reqMsg(pos > 0 ? "added" : "playing", vars),
  };
}

function v30Init() {
  // "Manage subscription" links
  document.addEventListener("click", (e) => {
    const b = e.target.closest(".linkBtn[data-url]");
    if (b) invoke("open_url", { url: b.dataset.url }).catch((err) => say(String(err)));
  });

  // song requests
  $("reqOn").addEventListener("change", () => {
    prefs.req = $("reqOn").checked;
    savePrefs();
    reqApply();
    reqQueueBtn();
    spModePaint();
  });
  $("reqPort").addEventListener("change", () => {
    const p = Math.round(Number($("reqPort").value));
    if (p >= 1024 && p <= 65535) { prefs.reqPort = p; savePrefs(); reqApply(); }
    else $("reqPort").value = prefs.reqPort || 7879;
  });
  $("reqNewKey").addEventListener("click", () => {
    prefs.reqKey = reqKeyNew();
    savePrefs();
    reqApply();
    say("New request key made. Update the address in your chat bot.");
  });
  const copy = (inputId, btnId) => $(btnId).addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($(inputId).value);
      $(btnId).textContent = "Copied";
      setTimeout(() => { $(btnId).textContent = "Copy"; }, 1500);
    } catch (e) {
      $(inputId).select();
    }
  });
  copy("reqUrl", "reqCopy");
  copy("reqNowUrl", "reqNowCopy");
  const stepper = (key, def, min, max, down, up) => {
    const set = (d) => { prefs[key] = Math.min(max, Math.max(min, (prefs[key] || def) + d)); savePrefs(); reqPaint(); };
    $(down).addEventListener("click", () => set(-1));
    $(up).addEventListener("click", () => set(1));
  };
  stepper("reqMax", 10, 1, 50, "reqMaxDown", "reqMaxUp");
  stepper("reqPerUser", 2, 1, 10, "reqUserDown", "reqUserUp");
  $("reqClear").addEventListener("click", clearRequests);
  reqApply();
}

// ---------- stream-safe output ----------
const isDefaultSink = () => !prefs.sink || prefs.sink === "default" || prefs.sink === "communications";

// Returns a reason not to play, or "" when the audio is going somewhere safe
function routeBlock(t) {
  if (prefs.safeOutput === false || !prefs.req) return "";
  if (srcOf(t) === "spotify") {
    return prefs.spMode === "desktop" || prefs.spAppRouted ? "" :
      "Stream-safe output: Spotify playing inside this app always uses your default device. " +
      "In Settings > Output Preferences, set \"Spotify plays through\" to the Spotify desktop app.";
  }
  if (isDefaultSink()) {
    return "Stream-safe output: choose an output device in Settings > Output Preferences first, " +
      "so nothing plays on your default device.";
  }
  return "";
}

function spModePaint() {
  const linked = FEATURES.spotify && spotifyPremium && prefs.spOn !== false;
  $("spModeRow").hidden = !linked;
  $("safeRow").hidden = !prefs.req;
  $("spRoutedRow").hidden = !prefs.req || !linked || prefs.spMode === "desktop";
  $("spMode").value = prefs.spMode === "desktop" ? "desktop" : "app";
  $("spModeNote").textContent = prefs.spMode === "desktop"
    ? "The Spotify app on this PC does the playing; keep it open. Choose its output in Windows: " +
      "Settings > System > Sound > Volume mixer > Spotify."
    : "Plays inside this app, using whatever output Windows gives this app. To keep it off your game audio, " +
      "route this app in Windows' Volume mixer (below), or use the desktop app.";
}

function v32Init() {
  $("safeOut").checked = prefs.safeOutput !== false;
  $("safeOut").addEventListener("change", () => {
    prefs.safeOutput = $("safeOut").checked;
    savePrefs();
    const t = curTrack();
    if (t && !audio.paused && routeBlock(t)) { audio.pause(); say(routeBlock(t)); }
  });
  $("spMode").addEventListener("change", () => {
    const t = curTrack();
    if (t && srcOf(t) === "spotify") spotifyPause();   // stop it where it was playing
    prefs.spMode = $("spMode").value;
    savePrefs();
    spModePaint();
  });
  spModePaint();

  // If the chosen device is unplugged, Windows would move the sound to the default device: pause instead
  if (navigator.mediaDevices && navigator.mediaDevices.addEventListener) {
    navigator.mediaDevices.addEventListener("devicechange", async () => {
      if (prefs.safeOutput === false || !prefs.req || isDefaultSink() || audio.paused) return;
      try {
        const outs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audiooutput");
        if (!outs.some((d) => d.deviceId === prefs.sink)) {
          audio.pause();
          say("Your output device disconnected, so playback paused to keep it off your default device.");
        }
      } catch (e) {}
    });
  }
}


// ---------- v33: SoundCloud on/off, chat bots, equalizer, animations ----------
const scOn = () => prefs.scOn !== false;
const cssEnabled = () => (prefs.cssOn === undefined ? !!(prefs.css || "").trim() : !!prefs.cssOn);

function scApply() {
  $("navDiscover").hidden = !platformsFor("discover").length;
  renderPlatChips();
}

// Chat bot instructions: only the one you use
function reqRenderBot() {
  const bot = prefs.reqBot || "streamerbot";
  $("botSel").value = bot;
  const port = Number(prefs.reqPort) || 7879;
  const base = "http://127.0.0.1:" + port;
  const key = prefs.reqKey || "";
  const vars = {
    streamerbot: { user: "%user%", q: "%rawInputUrlEncoded%" },
    mixitup: { user: "$username", q: "$allargs" },
    other: { user: "USERNAME", q: "SONG" },
  }[bot];
  const reqAddr = base + "/request?key=" + key + "&user=" + vars.user + "&q=" + vars.q;
  $("reqUrl").value = reqAddr;
  $("reqNowUrl").value = base + "/now";
  const help = $("botHelp");
  help.replaceChildren();
  const code = (s) => { const c = el("code", "", s); return c; };
  const li = (...parts) => { const x = el("li"); parts.forEach((p) => x.append(typeof p === "string" ? document.createTextNode(p) : p)); return x; };
  const ol = el("ol");
  if (bot === "streamerbot") {
    $("reqUrlLbl").textContent = "Fetch URL address (variables already filled in)";
    ol.append(
      li("Create an Action called Song request, with a Command trigger such as ", code("!sr"), "."),
      li("Add the sub-action Core > Network > Fetch URL, paste the address above, and set its variable name to ", code("reply"), "."),
      li("Add the sub-action Twitch > Chat > Send Message to Channel with the message ", code("%reply%"), "."),
      li("For ", code("!song"), ": the same again with the now playing address."),
    );
    help.append(ol, el("p", "note", "If your version names things slightly differently, look for the sub-action that fetches a URL and saves the reply in a variable."));
  } else if (bot === "mixitup") {
    $("reqUrlLbl").textContent = "Web Request address (special identifiers already filled in)";
    ol.append(
      li("Create a Chat Command such as ", code("!sr"), "."),
      li("Add a Web Request action, paste the address above, and set the response to plain text."),
      li("Add a Chat action that sends ", code("$webrequestresult"), "."),
      li("For ", code("!song"), ": the same again with the now playing address."),
    );
    help.append(ol, el("p", "note", "If $webrequestresult doesn't post anything in your version, check Mix It Up's list of special identifiers for the web request result."));
  } else {
    $("reqUrlLbl").textContent = "Request address (replace USERNAME and SONG with your bot's variables)";
    help.append(
      el("p", "", "Any bot that can call a web address and post the reply in chat will work:"),
      (() => { const l = el("ol"); l.append(
        li("Call ", code(base + "/request"), " with ", code("key"), " (required, from this page), ", code("q"),
           " (what was asked for: search words or a SoundCloud or Spotify link) and ", code("user"),
           " (who asked; used for the per-viewer limit and \"requested by\")."),
        li("Text in ", code("q"), " and ", code("user"), " must be URL-encoded (spaces as %20). Most bots have an encoded version of the message variable."),
        li("The reply is one line of plain text, ready to post in chat. Add ", code("&format=json"),
           " to get JSON instead: ok, message, title, artist, url and position."),
        li("Call ", code(base + "/now"), " for the current song (no key needed). It also supports ", code("&format=json"), "."),
        li("Only programs on this PC can reach these addresses."),
      ); return l; })(),
    );
  }
}

function reqQueueBtn() {
  const b = $("qClearReq");
  if (b) b.hidden = !(prefs.req && reqPendingList().length);
}

function clearRequests() {
  queue = queue.filter((t, i) => i <= qi || !t.requestedBy);
  renderQueue();
  reqPaint();
  say("Waiting requests removed.");
}

function renderReqMsgs() {
  const box = $("reqMsgs");
  box.replaceChildren();
  Object.keys(REQ_MSG_DEFAULTS).forEach((k) => {
    const row = el("div", "msgRow");
    const lab = el("label", "", REQ_MSG_LABELS[k]);
    lab.htmlFor = "msg_" + k;
    const inp = el("input");
    inp.id = "msg_" + k;
    inp.type = "text";
    inp.maxLength = 300;
    inp.placeholder = REQ_MSG_DEFAULTS[k];
    inp.value = (prefs.reqMsgs || {})[k] || "";
    inp.dataset.k = k;
    row.append(lab, inp);
    box.append(row);
  });
}

// ---------- equalizer ----------
const EQ_FREQS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const EQ_PRESETS = {
  flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  bass: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0],
  vocal: [-2, -2, -1, 1, 3, 3, 2, 1, 0, -1],
  treble: [0, 0, 0, 0, 0, 1, 2, 4, 5, 6],
  smile: [5, 4, 2, 0, -1, -1, 0, 2, 4, 5],
};
const eqGains = () => (Array.isArray(prefs.eq) && prefs.eq.length === 10 ? prefs.eq : EQ_PRESETS.flat.slice());

function eqApply() {
  if (!norm.ctx || !norm.eq) return;
  const g = eqGains();
  norm.eq.forEach((b, i) => b.gain.setTargetAtTime(prefs.eqOn ? g[i] : 0, norm.ctx.currentTime, 0.05));
}

function renderEq() {
  const box = $("eqBands");
  box.replaceChildren();
  const g = eqGains();
  EQ_FREQS.forEach((f, i) => {
    const band = el("div", "eqBand");
    const val = el("span", "gain", (g[i] > 0 ? "+" : "") + g[i]);
    const r = el("input");
    r.type = "range";
    r.min = -12;
    r.max = 12;
    r.step = 1;
    r.value = g[i];
    r.dataset.i = i;
    r.setAttribute("aria-label", (f >= 1000 ? f / 1000 + " kHz" : f + " Hz") + " gain");
    band.append(val, r, el("span", "", f >= 1000 ? f / 1000 + "k" : String(f)));
    box.append(band);
  });
}

function setEq(gains) {
  prefs.eq = gains.slice();
  savePrefs();
  renderEq();
  eqApply();
}

// ---------- animations (app only; the widget has its own) ----------
function applyAnim() {
  document.documentElement.classList.toggle("noAnim", prefs.animOn === false);
}

function v33Init() {
  // SoundCloud on/off without signing out
  $("scOn").checked = scOn();
  $("scOn").addEventListener("change", async () => {
    prefs.scOn = $("scOn").checked;
    savePrefs();
    scApply();
    refreshSidebar();
    if (curPage && curPage.view !== "now") { restoring = true; try { restorePage(curPage); } finally { restoring = false; } }
    else openLikes();
  });

  // Custom CSS on/off
  $("cssOn").checked = cssEnabled();
  $("cssSection").hidden = !cssEnabled();
  $("cssOn").addEventListener("change", () => {
    prefs.cssOn = $("cssOn").checked;
    savePrefs();
    $("cssSection").hidden = !prefs.cssOn;
    if (prefs.cssOn) $("cssSection").open = true;
    applyUserCss();
  });

  // Song requests: bot picker, messages, queue button
  $("botSel").addEventListener("change", () => { prefs.reqBot = $("botSel").value; savePrefs(); reqRenderBot(); });
  renderReqMsgs();
  $("reqMsgs").addEventListener("input", (e) => {
    const k = e.target.dataset.k;
    if (!k) return;
    prefs.reqMsgs = prefs.reqMsgs || {};
    prefs.reqMsgs[k] = e.target.value;
    savePrefs();
  });
  $("msgReset").addEventListener("click", () => { prefs.reqMsgs = {}; savePrefs(); renderReqMsgs(); });
  $("qClearReq").addEventListener("click", clearRequests);
  reqRenderBot();
  reqQueueBtn();

  // Widget animation options
  $("wFade").checked = prefs.wFade !== false;
  $("wScroll").checked = prefs.wScroll !== false;
  $("wSpin").checked = prefs.wSpin !== false;
  $("wColor").value = String(prefs.wColor === undefined ? 1.2 : prefs.wColor);
  [["wFade", "wFade"], ["wScroll", "wScroll"], ["wSpin", "wSpin"]].forEach(([id, key]) =>
    $(id).addEventListener("change", () => { prefs[key] = $(id).checked; savePrefs(); widgetPush(); }));
  $("wColor").addEventListener("change", () => { prefs.wColor = Number($("wColor").value); savePrefs(); widgetPush(); });

  // Equalizer
  $("eqOn").checked = !!prefs.eqOn;
  $("eqSection").hidden = !prefs.eqOn;
  renderEq();
  $("eqOn").addEventListener("change", () => {
    prefs.eqOn = $("eqOn").checked;
    savePrefs();
    $("eqSection").hidden = !prefs.eqOn;
    if (prefs.eqOn) {
      $("eqSection").open = true;
      audio.crossOrigin = "anonymous";
      $("eqNote").textContent = "Ten bands, for SoundCloud and local files. Changes are heard straight away.";
    }
    eqApply();
  });
  $("eqBands").addEventListener("input", (e) => {
    const i = Number(e.target.dataset.i);
    if (Number.isNaN(i)) return;
    const g = eqGains();
    g[i] = Number(e.target.value);
    prefs.eq = g;
    savePrefs();
    e.target.parentElement.querySelector(".gain").textContent = (g[i] > 0 ? "+" : "") + g[i];
    eqApply();
  });
  $("eqPreset").addEventListener("change", () => {
    const p = EQ_PRESETS[$("eqPreset").value];
    if (p) setEq(p);
    $("eqPreset").value = "";
  });
  $("eqReset").addEventListener("click", () => setEq(EQ_PRESETS.flat));

  // Spotify inside the app, once the app is routed in Windows
  $("spRouted").checked = !!prefs.spAppRouted;
  $("spRouted").addEventListener("change", () => { prefs.spAppRouted = $("spRouted").checked; savePrefs(); });

  // Animations
  $("animOn").checked = prefs.animOn !== false;
  $("animOn").addEventListener("change", () => { prefs.animOn = $("animOn").checked; savePrefs(); applyAnim(); });
  applyAnim();
}

// ---------- v35: platform icons and app name in the title bar ----------
function refreshTitle() {
  updateTitle(lastSignedIn);
}

function editAppName() {
  const span = $("titleText");
  if (span.hidden) return;
  const inp = el("input", "titleEdit");
  inp.value = (prefs.appName || "").trim() || span.textContent;
  inp.maxLength = 60;
  inp.setAttribute("aria-label", "App name");
  span.hidden = true;
  span.after(inp);
  inp.focus();
  inp.select();
  let finished = false;
  const done = (save) => {
    if (finished) return;
    finished = true;
    if (save) {
      prefs.appName = inp.value.trim();
      savePrefs();
      $("appNameInput").value = prefs.appName;
    }
    inp.remove();
    span.hidden = false;
    refreshTitle();
  };
  inp.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") done(true);
    else if (ev.key === "Escape") done(false);
  });
  inp.addEventListener("blur", () => done(true));
}

function v35Init() {
  $("titlePlat").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-p]");
    if (!b || b.classList.contains("on")) return;
    prefs.globalPlat = b.dataset.p;
    savePrefs();
    renderPlatChips();
    reqPaint();
    if (curPage && curPage.view !== "now") { restoring = true; try { restorePage(curPage); } finally { restoring = false; } }
  });

  $("titleText").addEventListener("contextmenu", (e) => { e.preventDefault(); editAppName(); });
  $("appNameInput").value = prefs.appName || "";
  $("appNameInput").placeholder = userName ? userName + "'s Music" : "Your Music";
  $("appNameInput").addEventListener("input", () => {
    prefs.appName = $("appNameInput").value;
    savePrefs();
    refreshTitle();
  });
  renderPlatChips();
}

// ---------- Discover on Spotify ----------
// Spotify switched off recommendations and related artists for new apps in November 2024, and
// caps searches at 10 results. So Discover on Spotify is built from what's left: Last.fm finds
// artists similar to the ones you like, and Spotify search supplies their tracks.
let spLikedCache = null;
let spLikedAt = 0;

async function spLiked() {
  if (!spLikedCache || Date.now() - spLikedAt > 10 * 60000) {
    spLikedCache = await spotifyLikedTracks(500);
    spLikedAt = Date.now();
  }
  return spLikedCache;
}

const firstArtist = (t) => artistName(t).split(",")[0].trim();

async function spArtistTracks(name) {
  try { return await spotifySearchTracks('artist:"' + name.replace(/"/g, "") + '"'); } catch (e) { return []; }
}

async function lfmSimilarNames(artist, n) {
  if (prefs.lfmOn === false) return [];
  try {
    const res = await invoke("lastfm_call", { method: "artist.getSimilar", params: [["artist", artist], ["limit", String(n)]] });
    return ((res.similarartists && res.similarartists.artist) || []).map((a) => a.name);
  } catch (e) {
    return [];
  }
}

async function loadSpotifyDiscover(src) {
  const tok = ++loadToken;
  discoverNote = "";
  say("Finding music on Spotify...");
  try {
    const liked = await spLiked();
    if (tok !== loadToken) return;
    const likedIds = new Set(liked.map((t) => t.id));
    let list = [];
    if (src.startsWith("genre:")) {
      // a few pages of a genre search, from different starting points so it changes each time
      const g = src.slice(6).toLowerCase();
      const offsets = shuffleInPlace([0, 10, 20, 30, 40, 50]).slice(0, 3);
      const pages = await Promise.all(offsets.map((o) =>
        spApi("GET", "/search?type=track&limit=10&offset=" + o + "&q=" + encodeURIComponent('genre:"' + g + '"'))
          .then((d) => ((d.tracks && d.tracks.items) || []).filter((x) => x && x.id).map(spTrack))
          .catch(() => [])));
      list = pages.flat();
    } else {
      // seeds: the current Spotify artist (Similar artists), or a few artists from your liked songs (For you)
      let seeds = [];
      const cur = curTrack();
      if (src === "similar" && cur && srcOf(cur) === "spotify") seeds = [firstArtist(cur)];
      if (!seeds.length) {
        const artists = [...new Set(liked.map(firstArtist).filter(Boolean))];
        seeds = shuffleInPlace(artists).slice(0, src === "similar" ? 1 : 3);
      }
      if (!seeds.length) throw new Error("Like a few songs on Spotify first, and Discover will suggest more.");
      let names = [];
      for (const s of seeds) names = names.concat(await lfmSimilarNames(s, 8));
      names = shuffleInPlace([...new Set(names)]).slice(0, 5);
      if (names.length) {
        discoverNote = "Similar to " + seeds.join(", ");
      } else {
        // no Last.fm: other songs by artists you like
        names = seeds;
        say("Showing more from artists you like. Turn on Last.fm in Settings > Integrations for new artists.");
      }
      const lists = await Promise.all(names.map(spArtistTracks));
      list = lists.flat();
    }
    if (tok !== loadToken) return;
    const seen = new Set();
    list = list.filter((t) => !likedIds.has(t.id) && !isMix(t) && !seen.has(t.id) && seen.add(t.id));
    list = shuffleInPlace(recFilter(list));
    setLocalPool(list);
    render();
    if (!viewTracks.length) say("No suggestions came back. Try another chip or a genre.");
    else if (!/Last\.fm/.test($("status").textContent)) say("");
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    nextHref = "";
    render();
    say(e && e.message ? e.message : String(e));
  }
}

// ---------- sidebar playlists for the chosen platform ----------
// SoundCloud: your playlists (and ones you liked). Spotify: your playlists. Local files: your albums.
let sidebarPlat = null;

async function refreshSidebar() {
  const plat = sidebarPlat || "none";
  document.querySelector(".plScroll").hidden = plat === "none";
  $("plHeadText").textContent = plat === "local" ? "Albums" : "Playlists";
  $("plNew").hidden = plat !== "soundcloud";
  $("plNewForm").hidden = true;
  if (plat === "soundcloud") {
    if (await invoke("signed_in")) loadPlaylists();
    else { playlists = []; renderPlaylists("Sign in to SoundCloud to see your playlists."); }
    return;
  }
  if (plat === "spotify") {
    playlists = [];
    renderPlaylists("Loading...");
    try {
      const list = await spotifyPlaylists();
      if (sidebarPlat !== "spotify") return;
      playlists = list;
      renderPlaylists();
    } catch (e) {
      if (sidebarPlat !== "spotify") return;
      playlists = [];
      renderPlaylists(/40[13]|scope/i.test(String(e))
        ? "Sign out of Spotify and back in (Settings > Integrations) to let the app read your playlists."
        : "Could not load your Spotify playlists.");
    }
    return;
  }
  if (plat === "local") {
    const albums = new Map();
    localTracks.forEach((t) => {
      const a = (t.album || "").trim();
      if (a && !albums.has(a)) albums.set(a, { source: "local", id: "album:" + a, album: a, title: a, user: { username: artistName(t) } });
    });
    playlists = [...albums.values()].sort((x, y) => x.title.localeCompare(y.title));
    renderPlaylists(playlists.length ? "" : "No album names found in your files' tags.");
    return;
  }
  playlists = [];
  renderPlaylists();
}

// Opening a Spotify playlist or a local album
async function loadPlatformPlaylist(p) {
  const tok = ++loadToken;
  $("listView").scrollTop = 0;
  say("Loading...");
  try {
    const list = p.source === "local"
      ? localTracks.filter((t) => (t.album || "").trim() === p.album)
      : p.spAlbumId ? await spotifyAlbumTracks(p.spAlbumId) : await spotifyPlaylistTracks(p.spId);
    if (tok !== loadToken) return;
    setLocalPool(list);
    render();
    say(viewTracks.length ? "" : "Nothing in here yet.");
    setTimeout(nearBottomCheck, 50);
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    nextHref = "";
    render();
    say(/40[13]/.test(String(e))
      ? "Spotify wouldn't share this playlist with the app. Spotify limits new apps to playlists you own or follow, and some playlists are blocked."
      : String(e));
  }
}

// ---------- albums mixed into lists ----------
// Album tiles sit among the tracks (one every 6 rows) on Likes, Search, Discover, Rediscover and artist pages.
// The Albums button beside the title hides them; tracks from albums always stay in the list.
const ALBUM_PAGES = ["likes", "search", "discover", "rediscover", "artist"];
const PLAY_ICON = "M8 5v14l11-7z";
let viewAlbums = [];
let albumsKey = "";

const albumsShown = () => prefs.showAlbums !== false;
const asItems = (d) => (Array.isArray(d) ? d : (d && d.collection) || []);
const isScAlbum = (p) => !!p && (/album|ep|compilation/i.test(p.playlist_type || p.set_type || "") || p.is_album === true);

function scAlbumTile(p) {
  return {
    source: "soundcloud", id: "sca:" + p.id, raw: p, title: p.title || "Untitled album",
    artist: (p.user && p.user.username) || "", count: p.track_count || 0, art: artUrl(p, "t200x200"),
  };
}

function spAlbumTile(a) {
  const imgs = a.images || [];
  return {
    source: "spotify", id: "spa:" + a.id, spAlbumId: a.id, title: a.name || "Untitled album",
    artist: (a.artists || []).map((x) => x.name).join(", "), count: a.total_tracks || 0,
    art: imgs.length ? (imgs[1] || imgs[0]).url : "",
  };
}

function localAlbumTiles(page, q) {
  const map = new Map();
  localTracks.forEach((t) => {
    const a = (t.album || "").trim();
    if (!a) return;
    if (!map.has(a)) map.set(a, { source: "local", id: "loca:" + a, title: a, artist: artistName(t), count: 0, art: t.artwork_url });
    map.get(a).count++;
  });
  let list = [...map.values()].filter((a) => a.count > 1);
  if (page === "search" && q) {
    const n = q.toLowerCase();
    list = list.filter((a) => (a.title + " " + a.artist).toLowerCase().includes(n));
  }
  if (page === "rediscover" || page === "discover") list = shuffleInPlace(list).slice(0, 6);
  return list;
}

async function scUserAlbums(uid) {
  if (!uid) return [];
  const d = await api("/users/" + uid + "/playlists?limit=50&linked_partitioning=true&show_tracks=false");
  return asItems(d).filter(isScAlbum).map(scAlbumTile);
}

async function fetchAlbums(p) {
  if (p.view === "artist") return scUserAlbums(p.artist && p.artist.id);
  const plat = curPlat(p.view);
  if (plat === "local") return localAlbumTiles(p.view, p.query);
  if (plat === "spotify") {
    if (p.view === "likes" || p.view === "rediscover") {
      const d = await spApi("GET", "/me/albums?limit=50");
      const list = (d.items || []).map((x) => x.album).filter((a) => a && a.id).map(spAlbumTile);
      return p.view === "rediscover" ? shuffleInPlace(list).slice(0, 6) : list;
    }
    if (p.view === "search" && p.query) {
      const d = await spApi("GET", "/search?type=album&limit=10&q=" + encodeURIComponent(p.query));
      return ((d.albums && d.albums.items) || []).filter((a) => a && a.id).map(spAlbumTile);
    }
    return [];   // Discover: built from the recommended tracks' own albums as the list draws
  }
  if (plat !== "soundcloud") return [];
  if (p.view === "likes" || p.view === "rediscover") {
    const list = asItems(await api("/me/likes/playlists?limit=50")).filter(isScAlbum).map(scAlbumTile);
    return p.view === "rediscover" ? shuffleInPlace(list).slice(0, 6) : list;
  }
  if (p.view === "search" && p.query) {
    const d = await api("/playlists?q=" + encodeURIComponent(p.query) + "&limit=30&linked_partitioning=true&show_tracks=false");
    return asItems(d).filter(isScAlbum).map(scAlbumTile);
  }
  if (p.view === "discover") {
    const src = p.discoverSrc || "";
    if (src.startsWith("genre:")) {
      const d = await api("/playlists?q=" + encodeURIComponent(src.slice(6)) + "&limit=30&linked_partitioning=true&show_tracks=false");
      return shuffleInPlace(asItems(d).filter(isScAlbum)).slice(0, 6).map(scAlbumTile);
    }
    // albums by the artists in the recommendations (wait briefly for the list)
    for (let i = 0; i < 30 && !viewTracks.length; i++) await new Promise((r) => setTimeout(r, 200));
    const ids = [...new Set(viewTracks.map((t) => t.user && t.user.id).filter(Boolean))].slice(0, 4);
    const lists = await Promise.all(ids.map((id) => scUserAlbums(id).catch(() => [])));
    return shuffleInPlace(lists.flat()).slice(0, 6);
  }
  return [];
}

// Called from navTrack whenever the page changes
function albumsOnPage(p) {
  const key = pageKey(p);
  if (key === albumsKey) return;
  albumsKey = key;
  viewAlbums = [];
  if (!albumsShown() || !ALBUM_PAGES.includes(p.view)) return;
  fetchAlbums(p).then((list) => {
    if (albumsKey !== key) return;
    viewAlbums = list || [];
    render();
  }).catch(() => {});
}

function paintAlbumToggle() {
  const b = $("albumToggle");
  b.hidden = !ALBUM_PAGES.includes(view);
  b.classList.toggle("on", albumsShown());
  b.setAttribute("aria-pressed", albumsShown() ? "true" : "false");
  b.title = albumsShown() ? "Hide albums (their tracks stay in the list)" : "Show albums";
}

function albumTileEl(a) {
  const li = el("li", "albumTile");
  li._album = a;
  const img = el("img", "albumArt");
  img.alt = "";
  img.loading = "lazy";
  if (a.art) img.src = a.art;
  const info = el("div", "albumInfo");
  const kind = el("div", "albumKind");
  kind.append(platformBadge({ source: a.source }), document.createTextNode("Album"));
  info.append(
    kind,
    el("div", "albumTitle", a.title),
    el("div", "albumArtist", a.artist || ""),
    el("div", "albumCount", a.count ? a.count + (a.count === 1 ? " track" : " tracks") : ""),
  );
  const play = el("button", "albumPlay");
  play.title = "Play album";
  play.setAttribute("aria-label", "Play " + a.title);
  play.append(icon(PLAY_ICON));
  li.append(img, info, play);
  return li;
}

function injectAlbums() {
  paintAlbumToggle();
  if (!albumsShown() || !ALBUM_PAGES.includes(view)) return;
  if (view === "likes" && likesFilter !== "all") return;
  let tiles = viewAlbums.slice();
  if ((view === "discover" || view === "search") && curPlat(view) === "spotify") {
    const seen = new Set(tiles.map((a) => a.id));
    viewTracks.forEach((t) => {
      const a = t.spAlbum;
      if (a && a.album_type === "album" && a.total_tracks > 1 && !seen.has("spa:" + a.id)) {
        seen.add("spa:" + a.id);
        tiles.push(spAlbumTile(a));
      }
    });
    tiles = tiles.slice(0, 10);
  }
  if (!tiles.length) return;
  const box = $("list");
  const rows = [...box.children];
  // one tile every 6 tracks; a couple more at the end, and more appear as the list grows
  tiles.slice(0, Math.ceil(rows.length / 6) + 2).forEach((a, k) => {
    const before = rows[(k + 1) * 6];
    if (before) box.insertBefore(albumTileEl(a), before);
    else box.append(albumTileEl(a));
  });
}

function albumAsPlaylist(a) {
  if (a.source === "soundcloud") return Object.assign({}, a.raw);
  if (a.source === "spotify") return { source: "spotify", id: a.id, spAlbumId: a.spAlbumId, title: a.title, user: { username: a.artist } };
  return { source: "local", id: "album:" + a.title, album: a.title, title: a.title, user: { username: a.artist } };
}

async function albumTracks(a) {
  if (a.source === "local") return localTracks.filter((t) => (t.album || "").trim() === a.title);
  if (a.source === "spotify") return spotifyAlbumTracks(a.spAlbumId);
  return asList(await api("/playlists/" + a.raw.id + "/tracks?access=playable&limit=200&linked_partitioning=true"));
}

async function playAlbum(a) {
  say("Loading " + a.title + "...");
  try {
    const list = await albumTracks(a);
    if (!list.length) { say("No playable tracks in this album."); return; }
    say("");
    startList(list, 0, false);
  } catch (e) {
    say(String(e));
  }
}

function v38Init() {
  $("albumToggle").addEventListener("click", () => {
    prefs.showAlbums = !albumsShown();
    savePrefs();
    albumsKey = "";
    if (albumsShown() && curPage) albumsOnPage(curPage);
    render();
  });
  // Album tiles handle their own clicks, ahead of the track-row handlers
  const tileOf = (e) => e.target.closest && e.target.closest(".albumTile");
  $("list").addEventListener("click", (e) => {
    const li = tileOf(e);
    if (!li) return;
    e.stopPropagation();
    if (e.target.closest(".albumPlay")) playAlbum(li._album);
    else openPlaylist(albumAsPlaylist(li._album));
  }, true);
  ["dblclick", "contextmenu"].forEach((type) => $("list").addEventListener(type, (e) => {
    if (!tileOf(e)) return;
    e.stopPropagation();
    if (type === "contextmenu") e.preventDefault();
  }, true));
}

// ---------- hooks called from app.js ----------
function trackStarted(t) {
  fadeIn(document.querySelector(".nowText"));
  fadeIn(document.querySelector(".npInfo"));
  widgetCleared = false;
  setTimeout(widgetPush, 300);
  if ((prefs.normalize !== "off" || prefs.eqOn) && normEnsure()) norm.fastUntil = performance.now() + 4000;
  session = { id: t.id, secs: 0, last: null, counted: false, scrobbled: false, startedAt: Date.now() };
  presenceUpdate();
  if (prefs.artAccent) updateArtAccent(t);
  if (view === "now") ensureLyrics();
  lfmNowPlaying(t);
}

function playbackTick() {
  const t = curTrack();
  if (!t || srcOf(t) === "spotify" || !session || session.id !== t.id) return;
  const now = audio.currentTime;
  if (session.last !== null) {
    const d = now - session.last;
    if (d > 0 && d < 2) session.secs += d;
  }
  session.last = now;
  const durS = (t.duration || (isFinite(audio.duration) ? audio.duration * 1000 : 0)) / 1000;
  if (!session.counted && session.secs >= Math.min(30, durS ? durS / 2 : 30)) {
    session.counted = true;
    countPlay(t);
  }
  if (!session.scrobbled && durS > 30 && session.secs >= Math.min(durS / 2, 240)) {
    session.scrobbled = true;
    lfmScrobble(t, session.startedAt);
  }
  if (lyricsLines && view === "now") highlightLyric(now);
}

function playbackState(playing) {
  widgetPush();
  if (playing && norm.ctx && norm.ctx.state === "suspended") norm.ctx.resume().catch(() => {});
  taskbarSync();
  if ("mediaSession" in navigator) {
    try { navigator.mediaSession.playbackState = playing ? "playing" : "paused"; } catch (e) {}
  }
  presenceUpdate();
}

// ---------- settings ----------
function extrasSyncSettings() {
  spModePaint();
  $("safeOut").checked = prefs.safeOutput !== false;
  reqPaint();
  $("artAccentToggle").checked = !!prefs.artAccent;
  $("normSel").value = prefs.normalize || "off";
  $("customCss").value = prefs.css || "";
  $("presToggle").checked = !!prefs.presence;
  $("scrobbleToggle").checked = !!prefs.scrobble;
  $("trayToggle").checked = !!prefs.closeToTray;
  lfmRefresh();
}

function extrasInit() {
  repeatMode = prefs.repeat || "off";
  shuffleOn = !!prefs.shuffle;
  paintModes();

  $("shuffleBtn").addEventListener("click", toggleShuffle);
  $("repeatBtn").addEventListener("click", cycleRepeat);
  $("lyricsBtn").addEventListener("click", () => setLyricsMode(!lyricsMode));
  $("navStats").addEventListener("click", () => showView("stats"));
  queueDndInit();

  audio.addEventListener("play", () => playbackState(true));
  audio.addEventListener("pause", () => playbackState(false));
  audio.addEventListener("seeked", presenceUpdate);
  audio.addEventListener("timeupdate", playbackTick);

  // Windows media overlay buttons (if the engine shows one)
  if ("mediaSession" in navigator) {
    const handlers = [
      ["play", () => { if (audio.paused) toggle(); }],
      ["pause", () => { if (!audio.paused) toggle(); }],
      ["previoustrack", () => step(-1)],
      ["nexttrack", () => step(1)],
    ];
    handlers.forEach(([action, fn]) => { try { navigator.mediaSession.setActionHandler(action, fn); } catch (e) {} });
  }

  $("artAccentToggle").addEventListener("change", () => {
    prefs.artAccent = $("artAccentToggle").checked;
    savePrefs();
    syncAccent2();
    updateArtAccent(curTrack());
  });
  $("customCss").addEventListener("input", () => {
    prefs.css = $("customCss").value;
    savePrefs();
    applyUserCss();
  });
  $("presToggle").addEventListener("change", () => {
    prefs.presence = $("presToggle").checked;
    savePrefs();
    presenceUpdate();
  });
  $("scrobbleToggle").addEventListener("change", () => {
    prefs.scrobble = $("scrobbleToggle").checked;
    savePrefs();
  });
  $("trayToggle").addEventListener("change", () => {
    prefs.closeToTray = $("trayToggle").checked;
    savePrefs();
  });
  $("lfmConnect").addEventListener("click", lfmConnectClick);
  playlistsInit();
  widgetInit();
  marqueeInit();
  v26Init();
  v28Init();
  v29Init();
  v30Init();
  v32Init();
  v33Init();
  v35Init();
  v38Init();
  // Route the player through the equalizer / leveler from the start, so changes are heard
  // immediately rather than from the next track (it has to be in place before audio loads)
  audio.crossOrigin = "anonymous";
  normEnsure();

  // Loudness leveling
  $("normSel").value = prefs.normalize || "off";
  if (prefs.normalize !== "off" || prefs.eqOn) audio.crossOrigin = "anonymous";
  setInterval(normTick, 100);
  $("normSel").addEventListener("change", () => {
    prefs.normalize = $("normSel").value;
    savePrefs();
    if (prefs.normalize !== "off" || prefs.eqOn) audio.crossOrigin = "anonymous";
    if (norm.ctx) {
      normApply();
      $("normNote").textContent = "Brings quiet tracks up and loud ones down, like a leveling compressor.";
    } else if (prefs.normalize !== "off") {
      $("normNote").textContent = "Starts with the next track you play.";
    }
  });

  // Recommendation language
  $("recLangSel").value = prefs.recLang || "any";
  $("recLangSel").addEventListener("change", () => {
    prefs.recLang = $("recLangSel").value;
    savePrefs();
    if (view === "discover") openDiscover(discoverSrc);
  });



  lfmRefresh();
}
