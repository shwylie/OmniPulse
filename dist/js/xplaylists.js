// Playlists on this PC: they can hold tracks from any platform and are never sent anywhere.
// Each keeps a small snapshot of its tracks, enough to show and play them without asking the
// platform. Local files are kept by id and looked up again, since their addresses change per run.

const XPL_KEY = "sc.xplaylists";
const XPL_FIELDS = ["source", "id", "ytId", "ytArtistId", "spArtists", "uri", "kind", "title", "displayArtist", "duration", "artwork_url", "permalink_url",
  "album", "spAlbum", "genre", "created_at", "playback_count", "sharing", "access", "policy", "license",
  // (the catalogues: Deezer, iTunes and MusicBrainz songs)
  "isrc", "dzId", "dzArtistId", "dzAlbumId", "dzRank", "itId", "mbId", "release_date"];

let xpls = [];
try { xpls = JSON.parse(localStorage.getItem(XPL_KEY) || "[]"); } catch (e) { xpls = []; }

// (quiet: without keeping the platforms in step again, js/xplmirror.js)
function xplSave(quiet) {
  try { localStorage.setItem(XPL_KEY, JSON.stringify(xpls)); } catch (e) { say("Could not save your playlists on this PC: " + e); }
  if (!quiet && typeof xmSoon === "function") xmSoon();
}

const isXpl = (p) => !!p && p.source === "xpl";
const xplFind = (id) => xpls.find((p) => p.id === id);

function xplSnap(t) {
  const s = {};
  XPL_FIELDS.forEach((k) => { if (t[k] !== undefined && t[k] !== null) s[k] = t[k]; });
  if (s.source === "soundcloud") delete s.source;   // the default
  if (t.user) s.user = { id: t.user.id, username: t.user.username, avatar_url: t.user.avatar_url };
  if (srcOf(t) === "local") { delete s.artwork_url; delete s.url; }   // rebuilt from the file's id
  return s;
}

// The tracks to show and play: fresh copies, with local files matched to this run's addresses
function xplTracks(p) {
  return (p.tracks || []).map((s) => {
    if (srcOf(s) === "local") {
      const live = localTracks.find((x) => x.id === s.id);
      return live ? Object.assign({}, live) : Object.assign({}, s, { missing: true });
    }
    return Object.assign({}, s, { user: Object.assign({}, s.user) });
  });
}

// What the sidebar shows (the "user" is you, so its track rows get remove buttons)
function xplSidebarItems() {
  return xpls.map((p) => ({ source: "xpl", id: p.id, title: p.title, user: { username: p.collab ? "Collaborative" : "This PC" }, count: p.tracks.length }));
}

function xplCreate(title, tracks, from) {
  const p = {
    id: "xpl:" + Date.now().toString(36) + Math.random().toString(36).slice(2, 7),
    title: title || "Untitled playlist",
    tracks: (tracks || []).map(xplSnap),
    created: Date.now(),
  };
  if (from) p.from = from;   // the platform playlist this replaced in the sidebar
  xpls.unshift(p);
  xplSave();
  return p;
}

function xplAdd(id, list) {
  const p = xplFind(id);
  if (!p) return 0;
  const have = new Set(p.tracks.map((s) => s.id));
  const fresh = list.filter((t) => t && !have.has(t.id) && have.add(t.id));
  p.tracks.push(...fresh.map(xplSnap));
  xplSave();
  if (typeof collabAdded === "function") collabAdded(p, fresh);   // (collaborative: to everyone in it)
  return fresh.length;
}

function xplRemove(id, trackId) {
  const p = xplFind(id);
  if (!p) return;
  p.tracks = p.tracks.filter((s) => s.id !== trackId);
  xplSave();
  if (typeof collabRemoved === "function") collabRemoved(p, trackId);
}

function xplRename(id, title) {
  const p = xplFind(id);
  if (!p || !title) return;
  p.title = title;
  xplSave();
}

// Deleting a switched copy brings the original back into the sidebar
function xplDelete(id) {
  const p = xplFind(id);
  if (!p) return;
  xpls = xpls.filter((x) => x !== p);
  xplSave(true);
  // the hidden copies made for it on the platforms go too (one of yours it was made from stays)
  Object.keys(p.made || {}).forEach((plat) => {
    const id = p.mirrors && p.mirrors[plat];
    if (!id) return;
    xmRemove(plat, id);
    prefs.hiddenPlaylists = (prefs.hiddenPlaylists || []).filter((x) => x !== xmSideId(plat, id));
  });
  savePrefs();
  if (p.from) {
    prefs.hiddenPlaylists = (prefs.hiddenPlaylists || []).filter((x) => x !== p.from);
    savePrefs();
  }
}

// Everything in a platform playlist (or one of your local albums)
async function platformPlaylistTracks(p) {
  if (p.source === "local") return localTracks.filter((t) => (t.album || "").trim() === p.album);
  if (p.source === "spotify") return p.spAlbumId ? spotifyAlbumTracks(p.spAlbumId) : spotifyPlaylistTracks(p.spId);
  return collectPages("/playlists/" + p.id + "/tracks?access=playable&limit=200&linked_partitioning=true", 10);
}

// "Switch to a playlist on this PC": copy it here, and hide the original from the sidebar
// (returns the copy as the sidebar has it, or null if it couldn't be made)
async function xplSwitch(p) {
  say('Copying "' + (p.title || "playlist") + '"...');
  try {
    const tracks = typeof playlistAllTracks === "function" ? await playlistAllTracks(p) : await platformPlaylistTracks(p);
    const copy = xplCreate(p.title, tracks, String(p.id));
    prefs.hiddenPlaylists = [...new Set((prefs.hiddenPlaylists || []).concat(String(p.id)))];
    savePrefs();
    await refreshSidebar();
    const shown = playlists.find((x) => x.id === copy.id);
    openPlaylist(shown);
    say('"' + copy.title + '" is now a playlist on this PC (' + copy.tracks.length + " tracks)." + (p.source === "mix" ? "" : " The original is unchanged on " +
      (p.source === "local" ? "your drive" : platInfo(p.source || "soundcloud").name) + "."));
    return shown || null;
  } catch (e) {
    say("Could not copy the playlist: " + e);
    return null;
  }
}

// ---------- the New playlist / Rename dialog ----------
let renameId = null;

// A new playlist: made straight away, here, taking songs from any platform (and kept on SoundCloud
// and Spotify by itself, js/xplmirror.js). Right-click it to rename it.
async function openNewPlaylistDialog(track) {
  const taken = new Set(xpls.map((p) => p.title));
  let name = "New Playlist";
  for (let n = 2; taken.has(name); n++) name = "New Playlist " + n;
  const p = xplCreate(name, track ? [track] : []);
  await refreshSidebar();
  if (!track) openPlaylist(playlists.find((x) => x.id === p.id));
  say('Created "' + name + '"' + (track ? " with that track in it" : "") + ". Right-click it in the sidebar to rename it.");
}

function openRenameDialog(p) {
  renameId = p.id;
  $("newPlHead").textContent = "Rename playlist";
  $("newPlName").value = p.title || "";
  $("newPlSave").textContent = "Save";
  $("newPlDialog").showModal();
  $("newPlName").select();
}

async function newPlaylistSubmit(e) {
  e.preventDefault();
  const name = $("newPlName").value.trim();
  if (!name) { $("newPlName").focus(); return; }
  $("newPlDialog").close();
  if (renameId) {
    xplRename(renameId, name);
    if (currentPlaylist && currentPlaylist.id === renameId) currentPlaylist.title = name;
    await refreshSidebar();
    if (view === "playlist") renderHero();
    return;
  }
}

// ---------- right-click on a playlist in the sidebar ----------
let ctxPlaylist = null;

function openPlaylistActions(p, x, y) {
  ctxPlaylist = p;
  const m = $("plMenu");
  m.replaceChildren();
  const add = (label, action) => {
    const b = el("button", "menuItem", label);
    b.dataset.paction = action;
    m.append(b);
  };
  add("Open", "open");
  add("Share", "share");
  SHARE_PLATS.forEach(([plat, name]) => { if (sharePlatformId(p, plat)) add("Copy " + name + " link", "share:" + plat); });
  add("Export as M3U...", "m3u");
  add("Export as CSV...", "csv");
  add("Hide playlist", "hide");
  if (isXpl(p)) {
    const x = xplFind(p.id) || {};
    if (!x.collab) add("Invite collaborators", "collab");
    else if (x.collab.owner) { add("Copy invite link again", "collab"); add("Stop collaborating", "collableave"); }
    else add("Leave collaboration", "collableave");
    add("Rename...", "rename");
    add("Delete", "delete");
  } else {
    add("Switch to a playlist on this PC", "switch");
    m.append(el("div", "menuNote", "Copies it here so it can hold tracks from any platform. The original stays on " +
      (p.source === "spotify" ? "Spotify" : p.source === "local" ? "your drive" : "SoundCloud") + "."));
  }
  m.hidden = false;
  const w = m.offsetWidth;
  const h = m.offsetHeight;
  m.style.left = Math.max(8, Math.min(window.innerWidth - w - 8, x)) + "px";
  m.style.top = Math.max(8, y + h + 8 > window.innerHeight ? y - h : y) + "px";
}

function runPlaylistAction(a) {
  const p = ctxPlaylist;
  closeMenu();
  if (!p) return;
  if (a === "open") openPlaylist(p);
  else if (a === "share") sharePlaylist(p);
  else if (a.startsWith("share:")) sharePlatform(p, a.slice(6));
  else if (a === "switch") xplSwitch(p);
  else if (a === "collab") collabInvite(p);
  else if (a === "collableave") collabLeave(p);
  else if (a === "rename") openRenameDialog(p);
  else if (a === "delete") deleteXpl(p);
  else if (a === "hide") hidePlaylist(p, true);
  else if (a === "m3u" || a === "csv") plExport(p, a === "m3u" ? "m3u8" : "csv");
}

// Playlists you hide are kept (and listed in Settings > Library), just not shown in the sidebar
async function hidePlaylist(p, hide) {
  if (hide && prefs.hideOn === false) { prefs.hideOn = true; savePrefs(); }
  const id = String(p.id);
  const set = new Set((prefs.hiddenByYou || []).map(String));
  if (hide) set.add(id); else set.delete(id);
  prefs.hiddenByYou = [...set];
  savePrefs();
  await refreshSidebar();
  paintHiddenPlaylists();
  if (hide && view === "playlist" && currentPlaylist && String(currentPlaylist.id) === id) openLikes();
  say(hide ? '"' + (p.title || "Playlist") + '" is hidden. Show it again from Settings > Library > Hidden & not recommended.' : "");
}

function paintHiddenPlaylists() {
  const ids = (prefs.hiddenByYou || []).map(String);
  const box = $("hiddenPlList");
  box.replaceChildren();
  ids.forEach((id) => {
    const p = allPlaylists.find((x) => String(x.id) === id) || { id, title: "A playlist that isn't loaded right now" };
    const row = el("div", "folderRow");
    const show = el("button", "soft", "Show");
    show.addEventListener("click", () => hidePlaylist(p, false));
    row.append(el("span", "", p.title || "Untitled playlist"), show);
    box.append(row);
  });
  $("hidPlSum").textContent = "Hidden Playlists (" + ids.length + ")";
  $("hidPlBox").hidden = !ids.length;
  paintHiddenSummary();
}

async function deleteXpl(p) {
  if (!confirm('Delete "' + (p.title || "Untitled") + '" from this PC?')) return;
  xplDelete(p.id);
  await refreshSidebar();
  if (view === "playlist" && currentPlaylist && currentPlaylist.id === p.id) openLikes();
  say("Playlist deleted.");
}

function xplInit() {
  $("plImport").addEventListener("click", plImport);
  $("newPlForm").addEventListener("submit", newPlaylistSubmit);
  $("newPlCancel").addEventListener("click", () => $("newPlDialog").close());
  $("sideLists").addEventListener("contextmenu", (e) => {
    const b = e.target.closest(".plItem");
    if (!b) return;
    e.preventDefault();
    const p = playlists.find((x) => String(x.id) === b.dataset.id);
    if (p) openPlaylistActions(p, e.clientX, e.clientY);
  });
}
