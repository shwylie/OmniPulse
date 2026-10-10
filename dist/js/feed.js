// Discover, as a feed of shelves: Spotify's own Daily Mixes and mixes, the app's cross-platform
// Your Mixes and artist stations (seeded by what you like on SoundCloud), more of what you like,
// new tracks from artists you follow, and genres. Each tile shows inside its cover which platforms
// it draws on. Tiles open as a playlist page (built when first opened), or play from their button.
// Spotify's mixes are read with the playback engine's token (developer apps can't see them).

const GENRES = ["Electronic", "House", "Techno", "Dubstep", "Drum & Bass", "Hip-hop & Rap", "Pop", "Indie", "R&B", "Ambient", "Lo-fi", "Rock"];
const MIX_BANDS = ["#3ee0d0", "#f5e663", "#ff5a4e", "#f28fd6", "#9ad94b", "#ffae4a"];   // Daily Mix label colours

let feedCache = null;   // { key, shelves } so going back to Discover shows the same feed

const shuffledCopy = (list) => shuffleInPlace(list.slice());
const dedupeTracks = (list) => { const seen = new Set(); return list.filter((t) => t && !seen.has(t.id) && seen.add(t.id)); };
const freshPick = (t) => !isMix(t) && langOk(t) && recOk(t);
const hueOf = (s) => [...String(s)].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);

// Already in your likes: that very track, or the same song liked on any platform (by its name)
function likedSongs() {
  const keys = new Set();
  const add = (t) => { const k = songKey(t); if (k) keys.add(k); };
  likedTracks.filter(isLikedTrack).forEach(add);
  (spLikedCache || []).forEach(add);
  ytLikedTracks().forEach(add);
  return keys;
}
// Anything liked or starred, on any platform (your music files don't count: they say nothing about taste)
const hasLikes = () => favs.length > 0 || [...likeState.values()].some(Boolean) || likedTracks.some(isLikedTrack) || (spLikedCache || []).length > 0 || ytLikedTracks().length > 0;
// The "Get better recommendations" card on Discover and Rediscover, while there are none
function paintLikesPrompt() {
  const v = view === "now" ? back : view;
  $("likesPrompt").hidden = !["discover", "rediscover"].includes(v) || hasLikes();
}
const notLiked = (keys) => (t) => !isLikedTrack(t) && !keys.has(songKey(t));

// ---------- your top artists, from your likes on every platform and what you play ----------
// One entry per artist, whichever platforms you like them on, so a mix can draw on all of them.
// (one artist however it's written: "The Glitch Mob" and "Glitch Mob", "Above & Beyond" and "Above and
// Beyond", "Tiësto" and "Tiesto")
const artistKey = (s) => String(s || "").toLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "")
  .replace(/&/g, " and ").replace(/^\s*the\s+(?=\S)/, "").replace(/[^a-z0-9À-ɏ]+/g, "");

function feedArtists(plats) {
  const plays = new Map();
  loadStats().forEach((x) => { const k = artistKey(x.artist); plays.set(k, (plays.get(k) || 0) + 1); });
  const by = new Map();
  const add = (plat, t) => {
    const name = plat === "spotify" ? firstArtist(t) : artistName(t);
    const k = artistKey(name);
    if (!k || isMix(t) || hiddenArtists.has(k)) return;
    if (!by.has(k)) by.set(k, { key: k, name, liked: { soundcloud: [], spotify: [], youtube: [] } });
    by.get(k).liked[plat].push(t);
  };
  if (plats.includes("soundcloud")) likedTracks.filter(isLikedTrack).forEach((t) => add("soundcloud", t));
  if (plats.includes("spotify")) (spLikedCache || []).forEach((t) => add("spotify", t));
  if (plats.includes("youtube")) ytLikedTracks().forEach((t) => add("youtube", t));
  const local = plats.includes("local") ? localTracks : [];
  return [...by.values()]
    // Spotify makes its own mixes now, so only artists you like on SoundCloud seed the app's ones
    .filter((a) => a.liked.soundcloud.length)
    .map((a) => Object.assign(a, {
      files: local.filter((t) => artistKey(artistName(t)) === a.key),
      score: (a.liked.soundcloud.length + a.liked.spotify.length + a.liked.youtube.length) * 2 + (plays.get(a.key) || 0),
    }))
    .sort((x, y) => y.score - x.score);
}

// Without SoundCloud likes to go on (no accounts at all, say): the artists you star and play on this
// PC, most first (a star counts as three plays). One play is enough to start from.
const HISTORY_MIN = 1;

function historyArtists(plats) {
  const by = new Map();
  const add = (name, w, t, art) => {
    const k = artistKey(name);
    if (!k || hiddenArtists.has(k) || (t && isMix(t))) return;
    if (!by.has(k)) by.set(k, { key: k, name, liked: { soundcloud: [], spotify: [], youtube: [] }, known: [], files: [], score: 0, art: "" });
    const a = by.get(k);
    a.score += w;
    if (t) a.known.push(t);
    if (!a.art) a.art = (t && artUrl(t, "t500x500")) || art || "";
  };
  favs.forEach((t) => add(guessNames(t).artist || firstArtist(t), 3, t));
  if (plats.includes("youtube")) ytLikedTracks().forEach((t) => add(firstArtist(t), 2, t));
  loadStats().forEach((x) => add(String(x.artist || "").split(",")[0].trim(), 1, null, x.art));
  const local = plats.includes("local") ? localTracks : [];
  return [...by.values()]
    .filter((a) => a.score >= HISTORY_MIN)
    .map((a) => Object.assign(a, { files: local.filter((t) => artistKey(artistName(t)) === a.key) }))
    .sort((x, y) => y.score - x.score);
}

// Popular artists right now (Deezer's chart), as stations: Discover's start before you've played anything
async function popularArtists() {
  const d = await dzGet("/chart/0/artists?limit=20");
  return (d.data || []).filter((x) => x && x.name && !hiddenArtists.has(artistKey(x.name))).map((x) => ({
    key: artistKey(x.name), name: x.name, liked: { soundcloud: [], spotify: [], youtube: [] }, known: [], files: [], score: 0,
    art: x.picture_big || x.picture_xl || "", popular: true,
  }));
}

// ---------- genres you've said you like (Pick genres, on Discover) ----------
// Deezer's genres, by its own ids. Picked ones start Discover off before there's anything to go on:
// stations for each genre's best-known artists, its chart as a spotlight, and its tile first.
const TASTE_GENRES = [["Pop", 132], ["Hip-hop & Rap", 116], ["Rock", 152], ["Dance", 113], ["Electronic", 106], ["R&B", 165],
  ["Alternative & Indie", 85], ["Soul & Funk", 169], ["Jazz", 129], ["Country", 84], ["Folk", 466], ["Metal", 464],
  ["Latin", 197], ["Reggae", 144], ["Blues", 153], ["Classical", 98], ["K-pop & Asian", 16], ["African", 2],
  ["Brazilian", 75], ["Indian", 81], ["Film & Game", 173]];
const tasteGenres = () => (Array.isArray(prefs.tasteGenres) ? prefs.tasteGenres : []).filter((g) => TASTE_GENRES.some(([n]) => n === g));
const tasteId = (g) => (TASTE_GENRES.find(([n]) => n === g) || [])[1];

// Subgenres, for fine-tuning once you've picked genres (Pick genres, then Fine-tune). Deezer has no
// charts for them, so each is made from Deezer's playlists of that name.
const SUBGENRES = {
  "Pop": ["Synth-pop", "Dance Pop", "Indie Pop", "Hyperpop", "80s Pop", "Teen Pop", "Electropop"],
  "Hip-hop & Rap": ["Trap", "Boom Bap", "Drill", "Conscious Hip-hop", "Lo-fi Hip-hop", "Old School Hip-hop", "UK Rap", "Cloud Rap"],
  "Rock": ["Classic Rock", "Indie Rock", "Punk", "Pop Punk", "Grunge", "Hard Rock", "Prog Rock", "Emo", "Post-rock"],
  "Dance": ["House", "Deep House", "Tech House", "Nu-disco", "EDM", "Eurodance", "Big Room", "UK Garage"],
  "Electronic": ["Dubstep", "Electro House", "Drum & Bass", "Techno", "Trance", "Future Bass", "Melodic Bass", "Riddim",
    "Synthwave", "Hardstyle", "Glitch Hop", "Liquid DnB", "Chillstep", "Ambient", "Lo-fi", "IDM", "Breakbeat"],
  "R&B": ["Neo-soul", "Contemporary R&B", "90s R&B", "Alternative R&B", "Slow Jams"],
  "Alternative & Indie": ["Shoegaze", "Dream Pop", "Post-punk", "Indie Folk", "Bedroom Pop", "Math Rock", "Garage Rock"],
  "Soul & Funk": ["Motown", "Funk", "Northern Soul", "Disco", "Neo-soul"],
  "Jazz": ["Smooth Jazz", "Bebop", "Jazz Fusion", "Vocal Jazz", "Nu Jazz", "Swing", "Acid Jazz"],
  "Country": ["Classic Country", "Country Pop", "Americana", "Bluegrass", "Outlaw Country"],
  "Folk": ["Indie Folk", "Celtic", "Singer-songwriter", "Folk Rock"],
  "Metal": ["Metalcore", "Death Metal", "Black Metal", "Thrash Metal", "Nu Metal", "Power Metal", "Djent", "Doom Metal"],
  "Latin": ["Reggaeton", "Salsa", "Bachata", "Latin Trap", "Regional Mexican", "Cumbia", "Latin Pop"],
  "Reggae": ["Dancehall", "Dub", "Roots Reggae", "Lovers Rock"],
  "Blues": ["Delta Blues", "Chicago Blues", "Blues Rock", "Electric Blues"],
  "Classical": ["Baroque", "Romantic", "Solo Piano", "Opera", "Contemporary Classical", "Minimalism"],
  "K-pop & Asian": ["K-pop", "J-pop", "City Pop", "C-pop", "J-rock", "Anime Songs"],
  "African": ["Afrobeats", "Amapiano", "Highlife", "Afro House"],
  "Brazilian": ["Bossa Nova", "Samba", "MPB", "Funk Carioca", "Sertanejo"],
  "Indian": ["Bollywood", "Punjabi", "Carnatic", "Indie Hindi"],
  "Film & Game": ["Film Scores", "Video Game Music", "Epic Orchestral", "Anime Soundtracks"],
};
// The subgenres you picked, of the genres you still have picked
const tasteSubgenres = () => {
  const ok = new Set(tasteGenres().flatMap((g) => SUBGENRES[g] || []));
  return (Array.isArray(prefs.tasteSubgenres) ? prefs.tasteSubgenres : []).filter((s) => ok.has(s));
};

// A subgenre's music: Deezer's two best-matching playlists of that name (real playlists of 15
// songs or more), taking turns, shuffled a little
async function dzSubgenre(name) {
  const found = ((await dzGet("/search/playlist?limit=10&q=" + encodeURIComponent(name))).data || []).filter((p) => (p.nb_tracks || 0) >= 15).slice(0, 2);
  const lists = await eachFew(found, async (p) => ((await dzGet("/playlist/" + p.id + "/tracks?limit=60")).data || []).map((x) => dzTrack(x)), 2);
  return recFilter(dedupeTracks(interleave(lists.filter(Boolean).map(shuffledCopy))));
}

// The artists of the genres you picked, as stations: the ones most often in each genre's chart (Deezer's
// own genre artist lists are the same overall chart for every genre), the genres taking turns
async function tasteArtists() {
  const lists = await eachFew(tasteGenres(), async (g) => {
    const by = new Map();
    ((await dzGet("/chart/" + tasteId(g) + "/tracks?limit=50")).data || []).forEach((t) => {
      const x = t.artist;
      if (!x || !x.name) return;
      const k = artistKey(x.name);
      if (!by.has(k)) by.set(k, { n: 0, a: { key: k, name: x.name, liked: { soundcloud: [], spotify: [], youtube: [] }, known: [], files: [], score: 0,
        art: x.picture_big || x.picture_xl || "", popular: true, genre: g } });
      by.get(k).n++;
    });
    return [...by.values()].sort((x, y) => y.n - x.n).slice(0, 8).map((x) => x.a);
  }, 3);
  const seen = new Set();
  return interleave(lists.filter(Boolean)).filter((a) => a.key && !hiddenArtists.has(a.key) && !seen.has(a.key) && seen.add(a.key));
}

// Nothing to go on yet: no likes, no favourites, nothing played, no genres picked
const coldStart = () => !hasLikes() && !loadStats().length && !tasteGenres().length;

// Discover's order: new releases, then the app's own mixes and stations, new for you and the genres;
// Deezer's charts last. Before there's anything to go on: the genres first, with popular artists and
// charts to start from.
const FEED_ORDER = ["releases", "mixes", "spmixes", "stations", "taste", "foryou", "subgenres", "genres", "top", "popular", "spotlight"];
const COLD_ORDER = ["genres", "popular", "spotlight", "top", "foryou"];
function orderFeed(shelves) {
  const order = coldStart() ? COLD_ORDER : FEED_ORDER;
  const rank = (s) => { const i = order.indexOf(s.key); return i < 0 ? order.length : i; };
  return shelves.map((s, i) => [s, i]).sort((a, b) => rank(a[0]) - rank(b[0]) || a[1] - b[1]).map(([s]) => s);
}

const scSeeded = (crew) => crew.some((a) => a.liked.soundcloud.length);
const useCatalog = (crew, plats) => plats.includes("catalog") && !scSeeded(crew);

// Which platforms a mix of these artists will draw on
function crewPlats(crew, plats) {
  const out = scSeeded(crew) ? ["soundcloud"] : [];
  if (useCatalog(crew, plats)) out.push("deezer");
  if (plats.includes("spotify")) out.push("spotify");
  if (plats.includes("youtube")) out.push("youtube");
  if (crew.some((a) => a.files.length)) out.push("local");
  return out;
}

// ---------- building a mix's tracks (when it's first opened or played) ----------
async function scRelatedTo(tracks, n) {
  const lists = await Promise.all(shuffledCopy(tracks).slice(0, n).map((t) =>
    api("/tracks/" + t.id + "/related?access=playable&limit=20&linked_partitioning=true").then(asList).catch(() => [])));
  return lists.flat();
}

async function spSimilarTracks(name, n) {
  const names = shuffledCopy(await lfmSimilarNames(name, 10)).slice(0, n);
  return (await Promise.all(names.map(spArtistTracks))).flat();
}

// New tracks in the vein of these artists, from every platform that's on
async function freshFor(crew, plats) {
  const sc = scRelatedTo(crew.flatMap((a) => a.liked.soundcloud), 4);
  const sp = plats.includes("spotify")
    ? Promise.all([spSimilarTracks(crew[0].name, 3), ...crew.map((a) => spArtistTracks(a.name))]).then((l) => l.flat())
    : Promise.resolve([]);
  // YouTube Music's radio from the first artist's best-known song
  const yt = plats.includes("youtube")
    ? ytSearch(crew[0].name).then((found) => (found[0] ? ytRadio(found[0].ytId) : [])).catch(() => [])
    : Promise.resolve([]);
  // Deezer's related artists' best-known songs (played from YouTube Music), for mixes that aren't
  // seeded by SoundCloud likes
  const cat = useCatalog(crew, plats)
    ? Promise.all(crew.slice(0, 3).map((a) => dzSimilarTracks(a.name).catch(() => []))).then((l) => l.flat())
    : Promise.resolve([]);
  const [a, b, c, d] = await Promise.all([sc, sp, yt, cat]);
  const fresh = notLiked(likedSongs());
  const known = new Set(crew.map((x) => x.key));
  // (the crew's own songs come in as the familiar ones, not as new)
  const others = (t) => !(useCatalog(crew, plats) && known.has(artistKey(firstArtist(t))));
  return withoutRecent(interleave([shuffledCopy(a), shuffledCopy(b), shuffledCopy(c), shuffledCopy(d)]).filter((t) => freshPick(t) && fresh(t) && others(t)));
}

// Your Mix: mostly new music in the vein of these artists, with some you like by them (on any
// platform, and your files) woven in: two new ones for each one you know
async function buildYourMix(crew, plats) {
  const familiar = shuffledCopy(crew.flatMap((a) => a.liked.soundcloud.concat(a.liked.spotify, a.liked.youtube, a.known || [], a.files))).slice(0, 12);
  const fresh = await freshFor(crew, plats);
  const out = [];
  for (let i = 0, k = 0; i < fresh.length || k < familiar.length; ) {
    if (i < fresh.length) out.push(fresh[i++]);
    if (i < fresh.length) out.push(fresh[i++]);
    if (k < familiar.length) out.push(familiar[k++]);
  }
  return dedupeTracks(out).slice(0, 60);
}

// An artist station: that sound, mostly from other artists
async function buildStation(a, plats) {
  let own = shuffledCopy(a.liked.soundcloud.concat(a.liked.spotify, a.liked.youtube, a.known || [], a.files)).slice(0, 4);
  // (nothing of theirs you know yet: a few of their best-known songs to start)
  if (!own.length && useCatalog([a], plats)) own = await dzArtistFor(a.name).then((d) => (d ? dzGet("/artist/" + d.id + "/top?limit=4") : { data: [] })).then((r) => (r.data || []).map((x) => dzTrack(x))).catch(() => []);
  return dedupeTracks(own.concat(await freshFor([a], plats))).slice(0, 60);
}

// The same mix with only music you haven't liked (I'm Feeling Lucky)
async function onlyNew(build) {
  const keep = notLiked(likedSongs());
  return dedupeTracks((await build()).filter((t) => !isMix(t) && keep(t)));
}

// A genre, from every active platform
async function buildGenre(g, plats) {
  const results = await Promise.allSettled(plats.filter((p) => DISCOVER_PICKERS[p]).map((p) => DISCOVER_PICKERS[p]("genre:" + g)));
  return dedupeTracks(interleave(results.filter((r) => r.status === "fulfilled").map((r) => shuffledCopy(r.value.list))));
}

// ---------- the shelves ----------
function makeMix(o) {
  return Object.assign({ source: "mix", tracks: null }, o);
}

// One of Spotify's own playlists (a Daily Mix, Discover Weekly, ...) as a tile
function spotifyTile(p) {
  const imgs = p.images || [];
  return {
    kind: "playlist", plats: ["spotify"], name: p.name, art: imgs.length ? imgs[0].url : "",
    sub: (p.description || "Made by Spotify").replace(/<[^>]+>/g, ""),
    mix: makeMix({ id: "mix:spotify:" + p.id, title: p.name, desc: (p.description || "Made for you by Spotify.").replace(/<[^>]+>/g, ""),
      build: () => spotifyPlaylistTracks(p.id, 200) }),
  };
}

async function buildFeed(plats) {
  if (plats.includes("soundcloud") || plats.includes("spotify")) await loadLikesFor(plats).catch(() => null);
  const liked = feedArtists(plats);
  // (no SoundCloud likes: what you star and play here instead)
  const artists = liked.length ? liked : historyArtists(plats);
  const fromSpotify = plats.includes("spotify") ? await spotifyMadeForYou().catch(() => ({ daily: [], other: [] })) : { daily: [], other: [] };
  const shelves = [];

  // Made for you: Spotify's Daily Mixes, then Your Mix 01-06 (one of your top artists plus two more)
  const mains = artists.slice(0, 6);
  const yours = mains.map((a, i) => {
    const crew = [a].concat(artists.filter((x) => !mains.includes(x)).slice(i * 2, i * 2 + 2));
    const n = String(i + 1).padStart(2, "0");
    const who = crew.map((x) => x.name).join(", ") + " and more";
    return {
      kind: "daily", band: MIX_BANDS[i % MIX_BANDS.length], number: n, plats: crewPlats(crew, plats),
      art: a.art || (a.liked.soundcloud[0] ? artUrl(a.liked.soundcloud[0], "t500x500") : ""), sub: who,
      mix: makeMix({ id: "mix:yours:" + a.key, title: "Your Mix " + n,
        desc: who + ": " + (scSeeded(crew) ? "tracks you like" : "songs you play") + ", mixed with new ones from " + crewPlats(crew, plats).map((p) => PLATFORMS[p].name).join(", ") + ".",
        build: () => buildYourMix(crew, plats), fresh: () => onlyNew(() => freshFor(crew, plats)) }),
    };
  });
  const madeForYou = fromSpotify.daily.map(spotifyTile).concat(yours);
  if (madeForYou.length) shelves.push({ key: "mixes", title: "Made for you", tiles: madeForYou });

  // Spotify's other mixes: Discover Weekly, Release Radar, your top mixes...
  if (fromSpotify.other.length) shelves.push({ key: "spmixes", title: "More mixes from Spotify", tiles: fromSpotify.other.map(spotifyTile) });

  const stationTile = (a) => ({
    kind: "station", plats: crewPlats([a], plats), art: a.art || (a.liked.soundcloud[0] ? artUrl(a.liked.soundcloud[0], "t500x500") : ""), name: a.name, hue: hueOf(a.name),
    sub: a.genre ? a.genre + " station" : a.popular ? "Popular right now" : "Artist station",
    mix: makeMix({ id: "mix:station:" + a.key, title: a.name + " station",
      desc: "Music like " + a.name + ".", build: () => buildStation(a, plats), fresh: () => onlyNew(() => freshFor([a], plats)) }),
  });
  // Artist stations: the next artists you like, a little shuffled each time
  const stationArtists = shuffledCopy(artists.slice(6, 30)).slice(0, 8);
  if (stationArtists.length) {
    shelves.push({
      key: "stations",
      title: "Artist stations",
      tiles: stationArtists.map(stationTile),
    });
  }
  // Popular artists: stations to start from, for everyone using the catalogues (and the whole of
  // Discover, with Top songs and the genres, before you've played anything)
  if (plats.includes("catalog") && !liked.length) {
    const yours = new Set(artists.map((a) => a.key));
    const picked = tasteGenres().length ? (await tasteArtists().catch(() => [])).filter((a) => !yours.has(a.key)).slice(0, 16) : [];
    if (picked.length) shelves.push({ key: "taste", title: "Artists from the genres you like", tiles: picked.map(stationTile) });
    else {
      const popular = (await popularArtists().catch(() => [])).filter((a) => !yours.has(a.key)).slice(0, 12);
      if (popular.length) shelves.push({ key: "popular", title: coldStart() ? "Popular artists to start with" : "Popular artists", tiles: popular.map(stationTile) });
    }
  }

  // Your subgenres: the ones you fine-tuned to, each its own mix
  const subs = plats.includes("catalog") ? tasteSubgenres() : [];
  if (subs.length) {
    shelves.push({
      key: "subgenres",
      title: "Your subgenres",
      tiles: subs.map((g) => ({
        kind: "genre", plats: ["deezer"], name: g, hue: hueOf(g), sub: "Mix",
        mix: makeMix({ id: "mix:sub:" + g, title: g, desc: g + ", from Deezer's playlists of it.",
          build: () => dzSubgenre(g), fresh: () => onlyNew(() => dzSubgenre(g)) }),
      })),
    });
  }

  // Trending by genre
  const genrePlats = plats.filter((p) => DISCOVER_PICKERS[p]);
  if (genrePlats.length) {
    // (the genres you picked first)
    const picked = tasteGenres();
    const order = picked.filter((g) => !GENRES.includes(g)).concat(GENRES.filter((g) => picked.includes(g)), GENRES.filter((g) => !picked.includes(g)));
    shelves.push({
      key: "genres",
      title: "Trending by genre",
      tiles: order.map((g) => ({
        kind: "genre", plats: genrePlats, name: g, hue: hueOf(g), sub: "Trending",
        mix: makeMix({ id: "mix:genre:" + g, title: g, desc: "Trending " + g + ", from " + genrePlats.map((p) => PLATFORMS[p].name).join(" and ") + ".",
          build: () => buildGenre(g, genrePlats), fresh: () => onlyNew(() => buildGenre(g, genrePlats)) }),
      })),
    });
  }
  return { shelves, artists };
}

// Shelves of single tracks, which need a moment to fetch; added below the others when ready
async function trackShelves(plats) {
  const out = [];
  const pickers = plats.filter((p) => DISCOVER_PICKERS[p]);
  const keep = notLiked(likedSongs());
  // New releases: albums and singles from your artists (Deezer) and new tracks from those you follow
  // (SoundCloud), one shelf, newest first
  const rel = plats.includes("catalog") ? await newReleasesShelf().catch(() => null) : null;
  let followed = [];
  if (plats.includes("soundcloud")) {
    const f = await scDiscover("following").catch(() => null);
    followed = f ? f.list.filter(keep).slice(0, 30) : [];
  }
  const dated = (rel ? rel.tiles.map((t) => [t.released || "", t]) : [])
    .concat(followed.map((t, i) => [String(t.created_at || "").slice(0, 10), { kind: "track", track: t, index: i, list: followed, plats: [srcOf(t)], sub: artistName(t) }]));
  if (dated.length) {
    out.push({ key: "releases", title: "New releases", notice: rel && rel.notice,
      tiles: dated.sort((a, b) => b[0].localeCompare(a[0])).map(([, t]) => t) });
  }
  if (plats.includes("catalog")) {
    const top = await dzChart().catch(() => []);
    if (top.length) out.push({ key: "top", title: "Top songs", tracks: top.filter(keep).slice(0, 30) });
    // (only before there's anything to go on, and no genres picked: otherwise the genre tiles below are
    // the same charts as mixes)
    if (coldStart()) {
      const seen = new Set(top.map((t) => t.id));
      const spot = (await dzChart(DZ_GENRES.pop).catch(() => [])).filter((t) => keep(t) && !seen.has(t.id)).slice(0, 20);
      if (spot.length) out.push({ key: "spotlight", title: "Pop spotlight", tracks: spot });
    }
  }
  const likes = await Promise.allSettled(pickers.map((p) => DISCOVER_PICKERS[p]("foryou")));
  const more = dedupeTracks(interleave(likes.filter((r) => r.status === "fulfilled").map((r) => shuffledCopy(r.value.list)))).filter(keep).slice(0, 30);
  if (more.length) out.push({ key: "foryou", title: "New for you", tracks: more });
  return out;
}

// ---------- drawing ----------
function platBadges(plats) {
  const box = el("span", "tileBadges");
  plats.forEach((p) => {
    const b = el("span", "tileBadge");
    b.title = PLATFORMS[p].name;
    b.append(platIcon(p));
    box.append(b);
  });
  return box;
}

function tileEl(tile) {
  if (tile.track) applyFix(tile.track);
  const node = el("div", "tile tile-" + tile.kind + (tile.safe || (tile.track && streamOk(tile.track)) ? "" : " notSafe"));
  node._tile = tile;
  const art = el("div", "tileArt");
  if (tile.kind === "daily") {
    if (tile.art) { const img = el("img"); img.alt = ""; img.loading = "lazy"; img.src = tile.art; art.append(img); }
    const band = el("span", "tileBand");
    band.style.setProperty("--band", tile.band);
    band.append(el("span", "", "Your Mix"), el("b", "", tile.number));
    art.append(band);
  } else if (tile.kind === "station") {
    art.style.setProperty("--hue", tile.hue);
    if (tile.art) { const img = el("img", "tileRound"); img.alt = ""; img.loading = "lazy"; img.src = tile.art; art.append(img); }
    const lab = el("span", "tileLabel");
    lab.append(el("i", "", "Station"), el("span", "", tile.name));
    art.append(lab);
  } else if (tile.kind === "playlist") {
    if (tile.art) { const img = el("img"); img.alt = ""; img.loading = "lazy"; img.src = tile.art; art.append(img); }
  } else if (tile.kind === "artist") {
    art.style.setProperty("--hue", tile.hue);
    if (tile.art) { const img = el("img", "tileRound"); img.alt = ""; img.loading = "lazy"; img.src = tile.art; art.append(img); }
    else art.append(el("span", "tileInitial", (tile.name || "?").trim().charAt(0).toUpperCase()));
  } else if (tile.kind === "genre") {
    art.style.setProperty("--hue", tile.hue);
    art.append(el("span", "tileGenre", tile.name));
  } else {
    const src = artUrl(tile.track, "t500x500");
    if (src) { const img = el("img"); img.alt = ""; img.loading = "lazy"; img.src = src; img.onerror = () => img.removeAttribute("src"); art.append(img); }
  }
  art.append(platBadges(tile.plats));
  if (tile.isNew) art.append(el("span", "tileNew", "New"));
  const play = el("button", "tilePlay");
  play.setAttribute("aria-label", "Play");
  play.title = "Play";
  play.append(icon(ICON_PLAY));
  art.append(play);
  node.append(art,
    el("div", "tileTitle", tile.kind === "track" ? tile.track.title || "Untitled" : tile.kind === "daily" ? tile.mix.title : tile.name),
    el("div", "tileSub", tile.sub));
  return node;
}

// A shelf moves sideways with its own buttons and bar, not the scroll wheel (which scrolls the page)
function shelfEl(shelf) {
  const sec = el("section", "shelf");
  const head = el("div", "shelfHead");
  const all = el("button", "shelfAll", "Show all");
  const nav = el("div", "shelfNavs");
  const row = el("div", "shelfRow");
  const tiles = shelf.tiles || shelf.tracks.map((t, i) => ({
    kind: "track", track: t, index: i, list: shelf.tracks, plats: [srcOf(t)], sub: artistName(t),
  }));
  tiles.forEach((t) => row.append(tileEl(t)));
  const prev = el("button", "shelfNav");
  const next = el("button", "shelfNav");
  prev.setAttribute("aria-label", "Scroll left");
  next.setAttribute("aria-label", "Scroll right");
  prev.title = "Back";
  next.title = "More";
  prev.append(icon("M15.4 7.4L14 6l-6 6 6 6 1.4-1.4L10.8 12z"));
  next.append(icon("M8.6 16.6L10 18l6-6-6-6-1.4 1.4 4.6 4.6z"));
  nav.append(prev, next);
  head.append(el("h2", "", shelf.title), nav, all);
  // the bar: where in the row you are, and how much of it shows; click or drag it to move
  const bar = el("div", "shelfBar");
  const thumb = el("div", "shelfThumb");
  bar.append(thumb);
  const smooth = () => (prefs.animOn === false ? "auto" : "smooth");
  const paintNav = () => {
    const over = row.scrollWidth > row.clientWidth + 2 && !sec.classList.contains("expanded");
    nav.hidden = !over;
    bar.hidden = !over;
    if (!over) return;
    prev.disabled = row.scrollLeft <= 2;
    next.disabled = row.scrollLeft + row.clientWidth >= row.scrollWidth - 2;
    thumb.style.width = (row.clientWidth / row.scrollWidth) * 100 + "%";
    thumb.style.left = (row.scrollLeft / row.scrollWidth) * 100 + "%";
  };
  const page = (d) => row.scrollBy({ left: d * row.clientWidth * 0.9, behavior: smooth() });
  prev.addEventListener("click", () => page(-1));
  next.addEventListener("click", () => page(1));
  row.addEventListener("scroll", paintNav, { passive: true });
  bar.addEventListener("pointerdown", (e) => {
    const r = bar.getBoundingClientRect();
    const span = row.scrollWidth - row.clientWidth;
    if (e.target !== thumb) {
      // a click beside the thumb centres the view there
      const frac = (e.clientX - r.left) / r.width;
      row.scrollTo({ left: frac * row.scrollWidth - row.clientWidth / 2, behavior: smooth() });
      return;
    }
    e.preventDefault();
    const x0 = e.clientX, left0 = row.scrollLeft;
    thumb.setPointerCapture(e.pointerId);
    bar.classList.add("dragging");
    const move = (m) => { row.scrollLeft = Math.max(0, Math.min(span, left0 + (m.clientX - x0) * row.scrollWidth / r.width)); };
    const up = () => { thumb.removeEventListener("pointermove", move); bar.classList.remove("dragging"); };
    thumb.addEventListener("pointermove", move);
    thumb.addEventListener("pointerup", up, { once: true });
    thumb.addEventListener("pointercancel", up, { once: true });
  });
  all.addEventListener("click", () => {
    const open = sec.classList.toggle("expanded");
    all.textContent = open ? "Show less" : "Show all";
    paintNav();
  });
  sec.append(head, row, bar);
  requestAnimationFrame(paintNav);
  if (typeof ResizeObserver === "function") new ResizeObserver(paintNav).observe(row);
  return sec;
}

function paintFeed(shelves) {
  $("feed").replaceChildren(...orderFeed(shelves).map(shelfEl));
}

// ---------- Pick genres: what you like, to start Discover off ----------
function tasteOpen() {
  const box = $("tasteChips");
  const on = new Set(tasteGenres());
  box.replaceChildren(...TASTE_GENRES.map(([g]) => {
    const b = el("button", "chip optChip" + (on.has(g) ? " on" : ""), g);
    b.type = "button";
    b.setAttribute("aria-pressed", on.has(g) ? "true" : "false");
    b.addEventListener("click", () => chipToggle(b));
    return b;
  }));
  $("tasteDlg").showModal();
}

const chipToggle = (b) => { const now = !b.classList.contains("on"); b.classList.toggle("on", now); b.setAttribute("aria-pressed", now ? "true" : "false"); };

function tasteSave(e) {
  e.preventDefault();
  prefs.tasteGenres = [...$("tasteChips").querySelectorAll(".chip.on")].map((b) => b.textContent);
  savePrefs();
  $("tasteDlg").close();
  paintTasteBtn();
  // then the second step, for the subgenres of the ones picked
  if (tasteGenres().some((g) => SUBGENRES[g])) { subOpen(); return; }
  tasteDone();
}

function tasteDone() {
  const subs = tasteSubgenres();
  say(tasteGenres().length ? "Thanks. Discover starts from " + (subs.length ? subs : tasteGenres()).join(", ") + " now." : "No genres picked.");
  openFeed(true);
}

// Fine-tune: the subgenres of each genre you picked, under its name
function subOpen() {
  const on = new Set(Array.isArray(prefs.tasteSubgenres) ? prefs.tasteSubgenres : []);
  $("subGroups").replaceChildren(...tasteGenres().filter((g) => SUBGENRES[g]).map((g) => {
    const box = el("div", "subGroup");
    const chips = el("div", "tasteChips");
    chips.append(...SUBGENRES[g].map((s) => {
      const b = el("button", "chip optChip" + (on.has(s) ? " on" : ""), s);
      b.type = "button";
      b.setAttribute("aria-pressed", on.has(s) ? "true" : "false");
      // (one that's under two genres is one choice: both its chips follow)
      b.addEventListener("click", () => { const now = !b.classList.contains("on"); $("subGroups").querySelectorAll(".chip").forEach((c) => { if (c.textContent === s && c.classList.contains("on") !== now) chipToggle(c); }); });
      return b;
    }));
    box.append(el("h3", "", g), chips);
    return box;
  }));
  $("subDlg").showModal();
}

function subSave(e) {
  e.preventDefault();
  prefs.tasteSubgenres = [...new Set([...$("subGroups").querySelectorAll(".chip.on")].map((b) => b.textContent))];
  savePrefs();
  $("subDlg").close();
  tasteDone();
}

function paintTasteBtn() {
  $("tasteBtn").textContent = tasteGenres().length ? "Your genres" : "Pick genres";
}

// ---------- opening and playing tiles ----------
async function mixTracks(mix) {
  if (!mix.tracks) mix.tracks = await mix.build();
  return mix.tracks;
}

async function openMix(p) {
  const tok = ++loadToken;
  $("listView").scrollTop = 0;
  if (!p.tracks) {
    viewTracks = [];
    render();
    say("Putting your mix together...");
  }
  try {
    const list = await mixTracks(p);
    if (tok !== loadToken) return;
    setLocalPool(list);
    render();
    say(list.length ? "" : "Nothing came back for this one. Try another, or refresh Discover.");
  } catch (e) {
    if (tok === loadToken) say("Couldn't make this mix: " + (e && e.message ? e.message : e));
  }
}

async function playTile(tile) {
  if (tile.play) { tile.play(); return; }
  if (tile.kind === "track") {
    // on Search and artist pages a track takes the playing one's place (see playFromPage)
    if ((view === "search" || view === "artist") && queue.length && qi >= 0) { queue[qi] = Object.assign({}, tile.track, { autoplay: undefined, requestedBy: undefined }); playCurrent(); return; }
    startList(tile.list, tile.index, false);
    return;
  }
  say("Putting your mix together...");
  try {
    const list = await mixTracks(tile.mix);
    if (!list.length) { say("Nothing came back for this one."); return; }
    say("");
    startList(list, 0, false, tile.mix.title || tile.name);
  } catch (e) {
    say("Couldn't make this mix: " + (e && e.message ? e.message : e));
  }
}

// ---------- the Discover page ----------
async function openFeed(fresh) {
  const plats = activePlats("discover");
  currentPlaylist = null;
  currentArtist = null;
  showView("discover");
  viewTracks = [];
  nextHref = "";
  render();
  const key = plats.join(",") + "|" + (prefs.recLang || "any");
  if (!fresh && feedCache && feedCache.key === key) { paintFeed(feedCache.shelves); return; }   // same feed when you come back
  $("listView").scrollTop = 0;
  const tok = ++loadToken;
  $("feed").replaceChildren(el("p", "feedNote", "Finding music for you..."));
  say("Finding music for you...");
  const { shelves } = await buildFeed(plats);
  if (tok !== loadToken) return;
  feedCache = { key, shelves };
  paintFeed(shelves);
  // These finish even if you open a tile meanwhile, so the feed is complete when you come back
  const later = await trackShelves(plats);
  if (!feedCache || feedCache.shelves !== shelves) return;   // refreshed since
  shelves.unshift(...later);   // (where each goes: orderFeed)
  if (view !== "discover") return;
  paintFeed(shelves);
  const notice = (later.find((s) => s.notice) || {}).notice;
  if (tok === loadToken) say(notice || (shelves.length ? "" : "Play or star a few songs, and Discover will fill up with music for you."), notice ? { force: true } : undefined);
}

// I'm Feeling Lucky: a random mix (one that draws on more than one platform, if there are any), made
// only of music you haven't liked anywhere. One that comes back too short is passed over for another.
async function feelingLucky() {
  if (!feedCache) { say("Finding music for you..."); await openFeed(); }
  const tiles = (feedCache ? feedCache.shelves : []).flatMap((s) => s.tiles || [])
    .filter((t) => t.mix && t.mix.fresh && t.kind !== "playlist");
  const mixed = tiles.filter((t) => t.plats.length > 1);
  const pool = shuffleInPlace((mixed.length ? mixed : tiles).slice());
  if (!pool.length) { say("Nothing to pick from yet. Like a few tracks, and Discover will fill up."); return; }
  for (const tile of pool.slice(0, 4)) {
    say("Feeling lucky: " + tile.mix.title + "...");
    let list = [];
    try { list = await tile.mix.fresh(); } catch (e) { list = []; }
    if (list.length < 8) continue;
    startList(list, 0, false, tile.mix.title + " (all new)");
    say("Feeling lucky: " + tile.mix.title + ", all music you haven't liked.");
    return;
  }
  say("Couldn't find enough new music for a mix just now. Try again in a bit.");
}

// Clicks on shelves (Discover's, and Search's): the play button plays; the tile opens (its own page,
// or a mix as a playlist); a track tile plays from there
function wireShelves(box) {
  box.addEventListener("click", (e) => {
    const node = e.target.closest(".tile");
    if (!node) return;
    const tile = node._tile;
    if (e.target.closest(".tilePlay") || tile.kind === "track") { playTile(tile); return; }
    if (tile.open) { tile.open(); return; }
    openPlaylist(tile.mix);
  });
  box.addEventListener("contextmenu", (e) => {
    const node = e.target.closest(".tile-track");
    if (!node) return;
    e.preventDefault();
    openTrackMenu(node._tile.track, e.clientX, e.clientY);
  });
}

function feedInit() {
  $("luckyBtn").addEventListener("click", feelingLucky);
  $("tasteBtn").addEventListener("click", tasteOpen);
  $("likesPromptGenres").addEventListener("click", tasteOpen);
  $("tasteForm").addEventListener("submit", tasteSave);
  $("tasteCancel").addEventListener("click", () => $("tasteDlg").close());
  $("subForm").addEventListener("submit", subSave);
  $("subSkip").addEventListener("click", () => { $("subDlg").close(); tasteDone(); });
  $("subBack").addEventListener("click", () => { $("subDlg").close(); tasteOpen(); });
  paintTasteBtn();
  $("likesPromptFind").addEventListener("click", () => { $("q").focus(); say("Search for songs you love, then like or star them."); });
  wireShelves($("feed"));
}
