// An addon, as an OmniPulse plugin: a server with a manifest.json at its address (the BeatBoss / Eclipse
// Music shape), asked for songs by search (GET /search?q=) and for how to play one (its streamURL, else
// GET /stream/:id). This script is OmniPulse's own; the address is the addon's.
const BASE = __BASE__;

async function get(path) {
  const r = await OmniPulse.fetch(BASE + path);
  if (!r.ok) throw new Error("The addon answered " + r.status + ".");
  return r.json();
}

// An artist as a name: "Name", { name }, or several of either
function artistOf(a) {
  if (Array.isArray(a)) return a.map(artistOf).filter(Boolean).join(", ");
  if (a && typeof a === "object") return String(a.name || "");
  return String(a || "");
}

function albumOf(a) {
  return a && typeof a === "object" ? String(a.name || a.title || "") : String(a || "");
}

// (seconds, as the spec says; a number far too big for seconds was milliseconds)
function msOf(d) {
  const n = Number(d) || 0;
  return Math.round(n > 20000 ? n : n * 1000);
}

function song(t) {
  return {
    id: String(t.id),
    title: String(t.title || t.name || "Untitled"),
    artist: artistOf(t.artist || t.artists) || "Unknown artist",
    album: albumOf(t.album),
    duration_ms: msOf(t.duration),
    artwork: String(t.artworkURL || t.image || t.cover || t.albumCover || ""),
    link: "",
    stream: String(t.streamURL || t.stream_url || ""),
    format: String(t.format || "").toLowerCase(),
  };
}

OmniPulse.register({
  async search(query) {
    const q = String(query || "").trim();
    if (!q) return [];
    const d = await get("/search?q=" + encodeURIComponent(q) + "&limit=50");
    return (Array.isArray(d) ? d : (d && d.tracks) || []).filter((t) => t && t.id != null).map(song);
  },

  async resolve(track) {
    if (track.stream) return { url: track.stream, ext: track.format };
    const s = await get("/stream/" + encodeURIComponent(track.id));
    const url = s && (s.url || s.streamURL || s.stream_url);
    if (!url) throw new Error("The addon gave no stream for it.");
    return { url: String(url), ext: String((s && s.format) || track.format || "").toLowerCase() };
  },
});
