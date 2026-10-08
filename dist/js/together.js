// Listen Along and Jams: listening together, through your relay (jam-server/, a Cloudflare Worker).
//
// The host's OmniPulse sends what it's playing: the track, where it's at, whether it's playing, the
// queue coming up, and its own touches (start and end times, fixed titles). A listener's OmniPulse
// plays the same songs itself, through its own accounts, with its own EQ and loudness, following the
// host: same track, same place, paused when the host pauses. Anything a listener can't play directly
// (no Spotify Premium, no SoundCloud sign-in, the host's own files, a plugin they don't have) plays
// from YouTube Music, or is skipped with a note.
//
//   Listen Along: from the "Listen along" button on your Discord status; anyone can listen, nobody
//                 can change anything
//   Jam:          invite-only (a link you send); members can add songs, shown as added by them
//
// Links (omnipulse://join/<code>, omnipulse://jam/<code>?k=<invite>) come from the relay's join page,
// which opens the app, or offers to download it.

const together = {
  role: null,          // "host" | "listener"
  kind: null,          // "listen" | "jam"
  code: "", hostKey: "", inviteKey: "", joinUrl: "", relay: "",
  ws: null, members: [], hostName: "", canAdd: false,
  state: null, stateAt: 0, trims: {}, saved: null, localPause: false, closing: false, pushTimer: null,
};

const relayUrl = () => String(prefs.relayUrl || "").trim().replace(/\/+$/, "");
const myName = () => (prefs.togetherName || "").trim() || (typeof spotifyName === "string" && spotifyName) || userName || "Someone";
const wsBase = (relay) => relay.replace(/^http/i, "ws");
// only a relay on the web (https), or one on this PC while testing
const relayOk = (u) => /^https:\/\/[^/\s]+$/i.test(u) || /^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/i.test(u);

// The host's start and end times win while listening along (js/trims.js asks here first)
function hostTrimOf(t) {
  return together.role === "listener" && t && together.trims[t.id] !== undefined ? together.trims[t.id] : undefined;
}

const snapOf = (t) => Object.assign(xplSnap(t), trimOf(t) ? { trim: trimOf(t) } : {}, t.requestedBy ? { requestedBy: t.requestedBy } : {});

function paintTogether() {
  const bar = $("togetherBar");
  const jamBtn = $("jamBtn");
  jamBtn.hidden = !relayUrl() || together.role === "listener";
  jamBtn.textContent = together.role === "host" && together.kind === "jam" ? "Jam" : "Start a Jam";
  jamBtn.classList.toggle("on", together.role === "host" && together.kind === "jam");
  if (together.role === "listener") {
    bar.hidden = false;
    $("togetherText").textContent = (together.kind === "jam" ? "In " + together.hostName + "'s Jam" : "Listening along with " + together.hostName) +
      (together.members.length > 1 ? " · " + together.members.length + " listening" : "");
  } else if (together.role === "host") {
    bar.hidden = false;
    const n = together.members.length;
    $("togetherText").textContent = (together.kind === "jam" ? "Your Jam" : "Listen Along") + ": " + (n ? n + (n === 1 ? " listener" : " listeners") + " (" + together.members.slice(0, 3).join(", ") + (n > 3 ? "..." : "") + ")" : "nobody yet");
  } else {
    bar.hidden = true;
  }
  $("togetherLeave").textContent = together.role === "host" ? "End" : "Leave";
}

// ---------- a connection to a room ----------
function roomConnect(url, onMessage) {
  const ws = new WebSocket(url);
  together.ws = ws;
  ws.onmessage = (e) => { let m; try { m = JSON.parse(e.data); } catch (x) { return; } onMessage(m); };
  ws.onclose = () => {
    if (together.ws !== ws || together.closing) return;
    // dropped: try again in a few seconds, while still in the room
    setTimeout(() => { if (together.ws === ws && !together.closing && together.role) roomConnect(url, onMessage); }, 4000);
  };
  return ws;
}

// ---------- hosting ----------
async function hostOpen(kind) {
  const relay = relayUrl();
  if (!relay) throw new Error("set your relay's address in Settings > Streaming > Listen Along & Jams first");
  if (together.role) await togetherEnd(true);
  const res = await fetch(relay + "/api/rooms", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ kind, host: myName() }) });
  if (!res.ok) throw new Error("the relay answered " + res.status);
  const r = await res.json();
  Object.assign(together, { role: "host", kind, code: r.code, hostKey: r.hostKey, inviteKey: r.inviteKey || "", joinUrl: r.joinUrl, relay, members: [], closing: false });
  roomConnect(wsBase(relay) + "/ws/" + r.code + "?role=host&key=" + encodeURIComponent(r.hostKey), hostMessage);
  together.ws.addEventListener("open", () => hostPush(true));
  paintTogether();
  return r;
}

function hostMessage(m) {
  if (m.type === "members") { together.members = m.list || []; paintTogether(); }
  else if (m.type === "add" && together.kind === "jam" && m.track) jamAdded(m.track, m.by);
}

function hostSnapshot() {
  const t = curTrack();
  return {
    v: 1, host: myName(), kind: together.kind,
    playing: nowPlaying(), pos: Math.round(curPosMs()),
    track: t ? snapOf(t) : null,
    next: queue.slice(qi + 1, qi + 21).map(snapOf),
  };
}

// What the host's doing, sent as it changes (and every few seconds, so drift is caught)
function hostPush(now) {
  if (together.role !== "host" || !together.ws || together.ws.readyState !== 1) return;
  clearTimeout(together.pushTimer);
  const send = () => { try { together.ws.send(JSON.stringify({ type: "state", state: hostSnapshot() })); } catch (e) {} };
  if (now) send(); else together.pushTimer = setTimeout(send, 250);
}

// A song a Jam member added: it goes in the queue, shown as theirs
function jamAdded(s, by) {
  const t = Object.assign({}, s, { user: Object.assign({}, s.user), requestedBy: (by || "Someone") + " (Jam)" });
  delete t.trim;
  addToQueueHere(t);
  say((by || "Someone") + ' added "' + (t.title || "a song") + '" to the Jam.');
  hostPush(true);
}

// Listen Along for the Discord button: a room while you're playing with the button on
async function togetherEnsureListen() {
  const want = !!(relayUrl() && prefs.presence && prefs.presListen !== false && nowPlaying() && together.role !== "listener");
  if (!want) {
    if (together.role === "host" && together.kind === "listen") togetherEnd(true);
    return "";
  }
  if (together.role === "host") return together.kind === "listen" ? together.joinUrl : "";
  try { await hostOpen("listen"); return together.joinUrl; } catch (e) { return ""; }
}

async function jamStart() {
  try {
    const r = await hostOpen("jam");
    jamDialog();
    try { await navigator.clipboard.writeText(r.joinUrl); say("Jam started. The invite link is copied: send it to the friends you want in."); } catch (e) {}
  } catch (e) {
    say("Couldn't start a Jam: " + (e && e.message ? e.message : e));
  }
}

function jamDialog() {
  const d = $("jamDialog");
  $("jamLink").value = together.joinUrl;
  $("jamMembers").textContent = together.members.length ? together.members.join(", ") : "Nobody has joined yet.";
  if (!d.open) d.showModal();
}

async function togetherEnd(quiet) {
  const was = together.role;
  together.closing = true;
  if (together.ws) {
    try { if (was === "host") together.ws.send(JSON.stringify({ type: "end" })); } catch (e) {}
    try { together.ws.close(); } catch (e) {}
  }
  if (was === "listener") listenerRestore();
  Object.assign(together, { role: null, kind: null, code: "", hostKey: "", inviteKey: "", joinUrl: "", ws: null, members: [], state: null, trims: {}, canAdd: false, localPause: false });
  paintTogether();
  if (!quiet) say(was === "host" ? "Ended. Listeners carry on with their own music." : "You left. Back to your own queue.");
  if (was === "host") presenceUpdate();
}

// ---------- listening along ----------
async function togetherLink(link) {
  let u;
  try { u = new URL(link); } catch (e) { return; }
  const kind = u.host === "jam" ? "jam" : u.host === "join" ? "listen" : null;
  const code = u.pathname.replace(/^\//, "");
  const relay = (u.searchParams.get("relay") || relayUrl()).replace(/\/+$/, "");
  if (!kind || !/^[a-z0-9]{8}$/.test(code) || !relayOk(relay)) { say("That link isn't an OmniPulse invite."); return; }
  let info = null;
  try { info = await (await fetch(relay + "/api/rooms/" + code)).json(); } catch (e) {}
  if (!info || info.error) { say("That session has ended."); return; }
  const what = kind === "jam" ? "Join " + info.host + "'s Jam?" : "Listen along with " + info.host + "?";
  const more = kind === "jam" ? " You can add songs; " + info.host + " picks what plays." : " You'll hear what they play, as they play it.";
  if (!confirm(what + more + " Your own queue is set aside until you leave, and your EQ and loudness stay yours.")) return;
  if (together.role) await togetherEnd(true);
  together.saved = { queue: queue.slice(), qi, listName: queueListName, wasPlaying: nowPlaying() };
  if (nowPlaying()) toggle();
  Object.assign(together, { role: "listener", kind, code, relay, hostName: info.host, closing: false, trims: {}, members: [] });
  const k = u.searchParams.get("k") || "";
  roomConnect(wsBase(relay) + "/ws/" + code + "?role=listener&name=" + encodeURIComponent(myName()) + (k ? "&k=" + encodeURIComponent(k) : ""), listenerMessage);
  paintTogether();
  say(kind === "jam" ? "Joined " + info.host + "'s Jam." : "Listening along with " + info.host + ".");
}

function listenerMessage(m) {
  if (m.type === "hello") { together.hostName = m.host || together.hostName; together.canAdd = !!m.canAdd; paintTogether(); }
  else if (m.type === "members") { together.members = m.list || []; paintTogether(); }
  else if (m.type === "state" && m.state) listenerFollow(m.state);
  else if (m.type === "ended") togetherEnd(true).then(() => say((together.hostName || "The host") + " ended the session. Back to your own queue."));
}

// Where the host is now: what they said, plus the time since
const hostPos = () => together.state ? together.state.pos + (together.state.playing ? performance.now() - together.stateAt : 0) : 0;

const freshCopy = (s) => {
  const t = Object.assign({}, s, { user: Object.assign({}, s.user) });
  if (srcOf(t) === "local") { const live = localTracks.find((x) => x.id === t.id); if (live) Object.assign(t, live); }
  return t;
};

function listenerFollow(s) {
  together.state = s;
  together.stateAt = performance.now();
  [s.track].concat(s.next || []).filter(Boolean).forEach((x) => { together.trims[x.id] = x.trim || null; });
  const target = s.track;
  if (!target) { if (nowPlaying()) togetherOrig.toggle(); return; }
  const upcoming = (s.next || []).map(freshCopy);
  const cur = curTrack();
  if (!cur || cur.id !== target.id) {
    queue = [freshCopy(target)].concat(upcoming);
    qi = 0;
    queueListName = together.hostName + (together.kind === "jam" ? "'s Jam" : " (Listen Along)");
    pendingResume = { id: target.id, posMs: Math.max(0, hostPos()) };
    together.localPause = false;
    togetherOrig.playCurrent().then(() => { if (!s.playing && nowPlaying()) togetherOrig.toggle(); });
    renderQueue();
    return;
  }
  queue = [cur].concat(upcoming);
  qi = 0;
  renderQueue();
  if (together.localPause) return;   // paused here on purpose: it catches up when played again
  if (s.playing && !nowPlaying()) togetherOrig.toggle();
  else if (!s.playing && nowPlaying()) togetherOrig.toggle();
  if (s.playing) {
    const want = hostPos(), have = curPosMs();
    if (Math.abs(want - have) > 3000) seekToMs(want);   // drifted: back in step
  }
}

function seekToMs(ms) {
  const t = curTrack();
  if (!t) return;
  if (onSpotifyEngine(t)) { if (spState.dur) spotifySeek(Math.min(0.999, ms / spState.dur)); }
  else if (audio.src) audio.currentTime = ms / 1000;
}

function listenerRestore() {
  const s = together.saved;
  together.saved = null;
  if (nowPlaying()) togetherOrig.toggle();
  if (!s) return;
  queue = s.queue;
  qi = s.qi;
  queueListName = s.listName;
  renderQueue();
  if (curTrack()) { fillNowPlaying(curTrack()); refreshLikeUI(); }
}

// Something a listener can't play directly: YouTube Music, or skipped with a note
async function listenerFallback(t, why) {
  const mine = streamToken;
  if (await playViaYouTube(t, mine, "Playing this one from YouTube Music (" + why + ").")) return true;
  say('Couldn\'t play "' + (t.title || "this song") + '" here (' + why + "): waiting for the next one.");
  return false;
}

// ---------- while listening along, the queue is the host's ----------
const togetherOrig = {};
function togetherGuard() {
  ["startList", "step", "playNext", "playFromPage", "addToQueue", "playCurrent", "onEnded", "toggle"].forEach((n) => { togetherOrig[n] = window[n]; });
  const askLeave = () => confirm("Leave listening along to play this? You'll go back to your own music.");
  const guard = (n) => function (...args) {
    if (together.role !== "listener") return togetherOrig[n].apply(this, args);
    if (n === "addToQueue" && together.canAdd && args[0]) {
      try { together.ws.send(JSON.stringify({ type: "add", track: snapOf(args[0]) })); say('Sent "' + (args[0].title || "the song") + '" to the Jam.'); } catch (e) { say("Couldn't reach the Jam."); }
      return;
    }
    if (n === "onEnded") return;   // the host moves on, and so does this
    if (n === "playCurrent") return togetherOrig[n].apply(this, args);
    if (n === "toggle") {
      // a pause here is yours; playing again catches up with the host
      const wasPlaying = nowPlaying();
      together.localPause = wasPlaying;
      const r = togetherOrig.toggle.apply(this, args);
      if (!wasPlaying && together.state) setTimeout(() => seekToMs(hostPos()), 400);
      return r;
    }
    if (!askLeave()) return;
    return togetherEnd(true).then(() => togetherOrig[n].apply(this, args));
  };
  ["startList", "step", "playNext", "playFromPage", "addToQueue", "onEnded", "toggle"].forEach((n) => { window[n] = guard(n); });
}
// the host's own adds (from a Jam member) go straight in
const addToQueueHere = (t) => togetherOrig.addToQueue(t);

function togetherInit() {
  togetherGuard();
  $("relayUrl").value = prefs.relayUrl || "";
  $("relayUrl").addEventListener("change", () => {
    const v = $("relayUrl").value.trim().replace(/\/+$/, "");
    if (v && !relayOk(v)) { $("relayNote").textContent = "That needs to be an https:// address (like https://omnipulse-jam.you.workers.dev)."; return; }
    prefs.relayUrl = v;
    savePrefs();
    $("relayNote").textContent = "";
    paintTogether();
    presenceUpdate();
  });
  $("togetherName").value = prefs.togetherName || "";
  $("togetherName").placeholder = myName();
  $("togetherName").addEventListener("change", () => { prefs.togetherName = $("togetherName").value.trim(); savePrefs(); });
  $("jamBtn").addEventListener("click", () => { if (together.role === "host" && together.kind === "jam") jamDialog(); else jamStart(); });
  $("togetherLeave").addEventListener("click", () => togetherEnd(false));
  $("jamCopy").addEventListener("click", async () => { try { await navigator.clipboard.writeText($("jamLink").value); say("Invite link copied."); } catch (e) {} });
  $("jamEnd").addEventListener("click", () => { $("jamDialog").close(); togetherEnd(false); });
  $("jamClose").addEventListener("click", () => $("jamDialog").close());
  // the host's state follows playback, the queue and every few seconds
  setInterval(() => hostPush(true), 8000);
  window.__deepLink = async () => {
    const links = await invoke("deep_links_take").catch(() => []);
    for (const l of links) await togetherLink(l);
  };
  window.__deepLink();
  paintTogether();
}
