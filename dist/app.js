const { invoke } = window.__TAURI__.core;

const $ = (id) => document.getElementById(id);
const audio = new Audio();
const win = window.__TAURI__.window.getCurrentWindow();

// ---------- preferences (stored in this app's browser profile) ----------
const DEFAULTS = { theme: "dark", accent: "#8e9bff", density: "comfortable", sink: "default", volume: 0.8, queue: false, repeat: "off", shuffle: false, artAccent: false, css: "", presence: true, scrobble: true, closeToTray: false, normalize: "off", recLang: "en" };
let prefs = { ...DEFAULTS };
try { prefs = { ...DEFAULTS, ...JSON.parse(localStorage.getItem("sc.prefs") || "{}") }; } catch (e) {}
const savePrefs = () => { try { localStorage.setItem("sc.prefs", JSON.stringify(prefs)); } catch (e) {} };

// Pick dark or light text so it stays readable on any accent color
function onAccent(hex) {
  const n = parseInt(hex.slice(1), 16);
  const r = (n >> 16) & 255, g = (n >> 8) & 255, b = n & 255;
  return 0.299 * r + 0.587 * g + 0.114 * b > 150 ? "#111318" : "#ffffff";
}

function applyPrefs() {
  const root = document.documentElement;
  root.dataset.theme = prefs.theme;
  root.dataset.density = prefs.density;
  if (prefs.artAccent && artColors) setAccentColors(artColors[0], artColors[1]);
  else setAccentColors(prefs.accent, prefs.accent2 || prefs.accent);
  applyUserCss();
}
applyPrefs();

// ---------- helpers ----------
const SVGNS = "http://www.w3.org/2000/svg";
function icon(d) {
  const s = document.createElementNS(SVGNS, "svg");
  s.setAttribute("viewBox", "0 0 24 24");
  const p = document.createElementNS(SVGNS, "path");
  p.setAttribute("d", d);
  s.append(p);
  return s;
}
function el(tag, cls, text) {
  const n = document.createElement(tag);
  if (cls) n.className = cls;
  if (text !== undefined) n.textContent = text;
  return n;
}
// Status and error messages slide out in a strip above the player bar, then go again.
// "Loading..." style messages stay until they're replaced or cleared; say("") clears.
let statusTimer = null;
function say(msg) {
  const bar = $("statusBar");
  clearTimeout(statusTimer);
  msg = msg ? String(msg) : "";
  if (!msg) { bar.classList.remove("show"); return; }
  const busy = /(\.\.\.|\u2026)$/.test(msg);
  const bad = !busy && /^(could not|couldn't|can't|error|stream error|scrobble failed)|failed|problem|expired|isn't available/i.test(msg);
  $("statusText").textContent = msg;
  bar.classList.toggle("busy", busy);
  bar.classList.toggle("bad", bad);
  bar.classList.add("show");
  statusTimer = setTimeout(() => bar.classList.remove("show"), busy ? 30000 : bad ? 7000 : 4000);
}
const api = (path) => invoke("api_get", { path });

const ICON_PLAY = "M8 5v14l11-7z";
const ICON_ADD = "M19 13h-6v6h-2v-6H5v-2h6V5h2v6h6v2z";
const ICON_NEXT = "M6 18l8.5-6L6 6v12zM16 6v12h2V6h-2z";
const ICON_HEART = "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z";
const ICON_HEART_OUT = "M16.5 3c-1.74 0-3.41.81-4.5 2.09C10.91 3.81 9.24 3 7.5 3 4.42 3 2 5.42 2 8.5c0 3.78 3.4 6.86 8.55 11.54L12 21.35l1.45-1.32C18.6 15.36 22 12.28 22 8.5 22 5.42 19.58 3 16.5 3zm-4.4 15.55l-.1.1-.1-.1C7.14 14.24 4 11.39 4 8.5 4 6.5 5.5 5 7.5 5c1.54 0 3.04.99 3.57 2.36h1.87C13.46 5.99 14.96 5 16.5 5c2 0 3.5 1.5 3.5 3.5 0 2.89-3.14 5.74-7.9 10.05z";
const ICON_OPEN = "M19 19H5V5h7V3H5c-1.11 0-2 .9-2 2v14c0 1.1.89 2 2 2h14c1.1 0 2-.9 2-2v-7h-2v7zM14 3v2h3.59l-9.83 9.83 1.41 1.41L19 6.41V10h2V3h-7z";
const ICON_PLIST = "M14 10H2v2h12v-2zm0-4H2v2h12V6zm4 8v-4h-2v4h-4v2h4v4h2v-4h4v-2h-4zM2 16h8v-2H2v2z";
const ICON_CLOSE = "M19 6.41L17.59 5 12 10.59 6.41 5 5 6.41 10.59 12 5 17.59 6.41 19 12 13.41 17.59 19 19 17.59 13.41 12z";

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
  spotify: { name: "Spotify", icon: "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" },
  xpl: { name: "Playlist on this PC", icon: "M15 6H3v2h12V6zm0 4H3v2h12v-2zM3 16h8v-2H3v2zM17 6v8.18c-.31-.11-.65-.18-1-.18-1.66 0-3 1.34-3 3s1.34 3 3 3 3-1.34 3-3V8h3V6h-5z" },
  local: { name: "Local files", icon: "M10 4H4c-1.1 0-1.99.9-1.99 2L2 18c0 1.1.9 2 2 2h16c1.1 0 2-.9 2-2V8c0-1.1-.9-2-2-2h-8l-2-2z" },
  youtube: { name: "YouTube Music", icon: "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" },
  apple: { name: "Apple Music", icon: "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" },
  amazon: { name: "Amazon Music", icon: "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z" },
};
const srcOf = (t) => (t && t.source) || "soundcloud";

function platformBadge(t) {
  const key = srcOf(t);
  const p = PLATFORMS[key];
  const b = el("span", "plat " + key);
  b.title = "Playing from " + p.name;
  b.setAttribute("aria-label", p.name);
  b.append(icon(p.icon));
  return b;
}

function setBadge(box, t) {
  box.replaceChildren();
  if (t) box.append(platformBadge(t));
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
const isLiked = (t) => !!t && likeState.get(t.id) === true;
let userName = "";
let meId = null;
let artistFollowing = null;   // true / false, or null while checking
let currentArtist = null;
let discoverSrc = "";
let discoverNote = "";

// ---------- views ----------
function showView(v) {
  view = v;
  const isNow = v === "now";
  if (!isNow) back = v;
  $("listView").hidden = isNow || v === "stats" || (v === "search" && !query);
  $("searchPane").hidden = v !== "search";
  $("searchPane").classList.toggle("empty", v === "search" && !query);
  $("searchForm").style.visibility = v === "search" ? "hidden" : "";
  $("likesChips").hidden = v !== "likes";
  $("navRediscover").classList.toggle("active", v === "rediscover");
  navTrack(v);
  $("npView").hidden = !isNow;
  $("statsView").hidden = v !== "stats";
  $("navStats").classList.toggle("active", v === "stats");
  if (v === "stats") renderStats();
  if (isNow) ensureLyrics();
  else if (lyricsMode) setLyricsMode(false);
  $("navDiscover").classList.toggle("active", v === "discover");
  if (!isNow) $("chips").hidden = v !== "discover";
  if (!isNow) $("langChip").hidden = v !== "discover";
  $("navLikes").classList.toggle("active", v === "likes");
  $("navSearch").classList.toggle("active", v === "search");
  document.querySelectorAll(".plItem").forEach((b) => {
    b.classList.toggle("active", v === "playlist" && currentPlaylist && String(b.dataset.id) === String(currentPlaylist.id));
  });
}

function renderHero() {
  const v = view === "now" ? back : view;
  let title = "Likes";
  let sub = "";
  if (v === "rediscover") {
    title = "Rediscover";
    sub = "";   // set below, in two parts
  } else if (v === "discover") {
    title = "Discover";
    sub = discoverNote ? discoverNote + ". " : "";
    if (recHidden) sub += recHidden + " hidden by the language filter. ";
  } else if (v === "search") {
    title = 'Results for "' + query + '"';
  } else if (v === "playlist" && currentPlaylist) {
    title = currentPlaylist.title || "Playlist";
    if (isXpl(currentPlaylist)) sub = "On this PC. Holds tracks from any platform.";
    else if (currentPlaylist.user && currentPlaylist.user.username) sub = "By " + currentPlaylist.user.username + ". ";
  } else if (v === "artist" && currentArtist) {
    title = currentArtist.username || "Artist";
    if (currentArtist.followers_count) sub = Number(currentArtist.followers_count).toLocaleString() + " followers. ";
  }
  const n = viewTracks.length;
  $("viewTitle").textContent = title;
  $("refreshBtn").hidden = !(v === "discover" || v === "rediscover");
  $("viewSub").textContent = sub.trim();
  if (v === "rediscover") {
    $("viewSub").replaceChildren(el("span", "subLead", "Fall in love all over again"),
      el("span", "", " with the likes you haven't heard in a while."));
  }
  const isArtist = v === "artist" && !!currentArtist;
  $("playAll").hidden = n === 0;
  $("shuffleAll").hidden = n === 0;
  $("followBtn").hidden = !isArtist || (meId !== null && String(currentArtist.id) === String(meId));
  $("plDelete").hidden = !(v === "playlist" && isOwnPlaylist(currentPlaylist));
  const av = $("heroAvatar");
  const src = v === "artist" && currentArtist && currentArtist.avatar_url
    ? currentArtist.avatar_url.replace("-large", "-t200x200") : "";
  if (src) { av.src = src; av.hidden = false; } else { av.removeAttribute("src"); av.hidden = true; }
  paintHeroActions();
}

// Go+ tracks are served to this app as 30-second previews only
function isPreview(t) { return !!t && (t.access === "preview" || t.policy === "SNIP"); }
// Go+ previews and blocked tracks are never shown
function isUnplayable(t) {
  return isPreview(t) || (!!t && t.access === "blocked") || (!!t && /\bpreview\b/i.test(t.title || ""));
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

function afterRender() {
  $("tableHead").hidden = viewTracks.length === 0;
  $("more").hidden = !nextHref;
  renderHero();
  refreshLikeUI();
  renderPlatChips();
  injectAlbums();
  paintHeroActions();
}

// The row under the title: play / shuffle and page buttons, with the view options on the right.
// Shown whenever any of them is, so the language filter stays reachable when it hid everything.
function paintHeroActions() {
  $("heroActions").hidden = !["playAll", "followBtn", "plDelete", "langChip", "albumToggle"].some((id) => !$(id).hidden);
}

function trackRow(t, i, playing) {
  const li = el("li");
  li.dataset.i = i;
  li.draggable = true;   // onto a playlist or the queue (js/dnd.js)
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
  const who = artistName(t);
  const artistLine = el("div", "artist");
  const whoEl = el("span", srcOf(t) === "soundcloud" ? "artistLink" : "artistName", who);
  if (srcOf(t) === "soundcloud") whoEl.title = uploaderNote(t);
  artistLine.append(platformBadge(t), whoEl);
  meta.append(el("div", "title", t.title || "Untitled"), artistLine);
  const cell = el("div", "trackCell");
  cell.append(img, meta);

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
  bOpen.title = srcOf(t) === "spotify" ? "Open in Spotify" : "Open on SoundCloud";
  bOpen.setAttribute("aria-label", bOpen.title);
  bOpen.append(icon(ICON_OPEN));
  const bList = el("button");
  bList.dataset.act = "plist";
  bList.title = "Add to playlist";
  bList.setAttribute("aria-label", "Add to playlist");
  bList.append(icon(ICON_PLIST));
  // Liking is SoundCloud-only; every track can go in a playlist (on this PC, if not SoundCloud)
  if (srcOf(t) === "soundcloud") acts.append(bLike);
  acts.append(bNext, bAdd, bList);
  if (view === "playlist" && isOwnPlaylist(currentPlaylist)) {
    const bRm = el("button");
    bRm.dataset.act = "unlist";
    bRm.title = "Remove from this playlist";
    bRm.setAttribute("aria-label", bRm.title);
    bRm.append(icon(ICON_CLOSE));
    acts.append(bRm);
  } else if (t.permalink_url) acts.append(bOpen);

  li.append(num, cell, acts, el("div", "time", fmt(t.duration || 0)));
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
  if (!li) return;
  const i = Number(li.dataset.i);
  const btn = e.target.closest("button[data-act]");
  if (btn) {
    if (btn.dataset.act === "like") toggleLike(viewTracks[i]);
    else if (btn.dataset.act === "next") playNext(viewTracks[i]);
    else if (btn.dataset.act === "plist") openPlaylistMenu(viewTracks[i], btn);
    else if (btn.dataset.act === "unlist") removeFromPlaylist(viewTracks[i]);
    else if (btn.dataset.act === "open") openOnSoundCloud(viewTracks[i]);
    else addToQueue(viewTracks[i]);
    return;
  }
  if (e.target.closest(".artistLink")) {
    const u = viewTracks[i].user;
    if (u) openArtist(u);
    return;
  }
  startList(viewTracks, i, false);
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
    if (view === "likes") got = applyLikesFilter(got);
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
  renderLikesChips();
  const plats = activePlats("likes");
  if (!plats.length) { showNothing(); return; }
  loadMerged(plats.map(likesFeed), applyLikesFilter);
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
  loadMerged(plats.map((id) => searchFeed(id, q)), (list) => list.filter((t) => !isMix(t)));
}

function openPlaylist(p) {
  currentPlaylist = p;
  showView("playlist");
  if (isXpl(p)) { ++loadToken; $("listView").scrollTop = 0; setLocalPool(xplTracks(xplFind(p.id) || { tracks: [] })); render(); say(viewTracks.length ? "" : "Nothing in here yet. Add tracks with the playlist button on any track, or drag them onto this playlist."); return; }
  if (p.source === "spotify" || p.source === "local") { loadPlatformPlaylist(p); return; }
  loadTracks("/playlists/" + p.id + "/tracks?access=playable&limit=50&linked_partitioning=true", false, p.tracks);
}

function openArtist(u) {
  if (!u || !u.id) return;
  currentArtist = { id: u.id, username: u.username, avatar_url: u.avatar_url };
  currentPlaylist = null;
  showView("artist");
  $("listView").scrollTop = 0;
  loadTracks("/users/" + u.id + "/tracks?access=playable&limit=50&linked_partitioning=true", false);
  // Are we following this artist? A 404 from SoundCloud means no.
  artistFollowing = null;
  paintFollow();
  api("/me/followings/" + u.id)
    .then(() => { if (currentArtist && currentArtist.id === u.id) { artistFollowing = true; paintFollow(); } })
    .catch(() => { if (currentArtist && currentArtist.id === u.id) { artistFollowing = false; paintFollow(); } });
  // Fetch follower count and a larger picture; fine if this fails
  api("/users/" + u.id).then((full) => {
    if (currentArtist && currentArtist.id === u.id) {
      currentArtist = { ...currentArtist, ...full };
      renderHero();
    }
  }).catch(() => {});
}

// ---------- discover ----------
const GENRES = ["Electronic", "House", "Techno", "Hip-hop & Rap", "Pop", "Indie", "Ambient", "Lo-fi", "R&B", "Drum & Bass", "Jazz", "Rock"];

function renderChips() {
  const box = $("chips");
  box.replaceChildren();
  // "Following" needs SoundCloud: Spotify doesn't let this app see who you follow
  const first = activePlats("discover").includes("soundcloud")
    ? [["foryou", "For you"], ["following", "Following"], ["similar", "Similar artists"]]
    : [["foryou", "For you"], ["similar", "Similar artists"]];
  const items = first.concat(GENRES.map((g) => ["genre:" + g, g]));
  items.forEach(([key, label]) => {
    const b = el("button", "chip" + (key === discoverSrc ? " on" : ""), label);
    b.dataset.src = key;
    box.append(b);
  });
}

$("chips").addEventListener("click", (e) => {
  const b = e.target.closest(".chip");
  if (b) openDiscover(b.dataset.src);
});
// The genre chips are one sideways-scrolling line: let the mouse wheel move it, and drop the
// fade at the right once the end is reached
$("chips").addEventListener("wheel", (e) => {
  const box = $("chips");
  if (box.scrollWidth <= box.clientWidth || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
  e.preventDefault();
  box.scrollLeft += e.deltaY;
}, { passive: false });
$("chips").addEventListener("scroll", () => {
  const box = $("chips");
  box.classList.toggle("atEnd", box.scrollLeft + box.clientWidth >= box.scrollWidth - 2);
});

// Every active platform's picker runs at once (js/discover.js); their suggestions are mixed
const DISCOVER_PICKERS = { soundcloud: (src) => scDiscover(src), spotify: (src) => spDiscover(src) };

async function openDiscover(src, fresh) {
  const plats = activePlats("discover");
  if (!plats.length) { openLikes(); return; }
  discoverSrc = src || "foryou";
  if (discoverSrc === "following" && !plats.includes("soundcloud")) discoverSrc = "foryou";
  recHidden = 0;
  discoverNote = "";
  currentPlaylist = null;
  currentArtist = null;
  showView("discover");
  renderChips();
  if (!fresh && useCachedPage()) return;   // came back to it: keep the same list
  $("listView").scrollTop = 0;
  const tok = ++loadToken;
  say("Finding music for you...");
  if (plats.includes("soundcloud")) await ensureLikes();   // so nothing you already like gets recommended
  const results = await Promise.allSettled(plats.map((p) => DISCOVER_PICKERS[p](discoverSrc)));
  if (tok !== loadToken) return;
  const found = results.filter((r) => r.status === "fulfilled").map((r) => r.value);
  const errors = results.filter((r) => r.status === "rejected").map((r) => (r.reason && r.reason.message) || String(r.reason));
  const seen = new Set();
  const list = interleave(found.map((r) => shuffleInPlace(r.list.slice()))).filter((t) => !seen.has(t.id) && seen.add(t.id));
  discoverNote = found.map((r) => r.note).filter(Boolean).join(". ");
  setLocalPool(list);
  render();
  const hints = found.map((r) => r.msg).filter(Boolean);
  say(list.length ? hints.join(" ") : errors[0] || hints[0] || "No suggestions came back. Try another chip or a genre.");
}

$("navDiscover").addEventListener("click", () => openDiscover(discoverSrc || "foryou"));
function paintFollow() {
  const b = $("followBtn");
  b.disabled = artistFollowing === null;
  b.classList.toggle("on", artistFollowing === true);
  b.textContent = artistFollowing ? "Following" : "Follow";
  b.title = artistFollowing ? "Click to unfollow" : "Follow this artist";
}

$("followBtn").addEventListener("click", async () => {
  if (!currentArtist || artistFollowing === null) return;
  const id = currentArtist.id;
  const next = !artistFollowing;
  const bump = (d) => {
    if (typeof currentArtist.followers_count === "number") currentArtist.followers_count += d;
  };
  artistFollowing = next;          // update right away, undo if SoundCloud refuses
  bump(next ? 1 : -1);
  paintFollow();
  renderHero();
  try {
    await invoke("set_follow", { id: String(id), follow: next });
  } catch (e) {
    if (currentArtist && currentArtist.id === id) {
      artistFollowing = !next;
      bump(next ? -1 : 1);
      paintFollow();
      renderHero();
    }
    say("Could not " + (next ? "follow" : "unfollow") + ": " + e);
  }
});

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
$("navSearch").addEventListener("click", openSearchPage);
$("bigSearchForm").addEventListener("submit", (e) => {
  e.preventDefault();
  const q = $("bigQ").value.trim();
  if (q) openSearch(q);
});
$("searchForm").addEventListener("submit", (e) => {
  e.preventDefault();
  const q = $("q").value.trim();
  if (q) openSearch(q);
});
// Infinite scroll: load the next page when the end of the list comes near
let loadingMore = false;
async function loadMore() {
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
$("shuffleAll").addEventListener("click", () => startList(viewTracks, 0, true));

// ---------- playlists (sidebar) ----------
function renderPlaylists(note) {
  const box = $("plList");
  box.replaceChildren();
  if (note) box.append(el("div", "plNote", note));
  // playlists first, then albums from your files under their own heading
  const item = (p, into) => {
    const b = el("button", "plItem");
    b.dataset.id = p.id;
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
function startList(list, i, shuffle) {
  if (!list.length) return;
  queue = list.slice();
  if (shuffle) { shuffleInPlace(queue); i = 0; }
  qi = i;
  if (shuffleOn && !shuffle) shuffleUpcoming();
  playCurrent();
}

function playNext(t) {
  if (!t) return;
  if (qi < 0) { startList([t], 0, false); return; }
  queue.splice(qi + 1, 0, t);
  renderQueue();
}

function addToQueue(t) {
  if (!t) return;
  if (qi < 0) { startList([t], 0, false); return; }
  queue.splice(autoplayStart(), 0, t);   // ahead of any autoplay picks
  renderQueue();
}

function queueRow(t, idx, removable) {
  const li = el("li", "qRow" + (idx === qi ? " cur" : ""));
  li.dataset.q = idx;
  if (removable) li.draggable = true;
  const img = el("img");
  img.alt = "";
  img.loading = "lazy";
  const src = artUrl(t, "t120x120");
  if (src) img.src = src;
  const meta = el("div", "meta");
  const qa = el("div", "artist");
  qa.append(platformBadge(t), el("span", "", artistName(t)));
  if (t.requestedBy) qa.append(el("span", "reqBy", "requested by " + t.requestedBy));
  else if (t.autoplay) qa.append(el("span", "reqBy", "autoplay"));
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
  const nowBox = $("qNow");
  const nextBox = $("qNext");
  nowBox.replaceChildren();
  nextBox.replaceChildren();
  const t = curTrack();
  if (t) nowBox.append(queueRow(t, qi, false));
  else nowBox.append(el("li", "qEmpty", "Nothing playing."));
  const upcoming = qi >= 0 ? queue.slice(qi + 1) : [];
  if (!upcoming.length) nextBox.append(el("li", "qEmpty", autoplayNote()));
  // A long queue (all your likes, say) only draws the next stretch of it
  upcoming.slice(0, QUEUE_SHOWN).forEach((tr, k) => nextBox.append(queueRow(tr, qi + 1 + k, true)));
  if (upcoming.length > QUEUE_SHOWN) nextBox.append(el("li", "qEmpty", "...and " + (upcoming.length - QUEUE_SHOWN) + " more."));
  reqQueueBtn();
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
  if (idx !== qi) { qi = idx; playCurrent(); }
});

$("clearQueue").addEventListener("click", () => {
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
function fillNowPlaying(t) {
  $("nowTitle").textContent = t ? t.title || "Untitled" : "Nothing playing";
  $("nowArtist").textContent = t ? artistName(t) : "";
  $("nowArtist").title = t ? uploaderNote(t) : "";
  $("nowAlbum").textContent = t ? t.album || (t.spAlbum && t.spAlbum.name) || "" : "";
  $("plAddBtn").hidden = !t;
  $("nowToggle").draggable = !!t;
  $("moreBtn").hidden = !t;
  setBadge($("nowPlat"), t);
  const small = t ? artUrl(t, "t120x120") : "";
  fadeImg($("nowArt"), small);

  $("npTitle").textContent = t ? t.title || "Untitled" : "Nothing playing";
  $("npArtist").textContent = t ? artistName(t) : "";
  $("npArtist").title = t ? uploaderNote(t) : "";
  setBadge($("npPlat"), t);
  const bits = [];
  if (t && t.genre) bits.push(t.genre);
  const year = t && t.created_at ? String(t.created_at).slice(0, 4) : "";
  if (/^\d{4}$/.test(year)) bits.push(year);
  if (t && t.playback_count) bits.push(Number(t.playback_count).toLocaleString() + " plays");
  $("npMeta").textContent = bits.join(", ");
  $("npDesc").textContent = t && t.description ? t.description.slice(0, 600) : "";
  const big = t ? artUrl(t, "t500x500") : "";
  if (big) { fadeImg($("npArt"), big); $("npArt").hidden = false; } else { $("npArt").hidden = true; }
  $("npOpen").hidden = !(t && t.permalink_url);
  $("npOpen").textContent = t && srcOf(t) === "spotify" ? "Open in Spotify" : "Open on SoundCloud";
  $("dur").textContent = fmt(t ? t.duration || 0 : 0);

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
  if (t && t.user && srcOf(t) === "soundcloud") openArtist(t.user);
}
$("nowArtist").addEventListener("click", openCurrentArtist);
$("npArtist").addEventListener("click", openCurrentArtist);

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
  lb.hidden = !t || srcOf(t) !== "soundcloud";
  if (t) setHeart(lb, isLiked(t));
  const nb = $("npLike");
  nb.hidden = !t || srcOf(t) !== "soundcloud";
  if (t) {
    nb.textContent = isLiked(t) ? "Liked" : "Like";
    nb.classList.toggle("on", isLiked(t));
  }
  document.querySelectorAll("#list li").forEach((li) => {
    const t2 = viewTracks[Number(li.dataset.i)];
    const b = li.querySelector('button[data-act="like"]');
    if (t2 && b && b.classList.contains("on") !== isLiked(t2)) setHeart(b, isLiked(t2));
  });
}

async function toggleLike(t) {
  if (!t || srcOf(t) !== "soundcloud") return;
  const next = !isLiked(t);
  likeState.set(t.id, next);       // update right away, undo if SoundCloud refuses
  refreshLikeUI();
  try {
    await invoke("set_like", { id: String(t.id), like: next });
  } catch (e) {
    likeState.set(t.id, !next);
    refreshLikeUI();
    say("Could not " + (next ? "like" : "unlike") + " this track: " + e);
  }
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

// ---------- playback ----------
async function playCurrent() {
  const t = curTrack();
  if (!t) return;
  fillNowPlaying(t);
  refreshLikeUI();
  trackStarted(t);
  markPlaying();
  renderQueue();
  say("");
  try {
    if (srcOf(t) === "spotify") {
      audio.pause();
      await spotifyStart(t);
    } else {
      spotifyPause();
      await startStream(t);
    }
  } catch (e) {
    if (e && e.name === "AbortError") return;   // skipped to another track
    say("Could not play this track: " + e);
  }
}

// Full tracks arrive as HLS, which the browser engine can't play without hls.js
let hls = null;
let streamToken = 0;   // ignore results for tracks we've already skipped past

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
    await tryStream(t.url, false, null);
    if (mine === streamToken) say("");
    return;
  }
  const info = await invoke("stream_url", { id: String(t.id) });
  if (mine !== streamToken) return;
  const failures = [];
  for (const c of info.candidates || []) {
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
      if (c.name === "preview") {
        say(isPreview(t) ? "Go+ track: playing the 30-second preview." : "Full stream unavailable, so this is only the short preview. " + (info.note || ""));
      } else {
        say("");
      }
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

function toggle() {
  const cur = curTrack();
  if (cur && srcOf(cur) === "spotify") { spotifyToggle(); return; }
  if (!audio.src) { if (viewTracks.length) startList(viewTracks, 0, false); return; }
  if (audio.paused) audio.play(); else audio.pause();
}

function step(d) {
  if (!queue.length) return;
  // Previous restarts the track if it has been playing for a few seconds
  const cur = curTrack();
  const onSpotify = !!cur && srcOf(cur) === "spotify";
  if (d < 0 && onSpotify && spotifyPos() > 3000) { spotifySeek(0); return; }
  if (d < 0 && !onSpotify && audio.currentTime > 3) { audio.currentTime = 0; return; }
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
audio.addEventListener("timeupdate", () => {
  $("cur").textContent = fmt(audio.currentTime * 1000);
  const d = curDurMs();
  if (d) $("seek").value = String(Math.round(Math.min(1, (audio.currentTime * 1000) / d) * 1000));
});
audio.addEventListener("loadedmetadata", () => {
  if (isFinite(audio.duration) && audio.duration) $("dur").textContent = fmt(audio.duration * 1000);   // streams of unknown length report Infinity
});

$("seek").addEventListener("input", () => {
  const cur = curTrack();
  if (cur && srcOf(cur) === "spotify") { spotifySeek(Number($("seek").value) / 1000); return; }
  const d = curDurMs();
  if (d) audio.currentTime = (Number($("seek").value) / 1000) * (d / 1000);
});

audio.volume = prefs.volume;
$("vol").value = String(Math.round(prefs.volume * 100));
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
$("prev").addEventListener("click", () => step(-1));
$("next").addEventListener("click", () => step(1));

// Media keys, the tray menu and the taskbar buttons call this from the Rust side
window.__mediaAction = (a) => {
  if (a === "play") toggle();
  else if (a === "next") step(1);
  else if (a === "prev") step(-1);
  else if (a === "shuffle") toggleShuffle();
  else if (a === "like") toggleLike(curTrack());
};

const dlg = $("settings");
document.addEventListener("keydown", (e) => {
  if (dlg.open) return;
  if (e.target.closest("input, select, textarea, button, [role='button']")) return;
  if (e.ctrlKey || e.altKey || e.metaKey) return;
  if (e.code === "Space") { e.preventDefault(); toggle(); }
  else if (e.shiftKey && e.code === "ArrowRight") step(1);
  else if (e.shiftKey && e.code === "ArrowLeft") step(-1);
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
  $("themeSel").value = prefs.theme;
  document.querySelectorAll("#densitySeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === prefs.density));
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

$("themeSel").addEventListener("change", () => { setPref("theme", $("themeSel").value); if (prefs.artAccent && curTrack()) updateArtAccent(curTrack()); });
$("densitySeg").addEventListener("click", (e) => { const b = e.target.closest("button"); if (b) setPref("density", b.dataset.v); });
$("swatches").addEventListener("click", (e) => { const b = e.target.closest(".swatch"); if (b) setPref("accent", b.dataset.c); });
$("swatches").addEventListener("input", (e) => { if (e.target.id === "accentCustom") setPref("accent", e.target.value); });

// ---------- settings: audio output ----------
async function applySink(id) {
  if (!audio.setSinkId) throw new Error("This engine cannot choose an output device.");
  await audio.setSinkId(id || "default");
  if (norm.ctx && norm.ctx.setSinkId) await norm.ctx.setSinkId(id && id !== "default" ? id : "");
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

$("outputSel").addEventListener("change", async () => {
  const id = $("outputSel").value;
  try {
    await applySink(id);
    prefs.sink = id;
    savePrefs();
    $("outputNote").textContent = "";
  } catch (e) {
    $("outputNote").textContent = "Could not switch output: " + e;
  }
});

// Browsers only reveal device names after a microphone permission is granted.
// We open the mic for an instant and close it right away. Nothing is recorded.
$("namesBtn").addEventListener("click", async () => {
  try {
    const s = await navigator.mediaDevices.getUserMedia({ audio: true });
    s.getTracks().forEach((t) => t.stop());
    $("outputNote").textContent = "";
  } catch (e) {
    $("outputNote").textContent = e && e.name === "NotAllowedError"
      ? "Access is still blocked from an earlier choice. Close SC Client, delete the folder %LOCALAPPDATA%\\dev.scclient.app\\EBWebView, then reopen it."
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
$("outputBtn").addEventListener("click", () => openSettings("audioSection"));
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
  $("scNote").textContent = signedIn ? "Signed in" + (userName ? " as " + userName : "") + "." : "Not signed in.";
  const text = $("titleText");
  const prompt = $("titlePrompt");
  lastSignedIn = signedIn;
  const custom = (prefs.appName || "").trim();
  let title;
  if (signedIn || custom || !scOn()) {
    title = custom || (userName ? userName + "'s Music" : "Your Music");
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

async function loadUser() {
  try {
    const me = await api("/me");
    userName = me.username || me.full_name || "";
    meId = me.id;
  } catch (e) {
    userName = "";
  }
  updateTitle(true);
}

$("titlePrompt").addEventListener("click", signIn);
$("statusBar").addEventListener("click", () => say(""));
$("winMin").addEventListener("click", () => win.minimize());
$("winMax").addEventListener("click", () => win.toggleMaximize());
$("winClose").addEventListener("click", () => { if (prefs.closeToTray) win.hide(); else win.close(); });

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
