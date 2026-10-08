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

// ---------- songs and artists on this page behave as they do everywhere ----------
// A recorded play, as a track to play: the app's own copy if it has one, otherwise rebuilt from what
// was recorded (enough to play it from its platform)
function statTrack(x) {
  const id = String(x.id);
  const pools = [queue, viewTracks, typeof likedTracks !== "undefined" ? likedTracks : [], typeof spLikedCache !== "undefined" ? spLikedCache || [] : [],
    typeof localTracks !== "undefined" ? localTracks : [], typeof ytLikes !== "undefined" ? ytLikes : []];
  for (const pool of pools) {
    const hit = (pool || []).find((t) => t && String(t.id) === id);
    if (hit) return Object.assign({}, hit, { autoplay: undefined, requestedBy: undefined });
  }
  const base = { id: x.id, title: x.title, duration: x.ms || 0, artwork_url: x.art || "", genre: x.genre || "", album: x.album || "" };
  if (x.src === "spotify") return tidyTrack(Object.assign(base, { source: "spotify", uri: "spotify:track:" + id.replace(/^sp:/, ""), displayArtist: x.artist, user: { username: x.artist } }));
  if (x.src === "youtube") { const v = id.replace(/^yt:/, ""); return tidyTrack(Object.assign(base, { source: "youtube", ytId: v, user: { username: x.artist }, permalink_url: "https://music.youtube.com/watch?v=" + v })); }
  if (x.src === "local") return Object.assign(base, { source: "local", user: { username: x.artist }, missing: true });
  return tidyTrack(Object.assign(base, { kind: "track", user: { username: x.artist } }));
}

const openArtistNamedStat = (name) => openArtist(artistFrom({ name }));

// A track line: clicks play (from here down the list), right-click has the usual menu, artists open
function statSongRow(t, list, i, rank, sub) {
  const li = el("li", "wSongRow");
  li.title = "Play";
  if (rank) li.append(el("span", "wRank", String(rank)));
  li.append(cover(artUrl(t, "t200x200") || t.artwork_url, "", t.title));
  const txt = el("div", "wText");
  const who = el("span", "artistLink");
  fillArtists(who, t);
  const subLine = el("div", "wSub");
  subLine.append(who);
  if (sub) subLine.append(document.createTextNode(" · " + sub));
  txt.append(el("div", "wTitle", t.title || "Untitled"), subLine);
  li.append(txt);
  li.addEventListener("click", (e) => {
    e.stopPropagation();
    if (e.target.closest(".artistLink")) { openArtistClicked(t, e.target); return; }
    startList(list, i, false, "Your Listening");
  });
  li.addEventListener("contextmenu", (e) => { e.preventDefault(); e.stopPropagation(); openTrackMenu(t, e.clientX, e.clientY); });
  return li;
}

function statArtistLink(name, cls) {
  const a = el("span", "wArtistLink " + (cls || ""), name);
  a.title = "Open " + name + "'s page";
  a.addEventListener("click", (e) => { e.stopPropagation(); openArtistNamedStat(name); });
  return a;
}

// A tile that opens a closer look at itself
function goCard(c, detail) {
  c.classList.add("go");
  c.tabIndex = 0;
  c.setAttribute("role", "button");
  const open = () => { statsDetail = detail; renderStats(); $("listView").scrollTop = 0; };
  c.addEventListener("click", open);
  c.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(); } });
  c.append(el("span", "wMore", "›"));
  return c;
}

let statsDetail = null;   // the tile looked at more closely, or null for them all

function renderStats() {
  const box = $("statsView");
  box.replaceChildren();
  const all = loadStats().sort((a, b) => a.ts - b.ts);
  const p = statsPeriod();
  const since = STATS_PERIODS[p].since();
  const list = all.filter((x) => x.ts >= since);

  const head = el("div", "wHead");
  if (statsDetail) {
    const back = el("button", "soft wBack", "‹ Your Listening");
    back.addEventListener("click", () => { statsDetail = null; renderStats(); });
    head.append(back);
  } else head.append(el("h1", "", "Your Listening"));
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
  if (statsDetail) { renderStatsDetail(box, statsDetail, list, all, p); return; }

  const ms = list.reduce((n, x) => n + (x.ms || 0), 0);
  const tracks = tally(list, (x) => x.title + "|" + x.artist);
  const artists = tally(list, firstArtistOf);
  const songTracks = tracks.map((t) => statTrack(t.last));
  const grid = el("div", "wGrid");

  // the big number
  grid.append(goCard(card("wHero",
    el("div", "wKicker", "You listened for"),
    el("div", "wBig", fmtHours(ms)),
    el("div", "wSub", plural(list.length, "play", "plays") + " · " + plural(artists.length, "artist", "artists") + " · " + plural(tracks.length, "song", "songs") + " " + STATS_PERIODS[p].label)), "time"));

  // the top artist
  const top = artists[0];
  const share = Math.round((top.plays / list.length) * 100);
  const spot = card("wSpot", cover(artistArt(top.key, list), "round big", top.key),
    el("div", "wKicker", "Your top artist"),
    statArtistLink(top.key, "wName"),
    el("div", "wSub", plural(top.plays, "play", "plays") + " · " + share + "% of everything you played"));
  spot.style.setProperty("--hue", hueOf(top.key));
  grid.append(goCard(spot, "artists"));

  // top songs
  const songs = card("wSongs", el("div", "wKicker", "Your top songs"));
  const ol = el("ol", "wList");
  tracks.slice(0, 5).forEach((t, i) => ol.append(statSongRow(songTracks[i], songTracks, i, i + 1, plural(t.plays, "play", "plays"))));
  songs.append(ol);
  grid.append(goCard(songs, "songs"));

  // top artists
  if (artists.length > 1) {
    const row = el("div", "wArtists");
    artists.slice(0, 6).forEach((a) => {
      const b = el("div", "wArtist");
      b.title = "Open " + a.key + "'s page";
      b.append(cover(artistArt(a.key, list), "round", a.key), el("div", "wTitle", a.key), el("div", "wSub", plural(a.plays, "play", "plays")));
      b.addEventListener("click", (e) => { e.stopPropagation(); openArtistNamedStat(a.key); });
      row.append(b);
    });
    grid.append(goCard(card("wArtistsCard", el("div", "wKicker", "Your top artists"), row), "artists"));
  }

  // when you listen
  const hours = Array(24).fill(0);
  const days = Array(7).fill(0);
  list.forEach((x) => { const d = new Date(x.ts); hours[d.getHours()] += (x.ms || 180000); days[d.getDay()] += 1; });
  const kind = listenerType(hours);
  const peak = hours.indexOf(Math.max(...hours));
  const busiest = days.indexOf(Math.max(...days));
  grid.append(goCard(card("wWhen", el("div", "wKicker", "You're a"), el("div", "wName", kind.name), el("div", "wSub", kind.note + " Your peak hour: " + hourName(peak) + ". Busiest on " + DAYS[busiest] + "."), statClock(hours, peak), statClockLabels()), "when"));

  // where it comes from
  const plats = tally(list, (x) => x.src || "other");
  if (plats.length) grid.append(goCard(card("wWhere", el("div", "wKicker", "Where it came from"), ...statPlatBar(plats, list.length)), "where"));

  // streaks and the song on repeat
  const st = streaks(all);
  const perDay = tally(list, (x) => dayKey(x.ts) + "|" + x.title + "|" + x.artist)[0];
  grid.append(goCard(card("wStreak",
    el("div", "wKicker", "Streak"),
    el("div", "wBig small", plural(st.now, "day", "days")),
    el("div", "wSub", "in a row" + (st.best > st.now ? ", your best is " + plural(st.best, "day", "days") : st.now > 1 ? ": your best yet" : ""))), "streak"));
  if (perDay && perDay.plays > 2) {
    const rt = statTrack(perDay.last);
    const rep = card("wRepeat", el("div", "wKicker", "On repeat"));
    const ul = el("ol", "wList");
    ul.append(statSongRow(rt, [rt], 0, 0, plural(perDay.plays, "time", "times") + " in one day, " + new Date(perDay.last.ts).toLocaleDateString(undefined, { weekday: "long", month: "short", day: "numeric" })));
    rep.append(ul);
    grid.append(rep);
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

function statClock(hours, peak) {
  const clock = el("div", "wClock");
  const maxH = Math.max(...hours) || 1;
  hours.forEach((v, h) => {
    const bar = el("span", h === peak ? "on" : "");
    bar.style.height = Math.max(4, Math.round((v / maxH) * 100)) + "%";
    bar.title = hourName(h) + ": " + fmtHours(v);
    clock.append(bar);
  });
  return clock;
}

function statClockLabels() {
  const l = el("div", "wClockLabels");
  ["12 am", "6 am", "noon", "6 pm", ""].forEach((t) => l.append(el("span", "", t)));
  return l;
}

function statPlatBar(plats, total) {
  const bar = el("div", "wPlatBar");
  const legend = el("div", "wLegend");
  plats.forEach((pl) => {
    const part = el("span", "wPlat " + pl.key);
    part.style.flexGrow = String(pl.plays);
    part.title = platInfo(pl.key).name + ": " + plural(pl.plays, "play", "plays");
    bar.append(part);
    const lg = el("span", "wLg " + pl.key);
    lg.append(el("i"), document.createTextNode(platInfo(pl.key).name + " " + Math.round((pl.plays / total) * 100) + "%"));
    legend.append(lg);
  });
  return [bar, legend];
}

// Bars with labels under them: [label, value, title]
function statBars(items, cls) {
  const wrap = el("div", "wBars " + (cls || ""));
  const max = Math.max(1, ...items.map((x) => x[1]));
  items.forEach(([label, v, tip]) => {
    const col = el("div", "wBarCol");
    const bar = el("span", "wBar" + (v === max && v > 0 ? " on" : ""));
    bar.style.height = Math.max(v ? 4 : 1, Math.round((v / max) * 100)) + "%";
    col.title = tip || label;
    col.append(el("div", "wBarSlot"), el("div", "wBarLbl", label));
    col.firstChild.append(bar);
    wrap.append(col);
  });
  return wrap;
}

// ---------- a closer look at one tile ----------
function renderStatsDetail(box, which, list, all, p) {
  const grid = el("div", "wDetail");
  const ms = list.reduce((n, x) => n + (x.ms || 0), 0);
  const label = STATS_PERIODS[p].label;
  const h1 = (t) => box.append(el("h1", "wDetailHead", t));
  if (which === "time") {
    h1("You listened for " + fmtHours(ms) + " " + label);
    // by day (or by month, over a year or more)
    const byMonth = p === "year" || p === "all";
    const m = new Map();
    list.forEach((x) => {
      const d = new Date(x.ts);
      const k = byMonth ? d.getFullYear() + "-" + String(d.getMonth()).padStart(2, "0") : dayKey(x.ts);
      const e = m.get(k) || { ms: 0, plays: 0, d };
      e.ms += x.ms || 0;
      e.plays++;
      m.set(k, e);
    });
    const items = [...m.values()].sort((a, b) => a.d - b.d).slice(-60).map((e) => [
      byMonth ? e.d.toLocaleDateString(undefined, { month: "short" }) : e.d.toLocaleDateString(undefined, { day: "numeric", month: "short" }),
      e.ms, fmtHours(e.ms) + ", " + plural(e.plays, "play", "plays")]);
    const span = Math.max(1, (Date.now() - Math.max(list[0].ts, STATS_PERIODS[p].since())) / 86400000);
    grid.append(card("wWide", el("div", "wKicker", byMonth ? "By month" : "By day"), statBars(items, "time")),
      card("", el("div", "wKicker", "On average"), el("div", "wBig small", fmtHours(ms / Math.max(1, Math.ceil(span)))), el("div", "wSub", "a day, " + plural(list.length, "play", "plays") + " in all")),
      card("", el("div", "wKicker", "Your biggest day"), (() => { const best = tally(list, (x) => dayKey(x.ts)).sort((a, b) => b.ms - a.ms)[0]; return el("div", "wName", new Date(best.last.ts).toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" })); })(),
        el("div", "wSub", fmtHours(tally(list, (x) => dayKey(x.ts)).sort((a, b) => b.ms - a.ms)[0].ms))));
  } else if (which === "artists") {
    const artists = tally(list, firstArtistOf).slice(0, 50);
    h1("Your top artists " + label);
    const ol = el("ol", "wList wRanked");
    artists.forEach((a, i) => {
      const li = el("li", "wArtistRow");
      li.append(el("span", "wRank", String(i + 1)), cover(artistArt(a.key, list), "round", a.key));
      const txt = el("div", "wText");
      txt.append(statArtistLink(a.key, "wTitle"), el("div", "wSub", plural(a.plays, "play", "plays") + " · " + fmtHours(a.ms)));
      const bar = el("div", "wShare");
      bar.append(el("span"));
      bar.firstChild.style.width = Math.round((a.plays / artists[0].plays) * 100) + "%";
      li.append(txt, bar);
      li.addEventListener("click", () => openArtistNamedStat(a.key));
      ol.append(li);
    });
    grid.append(card("wWide", ol));
  } else if (which === "songs") {
    const tracks = tally(list, (x) => x.title + "|" + x.artist).slice(0, 50);
    const ts = tracks.map((t) => statTrack(t.last));
    h1("Your top songs " + label);
    const ol = el("ol", "wList wRanked");
    tracks.forEach((t, i) => ol.append(statSongRow(ts[i], ts, i, i + 1, plural(t.plays, "play", "plays") + " · " + fmtHours(t.ms))));
    const play = el("button", "soft", "Play These");
    play.addEventListener("click", () => startList(ts, 0, false, "Your top songs"));
    grid.append(card("wWide", play, ol));
  } else if (which === "when") {
    const hours = Array(24).fill(0), days = Array(7).fill(0);
    list.forEach((x) => { const d = new Date(x.ts); hours[d.getHours()] += (x.ms || 180000); days[d.getDay()] += x.ms || 180000; });
    const kind = listenerType(hours);
    h1("You're a " + kind.name);
    grid.append(card("wWide", el("div", "wKicker", "By hour"), statBars(hours.map((v, h) => [h % 3 === 0 ? hourName(h).replace(" am", "a").replace(" pm", "p").replace("midnight", "12a") : "", v, hourName(h) + ": " + fmtHours(v)]), "hours")),
      card("wWide", el("div", "wKicker", "By day"), statBars(days.map((v, d) => [DAYS[d].slice(0, 3), v, DAYS[d] + ": " + fmtHours(v)]), "days")),
      card("", el("p", "", kind.note)));
  } else if (which === "where") {
    const plats = tally(list, (x) => x.src || "other");
    h1("Where it came from");
    grid.append(card("wWide", ...statPlatBar(plats, list.length)));
    plats.forEach((pl) => {
      const of = list.filter((x) => (x.src || "other") === pl.key);
      const topA = tally(of, firstArtistOf).slice(0, 3);
      const c = card("", el("div", "wKicker", platInfo(pl.key).name), el("div", "wBig small", Math.round((pl.plays / list.length) * 100) + "%"),
        el("div", "wSub", plural(pl.plays, "play", "plays") + " · " + fmtHours(pl.ms)));
      const tops = el("div", "wSub wTops");
      topA.forEach((a, i) => { if (i) tops.append(document.createTextNode(", ")); tops.append(statArtistLink(a.key)); });
      if (topA.length) c.append(el("div", "wSub", "Most played:"), tops);
      grid.append(c);
    });
  } else if (which === "streak") {
    const st = streaks(all);
    h1(plural(st.now, "day", "days") + " in a row");
    // the last twelve weeks, a square a day
    const daysWith = new Map();
    all.forEach((x) => daysWith.set(dayKey(x.ts), (daysWith.get(dayKey(x.ts)) || 0) + 1));
    const cal = el("div", "wCal");
    const today = new Date(); today.setHours(0, 0, 0, 0);
    const start = new Date(today); start.setDate(start.getDate() - 83 - start.getDay());
    for (let d = new Date(start); d <= today; d.setDate(d.getDate() + 1)) {
      const n = daysWith.get(dayKey(d.getTime())) || 0;
      const sq = el("span", n ? "on" + (n > 20 ? " lots" : n > 5 ? " some" : "") : "");
      sq.title = d.toLocaleDateString(undefined, { weekday: "short", month: "short", day: "numeric" }) + ": " + plural(n, "play", "plays");
      cal.append(sq);
    }
    grid.append(card("wWide", el("div", "wKicker", "The last twelve weeks"), cal),
      card("", el("div", "wKicker", "Your best"), el("div", "wBig small", plural(st.best, "day", "days")), el("div", "wSub", "in a row")));
  }
  box.append(grid);
}

// ---------- the Stats button: right-click to hide it (Settings > Library brings it back) ----------
function statsNavPaint() {
  const shown = prefs.statsShown !== false;
  $("navStats").hidden = !shown;
  $("statsShownOn").checked = shown;
  if (!shown && view === "stats") openLikes();
}

function statsInit() {
  $("navStats").addEventListener("click", () => { statsDetail = null; showView("stats"); });
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
