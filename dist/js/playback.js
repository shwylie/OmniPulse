// Shuffle, repeat, queue reordering, and the hooks app.js calls as tracks start and play.

// ---------- shuffle and repeat ----------
let repeatMode = "off";   // off | all | one
let shuffleOn = false;
let shuffleBackup = null; // upcoming tracks in their original order, to undo shuffle

// Taskbar thumbnail buttons: keep play/pause, shuffle and like icons current
// Is anything audible right now? (Spotify plays through its own engine, not the <audio> element)
function nowPlaying() {
  const t = curTrack();
  return !!t && (onSpotifyEngine(t) ? spState.playing : !audio.paused);
}

// Where the current track is up to, and how long it is, in ms, whichever engine is playing it
function curPosMs() {
  const t = curTrack();
  if (!t) return 0;
  return onSpotifyEngine(t) ? spotifyPos() : (audio.currentTime || 0) * 1000;
}
function curDurMs() {
  const t = curTrack();
  if (!t) return 0;
  if (onSpotifyEngine(t)) return spState.dur || t.duration || 0;
  return isFinite(audio.duration) && audio.duration ? audio.duration * 1000 : t.duration || 0;
}

function taskbarSync() {
  const t = curTrack();
  invoke("taskbar_state", {
    playing: nowPlaying(),
    shuffle: shuffleOn,
    liked: isLiked(t),
    canLike: canLike(t),
  }).catch(() => {});
}

function paintModes() {
  $("shuffleBtn").classList.toggle("on", shuffleOn);
  $("shuffleBtn").title = shuffleOn ? "Shuffle on" : "Shuffle off";
  $("repeatBtn").classList.toggle("on", repeatMode !== "off");
  $("repeatBtn").dataset.mode = repeatMode;
  $("repeatBtn").title = repeatMode === "one" ? "Repeat this track" : repeatMode === "all" ? "Repeat the queue" : "Repeat off";
  taskbarSync();
}

// Shuffles everything after the current track
function shuffleUpcoming() {
  if (qi < 0) return;
  const rest = queue.slice(qi + 1);
  shuffleBackup = rest.slice();
  // what you queued yourself and chat requests keep their place; autoplay's picks stay last
  const kept = rest.filter((t) => t.queuedByYou || t.requestedBy);
  const auto = rest.filter((t) => t.autoplay && !kept.includes(t));
  const others = rest.filter((t) => !kept.includes(t) && !auto.includes(t));
  queue = queue.slice(0, qi + 1).concat(kept, shuffledOrder(others), auto);
  renderQueue();
}

function unshuffleUpcoming() {
  if (qi < 0 || !shuffleBackup) { shuffleBackup = null; return; }
  const rest = queue.slice(qi + 1);
  const present = new Set(rest.map((t) => t.id));
  const restored = shuffleBackup.filter((t) => present.has(t.id));
  const known = new Set(shuffleBackup.map((t) => t.id));
  const added = rest.filter((t) => !known.has(t.id));
  queue = queue.slice(0, qi + 1).concat(restored, added);
  shuffleBackup = null;
  renderQueue();
}

function toggleShuffle() {
  shuffleOn = !shuffleOn;
  prefs.shuffle = shuffleOn;
  savePrefs();
  if (shuffleOn) shuffleUpcoming(); else unshuffleUpcoming();
  paintModes();
}

function cycleRepeat() {
  repeatMode = repeatMode === "off" ? "all" : repeatMode === "all" ? "one" : "off";
  prefs.repeat = repeatMode;
  savePrefs();
  paintModes();
  renderQueue();   // the autoplay note depends on repeat
}

// Called when a track finishes, from the audio element or the Spotify player
function onEnded() {
  if (repeatMode === "one") {
    if (onSpotifyEngine(curTrack())) playCurrent();
    else { audio.currentTime = trimStart(curTrack()) / 1000; audio.play(); }
    return;
  }
  if (qi < queue.length - 1) { step(1); return; }
  if (repeatMode === "all" && queue.length) { qi = 0; playCurrent(); return; }
  if (autoplayActive()) autoplayAdvance();
}

// ---------- hooks called from app.js ----------
function trackStarted(t) {
  fadeIn(document.querySelector(".nowText"));
  fadeIn(document.querySelector(".npInfo"));
  widgetCleared = false;
  setTimeout(widgetPush, 300);
  normEnsure();
  loudTrackStarted(t);
  castTrack();
  ytPrepareNext();
  session = { id: t.id, secs: 0, last: null, counted: false, scrobbled: false, startedAt: Date.now() };
  presenceUpdate();
  if (prefs.artAccent) updateArtAccent(t);
  if (view === "now") ensureLyrics();
  lfmNowPlaying(t);
}

function playbackTick() {
  const t = curTrack();
  if (!t || !session || session.id !== t.id) return;
  const now = curPosMs() / 1000;
  if (session.last !== null) {
    const d = now - session.last;
    if (d > 0 && d < 2) session.secs += d;
  }
  session.last = now;
  const durS = (t.duration || curDurMs()) / 1000;
  if (!session.counted && session.secs >= Math.min(30, durS ? durS / 2 : 30)) {
    session.counted = true;
    countPlay(t);
  }
  if (!session.scrobbled && durS > 30 && session.secs >= Math.min(durS / 2, 240)) {
    session.scrobbled = true;
    lfmScrobble(t, session.startedAt);
  }
  if (lyricsLines && view === "now") highlightLyric(now);
}

function playbackState(playing) {
  widgetPush();
  accentFollowPlayback(playing);
  if (playing && norm.ctx && norm.ctx.state === "suspended") norm.ctx.resume().catch(() => {});
  taskbarSync();
  if ("mediaSession" in navigator) {
    try { navigator.mediaSession.playbackState = playing ? "playing" : "paused"; } catch (e) {}
  }
  presenceUpdate();
}

function playbackInit() {
  repeatMode = prefs.repeat || "off";
  shuffleOn = !!prefs.shuffle;
  paintModes();

  $("shuffleBtn").addEventListener("click", toggleShuffle);
  $("repeatBtn").addEventListener("click", cycleRepeat);

  audio.addEventListener("play", () => playbackState(true));
  audio.addEventListener("pause", () => playbackState(false));
  audio.addEventListener("seeked", presenceUpdate);
  audio.addEventListener("timeupdate", playbackTick);

  // Windows media overlay buttons (if the engine shows one)
  if ("mediaSession" in navigator) {
    const handlers = [
      ["play", () => { if (audio.paused) toggle(); }],
      ["pause", () => { if (!audio.paused) toggle(); }],
      ["previoustrack", () => step(-1)],
      ["nexttrack", () => step(1)],
    ];
    handlers.forEach(([action, fn]) => { try { navigator.mediaSession.setActionHandler(action, fn); } catch (e) {} });
  }
}
