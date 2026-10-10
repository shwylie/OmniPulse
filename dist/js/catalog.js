// The catalogue (Settings > Integrations): Deezer's and Apple's (iTunes) open music data, with no
// account or key, so search, Discover and artist pages know songs that aren't on your own platforms.
// Its songs play from YouTube Music (yt-dlp), matched by title, artist and length; the match is
// remembered, and Track preferences changes it (js/trims.js). The Rust side paces and remembers the
// requests (crates/core/src/catalog.rs).

const catOn = () => prefs.catOn !== false;
// Songs known from a catalogue rather than a platform: each plays from YouTube Music
const CATALOGUE_SOURCES = ["musicbrainz", "deezer", "itunes"];
const viaYouTube = (t) => CATALOGUE_SOURCES.includes(srcOf(t));

async function catSearch(q) {
  return invoke("cat_search", { query: String(q || "") });
}

// A Deezer request ("/artist/27/top?limit=10"): its catalogue, read only
async function dzGet(path) {
  return invoke("deezer_get", { path });
}

// Deezer's own tracks, as the app's (dz_track in catalog.rs, for lists that come as Deezer sends them)
function dzTrack(x, album) {
  const al = album || x.album || {};
  return {
    id: "dz:" + x.id, dzId: x.id, source: "deezer", title: x.title, user: { username: (x.artist && x.artist.name) || "" },
    dzArtistId: x.artist && x.artist.id, dzAlbumId: al.id, album: al.title || "", duration: (x.duration || 0) * 1000,
    artwork_url: al.cover_big || al.cover_xl || al.cover_medium || "", dzRank: x.rank, isrc: x.isrc || "",
    explicit: !!x.explicit_lyrics, release_date: x.release_date || al.release_date || "",
    permalink_url: x.link || "https://www.deezer.com/track/" + x.id,
  };
}

function catPaint() {
  $("catOn").checked = catOn();
  $("catNote").textContent = !catOn() ? "Off."
    : ytOn() ? "On: search, Discover and artist pages find songs in Deezer's and Apple's catalogues too, played from YouTube Music."
    : "Its songs play from YouTube Music: switch that on to use it.";
}

function catInit() {
  catPaint();
  $("catOn").addEventListener("change", () => setTimeout(catPaint, 0));   // (js/platforms.js switches it)
}

// ---------- Deezer's artists, by name ----------
// Which Deezer artist a name is (kept a month; one that isn't there is asked again after a week)
const DZ_ARTISTS_KEY = "sc.dzArtists";
let dzArtists = null;
function dzArtistsLoad() {
  if (!dzArtists) { try { dzArtists = JSON.parse(localStorage.getItem(DZ_ARTISTS_KEY) || "{}"); } catch (e) { dzArtists = {}; } }
  return dzArtists;
}

async function dzArtistFor(name) {
  const k = artistKey(name);
  if (!k) return null;
  const saved = dzArtistsLoad()[k];
  if (saved && Date.now() - saved.at < (saved.id ? 30 : 7) * 86400000) return saved.id ? saved : null;
  const found = ((await dzGet("/search/artist?limit=5&q=" + encodeURIComponent(name))).data || []);
  const a = found.find((x) => sameArtist(x.name, name));
  dzArtists[k] = a ? { id: a.id, name: a.name, img: a.picture_big || a.picture_xl || "", followers: a.nb_fan || 0, url: a.link || "https://www.deezer.com/artist/" + a.id, at: Date.now() } : { id: 0, at: Date.now() };
  try { localStorage.setItem(DZ_ARTISTS_KEY, JSON.stringify(dzArtists)); } catch (e) {}
  return a ? dzArtists[k] : null;
}

// The artists you listen to most: liked on any platform, and played (most first)
function yourArtists(n) {
  const score = new Map();
  const names = new Map();
  const add = (name, w) => {
    const k = artistKey(name);
    if (!k || hiddenArtists.has(k)) return;
    score.set(k, (score.get(k) || 0) + w);
    if (!names.has(k)) names.set(k, name);
  };
  likedTracks.filter(isLikedTrack).forEach((t) => !isMix(t) && add(guessNames(t).artist || artistName(t), 2));
  (spLikedCache || []).forEach((t) => add(firstArtist(t), 2));
  ytLikedTracks().forEach((t) => add(firstArtist(t), 2));
  loadStats().forEach((x) => add(String(x.artist || "").split(",")[0].trim(), 1));
  return [...score.entries()].sort((a, b) => b[1] - a[1]).slice(0, n).map(([k]) => names.get(k));
}

// A few at a time (Deezer allows about 10 a second)
async function eachFew(list, fn, n) {
  const out = new Array(list.length);
  let i = 0;
  await Promise.all(Array.from({ length: n || 4 }, async () => {
    while (i < list.length) { const k = i++; try { out[k] = await fn(list[k]); } catch (e) { out[k] = null; } }
  }));
  return out;
}

// ---------- Discover ----------
// Deezer's genre charts, for Trending by genre (the closest of its genres to each of the app's)
const DZ_GENRES = { electronic: 106, house: 113, techno: 106, dubstep: 106, "drum & bass": 106, "hip-hop & rap": 116, pop: 132, indie: 85, "r&b": 165, ambient: 106, rock: 152, jazz: 129, latin: 197, metal: 464, reggae: 144, "k-pop": 16, classical: 98, country: 84, soul: 165, alternative: 85, dance: 113, rap: 116, "hip-hop": 116 };

async function dzChart(genreId) {
  const d = await dzGet("/chart/" + (genreId || 0) + "/tracks?limit=50");
  return (d.data || []).map((x) => dzTrack(x));
}

// Top tracks of artists like this one (Deezer's related artists)
async function dzSimilarTracks(name) {
  const a = await dzArtistFor(name);
  if (!a) return [];
  const rel = ((await dzGet("/artist/" + a.id + "/related?limit=8")).data || []);
  const lists = await eachFew(shuffleInPlace(rel.slice()).slice(0, 5), async (r) => ((await dzGet("/artist/" + r.id + "/top?limit=6")).data || []).map((x) => dzTrack(x)));
  return lists.filter(Boolean).flat();
}

async function catDiscover(src) {
  if (src === "following") return { list: [] };
  if (src.startsWith("genre:")) {
    const g = src.slice(6).toLowerCase();
    const id = DZ_GENRES[g];
    const list = id ? await dzChart(id) : ((await dzGet("/search?limit=50&q=" + encodeURIComponent(g))).data || []).map((x) => dzTrack(x));
    return { list: recFilter(list) };
  }
  const cur = curTrack();
  const seeds = src === "similar" && cur ? [guessNames(cur).artist || artistName(cur)] : shuffleInPlace(yourArtists(12)).slice(0, 2);
  if (!seeds.length) throw new Error("Like or play a few songs first, and Discover will suggest more.");
  const lists = await eachFew(seeds, dzSimilarTracks, 2);
  return { list: recFilter(lists.filter(Boolean).flat()), note: "Similar to " + seeds.join(", ") };
}

// ---------- New releases from your artists ----------
// Albums, EPs and singles from the last few weeks by the artists you listen to most, newest first.
// Ones you haven't seen before are marked New, and said once when Discover opens.
const RELEASE_DAYS = 45;
const RELEASES_SEEN_KEY = "sc.releasesSeen";

function releaseWhen(date) {
  const d = new Date(date + "T00:00:00");
  const days = Math.round((Date.now() - d.getTime()) / 86400000);
  return days <= 0 ? "Today" : days === 1 ? "Yesterday" : days < 7 ? days + " days ago" : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

async function newReleases() {
  const names = yourArtists(30);
  const artists = (await eachFew(names, dzArtistFor, 4)).filter(Boolean);
  const since = new Date(Date.now() - RELEASE_DAYS * 86400000).toISOString().slice(0, 10);
  const today = new Date().toISOString().slice(0, 10);
  const lists = await eachFew(artists, async (a) => ((await dzGet("/artist/" + a.id + "/albums?limit=10")).data || [])
    .filter((x) => x.release_date && x.release_date >= since && x.release_date <= today)
    .map((x) => dzAlbumTile(x, a.name)), 4);
  const seen = new Set();
  return lists.filter(Boolean).flat().filter((a) => !seen.has(a.id) && seen.add(a.id)).sort((a, b) => b.released.localeCompare(a.released));
}

async function newReleasesShelf() {
  const list = await newReleases();
  if (!list.length) return null;
  let seen;
  try { seen = new Set(JSON.parse(localStorage.getItem(RELEASES_SEEN_KEY) || "[]")); } catch (e) { seen = new Set(); }
  const first = !seen.size;
  const fresh = list.filter((a) => !seen.has(a.id));
  const tiles = list.map((a) => ({
    kind: "playlist", name: a.title, art: a.art, plats: ["deezer"], isNew: !first && !seen.has(a.id),
    sub: a.artist + " · " + (a.kind && a.kind !== "Album" ? a.kind + " · " : "") + releaseWhen(a.released),
    open: () => openPlaylist(albumAsPlaylist(a)),
    play: () => playAlbum(a),
  }));
  const notice = fresh.length && !first ? plural(fresh.length, "new release", "new releases") + " from artists you listen to, at the top of Discover." : "";
  try { localStorage.setItem(RELEASES_SEEN_KEY, JSON.stringify([...seen, ...fresh.map((a) => a.id)].slice(-600))); } catch (e) {}
  return { title: "New releases from your artists", tiles, notice };
}
