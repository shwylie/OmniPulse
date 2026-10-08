// Listening stats, kept only in this app's storage on this PC, shown Wrapped-style: a big number, your
// top artist and songs, when and where you listen, streaks, and a few facts picked fresh each visit.

// ---------- what's recorded (each counted play: 30 s or half the track) ----------
const STATS_KEY = "sc.stats";
const STATS_MAX = 8000;
let session = null;   // what's being listened to right now

function loadStats() {
  try { return JSON.parse(localStorage.getItem(STATS_KEY) || "[]"); } catch (e) { return []; }
}
function saveStats(list) {
  try { localStorage.setItem(STATS_KEY, JSON.stringify(list.slice(-STATS_MAX))); } catch (e) {}
}

function countPlay(t) {
  markHeard(t);
  const list = loadStats();
  const art = artUrl(t, "t200x200");
  list.push(Object.assign({
    ts: Date.now(),
    id: t.id,
    title: t.title || "Untitled",
    artist: artistName(t),
    ms: t.duration || 0,
    src: srcOf(t),
  }, art ? { art } : {}, albumShown(t) ? { album: albumShown(t) } : {}, t.genre ? { genre: t.genre } : {}));
  saveStats(list);
  if (view === "stats") renderStats();
}

// ---------- working it out ----------
const STATS_PERIODS = {
  week: { name: "This Week", since: () => Date.now() - 7 * 86400000, label: "this week" },
  month: { name: "This Month", since: () => { const d = new Date(); d.setDate(1); d.setHours(0, 0, 0, 0); return d.getTime(); }, label: "this month" },
  year: { name: "This Year", since: () => new Date(new Date().getFullYear(), 0, 1).getTime(), label: "this year" },
  all: { name: "All Time", since: () => 0, label: "so far" },
};
const statsPeriod = () => (prefs.statsPeriod in STATS_PERIODS ? prefs.statsPeriod : "month");

const firstArtistOf = (x) => String(x.artist || "").split(/\s*,\s*/)[0] || "Unknown artist";
const dayKey = (ts) => { const d = new Date(ts); return d.getFullYear() + "-" + d.getMonth() + "-" + d.getDate(); };
const DAYS = ["Sundays", "Mondays", "Tuesdays", "Wednesdays", "Thursdays", "Fridays", "Saturdays"];

function tally(list, keyFn) {
  const m = new Map();
  list.forEach((x) => {
    const k = keyFn(x);
    if (!k) return;
    const e = m.get(k) || { key: k, plays: 0, ms: 0, last: x };
    e.plays++;
    e.ms += x.ms || 0;
    e.last = x;
    m.set(k, e);
  });
  return [...m.values()].sort((a, b) => b.plays - a.plays || b.ms - a.ms);
}

// A cover for a track or artist: what was recorded, or a copy of the track the app has now
function statArt(x) {
  if (x && x.art) return x.art;
  const t = [queue, viewTracks, likedTracks].map((l) => l.find((y) => y && y.id === x.id)).find(Boolean);
  return t ? artUrl(t, "t200x200") : "";
}
function artistArt(name, list) {
  const hit = list.slice().reverse().find((x) => firstArtistOf(x) === name && statArt(x));
  return hit ? statArt(hit) : "";
}

function streaks(all) {
  const days = new Set(all.map((x) => dayKey(x.ts)));
  let best = 0, run = 0;
  const sorted = [...new Set(all.map((x) => { const d = new Date(x.ts); d.setHours(0, 0, 0, 0); return d.getTime(); }))].sort((a, b) => a - b);
  sorted.forEach((d, i) => { run = i && d - sorted[i - 1] <= 86400000 * 1.5 ? run + 1 : 1; best = Math.max(best, run); });
  let now = 0;
  const d = new Date();
  d.setHours(0, 0, 0, 0);
  if (!days.has(dayKey(d.getTime()))) d.setDate(d.getDate() - 1);   // today not started yet still counts yesterday's run
  while (days.has(dayKey(d.getTime()))) { now++; d.setDate(d.getDate() - 1); }
  return { best, now };
}

// listening sessions: plays less than 30 minutes apart
function longestSession(list) {
  let best = { ms: 0, start: 0, plays: 0 }, cur = null;
  list.forEach((x) => {
    if (!cur || x.ts - cur.end > 30 * 60000) cur = { start: x.ts, end: x.ts, ms: 0, plays: 0 };
    cur.end = x.ts;
    cur.ms += x.ms || 0;
    cur.plays++;
    if (cur.ms > best.ms) best = { ms: cur.ms, start: cur.start, plays: cur.plays };
  });
  return best;
}

const fmtHours = (ms) => {
  const h = ms / 3600000;
  return h >= 10 ? Math.round(h).toLocaleString() + " hours" : h >= 1 ? h.toFixed(1) + " hours" : Math.round(ms / 60000) + " minutes";
};
const plural = (n, one, many) => n.toLocaleString() + " " + (n === 1 ? one : many);
const hourName = (h) => (h === 0 ? "midnight" : h === 12 ? "noon" : (h % 12) + (h < 12 ? " am" : " pm"));

function listenerType(hours) {
  const share = (a, b) => hours.slice(a, b).reduce((n, v) => n + v, 0);
  const total = hours.reduce((n, v) => n + v, 0) || 1;
  const parts = [
    { name: "Night Owl", note: "Most of your listening happens after 10 pm.", v: (share(22, 24) + share(0, 4)) / total },
    { name: "Early Bird", note: "You get going before 9 am.", v: share(5, 9) / total },
    { name: "Daytime Listener", note: "Music keeps your days going.", v: share(9, 17) / total },
    { name: "Evening Unwinder", note: "Your evenings have a soundtrack.", v: share(17, 22) / total },
  ];
  return parts.sort((a, b) => b.v - a.v)[0];
}

// Facts that change from visit to visit: a few are picked at random from whatever's true
function statFacts(list, all, since) {
  const facts = [];
  const tracks = tally(list, (x) => x.title + "|" + x.artist);
  const artists = tally(list, firstArtistOf);
  if (tracks[0]) {
    const days = new Set(list.filter((x) => x.title + "|" + x.artist === tracks[0].key).map((x) => dayKey(x.ts))).size;
    if (days > 1) facts.push(["Day after day", `You played "${tracks[0].last.title}" on ${plural(days, "different day", "different days")}.`]);
  }
  const once = artists.filter((a) => a.plays === 1).length;
  if (once > 2) facts.push(["Explorer", `${plural(once, "artist", "artists")} got just one play. Plenty to dig into.`]);
  const fresh = artists.filter((a) => !all.some((x) => x.ts < since && firstArtistOf(x) === a.key)).length;
  if (since > 0 && fresh > 0) facts.push(["New to you", `${plural(fresh, "artist was", "artists were")} new to you in this time.`]);
  const short = list.filter((x) => x.ms && x.ms < 180000).length;
  if (list.length > 10) facts.push(["Short and sweet", `${Math.round((short / list.length) * 100)}% of your plays were under 3 minutes.`]);
  const long = list.filter((x) => x.ms > 15 * 60000).length;
  if (long) facts.push(["In it for the long haul", `${plural(long, "set or podcast", "sets and podcasts")} over 15 minutes made the cut.`]);
  const sess = longestSession(list);
  if (sess.plays > 3) facts.push(["Marathon", `Your longest session ran ${fmtHours(sess.ms)}, ${plural(sess.plays, "track", "tracks")} in a row, on ${new Date(sess.start).toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })}.`]);
  const genres = tally(list.filter((x) => x.genre), (x) => String(x.genre).toLowerCase());
  if (genres[0] && genres[0].plays > 2) facts.push(["Your sound", `${genres[0].last.genre} came up the most, ${plural(genres[0].plays, "time", "times")}.`]);
  const loyal = artists.find((a) => a.plays >= 5 && all.filter((x) => firstArtistOf(x) === a.key).length >= a.plays * 2);
  if (loyal) facts.push(["Old favourite", `${loyal.key} is still on your mind: ${plural(loyal.plays, "play", "plays")} now, and more before that.`]);
  const firstTs = all.length ? all[0].ts : 0;
  if (firstTs) facts.push(["Since the start", `Your listening here goes back to ${new Date(firstTs).toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" })}.`]);
  // shuffled, so each visit tells you something different
  for (let i = facts.length - 1; i > 0; i--) { const j = Math.floor(Math.random() * (i + 1)); [facts[i], facts[j]] = [facts[j], facts[i]]; }
  return facts.slice(0, 3);
}

// ---------- drawing ----------
function cover(src, cls, initial) {
  const box = el("div", "wCover " + (cls || ""));
  if (src) { const img = el("img"); img.alt = ""; img.loading = "lazy"; img.src = src; img.onerror = () => img.remove(); box.append(img); }
  else box.append(el("span", "", (initial || "?").trim().charAt(0).toUpperCase()));
  return box;
}

function card(cls, ...kids) {
  const c = el("section", "wCard " + (cls || ""));
  c.append(...kids.filter(Boolean));
  return c;
}

function renderStats() {
  const box = $("statsView");
  box.replaceChildren();
  const all = loadStats().sort((a, b) => a.ts - b.ts);
  const p = statsPeriod();
  const since = STATS_PERIODS[p].since();
  const list = all.filter((x) => x.ts >= since);

  const head = el("div", "wHead");
  head.append(el("h1", "", "Your Listening"));
  const chips = el("div", "chips wChips");
  Object.entries(STATS_PERIODS).forEach(([k, v]) => {
    const b = el("button", "chip" + (k === p ? " on" : ""), v.name);
    b.addEventListener("click", () => { prefs.statsPeriod = k; savePrefs(); renderStats(); });
    chips.append(b);
  });
  head.append(chips);
  box.append(head);

  if (!list.length) {
    box.append(card("wEmpty", el("h2", "", "Nothing here yet"), el("p", "", all.length ? "No plays " + STATS_PERIODS[p].label + ". Try a longer stretch above." : "Play some music and your stats will grow here.")));
    return;
  }

  const ms = list.reduce((n, x) => n + (x.ms || 0), 0);
  const tracks = tally(list, (x) => x.title + "|" + x.artist);
  const artists = tally(list, firstArtistOf);
  const grid = el("div", "wGrid");

  // the big number
  grid.append(card("wHero",
    el("div", "wKicker", "You listened for"),
    el("div", "wBig", fmtHours(ms)),
    el("div", "wSub", plural(list.length, "play", "plays") + " · " + plural(artists.length, "artist", "artists") + " · " + plural(tracks.length, "song", "songs") + " " + STATS_PERIODS[p].label)));

  // the top artist
  const top = artists[0];
  const share = Math.round((top.plays / list.length) * 100);
  const spot = card("wSpot", cover(artistArt(top.key, list), "round big", top.key),
    el("div", "wKicker", "Your top artist"),
    el("div", "wName", top.key),
    el("div", "wSub", plural(top.plays, "play", "plays") + " · " + share + "% of everything you played"));
  spot.style.setProperty("--hue", hueOf(top.key));
  grid.append(spot);

  // top songs
  const songs = card("wSongs", el("div", "wKicker", "Your top songs"));
  const ol = el("ol", "wList");
  tracks.slice(0, 5).forEach((t, i) => {
    const li = el("li");
    li.append(el("span", "wRank", String(i + 1)), cover(statArt(t.last), "", t.last.title));
    const txt = el("div", "wText");
    txt.append(el("div", "wTitle", t.last.title), el("div", "wSub", t.last.artist + " · " + plural(t.plays, "play", "plays")));
    li.append(txt);
    ol.append(li);
  });
  songs.append(ol);
  grid.append(songs);

  // top artists
  if (artists.length > 1) {
    const row = el("div", "wArtists");
    artists.slice(0, 6).forEach((a) => {
      const b = el("div", "wArtist");
      b.append(cover(artistArt(a.key, list), "round", a.key), el("div", "wTitle", a.key), el("div", "wSub", plural(a.plays, "play", "plays")));
      row.append(b);
    });
    grid.append(card("wArtistsCard", el("div", "wKicker", "Your top artists"), row));
  }

  // when you listen
  const hours = Array(24).fill(0);
  const days = Array(7).fill(0);
  list.forEach((x) => { const d = new Date(x.ts); hours[d.getHours()] += (x.ms || 180000); days[d.getDay()] += 1; });
  const kind = listenerType(hours);
  const peak = hours.indexOf(Math.max(...hours));
  const clock = el("div", "wClock");
  const maxH = Math.max(...hours) || 1;
  hours.forEach((v, h) => {
    const bar = el("span", h === peak ? "on" : "");
    bar.style.height = Math.max(4, Math.round((v / maxH) * 100)) + "%";
    bar.title = hourName(h) + ": " + fmtHours(v);
    clock.append(bar);
  });
  const clockLabels = el("div", "wClockLabels");
  ["12 am", "6 am", "noon", "6 pm", ""].forEach((t) => clockLabels.append(el("span", "", t)));
  const busiest = days.indexOf(Math.max(...days));
  grid.append(card("wWhen", el("div", "wKicker", "You're a"), el("div", "wName", kind.name), el("div", "wSub", kind.note + " Your peak hour: " + hourName(peak) + ". Busiest on " + DAYS[busiest] + "."), clock, clockLabels));

  // where it comes from
  const plats = tally(list, (x) => x.src || "other");
  if (plats.length) {
    const bar = el("div", "wPlatBar");
    const legend = el("div", "wLegend");
    plats.forEach((pl) => {
      const part = el("span", "wPlat " + pl.key);
      part.style.flexGrow = String(pl.plays);
      part.title = platInfo(pl.key).name + ": " + plural(pl.plays, "play", "plays");
      bar.append(part);
      const lg = el("span", "wLg " + pl.key);
      lg.append(el("i"), document.createTextNode(platInfo(pl.key).name + " " + Math.round((pl.plays / list.length) * 100) + "%"));
      legend.append(lg);
    });
    grid.append(card("wWhere", el("div", "wKicker", "Where it came from"), bar, legend));
  }

  // streaks and the song on repeat
  const st = streaks(all);
  const perDay = tally(list, (x) => dayKey(x.ts) + "|" + x.title + "|" + x.artist)[0];
  grid.append(card("wStreak",
    el("div", "wKicker", "Streak"),
    el("div", "wBig small", plural(st.now, "day", "days")),
    el("div", "wSub", "in a row" + (st.best > st.now ? ", your best is " + plural(st.best, "day", "days") : st.now > 1 ? ": your best yet" : ""))));
  if (perDay && perDay.plays > 2) {
    grid.append(card("wRepeat", el("div", "wKicker", "On repeat"), cover(statArt(perDay.last), "", perDay.last.title),
      el("div", "wTitle", perDay.last.title),
      el("div", "wSub", plural(perDay.plays, "time", "times") + " in one day, " + new Date(perDay.last.ts).toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" }))));
  }

  // a few facts, different each visit
  statFacts(list, all, since).forEach(([title, text]) => grid.append(card("wFact", el("div", "wKicker", title), el("p", "", text))));

  box.append(grid);
  const clear = el("button", "soft wClear", "Clear Listening History");
  clear.addEventListener("click", () => {
    if (confirm("Clear all listening history? This can't be undone.")) {
      saveStats([]);
      renderStats();
    }
  });
  box.append(clear);
}

// ---------- the Stats button: right-click to hide it (Settings > Library brings it back) ----------
function statsNavPaint() {
  const shown = prefs.statsShown !== false;
  $("navStats").hidden = !shown;
  $("statsShownOn").checked = shown;
  if (!shown && view === "stats") openLikes();
}

function statsInit() {
  $("navStats").addEventListener("click", () => showView("stats"));
  $("navStats").addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    showMenuAt(e.clientX, e.clientY, ["Stats", ["Hide Stats from the sidebar", () => {
      prefs.statsShown = false;
      savePrefs();
      statsNavPaint();
      say("Stats is hidden. Bring it back in Settings > Library.");
    }]]);
  });
  $("statsShownOn").addEventListener("change", () => { prefs.statsShown = $("statsShownOn").checked; savePrefs(); statsNavPaint(); });
  statsNavPaint();
}
