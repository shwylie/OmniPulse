// Autoplay: when the queue is about to run out, add similar tracks from every active platform,
// taking turns between them (a SoundCloud track, then Spotify, then SoundCloud...). They're kept
// apart from your own queue: the queue panel shows Next up (yours) and then Autoplay, and
// anything you or chat add goes before the autoplay picks.

const AUTOPLAY_BATCH = 10;
const AUTOPLAY_LEAD_MS = 30000;          // start looking this long before the last track ends
const AUTOPLAY_RECENT_MS = 7 * 86400000;     // "recently played": the last week
const AUTOPLAY_RECENT_MIN_MS = 3 * 3600000;  // if that leaves nothing to add, just the last three hours
let autoplayRecentMs = AUTOPLAY_RECENT_MS;

let autoplayBusy = null;
let autoplayFailedFor = null;   // the seed track we last came up empty for, so we don't keep asking

const autoplayOn = () => prefs.autoplay !== false;
const autoplayActive = () => autoplayOn() && repeatMode === "off";
const autoplaySets = () => !!prefs.queueSets;           // sets & podcasts in the queue and autoplay (one setting; off unless chosen)
const avoidRecent = () => prefs.avoidRecent !== false;  // on unless switched off

// Where "Add to queue" puts a track: after everything you queued, before the autoplay picks
function autoplayStart() {
  for (let i = Math.max(qi + 1, 0); i < queue.length; i++) if (queue[i].autoplay) return i;
  return queue.length;
}

function autoplayNote() {
  if (!autoplayOn()) return "Off: playback stops at the end of your queue.";
  if (repeatMode !== "off") return "Waiting while repeat is on.";
  return autoplayBusy ? "Finding similar tracks..." : "Similar tracks will be added near the end of the queue.";
}

function autoplayPaint(msg) {
  $("autoplayOn").checked = autoplayOn();
  $("apQueueOn").checked = autoplayOn();
  $("autoplayNote").textContent = msg !== undefined ? msg : autoplayNote();
}

// ---------- candidates from each platform, all seeded by the current track ----------
function shuffled(list) { return shuffleInPlace(list.slice()); }
const seedArtist = (seed) => (srcOf(seed) === "spotify" ? firstArtist(seed) : guessNames(seed).artist) || artistName(seed);

async function apSoundCloud(seed) {
  const related = (id) => api("/tracks/" + id + "/related?access=playable&limit=30&linked_partitioning=true").then(asList).catch(() => []);
  const ids = [];
  if (srcOf(seed) === "soundcloud") ids.push(seed.id);
  const earlier = queue.slice(0, qi).filter((t) => srcOf(t) === "soundcloud" && t.id !== seed.id);
  if (earlier.length) ids.push(shuffled(earlier)[0].id);
  if (!ids.length) {
    // the seed is from another platform: find it (or its artist) on SoundCloud first
    const found = await api("/tracks?q=" + encodeURIComponent(seedArtist(seed) + " " + guessNames(seed).title) +
      "&access=playable&limit=5&linked_partitioning=true").then(asList).catch(() => []);
    if (found[0]) ids.push(found[0].id);
  }
  return shuffled((await Promise.all(ids.slice(0, 2).map(related))).flat());
}

async function apSpotify(seed) {
  const artist = seedArtist(seed);
  if (!artist) return [];
  const names = shuffled(await lfmSimilarNames(artist, 10)).slice(0, 4);
  return shuffled((await Promise.all((names.length ? names : [artist]).map(spArtistTracks))).flat());
}

async function apLocal(seed) {
  if (!localTracks.length) return [];
  const who = seedArtist(seed).toLowerCase();
  const album = (seed.album || "").trim();
  const sameArtist = localTracks.filter((t) => artistName(t).toLowerCase() === who);
  const sameAlbum = album ? localTracks.filter((t) => (t.album || "").trim() === album) : [];
  return shuffled(sameArtist).concat(shuffled(sameAlbum), shuffled(localTracks));
}

// (YouTube's is in js/youtube.js, which loads later: looked up when it's used)
const AUTOPLAY_SOURCES = { soundcloud: apSoundCloud, spotify: apSpotify, local: apLocal, youtube: (seed) => apYouTube(seed) };

// What's worth adding: not queued, not removed, not heard lately (unless that's switched off),
// sets & podcasts only if you've asked for them, and at most two per artist in a batch
function autoplayPickable(taken, perArtist) {
  return (t) => {
    if (!t || taken.has(t.id) || !recOk(t)) return false;
    if (isMix(t) && !autoplaySets()) return false;
    if (avoidRecent() && heardWithin(t, autoplayRecentMs)) return false;
    if (!langOk(t)) return false;
    return (perArtist.get(artistName(t).toLowerCase()) || 0) < 2;
  };
}

// Takes turns between the platforms' lists, starting with one other than the queue's last track.
// A platform isn't used twice in a row unless it's the only one with anything to add.
function autoplayTakeTurns(lists, lastPlat) {
  // a week of plays can rule out everything similar: then only the last few hours count
  const copy = {};
  Object.keys(lists).forEach((p) => { copy[p] = lists[p].slice(); });
  autoplayRecentMs = AUTOPLAY_RECENT_MS;
  let out = autoplayTakeTurnsOnce(lists, lastPlat);
  if (!out.length && avoidRecent()) {
    autoplayRecentMs = AUTOPLAY_RECENT_MIN_MS;
    out = autoplayTakeTurnsOnce(copy, lastPlat);
    autoplayRecentMs = AUTOPLAY_RECENT_MS;
  }
  return out;
}

function autoplayTakeTurnsOnce(lists, lastPlat) {
  const taken = new Set(queue.map((t) => t.id));
  const perArtist = new Map();
  const ok = autoplayPickable(taken, perArtist);
  let order = Object.keys(lists).filter((p) => lists[p].length);
  const several = order.length > 1;
  if (several && order[0] === lastPlat) order = order.slice(1).concat(order[0]);
  const out = [];
  while (out.length < AUTOPLAY_BATCH && order.length) {
    for (const p of order.slice()) {
      if (out.length >= AUTOPLAY_BATCH) break;
      const list = lists[p];
      let t = list.shift();
      while (t && !ok(t)) t = list.shift();
      if (!t) { order = order.filter((x) => x !== p); continue; }
      taken.add(t.id);
      const a = artistName(t).toLowerCase();
      perArtist.set(a, (perArtist.get(a) || 0) + 1);
      out.push(Object.assign({}, t, { autoplay: true, requestedBy: undefined }));
    }
    // once only one platform has tracks left, stop rather than play it twice in a row
    if (several && order.length === 1) break;
  }
  return out;
}

// Adds a batch of similar tracks after the queue. Safe to call often.
function autoplayFill() {
  if (autoplayBusy) return autoplayBusy;
  const seed = curTrack();
  if (!seed || !autoplayActive() || autoplayFailedFor === seed.id) return Promise.resolve();
  autoplayBusy = (async () => {
    await null;   // let autoplayBusy be set first, so the queue shows "Finding..."
    try {
      const plats = activePlats().filter((p) => AUTOPLAY_SOURCES[p]);
      const results = await Promise.allSettled(plats.map((p) => AUTOPLAY_SOURCES[p](seed)));
      if (curTrack() !== seed) return;   // you started something else meanwhile
      const lists = {};
      plats.forEach((p, i) => { lists[p] = results[i].status === "fulfilled" ? results[i].value.slice() : []; });
      const picks = autoplayTakeTurns(lists, srcOf(queue[queue.length - 1]));
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
  const dur = trimmedEnd(t, curDurMs());
  if (!dur || dur - curPosMs() <= AUTOPLAY_LEAD_MS) autoplayFill();
}

function setAutoplay(on) {
  prefs.autoplay = on;
  savePrefs();
  if (!on) queue = queue.filter((t, i) => i <= qi || !t.autoplay);   // take back the unplayed picks
  autoplayFailedFor = null;
  autoplayPaint();
  renderQueue();
}

function autoplayInit() {
  // the switch in the queue panel and the one in Settings are the same setting
  $("autoplayOn").addEventListener("change", () => setAutoplay($("autoplayOn").checked));
  $("apQueueOn").addEventListener("change", () => setAutoplay($("apQueueOn").checked));
  const sw = (id, key, dflt) => {
    $(id).checked = prefs[key] === undefined ? dflt : !!prefs[key];
    $(id).addEventListener("change", () => {
      prefs[key] = $(id).checked;
      savePrefs();
      if (key === "queueSets" || key === "avoidRecent") autoplayFailedFor = null;
    });
  };
  // there were two settings (queue, autoplay); either one on carries over
  if (prefs.autoplaySets !== undefined) { prefs.queueSets = !!(prefs.queueSets || prefs.autoplaySets); delete prefs.autoplaySets; savePrefs(); }
  sw("queueSets", "queueSets", false);
  sw("avoidRecentOn", "avoidRecent", true);
  autoplayPaint();
  setInterval(autoplayTick, 2000);
  setTimeout(lfmHeardSync, 8000);
  setInterval(lfmHeardSync, 5 * 60000);
}
