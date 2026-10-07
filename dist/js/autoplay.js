// Autoplay: when the queue is about to run out, add similar tracks so the music keeps going.
// Autoplay tracks sit at the end of the queue; anything you or chat add goes in front of them.

const AUTOPLAY_BATCH = 10;
const AUTOPLAY_LEAD_MS = 30000;          // start looking this long before the last track ends
const AUTOPLAY_RECENT_MS = 3 * 3600000;  // skip tracks played in the last few hours

let autoplayBusy = null;
let autoplayFailedFor = null;   // the seed track we last came up empty for, so we don't keep asking

const autoplayOn = () => prefs.autoplay !== false;
const autoplayActive = () => autoplayOn() && repeatMode === "off";

// Where "Add to queue" puts a track: after everything you queued, before the autoplay picks
function autoplayStart() {
  for (let i = Math.max(qi + 1, 0); i < queue.length; i++) if (queue[i].autoplay) return i;
  return queue.length;
}

function autoplayNote() {
  if (!autoplayOn()) return "Nothing queued after this.";
  if (repeatMode !== "off") return "Nothing queued after this. Autoplay waits while repeat is on.";
  return autoplayBusy ? "Finding similar tracks..." : "Autoplay will add similar tracks near the end of this one.";
}

function autoplayPaint(msg) {
  $("autoplayOn").checked = autoplayOn();
  $("autoplayNote").textContent = msg !== undefined ? msg
    : autoplayOn() ? "Keeps playing similar tracks when the queue runs out." : "Off: playback stops at the end of the queue.";
}

// ---------- finding candidates ----------
function shuffled(list) { return shuffleInPlace(list.slice()); }

async function autoplaySoundCloud(seed) {
  const related = (id) => api("/tracks/" + id + "/related?access=playable&limit=30&linked_partitioning=true").then(asList).catch(() => []);
  let list = await related(seed.id);
  // a thin result: also try something played earlier in this queue, for variety
  if (list.length < AUTOPLAY_BATCH) {
    const earlier = queue.slice(0, qi).filter((t) => srcOf(t) === "soundcloud" && t.id !== seed.id);
    if (earlier.length) list = list.concat(await related(shuffled(earlier)[0].id));
  }
  return list;
}

async function autoplaySpotify(seed) {
  const artist = firstArtist(seed);
  if (!artist) return [];
  const names = shuffled(await lfmSimilarNames(artist, 10)).slice(0, 4);
  const lists = await Promise.all((names.length ? names : [artist]).map(spArtistTracks));
  return shuffled(lists.flat());
}

function autoplayLocal(seed) {
  const who = artistName(seed).toLowerCase();
  const album = (seed.album || "").trim();
  const sameArtist = localTracks.filter((t) => artistName(t).toLowerCase() === who);
  const sameAlbum = album ? localTracks.filter((t) => (t.album || "").trim() === album) : [];
  return shuffled(sameArtist).concat(shuffled(sameAlbum), shuffled(localTracks));
}

// What's worth adding: not already queued, not removed, not played lately, and mixes only after a mix
function autoplayPick(seed, list) {
  const queued = new Set(queue.map((t) => t.id));
  const lastPlayed = new Map();
  const since = Date.now() - AUTOPLAY_RECENT_MS;
  loadStats().forEach((x) => { if (x.ts > since) lastPlayed.set(x.id, x.ts); });
  const perArtist = new Map();
  const out = [];
  for (const t of list) {
    if (out.length >= AUTOPLAY_BATCH) break;
    if (!t || queued.has(t.id) || hiddenRecs.has(String(t.id)) || lastPlayed.has(t.id)) continue;
    if (isMix(t) !== isMix(seed) || !langOk(t)) continue;
    const a = artistName(t).toLowerCase();
    if ((perArtist.get(a) || 0) >= 2) continue;   // a little variety
    perArtist.set(a, (perArtist.get(a) || 0) + 1);
    queued.add(t.id);
    out.push(Object.assign({}, t, { autoplay: true, requestedBy: undefined }));
  }
  return out;
}

// Adds a batch of similar tracks after the current one. Safe to call often.
function autoplayFill() {
  if (autoplayBusy) return autoplayBusy;
  const seed = curTrack();
  if (!seed || !autoplayActive() || autoplayFailedFor === seed.id) return Promise.resolve();
  autoplayBusy = (async () => {
    await null;   // let autoplayBusy be set first, so the queue shows "Finding..."
    try {
      const src = srcOf(seed);
      const list = src === "spotify" ? await autoplaySpotify(seed)
        : src === "local" ? autoplayLocal(seed)
        : await autoplaySoundCloud(seed);
      if (curTrack() !== seed) return;   // you started something else meanwhile
      const picks = autoplayPick(seed, list);
      if (!picks.length) {
        autoplayFailedFor = seed.id;
        autoplayPaint("Autoplay couldn't find anything similar to this track.");
        return;
      }
      queue.push(...picks);
      autoplayPaint();
    } catch (e) {
      autoplayFailedFor = seed.id;
      autoplayPaint("Autoplay couldn't load suggestions: " + (e && e.message ? e.message : e));
    } finally {
      autoplayBusy = null;
      renderQueue();
    }
  })();
  renderQueue();
  return autoplayBusy;
}

// The queue ran out (track ended, or Next on the last track): fill it, then move on
async function autoplayAdvance() {
  const seed = curTrack();
  await autoplayFill();
  if (curTrack() === seed && qi < queue.length - 1) { qi++; playCurrent(); return true; }
  return false;
}

// Checked every couple of seconds: is the last track close to its end?
function autoplayTick() {
  const t = curTrack();
  if (!t || !autoplayActive() || autoplayBusy || qi < queue.length - 1) return;
  const dur = curDurMs();
  if (!dur || dur - curPosMs() <= AUTOPLAY_LEAD_MS) autoplayFill();
}

function autoplayInit() {
  autoplayPaint();
  $("autoplayOn").addEventListener("change", () => {
    prefs.autoplay = $("autoplayOn").checked;
    savePrefs();
    if (!prefs.autoplay) {
      // take back the suggestions that haven't played yet
      queue = queue.filter((t, i) => i <= qi || !t.autoplay);
    }
    autoplayFailedFor = null;
    autoplayPaint();
    renderQueue();
  });
  setInterval(autoplayTick, 2000);
}
