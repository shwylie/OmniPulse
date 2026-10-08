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
  rediscover: ["all", "tracks", "albums", "mixes"],
  artist: ["all", "tracks", "albums"],
  artists: ["all", "soundcloud", "spotify", "youtube"],
};
const FILTER_NAMES = { all: "All", tracks: "Tracks", artists: "Artists", albums: "Albums", playlists: "Playlists", mixes: "Sets & Podcasts", soundcloud: "SoundCloud", spotify: "Spotify", youtube: "YouTube Music" };

function renderListChips() {
  const box = $("listChips");
  box.replaceChildren();
  let list = LIST_FILTERS[view];
  if (view === "artists") list = list.filter((k) => k === "all" || activePlats().includes(k));   // platforms that are on
  box.hidden = !list || (view === "search" && !query) || (view === "artists" && list.length < 3);
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
    // sets & podcasts too, like Library (the Sets & Podcasts filter picks them out)
    let pool = source.filter((t) =>
      recOk(t) && (lastPlayed.get(t.id) || 0) < monthAgo);
    shuffleInPlace(pool);
    pool.sort((a, b) => (lastPlayed.get(a.id) || 0) - (lastPlayed.get(b.id) || 0));   // never played first
    pool = alternatePlatforms(pool);   // an even split, one platform then the next (js/shuffle.js)
    pool = applyListFilter(pool);   // All, Tracks or Sets & Podcasts
    setLocalPool(pool);
    render();
    say(viewTracks.length ? "" : !source.length ? "No likes found."
      : listFilter === "mixes" ? "No sets or podcasts in your likes to rediscover." : "You've played all your likes recently.");
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    render();
    say(String(e));
  }
}

// ---------- "Don't recommend this track / this artist" ----------
// Tracks (by id) and artists (by name) left out of everything the app picks for you: Discover's mixes
// and stations, Rediscover, autoplay and radio, and the sets & podcasts from artists you follow.
// Searching still finds them. Settings > Library > Hidden & not recommended undoes it.
let hiddenRecs = new Set();
let hiddenArtists = new Set();
let hiddenAlbums = new Set();

// An album, the same on every platform: its first artist and its title, without "(Deluxe)" and the like
const albumKeyOf = (title, artist) => {
  const t = String(title || "").toLowerCase().replace(/\s*[([][^)\]]*(deluxe|edition|remaster(ed)?|expanded|bonus)[^)\]]*[)\]]/g, "").replace(/\s+/g, " ").trim();
  const a = artistKey(String(artist || "").split(",")[0]);
  return t ? a + "|" + t : "";
};
const trackAlbumName = (t) => (t && (t.album || (t.spAlbum && t.spAlbum.name))) || "";
const trackAlbumKey = (t) => albumKeyOf(trackAlbumName(t), t && t.spAlbum && t.spAlbum.artists && t.spAlbum.artists[0] ? t.spAlbum.artists[0].name : recArtistName(t));
const albumHidden = (a) => hidingOn() && hiddenAlbums.size > 0 && hiddenAlbums.has(albumKeyOf(a.title, a.artist));

// The artist a track is "by", as the menu names it: the one in "Artist - Title", or who's credited
const recArtistName = (t) => (guessNames(t).artist || artistName(t) || "").trim();

function hiddenArtistOf(t) {
  if (!hiddenArtists.size || !t) return false;
  const names = [guessNames(t).artist, ...String(artistName(t)).split(/,|&/), t.user && t.user.username];
  return names.some((n) => n && hiddenArtists.has(artistKey(n)));
}

// Off (Settings > Library > Hidden & not recommended), nothing is left out; the lists are kept
const hidingOn = () => prefs.hideOn !== false;
function hidingTurnOn() {
  if (hidingOn()) return;
  prefs.hideOn = true;
  savePrefs();
  paintHiddenSummary();
  if (typeof refreshSidebar === "function") refreshSidebar();
}

// Whether the app may recommend this track
const recOk = (t) => !safeOk(t) ? false : !hidingOn() ? !!t : !!t && !hiddenRecs.has(String(t.id)) && !hiddenArtistOf(t) && !(hiddenAlbums.size && hiddenAlbums.has(trackAlbumKey(t)));

// on pages of recommendations, it goes straight away
const onRecPage = () => view === "discover" || view === "rediscover" || (view === "playlist" && currentPlaylist && currentPlaylist.source === "mix");

function hideRecommendation(t) {
  hidingTurnOn();
  hiddenRecs.add(String(t.id));
  prefs.hiddenRecs = [...hiddenRecs].slice(-3000);
  // its name, for the list in Settings
  const names = Object.assign({}, prefs.hiddenRecNames || {});
  names[String(t.id)] = (t.title || "Untitled") + (artistName(t) ? " \u2014 " + artistName(t) : "");
  const keep = new Set(prefs.hiddenRecs);
  Object.keys(names).forEach((k) => { if (!keep.has(k)) delete names[k]; });
  prefs.hiddenRecNames = names;
  savePrefs();
  if (onRecPage()) { viewTracks = viewTracks.filter((x) => x.id !== t.id); render(); }
  dropFromAutoplay();
  paintNotRecommended();
  say("It won't be recommended again.");
}

// An album (from its tile, or one of its tracks): its tracks and its tile are left out of recommendations
function hideAlbum(title, artist) {
  const key = albumKeyOf(title, artist);
  if (!key) return;
  hidingTurnOn();
  hiddenAlbums.add(key);
  prefs.hiddenAlbums = Object.assign({}, prefs.hiddenAlbums || {}, { [key]: title + (artist ? " \u2014 " + artist : "") });
  savePrefs();
  if (onRecPage()) { viewTracks = viewTracks.filter(recOk); viewAlbums = viewAlbums.filter((a) => !albumHidden(a)); render(); }
  dropFromAutoplay();
  paintNotRecommended();
  say('"' + title + '" won\'t be recommended again.');
}

function hideArtist(t) {
  const name = recArtistName(t);
  const key = artistKey(name);
  if (!key) return;
  hidingTurnOn();
  hiddenArtists.add(key);
  prefs.hiddenArtists = Object.assign({}, prefs.hiddenArtists || {}, { [key]: name });
  savePrefs();
  if (onRecPage()) { viewTracks = viewTracks.filter((x) => !hiddenArtistOf(x)); render(); }
  dropFromAutoplay();
  paintNotRecommended();
  say(name + " won't be recommended again.");
}

// autoplay's picks that are now hidden leave the queue
function dropFromAutoplay() {
  const before = queue.length;
  queue = queue.filter((x, i) => i <= qi || !x.autoplay || recOk(x));
  if (queue.length !== before) renderQueue();
}

// Settings > Library > Hidden & not recommended: a line saying what's there, and a folded list for
// each kind (artists, tracks, playlists), shown only when it has something in it
function paintHiddenSummary() {
  $("hideOn").checked = hidingOn();
  $("hiddenBody").hidden = !hidingOn();
  const a = Object.keys(prefs.hiddenArtists || {}).length;
  const al = Object.keys(prefs.hiddenAlbums || {}).length;
  const n = hiddenRecs.size;
  const p = (prefs.hiddenByYou || []).length;
  const plural = (k, one, many) => k + " " + (k === 1 ? one : many);
  const rec = [a && plural(a, "artist", "artists"), al && plural(al, "album", "albums"), n && plural(n, "track", "tracks")].filter(Boolean);
  const parts = [];
  if (rec.length) parts.push(andList(rec) + (a + al + n === 1 ? " isn't" : " aren't") + " recommended");
  if (p) parts.push(plural(p, "playlist is", "playlists are") + " hidden from the sidebar");
  if (!hidingOn()) { $("hiddenNote").textContent = "Off: nothing is hidden or left out. Your lists are kept."; return; }
  $("hiddenNote").textContent = parts.length
    ? parts.join("; ") + "."
    : "Nothing yet. Right-click a track or album to stop it (or its artist) being recommended, or a playlist in the sidebar to hide it.";
}

function hiddenRow(name, btn, title, data) {
  const row = el("div", "folderRow");
  const b = el("button", btn === "\u00d7" ? "" : "soft", btn);
  b.title = title;
  b.setAttribute("aria-label", title);
  Object.assign(b.dataset, data);
  row.append(el("span", "", name), b);
  return row;
}

function paintNotRecommended() {
  const names = prefs.hiddenArtists || {};
  const a = Object.entries(names).sort((x, y) => x[1].localeCompare(y[1]));
  $("notRecArtists").replaceChildren(...a.map(([key, name]) => hiddenRow(name, "\u00d7", "Recommend " + name + " again", { key })));
  $("hidArtistsSum").textContent = "Artists Not Recommended (" + a.length + ")";
  $("hidArtistsBox").hidden = !a.length;
  const al = Object.entries(prefs.hiddenAlbums || {}).sort((x, y) => x[1].localeCompare(y[1]));
  $("notRecAlbums").replaceChildren(...al.map(([key, name]) => hiddenRow(name, "\u00d7", "Recommend it again", { album: key })));
  $("hidAlbumsSum").textContent = "Albums Not Recommended (" + al.length + ")";
  $("hidAlbumsBox").hidden = !al.length;
  const tn = prefs.hiddenRecNames || {};
  const ids = [...hiddenRecs];
  const named = ids.filter((id) => tn[id]).sort((x, y) => tn[x].localeCompare(tn[y]));
  const unnamed = ids.length - named.length;
  const rows = named.map((id) => hiddenRow(tn[id], "\u00d7", "Recommend it again", { track: id }));
  if (unnamed) rows.push(el("p", "note", unnamed + (unnamed === 1 ? " track" : " tracks") + " from before names were kept here."));
  $("notRecTrackList").replaceChildren(...rows);
  $("hidTracksSum").textContent = "Tracks Not Recommended (" + ids.length + ")";
  $("hidTracksBox").hidden = !ids.length;
  const nv = Object.entries(prefs.noVersion || {}).sort((x, y) => x[1].localeCompare(y[1]));
  $("noVerList").replaceChildren(...nv.map(([id, name]) => hiddenRow(name, "\u00d7", "Show it again", { track: id })));
  $("noVerSum").textContent = "Hidden: No Full Version (" + nv.length + ")";
  $("noVerBox").hidden = !nv.length;
  paintHiddenSummary();
}

// ---------- right-click menu on tracks ----------
let ctxTrack = null;
let ctxAnchor = null;

// fromPlayer: opened from the player bar, so leave out what only makes sense on a page
// queueIdx: opened on a queue row (its place in the queue)
let ctxQueueIdx = null;
function openTrackMenu(t, x, y, fromPlayer, queueIdx) {
  ctxTrack = t;
  ctxQueueIdx = queueIdx === undefined ? null : queueIdx;
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
  if (ctxQueueIdx !== null && ctxQueueIdx > qi) add("Remove from queue", "unqueue");
  sep();
  if (canLike(t)) add(isLiked(t) ? "Remove from Likes" : "Add to Likes", "like");
  if (favOn()) add(isFav(t) ? "Remove from Favourites" : "Add to Favourites", "fav");
  const names = artistNames(t);
  if (names.length > 1) names.forEach((n, k) => add("Go to " + n, "artist:" + k));
  else add("Go to artist", "artist");
  if (!fromPlayer && view === "playlist" && isOwnPlaylist(currentPlaylist)) add("Remove from this playlist", "unlist");
  sep();
  add("Don't recommend this track", "hide");
  if (trackAlbumName(t)) add("Don't recommend this album", "hideAlbum");
  if (recArtistName(t)) add("Don't recommend " + recArtistName(t), "hideArtist");
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
  if (a.startsWith("artist:")) { openArtistNamed(t, artistNames(t)[Number(a.slice(7))]); return; }
  switch (a) {
    case "play":
      if (ctxQueueIdx !== null) { qi = ctxQueueIdx; playCurrent(); }   // a queue row: play it there
      else if (i >= 0) playFromPage(i); else startList([t], 0, false);
      break;
    case "unqueue": if (ctxQueueIdx > qi && queue[ctxQueueIdx] === t) { queue.splice(ctxQueueIdx, 1); renderQueue(); } break;
    case "next": playNext(t); break;
    case "queue": addToQueue(t); break;
    case "playlist": openPlaylistMenu(t, ctxAnchor); break;
    case "like": toggleLike(t); break;
    case "fav": toggleFav(t); break;
    case "artist": openArtistFor(t); break;
    case "unlist": removeFromPlaylist(t); break;
    case "hide": hideRecommendation(t); break;
    case "hideArtist": hideArtist(t); break;
    case "hideAlbum": hideAlbum(trackAlbumName(t), t.spAlbum && t.spAlbum.artists && t.spAlbum.artists[0] ? t.spAlbum.artists[0].name : recArtistName(t)); break;
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
    prefs.pageFilters = Object.assign({}, prefs.pageFilters, { [view]: b.dataset.f });
    savePrefs();
    restoring = true;
    try { restorePage(Object.assign({}, curPage, { listFilter: b.dataset.f })); } finally { restoring = false; }
  });

  // Rediscover
  $("navRediscover").addEventListener("click", () => openRediscover());

  // removed recommendations
  hiddenRecs = new Set((prefs.hiddenRecs || []).map(String));
  hiddenArtists = new Set(Object.keys(prefs.hiddenArtists || {}));
  hiddenAlbums = new Set(Object.keys(prefs.hiddenAlbums || {}));
  paintNotRecommended();
  $("hideOn").addEventListener("change", () => {
    prefs.hideOn = $("hideOn").checked;
    savePrefs();
    paintHiddenSummary();
    refreshSidebar();
    say(prefs.hideOn ? "Hidden and not recommended things are left out again." : "Nothing is hidden or left out now. Your lists are kept.");
  });
  $("notRecAlbums").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-album]");
    if (!b) return;
    const names = Object.assign({}, prefs.hiddenAlbums || {});
    const name = names[b.dataset.album];
    delete names[b.dataset.album];
    prefs.hiddenAlbums = names;
    hiddenAlbums = new Set(Object.keys(names));
    savePrefs();
    paintNotRecommended();
    say('"' + String(name || "That album").split(" \u2014 ")[0] + '" can be recommended again.');
  });
  $("notRecArtists").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-key]");
    if (!b) return;
    const names = Object.assign({}, prefs.hiddenArtists || {});
    const name = names[b.dataset.key];
    delete names[b.dataset.key];
    prefs.hiddenArtists = names;
    hiddenArtists = new Set(Object.keys(names));
    savePrefs();
    paintNotRecommended();
    say(name + " can be recommended again.");
  });
  $("notRecTrackList").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-track]");
    if (!b) return;
    hiddenRecs.delete(b.dataset.track);
    prefs.hiddenRecs = [...hiddenRecs];
    const names = Object.assign({}, prefs.hiddenRecNames || {});
    const name = names[b.dataset.track];
    delete names[b.dataset.track];
    prefs.hiddenRecNames = names;
    savePrefs();
    paintNotRecommended();
    say((name ? '"' + name.split(" \u2014 ")[0] + '"' : "That track") + " can be recommended again.");
  });
  $("noVerList").addEventListener("click", (e) => {
    const b = e.target.closest("button[data-track]");
    if (!b) return;
    const all = Object.assign({}, prefs.noVersion || {});
    const name = all[b.dataset.track];
    delete all[b.dataset.track];
    prefs.noVersion = all;
    savePrefs();
    paintNotRecommended();
    render();
    say((name ? '"' + name.split(" \u2014 ")[0] + '"' : "That track") + " is shown again.");
  });
  $("notRecTracks").addEventListener("click", () => {
    hiddenRecs = new Set();
    prefs.hiddenRecs = [];
    prefs.hiddenRecNames = {};
    savePrefs();
    paintNotRecommended();
    say("Those tracks can be recommended again.");
  });

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
