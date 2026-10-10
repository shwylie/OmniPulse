// Listen Along and Jams: listening together, through your relay (jam-server/, a Cloudflare Worker).
//
// The host's OmniPulse sends what it's playing: the track, where it's at, whether it's playing, the
// queue coming up, and its own touches (start and end times, fixed titles). A listener's OmniPulse
// plays the same songs itself, through its own accounts, with its own EQ and loudness, following the
// host: same track, same place, paused when the host pauses. Anything a listener can't play directly
// (no Spotify Premium, no SoundCloud sign-in, the host's own files, a plugin they don't have) plays
// from YouTube Music, or is skipped with a note.
//
//   Listen Along: from the "Listen in" button on your Discord status; anyone can listen, nobody
//                 can change anything
//   Jam:          invite-only (a link you send); members can add songs, shown as added by them
//
// Links (omnipulse://join/<code>, omnipulse://jam/<code>?k=<invite>, and omnipulse://playlist/<code> for a
// shared playlist, js/share.js) come from the relay's pages,
// which opens the app, or offers to download it.

const together = {
  role: null,          // "host" | "listener"
  kind: null,          // "listen" | "jam"
  code: "", hostKey: "", inviteKey: "", joinUrl: "", relay: "", joinAsks: [], waiting: false, url: "",
  ws: null, members: [], hostName: "", canAdd: false,
  state: null, stateAt: 0, trims: {}, picks: {}, saved: null, localPause: false, closing: false, pushTimer: null,
};

// OmniPulse's relay (jam-server/, on Cloudflare); one set by hand before there was a built-in one still wins
const RELAY_URL = "https://omnipulse-jam.shwylie101.workers.dev";
const relayUrl = () => (String(prefs.relayUrl || "").trim() || RELAY_URL).replace(/\/+$/, "");
const myName = () => (prefs.togetherName || "").trim() || (typeof spotifyName === "string" && spotifyName) || userName || "Someone";
const wsBase = (relay) => relay.replace(/^http/i, "ws");
// Settings > Listen Along & Jams (on unless switched off)
const togetherOn = () => prefs.jamsOn !== false;
// This app's own id, so the relay can keep each person to one request waiting (and to none, once
// declined), whatever name they give
function myCid() {
  if (!/^[a-z0-9]{20}$/.test(prefs.clientId || "")) {
    const abc = "abcdefghijklmnopqrstuvwxyz0123456789";
    prefs.clientId = [...crypto.getRandomValues(new Uint8Array(20))].map((b) => abc[b % abc.length]).join("");
    savePrefs();
  }
  return prefs.clientId;
}
// only a relay on the web (https), or one on this PC while testing
const relayOk = (u) => /^https:\/\/[^/\s]+$/i.test(u) || /^http:\/\/(127\.0\.0\.1|localhost)(:\d+)?$/i.test(u);

// The host's start and end times win while listening along (js/trims.js asks here first)
function hostTrimOf(t) {
  return together.role === "listener" && t && together.trims[t.id] !== undefined ? together.trims[t.id] : undefined;
}

const snapOf = (t) => Object.assign(xplSnap(t), trimOf(t) ? { trim: trimOf(t) } : {}, t.requestedBy ? { requestedBy: t.requestedBy } : {});

// The title bar's button: Start a Jam, your Jam (who's in, the invite, End), or Leave when you've
// joined someone. The bar in the queue shows only while you're listening along with someone; who's
// listening along with you isn't shown.
function paintTogether() {
  const jamBtn = $("jamBtn");
  const listening = together.role === "listener";
  const hostingJam = together.role === "host" && together.kind === "jam";
  jamBtn.hidden = (!relayUrl() && !listening) || (typeof jamAsk !== "undefined" && !!jamAsk) || !togetherOn();
  // the title bar says it all: your Jam (who's in, who's waiting), the one you're in, or your own
  // request waiting for an answer
  const waitingHere = listening && together.waiting;
  const asking = typeof myAsk !== "undefined" && myAsk && !listening;
  const label = waitingHere ? "Waiting for " + together.hostName + "..." : asking ? "Asked " + myAsk.host + "..."
    : listening ? (together.kind === "jam" ? "Leave Jam" : "Leave Listen Along")
    : hostingJam ? "Jam" + (together.members.length ? " \u00b7 " + together.members.length : "") : "Start a Jam";
  const live = (hostingJam || (listening && together.kind === "jam")) && !waitingHere;
  jamBtn.replaceChildren(...[
    live ? (() => { const b = el("span", "jamBars"); b.append(el("i"), el("i"), el("i")); return b; })() : null,
    el("span", "jamText", label),
    hostingJam && together.joinAsks.length ? el("span", "jamWait", together.joinAsks.length + " waiting") : null,
  ].filter(Boolean));
  jamBtn.title = hostingJam ? (together.joinAsks.length ? "People are waiting to join: click to let them in" : "Your Jam: the invite, and who's in")
    : waitingHere || asking ? "Your request is waiting for an answer" : listening ? "Leave and go back to your own music" : "Listen together: an invite-only Jam";
  jamBtn.classList.toggle("asks", hostingJam && together.joinAsks.length > 0);
  jamBtn.classList.toggle("on", (hostingJam || listening) && !waitingHere);
  jamBtn.classList.toggle("live", live);
  jamBtn.classList.toggle("pendingAsk", !!(waitingHere || asking));
  if (hostingJam && $("jamDialog").open) { $("jamMembers").textContent = together.members.length ? together.members.join(", ") : "Nobody has joined yet."; jamWaitPaint(); }
  $("togetherBar").hidden = !listening;
  // the queue is the host's: nothing to clear, no autoplay of your own
  $("clearQueue").hidden = listening;
  document.querySelectorAll(".qAutoHead").forEach((x) => { x.hidden = listening; });
  if (listening) {
    $("togetherText").textContent = (together.kind === "jam" ? "In " + together.hostName + "'s Jam" : "Listening along with " + together.hostName) +
      (together.members.length > 1 ? " · " + together.members.length + " listening" : "");
  }
}

// ---------- a connection to a room ----------
// (refused: called if the relay turns it away before it ever opens, instead of trying again)
function roomConnect(url, onMessage, refused) {
  const ws = new WebSocket(url);
  together.ws = ws;
  let opened = false;
  ws.onopen = () => { opened = true; };
  ws.onmessage = (e) => { let m; try { m = JSON.parse(e.data); } catch (x) { return; } onMessage(m); };
  ws.onclose = () => {
    if (together.ws !== ws || together.closing) return;
    if (!opened && refused) { refused(); return; }
    // dropped: try again in a few seconds, while still in the room
    setTimeout(() => { if (together.ws === ws && !together.closing && together.role) roomConnect(url, onMessage); }, 4000);
  };
  return ws;
}

// ---------- hosting ----------
// A new room on the relay
// (admit: app ids let into a Jam already)
async function roomCreate(kind, admit) {
  const res = await fetch(relayUrl() + "/api/rooms", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ kind, host: myName(), admit: admit || [] }) });
  if (!res.ok) throw new Error("the relay answered " + res.status);
  return res.json();
}

// (made: a room made already, by roomCreate)
async function hostOpen(kind, made) {
  const relay = relayUrl();
  if (together.role) await togetherEnd(true);
  const r = made || await roomCreate(kind);
  Object.assign(together, { role: "host", kind, code: r.code, hostKey: r.hostKey, inviteKey: r.inviteKey || "", joinUrl: r.joinUrl, relay, members: [], closing: false });
  roomConnect(wsBase(relay) + "/ws/" + r.code + "?role=host&key=" + encodeURIComponent(r.hostKey), hostMessage);
  together.ws.addEventListener("open", () => hostPush(true));
  paintTogether();
  return r;
}

function hostMessage(m) {
  if (m.type === "members") { together.members = m.list || []; paintTogether(); }
  else if (m.type === "add" && together.kind === "jam" && m.track) jamAdded(m.track, m.by, m.next);
  else if (m.type === "move" && together.kind === "jam" && m.id) {
    // a member moving a song: the same place among what's coming up, for everyone
    const from = queue.findIndex((x, k) => k > qi && x.id === m.id);
    const to = Math.max(qi + 1, Math.min(queue.length - 1, qi + (Number(m.to) || 1)));
    if (from > qi && from !== to) {
      const [moved] = queue.splice(from, 1);
      queue.splice(to, 0, moved);
      renderQueue();
      say((m.by || "Someone") + ' moved "' + (moved.title || "a song") + '" in the Jam.');
      hostPush(true);
    }
  }
  else if (m.type === "skip" && together.kind === "jam") {
    togetherOrig.skipPress(m.d === -1 ? -1 : 1);
    say((m.by || "Someone") + (m.d === -1 ? " went back a song." : " skipped the song."));
  }
  else if (m.type === "ask" && m.id) jamAsked(m.id, m.name || "Someone", m.cid || "");
  else if (m.type === "joinAsk" && m.id) jamJoinAsked(m.id, m.name || "Someone", m.cid || "");
  else if (m.type === "askGone") {
    if (jamAsk && jamAsk.id === m.id) jamAskClose();
    together.joinAsks = together.joinAsks.filter((x) => x.id !== m.id);
    paintTogether();
  }
}

// ---------- asks to join your Jam: each person once, waiting until you let them in or not ----------
function jamJoinAsked(id, name, cid) {
  if (!togetherOn()) { try { together.ws.send(JSON.stringify({ type: "admit", id, ok: false })); } catch (e) {} return; }
  const same = together.joinAsks.find((x) => (cid && x.cid === cid) || x.id === id);
  if (same) { same.id = id; return; }   // (the same person again: once in the list, at their newest ask)
  together.joinAsks.push({ id, name, cid });
  paintTogether();
  // (the title bar's Jam button says so: "1 waiting")
}

function jamWaitPaint() {
  const box = $("jamWaiting");
  box.replaceChildren(...together.joinAsks.map((x) => {
    const row = el("div", "jamWaitRow");
    const yes = el("button", "soft fixSave", "Let In"), no = el("button", "soft", "Decline");
    yes.addEventListener("click", () => jamAdmit(x, true));
    no.addEventListener("click", () => jamAdmit(x, false));
    row.append(el("span", "who", x.name), yes, no);
    return row;
  }));
  $("jamWaitBox").hidden = !together.joinAsks.length;
}

function jamAdmit(x, ok) {
  try { together.ws.send(JSON.stringify({ type: "admit", id: x.id, ok, url: together.joinUrl })); } catch (e) {}
  together.joinAsks = together.joinAsks.filter((y) => y !== x);
  paintTogether();
  jamWaitPaint();
  if (ok) say(x.name + " is joining your Jam.", { force: true });
}

// ---------- Ask to Jam (the Discord button): someone asks you to start a Jam ----------
// In the Start a Jam button's place: "<name> wants you to Start a Jam", Accept or Decline, for a minute
// (a ring empties as it runs out); then it's declined and the button is back. Accepting starts the
// Jam and joins them to it.
const JAM_ASK_MS = 60000;
let jamAsk = null;   // { id, name, timer }

// (busy: "not right now", which isn't a decline: they can ask again later)
function jamAskReply(id, ok, url, busy) {
  try { together.ws.send(JSON.stringify({ type: "askReply", id, ok, url: url || "", busy: !!busy })); } catch (e) {}
}

function jamAsked(id, name, cid) {
  if (!togetherOn()) { jamAskReply(id, false); return; }
  // the same person again: the one prompt they have already
  if (jamAsk && cid && jamAsk.cid === cid) { jamAsk.id = id; return; }
  // one at a time: anyone else asking meanwhile is told you're busy
  if (jamAsk || (together.role === "host" && together.kind === "jam")) { jamAskReply(id, false, "", true); return; }
  jamAsk = { id, name, cid, timer: setTimeout(() => { jamAskReply(id, false); jamAskClose(); }, JAM_ASK_MS) };
  $("jamAskText").textContent = name + " wants you to Start a Jam";
  const ring = $("askRingLeft");
  ring.classList.remove("run");
  void ring.getBoundingClientRect();
  ring.classList.add("run");
  $("jamAsk").hidden = false;
  $("jamBtn").hidden = true;
}

function jamAskClose() {
  if (jamAsk) clearTimeout(jamAsk.timer);
  jamAsk = null;
  $("jamAsk").hidden = true;
  $("askRingLeft").classList.remove("run");
  paintTogether();
}

async function jamAskAccept() {
  if (!jamAsk) return;
  const { id, name, cid } = jamAsk;
  jamAskClose();
  try {
    // the Jam first (with them let in already), so they're sent its invite on the room they asked
    // in, then the move to it
    const r = await roomCreate("jam", cid ? [cid] : []);
    jamAskReply(id, true, r.joinUrl);
    await new Promise((res) => setTimeout(res, 400));
    await hostOpen("jam", r);
    say("Jam started with " + name + ". Invite more friends from the Jam button.", { force: true });
  } catch (e) {
    jamAskReply(id, false);
    say("Couldn't start a Jam: " + (e && e.message ? e.message : e), { force: true });
  }
}

// Asking someone (their Discord status's Ask to Jam button opened omnipulse://ask/<code>)
// A request of yours that's still waiting (to start a Jam, or to be let into one): while it is, no other
// is sent
let myAsk = null;   // { host, what }
const askPendingSay = () => say("Your request " + myAsk.what + " is still waiting. You'll hear back here.", { force: true });

async function jamAskSend(code, relay) {
  if (!togetherOn()) { say("Listen Along & Jams are switched off (Settings > App & Look).", { force: true }); return; }
  if (myAsk) { askPendingSay(); return; }
  let info = null;
  try { info = await (await fetch(relay + "/api/rooms/" + code)).json(); } catch (e) {}
  if (!info || info.error) { say("They're not taking requests right now."); return; }
  const host = info.host || "They";
  const join = info.kind === "jam";
  if (together.role === "listener" && together.code === code) { say("You're in " + host + "'s Jam already."); return; }
  myAsk = { host, what: join ? "to join " + host + "'s Jam" : "for " + host + " to start a Jam" };
  const ws = new WebSocket(wsBase(relay) + "/ws/" + code + "?role=asker&name=" + encodeURIComponent(myName()) + "&cid=" + myCid());
  let answered = false, opened = false;
  const done = (note) => { if (answered) return; answered = true; myAsk = null; paintTogether(); if (note) say(note, { force: true }); };
  ws.onopen = () => { opened = true; };
  paintTogether();   // (the title bar says it's waiting)
  ws.onmessage = (e) => {
    let m; try { m = JSON.parse(e.data); } catch (x) { return; }
    if (m.type === "declined") { done(host + " didn't let you in this time."); return; }
    if (m.type === "admitted") m = { type: "askReply", ok: true, url: m.url };
    if (m.type !== "askReply") return;
    if (!m.ok) { done(m.busy ? host + " is answering someone else right now. Try again in a minute." : host + " didn't start a Jam this time."); return; }
    done("");
    if (m.ok && m.url) {
      // the Jam's invite (its web page's address), as the app link that page would open
      say("", { force: true });
      let link = m.url;
      try { const w = new URL(m.url); link = "omnipulse:/" + w.pathname + "?k=" + encodeURIComponent(w.searchParams.get("k") || "") + "&relay=" + encodeURIComponent(w.origin); } catch (x) {}
      togetherLink(link, true);
    }
  };
  // (refused before it was even asked: they declined you before, or there are too many asking)
  ws.onclose = () => done(opened ? host + " didn't answer." : "Couldn't ask " + host + ": they may have declined you already, or have too many requests waiting.");
  // a Jam's host has as long as they like; starting one, a minute
  if (!join) setTimeout(() => { if (!answered) { try { ws.close(); } catch (e) {} done(host + " didn't answer."); } }, JAM_ASK_MS + 10000);
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
function jamAdded(s, by, next) {
  const t = Object.assign({}, s, { user: Object.assign({}, s.user), requestedBy: (by || "Someone") + " (Jam)" });
  delete t.trim;
  if (next) togetherOrig.playNext(t); else addToQueueHere(t);
  say((by || "Someone") + (next ? ' put "' + (t.title || "a song") + '" on next.' : ' added "' + (t.title || "a song") + '" to the Jam.'));
  hostPush(true);
}

// Listen Along for the Discord buttons: a room while you're playing with Listen along or Ask to Jam on
async function togetherEnsureListen() {
  const want = !!(togetherOn() && relayUrl() && prefs.presence && (prefs.presListen !== false || prefs.presAsk) && nowPlaying() && together.role !== "listener");
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
  Object.assign(together, { role: null, kind: null, code: "", hostKey: "", inviteKey: "", joinUrl: "", ws: null, members: [], joinAsks: [], waiting: false, url: "", state: null, trims: {}, picks: {}, canAdd: false, localPause: false });
  if (was === "listener") myAsk = null;
  paintTogether();
  if (!quiet) say(was === "host" ? "Ended. Listeners carry on with their own music." : "You left. Back to your own queue.");
  if (was === "host") presenceUpdate();
}

// ---------- listening along ----------
// (asked: the Jam you asked for, so you're not asked again whether to join)
async function togetherLink(link, asked) {
  let u;
  try { u = new URL(link); } catch (e) { return; }
  const kind = u.host === "jam" ? "jam" : u.host === "join" ? "listen" : null;
  const code = u.pathname.replace(/^\//, "");
  const relay = (u.searchParams.get("relay") || relayUrl()).replace(/\/+$/, "");
  if (u.host === "ask") {
    if (/^[a-z0-9]{8}$/.test(code) && relayOk(relay)) await jamAskSend(code, relay); else say("That link isn't an OmniPulse invite.");
    return;
  }
  if (u.host === "playlist") {
    if (/^[a-z0-9]{8}$/.test(code) && relayOk(relay)) await shareOpen(code, relay); else say("That link isn't an OmniPulse playlist.");
    return;
  }
  if (!kind || !/^[a-z0-9]{8}$/.test(code) || !relayOk(relay)) { say("That link isn't an OmniPulse invite."); return; }
  if (!togetherOn()) { say("Listen Along & Jams are switched off (Settings > App & Look).", { force: true }); return; }
  let info = null;
  try { info = await (await fetch(relay + "/api/rooms/" + code)).json(); } catch (e) {}
  if (!info || info.error) { say("That session has ended."); return; }
  const more = kind === "jam" ? " You can add songs and skip them too." : " You'll hear what they play, as they play it.";
  if (myAsk) { askPendingSay(); return; }
  if (together.role === "listener" && together.code === code) { say(together.waiting ? "Your request to join " + info.host + "'s Jam is still waiting." : "You're in " + info.host + "'s Jam already."); return; }
  if (together.role) await togetherEnd(true);
  together.saved = { queue: queue.slice(), qi, listName: queueListName, wasPlaying: nowPlaying() };
  if (nowPlaying()) toggle();
  Object.assign(together, { role: "listener", kind, code, relay, hostName: info.host, closing: false, trims: {}, members: [] });
  const k = u.searchParams.get("k") || "";
  together.url = wsBase(relay) + "/ws/" + code + "?role=listener&name=" + encodeURIComponent(myName()) + "&cid=" + myCid() + (k ? "&k=" + encodeURIComponent(k) : "");
  roomConnect(together.url, listenerMessage, () => {
    // refused at the door: declined before, too many waiting, or the Jam's over
    togetherEnd(true).then(() => say("Couldn't join " + info.host + "'s " + (kind === "jam" ? "Jam" : "session") + ": they may have declined you already, or have too many requests waiting.", { force: true }));
  });
  paintTogether();
  say((kind === "jam" ? "Joining " + info.host + "'s Jam." : "Listening along with " + info.host + ".") + more + " Your own queue is set aside until you leave.", { force: true, actions: [["Leave", () => togetherEnd(false)]] });
}

function listenerMessage(m) {
  // a Jam: waiting for the host to let you in, then in (connected again), or not
  if (m.type === "waiting") {
    together.waiting = true;
    myAsk = { host: together.hostName, what: "to join " + together.hostName + "'s Jam" };
    paintTogether();   // (the title bar says it's waiting)
    return;
  }
  if (m.type === "admitted") {
    together.waiting = false;
    myAsk = null;
    say("", { force: true });
    if (together.ws) { const old = together.ws; together.ws = null; try { old.close(); } catch (e) {} }
    roomConnect(together.url, listenerMessage);
    return;
  }
  if (m.type === "declined") {
    myAsk = null;
    const host = together.hostName;
    togetherEnd(true).then(() => say(host + " didn't let you in this time.", { force: true }));
    return;
  }
  if (m.type === "hello") { together.hostName = m.host || together.hostName; together.canAdd = !!m.canAdd; together.waiting = false; paintTogether(); }
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
  listenerPrepare(s.next);
  const cur = curTrack();
  const onTarget = cur && (cur.id === target.id || (cur.standInFor && cur.standInFor.id === target.id));
  if (!onTarget) {
    if (cur && cur.standInFor && nowPlaying()) fadeTo(0, FADE_SKIP_MS);   // the stand-in bows out gently
    queue = [freshCopy(target)].concat(upcoming);
    qi = 0;
    queueListName = together.hostName + (together.kind === "jam" ? "'s Jam" : " (Listen Along)");
    together.localPause = false;
    if (!canPlayHere(target)) {
      // nothing here can play it: a stand-in of the same length (the queue shows the Jam's track)
      playStandIn(target, Math.max(0, ((target.trim && target.trim.end) || target.duration || 0) - hostPos()) || hostLenMs(target));
      renderQueue();
      return;
    }
    pendingResume = { id: target.id, posMs: Math.max(0, hostPos()) };
    const late = hostPos() > 2000;
    const started = togetherOrig.playCurrent();
    // joining partway (or this one loading slower than the host's): it fades in, where the host is
    if (late) fadeState.forceIn = Math.max(fadeState.forceIn, 1500);
    started.then(() => {
      if (!s.playing) { if (nowPlaying()) togetherOrig.toggle(); return; }
      // the host always wins: what loading took (Spotify's start especially) is caught up straight away
      const t = curTrack();
      if (t && t.id === target.id && together.role === "listener" && Math.abs(hostPos() - curPosMs()) > 1500) fadeJump(1500, () => seekToMs(hostPos()));
    });
    renderQueue();
    return;
  }
  queue = [cur].concat(upcoming);
  qi = 0;
  renderQueue();
  if (together.localPause) return;   // paused here on purpose: it catches up when played again
  if (s.playing && !nowPlaying()) togetherOrig.toggle();
  else if (!s.playing && nowPlaying()) togetherOrig.toggle();
  if (s.playing && !cur.standInFor) {
    const want = hostPos(), have = curPosMs();
    if (Math.abs(want - have) > 3000) fadeJump(800, () => seekToMs(hostPos()));   // drifted: back in step, smoothly
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

// ---------- what a listener can't play: a stand-in of the same length ----------
// Whether this app can play a track as it is (its own account, file or plugin)
function canPlayDirect(t) {
  const src = srcOf(t);
  if (src === "local") return localTracks.some((x) => x.id === t.id);
  if (src === "spotify") return spotifyPremium && !t.spGone;
  if (src === "soundcloud") return meId !== null || ytAvailable();   // (signed out: through yt-dlp)
  if (src === "youtube" || src === "musicbrainz") return ytOn();
  if (src.startsWith("plug:")) return pluginRuns.has(src.slice(5));
  return false;
}
// ...or at all (YouTube Music can stand in for the rest, if it finds the song)
const canPlayHere = (t) => canPlayDirect(t) || ytOn();

// How long the host will play a track: its own start and end times count (the length as it plays)
const hostLenMs = (s) => Math.max(0, ((s.trim && s.trim.end) || s.duration || 0) - ((s.trim && s.trim.start) || 0));

// A track of your own about as long as the time to fill: the closest that's at least as long, or else
// the longest there is. From your likes, your files and your own queue (set aside while listening).
function pickStandIn(ms, avoid) {
  const pool = [].concat(
    (together.saved && together.saved.queue) || [], localTracks, likedTracks.filter(isLikedTrack),
    (typeof spLikedCache !== "undefined" && spLikedCache) || [], (typeof ytLikes !== "undefined" && ytLikes) || [],
  ).filter((t) => t && t.duration > 30000 && !isMix(t) && canPlayDirect(t) && !(avoid || []).includes(t.id));
  if (!pool.length) return null;
  const longer = pool.filter((t) => t.duration >= ms).sort((a, b) => a.duration - b.duration);
  if (longer.length) return longer[Math.floor(Math.random() * Math.min(3, longer.length))];   // one of the closest, so it varies
  return pool.sort((a, b) => b.duration - a.duration)[0];
}

// Plays a stand-in for the Jam's track: the queue still shows the Jam's (greyed), and a note says why
function playStandIn(hostTrack, ms) {
  const pick = (together.picks[hostTrack.id] && canPlayDirect(together.picks[hostTrack.id]) && together.picks[hostTrack.id]) || pickStandIn(ms, [hostTrack.id]);
  delete together.picks[hostTrack.id];
  if (!pick) {
    say('"' + (hostTrack.title || "This song") + "\" isn't available on your end, and there's nothing of yours to play meanwhile.");
    return false;
  }
  const t = Object.assign({}, pick, { user: Object.assign({}, pick.user), standInFor: hostTrack, autoplay: undefined, requestedBy: undefined });
  queue[qi] = t;
  pendingResume = null;
  together.trims[t.id] = null;   // the stand-in plays from its start
  togetherOrig.playCurrent().then(() => {
    say('"' + (hostTrack.title || "This song") + '" isn\'t available on your end, so "' + (t.title || "a song of yours") + '" plays in its place until the ' + (together.kind === "jam" ? "Jam" : "host") + " moves on.");
  });
  return true;
}

// Something a listener can't play directly: YouTube Music, or a stand-in of the same length
async function listenerFallback(t, why) {
  const mine = streamToken;
  if (ytOn() && await playViaYouTube(t, mine, "Playing this one from YouTube Music (" + why + ").")) return true;
  if (mine !== streamToken) return true;
  const left = Math.max(0, ((t.trim && t.trim.end) || (together.trims[t.id] && together.trims[t.id].end) || t.duration || 0) - hostPos());
  return playStandIn(t, left || hostLenMs(t));
}

// Ahead of time: a stand-in ready for each of the next few the listener can't play
function listenerPrepare(next) {
  (next || []).slice(0, 3).forEach((s) => {
    if (canPlayHere(s) || together.picks[s.id]) return;
    const pick = pickStandIn(hostLenMs(s), [s.id]);
    if (pick) together.picks[s.id] = pick;
  });
}

// ---------- while listening along, the queue is the host's ----------
const togetherOrig = {};
function togetherGuard() {
  ["startList", "step", "skipPress", "playNext", "playFromPage", "addToQueue", "playCurrent", "onEnded", "toggle"].forEach((n) => { togetherOrig[n] = window[n]; });
  const send = (m) => { try { together.ws.send(JSON.stringify(m)); return true; } catch (e) { say("Couldn't reach the Jam."); return false; } };
  const askLeave = () => confirm("Leave listening along to play this? You'll go back to your own music.");
  const guard = (n) => function (...args) {
    if (together.role !== "listener") return togetherOrig[n].apply(this, args);
    if (n === "addToQueue" || n === "playNext") {
      if (!args[0]) return;
      if (!together.canAdd) { say("You're listening along with " + together.hostName + ": only a Jam takes songs from listeners.", { force: true }); return; }
      if (send({ type: "add", track: snapOf(args[0]), next: n === "playNext" })) say('Sent "' + (args[0].title || "the song") + '" to the Jam' + (n === "playNext" ? ", to play next." : "."));
      return;
    }
    if (n === "skipPress") {
      if (together.kind !== "jam") { say(together.hostName + " picks what plays while you listen along.", { force: true }); return; }
      if (send({ type: "skip", d: args[0] === -1 ? -1 : 1 })) say(args[0] === -1 ? "Going back a song for the Jam..." : "Skipping for the Jam...");
      return;
    }
    // the player moving itself on (a song that ended, or couldn't play): the host decides what's next
    if (n === "onEnded" || n === "step") return;
    // playing a song (from a page or a list): in a Jam it goes to the host, to play next; listening
    // along, you're asked first (leaving is what playing it here would take)
    if (n === "playFromPage" || n === "startList") {
      const t = n === "playFromPage" ? viewTracks[args[0]] : (args[0] || [])[args[2] ? 0 : args[1] || 0];
      if (together.canAdd && t && !args[2]) {
        if (send({ type: "add", track: snapOf(t), next: true })) say('Sent "' + (t.title || "the song") + '" to the Jam, to play next.');
        return;
      }
      const self = this;
      say((together.kind === "jam" ? "You're in " + together.hostName + "'s Jam" : "You're listening along with " + together.hostName) + ": playing this leaves it.", { force: true, actions: [["Leave and play", () => togetherEnd(true).then(() => togetherOrig[n].apply(self, args))]] });
      return;
    }
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
  ["startList", "step", "skipPress", "playNext", "playFromPage", "addToQueue", "onEnded", "toggle"].forEach((n) => { window[n] = guard(n); });
}
// A listener moving a song in the Jam's queue: asked of the host (who moves it for everyone); it moves
// here at once too, until the host's next word
function jamMoveAsk(from, to) {
  if (together.kind !== "jam") { say(together.hostName + " picks the order while you listen along.", { force: true }); return; }
  const t = queue[from];
  if (!t) return;
  try { together.ws.send(JSON.stringify({ type: "move", id: t.id, to: to - qi })); } catch (e) { say("Couldn't reach the Jam."); return; }
  const [moved] = queue.splice(from, 1);
  queue.splice(to, 0, moved);
  renderQueue();
}

// the host's own adds (from a Jam member) go straight in
const addToQueueHere = (t) => togetherOrig.addToQueue(t);

function togetherInit() {
  togetherGuard();
  $("togetherName").value = prefs.togetherName || "";
  $("togetherName").placeholder = myName();
  $("togetherName").addEventListener("change", () => { prefs.togetherName = $("togetherName").value.trim(); savePrefs(); });
  $("jamsOn").checked = togetherOn();
  $("jamsOn").addEventListener("change", async () => {
    prefs.jamsOn = $("jamsOn").checked;
    savePrefs();
    // off: out of whatever's going on, and any ask waiting declined
    if (!prefs.jamsOn) {
      if (jamAsk) { jamAskReply(jamAsk.id, false); jamAskClose(); }
      if (together.role) await togetherEnd(true);
    }
    paintTogether();
    presenceUpdate();
  });
  $("jamAskYes").addEventListener("click", jamAskAccept);
  $("jamAskNo").addEventListener("click", () => { if (jamAsk) jamAskReply(jamAsk.id, false); jamAskClose(); });
  $("jamBtn").addEventListener("click", () => {
    if (together.role === "listener") togetherEnd(false);
    else if (together.role === "host" && together.kind === "jam") jamDialog();
    else jamStart();
  });
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
