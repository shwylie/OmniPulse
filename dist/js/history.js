// When each track was last heard (counted, like the stats: 30 s or half the track), kept between
// sessions. Autoplay uses it to avoid recent plays, and shuffle to put long-unheard tracks first.

const HISTORY_KEY = "sc.lastPlayed";
const HISTORY_MAX = 20000;

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
    let entries = [...lastPlayedMap];
    if (entries.length > HISTORY_MAX) {
      entries = entries.sort((a, b) => b[1] - a[1]).slice(0, HISTORY_MAX);
      lastPlayedMap = new Map(entries);
    }
    try { localStorage.setItem(HISTORY_KEY, JSON.stringify(entries)); } catch (e) {}
  }, 1000);
}

function markHeard(t) {
  if (!t || t.id == null) return;
  lastPlayedMap.set(t.id, Date.now());
  historySave();
}

const lastHeard = (t) => (t && lastPlayedMap.get(t.id)) || 0;
const heardWithin = (t, ms) => Date.now() - lastHeard(t) < ms;
