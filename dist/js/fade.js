// Fade Between Tracks (Settings > Sound): each track fades out over its last seconds and the next fades
// in, as heard: its own start and end times count. Skipping fades the old one out quickly. A stand-in
// in a Jam (js/together.js) always fades, over its own fixed length, whatever the setting.
// "Its last seconds" are the last of its sound: many tracks fade themselves out, or end on a quiet
// tail, so the fade ends where the measured sound does (js/loudness.js, audible_ms) and the next
// track starts there, rather than fading over the near-silence at the end of the file.
//
//   input -> fade -> match -> EQ -> limiter -> volume   (js/audio.js)

const FADE_STANDIN_MS = 2000;
const FADE_SKIP_MS = 300;
// every track change, fading set or not: the old one goes down this fast and the new one comes up
// over this long, so a change is never a hard cut
const FADE_BASE_IN_MS = 450;
const fadeState = { out: false, pendingIn: false, forceIn: 0, movedOn: null, waitPos: null, quietSince: 0 };

// How long a track fades for: a stand-in's own, the setting's, or not at all
function fadeMsFor(t) {
  if (t && t.standInFor) return FADE_STANDIN_MS;
  // listening along: the host's fade (Fade Between Tracks as they have it), so both fade alike
  const host = typeof together !== "undefined" && together.role === "listener" && together.state && together.state.feel;
  const on = host ? host.fadeOn : prefs.fadeOn;
  const ms = host ? host.fadeMs : prefs.fadeMs;
  return on ? Math.max(500, Math.min(10000, Number(ms) || 3000)) : 0;
}

// Where the track's sound ends: its measured end, if that's before the file's (or its own end time)
function fadeEndMs(t) {
  const end = trimmedEnd(t, curDurMs());
  const a = onSpotifyEngine(t) ? null : loudFor(t);
  return a && a.audible_ms > 0 && a.audible_ms < end ? a.audible_ms : end;
}

function fadeTo(value, ms) {
  if (!norm.fade) return;
  const g = norm.fade.gain, now = norm.ctx.currentTime;
  g.cancelScheduledValues(now);
  g.setValueAtTime(Math.max(0.0001, g.value), now);
  if (ms > 0) g.linearRampToValueAtTime(Math.max(0.0001, value), now + ms / 1000);
  else g.setValueAtTime(value, now);
}

// A new track is about to play: the one still heard fades out quickly, and the new one fades in once
// its sound starts (over the setting's length if fading's on, a short moment if not)
function fadeTrackChange(prev, next) {
  fadeState.pendingIn = true;
  fadeState.waitPos = null;
  fadeState.out = false;
  fadeState.movedOn = null;
  // after a stand-in, the Jam's next track fades in too
  fadeState.forceIn = prev && prev.standInFor ? FADE_STANDIN_MS : 0;
  if (prev && nowPlaying()) fadeTo(0, FADE_SKIP_MS);
}

// The new track's sound has started
function fadeHeard() {
  if (!fadeState.pendingIn) return;
  fadeState.pendingIn = false;
  const ms = Math.max(fadeMsFor(curTrack()), fadeState.forceIn, FADE_BASE_IN_MS);
  fadeState.forceIn = 0;
  if (!norm.fade) return;
  const g = norm.fade.gain, now = norm.ctx.currentTime;
  g.cancelScheduledValues(now);
  if (ms) { g.setValueAtTime(0.0001, now); g.linearRampToValueAtTime(1, now + ms / 1000); } else g.setValueAtTime(1, now);
}

// A jump in what's playing (catching up with a Jam's host): down quickly, the jump, then in again
// over `ms` once the sound carries on (fadeHeard, or the watch in fadeTick)
function fadeJump(ms, jump) {
  fadeTo(0, 120);
  fadeState.pendingIn = true;
  fadeState.waitPos = null;
  fadeState.forceIn = ms;
  setTimeout(() => { jump(); fadeState.waitPos = null; }, 130);   // (the jump itself isn't the sound carrying on)
}

// Near the end: down to silence over the fade's length (and back up if you seek away from the end)
function fadeTick() {
  const t = curTrack();
  if (!t || !norm.fade || !nowPlaying() || switching) return;
  // Never stuck silent: skipping quickly can miss the moment a track's sound starts, which leaves
  // its fade-in waiting (and the volume down) while it plays. Playing on for a moment counts as
  // started; and turned down while not fading out, it comes back up.
  if (fadeState.pendingIn) {
    const p = curPosMs();
    if (fadeState.waitPos === null) fadeState.waitPos = p;
    else if (p - fadeState.waitPos > 700) fadeHeard();
    return;
  }
  if (!fadeState.out && norm.fade.gain.value < 0.05) {
    if (!fadeState.quietSince) fadeState.quietSince = performance.now();
    else if (performance.now() - fadeState.quietSince > 1500) { fadeState.quietSince = 0; fadeTo(1, 300); }
  } else fadeState.quietSince = 0;
  const ms = fadeMsFor(t);
  if (!ms) { if (fadeState.out) { fadeState.out = false; fadeTo(1, 0); } return; }
  const end = fadeEndMs(t);
  if (!end) return;
  const left = end - curPosMs();
  // faded out where the sound ends: on to the next one, without the quiet tail
  if (fadeState.out && left <= 0 && end < trimmedEnd(t, curDurMs()) - 300 && fadeState.movedOn !== t) {
    fadeState.movedOn = t;
    onEnded();
    return;
  }
  if (!fadeState.out && left > 0 && left <= ms) {
    fadeState.out = true;
    fadeTo(0, left);
  } else if (fadeState.out && left > ms + 400) {
    fadeState.out = false;
    fadeTo(1, 200);
  }
}

function fadeInit() {
  $("fadeOn").checked = !!prefs.fadeOn;
  $("fadeMs").value = String(prefs.fadeMs || 3000);
  const paint = () => { $("fadeSection").hidden = !prefs.fadeOn; };
  $("fadeOn").addEventListener("change", () => { prefs.fadeOn = $("fadeOn").checked; savePrefs(); paint(); });
  $("fadeMs").addEventListener("change", () => { prefs.fadeMs = Number($("fadeMs").value); savePrefs(); });
  paint();
  audio.addEventListener("playing", fadeHeard);
  setInterval(fadeTick, 100);
}
