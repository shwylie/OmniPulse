// Your taste: what Discover picks by. The platforms (Deezer, SoundCloud, YouTube Music, Spotify) only
// put songs forward; which of them make a mix, and in what order, is decided here, from what you like
// and listen to:
//   - an artist you like or play, with a song of theirs you haven't heard: up
//   - an artist put forward by more than one platform: up (they agree)
//   - a genre you listen to: up
//   - an artist you keep skipping: down
//   - a little chance, so a mix made again isn't the same; at most two by one artist near the top

// ---------- skips ----------
// A song you skip past (Next, a media key) after a few seconds but before it counts as a play (30 s
// or half of it): its artist, noted. Kept 90 days, only on this PC.
const SKIPS_KEY = "sc.skips";
const SKIPS_KEEP = 90 * 86400000;

function loadSkips() {
  try { return JSON.parse(localStorage.getItem(SKIPS_KEY) || "[]").filter((x) => Date.now() - x.ts < SKIPS_KEEP); } catch (e) { return []; }
}

function noteSkip(t, posMs, durMs) {
  if (!t || isMix(t) || isLikedTrack(t)) return;
  const counted = Math.min(30000, (durMs || 0) / 2 || 30000);
  if (posMs < 3000 || posMs >= counted) return;
  const a = artistKey(firstArtist(t));
  if (!a) return;
  const list = loadSkips();
  list.push({ a, ts: Date.now() });
  try { localStorage.setItem(SKIPS_KEY, JSON.stringify(list.slice(-2000))); } catch (e) {}
}

// ---------- the profile ----------
// (made once per Discover visit: likes on every platform, plays, skips, genres)
function tasteProfile() {
  const artists = new Map();
  const genres = new Map();
  const add = (m, k, w) => { if (k) m.set(k, (m.get(k) || 0) + w); };
  likedTracks.filter(isLikedTrack).forEach((t) => { if (!isMix(t)) { add(artists, artistKey(guessNames(t).artist || artistName(t)), 2); add(genres, artistKey(t.genre), 2); } });
  (spLikedCache || []).forEach((t) => add(artists, artistKey(firstArtist(t)), 2));
  ytLikedTracks().forEach((t) => add(artists, artistKey(firstArtist(t)), 2));
  loadStats().forEach((x) => { add(artists, artistKey(String(x.artist || "").split(",")[0]), 1); add(genres, artistKey(x.genre), 1); });
  const skips = new Map();
  loadSkips().forEach((x) => add(skips, x.a, 1));
  const topGenre = Math.max(1, ...genres.values());
  return { artists, skips, genres, topGenre };
}

function tasteScore(t, prof, votes) {
  const a = artistKey(firstArtist(t));
  let s = 1 + 0.6 * Math.max(0, (votes.get(a) || 1) - 1);
  const w = prof.artists.get(a) || 0;
  if (w) s += 1.2 * Math.log2(1 + w);
  const g = prof.genres.get(artistKey(t.genre)) || 0;
  if (g) s += 0.8 * (g / prof.topGenre);
  s /= 1 + 0.5 * (prof.skips.get(a) || 0);
  return s * (0.85 + Math.random() * 0.3);
}

// The platforms' lists (one each) made into one: best for you first, at most two by one artist until
// the rest have had their turn
function tastePick(lists, prof) {
  prof = prof || tasteProfile();
  const votes = new Map();
  lists.forEach((l) => new Set(l.map((t) => artistKey(firstArtist(t)))).forEach((a) => votes.set(a, (votes.get(a) || 0) + 1)));
  const scored = lists.flat().map((t) => [tasteScore(t, prof, votes), t]).sort((x, y) => y[0] - x[0]).map(([, t]) => t);
  const per = new Map();
  const first = [];
  const later = [];
  scored.forEach((t) => {
    const a = artistKey(firstArtist(t));
    const n = per.get(a) || 0;
    per.set(a, n + 1);
    (n < 2 ? first : later).push(t);
  });
  return first.concat(later);
}
