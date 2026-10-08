// Stream-Safe Music (Settings > Streaming): which tracks may be played on stream. A track counts when
//   - it's under an open licence: SoundCloud's Creative Commons ones and "no-rights-reserved", or a
//     plugin's track that says so (openLicence, in app.js)
//   - it's a YouTube video its uploader marked Creative Commons (asked of YouTube with your account)
//   - its artist or record label is cleared for streaming: copyrighted, but its owners let streamers
//     play it (NCS, StreamBeats and the like, plus any you add). Spotify's tracks are checked by their
//     album's label, SoundCloud's by theirs when they give one, and every track by its artists.
// Some labels clear you only once you've signed up with them: those start off. Nothing here can
// promise a claim won't happen; each entry's note says what it needs.

const STREAM_CLEARED = [
  { id: "ncs", name: "NoCopyrightSounds (NCS)", labels: ["NCS", "NoCopyrightSounds"], artists: ["NoCopyrightSounds"], on: true,
    note: "Free on streams and in videos. NCS asks you to credit the track." },
  { id: "streambeats", name: "StreamBeats by Harris Heller", labels: ["StreamBeats"], artists: ["StreamBeats", "Harris Heller"], on: true,
    note: "Made for streamers: free to play on stream." },
  { id: "monstercat", name: "Monstercat", labels: ["Monstercat", "Monstercat Silk", "Monstercat Instinct", "Monstercat Uncaged"], artists: [], on: false,
    note: "Only with Monstercat Gold or a Creator licence, and your channel registered with them." },
  { id: "epidemic", name: "Epidemic Sound", labels: ["Epidemic Sound"], artists: [], on: false,
    note: "Only with an Epidemic Sound subscription that covers your channel." },
  { id: "lofigirl", name: "Lofi Girl", labels: ["Lofi Records", "Lofi Girl"], artists: [], on: false,
    note: "Lofi Girl lets creators use its releases with credit. Check its current terms first." },
  { id: "chillhop", name: "Chillhop Music", labels: ["Chillhop Music", "Chillhop Records"], artists: [], on: false,
    note: "Check Chillhop's current terms for streamers first." },
];

// prefs.streamCleared: { custom: [{ id, kind: "artist" | "label", name }], on: { id: true / false } }
const clearedPrefs = () => prefs.streamCleared || { custom: [], on: {} };

// Every entry, as { id, name, artists, labels, on, note, custom } (worked out again only when they change)
let clearedMemo = { p: null, list: [] };
function clearedEntries() {
  const p = clearedPrefs();
  if (clearedMemo.p === prefs.streamCleared && prefs.streamCleared) return clearedMemo.list;
  clearedMemo = { p: prefs.streamCleared, list: clearedBuild(p) };
  return clearedMemo.list;
}
function clearedBuild(p) {
  const set = (e, dflt) => (p.on && e.id in p.on ? !!p.on[e.id] : dflt);
  return STREAM_CLEARED.map((e) => Object.assign({}, e, { on: set(e, e.on), custom: false }))
    .concat((p.custom || []).map((c) => ({
      id: c.id, name: c.name, custom: true, on: set(c, true), note: c.kind === "label" ? "Label" : "Artist",
      artists: c.kind === "artist" ? [c.name] : [], labels: c.kind === "label" ? [c.name] : [],
    })));
}

// ---------- what's known about each track ----------
const SP_LABELS_KEY = "sc.spLabels";
const YT_LIC_KEY = "sc.ytLicences";
const loadMap = (k) => { try { return JSON.parse(localStorage.getItem(k) || "{}"); } catch (e) { return {}; } };
const saveMap = (k, m) => { try { localStorage.setItem(k, JSON.stringify(m)); } catch (e) {} };
let spLabels = loadMap(SP_LABELS_KEY);   // Spotify album id -> its record label ("" if none given)
let ytLicences = loadMap(YT_LIC_KEY);    // YouTube video id -> "creativeCommon" / "youtube"
const lookAsked = new Set();             // asked already this run (found or not)

const labelOf = (t) => !t ? "" : srcOf(t) === "spotify" ? (t.spAlbum && spLabels[t.spAlbum.id]) || "" : String(t.label_name || "");
const ytCC = (t) => !!t && srcOf(t) === "youtube" && ytLicences[ytIdOf(t)] === "creativeCommon";

// The entry that clears this track, if one does
function clearedEntry(t) {
  if (!t) return null;
  const names = new Set([...artistNames(t), guessNames(t).artist, t.user && t.user.username].filter(Boolean).map(artistKey));
  const label = artistKey(labelOf(t));
  return clearedEntries().find((e) => e.on && (e.artists.some((a) => names.has(artistKey(a))) || (!!label && e.labels.some((l) => artistKey(l) === label)))) || null;
}

// Whether a track may be played on stream (whether or not Stream-Safe Music is on)
const streamOk = (t) => openLicence(t) || ytCC(t) || !!clearedEntry(t);

// Finds the labels of Spotify tracks and the licences of YouTube ones not known yet; true if any were
async function streamLookups(list) {
  let found = false;
  const albums = [...new Set(list.filter((t) => srcOf(t) === "spotify" && t.spAlbum && t.spAlbum.id && !(t.spAlbum.id in spLabels) && !lookAsked.has("a" + t.spAlbum.id)).map((t) => t.spAlbum.id))];
  if (albums.length && (spotifyPremium || spotifyFree)) {
    for (let i = 0; i < albums.length && i < 200; i += 20) {
      const batch = albums.slice(i, i + 20);
      batch.forEach((id) => lookAsked.add("a" + id));
      let got = [];
      try { got = ((await spApi("GET", "/albums?ids=" + batch.join(","))).albums || []); }
      catch (e) {
        // (not every app may ask for several at once: one at a time, then)
        for (const id of batch.slice(0, 10)) { try { got.push(await spApi("GET", "/albums/" + id)); } catch (e2) { break; } }
      }
      got.forEach((a) => { if (a && a.id) { spLabels[a.id] = String(a.label || ""); found = true; } });
    }
    if (found) saveMap(SP_LABELS_KEY, spLabels);
  }
  const vids = [...new Set(list.filter((t) => srcOf(t) === "youtube").map(ytIdOf).filter((v) => v && !(v in ytLicences) && !lookAsked.has("v" + v)))];
  if (vids.length && ytAcc.connected) {
    vids.forEach((v) => lookAsked.add("v" + v));
    try {
      const lic = await invoke("youtube_licences", { ids: vids.slice(0, 500) });
      Object.assign(ytLicences, lic);
      if (Object.keys(lic).length) { found = true; saveMap(YT_LIC_KEY, ytLicences); }
    } catch (e) { /* not allowed or used up: those stay unknown (not played) */ }
  }
  return found;
}

// While it's on: what's on the page and in the queue gets looked up, then shown if it's cleared
let streamLookTimer = null;
function streamLookSoon() {
  if (!streamSafe()) return;
  clearTimeout(streamLookTimer);
  streamLookTimer = setTimeout(async () => {
    if (await streamLookups(viewTracks.concat(queue))) { render(); renderQueue(); }
  }, 150);
}

// ---------- Settings > Streaming > Cleared for Streaming ----------
function clearedSave(fn) {
  const p = clearedPrefs();
  const next = { custom: (p.custom || []).slice(), on: Object.assign({}, p.on) };
  fn(next);
  prefs.streamCleared = next;
  savePrefs();
  clearedPaint();
  render();
  renderQueue();
}

function clearedPaint() {
  const rows = clearedEntries().map((e) => {
    const row = el("div", "clearedRow");
    const txt = el("div", "clearedText");
    txt.append(el("span", "lbl", e.name), el("span", "note", e.note));
    const sw = el("label", "switch");
    const box = el("input");
    box.type = "checkbox";
    box.checked = e.on;
    box.setAttribute("aria-label", "Cleared: " + e.name);
    box.addEventListener("change", () => clearedSave((p) => { p.on[e.id] = box.checked; }));
    sw.append(box, el("span"));
    row.append(txt, sw);
    if (e.custom) {
      const x = el("button", "", "×");
      x.title = "Take " + e.name + " off the list";
      x.setAttribute("aria-label", x.title);
      x.addEventListener("click", () => clearedSave((p) => { p.custom = p.custom.filter((c) => c.id !== e.id); delete p.on[e.id]; }));
      row.append(x);
    }
    return row;
  });
  $("clearedList").replaceChildren(...rows);
  $("clearedYt").textContent = ytAcc.connected
    ? "YouTube videos their uploader marked Creative Commons count too (asked of YouTube with your account)."
    : "Signed in to YouTube (Integrations), YouTube videos their uploader marked Creative Commons count too.";
}

// Adds an artist or label; true if it's new
function clearedAdd(kind, name) {
  name = String(name || "").trim();
  if (!name) return false;
  const key = artistKey(name);
  if (clearedEntries().some((e) => e.on && (kind === "artist" ? e.artists : e.labels).some((x) => artistKey(x) === key))) return false;
  clearedSave((p) => {
    const old = p.custom.find((c) => c.kind === kind && artistKey(c.name) === key);
    if (old) { p.on[old.id] = true; return; }
    const id = "c" + Date.now().toString(36);
    p.custom.push({ id, kind, name });
  });
  return true;
}

// The track menu: allow its artist on stream, or stop allowing whoever clears it
function clearedMenuLabel(t) {
  if (openLicence(t) || ytCC(t)) return null;
  const e = clearedEntry(t);
  if (e) return ["Stop allowing " + e.name + " on stream", "unclear"];
  const a = recArtistName(t);
  return a ? ["Allow " + a + " on stream", "clear"] : null;
}

function clearedMenuRun(t, action) {
  if (action === "clear") {
    const a = recArtistName(t);
    if (clearedAdd("artist", a)) say(a + " is cleared for streaming: Stream-Safe Music plays them.");
  } else {
    const e = clearedEntry(t);
    if (e) { clearedSave((p) => { p.on[e.id] = false; }); say(e.name + " isn't cleared for streaming any more."); }
  }
}

function streamSafeInit() {
  clearedPaint();
  $("clearedAdd").addEventListener("submit", (e) => {
    e.preventDefault();
    const name = $("clearedName").value;
    if (clearedAdd($("clearedKind").value, name)) { $("clearedName").value = ""; say(name.trim() + " is cleared for streaming."); }
    else if (name.trim()) say(name.trim() + " is already on the list.");
  });
}
