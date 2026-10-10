// Start and end times for a track (Track preferences, from a track's right-click menu).
//
// A track can be set to start later and end sooner than it does. It then plays only that part, and
// shows as that long everywhere you see a length or a position: the player bar, the progress bar and
// seeking, Now Playing, the OBS widget, Discord and the track lists. (A 1:00 track set to 0:10-0:50 is
// a 0:40 track.) Now Playing marks it with an asterisk, which opens Track preferences.
//
// In Track preferences the times are set on a timeline (with any silence the loudness measurement found
// shaded in), typed, or taken from where the track is playing; Preview plays the chosen start, or the
// last seconds up to the chosen end, and pauses there instead of moving on.

const PREVIEW_MS = 5000;
let trimTarget = null;      // the track in Track preferences
let trimDraft = null;       // { start, end } being edited, in ms (end null: the track's own end)
let trimPreview = null;     // { id, until }: a preview pauses at "until" instead of the track ending
let trimEndedAt = "";       // the track the end time just ended (so it's only done once)

// ---------- the times ----------
function trimOf(t) {
  const host = typeof hostTrimOf === "function" ? hostTrimOf(t) : undefined;
  if (host !== undefined) return host;   // listening along: the host's (js/together.js)
  const x = t && (prefs.trackTrims || {})[t.id];
  return x && (x.start > 0 || x.end > 0) ? x : null;
}
const trimStart = (t) => (trimOf(t) && trimOf(t).start) || 0;
const trimmedEnd = (t, rawDur) => (trimOf(t) && trimOf(t).end) || rawDur;

// What the screen shows: the position and length within the part that plays
const viewPos = (t, raw) => Math.max(0, raw - trimStart(t));
const viewDur = (t, rawDur) => Math.max(0, trimmedEnd(t, rawDur) - trimStart(t));
// From a fraction of what the screen shows, back to the track's own time
const rawAt = (t, frac, rawDur) => trimStart(t) + frac * viewDur(t, rawDur);

// m:ss (or m:ss.s) to ms; empty is null
function parseTime(s) {
  const v = String(s || "").trim();
  if (!v) return null;
  const parts = v.split(":").map(Number);
  if (parts.some((n) => !isFinite(n) || n < 0)) return NaN;
  return Math.round(parts.reduce((acc, n) => acc * 60 + n, 0) * 1000);
}
const fmtTenth = (ms) => {
  const s = Math.max(0, ms) / 1000;
  const m = Math.floor(s / 60);
  const rest = s - m * 60;
  return m + ":" + (rest < 10 ? "0" : "") + rest.toFixed(1).replace(/\.0$/, "");
};

// ---------- while it plays ----------
function trimTick() {
  const t = curTrack();
  if (!t || !nowPlaying()) return;
  const pos = curPosMs();
  if (trimPreview && trimPreview.id === t.id) {
    if (pos >= trimPreview.until) { trimPreview = null; toggle(); say("Preview done."); }
    return;
  }
  const tr = trimOf(t);
  if (!tr || !tr.end || trimEndedAt === t.id + ":" + qi) return;
  if (pos >= tr.end) {
    trimEndedAt = t.id + ":" + qi;
    say("Ended at " + fmt(tr.end) + ", the end time you set for this track.");
    onEnded();
  }
}

// Where a track should start playing (a resumed position wins if it's inside the part that plays)
function trimStartFor(t, resumeAt) {
  const s = trimStart(t);
  const tr = trimOf(t);
  if (resumeAt && resumeAt > s && (!tr || !tr.end || resumeAt < tr.end)) return resumeAt;
  trimEndedAt = "";
  return s;
}

// ---------- Track preferences ----------
// For tracks that play from YouTube Music (Go+, Spotify without Premium): which version, and a search
// to choose another
function ytPickPaint(t) {
  const wanted = viaYouTube(t) || (srcOf(t) === "spotify" ? !spotifyPremium : srcOf(t) === "soundcloud" && isPreview(t));
  $("ytPick").hidden = !wanted || !ytOn();
  if ($("ytPick").hidden) return;
  const m = ytMatchesLoad()[t.id];
  $("ytPickNote").textContent = m && m.id ? "Plays " + '"' + m.title + '" by ' + m.artists + (m.chosen ? " (your choice)." : " (found by itself).") + " Pick another below if it's the wrong one."
    : "No version found by itself" + (m && m.closest ? " (the closest was " + m.closest + ")" : "") + ". Search and pick one.";
  const n = guessNames(t);
  $("ytPickQ").value = (n.artist || artistName(t)) + " " + n.title;
  $("ytPickList").replaceChildren();
}

async function ytPickSearch() {
  const t = trimTarget;
  const box = $("ytPickList");
  box.replaceChildren(el("div", "note", "Searching..."));
  let ranked = [];
  const q = $("ytPickQ").value.trim();
  // a YouTube or YouTube Music link (or a video id): that video, whatever a search would say
  const id = ytLinkId(q);
  if (id) {
    try {
      const found = (await invoke("yt_radio", { id })).map(ytTrack).find((c) => c.ytId === id);
      ranked = found ? [{ c: found, score: 1 }] : [];
    } catch (e) { box.replaceChildren(el("div", "note warn", String(e))); return; }
    if (!ranked.length) { box.replaceChildren(el("div", "note warn", "That video couldn't be found on YouTube.")); return; }
  } else try { ranked = await ytCandidates(t, q); } catch (e) { box.replaceChildren(el("div", "note warn", String(e))); return; }
  box.replaceChildren(...ranked.slice(0, 8).map(({ c }) => {
    const b = el("button", "ytPickRow");
    b.type = "button";
    b.append(el("span", "ytPickTitle", c.title), el("span", "ytPickBy", artistName(c) + " \u00b7 " + fmt(c.duration || 0)));
    b.addEventListener("click", () => {
      ytChooseMatch(t, c);
      ytPickPaint(t);
      say("It'll play " + c.title + " from YouTube Music from now on.");
      if (curTrack() && curTrack().id === t.id) playCurrent();
    });
    return b;
  }));
  if (!ranked.length) box.append(el("div", "note", "Nothing found."));
}

function trimDialogOpen(t) {
  trimTarget = t;
  ytPickPaint(t);
  const tr = trimOf(t);
  trimDraft = { start: tr ? tr.start || 0 : 0, end: tr && tr.end ? tr.end : null };
  $("fixWhat").textContent = (t.title || "Untitled") + " · " + artistName(t);
  paintTrimDialog();
}

const trimDur = () => {
  const t = trimTarget;
  if (!t) return 0;
  return curTrack() && curTrack().id === t.id && curDurMs() ? curDurMs() : t.duration || 0;
};

function paintTrimDialog() {
  const t = trimTarget;
  if (!t) return;
  const dur = trimDur();
  const start = trimDraft.start || 0;
  const end = trimDraft.end || dur;
  $("trimStartIn").value = start ? fmtTenth(start) : "";
  $("trimEndIn").value = trimDraft.end ? fmtTenth(trimDraft.end) : "";
  $("trimEndIn").placeholder = dur ? fmtTenth(dur) : "end";
  const pct = (ms) => (dur ? Math.max(0, Math.min(100, (ms / dur) * 100)) : 0);
  $("trimSel").style.left = pct(start) + "%";
  $("trimSel").style.width = Math.max(0, pct(end) - pct(start)) + "%";
  $("trimStartH").style.left = pct(start) + "%";
  $("trimEndH").style.left = pct(end) + "%";
  // silence the loudness measurement found, shaded in
  const a = loudFor(t);
  $("trimSilence").replaceChildren(...((a && a.silences) || []).map(([from, to]) => {
    const s = el("span", "trimQuiet");
    s.style.left = pct(from) + "%";
    s.style.width = Math.max(0.5, pct(to) - pct(from)) + "%";
    s.title = "Silent from " + fmt(from) + " to " + fmt(to);
    return s;
  }));
  const playing = curTrack() && curTrack().id === t.id;
  $("trimStartNow").disabled = !playing;
  $("trimEndNow").disabled = !playing;
  const changed = start > 0 || !!trimDraft.end;
  $("trimNote").textContent = !dur ? "This track's length isn't known yet: play it once, or type the times." :
    end <= start ? "The end has to be after the start." :
    changed ? "Plays " + fmt(end - start) + " of " + fmt(dur) + " (" + fmt(start) + " to " + fmt(end) + ")." : "Plays the whole track (" + fmt(dur) + ").";
  paintTrimHead();
}

function paintTrimHead() {
  const t = trimTarget;
  const head = $("trimHead");
  const dur = trimDur();
  if (!t || !dur || !curTrack() || curTrack().id !== t.id) { head.hidden = true; return; }
  head.hidden = false;
  head.style.left = Math.min(100, (curPosMs() / dur) * 100) + "%";
}

// Dragging a handle (or clicking the timeline): moves the nearer end; with this track playing, the
// player follows so you hear where you are
function trimDrag(e) {
  const bar = $("trimBar");
  const dur = trimDur();
  if (!dur) return;
  const r = bar.getBoundingClientRect();
  const at = (x) => Math.round(Math.max(0, Math.min(1, (x - r.left) / r.width)) * dur);
  const ms = at(e.clientX);
  const end = trimDraft.end || dur;
  const which = e.target.id === "trimStartH" ? "start" : e.target.id === "trimEndH" ? "end"
    : Math.abs(ms - (trimDraft.start || 0)) <= Math.abs(ms - end) ? "start" : "end";
  const move = (x) => {
    const v = at(x);
    if (which === "start") trimDraft.start = Math.min(v, (trimDraft.end || dur) - 1000);
    else trimDraft.end = Math.max(v, (trimDraft.start || 0) + 1000) >= dur - 250 ? null : Math.max(v, (trimDraft.start || 0) + 1000);
    paintTrimDialog();
  };
  move(e.clientX);
  const up = () => {
    window.removeEventListener("pointermove", onMove);
    window.removeEventListener("pointerup", up);
    if (curTrack() && curTrack().id === trimTarget.id) trimSeekRaw(which === "start" ? trimDraft.start || 0 : Math.max(0, (trimDraft.end || dur) - PREVIEW_MS));
  };
  const onMove = (ev) => move(ev.clientX);
  window.addEventListener("pointermove", onMove);
  window.addEventListener("pointerup", up);
  e.preventDefault();
}

function trimSeekRaw(ms) {
  const t = curTrack();
  if (!t) return;
  if (onSpotifyEngine(t)) { if (spState.dur) spotifySeek(ms / spState.dur); }
  else audio.currentTime = ms / 1000;
}

// Preview: the start, or the last few seconds up to the end, then a pause
async function trimPreviewPlay(which) {
  const t = trimTarget;
  if (!t) return;
  if (!curTrack() || curTrack().id !== t.id) {
    playNext(t);
    step(1);
    const t0 = Date.now();
    while (!(nowPlaying() && curTrack() && curTrack().id === t.id) && Date.now() - t0 < 15000) await new Promise((r) => setTimeout(r, 150));
    if (!curTrack() || curTrack().id !== t.id) { say("Couldn't play this track to preview it."); return; }
  }
  const dur = trimDur();
  const end = trimDraft.end || dur;
  const from = which === "start" ? trimDraft.start || 0 : Math.max(trimDraft.start || 0, end - PREVIEW_MS);
  trimPreview = { id: t.id, until: which === "start" ? Math.min(end, from + PREVIEW_MS) : end };
  trimSeekRaw(from);
  if (!nowPlaying()) toggle();
  say("Previewing " + (which === "start" ? "the start" : "the end") + "...");
}

function trimFromInputs() {
  const s = parseTime($("trimStartIn").value);
  const e = parseTime($("trimEndIn").value);
  const dur = trimDur();
  if (Number.isNaN(s) || Number.isNaN(e)) return false;
  trimDraft.start = Math.max(0, s || 0);
  trimDraft.end = e && (!dur || e < dur) ? e : null;
  return true;
}

// Saved with the title and artist (js/titles.js saveFix); false if the times don't make sense
function trimDialogSave() {
  const t = trimTarget;
  if (!t) return true;
  if (!trimFromInputs()) { $("trimNote").textContent = "Write times like 1:05 or 0:42.5."; return false; }
  const dur = trimDur();
  const start = trimDraft.start || 0;
  const end = trimDraft.end;
  if (end && end <= start + 1000) { $("trimNote").textContent = "The end has to be at least a second after the start."; return false; }
  if (dur && start >= dur - 1000) { $("trimNote").textContent = "The start has to be before the end of the track."; return false; }
  prefs.trackTrims = prefs.trackTrims || {};
  if (start > 0 || end) prefs.trackTrims[t.id] = { start, end: end || null };
  else delete prefs.trackTrims[t.id];
  savePrefs();
  trimApplied(t);
  return true;
}

function trimReset(t) {
  if (prefs.trackTrims && prefs.trackTrims[t.id]) { delete prefs.trackTrims[t.id]; savePrefs(); trimApplied(t); }
}

// Everything that shows a length or position, after a change
function trimApplied(t) {
  trimPreview = null;
  const cur = curTrack();
  if (cur && cur.id === t.id) {
    fillNowPlaying(cur);
    const tr = trimOf(cur);
    const pos = curPosMs();
    if (tr && (pos < (tr.start || 0) - 500 || (tr.end && pos > tr.end))) trimSeekRaw(tr.start || 0);
    widgetPush();
    presenceUpdate();
  }
  render();
  renderQueue();
}

// The asterisk after a trimmed track's title (player bar and Now Playing): opens Track preferences
function trimMark(t) {
  const b = el("button", "trimMark", "*");
  b.type = "button";
  const tr = trimOf(t);
  b.title = "Plays " + fmt(tr.start || 0) + " to " + (tr.end ? fmt(tr.end) : "the end") + ". Click to change (Track preferences).";
  b.setAttribute("aria-label", b.title);
  b.addEventListener("click", (e) => { e.stopPropagation(); openFixDialog(t); });
  return b;
}

function trimsInit() {
  $("ytPickGo").addEventListener("click", ytPickSearch);
  $("ytPickQ").addEventListener("keydown", (e) => { if (e.key === "Enter") { e.preventDefault(); ytPickSearch(); } });
  setInterval(trimTick, 200);
  setInterval(() => { if ($("fixDialog").open) paintTrimHead(); }, 250);
  $("trimBar").addEventListener("pointerdown", trimDrag);
  $("trimStartIn").addEventListener("change", () => { if (trimFromInputs()) paintTrimDialog(); });
  $("trimEndIn").addEventListener("change", () => { if (trimFromInputs()) paintTrimDialog(); });
  $("trimStartNow").addEventListener("click", () => { trimDraft.start = Math.round(curPosMs()); paintTrimDialog(); });
  $("trimEndNow").addEventListener("click", () => { trimDraft.end = Math.round(curPosMs()); paintTrimDialog(); });
  $("trimPrevStart").addEventListener("click", () => trimPreviewPlay("start"));
  $("trimPrevEnd").addEventListener("click", () => trimPreviewPlay("end"));
  $("trimClear").addEventListener("click", () => { trimDraft = { start: 0, end: null }; paintTrimDialog(); });
}
