// Mixes, your playlists, the Likes filter, Rediscover, and the track right-click menu.

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

function libraryInit() {
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
