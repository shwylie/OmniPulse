// Spotify side of the app: sign-in, playback through the librespot engine on the Rust side,
// and the Web API calls behind Likes, Search and playlists. Spotify tracks never share a queue
// with other platforms. Loaded before app.js, so this file only declares things; app.js calls
// spotifyInit() at startup.

let spotifyPremium = false;
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
  // like Last.fm's options: they fold out under Spotify once it's signed in and switched on
  $("spotifyOptions").hidden = !(FEATURES.spotify && spotifyPremium && prefs.spOn !== false);
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
  await invoke("spotify_logout");
  await invoke("spe_logout").catch(() => {});   // playback too
  speSettingsPaint();
  spotifyPremium = false;
  if (typeof renderPlatChips === "function") renderPlatChips();
  syncSpotifySettings();
  $("spStatus").textContent = SP_DEFAULT_NOTE;
  $("spConnect").textContent = "Sign in";
  if (queue.length && srcOf(queue[0]) === "spotify") {
    queue = [];
    qi = -1;
    fillNowPlaying(null);
    renderQueue();
  }
}

// ---------- playback (librespot in the Rust side, played through the app's audio chain) ----------
// The engine streams PCM over a Tauri channel; js/spotify-worklet.js plays it into norm.input,
// so Spotify gets the equalizer, leveling, volume and output device like everything else.
// Each load or seek starts a new "generation"; audio from an older one is dropped.
const spe = { node: null, ready: null, gen: 0, basePos: 0, level: 0, endPending: false, seekTimer: null };

function speFlush(gen) {
  spe.gen = gen;
  spe.level = 0;
  spe.endPending = false;
  if (spe.node) spe.node.port.postMessage({ type: "flush", gen });
}

function speAudio(msg) {
  let buf = msg;
  if (Array.isArray(buf)) buf = new Uint8Array(buf).buffer;
  else if (ArrayBuffer.isView(buf)) buf = buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength);
  if (!(buf instanceof ArrayBuffer) || buf.byteLength < 8) return;
  const gen = new DataView(buf).getUint32(0, true);
  if (gen < spe.gen) return;          // from before a seek or track change
  if (gen > spe.gen) speFlush(gen);   // audio for a new load can beat the command's reply
  const samples = new Float32Array(buf, 8);
  spe.node.port.postMessage({ type: "pcm", gen, samples }, [buf]);
}

function speEvent(ev) {
  if (!ev || !curTrack() || srcOf(curTrack()) !== "spotify") return;
  if (ev.type === "end") {
    // the whole track has been sent; it ends when the worklet has played the rest
    spe.endPending = true;
    spe.node.port.postMessage({ type: "eos", gen: spe.gen });
    if (spe.level === 0) speEnded();
  } else if (ev.type === "unavailable") {
    say("This track isn't available to play on Spotify.");
    spe.endPending = false;
    spState.playing = false;
    paintSpotify();
    if (qi < queue.length - 1) step(1);
  }
}

function speWorklet(e) {
  const m = e.data;
  if (m.gen !== spe.gen) return;
  if (m.type === "played") {
    spe.level = m.level;
    spState.pos = spe.basePos + (m.frames / 44100) * 1000;
    spState.ts = performance.now();
    invoke("spe_played", { gen: m.gen, frames: m.frames }).catch(() => {});
  } else if (m.type === "empty") {
    spe.level = 0;
    spState.pos = spe.basePos + (m.frames / 44100) * 1000;
    spState.ts = performance.now();
    invoke("spe_played", { gen: m.gen, frames: m.frames }).catch(() => {});
    if (spe.endPending) speEnded();
  }
}

function speEnded() {
  spe.endPending = false;
  spState.playing = false;
  spState.pos = spState.dur;
  paintSpotify();
  onEnded();
}

// Sets up the worklet and the channels once
function speSetup() {
  if (spe.ready) return spe.ready;
  spe.ready = (async () => {
    if (!normEnsure()) throw new Error("the audio engine couldn't start");
    await norm.ctx.audioWorklet.addModule("js/spotify-worklet.js");
    spe.node = new AudioWorkletNode(norm.ctx, "spotify-pcm", { numberOfInputs: 0, outputChannelCount: [2] });
    spe.node.port.onmessage = speWorklet;
    spe.node.connect(norm.input);
    const { Channel } = window.__TAURI__.core;
    const audioCh = new Channel();
    audioCh.onmessage = speAudio;
    const eventCh = new Channel();
    eventCh.onmessage = speEvent;
    await invoke("spe_attach", { audio: audioCh, events: eventCh });
  })();
  spe.ready.catch(() => { spe.ready = null; });   // allow a retry
  return spe.ready;
}

// Spotify playback signs in separately from the Spotify account above (it needs Spotify's own
// app id); it happens once, in the browser, the first time you play something
async function speLogin() {
  say("Approve Spotify playback in your browser (only needed once)...");
  await invoke("spe_login");
  speSettingsPaint();
}

async function speCall(cmd, args) {
  try {
    return await invoke(cmd, args);
  } catch (e) {
    if (String(e) !== "NOT_SIGNED_IN") throw e;
    await speLogin();
    return invoke(cmd, args);
  }
}

function spotifyPos() {
  return spState.playing ? spState.pos + (performance.now() - spState.ts) : spState.pos;
}

function paintSpotify() {
  const t = curTrack();
  if (!t || srcOf(t) !== "spotify") return;
  const pos = Math.min(spotifyPos(), spState.dur || Infinity);
  $("cur").textContent = fmt(pos);
  if (spState.dur) {
    $("seek").value = String(Math.round(Math.min(1, pos / spState.dur) * 1000));
    $("dur").textContent = fmt(spState.dur);
  }
  $("icoPlay").classList.toggle("off", spState.playing);
  $("icoPause").classList.toggle("off", !spState.playing);
}

// Progress bar, plus what the <audio> element's timeupdate does for other tracks: stats,
// scrobbling and the lyrics highlight
function spotifyTick() {
  const t = curTrack();
  if (!t || srcOf(t) !== "spotify") return;
  paintSpotify();
  if (spState.playing) playbackTick();
}

async function spotifyStart(t) {
  say("Starting Spotify...");
  await speSetup();
  if (norm.ctx.state === "suspended") await norm.ctx.resume().catch(() => {});
  spe.basePos = 0;
  spe.endPending = false;
  spe.node.port.postMessage({ type: "play" });
  const gen = await speCall("spe_play", { uri: t.uri, positionMs: 0 });
  if (curTrack() !== t) return;   // skipped meanwhile
  if (gen > spe.gen) speFlush(gen);
  spState = { pos: 0, ts: performance.now(), playing: true, dur: t.duration || 0 };
  say("");
  paintSpotify();
  playbackState(true);
}

function spotifyToggle() {
  if (spState.playing) { spotifyPause(); return; }
  if (!spe.node) { const t = curTrack(); if (t) spotifyStart(t).catch((e) => say("Could not play this track: " + e)); return; }
  if (norm.ctx.state === "suspended") norm.ctx.resume().catch(() => {});
  spe.node.port.postMessage({ type: "play" });
  invoke("spe_resume").catch((e) => say("Spotify: " + e));
  spState.ts = performance.now();
  spState.playing = true;
  paintSpotify();
  playbackState(true);
}

function spotifyPause() {
  if (!spe.node) return;
  spe.node.port.postMessage({ type: "pause" });   // silent straight away; the buffer is kept for resume
  invoke("spe_pause").catch(() => {});
  if (spState.playing) {
    spState.pos = spotifyPos();
    spState.playing = false;
    paintSpotify();
    playbackState(false);
  }
}

// The seek bar fires continuously while dragging: move the bar now, seek once it settles
function spotifySeek(frac) {
  if (!spState.dur || !spe.node) return;
  const ms = Math.round(frac * spState.dur);
  spState.pos = ms;
  spState.ts = performance.now();
  paintSpotify();
  clearTimeout(spe.seekTimer);
  spe.seekTimer = setTimeout(async () => {
    try {
      const gen = await invoke("spe_seek", { positionMs: ms });
      if (gen > spe.gen) speFlush(gen);
      spe.basePos = ms;
      presenceUpdate();   // what the <audio> element's "seeked" event does for other tracks
      widgetPush();
    } catch (e) {
      say("Spotify: " + e);
    }
  }, 150);
}

async function speSettingsPaint() {
  const on = await invoke("spe_signed_in").catch(() => false);
  $("speNote").textContent = on
    ? "Playback is signed in. Spotify plays through the equalizer, leveling and your output device."
    : "Signs in separately the first time you play a Spotify track.";
  $("speAuth").textContent = on ? "Sign out" : "Sign in";
}

function spotifyInit() {
  $("spConnect").addEventListener("click", () => {
    if (spotifyPremium) spotifyDisconnect(); else spotifyConnect();
  });
  $("speAuth").addEventListener("click", async () => {
    try {
      if (await invoke("spe_signed_in")) {
        spotifyPause();
        await invoke("spe_logout");
      } else {
        await speLogin();
        say("");
      }
    } catch (e) {
      say(String(e));
    }
    speSettingsPaint();
  });
  setInterval(spotifyTick, 500);
  spotifyCheck();
  speSettingsPaint();
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
