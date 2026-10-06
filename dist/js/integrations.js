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

async function sendPresence() {
  const t = curTrack();
  if (!prefs.presence || !t || srcOf(t) === "spotify" || audio.paused) {
    invoke("presence_clear").catch(() => {});
    return;
  }
  try {
    const now = Math.floor(Date.now() / 1000);
    const start = now - Math.floor(audio.currentTime || 0);
    const end = t.duration ? start + Math.floor(t.duration / 1000) : null;
    const note = await invoke("presence_set", {
      details: t.title || "Unknown",
      state: artistName(t) || "Unknown",
      start,
      end,
      image: srcOf(t) === "local" ? null : (artUrl(t, "t500x500") || null),
      url: t.sharing === "private" ? null : (t.permalink_url || null),
    });
    presenceNote(note || "Showing on your Discord profile, with a link to the track.", false);
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
  if (!lfmActive() || !prefs.scrobble || srcOf(t) === "spotify") return;
  invoke("lastfm_call", { method: "track.updateNowPlaying", params: lfmTrackParams(t) }).catch(() => {});
}

function lfmScrobble(t, startedAt) {
  if (!lfmActive() || !prefs.scrobble || srcOf(t) === "spotify") return;
  const params = lfmTrackParams(t);
  params.push(["timestamp", String(Math.floor(startedAt / 1000))]);
  invoke("lastfm_call", { method: "track.scrobble", params }).catch((e) => say("Scrobble failed: " + e));
}

function integrationsInit() {
  $("presToggle").addEventListener("change", () => {
    prefs.presence = $("presToggle").checked;
    savePrefs();
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
