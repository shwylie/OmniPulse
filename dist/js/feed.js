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
const notLiked = (keys) => (t) => !isLikedTrack(t) && !keys.has(songKey(t));

// ---------- your top artists, from your likes on every platform and what you play ----------
// One entry per artist, whichever platforms you like them on, so a mix can draw on all of them.
const artistKey = (s) => String(s || "").toLowerCase().replace(/[^a-z0-9À-ɏ]+/g, "");

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

// Which platforms a mix of these artists will draw on
function crewPlats(crew, plats) {
  const out = ["soundcloud"];
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
  const [a, b, c] = await Promise.all([sc, sp, yt]);
  const fresh = notLiked(likedSongs());
  return withoutRecent(interleave([shuffledCopy(a), shuffledCopy(b), shuffledCopy(c)]).filter((t) => freshPick(t) && fresh(t)));
}

// Your Mix: mostly new music in the vein of these artists, with some you like by them (on any
// platform, and your files) woven in: two new ones for each one you know
async function buildYourMix(crew, plats) {
  const familiar = shuffledCopy(crew.flatMap((a) => a.liked.soundcloud.concat(a.liked.spotify, a.liked.youtube, a.files))).slice(0, 12);
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
  const own = shuffledCopy(a.liked.soundcloud.concat(a.liked.spotify, a.liked.youtube, a.files)).slice(0, 4);
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
  const artists = feedArtists(plats);
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
      art: artUrl(a.liked.soundcloud[0], "t500x500"), sub: who,
      mix: makeMix({ id: "mix:yours:" + a.key, title: "Your Mix " + n,
        desc: who + ": tracks you like, mixed with new ones from " + crewPlats(crew, plats).map((p) => PLATFORMS[p].name).join(", ") + ".",
        build: () => buildYourMix(crew, plats), fresh: () => onlyNew(() => freshFor(crew, plats)) }),
    };
  });
  const madeForYou = fromSpotify.daily.map(spotifyTile).concat(yours);
  if (madeForYou.length) shelves.push({ title: "Made for you", tiles: madeForYou });

  // Spotify's other mixes: Discover Weekly, Release Radar, your top mixes...
  if (fromSpotify.other.length) shelves.push({ title: "More mixes from Spotify", tiles: fromSpotify.other.map(spotifyTile) });

  // Artist stations: the next artists you like, a little shuffled each time
  const stationArtists = shuffledCopy(artists.slice(6, 30)).slice(0, 8);
  if (stationArtists.length) {
    shelves.push({
      title: "Artist stations",
      tiles: stationArtists.map((a) => ({
        kind: "station", plats: crewPlats([a], plats), art: artUrl(a.liked.soundcloud[0], "t500x500"), name: a.name, hue: hueOf(a.name),
        sub: "Artist station",
        mix: makeMix({ id: "mix:station:" + a.key, title: a.name + " station",
          desc: "Music like " + a.name + ".", build: () => buildStation(a, plats), fresh: () => onlyNew(() => freshFor([a], plats)) }),
      })),
    });
  }

  // Trending by genre
  const genrePlats = plats.filter((p) => DISCOVER_PICKERS[p]);
  if (genrePlats.length) {
    shelves.push({
      title: "Trending by genre",
      tiles: GENRES.map((g) => ({
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
  if (plats.includes("soundcloud")) {
    const f = await scDiscover("following").catch(() => null);
    const newest = f ? f.list.filter(keep).sort((a, b) => String(b.created_at || "").localeCompare(String(a.created_at || ""))) : [];
    if (newest.length) out.push({ title: "New from artists you follow", tracks: newest.slice(0, 30) });
  }
  const likes = await Promise.allSettled(pickers.map((p) => DISCOVER_PICKERS[p]("foryou")));
  const more = dedupeTracks(interleave(likes.filter((r) => r.status === "fulfilled").map((r) => shuffledCopy(r.value.list)))).filter(keep).slice(0, 30);
  if (more.length) out.push({ title: "New for you", tracks: more });
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
  $("feed").replaceChildren(...shelves.map(shelfEl));
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
  // new music first: new from artists you follow, and new for you, above the mixes
  shelves.unshift(...later);
  if (view !== "discover") return;
  paintFeed(shelves);
  if (tok === loadToken) say(shelves.length ? "" : "Like some tracks on SoundCloud or Spotify, and Discover will fill up with music for you.");
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
  wireShelves($("feed"));
}
