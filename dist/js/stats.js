// Listening stats, kept only in this app's storage on this PC.

// ---------- listening stats (kept in this app's storage on your PC) ----------
const STATS_KEY = "sc.stats";
let session = null;   // what's being listened to right now

function loadStats() {
  try { return JSON.parse(localStorage.getItem(STATS_KEY) || "[]"); } catch (e) { return []; }
}
function saveStats(list) {
  try { localStorage.setItem(STATS_KEY, JSON.stringify(list.slice(-5000))); } catch (e) {}
}

function countPlay(t) {
  const list = loadStats();
  list.push({
    ts: Date.now(),
    id: t.id,
    title: t.title || "Untitled",
    artist: artistName(t),
    ms: t.duration || 0,
  });
  saveStats(list);
  if (view === "stats") renderStats();
}

function topBy(list, keyFn) {
  const counts = new Map();
  list.forEach((x) => { const k = keyFn(x); if (k) counts.set(k, (counts.get(k) || 0) + 1); });
  return [...counts.entries()].sort((a, b) => b[1] - a[1]).slice(0, 10);
}

function statBlock(title, rows) {
  const box = el("div", "statBlock");
  box.append(el("h2", "", title));
  if (!rows.length) { box.append(el("p", "statsSub", "Nothing yet.")); return box; }
  const max = rows[0][1];
  const ol = el("ol");
  rows.forEach(([name, count]) => {
    const li = el("li", "statRow");
    const bar = el("div", "statBar");
    bar.style.width = Math.max(4, Math.round((count / max) * 100)) + "%";
    li.append(el("span", "statName", name), el("span", "statCount", String(count)), bar);
    ol.append(li);
  });
  box.append(ol);
  return box;
}

function renderStats() {
  const box = $("statsView");
  box.replaceChildren();
  const all = loadStats();
  const start = new Date();
  start.setDate(1);
  start.setHours(0, 0, 0, 0);
  const month = all.filter((x) => x.ts >= start.getTime());
  const hours = month.reduce((n, x) => n + (x.ms || 0), 0) / 3600000;
  box.append(el("h1", "", "Your listening"));
  box.append(el("p", "statsSub",
    month.length + (month.length === 1 ? " play" : " plays") + " this month, about " + hours.toFixed(1) +
    " hours. " + all.length + " plays recorded in total."));
  box.append(statBlock("Top artists this month", topBy(month, (x) => x.artist)));
  box.append(statBlock("Top tracks this month", topBy(month, (x) => x.title + (x.artist ? " (" + x.artist + ")" : ""))));
  box.append(statBlock("Top artists of all time", topBy(all, (x) => x.artist)));
  const clear = el("button", "soft", "Clear listening history");
  clear.addEventListener("click", () => {
    if (confirm("Clear all listening history? This can't be undone.")) {
      saveStats([]);
      renderStats();
    }
  });
  box.append(clear);
}

function statsInit() {
  $("navStats").addEventListener("click", () => showView("stats"));
}
