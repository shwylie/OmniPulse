// Playlists as files: any playlist exports as M3U (each track's link, or its file on this PC) or CSV
// (title, artist, album, length, ISRC, platform, link), from its right-click menu; and an M3U or CSV
// file imports as a playlist on this PC (the import button beside New playlist), Exportify's Spotify CSVs too.
// An imported song is the file itself, or the link's own track, or else found by its title and
// artist: on your platforms first (Spotify by its ISRC), then in the catalogues. Songs not found
// anywhere are listed, to copy.

// ---------- CSV ----------
function csvRows(text) {
  const rows = [];
  let row = [], cell = "", quoted = false;
  for (let i = 0; i < text.length; i++) {
    const c = text[i];
    if (quoted) {
      if (c === '"' && text[i + 1] === '"') { cell += '"'; i++; }
      else if (c === '"') quoted = false;
      else cell += c;
    } else if (c === '"') quoted = true;
    else if (c === ",") { row.push(cell); cell = ""; }
    else if (c === "\n" || c === "\r") {
      if (c === "\r" && text[i + 1] === "\n") i++;
      row.push(cell); cell = "";
      if (row.some((x) => x.trim())) rows.push(row);
      row = [];
    } else cell += c;
  }
  row.push(cell);
  if (row.some((x) => x.trim())) rows.push(row);
  return rows;
}
const csvCell = (v) => { const s = String(v == null ? "" : v); return /[",\n\r]/.test(s) ? '"' + s.replace(/"/g, '""') + '"' : s; };

// The songs in a CSV: its header names the columns (ours, Exportify's, and most others')
function csvEntries(text) {
  const rows = csvRows(text);
  if (rows.length < 2) return [];
  const head = rows[0].map((h) => h.trim().toLowerCase());
  const col = (...names) => head.findIndex((h) => names.includes(h));
  const ti = col("title", "track name", "track", "name", "song", "song name");
  const ar = col("artist", "artist name(s)", "artists", "artist name", "artist(s)");
  const al = col("album", "album name");
  const du = col("duration (ms)", "duration", "length");
  const is = col("isrc");
  const li = col("link", "url", "track uri", "spotify uri", "uri", "spotify url");
  const pa = col("path", "file", "location");
  if (ti < 0) return [];
  return rows.slice(1).map((r) => {
    const dur = du >= 0 ? String(r[du] || "").trim() : "";
    // (a duration in m:ss, or ms)
    const ms = /^\d+:\d{2}$/.test(dur) ? dur.split(":").reduce((a, n) => a * 60 + Number(n), 0) * 1000 : Number(dur) || 0;
    return {
      title: (r[ti] || "").trim(), artist: ar >= 0 ? (r[ar] || "").replace(/;/g, ", ").trim() : "", album: al >= 0 ? (r[al] || "").trim() : "",
      ms, isrc: is >= 0 ? (r[is] || "").trim() : "", link: li >= 0 ? (r[li] || "").trim() : "", path: pa >= 0 ? (r[pa] || "").trim() : "",
    };
  }).filter((e) => e.title || e.link || e.path);
}

// ---------- M3U ----------
function m3uEntries(text) {
  const out = [];
  let info = null;
  let name = "";
  text.split(/\r?\n/).forEach((raw) => {
    const line = raw.trim();
    if (!line) return;
    if (/^#PLAYLIST:/i.test(line)) { name = line.slice(10).trim(); return; }
    const m = line.match(/^#EXTINF:\s*(-?\d+(?:\.\d+)?)[^,]*,(.*)$/i);
    if (m) {
      const secs = Number(m[1]);
      const label = m[2].trim();
      const parts = label.split(/\s+[-–]\s+/);
      info = { ms: secs > 0 ? secs * 1000 : 0, artist: parts.length > 1 ? parts[0] : "", title: parts.length > 1 ? parts.slice(1).join(" - ") : label };
      return;
    }
    if (line.startsWith("#")) return;
    const e = Object.assign({ title: "", artist: "", album: "", ms: 0, isrc: "", link: "", path: "" }, info || {});
    if (/^https?:\/\/|^spotify:/i.test(line)) e.link = line; else e.path = line.replace(/^file:\/\/\/?/i, "");
    // (no #EXTINF: the file's name is the best there is)
    if (!e.title && e.path) e.title = e.path.split(/[\\/]/).pop().replace(/\.[^.]+$/, "");
    out.push(e);
    info = null;
  });
  out.name = name;
  return out;
}

// ---------- finding each song ----------
async function linkTrack(link) {
  const sp = link.match(/(?:open\.spotify\.com\/(?:intl-[a-z]+\/)?track\/|spotify:track:)([A-Za-z0-9]{22})/);
  if (sp && spotifyUsable()) { const x = await spApi("GET", "/tracks/" + sp[1]).catch(() => null); if (x && x.id) return spTrack(x); }
  const yt = ytLinkId(link);
  if (yt && ytOn()) { const list = await invoke("yt_radio", { id: yt }).catch(() => []); const x = list.map(ytTrack).find((c) => c.ytId === yt); if (x) return x; }
  if (/^https:\/\/(m\.|www\.)?soundcloud\.com\//i.test(link) && lastSignedIn) {
    const x = await api("/resolve?url=" + encodeURIComponent(link.split(/[?#]/)[0])).catch(() => null);
    if (x && x.kind === "track") return x;
  }
  const dz = link.match(/deezer\.com\/(?:[a-z]{2}\/)?track\/(\d+)/);
  if (dz && catOn() && ytOn()) { const x = await dzGet("/track/" + dz[1]).catch(() => null); if (x && x.id) return dzTrack(x); }
  return null;
}

async function entryTrack(e, i) {
  if (e.path) {
    const p = e.path.replace(/\//g, "\\").toLowerCase();
    const f = localTracks.find((t) => t.path && t.path.toLowerCase() === p);
    if (f) return f;
  }
  if (e.link) { const t = await linkTrack(e.link).catch(() => null); if (t) return t; }
  if (!e.title) return null;
  const s = { id: "imp:" + i, title: e.title, artist: e.artist, album: e.album, ms: e.ms, isrc: e.isrc, art: "" };
  const f = await appleFind(s).catch(() => null);   // (your platforms: Spotify by ISRC, then SoundCloud, then YouTube Music)
  if (f) return f.track;
  if (catOn() && ytOn()) {
    const want = songKey({ title: e.title, user: { username: e.artist } });
    const found = await catSearch((e.artist + " " + e.title).trim()).catch(() => []);
    const c = found.find((x) => songKey(x) === want) || null;
    if (c) return c;
  }
  return null;
}

async function plImport() {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg) { say("Importing needs the app."); return; }
  const src = await dlg.open({ multiple: false, title: "Import a playlist", filters: [{ name: "Playlist", extensions: ["m3u", "m3u8", "csv", "txt"] }] }).catch(() => null);
  if (!src) return;
  const path = String(src);
  let entries;
  try {
    const text = await invoke("playlist_file_read", { path });
    entries = /\.csv$/i.test(path) ? csvEntries(text) : m3uEntries(text);
  } catch (e) { say("Couldn't read that file: " + e, { force: true }); return; }
  if (!entries.length) { say("No songs in that file (a CSV needs a header row with a title column).", { force: true }); return; }
  const name = entries.name || path.split(/[\\/]/).pop().replace(/\.[^.]+$/, "");
  const found = new Array(entries.length).fill(null);
  const missing = [];
  await appleEach(entries, '"' + name + '"', async (e, k) => {
    found[k] = await entryTrack(e, k);
    if (!found[k]) missing.push((e.artist ? e.artist + " - " : "") + (e.title || e.link || e.path));
  }, "Importing");
  const tracks = found.filter(Boolean);
  if (!tracks.length) { say("None of its songs were found anywhere.", { force: true, actions: [["Copy the missing", () => navigator.clipboard.writeText(missing.join("\n"))]] }); return; }
  const p = xplCreate(name, tracks);
  await refreshSidebar();
  const shown = playlists.find((x) => x.id === p.id);
  if (shown) openPlaylist(shown);
  say('Imported "' + name + '": ' + plural(tracks.length, "song", "songs") + (missing.length ? ", " + plural(missing.length, "song wasn't", "songs weren't") + " found anywhere." : "."),
    { stay: !!missing.length, force: true, actions: missing.length ? [["Copy the missing", () => navigator.clipboard.writeText(missing.join("\n")).then(() => say("The missing songs are copied.", { force: true }))]] : [] });
}

// ---------- exporting ----------
async function playlistAllTracks(p) {
  if (isXpl(p) || String(p.id).startsWith("xpl:")) return xplTracks(xplFind(p.id) || { tracks: [] });
  if (p.source === "youtube") return p.ytPlaylistId ? (await invoke("yt_playlist", { id: p.ytPlaylistId })).map(ytTrack) : (await invoke("yt_album", { id: p.ytAlbumId })).tracks.map(ytTrack);
  if (p.source === "deezer") return dzAlbumTracks(p.dzAlbumId);
  if (p.source === "mix") return mixTracks(p);
  return platformPlaylistTracks(p);
}

const trackLink = (t) => (srcOf(t) === "local" ? "" : t.permalink_url || (srcOf(t) === "youtube" && ytIdOf(t) ? "https://music.youtube.com/watch?v=" + ytIdOf(t) : ""));

async function plExport(p, kind) {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg || !dlg.save) { say("Exporting needs the app."); return; }
  const title = (p.title || "Playlist").replace(/[<>:"/\\|?*]+/g, " ").trim() || "Playlist";
  const dest = await dlg.save({ title: "Export " + (p.title || "playlist"), defaultPath: title + "." + kind,
    filters: [kind === "csv" ? { name: "CSV", extensions: ["csv"] } : { name: "M3U playlist", extensions: ["m3u8", "m3u"] }] }).catch(() => null);
  if (!dest) return;
  say("Exporting...");
  let list;
  try { list = await playlistAllTracks(p); } catch (e) { say("Couldn't read the playlist: " + e, { force: true }); return; }
  const rows = list.map((t) => { applyFix(t); return t; });
  let text;
  if (kind === "csv") {
    text = ["Title,Artist,Album,Duration (ms),ISRC,Platform,Link,Path"].concat(rows.map((t) => [t.title, artistName(t), t.album || "", t.duration || "", t.isrc || "",
      platInfo(srcOf(t)).name, trackLink(t), t.path || ""].map(csvCell).join(","))).join("\r\n") + "\r\n";
  } else {
    text = "#EXTM3U\n#PLAYLIST:" + (p.title || "Playlist") + "\n" + rows.map((t) => "#EXTINF:" + Math.round((t.duration || 0) / 1000) + "," + artistName(t) + " - " + t.title + "\n" +
      (t.path || trackLink(t))).filter((x) => !x.endsWith("\n")).join("\n") + "\n";
  }
  try {
    await invoke("playlist_file_write", { path: String(dest), text });
    say('Exported "' + (p.title || "playlist") + '": ' + plural(rows.length, "song", "songs") + ".", { force: true });
  } catch (e) {
    say("Couldn't save it: " + e, { force: true });
  }
}
