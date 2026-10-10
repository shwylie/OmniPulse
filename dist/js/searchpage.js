// Search results in categories. Under "All": Tracks first (the first few, best match first; Show all
// lists every one), then Artists, Albums, Playlists and Sets & Podcasts as shelves (one row each; Show
// all opens them up). The filter chips show one category on its own.
//
// A search for a genre ("house", "melodic dubstep") also gets that genre's own section first: a mix of
// what's trending in it on every platform and its top artists, and the tracks are found by genre (as
// SoundCloud's genre tag, Spotify's genre: filter and YouTube Music's own search) rather than by title.

const SEARCH_TRACKS_SHOWN = 8;

// Best match first:
//   1. a track with every word searched for (in its title or artist) beats one with fewer
//      ("ginsu ghosts n stuff" puts Ginsu's "Ghosts n Stuff" above the original and its remixes)
//   2. then one whose title (or artist) is just what was searched ("hello": "Hello" before "Hello
//      Hello"; "deadmau5": deadmau5's songs, not only ones called "deadmau5")
//   3. then the more played, across platforms ("hello": Adele's before one with a 25th of the plays)
// and otherwise the platforms take turns, as they came.
// Plays: SoundCloud's and YouTube Music's counts; Spotify gives a 0-100 popularity instead, read as
// plays on a log scale (50 is about 300 thousand, 80 about 40 million). Unknown (your files,
// MusicBrainz) counts as middling.
const searchPlays = (t) => {
  if (t.playback_count) return Number(t.playback_count);
  if (typeof t.spPop === "number") return Math.pow(10, 2 + t.spPop * 0.07);
  // (Deezer's rank runs to about 1,000,000 for the biggest songs)
  if (typeof t.dzRank === "number") return Math.pow(10, 2 + (t.dzRank / 1e6) * 7);
  return 1e5;
};
function searchRanker(q) {
  const want = [...new Set(reqWords(q))];
  if (!want.length) return null;
  const asked = reqWords(q).join(" ");
  const score = (t) => {
    const have = new Set(reqWords((t.title || "") + " " + artistName(t)));
    return want.filter((w) => have.has(w)).length / want.length;
  };
  const exact = (t) => (reqWords(t.title || "").join(" ") === asked || reqWords(artistName(t)).join(" ") === asked ? 1 : 0);
  return (list) => list.map((t, i) => ({ t, i, s: score(t), e: exact(t), p: Math.log10(1 + searchPlays(t)) }))
    .sort((a, b) => b.s - a.s || b.e - a.e || b.p - a.p || a.i - b.i).map((x) => x.t);
}
let searchShelfTok = 0;
let searchMixes = [];       // sets & podcasts among the search results, for their own shelf
let tracksExpanded = false;

// ---------- genres ----------
const GENRE_WORDS = [
  "electronic", "edm", "house", "deep house", "tech house", "progressive house", "future house", "techno", "trance",
  "dubstep", "melodic dubstep", "riddim", "drum and bass", "drum & bass", "dnb", "liquid dnb", "jungle", "garage", "uk garage",
  "future bass", "trap", "hip hop", "hip-hop", "hip-hop & rap", "rap", "grime", "drill", "pop", "indie", "indie pop", "rock",
  "alternative", "metal", "punk", "emo", "jazz", "blues", "soul", "funk", "disco", "r&b", "rnb", "classical", "ambient",
  "lofi", "lo-fi", "chillout", "chillstep", "downtempo", "synthwave", "hardstyle", "phonk", "country", "folk", "reggae",
  "reggaeton", "latin", "afrobeats", "k-pop", "kpop", "j-pop", "gospel", "orchestral", "soundtrack", "breakbeat", "idm",
];
const genreKey = (s) => String(s || "").toLowerCase().replace(/\s+music$/, "").replace(/\s+/g, " ").trim();
const genreOf = (q) => {
  const k = genreKey(q);
  return GENRE_WORDS.includes(k) ? k : "";
};
const titleCase = (s) => s.replace(/\b[a-z]/g, (c) => c.toUpperCase()).replace(/\bDnb\b/, "DnB").replace(/\bEdm\b/, "EDM").replace(/\bR&b\b/i, "R&B");

// The tracks for a genre, from each platform
function genreFeed(id, g) {
  if (id === "soundcloud") return scFeed("/tracks?genres=" + encodeURIComponent(g) + "&access=playable&limit=30&linked_partitioning=true");
  if (id === "spotify") return arrayFeed(() => spotifySearchTracks('genre:"' + g + '"'));
  if (id === "youtube") return arrayFeed(() => ytSearch(g + " music"));
  return arrayFeed(async () => []);
}

// ---------- the categories ----------
async function searchArtists(q, plats, genre) {
  const lists = await Promise.all([
    plats.includes("soundcloud") && !genre
      ? api("/users?q=" + encodeURIComponent(q) + "&limit=10&linked_partitioning=true").then((d) => asItems(d).map((u) => ({ plat: "soundcloud", name: u.username, info: { id: u.id, name: u.username, img: (u.avatar_url || "").replace("-large", "-t500x500"), followers: u.followers_count || 0 } }))).catch(() => [])
      : [],
    plats.includes("spotify")
      ? spApi("GET", "/search?type=artist&limit=10&q=" + encodeURIComponent(genre ? 'genre:"' + genre + '"' : q))
        .then((d) => ((d.artists && d.artists.items) || []).filter(Boolean).map((a) => ({ plat: "spotify", name: a.name, info: { id: a.id, name: a.name, img: (a.images && a.images[0] && a.images[0].url) || "", followers: spFollowers(a) } }))).catch(() => [])
      : [],
    plats.includes("youtube") && !genre
      ? invoke("yt_artists", { query: q }).then((l) => l.map((a) => ({ plat: "youtube", name: a.name, info: { id: a.id, name: a.name, img: a.thumb, followers: a.subscribers } }))).catch(() => [])
      : [],
    plats.includes("catalog") && !genre
      ? dzGet("/search/artist?limit=10&q=" + encodeURIComponent(q)).then((d) => (d.data || []).map((a) => ({ plat: "deezer", name: a.name, info: { id: a.id, name: a.name, img: a.picture_big || a.picture_xl || "", followers: a.nb_fan || 0, url: a.link || "" } }))).catch(() => [])
      : [],
    // a genre's best-known artists, from Last.fm (names only)
    genre && prefs.lfmOn !== false
      ? invoke("lastfm_call", { method: "tag.getTopArtists", params: [["tag", genre], ["limit", "20"]] })
        .then((r) => ((r.topartists && r.topartists.artist) || []).map((a) => ({ plat: "", name: a.name, info: null }))).catch(() => [])
      : [],
  ]);
  // one tile per artist, whichever platforms they turned up on
  const by = new Map();
  interleave(lists).forEach((x, order) => {
    const k = artistKey(x.name);
    if (!k) return;
    if (!by.has(k)) by.set(k, { name: x.name, on: {}, img: "", followers: 0, order });
    const a = by.get(k);
    if (x.plat && !a.on[x.plat]) { a.on[x.plat] = x.info; a.followers += countOf(x.info && x.info.followers); }
    // (a real picture beats a stand-in avatar)
    const img = x.info && x.info.img;
    if (img && (!a.img || (!realImage(a.img) && realImage(img)))) a.img = img;
  });
  // The one you meant first: the name itself, then one with all the words; then a real picture,
  // then how many follow them (on every platform together), then how many platforms they're on
  const want = reqWords(q).join(" ");
  const wantSet = new Set(reqWords(q));
  const rank = (a) => {
    const have = reqWords(a.name);
    const exact = genre ? 0 : have.join(" ") === want ? 2 : [...wantSet].every((w) => have.includes(w)) ? 1 : 0;
    return [exact, realImage(a.img) ? 1 : 0, Math.log10(1 + a.followers) + Object.keys(a.on).length * 0.5];
  };
  const ranked = [...by.values()].map((a) => ({ a, r: rank(a) }))
    .sort((x, y) => y.r[0] - x.r[0] || y.r[1] - x.r[1] || y.r[2] - x.r[2] || x.a.order - y.a.order).map((x) => x.a);
  return ranked.slice(0, 24).map((a) => {
    const plats = Object.keys(a.on);
    const seed = () => { const s = artistFrom({ name: a.name, sc: a.on.soundcloud, sp: a.on.spotify, yt: a.on.youtube }); if (a.on.deezer && !s.on.deezer) s.on.deezer = a.on.deezer; return s; };
    return {
      kind: "artist", name: a.name, art: a.img, hue: hueOf(a.name), plats, sub: "Artist",
      open: () => openArtist(seed()),
      play: async () => {
        say("Gathering " + a.name + "'s music...");
        const list = await artistAllTracks(seed());
        if (!list.length) { say("No tracks by " + a.name + " found."); return; }
        say("");
        startList(list, 0, false);
      },
    };
  });
}

async function searchPlaylists(q, plats) {
  const lists = await Promise.all([
    plats.includes("soundcloud")
      ? api("/playlists?q=" + encodeURIComponent(q) + "&limit=10&linked_partitioning=true&show_tracks=false")
        .then((d) => asItems(d).filter((p) => p && p.id && !isScAlbum(p)).map((p) => ({ p, plat: "soundcloud", art: artUrl(p, "t500x500"), sub: (p.user && p.user.username) || "" }))).catch(() => [])
      : [],
    plats.includes("spotify")
      ? spApi("GET", "/search?type=playlist&limit=10&q=" + encodeURIComponent(q))
        .then((d) => ((d.playlists && d.playlists.items) || []).filter((x) => x && x.id).map((x) => ({
          p: { source: "spotify", id: "sp:" + x.id, spId: x.id, title: x.name, user: { username: (x.owner && x.owner.display_name) || "" } },
          plat: "spotify", art: (x.images && x.images[0] && x.images[0].url) || "", sub: (x.owner && x.owner.display_name) || "",
        }))).catch(() => [])
      : [],
    plats.includes("youtube")
      ? invoke("yt_playlists", { query: q }).then((l) => l.map((x) => ({
          p: { source: "youtube", id: "ytp:" + x.id, ytPlaylistId: x.id, title: x.title, user: { username: x.author } },
          plat: "youtube", art: x.thumb, sub: x.author,
        }))).catch(() => [])
      : [],
  ]);
  return interleave(lists).slice(0, 24).map((x) => ({
    kind: "playlist", name: x.p.title || "Untitled playlist", art: x.art, plats: [x.plat], sub: x.sub,
    open: () => openPlaylist(x.p),
    play: async () => {
      openPlaylist(x.p);   // its page fills the list; play once it's there
      const t0 = Date.now();
      while (!viewTracks.length && Date.now() - t0 < 15000) await new Promise((r) => setTimeout(r, 200));
      if (viewTracks.length) startList(viewTracks, 0, false);
    },
  }));
}

function albumShelfTiles(list) {
  return list.slice(0, 24).map((a) => ({
    kind: "playlist", name: a.title, art: a.art, plats: [a.source], sub: a.artist + (a.kind && a.kind !== "Album" ? " · " + a.kind : ""),
    open: () => openPlaylist(albumAsPlaylist(a)),
    play: () => playAlbum(a),
  }));
}

function genreTiles(g, plats) {
  const pickers = plats.filter((p) => DISCOVER_PICKERS[p]);
  const name = titleCase(g);
  return [{
    kind: "genre", name, hue: hueOf(name), plats: pickers, sub: "Trending now",
    mix: makeMix({ id: "mix:genre:" + g, title: "Trending " + name, desc: "What's trending in " + name + ", from " + pickers.map((p) => PLATFORMS[p].name).join(", ") + ".", build: () => buildGenre(name, pickers) }),
  }];
}

// ---------- drawing ----------
// Shelves are drawn as soon as each category arrives, always in the same order
async function paintSearchShelves(q) {
  const tok = ++searchShelfTok;
  const box = $("searchShelves");
  const only = { artists: "artists", playlists: "playlists" }[listFilter];
  box.hidden = !(listFilter === "all" || only);
  box.replaceChildren();
  if (box.hidden) return;
  const plats = activePlats("search");
  const genre = genreOf(q);
  const shelves = {};
  const order = ["genre", "artists", "albums", "playlists", "sets"].filter((k) => !only || k === only);
  const draw = () => {
    if (tok !== searchShelfTok || view !== "search" || query !== q) return;
    box.replaceChildren(...order.filter((k) => shelves[k] && (shelves[k].tiles || shelves[k].tracks).length).map((k) => {
      const sec = shelfEl(shelves[k]);
      if (only) { sec.classList.add("expanded"); sec.querySelector(".shelfAll").hidden = true; }
      return sec;
    }));
    if (only && !box.children.length) box.append(el("p", "feedNote", "Nothing found."));
  };
  const put = (k, shelf) => { shelves[k] = shelf; draw(); };
  const jobs = [];
  if (genre && !only) put("genre", { title: titleCase(genre), tiles: genreTiles(genre, plats) });
  if (!only || only === "artists") jobs.push(searchArtists(q, plats, genre).then((tiles) => put("artists", { title: genre ? "Top " + titleCase(genre) + " artists" : "Artists", tiles })));
  if (!only) jobs.push(fetchAlbums({ view: "search", query: q }).then((l) => put("albums", { title: "Albums", tiles: albumShelfTiles(l || []) })).catch(() => {}));
  if (!only || only === "playlists") jobs.push(searchPlaylists(genre || q, plats).then((tiles) => put("playlists", { title: "Playlists", tiles })));
  await Promise.allSettled(jobs);
}

// The sets & podcasts the track search turned up (they stay out of the Tracks list)
function searchSetsFound(list) {
  if (view !== "search" || listFilter !== "all") return;
  const fresh = list.filter((t) => isMix(t) && !searchMixes.some((x) => x.id === t.id));
  if (!fresh.length) return;
  searchMixes = searchMixes.concat(fresh);
  const box = $("searchShelves");
  const old = box.querySelector(".shelf.setsShelf");
  const sec = shelfEl({ title: "Sets & Podcasts", tracks: searchMixes.slice(0, 24) });
  sec.classList.add("setsShelf");
  if (old) old.replaceWith(sec); else box.append(sec);
}

// Tracks under "All": the first few, until Show all
function paintTrackHead() {
  const onAll = view === "search" && listFilter === "all" && !!query;
  $("trackHead").hidden = !onAll || !viewTracks.length;
  $("list").classList.toggle("collapsed", onAll && !tracksExpanded);
  $("trackAll").textContent = tracksExpanded ? "Show less" : "Show all";
  $("trackAll").hidden = viewTracks.length <= SEARCH_TRACKS_SHOWN && !nextHref;
}
const tracksCollapsed = () => view === "search" && listFilter === "all" && !tracksExpanded;

function searchPageInit() {
  wireShelves($("searchShelves"));
  $("trackAll").addEventListener("click", () => {
    tracksExpanded = !tracksExpanded;
    paintTrackHead();
    if (tracksExpanded) setTimeout(nearBottomCheck, 50);
    else $("listView").scrollTop = 0;
  });
}
