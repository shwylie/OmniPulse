// Playlists on this PC, kept on SoundCloud and Spotify too.
//
// Mirrors: every playlist here has a hidden playlist on each platform you're signed in to, holding
// that platform's songs: its own, and the same songs matched from the other platforms. A song from
// Spotify (or YouTube Music, or your files) goes on SoundCloud only as its Go+ upload (the label's
// own, so the same recording); a SoundCloud Go+ song goes on Spotify by the usual close match; any
// other SoundCloud upload (remixes, edits, bootlegs) is rarely on Spotify, so it goes only when the
// title, the artist and the length all match.
//
// Follows: a platform playlist you change here becomes a playlist here, and the original is hidden.
// One of yours keeps being written to, as that platform's mirror. Anyone else's (or one you only
// follow) can't be changed, so your changes stay here; songs added or taken out over there still
// come in, as they happen.

const XM_PLATS = ["soundcloud", "spotify"];
const XM_MATCH_KEY = "sc.xplMatches";
const XM_MISS_TTL = 7 * 86400000;   // "no match" is looked for again after a week
const XM_PULL_MS = 10 * 60000;      // followed playlists are checked this often

let xmMatches = null;
function xmMatchesLoad() {
  if (!xmMatches) { try { xmMatches = JSON.parse(localStorage.getItem(XM_MATCH_KEY) || "{}"); } catch (e) { xmMatches = {}; } }
  return xmMatches;
}
function xmMatchesSave() { try { localStorage.setItem(XM_MATCH_KEY, JSON.stringify(xmMatches)); } catch (e) {} }

function xmPlatOn(plat) {
  if (plat === "soundcloud") return activePlats().includes("soundcloud") && meId !== null;
  return activePlats().includes("spotify") && (spotifyPremium || spotifyFree);
}

// A platform playlist you can change: your own SoundCloud or Spotify one
const isMyPlatformList = (p) => !!p && !isXpl(p) && (p.source === "spotify" ? !!p.own : p.source !== "local" && isOwnPlaylist(p));
// Its platform, and its id there
const xmPlatOf = (p) => (p.source === "spotify" ? "spotify" : "soundcloud");
const xmIdOf = (p) => (p.source === "spotify" ? p.spId : String(p.id));
// How the sidebar knows a platform playlist (for hiding it)
const xmSideId = (plat, id) => (plat === "spotify" ? "sp:" + id : String(id));

// ---------- the same song on the other platform ----------
async function xmFindSc(t) {
  const names = guessNames(t);
  const q = (names.artist || artistName(t)) + " " + matchTitle(t);
  const found = asList(await api("/tracks?q=" + encodeURIComponent(q) + "&limit=20&access=playable,preview"));
  // only its Go+ upload: the label's own release of it
  const best = found.filter((c) => c && c.id && isPreview(c))
    .map((c) => ({ c, s: ytMatchScore(t, c) }))
    .filter((x) => x.s >= 0.8 && artistHit(t, x.c))
    .sort((a, b) => b.s - a.s)[0];
  return best ? best.c.id : null;
}

async function xmFindSp(t) {
  const names = guessNames(t);
  const title = matchTitle(t), artist = names.artist || artistName(t);
  // a SoundCloud upload that isn't Go+: the title, the artist and the length must all match
  const strict = srcOf(t) === "soundcloud" && !isPreview(t);
  const len = t.full_duration || t.duration || 0;
  let items = [];
  for (const q of ['track:"' + title + '" artist:"' + artist + '"', artist + " " + title]) {
    const data = await spApi("GET", "/search?type=track&limit=10&q=" + encodeURIComponent(q));
    items = ((data.tracks && data.tracks.items) || []).filter((x) => x && x.uri && x.is_playable !== false).map(spTrack);
    if (items.length) break;
  }
  const ok = (x) => strict ? x.s >= 1.5 && len > 0 && Math.abs(len - (x.c.duration || 0)) <= 3000 : x.s >= 0.8;
  const best = items.map((c) => ({ c, s: ytMatchScore(t, c) }))
    .filter((x) => artistHit(t, x.c) && ok(x))
    .sort((a, b) => b.s - a.s)[0];
  return best ? best.c.uri : null;
}

// The song's id on a platform (a SoundCloud id, a Spotify uri), or null if it isn't there
async function xmFind(t, plat) {
  if (srcOf(t) === plat) return plat === "spotify" ? t.uri || null : (isPluginTrack(t) ? null : t.id);
  const m = xmMatchesLoad();
  const key = String(t.id);
  const saved = m[key] && m[key][plat];
  if (saved && saved.id) return saved.id;
  if (saved && saved.none && Date.now() - (saved.at || 0) < XM_MISS_TTL) return null;
  let found;
  try { found = plat === "soundcloud" ? await xmFindSc(t) : await xmFindSp(t); }
  catch (e) { return null; }   // couldn't search: tried again next time
  m[key] = Object.assign(m[key] || {}, { [plat]: found ? { id: found } : { none: true, at: Date.now() } });
  xmMatchesSave();
  return found;
}

// ---------- the platforms' side ----------
async function xmCreate(plat, title) {
  if (plat === "soundcloud") {
    const c = await invoke("playlist_write", { method: "POST", path: "/playlists", body: { playlist: { title, sharing: "private", tracks: [] } } });
    return String(c.id);
  }
  const c = await spApi("POST", "/me/playlists", { name: title, public: false, description: "Kept in step with OmniPulse." });
  return c.id;
}

async function xmWrite(plat, id, title, ids) {
  if (plat === "soundcloud") {
    await invoke("playlist_write", { method: "PUT", path: "/playlists/" + id, body: { playlist: { title, tracks: ids.map((x) => ({ id: x })) } } });
    return;
  }
  await spApi("PUT", "/playlists/" + id, { name: title });
  await spApi("PUT", "/playlists/" + id + "/items", { uris: ids.slice(0, 100) });
  for (let i = 100; i < ids.length; i += 100) await spApi("POST", "/playlists/" + id + "/items", { uris: ids.slice(i, i + 100) });
}

function xmRemove(plat, id) {
  const done = plat === "soundcloud"
    ? invoke("playlist_write", { method: "DELETE", path: "/playlists/" + id, body: null })
    : spApi("DELETE", "/me/library?uris=" + encodeURIComponent("spotify:playlist:" + id));
  return done.catch(() => {});
}

function xmPlatTracks(plat, id) {
  return plat === "spotify" ? spotifyPlaylistTracks(id) : platformPlaylistTracks({ source: "soundcloud", id });
}

// What a platform playlist holds, as the track ids this app gives them
const xmSeenOf = (plat, ids) => ids.map((x) => (plat === "spotify" ? "sp:" + String(x).split(":").pop() : String(x)));

// ---------- keeping them in step ----------
// A followed playlist's changes over there, brought in here; true if anything changed
async function xmPull(p) {
  const f = p.follow;
  if (!f || !xmPlatOn(f.plat)) return false;
  let now;
  try { now = await xmPlatTracks(f.plat, f.id); } catch (e) { return false; }
  const nowIds = now.map((t) => String(t.id));
  if (!Array.isArray(f.seen)) { f.seen = nowIds; return false; }
  const seen = new Set(f.seen), here = new Set(nowIds);
  const gone = new Set(f.seen.filter((k) => !here.has(k)));
  const fresh = now.filter((t) => !seen.has(String(t.id)));
  f.seen = nowIds;
  if (!gone.size && !fresh.length) return false;
  p.tracks = p.tracks.filter((s) => !gone.has(String(s.id)));
  const have = new Set(p.tracks.map((s) => String(s.id)));
  fresh.forEach((t) => { if (!have.has(String(t.id))) { have.add(String(t.id)); p.tracks.push(xplSnap(t)); } });
  return true;
}

const xmSaid = new Set();
function xmFail(p, plat, e) {
  const msg = String(e && e.message ? e.message : e);
  const name = plat === "spotify" ? "Spotify" : "SoundCloud";
  const why = plat === "spotify" && /\b(403|insufficient|scope)\b/i.test(msg)
    ? "sign in to Spotify again (Settings > Integrations) to let OmniPulse keep your playlists there."
    : msg;
  const line = 'Couldn\'t keep "' + (p.title || "a playlist") + '" up to date on ' + name + ": " + why;
  if (!xmSaid.has(line)) { xmSaid.add(line); say(line); }
}

async function xmSyncOne(p, pull) {
  const changed = pull && p.follow ? await xmPull(p) : false;
  const on = XM_PLATS.filter(xmPlatOn);
  const sig = JSON.stringify([p.title, p.tracks.map((s) => String(s.id)), on]);
  if (!changed && p.mirrorSig === sig) return false;   // nothing new since the last time
  const tracks = xplTracks(p).filter((t) => !t.missing);
  let failed = false;
  for (const plat of on) {
    try {
      const ids = [];
      for (const t of tracks) {
        const id = await xmFind(t, plat);
        if (id && !ids.includes(id)) ids.push(id);
      }
      let id = p.mirrors && p.mirrors[plat];
      if (!id && !ids.length) continue;
      if (!id) {
        id = await xmCreate(plat, p.title);
        p.mirrors = Object.assign(p.mirrors || {}, { [plat]: id });
        p.made = Object.assign(p.made || {}, { [plat]: true });
        // hidden from the app: it's this playlist, on that platform
        prefs.hiddenPlaylists = [...new Set((prefs.hiddenPlaylists || []).concat(xmSideId(plat, id)))];
        savePrefs();
        xplSave(true);
      }
      await xmWrite(plat, id, p.title, ids);
      // (what was written there isn't a change from that side)
      if (p.follow && p.follow.plat === plat && String(p.follow.id) === String(id)) p.follow.seen = xmSeenOf(plat, ids);
    } catch (e) {
      failed = true;
      xmFail(p, plat, e);
    }
  }
  if (!failed) p.mirrorSig = sig;
  xplSave(true);
  return changed;
}

let xmBusy = false, xmAgain = false, xmTimer = null;
// Soon after a change (several changes in a row are written once)
function xmSoon(ms) {
  clearTimeout(xmTimer);
  xmTimer = setTimeout(() => xmSyncAll(false), ms === undefined ? 2000 : ms);
}

async function xmSyncAll(pull) {
  if (xmBusy) { xmAgain = true; return; }
  xmBusy = true;
  let changed = false;
  try {
    for (const p of xpls.slice()) {
      if (await xmSyncOne(p, pull).catch(() => false)) changed = true;
    }
  } finally {
    xmBusy = false;
  }
  if (changed) {
    await refreshSidebar();
    if (view === "playlist" && currentPlaylist && isXpl(currentPlaylist)) openPlaylist(playlists.find((x) => x.id === currentPlaylist.id) || currentPlaylist);
  }
  if (xmAgain) { xmAgain = false; xmSoon(); }
}

// A platform playlist you changed here: from now on a playlist here (see the top of this file)
async function xplAdopt(p, add) {
  const plat = xmPlatOf(p), pid = xmIdOf(p), mine = isMyPlatformList(p);
  const tracks = await platformPlaylistTracks(p);
  const copy = xplCreate(p.title, tracks, String(p.id));
  copy.follow = { plat, id: pid, seen: tracks.map((t) => String(t.id)) };
  if (mine) copy.mirrors = { [plat]: pid };   // yours: kept up to date there
  prefs.hiddenPlaylists = [...new Set((prefs.hiddenPlaylists || []).concat(String(p.id)))];
  savePrefs();
  const n = xplAdd(copy.id, add || []);
  await refreshSidebar();
  if (view === "playlist" && currentPlaylist && String(currentPlaylist.id) === String(p.id)) openPlaylist(playlists.find((x) => x.id === copy.id));
  const where = plat === "spotify" ? "Spotify" : "SoundCloud";
  // (said once: from now on it's a playlist here, and later changes just say what was added)
  say(mine ? (n ? "Added " + plural(n, "track", "tracks") + ". " : "") + '"' + copy.title + '" now takes songs from any platform, and stays up to date on ' + where + "."
    : "You can't edit this playlist on " + where + ", so your changes are saved on this PC only.", { force: true });
  return copy;
}

function xmInit() {
  // once the platforms are signed in, and from then on now and then
  setTimeout(() => xmSyncAll(true), 15000);
  setInterval(() => xmSyncAll(true), XM_PULL_MS);
}
