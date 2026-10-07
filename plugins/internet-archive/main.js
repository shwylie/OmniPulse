// Internet Archive, as an OmniPulse plugin: searches archive.org's audio (artist or title), lists the
// tracks of the best-known recordings, and plays each from the archive (FLAC if you prefer it).
// Uses only archive.org's public APIs: advancedsearch (find recordings) and metadata (their files).

const A = "https://archive.org";

function seconds(len) {
  if (len == null) return 0;
  const s = String(len);
  if (s.includes(":")) return s.split(":").map(Number).reduce((acc, n) => acc * 60 + n, 0);
  return Number(s) || 0;
}

// One track per song: the FLAC original and its MP3 copy share a name ("...t01.flac" / "...t01.mp3")
function tracksOf(id, meta, lossless) {
  const files = (meta.files || []).filter((f) => /^(flac|24bit flac|vbr mp3|mp3|128kbps mp3|ogg vorbis)$/i.test(f.format || ""));
  const songs = new Map();
  files.forEach((f) => {
    const base = f.name.replace(/\.[^.]+$/, "");
    if (!songs.has(base)) songs.set(base, []);
    songs.get(base).push(f);
  });
  const by = (meta.metadata && meta.metadata.creator) || "";
  const album = (meta.metadata && meta.metadata.title) || id;
  const out = [];
  songs.forEach((list, base) => {
    const flac = list.find((f) => /flac/i.test(f.format));
    const mp3 = list.find((f) => /mp3/i.test(f.format)) || list.find((f) => /ogg/i.test(f.format));
    const pick = lossless ? flac || mp3 : mp3 || flac;
    if (!pick) return;
    const named = list.find((f) => f.title) || pick;
    out.push({
      id: id + "/" + pick.name,
      title: named.title || base.replace(/[_-]+/g, " "),
      artist: named.creator || (Array.isArray(by) ? by[0] : by) || "Internet Archive",
      album,
      duration_ms: Math.round(seconds(named.length || pick.length) * 1000),
      artwork: A + "/services/img/" + encodeURIComponent(id),
      link: A + "/details/" + encodeURIComponent(id),
      file: pick.name,
      item: id,
      track: Number(named.track) || 0,
    });
  });
  out.sort((x, y) => (x.track || 0) - (y.track || 0) || x.file.localeCompare(y.file));
  return out;
}

OmniPulse.register({
  async search(query) {
    const s = OmniPulse.settings;
    const q = String(query).replace(/"/g, "").trim();
    if (!q) return [];
    const where = '(creator:("' + q + '") OR title:("' + q + '")) AND mediatype:(audio)' + (s.liveOnly ? " AND collection:(etree)" : "");
    const rows = Number(s.items) || 6;
    const r = await OmniPulse.fetch(A + "/advancedsearch.php?q=" + encodeURIComponent(where) +
      "&fl%5B%5D=identifier&sort%5B%5D=downloads%20desc&rows=" + rows + "&output=json");
    if (!r.ok) throw new Error("archive.org search answered " + r.status);
    const ids = ((await r.json()).response.docs || []).map((d) => d.identifier);
    const lists = await Promise.all(ids.map(async (id) => {
      try {
        const m = await OmniPulse.fetch(A + "/metadata/" + encodeURIComponent(id));
        return m.ok ? tracksOf(id, await m.json(), s.lossless !== false).slice(0, 12) : [];
      } catch (e) { return []; }
    }));
    return lists.flat();
  },

  async resolve(track) {
    return { url: A + "/download/" + encodeURIComponent(track.item) + "/" + track.file.split("/").map(encodeURIComponent).join("/") };
  },
});
