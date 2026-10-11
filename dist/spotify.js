// Spotify side of the app: sign-in, playback through the librespot engine on the Rust side,
// and the Web API calls behind Likes, Search and playlists. Spotify tracks never share a queue
// with other platforms. Loaded before app.js, so this file only declares things; app.js calls
// spotifyInit() at startup.

let spotifyPremium = false;
let spotifyFree = false;   // signed in without Premium: your library and search, played from YouTube Music

// Spotify's own player only plays for Premium; without it a Spotify track plays from YouTube Music
// (in the app's <audio> player like everything else)
const onSpotifyEngine = (t) => !!t && srcOf(t) === "spotify" && spotifyPremium && !t.ytVia && !t.spGone;
const spotifyUsable = () => FEATURES.spotify && (spotifyPremium || (spotifyFree && ytOn()));
let spotifyName = "";
let spotifyMeId = "";   // your Spotify user id (which playlists are yours to change)
let spState = { pos: 0, ts: 0, playing: false, dur: 0 };

const SP_MARKET_PATHS = /^\/(me\/tracks|playlists\/[^/]+\/items|albums\/|search\?type=track|tracks\/)/;
const spApi = (method, path, body) => {
  if (method === "GET" && SP_MARKET_PATHS.test(path) && !/[?&]market=/.test(path)) path += (path.includes("?") ? "&" : "?") + "market=from_token";
  return invoke("spotify_api", { method, path, body: body === undefined ? null : body });
};

function spTrack(x) {
  const imgs = (x.album && x.album.images) || [];
  const img = imgs.length ? (imgs[1] || imgs[0]).url : "";
  return tidyFeat({
    source: "spotify",
    id: "sp:" + x.id,
    uri: x.uri,
    title: x.name,
    user: { username: (x.artists || []).map((a) => a.name).join(", ") },
    spArtistId: (x.artists && x.artists[0] && x.artists[0].id) || "",
    spGone: x.is_playable === false || undefined,   // removed from Spotify (greyed out in its app)
    spArtists: (x.artists || []).map((a) => ({ id: a.id, name: a.name })),
    artwork_url: img,
    duration: x.duration_ms,
    spPop: typeof x.popularity === "number" ? x.popularity : undefined,   // 0-100 (search ranks by it)
    permalink_url: x.external_urls && x.external_urls.spotify,
    spAlbum: x.album && x.album.id ? {
      id: x.album.id, name: x.album.name, images: x.album.images || [],
      total_tracks: x.album.total_tracks, album_type: x.album.album_type, artists: x.album.artists || [],
    } : null,
  });
}

// ---------- Spotify asking the app to wait (too many requests) ----------
// It says how long; the app waits all of it (crates/core/src/spotify.rs), and says until when
function spWaitSecs(e) {
  const m = /limiting requests[^]*?(\d+)\s*s\b/i.exec(String(e));
  return m ? Number(m[1]) : 0;
}
function spWaitUntilText(secs) {
  const at = new Date(Date.now() + secs * 1000);
  const sameDay = at.toDateString() === new Date().toDateString();
  return (sameDay ? "" : at.toLocaleDateString(undefined, { weekday: "long" }) + " ") + at.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

// ---------- account: Spotify stays hidden unless this is a Premium account ----------
const SP_DEFAULT_NOTE = "Not signed in. Without Premium, songs play from YouTube Music.";

function syncSpotifySettings() {
  // its options (Your Keys) fold out under Spotify once it's set up and switched on (js/apisetup.js)
  if (typeof keySetupStatus === "function") keySetupStatus();
  $("spConnect").textContent = spotifyPremium || spotifyFree ? "Sign out" : "Sign in";
  if (spotifyPremium) nameNote($("spStatus"), "Signed in as ", spotifyName || "you", " \u00b7 Premium.");
  else if (spotifyFree) {
    nameNote($("spStatus"), "Signed in as ", spotifyName || "you", " \u00b7 Free. " +
      (ytOn() ? "Spotify tracks play from YouTube Music." : "Switch on YouTube Music to play your Spotify library."));
  } else $("spStatus").textContent = prefs.spNeedsKeys ? SP_FREE_NOTE : SP_DEFAULT_NOTE;
}

// Checks the account (Premium or not). If Spotify can't be asked right now (rate limited, offline),
// you stay signed in as you were last time and it's asked again in a minute; only Spotify turning the
// sign-in down (a revoked or expired refresh token) needs a new sign-in.
let spotifyCheckTimer = null;
let spCrashSaid = false;
const spNotPremium = (e) => /premium account required/i.test(String(e));
const SP_FREE_NOTE = "That's a free Spotify account. Spotify only lets Premium accounts sign in straight away; a free one signs in with the Spotify app of someone with Premium who adds you to it: Set up shows how.";
// Spotify turning down an app whose owner hasn't Premium (its rule since February 2026)
const spOwnerNotPremium = (e) => /premium subscription required for the owner/i.test(String(e));
const SP_OWNER_NOTE = "Spotify turned these app keys down: an app only works when its owner has Premium (Spotify's rule since February 2026), so a free account can't use one of its own. Use the Client ID of someone with Premium who's added you to their app: Your keys shows how.";
const spRevoked = (e) => /invalid_grant|invalid_client|revoked|connect again|Not connected/i.test(String(e));

async function spotifyCheck() {
  clearTimeout(spotifyCheckTimer);
  let ok = false;
  let note = "";
  // Spotify playback took the app down last time (it can, for some accounts): it's been signed out
  const crashed = await invoke("spe_crashed_last_time").catch(() => false);
  if (crashed && !spCrashSaid) {
    spCrashSaid = true;
    say("Spotify playback closed the app last time, so it's been signed out of it. A free Spotify account needs your own free app keys: Settings > Integrations > Spotify.", { force: true });
  }
  if (await invoke("spotify_signed_in")) {
    try {
      // signed in for playback: the account from the playback sign-in's own login (Premium or not),
      // not the Web API, which everyone signed in this way shares (Spotify's limits are per app); its
      // display name from the Web API after, when Spotify allows
      spNote("checking the account");
      const pb = (await invoke("spe_signed_in").catch(() => false)) ? await invoke("spe_account").catch((e) => { spNote("account through playback failed: " + e); return null; }) : null;
      spNote("account: " + (pb ? pb.product || "(none given)" : "not through playback"));
      // (not Premium: the playback sign-in only serves Premium, and using it for more can take the
      // app down: signed out of it at once, as Spotify turning it down would)
      if (pb && pb.product && pb.product !== "premium") throw new Error("Premium account required");
      if (pb && pb.product) {
        spApi("GET", "/me").then((me) => {
          if (!me || !me.id) return;
          spotifyName = me.display_name || me.id;
          spotifyMeId = me.id;
          prefs.spAccount = { product: pb.product, name: spotifyName };
          savePrefs();
          syncSpotifySettings();
        }).catch(() => {});
      }
      const me = pb && pb.product
        ? { product: pb.product, id: pb.id, display_name: (prefs.spAccount && prefs.spAccount.name) || "" }
        : await spApi("GET", "/me");
      ok = me.product === "premium";
      spotifyName = me.display_name || me.id || "";
      spotifyMeId = me.id || "";
      spotifyFree = !ok;
      prefs.spAccount = { product: me.product || "", name: spotifyName };
      savePrefs();
    } catch (e) {
      const last = prefs.spAccount;
      if (spNotPremium(e)) {
        // Premium has ended: the playback sign-in is over (it only takes Premium); your own
        // developer app's sign-in carries on, if you have one
        await invoke("spe_logout").catch(() => {});
        if (await invoke("spotify_signed_in")) return spotifyCheck();
        spotifyFree = false;
        prefs.spNeedsKeys = true;
        savePrefs();
        if (typeof keySetupStatus === "function") await keySetupStatus();
        note = SP_FREE_NOTE;
      } else if (spOwnerNotPremium(e)) {
        spotifyFree = false;
        prefs.spNeedsKeys = true;
        savePrefs();
        note = SP_OWNER_NOTE;
        say(SP_OWNER_NOTE, { force: true, stay: true, actions: [["Show Me How", () => keySetupOpen("spotifyFree", null, () => spotifyConnect())]] });
      } else if (spRevoked(e)) {
        spotifyFree = false;
        note = "Spotify ended the sign-in. Sign in again.";
      } else if (!last) {
        // nothing known yet: hidden until the next try, which brings it back by itself (as soon as
        // Spotify said it can, when it asked for a pause)
        spotifyFree = false;
        const wait = spWaitSecs(e);
        note = wait ? "Signed in. Spotify asked OmniPulse to wait a moment before checking your account: back by itself in " + wait + " s."
          : "Could not check your Spotify account (" + e + "): trying again in a minute.";
        spotifyCheckTimer = setTimeout(spotifyCheck, wait ? (wait + 2) * 1000 : 60000);
      } else {
        // still signed in, as last time; ask again in a minute
        ok = last.product === "premium";
        spotifyName = last.name || "";
        spotifyFree = !ok;
        const wait = spWaitSecs(e);
        if (wait) {
          // Spotify asked for a pause: nothing is sent until it's over, then it carries on by itself
          note = ["Signed in as ", spotifyName || "you", ". Spotify asked OmniPulse to slow down, so it's back at " + spWaitUntilText(wait) + " by itself."];
          spotifyCheckTimer = setTimeout(spotifyCheck, Math.max(60, wait + 3) * 1000);
        } else {
          note = "Couldn't reach Spotify: checking again in a minute.";
          spotifyCheckTimer = setTimeout(spotifyCheck, 60000);
        }
      }
    }
  }
  spotifyPremium = ok;
  if (typeof renderPlatChips === "function") renderPlatChips();
  syncSpotifySettings();
  if (Array.isArray(note)) nameNote($("spStatus"), note[0], note[1], note[2]);   // the name hidden in Streamer Mode
  else if (note) $("spStatus").textContent = note;
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

// One sign-in: Spotify's own (the playback engine's), which covers your library, likes, search and
// playback. It only takes Premium accounts; a free account signs in through a developer app of your
// own instead (Set up, js/apisetup.js), and plays from YouTube Music.
// (a step of Spotify's sign-in, in diag.log: an app that closes leaves its last steps there)
const spNote = (msg) => invoke("diag_note", { msg: "spotify: " + msg }).catch(() => {});

async function spotifyConnect() {
  spNote("sign-in started");
  $("spStatus").textContent = "Finish signing in in your browser...";
  let keys = false;
  try { keys = !!(await invoke("config_status")).spotify; } catch (e) {}
  // a free account, known already, with its keys set up: straight to signing in with them (not the
  // playback sign-in again, which only takes Premium)
  if (prefs.spNeedsKeys && keys) {
    try { await invoke("spotify_login"); } catch (e2) { $("spStatus").textContent = String(e2); return; }
    await spotifyCheck();
    speSettingsPaint();
    if (spotifyPremium || spotifyFree) say("Signed in to Spotify.", { force: true });
    return;
  }
  try {
    await invoke("spe_login");
    spNote("playback sign-in done");
    prefs.spNeedsKeys = false;
    savePrefs();
  } catch (e) {
    spNote("playback sign-in refused: " + e);
    if (!spNotPremium(e)) { $("spStatus").textContent = String(e); return; }
    if (!keys) {
      prefs.spNeedsKeys = true;
      savePrefs();
      if (typeof keySetupStatus === "function") await keySetupStatus();
      $("spStatus").textContent = SP_FREE_NOTE;
      // the steps for your own keys, at once: saved, it signs in with them
      if (typeof keySetupOpen === "function") {
        keySetupOpen("spotifyFree", "That's a free Spotify account. Spotify only lets Premium accounts sign in through its own player, and only lets an app's keys work when the app's owner has Premium, so a free account signs in with the app of someone with Premium who adds you to it. The steps are below; when you save, OmniPulse signs you in.", () => spotifyConnect());
      }
      return;
    }
    try {
      await invoke("spotify_login");
    } catch (e2) {
      $("spStatus").textContent = String(e2);
      return;
    }
  }
  await spotifyCheck();
  speSettingsPaint();
  if (spotifyPremium || spotifyFree) say("Signed in to Spotify.", { force: true });
}

async function spotifyDisconnect() {
  spotifyPause();
  await invoke("spotify_logout");
  await invoke("spe_logout").catch(() => {});   // playback too
  // the saved playlist list was this account's (js/platforms.js)
  try { localStorage.removeItem("sc.spPlaylists"); } catch (e) {}
  spListsChecked = false;
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
    spGoneNow(curTrack());
  } else if (ev.type === "alternative") {
    // librespot played another release in its place: fine if it's the same song, not if it isn't
    const t = curTrack();
    const alt = { title: ev.title || "", user: { username: ev.artists || artistName(t) } };
    const same = matchWords(matchTitle(alt)).join(" ") === matchWords(matchTitle(t)).join(" ");
    if (!same) spGoneNow(t);
  }
}

// A Spotify track that can't be played there any more: the same song from YouTube Music instead
function spGoneNow(t) {
  if (!t) return;
  spe.endPending = false;
  spotifyPause();
  t.spGone = true;
  forEachCopy(t.id, (x) => { x.spGone = true; });
  playCurrent();
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

// Spotify playback's sign-in, if it's missing (signed in through your own developer app before it was
// the default): once, in the browser, the first time you play something
async function speLogin() {
  say("Finish signing in to Spotify playback in your browser (only needed once)...", { stay: true, force: true });
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
  // (while the next track is found, the Spotify track before is still what's heard: its progress shows)
  const t = typeof switching !== "undefined" && switching ? switching.prev : curTrack();
  if (!onSpotifyEngine(t)) return;
  if (pendingResume && pendingResume.id === t.id) return;   // restored, not started: the restored position stays shown
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
  if (curTrack() !== t) {
    // skipped meanwhile: to another Spotify track, which starts itself; to anything else, this stops
    if (!onSpotifyEngine(curTrack())) { spe.node.port.postMessage({ type: "pause" }); invoke("spe_pause").catch(() => {}); }
    return;
  }
  if (gen > spe.gen) speFlush(gen);
  spState = { pos: startMs, ts: performance.now(), playing: true, dur: t.duration || 0 };
  fadeHeard();
  say("");
  paintSpotify();
  playbackState(true);
}

function spotifyToggle() {
  if (spState.playing) { spotifyPause(); return; }
  // not started yet (a restored session, say): from where it was left, or its set start
  if (!spe.node) { if (curTrack()) playCurrent(); return; }
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
  nativeOutFlush();
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

// (playback signs in with the account now: nothing of its own to show)
function speSettingsPaint() {}

function spotifyInit() {
  $("spConnect").addEventListener("click", () => {
    if (spotifyPremium || spotifyFree) spotifyDisconnect(); else spotifyConnect();
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
    if (typeof spLikedNote === "function") spLikedNote(t, on);
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
  // (all your likes kept here: known without asking Spotify)
  if (typeof spLikedComplete !== "undefined" && spLikedComplete) {
    let any = false;
    list.forEach((t) => { if (srcOf(t) === "spotify" && !likeState.has(t.id)) { likeState.set(t.id, spLikedIds.has(t.id)); any = true; } });
    if (any) refreshLikeUI();
    return;
  }
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
      own: !!(x.owner && spotifyMeId && x.owner.id === spotifyMeId),
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
