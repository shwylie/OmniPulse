// Discord activity and Last.fm.

// ---------- Discord activity ----------
let presenceTimer = null;

function presenceUpdate() {
  clearTimeout(presenceTimer);
  presenceTimer = setTimeout(sendPresence, 800);   // Discord limits how often this can change
}

let presenceLastMsg = "";

function presenceNote(msg, isError) {
  $("presNote").textContent = msg;
  if (isError && msg !== presenceLastMsg) say("Discord: " + msg);
  presenceLastMsg = msg;
}

// Discord options (Settings > Streaming > Discord options); all on unless switched off
const presOpt = (key) => prefs[key] !== false;

// "m:ss", as SoundCloud links take it after #t=
const tStamp = (ms) => { const s = Math.max(0, Math.floor(ms / 1000)); return Math.floor(s / 60) + ":" + String(s % 60).padStart(2, "0"); };

// The buttons other people see under your status (up to two). Listen along opens the track at about
// where you are (SoundCloud links can say where to start; Spotify's can't, so it just opens the track).
function presenceButtons(t) {
  const link = t.sharing === "private" ? "" : t.permalink_url || "";
  if (!link) return [];
  const sp = srcOf(t) === "spotify" || srcOf(t) === "youtube";   // links that can't say where to start
  const out = [];
  if (sp) {
    // Spotify's and YouTube's links only open the track, so one button, named for what it does
    if (presOpt("presListen") || presOpt("presLink")) out.push({ label: openLabel(t), url: link });
    return out;
  }
  if (presOpt("presListen")) out.push({ label: "Listen along", url: link.split("#")[0] + "#t=" + tStamp(curPosMs()) });
  if (presOpt("presLink")) out.push({ label: openLabel(t), url: link });
  return out;
}

async function sendPresence() {
  const t = curTrack();
  if (!prefs.presence || !t || !nowPlaying()) {
    invoke("presence_clear").catch(() => {});
    return;
  }
  try {
    const now = Math.floor(Date.now() / 1000);
    const start = now - Math.floor(viewPos(t, curPosMs()) / 1000);
    const end = t.duration ? start + Math.floor(viewDur(t, t.duration) / 1000) : null;
    const showTime = presOpt("presTime");
    const note = await invoke("presence_set", {
      details: t.title || "Unknown",
      state: artistName(t) || "Unknown",
      start: showTime ? start : null,
      end: showTime ? end : null,
      image: !presOpt("presArt") || srcOf(t) === "local" ? null : (artUrl(t, "t500x500") || null),
      buttons: presenceButtons(t),
    });
    presenceNote(note || "Showing on your Discord profile.", false);
  } catch (e) {
    presenceNote(String(e), true);
  }
}

// ---------- Last.fm ----------
let lfmConnected = false;
const lfmActive = () => lfmConnected && prefs.lfmOn !== false;

async function lfmRefresh() {
  try {
    const s = await invoke("lastfm_status");
    lfmConnected = !!s.connected;
    $("lastfmSection").hidden = !(lfmConnected && prefs.lfmOn !== false);
    $("lfmOn").checked = prefs.lfmOn !== false;
    $("lfmNote").textContent = lfmConnected
      ? "Signed in as " + (s.name || "your account") + "."
      : "Not signed in. Needs lastfm_api_key and lastfm_secret in config.json.";
    $("lfmConnect").textContent = lfmConnected ? "Sign out" : "Sign in";
  } catch (e) {}
}

async function lfmConnectClick() {
  if (lfmConnected) {
    await invoke("lastfm_disconnect");
    await lfmRefresh();
    return;
  }
  $("lfmNote").textContent = "Approve the app in your browser, then come back here...";
  try {
    await invoke("lastfm_connect");
  } catch (e) {
    $("lfmNote").textContent = String(e);
    return;
  }
  await lfmRefresh();
}

function lfmTrackParams(t) {
  const { artist, title } = guessNames(t);
  const p = [["artist", artist || "Unknown"], ["track", title || t.title || "Unknown"]];
  if (t.duration) p.push(["duration", String(Math.round(t.duration / 1000))]);
  return p;
}

function lfmNowPlaying(t) {
  if (!lfmActive() || !prefs.scrobble) return;
  invoke("lastfm_call", { method: "track.updateNowPlaying", params: lfmTrackParams(t) }).catch(() => {});
}

function lfmScrobble(t, startedAt) {
  if (!lfmActive() || !prefs.scrobble) return;
  const params = lfmTrackParams(t);
  params.push(["timestamp", String(Math.floor(startedAt / 1000))]);
  invoke("lastfm_call", { method: "track.scrobble", params }).catch((e) => say("Scrobble failed: " + e));
}

function integrationsInit() {
  ["presArt", "presTime", "presLink", "presListen"].forEach((key) => {
    $(key).checked = presOpt(key);
    $(key).addEventListener("change", () => { prefs[key] = $(key).checked; savePrefs(); presenceUpdate(); });
  });
  // Listen along points at where you are: keep it roughly current while playing
  // (Discord accepts an update every 15 s at most)
  setInterval(() => { if (prefs.presence && presOpt("presListen") && nowPlaying()) presenceUpdate(); }, 30000);
  $("presSection").hidden = !prefs.presence;
  $("presToggle").addEventListener("change", () => {
    prefs.presence = $("presToggle").checked;
    savePrefs();
    $("presSection").hidden = !prefs.presence;   // its options fold out only while it's on
    presenceUpdate();
  });
  $("scrobbleToggle").addEventListener("change", () => {
    prefs.scrobble = $("scrobbleToggle").checked;
    savePrefs();
  });
  $("lfmConnect").addEventListener("click", lfmConnectClick);
  // Last.fm can be switched off without signing out
  $("lfmOn").checked = prefs.lfmOn !== false;
  $("lfmOn").addEventListener("change", () => {
    prefs.lfmOn = $("lfmOn").checked;
    savePrefs();
    lfmRefresh();
  });
  lfmRefresh();
}
