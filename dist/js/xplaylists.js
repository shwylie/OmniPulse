// Playlists on this PC: they can hold tracks from any platform and are never sent anywhere.
// Each keeps a small snapshot of its tracks, enough to show and play them without asking the
// platform. Local files are kept by id and looked up again, since their addresses change per run.

const XPL_KEY = "sc.xplaylists";
const XPL_FIELDS = ["source", "id", "uri", "kind", "title", "displayArtist", "duration", "artwork_url", "permalink_url",
  "album", "spAlbum", "genre", "created_at", "playback_count", "sharing", "access", "policy"];

let xpls = [];
try { xpls = JSON.parse(localStorage.getItem(XPL_KEY) || "[]"); } catch (e) { xpls = []; }

function xplSave() {
  try { localStorage.setItem(XPL_KEY, JSON.stringify(xpls)); } catch (e) { say("Could not save your playlists on this PC: " + e); }
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
  return xpls.map((p) => ({ source: "xpl", id: p.id, title: p.title, user: { username: "This PC" }, count: p.tracks.length }));
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
  return fresh.length;
}

function xplRemove(id, trackId) {
  const p = xplFind(id);
  if (!p) return;
  p.tracks = p.tracks.filter((s) => s.id !== trackId);
  xplSave();
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
  xplSave();
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
async function xplSwitch(p) {
  say('Copying "' + (p.title || "playlist") + '"...');
  try {
    const tracks = await platformPlaylistTracks(p);
    const copy = xplCreate(p.title, tracks, String(p.id));
    prefs.hiddenPlaylists = [...new Set((prefs.hiddenPlaylists || []).concat(String(p.id)))];
    savePrefs();
    await refreshSidebar();
    openPlaylist(playlists.find((x) => x.id === copy.id));
    say('"' + copy.title + '" is now a playlist on this PC (' + copy.tracks.length + " tracks). The original is unchanged on " +
      (p.source === "spotify" ? "Spotify" : p.source === "local" ? "your drive" : "SoundCloud") + ".");
  } catch (e) {
    say("Could not copy the playlist: " + e);
  }
}

// ---------- the New playlist / Rename dialog ----------
let newPlTrack = null;   // a track to put in the new playlist
let renameId = null;

function openNewPlaylistDialog(track) {
  newPlTrack = track || null;
  renameId = null;
  const scOk = activePlats().includes("soundcloud") && (!track || srcOf(track) === "soundcloud");
  $("newPlHead").textContent = "New playlist";
  $("newPlName").value = "";
  $("newPlWhere").hidden = false;
  $("newPlSc").disabled = !scOk;
  $("newPlScNote").textContent = scOk ? "Private, on your SoundCloud account. SoundCloud tracks only."
    : track ? "Only for SoundCloud tracks." : "Needs SoundCloud switched on.";
  $("newPlLocal").checked = true;
  $("newPlSave").textContent = "Create";
  $("newPlDialog").showModal();
  $("newPlName").focus();
}

function openRenameDialog(p) {
  renameId = p.id;
  newPlTrack = null;
  $("newPlHead").textContent = "Rename playlist";
  $("newPlName").value = p.title || "";
  $("newPlWhere").hidden = true;
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
  const track = newPlTrack;
  try {
    if ($("newPlSc").checked && !$("newPlSc").disabled) {
      const created = await createPlaylist(name, track);   // on SoundCloud
      const p = created && playlists.find((x) => String(x.id) === String(created.id));
      if (p && !track) openPlaylist(p);
    } else {
      const p = xplCreate(name, track ? [track] : []);
      await refreshSidebar();
      if (!track) openPlaylist(playlists.find((x) => x.id === p.id));
    }
    say('Created "' + name + '"' + (track ? " with that track in it." : ". Add tracks with the playlist button on any track, or drag them here."));
  } catch (err) {
    say("Could not create the playlist: " + err);
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
  if (isXpl(p)) {
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
  else if (a === "switch") xplSwitch(p);
  else if (a === "rename") openRenameDialog(p);
  else if (a === "delete") deleteXpl(p);
}

async function deleteXpl(p) {
  if (!confirm('Delete "' + (p.title || "Untitled") + '" from this PC?')) return;
  xplDelete(p.id);
  await refreshSidebar();
  if (view === "playlist" && currentPlaylist && currentPlaylist.id === p.id) openLikes();
  say("Playlist deleted.");
}

function xplInit() {
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
