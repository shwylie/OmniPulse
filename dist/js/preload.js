// Preloading the next track: once the one playing is 70% through, or has a minute left (whichever
// comes first), what the next one needs before it can start is found ahead of time, so it starts
// straight away instead of after a silence. That's its YouTube Music match and stream (yt-dlp), its
// SoundCloud stream addresses, or a plugin's address; your files and Spotify Premium start at once
// anyway. While it's found, the marquee glides over the seek bar (which stays usable). What's found
// is kept ten minutes (streams' addresses expire), and used once.

const PRELOAD_AT = 0.7;
const PRELOAD_LEFT_MS = 60000;
const PRELOAD_TTL = 10 * 60000;
const preloaded = new Map();   // track id -> { kind, at, url | info, first }
let preloadFor = "";           // the track looked for last (so it's done once)

// What plays after this one: the next in the queue (or the first again, repeating all)
function preloadNext() {
  if (repeatMode === "one") return null;
  if (qi < queue.length - 1) return queue[qi + 1];
  return repeatMode === "all" && queue.length > 1 ? queue[0] : null;
}

// Something found ahead for this track (kind: "plugin", "scdlp", "sc"), taken once; null if none or stale
function preloadTake(t, kind) {
  const p = t && preloaded.get(t.id);
  if (!p) return null;
  preloaded.delete(t.id);
  return p.kind === kind && Date.now() - p.at < PRELOAD_TTL ? p : null;
}

function preloadBar(on) {
  document.querySelectorAll(".seekRow, .miniProg").forEach((n) => n.classList.toggle("preloading", !!on));
}

async function preloadTrack(t) {
  const src = srcOf(t);
  if (src === "local" || onSpotifyEngine(t)) return;
  // YouTube Music: its stream (yt-dlp), kept by the Rust side for when it starts
  if (src === "youtube") { if (ytAvailable()) await invoke("yt_prepare", { id: ytIdOf(t) }); return; }
  // what plays from YouTube Music (the catalogues, MusicBrainz, Spotify without Premium, Go+): its match, then its stream
  const viaYt = viaYouTube(t) || src === "spotify" || (src === "soundcloud" && isPreview(t));
  if (viaYt) {
    if (!ytOn()) return;
    const m = await ytMatch(t, src !== "spotify");
    if (m && ytIdOf(m)) await invoke("yt_prepare", { id: ytIdOf(m) });
    return;
  }
  if (isPluginTrack(t)) { preloaded.set(t.id, { kind: "plugin", at: Date.now(), url: await pluginPlayUrl(t) }); return; }
  if (src !== "soundcloud") return;
  // SoundCloud signed out: its own audio, through yt-dlp
  if (meId === null) {
    if (!/^https:\/\/(m\.)?soundcloud\.com\//.test(t.permalink_url || "") || !ytAvailable()) return;
    preloaded.set(t.id, { kind: "scdlp", at: Date.now(), url: await invoke("sc_ytdlp", { url: String(t.permalink_url).split(/[?#]/)[0] }) });
    return;
  }
  // signed in: its streams, and the first one's address
  const info = await invoke("stream_url", { id: String(t.id) });
  const c = (info.candidates || []).find((x) => x.name !== "preview");
  let first = null;
  if (c) { try { first = { name: c.name, resolved: await invoke("stream_resolve", { url: c.url }) }; } catch (e) {} }
  preloaded.set(t.id, { kind: "sc", at: Date.now(), info, first });
}

function preloadTick() {
  const t = curTrack();
  if (!t || switching || !nowPlaying()) return;
  const next = preloadNext();
  if (!next || next.id === preloadFor || next.id === t.id) return;
  const dur = onSpotifyEngine(t) ? spState.dur : curDurMs();
  if (!dur) return;
  const pos = curPosMs();
  if (pos < dur * PRELOAD_AT && dur - pos > PRELOAD_LEFT_MS) return;
  preloadFor = next.id;
  if (srcOf(next) === "local" || onSpotifyEngine(next)) return;   // (nothing to find)
  preloadBar(true);
  preloadTrack(next).catch(() => {}).finally(() => preloadBar(false));
}

function preloadInit() {
  setInterval(preloadTick, 1000);
}
