// Platforms (SoundCloud, Spotify, local files), local music, and the sidebar for each.

const scOn = () => prefs.scOn !== false;

function scApply() {
  renderPlatChips();
}

// ---------- platforms: every page shows all the active ones together ----------
// A platform is available once it's set up (signed in, Premium, or has music folders), and active
// while it's also switched on: in Settings > Integrations, or with its icon in the title bar.
// To add a platform: an entry here (and an icon in PLATFORMS in app.js), then teach the feeds
// below (and DISCOVER_PICKERS in app.js) how to fetch its tracks.
const PLATFORM_LIST = [
  { id: "soundcloud", pref: "scOn", pages: ["likes", "search", "discover", "rediscover"], available: () => lastSignedIn },
  { id: "spotify", pref: "spOn", pages: ["likes", "search", "discover", "rediscover"], available: () => FEATURES.spotify && spotifyPremium },
  { id: "local", pref: "localOn", pages: ["likes", "search", "rediscover"], available: () => localFolderList.length > 0 },
];

const platformOn = (p) => prefs[p.pref] !== false;

// The ids of the active platforms (that a page supports, if one is given)
function activePlats(page) {
  return PLATFORM_LIST.filter((p) => (!page || p.pages.includes(page)) && p.available() && platformOn(p)).map((p) => p.id);
}

const LIST_PAGES = ["likes", "search", "discover", "rediscover"];
const NOTHING_ON = "Nothing to show yet: switch on SoundCloud, Spotify or Local music in Settings > Integrations, or with the icons in the title bar.";

// Title bar: one icon per available platform, lit while it's active; click to switch it on or off
function renderPlatChips() {
  $("navDiscover").hidden = !activePlats("discover").length;
  // When the active platforms change (switched on or off, or Spotify / your files finished
  // loading), rebuild the sidebar and reload the page so it shows the right mix
  // (local files count as changed once their tracks arrive, which is a moment after their folders)
  const active = activePlats();
  const key = active.join(",") + (active.includes("local") && localTracks.length ? "+files" : "");
  if (key !== sidebarKey) {
    const first = sidebarKey === null;
    sidebarKey = key;
    refreshSidebar();
    if (!first && curPage && LIST_PAGES.includes(curPage.view)) {
      restoring = true;
      try { restorePage(curPage); } finally { restoring = false; }
    }
  }
  const box = $("titlePlat");
  const plats = PLATFORM_LIST.filter((p) => p.available());
  box.hidden = plats.length < 2;
  box.replaceChildren();
  if (plats.length < 2) return;
  plats.forEach((p) => {
    const on = platformOn(p);
    const b = el("button", on ? "on" : "");
    b.dataset.p = p.id;
    b.title = PLATFORMS[p.id].name + (on ? " is on. Click to hide it everywhere." : " is off. Click to show it again.");
    b.setAttribute("aria-label", PLATFORMS[p.id].name);
    b.setAttribute("aria-pressed", on ? "true" : "false");
    b.append(platIcon(p.id));
    box.append(b);
  });
}

// Switching a platform on or off, from Settings or the title bar
async function setPlatformOn(id, on) {
  const p = PLATFORM_LIST.find((x) => x.id === id);
  prefs[p.pref] = on;
  savePrefs();
  $(p.pref).checked = on;   // the Settings switch has the same id as the pref
  if (id === "local") {
    await localLoad();
    if (on) localRescan();
  }
  if (id === "spotify") syncSpotifySettings();
  renderPlatChips();   // reloads the page for the new mix
  reqPaint();
}

// ---------- merged lists: Likes and Search across platforms ----------
// A feed hands out one platform's tracks a page at a time: next() resolves to a list, or null at
// the end. Each page of the view takes the next page from every feed and interleaves them.
function arrayFeed(load) {
  let list = null;
  let pos = 0;
  return {
    next: async () => {
      if (!list) list = await load();
      if (pos >= list.length) return null;
      const page = list.slice(pos, pos + 50);
      pos += page.length;
      return page;
    },
  };
}

function scFeed(path, onPage) {
  let next = path;
  return {
    next: async () => {
      if (!next) return null;
      const data = await api(next);
      next = !Array.isArray(data) && data.next_href ? toPath(data.next_href) : "";
      const list = asList(data);
      list.forEach((t) => { if (typeof t.user_favorite === "boolean") likeState.set(t.id, t.user_favorite); });
      if (onPage) onPage(list);
      return list;
    },
  };
}

const markLiked = (list) => list.forEach((t) => likeState.set(t.id, true));

function likesFeed(id) {
  if (id === "soundcloud") return scFeed(LIKES_PATH, markLiked);
  if (id === "spotify") return arrayFeed(() => spotifyLikedTracks(500));
  return arrayFeed(async () => localTracks.slice());
}

function searchFeed(id, q) {
  if (id === "soundcloud") return scFeed("/tracks?q=" + encodeURIComponent(q) + "&access=playable&limit=30&linked_partitioning=true");
  if (id === "spotify") return arrayFeed(() => spotifySearchTracks(q));
  return arrayFeed(async () => localSearch(q));
}

let merged = null;   // { feeds, filter, tok, seen, errors } for the list on screen

async function pullMerged() {
  const m = merged;
  const pages = await Promise.all(m.feeds.map(async (f) => {
    if (f.done) return [];
    try {
      const page = await f.next();
      if (page === null) { f.done = true; return []; }
      return m.filter(page);
    } catch (e) {
      f.done = true;   // e.g. signed out of one platform: show the others
      m.errors.push(e && e.message ? e.message : String(e));
      return [];
    }
  }));
  return interleave(pages).filter((t) => !m.seen.has(t.id) && m.seen.add(t.id));
}

async function loadMerged(feeds, filter) {
  const tok = ++loadToken;
  merged = { feeds, filter: filter || ((l) => l), tok, seen: new Set(), errors: [] };
  say("Loading...");
  const got = await pullMerged();
  if (tok !== loadToken) return;
  viewTracks = got;
  nextHref = feeds.some((f) => !f.done) ? "merged:" : "";
  render();
  say(viewTracks.length ? "" : merged.errors[0] || "Nothing found.");
  setTimeout(nearBottomCheck, 50);
}

async function pageMerged() {
  const m = merged;
  if (!m || m.tok !== loadToken) { nextHref = ""; return; }
  const shown = viewTracks.length;
  const got = await pullMerged();
  if (m.tok !== loadToken) return;
  viewTracks = viewTracks.concat(got);
  nextHref = m.feeds.some((f) => !f.done) ? "merged:" : "";
  renderAppend(shown);
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
  const shown = viewTracks.length;
  localPos += more.length;
  viewTracks = viewTracks.concat(more);
  nextHref = localPos < localPool.length ? "local:" : "";
  renderAppend(shown);
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
    $("localNote").textContent = "Off. Your folders are kept.";
  } else if (!localFolderList.length) {
    $("localNote").textContent = "Add a folder to get started.";
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
  if (activePlats().includes("local")) refreshSidebar();   // albums come from your files
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

// ---------- sidebar: playlists from every active platform ----------
// SoundCloud: your playlists (and ones you liked). Spotify: your playlists. Local files: your albums.
let sidebarKey = null;   // which platforms the sidebar was last built for
let sidebarTok = 0;

function localAlbums() {
  const albums = new Map();
  localTracks.forEach((t) => {
    const a = (t.album || "").trim();
    if (a && !albums.has(a)) albums.set(a, { source: "local", group: "albums", id: "album:" + a, album: a, title: a, user: { username: artistName(t) } });
  });
  return [...albums.values()].sort((x, y) => x.title.localeCompare(y.title));
}

async function sidebarItems(id) {
  if (id === "soundcloud") return (await invoke("signed_in")) ? scPlaylists() : [];
  if (id === "spotify") {
    try {
      return await spotifyPlaylists();
    } catch (e) {
      throw new Error(/40[13]|scope/i.test(String(e))
        ? "Sign out of Spotify and back in (Settings > Integrations) to let the app read your playlists."
        : "Could not load your Spotify playlists.");
    }
  }
  return localAlbums();
}

async function refreshSidebar() {
  const tok = ++sidebarTok;
  const plats = activePlats();
  $("sideLists").hidden = !plats.length && !xpls.length;
  $("plHeadText").textContent = "Playlists";
  $("plNew").hidden = false;
  if (!playlists.length) renderPlaylists("Loading...");
  const notes = [];
  const parts = await Promise.all(plats.map((id) => sidebarItems(id).catch((e) => { notes.push(e.message || String(e)); return []; })));
  if (tok !== sidebarTok) return;   // switched platforms meanwhile
  // playlists on this PC first; platform playlists you switched to one of those stay hidden
  const hidden = new Set((prefs.hiddenPlaylists || []).map(String));
  playlists = xplSidebarItems().concat(parts.flat().filter((p) => !hidden.has(String(p.id))));
  renderPlaylists(notes.join(" "));
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
      ? "Couldn't open this playlist. Check Spotify playback is signed in (Settings > Accounts > Spotify options)."
      : String(e));
  }
}

function platformsInit() {
  // Platform icons in the title bar switch platforms on and off, like the switches in Settings
  $("titlePlat").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-p]");
    if (b) setPlatformOn(b.dataset.p, !b.classList.contains("on"));
  });
  // Each platform can be switched off without signing out
  PLATFORM_LIST.forEach((p) => {
    $(p.pref).checked = platformOn(p);
    $(p.pref).addEventListener("change", () => setPlatformOn(p.id, $(p.pref).checked));
  });

  // Local music settings
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
  renderPlatChips();
}
