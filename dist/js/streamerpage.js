// The Streamer Mode page: while Stream-Safe Music is on it takes the place of Discover, Rediscover,
// Library, Favourites, Sets & Podcasts and Artists, with collections of music that's safe on stream:
//   - each artist or label cleared for streaming (NCS, StreamBeats...), from every platform that's on
//   - Creative Commons music by genre, from SoundCloud (and the Internet Archive plugin, if added)
//   - your own likes that are safe
// Autoplay is on here unless you switch it off here, and only adds from these collections.

const STREAM_GENRES = ["Electronic", "Lo-fi", "Chill", "Hip-hop", "Ambient", "Rock", "Synthwave", "Acoustic"];
const streamColl = new Map();   // collection id -> its tracks, once made

async function streamSearchAll(q) {
  const plats = activePlats("search");
  const jobs = [];
  if (plats.includes("soundcloud")) jobs.push(api("/tracks?q=" + encodeURIComponent(q) + "&access=playable&limit=50&linked_partitioning=true").then(asList));
  if (plats.includes("spotify") && (spotifyPremium || spotifyFree)) jobs.push(spotifySearchTracks(q));
  if (plats.includes("youtube")) jobs.push(ytSearch(q));
  plats.filter((p) => p.startsWith("plug:")).forEach((p) => jobs.push(pluginSearch(p.slice(5), q)));
  const got = await Promise.allSettled(jobs);
  return got.filter((r) => r.status === "fulfilled").flatMap((r) => r.value);
}

// Only what's safe, each song once, in a mixed order
async function streamKeep(list) {
  await streamLookups(list);
  const seen = new Set();
  return shuffleInPlace(list.filter((t) => t && !isMix(t) && streamOk(t) && !seen.has(t.id) && seen.add(t.id)));
}

// An artist or label cleared for streaming: its names searched everywhere, then only what it clears
async function streamCleared(e) {
  const names = [...new Set(e.artists.concat(e.labels))].slice(0, 4);
  const lists = await Promise.all(names.map((n) => streamSearchAll(n).catch(() => [])));
  return streamKeep(lists.flat());
}

// Creative Commons music of a genre: SoundCloud's uploads with an open licence (and plugins' that say so)
async function streamGenre(g) {
  const qs = [g, g + " creative commons", g + " royalty free", g + " no copyright"];
  const lists = await Promise.all(qs.map((q) => streamSearchAll(q).catch(() => [])));
  return streamKeep(lists.flat().filter(openLicence));
}

async function streamLikes() {
  return streamKeep(libraryList(activePlats("likes")));
}

// The page's collections, as tiles (made when first opened or played)
function streamCollections() {
  const coll = (id, name, sub, desc, build) => ({
    kind: "genre", safe: true, plats: activePlats("search").filter((p) => PLATFORMS[p]), name, hue: hueOf(name), sub,
    mix: makeMix({ id: "stream:" + id, title: name, desc, build: async () => {
      if (!streamColl.has(id)) streamColl.set(id, await build());
      return streamColl.get(id);
    } }),
  });
  const cleared = clearedEntries().filter((e) => e.on).map((e) =>
    coll("cl:" + e.id, e.name, "Cleared for streaming", e.name + ": " + e.note, () => streamCleared(e)));
  const genres = STREAM_GENRES.map((g) => coll("cc:" + g, g, "Creative Commons", "Creative Commons " + g + ": free to play on stream, by the uploaders' own licence.", () => streamGenre(g)));
  return [
    { title: "Cleared for streaming", tiles: cleared },
    { title: "Creative Commons", tiles: genres },
    { title: "From your likes", tiles: [coll("likes", "Your Safe Likes", "From your likes", "Your likes that are safe on stream.", streamLikes)] },
  ].filter((s) => s.tiles.length);
}

let streamShelves = null;
function openStreamer() {
  currentPlaylist = null;
  currentArtist = null;
  showView("streamer");
  ++loadToken;
  viewTracks = [];
  nextHref = "";
  render();
  if (!streamShelves) streamShelves = streamCollections();
  paintFeed(streamShelves);
  say("");
}

// Autoplay in Streamer Mode: from the collections only (those made already, and a couple more if
// there's little to pick from), never what's queued already
async function streamAutoplayPool() {
  const all = (streamShelves || streamCollections()).flatMap((s) => s.tiles);
  let have = [...streamColl.values()].flat();
  if (have.length < 30) {
    const todo = shuffleInPlace(all.filter((t) => !streamColl.has(t.mix.id.slice(7)))).slice(0, 2);
    await Promise.all(todo.map((t) => t.mix.build().catch(() => [])));
    have = [...streamColl.values()].flat();
  }
  const queued = new Set(queue.map((t) => t.id));
  return shuffleInPlace(have.filter((t) => !queued.has(t.id) && recentOk(t)));
}

// The pages Streamer Mode stands in for: hidden while it's on (body.streamSafe, in the CSS); being
// on one of them when it's switched on goes to Streamer Mode, and switched off, back to the Library
const STREAM_HIDES = ["discover", "rediscover", "likes", "favourites", "sets", "artists", "playlist", "artist"];
function streamerPageSync(on) {
  streamShelves = null;
  if (on && STREAM_HIDES.includes(view)) openStreamer();
  else if (!on && view === "streamer") openLikes();
}

function streamerPageInit() {
  $("navStreamer").addEventListener("click", openStreamer);
}
