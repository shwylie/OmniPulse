// Picks up where you left off: the queue, where you were in it and in the current track are saved
// as you listen, and restored (paused) when the app starts. Press play to carry on.

const SESSION_KEY = "sc.session";
let sessionTimer = null;
let pendingResume = null;   // { id, posMs }: where to start the restored track when it's played

function sessionSnapshot() {
  return {
    savedAt: Date.now(),
    qi,
    listName: queueListName,
    // until the restored track is played, keep the position it was restored at
    posMs: Math.round(pendingResume && curTrack() && pendingResume.id === curTrack().id ? pendingResume.posMs : curPosMs()),
    // the same small snapshots as playlists on this PC, plus who asked for it and autoplay picks
    queue: queue.slice(0, 1000).map((t) => Object.assign(xplSnap(t),
      t.autoplay ? { autoplay: true } : {}, t.requestedBy ? { requestedBy: t.requestedBy } : {},
      t.queuedByYou ? { queuedByYou: true } : {})),
  };
}

function sessionSave(now) {
  clearTimeout(sessionTimer);
  const write = () => {
    try {
      if (qi < 0 || !queue.length) localStorage.removeItem(SESSION_KEY);
      else localStorage.setItem(SESSION_KEY, JSON.stringify(sessionSnapshot()));
    } catch (e) {}
  };
  if (now) write(); else sessionTimer = setTimeout(write, 600);
}

// Local files are matched to this run's addresses when they're played (see startStream)
function liveLocal(t) {
  if (srcOf(t) !== "local" || t.url) return t;
  const live = localTracks.find((x) => x.id === t.id);
  return live ? Object.assign({}, live, { autoplay: t.autoplay, requestedBy: t.requestedBy }) : t;
}

function sessionRestore() {
  let s = null;
  try { s = JSON.parse(localStorage.getItem(SESSION_KEY) || "null"); } catch (e) {}
  if (!s || !Array.isArray(s.queue) || !s.queue.length || s.qi < 0 || s.qi >= s.queue.length) return false;
  queue = s.queue.map((x) => Object.assign({}, x, { user: Object.assign({}, x.user) }));
  queueListName = s.listName || "";
  qi = s.qi;
  const t = curTrack();
  pendingResume = { id: t.id, posMs: s.posMs || 0 };
  fillNowPlaying(t);
  refreshLikeUI();
  renderQueue();
  $("cur").textContent = fmt(s.posMs || 0);
  if (t.duration) paintProgress((s.posMs || 0) / t.duration);
  return true;
}

// Where the restored track should start, once (then playback is normal again)
function takeResume(t) {
  if (!pendingResume || !t || pendingResume.id !== t.id) { pendingResume = null; return 0; }
  const ms = pendingResume.posMs;
  pendingResume = null;
  return ms > 3000 && (!t.duration || ms < t.duration - 3000) ? ms : 0;
}

function sessionInit() {
  setInterval(() => { if (nowPlaying()) sessionSave(true); }, 5000);   // keeps the position fresh
  window.addEventListener("beforeunload", () => sessionSave(true));
}
