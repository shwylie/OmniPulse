// Mixes, your playlists, the Likes filter, Rediscover, and the track right-click menu.

// ---------- sets and podcasts ("mixes" in the code): anything longer than 15 minutes ----------
const MIX_MS = 15 * 60 * 1000;
const isMix = (t) => !!t && (t.duration || 0) > MIX_MS;
// They stay out of these; they still show in Likes (under Sets & podcasts) and playlists
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
// Yours to change: playlists on this PC, and your own on SoundCloud
const isOwnPlaylist = (p) => isXpl(p) || (!!p && !!p.user && meId !== null && String(p.user.id) === String(meId));

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
  if (isXpl(p)) {
    const n = xplAdd(p.id, [t]);
    say(n ? "Added to " + p.title + "." : "Already in " + p.title + ".");
    if (view === "playlist" && currentPlaylist && currentPlaylist.id === p.id) openPlaylist(currentPlaylist);
    refreshSidebar();
    return;
  }
  if (srcOf(t) !== "soundcloud") { say("SoundCloud playlists only take SoundCloud tracks. Use a playlist on this PC for anything else."); return; }
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
  if (isXpl(p)) {
    xplRemove(p.id, t.id);
    viewTracks = viewTracks.filter((x) => x.id !== t.id);
    render();
    refreshSidebar();
    say("Removed from " + p.title + ".");
    return;
  }
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
  if (isXpl(p)) { deleteXpl(p); return; }
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
  // playlists on this PC take anything; SoundCloud ones only SoundCloud tracks
  const here = playlists.filter(isXpl);
  const sc = srcOf(t) === "soundcloud" ? playlists.filter((p) => !isXpl(p) && isOwnPlaylist(p)) : [];
  const group = (label, list) => {
    if (!list.length) return;
    if (here.length && sc.length) m.append(el("div", "menuNote menuGroup", label));
    list.forEach((p) => {
      const b = el("button", "menuItem", p.title || "Untitled");
      b.dataset.id = p.id;
      m.append(b);
    });
  };
  group("On this PC", here);
  group("On SoundCloud", sc);
  if (!here.length && !sc.length) m.append(el("div", "menuNote", "You have no playlists yet."));
  const nb = el("button", "menuItem menuNew", "New playlist...");
  nb.dataset.new = "1";
  m.append(nb);
  m.hidden = false;
  placeMenu(anchor);
  m.dataset.anchor = "";
  m._anchor = anchor;
}

// ---------- list filters: All / Tracks / Albums / Sets & Podcasts ----------
// All mixes album tiles in among the tracks; Albums shows only albums; Tracks and Sets & Podcasts
// leave albums out. Rediscover and artist pages have no sets & podcasts of their own to filter.
let listFilter = "all";
const LIST_FILTERS = {
  likes: ["all", "tracks", "albums", "mixes"],
  search: ["all", "tracks", "artists", "albums", "playlists", "mixes"],
  rediscover: ["all", "tracks", "albums"],
  artist: ["all", "tracks", "albums"],
};
const FILTER_NAMES = { all: "All", tracks: "Tracks", artists: "Artists", albums: "Albums", playlists: "Playlists", mixes: "Sets & Podcasts" };

function renderListChips() {
  const box = $("listChips");
  box.replaceChildren();
  const list = LIST_FILTERS[view];
  box.hidden = !list || (view === "search" && !query);
  if (box.hidden) return;
  if (!list.includes(listFilter)) listFilter = "all";
  list.forEach((k) => {
    const b = el("button", "chip" + (k === listFilter ? " on" : ""), FILTER_NAMES[k]);
    b.dataset.f = k;
    box.append(b);
  });
}

function applyListFilter(list) {
  if (listFilter === "mixes") return list.filter(isMix);
  if (listFilter === "tracks") return list.filter((t) => !isMix(t));
  if (listFilter === "albums" || listFilter === "artists" || listFilter === "playlists") return [];
  return list;
}

// With Albums (or on Search, Artists or Playlists) chosen there are no tracks to load: the page
// shows only those
function albumsOnly() {
  if (!["albums", "artists", "playlists"].includes(listFilter)) return false;
  ++loadToken;
  viewTracks = [];
  nextHref = "";
  localPool = null;
  render();
  say("Loading " + FILTER_NAMES[listFilter].toLowerCase() + "...");
  if (listFilter !== "albums") setTimeout(() => say(""), 1500);
  return true;
}

// ---------- everything you've liked (for Discover and Rediscover) ----------
let likedTracks = [];
let likedIds = new Set();
let likesLoadedAt = 0;
let likesLoading = null;

// Your SoundCloud likes are kept on this PC (js/store.js), so pages that use them open at once;
// they're refreshed in the background when they're more than ten minutes old.
let likesCacheRead = null;

function useLikes(all) {
  likedTracks = all;
  likedIds = new Set(all.map((t) => t.id));
  all.forEach((t) => likeState.set(t.id, true));
}

function likesFromCache() {
  likesCacheRead ??= cachedTracks("likes:soundcloud").then((c) => {
    if (c && !likedTracks.length) { useLikes(c.tracks); likesLoadedAt = c.at; }
  });
  return likesCacheRead;
}

function likesFetch() {
  if (likesLoading) return likesLoading;
  likesLoading = (async () => {
    try {
      const all = await collectPages("/me/likes/tracks?access=playable&limit=200&linked_partitioning=true", 25);
      useLikes(all);
      likesLoadedAt = Date.now();
      cacheTracks("likes:soundcloud", all);
    } catch (e) {
      // offline or signed out: Discover still works, it just can't skip your likes
    } finally {
      likesLoading = null;
    }
  })();
  return likesLoading;
}

// force: wait for a fresh copy. Otherwise this returns as soon as there's any copy, cached or not.
async function ensureLikes(force) {
  if (prefs.scOn === false) return;
  await likesFromCache();
  const stale = !likesLoadedAt || Date.now() - likesLoadedAt > 10 * 60000;
  if (force || (stale && !likedTracks.length)) return likesFetch();
  if (stale) likesFetch();   // in the background
}

function isLikedTrack(t) {
  const s = likeState.get(t.id);
  return s === true || (s !== false && likedIds.has(t.id)) || (t.dupes || []).some((d) => likeState.get(d.id) === true || likedIds.has(d.id));
}

// Rediscover: your own likes you haven't played in the last month
async function openRediscover(fresh) {
  currentPlaylist = null;
  currentArtist = null;
  showView("rediscover");
  if (albumsOnly()) return;
  if (!fresh && useCachedPage()) return;
  $("listView").scrollTop = 0;
  const tok = ++loadToken;
  say("Looking through your likes...");
  try {
    const plats = activePlats("rediscover");
    if (!plats.length) { showNothing(); return; }
    // your likes from every active platform (local files count as liked)
    const parts = await Promise.all(plats.map(async (id) => {
      if (id === "spotify") return spotifyLikedTracks(500).catch(() => []);
      if (id === "local") return localTracks;
      if (id === "youtube") return ytLikedTracks();
      await ensureLikes();
      return likedTracks.filter(isLikedTrack);
    }));
    const source = parts.flat();
    if (tok !== loadToken) return;
    const lastPlayed = new Map();
    loadStats().forEach((x) => lastPlayed.set(x.id, Math.max(lastPlayed.get(x.id) || 0, x.ts)));
    const monthAgo = Date.now() - 30 * 86400000;
    let pool = source.filter((t) =>
      !isMix(t) && !hiddenRecs.has(String(t.id)) && (lastPlayed.get(t.id) || 0) < monthAgo);
    shuffleInPlace(pool);
    pool.sort((a, b) => (lastPlayed.get(a.id) || 0) - (lastPlayed.get(b.id) || 0));   // never played first
    pool = alternatePlatforms(pool);   // an even split, one platform then the next (js/shuffle.js)
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

// fromPlayer: opened from the player bar, so leave out what only makes sense on a page
function openTrackMenu(t, x, y, fromPlayer) {
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
  if (!fromPlayer) add("Play", "play");   // it is already playing
  add("Play next", "next");
  add("Add to queue", "queue");
  add("Add to playlist...", "playlist");
  sep();
  if (canLike(t)) add(isLiked(t) ? "Remove from Likes" : "Add to Likes", "like");
  if (favOn()) add(isFav(t) ? "Remove from Favourites" : "Add to Favourites", "fav");
  add("Go to artist", "artist");
  if (!fromPlayer && view === "playlist" && isOwnPlaylist(currentPlaylist)) add("Remove from this playlist", "unlist");
  if (!fromPlayer && (view === "discover" || view === "rediscover")) add("Remove recommendation", "hide");
  if (t.permalink_url) {
    sep();
    add(openLabel(t), "open");
    add("Copy link", "copy");
  }
  sep();
  add("Track preferences...", "fix");
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
    case "fav": toggleFav(t); break;
    case "artist": openArtistFor(t); break;
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

function libraryInit() {
  $("plMenu").addEventListener("click", (e) => {
    const b = e.target.closest(".menuItem");
    if (!b) return;
    if (b.dataset.action) { runMenuAction(b.dataset.action); return; }
    if (b.dataset.paction) { runPlaylistAction(b.dataset.paction); return; }
    if (b.dataset.new) { const t = menuTrack; closeMenu(); openNewPlaylistDialog(t); return; }
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

  // Sidebar "+": the New playlist dialog (on this PC, or on SoundCloud)
  $("plNew").addEventListener("click", () => openNewPlaylistDialog(null));
  $("plDelete").addEventListener("click", deletePlaylist);

  // All / Tracks / Albums / Sets & Podcasts: the same page again, without a step in the Back history
  $("listChips").addEventListener("click", (e) => {
    const b = e.target.closest(".chip");
    if (!b || !curPage || b.dataset.f === listFilter) return;
    restoring = true;
    try { restorePage(Object.assign({}, curPage, { listFilter: b.dataset.f })); } finally { restoring = false; }
  });

  // Rediscover
  $("navRediscover").addEventListener("click", () => openRediscover());

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
