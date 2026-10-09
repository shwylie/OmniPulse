// Discover: recommendation language, Similar artists, and Discover on Spotify.

// ---------- recommendation language ----------
// SoundCloud doesn't record a track's language, so this guesses from the title and tags:
// other alphabets, accented letters and common words. Tracks it can't place are kept.
const LANGS = {
  en: { words: "the and you your my me i'm im don't dont with for of in on we is it be feel never night baby heart all this that what when like just got love so up down", chars: "" },
  es: { words: "el la los las que y de con para por mi tu te amor corazón corazon noche quiero sin una eres como más mas bebé nada vida mía mia contigo", chars: "ñ¿¡áéíóú" },
  pt: { words: "não nao você voce meu minha com pra para uma amor coração coracao sem é eu te saudade quero vida também tambem", chars: "ãõçâêô" },
  fr: { words: "le la les de des et je tu pour avec sans mon ma toi moi amour coeur cœur nuit c'est est pas une", chars: "àèùçœëêîï" },
  de: { words: "der die das und ich nicht mit für fur dich mich liebe herz nacht ein eine ist du wir", chars: "äöüß" },
};
Object.values(LANGS).forEach((L) => { L.set = new Set(L.words.split(" ")); });
const OTHER_SCRIPT = /[\u0370-\u03ff\u0400-\u04ff\u0590-\u05ff\u0600-\u06ff\u0900-\u097f\u0e00-\u0e7f\u3040-\u30ff\u3400-\u9fff\uac00-\ud7af]/;

function detectLang(t) {
  const text = ((t.title || "") + " " + (t.tag_list || "")).toLowerCase();
  if (OTHER_SCRIPT.test(text)) return "other";
  const words = text.split(/[^a-z\u00c0-\u024f\u0153']+/).filter(Boolean);
  let best = "unknown";
  let bestScore = 0;
  for (const [code, L] of Object.entries(LANGS)) {
    let s = 0;
    words.forEach((w) => { if (L.set.has(w)) s += 1; });
    for (const ch of text) if (L.chars && L.chars.includes(ch)) s += 2;
    if (s > bestScore) { bestScore = s; best = code; }
  }
  if (bestScore === 0) return "unknown";
  return bestScore >= 2 ? best : best + "?";   // "?" = only a weak hint
}


function recFilter(list) {
  list = withoutRecent(list.filter((t) => !isLikedTrack(t) && recOk(t)));
  const kept = list.filter(langOk);
  return kept;
}

// The Discover language picker; tracks it can't place are kept
function langOk(t) {
  const want = prefs.recLang || "any";
  if (want === "any") return true;
  const d = detectLang(t);
  return d === "unknown" || d.endsWith("?") || d === want;
}

// ---------- Discover pickers ----------
// Each active platform has a picker that returns { list, note, msg } for a chip, or throws a
// message to show. openDiscover (app.js) mixes what they find.
const SC_Q = "access=playable&linked_partitioning=true";

async function scDiscover(src) {
  if (src === "foryou") return scForYou();
  if (src === "similar") return scSimilar();
  if (src === "following") {
    const list = await collectPages("/me/followings/tracks?" + SC_Q + "&limit=50", 2).catch((e) => {
      throw new Error("Couldn't load new tracks from artists you follow (" + (e && e.message ? e.message : e) + ").");
    });
    if (!list.length) throw new Error("Follow some artists on SoundCloud and their new tracks will show up here.");
    return { list: recFilter(list.filter((t) => !isMix(t))) };
  }
  const g = encodeURIComponent(src.slice(6));
  let list = await collectPages("/tracks?genres=" + g + "&" + SC_Q + "&limit=50", 2).catch(() => []);
  if (!list.length) list = await collectPages("/tracks?q=" + g + "&" + SC_Q + "&limit=50", 2);
  return { list: recFilter(list.filter((t) => !isMix(t))) };
}

// For you: tracks related to a few of your likes, minus ones you already like
async function scForYou() {
  const likes = asList(await api("/me/likes/tracks?limit=30&" + SC_Q));
  if (!likes.length) throw new Error("Like a few tracks first, and Discover will suggest similar ones.");
  likes.forEach((t) => likeState.set(t.id, true));
  const picks = shuffleInPlace(likes.slice()).slice(0, 3);
  const lists = await Promise.all(picks.map((seed) =>
    api("/tracks/" + seed.id + "/related?" + SC_Q + "&limit=20").then(asList).catch(() => [])));
  const seen = new Set(likes.map((t) => t.id));
  const out = [];
  lists.flat().forEach((t) => { if (!seen.has(t.id)) { seen.add(t.id); out.push(t); } });
  const names = picks.map((x) => x.title).join(", ");
  return {
    list: recFilter(out.filter((t) => !isMix(t))),
    note: "Because you liked " + (names.length > 90 ? names.slice(0, 90) + "..." : names),
  };
}

// Similar artists: Last.fm finds related artists, SoundCloud supplies their tracks
async function scSimilar() {
  if (prefs.lfmOn === false) throw new Error("Last.fm is switched off in Settings, so Similar artists is unavailable.");
  const cur = curTrack();
  let seed = cur && srcOf(cur) === "soundcloud" ? guessNames(cur).artist : "";
  if (!seed) {
    const likes = asList(await api("/me/likes/tracks?limit=30&" + SC_Q));
    if (!likes.length) throw new Error("Play or like something first, and this will suggest similar artists.");
    seed = guessNames(shuffleInPlace(likes.slice())[0]).artist;
  }
  const names = (await lfmSimilarNames(seed, 6)).slice(0, 6);
  if (!names.length) throw new Error("Last.fm doesn't know any artists similar to " + seed + ".");
  const lists = await Promise.all(names.map((n) =>
    api("/tracks?q=" + encodeURIComponent(n) + "&" + SC_Q + "&limit=5").then(asList).catch(() => [])));
  const seen = new Set();
  const out = [];
  lists.flat().forEach((t) => { if (!seen.has(t.id)) { seen.add(t.id); out.push(t); } });
  return { list: recFilter(out.filter((t) => !isMix(t))), note: "Similar to " + seed };
}

// ---------- Discover on Spotify ----------
// Spotify switched off recommendations and related artists for new apps in November 2024, and
// caps searches at 10 results. So Discover on Spotify is built from what's left: Last.fm finds
// artists similar to the ones you like, and Spotify search supplies their tracks.
let spLikedCache = null;
let spLikedAt = 0;

let spLikedFetching = null;
let spLikedCacheRead = null;

function spLikedFetch() {
  spLikedFetching ??= spotifyLikedTracks(500).then((list) => {
    spLikedCache = list;
    spLikedAt = Date.now();
    cacheTracks("likes:spotify", list);
    return list;
  }).finally(() => { spLikedFetching = null; });
  return spLikedFetching;
}

// Your Spotify likes, from this PC's copy when there is one (refreshed in the background when
// it's over ten minutes old); force waits for a fresh copy
function spLikedFromCache() {
  spLikedCacheRead ??= cachedTracks("likes:spotify").then((c) => {
    if (c && !spLikedCache) { spLikedCache = c.tracks; spLikedAt = c.at; c.tracks.forEach((t) => likeState.set(t.id, true)); }
  });
  return spLikedCacheRead;
}

async function spLiked(force) {
  await spLikedFromCache();
  const stale = !spLikedAt || Date.now() - spLikedAt > 10 * 60000;
  if (force || (stale && !spLikedCache)) return spLikedFetch();
  if (stale) spLikedFetch().catch(() => null);
  return spLikedCache;
}

const firstArtist = (t) => artistName(t).split(",")[0].trim();

async function spArtistTracks(name) {
  try { return await spotifySearchTracks('artist:"' + name.replace(/"/g, "") + '"'); } catch (e) { return []; }
}

async function lfmSimilarNames(artist, n) {
  if (prefs.lfmOn === false) return [];
  try {
    const res = await invoke("lastfm_call", { method: "artist.getSimilar", params: [["artist", artist], ["limit", String(n)]] });
    return ((res.similarartists && res.similarartists.artist) || []).map((a) => a.name);
  } catch (e) {
    return [];
  }
}

async function spDiscover(src) {
  if (src === "following") return { list: [] };   // Spotify doesn't let this app see who you follow
  const liked = await spLiked();
  const likedIds = new Set(liked.map((t) => t.id));
  let list = [];
  let note = "";
  let msg = "";
  if (src.startsWith("genre:")) {
    // a few pages of a genre search, from different starting points so it changes each time
    const g = src.slice(6).toLowerCase();
    const offsets = shuffleInPlace([0, 10, 20, 30, 40, 50]).slice(0, 3);
    const pages = await Promise.all(offsets.map((o) =>
      spApi("GET", "/search?type=track&limit=10&offset=" + o + "&q=" + encodeURIComponent('genre:"' + g + '"'))
        .then((d) => ((d.tracks && d.tracks.items) || []).filter((x) => x && x.id).map(spTrack))
        .catch(() => [])));
    list = pages.flat();
  } else {
    // seeds: the current Spotify artist (Similar artists), or a few artists from your liked songs (For you)
    let seeds = [];
    const cur = curTrack();
    if (src === "similar" && cur && srcOf(cur) === "spotify") seeds = [firstArtist(cur)];
    if (!seeds.length) {
      const artists = [...new Set(liked.map(firstArtist).filter(Boolean))];
      seeds = shuffleInPlace(artists).slice(0, src === "similar" ? 1 : 3);
    }
    if (!seeds.length) throw new Error("Like a few songs on Spotify first, and Discover will suggest more.");
    let names = [];
    for (const s of seeds) names = names.concat(await lfmSimilarNames(s, 8));
    names = shuffleInPlace([...new Set(names)]).slice(0, 5);
    if (names.length) {
      note = "Similar to " + seeds.join(", ");
    } else {
      // no Last.fm: other songs by artists you like
      names = seeds;
      msg = "Showing more from artists you like on Spotify. Turn on Last.fm in Settings > Integrations for new artists.";
    }
    const lists = await Promise.all(names.map(spArtistTracks));
    list = lists.flat();
  }
  const seen = new Set();
  list = list.filter((t) => !likedIds.has(t.id) && !isMix(t) && !seen.has(t.id) && seen.add(t.id));
  return { list: recFilter(list), note, msg };
}

function discoverInit() {
  $("refreshBtn").addEventListener("click", () => {
    if (view === "discover") openDiscover(discoverSrc, true);
    else if (view === "rediscover") openRediscover(true);
    else if (view === "sets") openSets();
    else if (view === "artists") { followedCache = null; openArtistsPage(); }
    else if (view === "streamer") { streamColl.clear(); streamShelves = null; openStreamer(); }
  });
  $("recLangSel").value = prefs.recLang || "any";
  $("recLangSel").addEventListener("change", () => {
    prefs.recLang = $("recLangSel").value;
    savePrefs();
    if (view === "discover") openDiscover(discoverSrc);
  });
}
