// When each track was last heard (counted, like the stats: 30 s or half the track), kept between
// sessions. Autoplay uses it to avoid recent plays, and shuffle to put long-unheard tracks first.
// Only text is kept: track ids, "artist|title" keys and times, never the music itself.

const HISTORY_KEY = "sc.lastPlayed";

let lastPlayedMap = new Map();
try { lastPlayedMap = new Map(JSON.parse(localStorage.getItem(HISTORY_KEY) || "[]")); } catch (e) { lastPlayedMap = new Map(); }
// first run: start from the stats you already have
if (!lastPlayedMap.size) {
  try {
    JSON.parse(localStorage.getItem("sc.stats") || "[]").forEach((x) => {
      if (x && x.id != null && x.ts > (lastPlayedMap.get(x.id) || 0)) lastPlayedMap.set(x.id, x.ts);
    });
  } catch (e) {}
}

let historyTimer = null;
function historySave() {
  clearTimeout(historyTimer);
  historyTimer = setTimeout(() => {
    try { localStorage.setItem(HISTORY_KEY, JSON.stringify([...lastPlayedMap])); } catch (e) {}
  }, 1000);
}

// The same song by name ("artist|title"), so one heard on Spotify counts when it turns up on
// SoundCloud; the last month of it is kept
const HEARD_SONGS_KEY = "sc.heardSongs";
const HEARD_SONGS_DAYS = 31;
let heardSongs = new Map();
try { heardSongs = new Map(JSON.parse(localStorage.getItem(HEARD_SONGS_KEY) || "[]")); } catch (e) { heardSongs = new Map(); }
let heardSongsTimer = null;
function heardSongsSave() {
  clearTimeout(heardSongsTimer);
  heardSongsTimer = setTimeout(() => {
    try { localStorage.setItem(HEARD_SONGS_KEY, JSON.stringify([...heardSongs])); } catch (e) {}
  }, 1000);
}
// Once, as the app opens: entries older than a month go, one by one (the rest of the history stays).
// Nothing is trimmed while the app runs. Your Stats are kept separately and aren't touched.
(function historyTrim() {
  const cut = Date.now() - HEARD_SONGS_DAYS * 86400000;
  let gone = 0;
  for (const [k, ts] of heardSongs) if (ts < cut) { heardSongs.delete(k); gone++; }
  if (gone) { try { localStorage.setItem(HEARD_SONGS_KEY, JSON.stringify([...heardSongs])); } catch (e) {} }
  let goneIds = 0;
  for (const [id, ts] of lastPlayedMap) if (ts < cut) { lastPlayedMap.delete(id); goneIds++; }
  if (goneIds) { try { localStorage.setItem(HISTORY_KEY, JSON.stringify([...lastPlayedMap])); } catch (e) {} }
})();

function heardSong(key, ts) {
  if (key && ts > (heardSongs.get(key) || 0)) heardSongs.set(key, ts);
}

function markHeard(t) {
  if (!t || t.id == null) return;
  lastPlayedMap.set(t.id, Date.now());
  historySave();
  heardSong(songKey(t), Date.now());
  heardSongsSave();
}

// Avoid Recent Plays (Settings > Library): recommendations, Discover, Sets & Podcasts and autoplay
// skip what you've heard within the time frame chosen (a day, a week or a month)
const RECENT_WINDOWS = { day: 86400000, week: 7 * 86400000, month: 30 * 86400000 };
const avoidRecent = () => prefs.avoidRecent !== false;   // on unless switched off
const recentWindowMs = () => RECENT_WINDOWS[prefs.recentWindow] || RECENT_WINDOWS.week;
const recentOk = (t) => !avoidRecent() || !heardWithin(t, recentWindowMs());

// Leaves out recent plays, unless that would leave too little to choose from
function withoutRecent(list, keep) {
  if (!avoidRecent()) return list;
  const fresh = list.filter(recentOk);
  return fresh.length >= Math.min(keep || 10, Math.ceil(list.length / 3)) ? fresh : list;
}

// Last.fm (when you're signed in there): what you scrobbled within the time frame, on any device, counts too
let lfmHeardAt = 0;
async function lfmHeardSync() {
  if (typeof lfmActive !== "function" || !lfmActive() || Date.now() - lfmHeardAt < 30 * 60000) return;
  lfmHeardAt = Date.now();
  try {
    const from = Math.floor((Date.now() - recentWindowMs()) / 1000);
    const list = await invoke("lastfm_recent", { from, pages: prefs.recentWindow === "month" ? 10 : 5 });
    list.forEach((x) => heardSong(songKey({ title: x.title, user: { username: x.artist } }), x.ts));
    heardSongsSave();
  } catch (e) {
    lfmHeardAt = Date.now() - 25 * 60000;   // try again in five minutes
  }
}

// a track's name key, worked out once (shuffle asks for thousands at a time)
const songKeyCache = new WeakMap();
function songKeyOf(t) {
  let k = songKeyCache.get(t);
  if (k === undefined) { k = songKey(t); songKeyCache.set(t, k); }
  return k;
}

const lastHeard = (t) => {
  if (!t) return 0;
  const byId = lastPlayedMap.get(t.id) || 0;
  const k = heardSongs.size ? songKeyOf(t) : "";
  return Math.max(byId, (k && heardSongs.get(k)) || 0);
};
const heardWithin = (t, ms) => Date.now() - lastHeard(t) < ms;
