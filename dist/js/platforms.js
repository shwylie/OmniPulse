// Platforms (SoundCloud, Spotify, local files), local music, and the sidebar for each.

const scOn = () => prefs.scOn !== false;

function scApply() {
  $("navDiscover").hidden = !platformsFor("discover").length;
  renderPlatChips();
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

function platformsInit() {
  // Platform icons in the title bar: one choice for the whole app
  $("titlePlat").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-p]");
    if (!b || b.classList.contains("on")) return;
    prefs.globalPlat = b.dataset.p;
    savePrefs();
    renderPlatChips();
    reqPaint();
    if (curPage && curPage.view !== "now") { restoring = true; try { restorePage(curPage); } finally { restoring = false; } }
  });

  // SoundCloud and Spotify can be switched off without signing out
  $("scOn").checked = scOn();
  $("scOn").addEventListener("change", async () => {
    prefs.scOn = $("scOn").checked;
    savePrefs();
    scApply();
    refreshSidebar();
    if (curPage && curPage.view !== "now") { restoring = true; try { restorePage(curPage); } finally { restoring = false; } }
    else openLikes();
  });
  $("spOn").checked = prefs.spOn !== false;
  $("spOn").addEventListener("change", () => {
    prefs.spOn = $("spOn").checked;
    savePrefs();
    if (typeof spotifyCheck === "function") spotifyCheck();
    renderPlatChips();
  });

  // Local music settings
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
  renderPlatChips();
}
