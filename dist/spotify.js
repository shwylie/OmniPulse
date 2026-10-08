// Spotify side of the app: sign-in, playback through the librespot engine on the Rust side,
// and the Web API calls behind Likes, Search and playlists. Spotify tracks never share a queue
// with other platforms. Loaded before app.js, so this file only declares things; app.js calls
// spotifyInit() at startup.

let spotifyPremium = false;
let spotifyFree = false;   // signed in without Premium: your library and search, played from YouTube Music

// Spotify's own player only plays for Premium; without it a Spotify track plays from YouTube Music
// (in the app's <audio> player like everything else)
const onSpotifyEngine = (t) => !!t && srcOf(t) === "spotify" && spotifyPremium && !t.ytVia;
const spotifyUsable = () => FEATURES.spotify && (spotifyPremium || (spotifyFree && ytOn()));
let spotifyName = "";
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
    spArtistId: (x.artists && x.artists[0] && x.artists[0].id) || "",
    spArtists: (x.artists || []).map((a) => ({ id: a.id, name: a.name })),
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
const SP_DEFAULT_NOTE = "Not signed in. Without Premium, songs play from YouTube Music.";

function syncSpotifySettings() {
  // like Last.fm's options: they fold out under Spotify once it's signed in and switched on
  $("spotifyOptions").hidden = !(FEATURES.spotify && spotifyPremium && prefs.spOn !== false);
  $("spConnect").textContent = spotifyPremium || spotifyFree ? "Sign out" : "Sign in";
  if (spotifyPremium) $("spStatus").textContent = "Signed in as " + (spotifyName || "you") + " \u00b7 Premium.";
  else if (spotifyFree) {
    $("spStatus").textContent = "Signed in as " + (spotifyName || "you") + " \u00b7 Free. " +
      (ytOn() ? "Spotify tracks play from YouTube Music." : "Switch on YouTube Music to play your Spotify library.");
  }
}

// Checks the account (Premium or not). If Spotify can't be asked right now (rate limited, offline),
// you stay signed in as you were last time and it's asked again in a minute; only Spotify turning the
// sign-in down (a revoked or expired refresh token) needs a new sign-in.
let spotifyCheckTimer = null;
const spRevoked = (e) => /invalid_grant|invalid_client|revoked|connect again|Not connected/i.test(String(e));

async function spotifyCheck() {
  clearTimeout(spotifyCheckTimer);
  let ok = false;
  let note = "";
  if (await invoke("spotify_signed_in")) {
    try {
      const me = await spApi("GET", "/me");
      ok = me.product === "premium";
      spotifyName = me.display_name || me.id || "";
      spotifyFree = !ok;
      prefs.spAccount = { product: me.product || "", name: spotifyName };
      savePrefs();
    } catch (e) {
      const last = prefs.spAccount;
      if (spRevoked(e)) {
        spotifyFree = false;
        note = "Spotify ended the sign-in. Sign in again.";
      } else if (!last) {
        // nothing known yet: hidden until the next try, which brings it back by itself
        spotifyFree = false;
        note = "Could not check your Spotify account (" + e + "): trying again in a minute.";
        spotifyCheckTimer = setTimeout(spotifyCheck, 60000);
      } else {
        // still signed in, as last time; ask again in a minute
        ok = last.product === "premium";
        spotifyName = last.name || "";
        spotifyFree = !ok;
        note = (/limiting requests|429/i.test(String(e)) ? "Spotify is busy right now" : "Couldn't reach Spotify") + ": checking again in a minute.";
        spotifyCheckTimer = setTimeout(spotifyCheck, 60000);
      }
    }
  }
  spotifyPremium = ok;
  if (typeof renderPlatChips === "function") renderPlatChips();
  syncSpotifySettings();
  if (note) $("spStatus").textContent = note;
  return ok;
}

// Spotify turned a request down for a permission this sign-in doesn't have (one signed in before the
// app asked for it, like following artists)
const spScopeMissing = (e) => /insufficient client scope|\b403\b.*scope|scope/i.test(String(e));

// Signs in to Spotify's Web API again, for the permissions the app asks for now. Spotify playback
// (librespot) stays signed in.
async function spotifyGrant() {
  say("Finish in your browser: Spotify asks you to allow OmniPulse to see and change who you follow.");
  await spotifyConnect();
  if (spotifyPremium || spotifyFree) say("Thanks. Spotify can be used for following now.");
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
  spotifyFree = false;
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
  if (!ev || !onSpotifyEngine(curTrack())) return;
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
  if (!onSpotifyEngine(t)) return;
  const pos = Math.min(spotifyPos(), spState.dur || Infinity);
  $("cur").textContent = fmt(viewPos(t, pos));
  if (spState.dur) {
    const d = viewDur(t, spState.dur);
    paintProgress(d ? viewPos(t, pos) / d : 0);
    $("dur").textContent = fmt(d);
  }
  $("icoPlay").classList.toggle("off", spState.playing);
  $("icoPause").classList.toggle("off", !spState.playing);
}

// Progress bar, plus what the <audio> element's timeupdate does for other tracks: stats,
// scrobbling and the lyrics highlight
function spotifyTick() {
  const t = curTrack();
  if (!onSpotifyEngine(t)) return;
  paintSpotify();
  if (spState.playing) playbackTick();
}

async function spotifyStart(t, startMs) {
  startMs = startMs || 0;
  say("Starting Spotify...");
  await speSetup();
  if (norm.ctx.state === "suspended") await norm.ctx.resume().catch(() => {});
  spe.basePos = startMs;
  spe.endPending = false;
  spe.node.port.postMessage({ type: "play" });
  const gen = await speCall("spe_play", { uri: t.uri, positionMs: startMs });
  if (curTrack() !== t) return;   // skipped meanwhile
  if (gen > spe.gen) speFlush(gen);
  spState = { pos: startMs, ts: performance.now(), playing: true, dur: t.duration || 0 };
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
    ? "Signed in. Spotify plays through your equalizer and output device."
    : "A separate sign-in, asked for the first time you play Spotify.";
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
    items.forEach((t) => likeState.set(t.id, true));
    out.push(...items);
    if (!data.next || !items.length) break;
    offset += 50;
  }
  return out;
}

// ---------- Spotify's own mixes (Daily Mixes, Discover Weekly, ...) ----------
// Developer apps' Web API tokens can't see these, so they're read with the playback engine's
// token (spe_web_get): the "Made For You" category, or failing that, a search for your Daily Mixes.
const MADE_FOR_YOU = "0JQ5DAt0tbjZptfcdMSKl3";

async function spotifyMadeForYou() {
  const keep = (list) => (list || []).filter((p) => p && p.id && p.owner && p.owner.id === "spotify");
  let found = [];
  try {
    const d = await speCall("spe_web_get", { path: "/browse/categories/" + MADE_FOR_YOU + "/playlists?limit=50" });
    found = keep(d.playlists && d.playlists.items);
  } catch (e) {
    // not offered any more: try a search instead
  }
  if (!found.some((p) => /^daily mix \d+$/i.test(p.name))) {
    try {
      const d = await speCall("spe_web_get", { path: "/search?type=playlist&limit=50&q=" + encodeURIComponent("Daily Mix") });
      const seen = new Set(found.map((p) => p.id));
      found = found.concat(keep(d.playlists && d.playlists.items).filter((p) => /^daily mix \d+$/i.test(p.name) && !seen.has(p.id)));
    } catch (e) {
      // offline, or playback isn't signed in
    }
  }
  const num = (p) => Number((p.name.match(/\d+/) || [99])[0]);
  const daily = found.filter((p) => /^daily mix \d+$/i.test(p.name)).sort((a, b) => num(a) - num(b));
  const other = found.filter((p) => !daily.includes(p));
  return { daily, other };
}

// ---------- likes (your Spotify library) ----------
// Spotify replaced its track-specific endpoints with /me/library in February 2026: it takes URIs.
async function spotifySetLiked(t, on) {
  try {
    await spApi(on ? "PUT" : "DELETE", "/me/library?uris=" + encodeURIComponent(t.uri));
  } catch (e) {
    if (/40[13]|scope|permission/i.test(String(e))) {
      throw new Error("Spotify needs a fresh sign-in to change your likes. Settings > Integrations > Spotify: Sign out, then Sign in.");
    }
    throw e;
  }
}

// Whether Spotify tracks on screen are in your library, asked 40 at a time
const spLikeAsked = new Set();
async function spotifyLikeCheck(list) {
  if (!spotifyPremium && !spotifyFree) return;
  const todo = list.filter((t) => srcOf(t) === "spotify" && t.uri && !likeState.has(t.id) && !spLikeAsked.has(t.id));
  for (let i = 0; i < todo.length; i += 40) {
    const batch = todo.slice(i, i + 40);
    batch.forEach((t) => spLikeAsked.add(t.id));
    try {
      const res = await spApi("GET", "/me/library/contains?uris=" + encodeURIComponent(batch.map((t) => t.uri).join(",")));
      if (Array.isArray(res)) batch.forEach((t, k) => likeState.set(t.id, !!res[k]));
    } catch (e) {
      return;   // not allowed yet, or offline: hearts just show as not liked
    }
  }
  if (!todo.length) return;
  if (view === "search" && prefs.hideLikes && viewTracks.some(isLikedTrack)) {
    viewTracks = viewTracks.filter((t) => !isLikedTrack(t));
    render();
  } else {
    refreshLikeUI();
  }
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

// Since February 2026 the Web API only lists the tracks of playlists you own; for anyone else's
// (and Spotify's own) it returns no items. Those are read through the playback engine instead.
async function spotifyPlaylistTracks(id, max) {
  const out = [];
  let offset = 0;
  let refused = null;
  while (out.length < (max || 500)) {
    let data;
    try {
      data = await spApi("GET", "/playlists/" + id + "/items?limit=50&offset=" + offset);
    } catch (e) {
      refused = e;
      break;
    }
    const raw = data.items || [];
    raw.map((x) => x.item || x.track)
      .filter((x) => x && x.id && (!x.type || x.type === "track"))   // skip podcast episodes
      .forEach((x) => out.push(spTrack(x)));
    if (!data.next || !raw.length) break;
    offset += 50;
  }
  if (out.length) return out;
  try {
    say("Opening this playlist through Spotify playback...");
    const list = await speCall("spe_playlist", { uri: "spotify:playlist:" + id });
    say("");
    return list.slice(0, max || 1000).map(spTrack);
  } catch (e) {
    throw refused || e;
  }
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
