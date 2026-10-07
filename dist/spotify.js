// Spotify side of the app: sign-in, Spotify's web player, and the Web API calls behind
// Likes, Search and playlists. Spotify tracks never share a queue with other platforms.
// The web player plays inside Spotify's own protected iframe, so its sound can't go through
// the equalizer, the leveler or the output picker: it uses whatever device Windows gives
// this app. Loaded before app.js, so this file only declares things; app.js calls
// spotifyInit() at startup.

let spotifyPremium = false;
let spotifyPlayer = null;
let spotifyDeviceId = null;
let spotifySdk = null;
let spotifyEnded = false;
let spState = { pos: 0, ts: 0, playing: false, dur: 0 };

const spApi = (method, path, body) =>
  invoke("spotify_api", { method, path, body: body === undefined ? null : body });

function spTrack(x) {
  const imgs = (x.album && x.album.images) || [];
  const img = imgs.length ? (imgs[1] || imgs[0]).url : "";
  return {
    source: "spotify",
    id: "sp:" + x.id,
    uri: x.uri,
    title: x.name,
    user: { username: (x.artists || []).map((a) => a.name).join(", ") },
    artwork_url: img,
    duration: x.duration_ms,
    permalink_url: x.external_urls && x.external_urls.spotify,
    spAlbum: x.album && x.album.id ? {
      id: x.album.id, name: x.album.name, images: x.album.images || [],
      total_tracks: x.album.total_tracks, album_type: x.album.album_type, artists: x.album.artists || [],
    } : null,
  };
}

// ---------- account: Spotify stays hidden unless this is a Premium account ----------
const SP_DEFAULT_NOTE = "Not signed in. Spotify only appears once you sign in with a Premium account.";

function syncSpotifySettings() {
  $("spConnect").textContent = spotifyPremium ? "Sign out" : "Sign in";
  if (spotifyPremium) $("spStatus").textContent = "Signed in with a Premium account.";
}

async function spotifyCheck() {
  let ok = false;
  if (await invoke("spotify_signed_in")) {
    try {
      const me = await spApi("GET", "/me");
      ok = me.product === "premium";
      if (!ok) {
        await invoke("spotify_logout");
        $("spStatus").textContent = "Spotify stays hidden: this account is not Premium.";
      }
    } catch (e) {
      $("spStatus").textContent = "Could not check your Spotify account: " + e;
    }
  }
  spotifyPremium = ok;
  if (typeof renderPlatChips === "function") renderPlatChips();
  syncSpotifySettings();
  return ok;
}

async function spotifyConnect() {
  $("spStatus").textContent = "Finish signing in in your browser...";
  try {
    await invoke("spotify_login");
  } catch (e) {
    $("spStatus").textContent = String(e);
    return;
  }
  await spotifyCheck();
}

async function spotifyDisconnect() {
  spotifyPause();
  if (spotifyPlayer) { try { spotifyPlayer.disconnect(); } catch (e) {} }
  spotifyPlayer = null;
  spotifyDeviceId = null;
  spotifySdk = null;
  await invoke("spotify_logout");
  spotifyPremium = false;
  if (typeof renderPlatChips === "function") renderPlatChips();
  $("spStatus").textContent = SP_DEFAULT_NOTE;
  $("spConnect").textContent = "Sign in";
  if (queue.length && srcOf(queue[0]) === "spotify") {
    queue = [];
    qi = -1;
    fillNowPlaying(null);
    renderQueue();
  }
}

// ---------- playback (Spotify's own web player, which acts as a device) ----------
function spotifyLoadSdk() {
  if (spotifySdk) return spotifySdk;
  spotifySdk = new Promise((resolve, reject) => {
    window.onSpotifyWebPlaybackSDKReady = () => {
      const p = new window.Spotify.Player({
        name: "SC Client",
        getOAuthToken: (cb) => {
          invoke("spotify_token").then(cb).catch((e) => say("Spotify sign-in expired: " + e));
        },
        volume: prefs.volume,
      });
      p.addListener("ready", ({ device_id }) => { spotifyDeviceId = device_id; resolve(p); });
      p.addListener("not_ready", () => { spotifyDeviceId = null; });
      p.addListener("initialization_error", ({ message }) =>
        reject(new Error("Spotify's player could not start in this app (" + message + ").")));
      p.addListener("authentication_error", ({ message }) =>
        reject(new Error("Spotify sign-in problem: " + message)));
      p.addListener("account_error", ({ message }) =>
        reject(new Error("Spotify account problem: " + message)));
      p.addListener("playback_error", ({ message }) => say("Spotify playback error: " + message));
      p.addListener("player_state_changed", spotifyState);
      p.connect().then((ok) => { if (!ok) reject(new Error("Spotify's player could not connect.")); });
      spotifyPlayer = p;
    };
    const s = document.createElement("script");
    s.src = "https://sdk.scdn.co/spotify-player.js";
    s.onerror = () => reject(new Error("Could not load Spotify's player (no internet?)."));
    document.head.append(s);
  });
  spotifySdk.catch(() => { spotifySdk = null; spotifyPlayer = null; });   // allow a retry
  return spotifySdk;
}

function spotifyState(s) {
  if (!s) { spState.playing = false; return; }
  spState = { pos: s.position, ts: performance.now(), playing: !s.paused, dur: s.duration };
  paintSpotify();
}

function spotifyPos() {
  return spState.playing ? spState.pos + (performance.now() - spState.ts) : spState.pos;
}

function paintSpotify() {
  const t = curTrack();
  if (!t || srcOf(t) !== "spotify") return;
  const pos = spotifyPos();
  $("cur").textContent = fmt(pos);
  if (spState.dur) {
    $("seek").value = String(Math.round(Math.min(1, pos / spState.dur) * 1000));
    $("dur").textContent = fmt(spState.dur);
  }
  $("icoPlay").classList.toggle("off", spState.playing);
  $("icoPause").classList.toggle("off", !spState.playing);
}

// Moves the progress bar and starts the next queued track when this one ends
function spotifyTick() {
  const t = curTrack();
  if (!t || srcOf(t) !== "spotify") return;
  paintSpotify();
  if (spState.playing && spState.dur && !spotifyEnded && spotifyPos() >= spState.dur - 400) {
    spotifyEnded = true;
    onEnded();
  }
}

async function spotifyStart(t) {
  spotifyEnded = false;
  say("Starting Spotify...");
  const p = await spotifyLoadSdk();
  if (p.activateElement) { try { await p.activateElement(); } catch (e) {} }
  const play = () => spApi("PUT", "/me/player/play?device_id=" + encodeURIComponent(spotifyDeviceId), { uris: [t.uri] });
  try {
    await play();
  } catch (e) {
    await new Promise((r) => setTimeout(r, 1000));   // the new device can take a moment to register
    await play();
  }
  spState = { pos: 0, ts: performance.now(), playing: true, dur: t.duration || 0 };
  say("");
  paintSpotify();
}

function spotifyToggle() {
  if (spotifyPlayer) spotifyPlayer.togglePlay();
}
function spotifyPause() {
  if (spotifyPlayer) { try { spotifyPlayer.pause(); } catch (e) {} }
  spState.playing = false;
}
function spotifySeek(frac) {
  if (!spState.dur || !spotifyPlayer) return;
  const ms = Math.round(frac * spState.dur);
  spotifyPlayer.seek(ms);
  spState.pos = ms;
  spState.ts = performance.now();
}
function spotifyVolume(v) {
  if (spotifyPlayer) spotifyPlayer.setVolume(v);
}
function spotifyMute(muted) { spotifyVolume(muted ? 0 : prefs.volume); }

function spotifyInit() {
  $("spConnect").addEventListener("click", () => {
    if (spotifyPremium) spotifyDisconnect(); else spotifyConnect();
  });
  setInterval(spotifyTick, 500);
  spotifyCheck();
}

// ---------- lists for the platform switch on Likes / Search / Rediscover ----------
async function spotifyLikedTracks(max) {
  const out = [];
  let offset = 0;
  while (out.length < (max || 50)) {
    const data = await spApi("GET", "/me/tracks?limit=50&offset=" + offset);
    const items = (data.items || []).map((x) => x.track).filter((x) => x && x.id).map(spTrack);
    out.push(...items);
    if (!data.next || !items.length) break;
    offset += 50;
  }
  return out;
}

async function spotifySearchTracks(q) {
  const base = "/search?type=track&q=" + encodeURIComponent(q) + "&limit=";
  let data;
  try { data = await spApi("GET", base + "20"); } catch (e) { data = await spApi("GET", base + "10"); }
  return ((data.tracks && data.tracks.items) || []).filter((x) => x && x.id).map(spTrack);
}

// ---------- your Spotify playlists (sidebar) ----------
async function spotifyPlaylists() {
  const out = [];
  let offset = 0;
  while (out.length < 200) {
    const data = await spApi("GET", "/me/playlists?limit=50&offset=" + offset);
    const items = (data.items || []).filter((x) => x && x.id);
    items.forEach((x) => out.push({
      source: "spotify",
      id: "sp:" + x.id,
      spId: x.id,
      title: x.name || "Untitled playlist",
      user: { username: (x.owner && x.owner.display_name) || "" },
    }));
    if (!data.next || !items.length) break;
    offset += 50;
  }
  return out;
}

async function spotifyPlaylistTracks(id, max) {
  const out = [];
  let offset = 0;
  while (out.length < (max || 500)) {
    let data;
    try {
      data = await spApi("GET", "/playlists/" + id + "/tracks?limit=50&offset=" + offset);
    } catch (e) {
      data = await spApi("GET", "/playlists/" + id + "/items?limit=50&offset=" + offset);   // newer name for the same list
    }
    const raw = data.items || [];
    raw.map((x) => x.track || x.item)
      .filter((x) => x && x.id && (!x.type || x.type === "track"))   // skip podcast episodes
      .forEach((x) => out.push(spTrack(x)));
    if (!data.next || !raw.length) break;
    offset += 50;
  }
  return out;
}

// ---------- album tracks ----------
async function spotifyAlbumTracks(id) {
  const album = await spApi("GET", "/albums/" + id);
  const albumInfo = { id: album.id, name: album.name, images: album.images || [], total_tracks: album.total_tracks, album_type: album.album_type, artists: album.artists || [] };
  let page = album.tracks || { items: [] };
  const out = (page.items || []).filter((x) => x && x.id).map((x) => spTrack(Object.assign({}, x, { album: albumInfo })));
  let offset = out.length;
  while (page.next && offset < 300) {
    page = await spApi("GET", "/albums/" + id + "/tracks?limit=50&offset=" + offset);
    const more = (page.items || []).filter((x) => x && x.id).map((x) => spTrack(Object.assign({}, x, { album: albumInfo })));
    if (!more.length) break;
    out.push(...more);
    offset += more.length;
  }
  return out;
}
