// Match loudness and skip long silences.
//
// Every local file and SoundCloud track is measured over the whole track (crates/core/src/loudness.rs:
// integrated loudness in LUFS, plus where it's silent), and played at a gain that brings it to the
// chosen target. Spotify's files carry Spotify's own whole-track values, which librespot applies
// (spe_set_normalisation), so Spotify isn't touched here. Until a track's measurement arrives (it's
// usually ready before the track starts: the next track is measured ahead), a typical gain is used.
//
// Silences of the chosen length or more are skipped: jumped over inside a track, or the track ends
// if the rest is silent. Where nothing is measured (Spotify, or past the measured part of a long set)
// the level is watched as it plays, and the track ends after that long in silence.

const LOUD_TARGETS = { quiet: -18, normal: -14, loud: -11 };
const LOUD_NOTES = {
  off: "Tracks play as loud as they were made.",
  quiet: "Every track at the same loudness, measured over the whole track (-18 LUFS, the most room for dynamic tracks).",
  normal: "Every track at the same loudness, measured over the whole track (-14 LUFS, like Spotify and YouTube).",
  loud: "Every track at the same loudness, measured over the whole track (-11 LUFS, like Spotify's Loud).",
};
const LOUD_MAX_BOOST = 9;        // dB: quiet tracks are raised at most this much (the limiter catches peaks)
const LOUD_TYPICAL_LUFS = -10;   // until a track is measured, it's assumed to be this loud (most releases are close)

const loudResults = new Map();   // key -> analysis, or null if it couldn't be measured
const loudPending = new Map();   // key -> promise
let loudGainNow = 0;             // dB, what the current track is playing at

const loudTarget = () => LOUD_TARGETS[prefs.loudMatch] ?? null;

function loudKey(t) {
  if (!t || isPreview(t)) return null;
  const src = srcOf(t);
  return src === "local" || src === "soundcloud" || src === "youtube" ? src + ":" + t.id : null;
}

function loudAnalyse(t) {
  const key = loudKey(t);
  if (!key) return Promise.resolve(null);
  if (loudResults.has(key)) return Promise.resolve(loudResults.get(key));
  if (loudPending.has(key)) return loudPending.get(key);
  // local tracks are "local:<file id>"; the core knows files by their id alone
  const id = srcOf(t) === "youtube" ? t.ytId : String(t.id).replace(/^local:/, "");
  const p = invoke("loudness_analyse", { source: srcOf(t), id })
    .then((a) => a, () => null)
    .then((a) => { loudResults.set(key, a); loudPending.delete(key); return a; });
  loudPending.set(key, p);
  return p;
}

const loudFor = (t) => (loudKey(t) ? loudResults.get(loudKey(t)) : undefined);   // undefined: not yet known

// The gain for a track, in dB
function loudGainFor(t) {
  const target = loudTarget();
  if (target === null || !t || srcOf(t) === "spotify") return 0;
  const a = loudFor(t);
  const lufs = a ? a.lufs : LOUD_TYPICAL_LUFS;
  return Math.max(-30, Math.min(LOUD_MAX_BOOST, target - lufs));
}

function loudApply(t, instant) {
  if (!norm.match) return;
  const db = loudGainFor(t);
  // nothing changed (the measurement was already known): leave the level set at the start alone
  if (!instant && Math.abs(db - loudGainNow) < 0.05) { loudPaint(); return; }
  loudGainNow = db;
  const g = Math.pow(10, db / 20);
  const now = norm.ctx.currentTime;
  norm.match.gain.cancelScheduledValues(now);
  if (instant) norm.match.gain.setValueAtTime(g, now);
  else norm.match.gain.setTargetAtTime(g, now, 0.4);   // a measurement arrived mid-track: ease into it
  loudPaint();
}

// Called as each track starts
function loudTrackStarted(t) {
  silence.live = 0;
  silence.lastPos = -1;
  loudApply(t, true);
  if (loudTarget() === null && !Number(prefs.skipSilence)) return;
  loudAnalyse(t).then(() => {
    if (curTrack() !== t) return;
    loudApply(t, false);
    // measure the next one now, so it starts at the right level
    const next = queue[qi + 1];
    if (next) loudAnalyse(next);
  });
}

// The line under "Match loudness": what the current track measured and what's being done about it
function loudPaint() {
  const box = $("loudNow");
  if (!box) return;
  const t = curTrack();
  if (!t || loudTarget() === null) { box.hidden = true; return; }
  box.hidden = false;
  if (srcOf(t) === "spotify") { box.textContent = "Now playing: Spotify's own loudness value for this track."; return; }
  const a = loudFor(t);
  const change = (db) => (Math.abs(db) < 0.1 ? "played as it is" : (db > 0 ? "turned up " : "turned down ") + Math.abs(db).toFixed(1) + " dB");
  if (a) {
    const part = a.analysed_ms < a.duration_ms - 1000 ? " (from its first " + Math.round(a.analysed_ms / 60000) + " minutes)" : "";
    box.textContent = "Now playing: " + a.lufs.toFixed(1) + " LUFS" + part + ", " + change(loudGainNow) + ".";
  } else if (a === null) {
    box.textContent = "Now playing: couldn't be measured, so it's " + change(loudGainNow) + " as a typical track.";
  } else {
    box.textContent = "Now playing: measuring...";
  }
}

// ---------- skipping silence ----------
const silence = { live: 0, lastPos: -1, lastTick: 0 };
const SILENT_RMS = -55;   // dBFS, as in the measurement
const SILENT_PEAK = -40;

function silenceTick() {
  const min = Number(prefs.skipSilence) * 1000;
  const t = curTrack();
  const now = performance.now();
  const dt = silence.lastTick ? now - silence.lastTick : 0;
  silence.lastTick = now;
  if (!min || !t || !nowPlaying()) { silence.live = 0; return; }
  const pos = curPosMs();
  const moving = silence.lastPos >= 0 && pos > silence.lastPos;
  silence.lastPos = pos;
  const a = srcOf(t) === "spotify" ? null : loudFor(t);

  if (a && pos < a.analysed_ms) {
    silence.live = 0;
    const run = a.silences.find((r) => r[1] - r[0] >= min && pos >= r[0] + 200 && pos < r[1] - 500);
    if (!run) return;
    const secs = Math.round((run[1] - Math.max(pos, run[0])) / 1000);
    if (run[1] >= a.duration_ms - 1500 && a.analysed_ms >= a.duration_ms - 1500) {
      say("Skipped " + secs + " s of silence at the end.");
      onEnded();
    } else {
      audio.currentTime = Math.max(0, run[1] - 150) / 1000;
      say("Skipped " + secs + " s of silence.");
    }
    return;
  }
  // nothing measured here: wait for the measurement if it's on its way, otherwise listen
  if (a === undefined && loudKey(t)) return;
  const lvl = probeLevel();
  if (!lvl || !moving) return;
  if (lvl.rms < SILENT_RMS && lvl.peak < SILENT_PEAK) silence.live += dt;
  else silence.live = 0;
  const left = curDurMs() - pos;
  if (silence.live >= min && left > 3000) {
    silence.live = 0;
    say("Moved on after " + Math.round(min / 1000) + " s of silence.");
    onEnded();
  }
}

// ---------- settings ----------
function loudSpotifySync() {
  const target = loudTarget();
  invoke("spe_set_normalisation", { on: target !== null, targetLufs: target ?? -14 }).catch(() => {});
}

function loudInit() {
  const sel = $("loudSel");
  const paint = () => {
    sel.value = prefs.loudMatch in LOUD_TARGETS ? prefs.loudMatch : "off";
    $("loudNote").textContent = LOUD_NOTES[sel.value];
    loudPaint();
  };
  paint();
  sel.addEventListener("change", () => {
    prefs.loudMatch = sel.value;
    savePrefs();
    paint();
    loudSpotifySync();
    const t = curTrack();
    if (t) {
      loudApply(t, false);
      if (loudTarget() !== null) loudAnalyse(t).then(() => { if (curTrack() === t) loudApply(t, false); });
    }
  });
  loudSpotifySync();

  const sil = $("silenceSel");
  sil.value = String(Number(prefs.skipSilence) || 0);
  sil.addEventListener("change", () => {
    prefs.skipSilence = Number(sil.value);
    savePrefs();
    const t = curTrack();
    if (t && prefs.skipSilence) loudAnalyse(t);
  });
  setInterval(silenceTick, 250);
}
