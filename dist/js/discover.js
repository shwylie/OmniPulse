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

let recHidden = 0;

function recFilter(list) {
  list = list.filter((t) => !isLikedTrack(t) && !hiddenRecs.has(String(t.id)));
  const kept = list.filter(langOk);
  recHidden += list.length - kept.length;
  return kept;
}

// The Discover language picker; tracks it can't place are kept
function langOk(t) {
  const want = prefs.recLang || "any";
  if (want === "any") return true;
  const d = detectLang(t);
  return d === "unknown" || d.endsWith("?") || d === want;
}

// Discover > Similar artists: Last.fm finds related artists, SoundCloud supplies their tracks
async function loadSimilar() {
  const tok = ++loadToken;
  if (prefs.lfmOn === false) {
    viewTracks = [];
    render();
    say("Last.fm is switched off in Settings, so Similar artists is unavailable.");
    return;
  }
  say("Finding similar artists...");
  try {
    const cur = curTrack();
    let seed = cur && srcOf(cur) === "soundcloud" ? guessNames(cur).artist : "";
    if (!seed) {
      const likes = asList(await api("/me/likes/tracks?limit=30&access=playable&linked_partitioning=true"));
      if (!likes.length) throw new Error("Play or like something first, and this will suggest similar artists.");
      seed = guessNames(shuffleInPlace(likes.slice())[0]).artist;
    }
    const res = await invoke("lastfm_call", { method: "artist.getSimilar", params: [["artist", seed], ["limit", "6"]] });
    const names = ((res.similarartists && res.similarartists.artist) || []).map((a) => a.name).slice(0, 6);
    if (!names.length) throw new Error("Last.fm doesn't know any artists similar to " + seed + ".");
    const lists = await Promise.all(names.map((n) =>
      api("/tracks?q=" + encodeURIComponent(n) + "&access=playable&limit=5&linked_partitioning=true").then(asList).catch(() => [])
    ));
    if (tok !== loadToken) return;
    const seen = new Set();
    const out = [];
    lists.flat().forEach((t) => { if (!seen.has(t.id)) { seen.add(t.id); out.push(t); } });
    viewTracks = recFilter(out.filter((t) => !isMix(t)));
    nextHref = "";
    discoverNote = "Similar to " + seed;
    render();
    say(out.length ? "" : "No matching tracks on SoundCloud.");
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    nextHref = "";
    render();
    say(String(e));
  }
}

// ---------- Discover on Spotify ----------
// Spotify switched off recommendations and related artists for new apps in November 2024, and
// caps searches at 10 results. So Discover on Spotify is built from what's left: Last.fm finds
// artists similar to the ones you like, and Spotify search supplies their tracks.
let spLikedCache = null;
let spLikedAt = 0;

async function spLiked() {
  if (!spLikedCache || Date.now() - spLikedAt > 10 * 60000) {
    spLikedCache = await spotifyLikedTracks(500);
    spLikedAt = Date.now();
  }
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

async function loadSpotifyDiscover(src) {
  const tok = ++loadToken;
  discoverNote = "";
  say("Finding music on Spotify...");
  try {
    const liked = await spLiked();
    if (tok !== loadToken) return;
    const likedIds = new Set(liked.map((t) => t.id));
    let list = [];
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
        discoverNote = "Similar to " + seeds.join(", ");
      } else {
        // no Last.fm: other songs by artists you like
        names = seeds;
        say("Showing more from artists you like. Turn on Last.fm in Settings > Integrations for new artists.");
      }
      const lists = await Promise.all(names.map(spArtistTracks));
      list = lists.flat();
    }
    if (tok !== loadToken) return;
    const seen = new Set();
    list = list.filter((t) => !likedIds.has(t.id) && !isMix(t) && !seen.has(t.id) && seen.add(t.id));
    list = shuffleInPlace(recFilter(list));
    setLocalPool(list);
    render();
    if (!viewTracks.length) say("No suggestions came back. Try another chip or a genre.");
    else if (!/Last\.fm/.test($("status").textContent)) say("");
  } catch (e) {
    if (tok !== loadToken) return;
    viewTracks = [];
    nextHref = "";
    render();
    say(e && e.message ? e.message : String(e));
  }
}

function discoverInit() {
  $("refreshBtn").addEventListener("click", () => {
    if (view === "discover") openDiscover(discoverSrc, true);
    else if (view === "rediscover") openRediscover(true);
  });
  $("recLangSel").value = prefs.recLang || "any";
  $("recLangSel").addEventListener("change", () => {
    prefs.recLang = $("recLangSel").value;
    savePrefs();
    if (view === "discover") openDiscover(discoverSrc);
  });
}
