// Discover, as a feed of shelves: Daily Mixes and artist stations built from your likes, more of
// what you like, new tracks from artists you follow, and genres. Every platform that's switched
// on takes part; each tile shows where it comes from inside its cover. Tiles open as a playlist
// page (built when first opened), or play straight away from their play button.
//
// Spotify doesn't let apps like this one read its own Daily Mixes or "Made for you" playlists
// (since November 2024), and SoundCloud's stations aren't in its API, so these are made here.

const GENRES = ["Electronic", "House", "Techno", "Dubstep", "Drum & Bass", "Hip-hop & Rap", "Pop", "Indie", "R&B", "Ambient", "Lo-fi", "Rock"];
const MIX_BANDS = ["#3ee0d0", "#f5e663", "#ff5a4e", "#f28fd6", "#9ad94b", "#ffae4a"];   // Daily Mix label colours

let feedCache = null;   // { key, shelves } so going back to Discover shows the same feed

const shuffledCopy = (list) => shuffleInPlace(list.slice());
const dedupeTracks = (list) => { const seen = new Set(); return list.filter((t) => t && !seen.has(t.id) && seen.add(t.id)); };
const freshPick = (t) => !isMix(t) && langOk(t) && !hiddenRecs.has(String(t.id));
const hueOf = (s) => [...String(s)].reduce((h, c) => (h * 31 + c.charCodeAt(0)) % 360, 7);

// ---------- your top artists, from your likes and what you play ----------
function feedArtists(plats) {
  const plays = new Map();
  loadStats().forEach((x) => { const k = (x.artist || "").toLowerCase(); plays.set(k, (plays.get(k) || 0) + 1); });
  const by = new Map();
  const add = (plat, t) => {
    const name = plat === "spotify" ? firstArtist(t) : artistName(t);
    if (!name || isMix(t)) return;
    const k = plat + "|" + name.toLowerCase();
    if (!by.has(k)) by.set(k, { plat, name, tracks: [] });
    by.get(k).tracks.push(t);
  };
  if (plats.includes("soundcloud")) likedTracks.filter(isLikedTrack).forEach((t) => add("soundcloud", t));
  if (plats.includes("spotify")) (spLikedCache || []).forEach((t) => add("spotify", t));
  const all = [...by.values()].map((a) => Object.assign(a, { score: a.tracks.length * 2 + (plays.get(a.name.toLowerCase()) || 0) }));
  // each platform's artists by how much you like them, taking turns between platforms
  return interleave(plats.map((p) => all.filter((a) => a.plat === p).sort((x, y) => y.score - x.score)));
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

// A Daily Mix: the artists' tracks you like, woven with new ones in the same vein
async function buildDailyMix(artists) {
  const plat = artists[0].plat;
  const liked = artists.flatMap((a) => a.tracks);
  const familiar = shuffledCopy(liked).slice(0, 18);
  let fresh = plat === "spotify"
    ? (await spSimilarTracks(artists[0].name, 3)).concat((await Promise.all(artists.map((a) => spArtistTracks(a.name)))).flat())
    : await scRelatedTo(liked, 4);
  fresh = shuffledCopy(fresh.filter((t) => freshPick(t) && !isLikedTrack(t)));
  return dedupeTracks(interleave([familiar, fresh])).slice(0, 60);
}

// An artist station: that sound, mostly from other artists
async function buildStation(a) {
  const fresh = a.plat === "spotify"
    ? (await spSimilarTracks(a.name, 5)).concat(await spArtistTracks(a.name))
    : await scRelatedTo(a.tracks, 4);
  return dedupeTracks(shuffledCopy(a.tracks).slice(0, 4).concat(shuffledCopy(fresh.filter(freshPick)))).slice(0, 60);
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

async function buildFeed(plats) {
  if (plats.includes("soundcloud") || plats.includes("spotify")) await loadLikesFor(plats).catch(() => null);
  const artists = feedArtists(plats);
  const shelves = [];

  // Made for you: Daily Mix 01-06, each one of your top artists plus two more from the same platform
  const mains = artists.slice(0, 6);
  if (mains.length) {
    shelves.push({
      title: "Made for you",
      tiles: mains.map((a, i) => {
        const same = artists.filter((x) => x.plat === a.plat && !mains.includes(x));
        const crew = [a].concat(same.slice(i * 2, i * 2 + 2));
        const n = String(i + 1).padStart(2, "0");
        return {
          kind: "daily", band: MIX_BANDS[i % MIX_BANDS.length], number: n, plats: [a.plat],
          art: artUrl(a.tracks[0], "t500x500"),
          sub: crew.map((x) => x.name).join(", ") + " and more",
          mix: makeMix({ id: "mix:daily:" + a.plat + ":" + a.name, title: "Daily Mix " + n,
            desc: crew.map((x) => x.name).join(", ") + " and more: tracks you like, mixed with new ones.",
            build: () => buildDailyMix(crew) }),
        };
      }),
    });
  }

  // Artist stations: the next artists you like, a little shuffled each time
  const stationArtists = shuffledCopy(artists.slice(6, 30)).slice(0, 8);
  if (stationArtists.length) {
    shelves.push({
      title: "Artist stations",
      tiles: stationArtists.map((a) => ({
        kind: "station", plats: [a.plat], art: artUrl(a.tracks[0], "t500x500"), name: a.name, hue: hueOf(a.name),
        sub: "Artist station",
        mix: makeMix({ id: "mix:station:" + a.plat + ":" + a.name, title: a.name + " station",
          desc: "Music like " + a.name + ".", build: () => buildStation(a) }),
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
          build: () => buildGenre(g, genrePlats) }),
      })),
    });
  }
  return { shelves, artists };
}

// Shelves of single tracks, which need a moment to fetch; added below the others when ready
async function trackShelves(plats) {
  const out = [];
  const pickers = plats.filter((p) => DISCOVER_PICKERS[p]);
  const likes = await Promise.allSettled(pickers.map((p) => DISCOVER_PICKERS[p]("foryou")));
  const more = dedupeTracks(interleave(likes.filter((r) => r.status === "fulfilled").map((r) => shuffledCopy(r.value.list)))).slice(0, 30);
  if (more.length) out.push({ title: "More of what you like", tracks: more });
  if (plats.includes("soundcloud")) {
    const f = await scDiscover("following").catch(() => null);
    if (f && f.list.length) out.push({ title: "New from artists you follow", tracks: f.list.slice(0, 30) });
  }
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
  const node = el("div", "tile tile-" + tile.kind);
  node._tile = tile;
  const art = el("div", "tileArt");
  if (tile.kind === "daily") {
    if (tile.art) { const img = el("img"); img.alt = ""; img.loading = "lazy"; img.src = tile.art; art.append(img); }
    const band = el("span", "tileBand");
    band.style.setProperty("--band", tile.band);
    band.append(el("span", "", "Daily Mix"), el("b", "", tile.number));
    art.append(band);
  } else if (tile.kind === "station") {
    art.style.setProperty("--hue", tile.hue);
    if (tile.art) { const img = el("img", "tileRound"); img.alt = ""; img.loading = "lazy"; img.src = tile.art; art.append(img); }
    const lab = el("span", "tileLabel");
    lab.append(el("i", "", "Station"), el("span", "", tile.name));
    art.append(lab);
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

function shelfEl(shelf) {
  const sec = el("section", "shelf");
  const head = el("div", "shelfHead");
  const all = el("button", "shelfAll", "Show all");
  head.append(el("h2", "", shelf.title), all);
  const row = el("div", "shelfRow");
  const tiles = shelf.tiles || shelf.tracks.map((t, i) => ({
    kind: "track", track: t, index: i, list: shelf.tracks, plats: [srcOf(t)], sub: artistName(t),
  }));
  tiles.forEach((t) => row.append(tileEl(t)));
  const prev = el("button", "shelfNav prev");
  const next = el("button", "shelfNav next");
  prev.setAttribute("aria-label", "Scroll left");
  next.setAttribute("aria-label", "Scroll right");
  prev.append(icon("M15.4 7.4L14 6l-6 6 6 6 1.4-1.4L10.8 12z"));
  next.append(icon("M8.6 16.6L10 18l6-6-6-6-1.4 1.4 4.6 4.6z"));
  const paintNav = () => {
    prev.hidden = row.scrollLeft <= 2 || sec.classList.contains("expanded");
    next.hidden = row.scrollLeft + row.clientWidth >= row.scrollWidth - 2 || sec.classList.contains("expanded");
  };
  const page = (d) => row.scrollBy({ left: d * row.clientWidth * 0.9, behavior: prefs.animOn === false ? "auto" : "smooth" });
  prev.addEventListener("click", () => page(-1));
  next.addEventListener("click", () => page(1));
  row.addEventListener("scroll", paintNav, { passive: true });
  row.addEventListener("wheel", (e) => {
    if (sec.classList.contains("expanded") || row.scrollWidth <= row.clientWidth || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    e.preventDefault();
    row.scrollLeft += e.deltaY;
  }, { passive: false });
  all.addEventListener("click", () => {
    const open = sec.classList.toggle("expanded");
    all.textContent = open ? "Show less" : "Show all";
    paintNav();
  });
  sec.append(head, row, prev, next);
  requestAnimationFrame(paintNav);
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
  if (tile.kind === "track") { startList(tile.list, tile.index, false); return; }
  say("Putting your mix together...");
  try {
    const list = await mixTracks(tile.mix);
    if (!list.length) { say("Nothing came back for this one."); return; }
    say("");
    startList(list, 0, false);
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
  const later = await trackShelves(plats);
  if (tok !== loadToken) return;
  // "More of what you like" goes second, the rest at the end
  if (later.length) shelves.splice(Math.min(1, shelves.length), 0, later[0]);
  shelves.push(...later.slice(1));
  paintFeed(shelves);
  say(shelves.length ? "" : "Like some tracks on SoundCloud or Spotify, and Discover will fill up with music for you.");
}

function feedInit() {
  $("feed").addEventListener("click", (e) => {
    const node = e.target.closest(".tile");
    if (!node) return;
    const tile = node._tile;
    if (e.target.closest(".tilePlay") || tile.kind === "track") { playTile(tile); return; }
    openPlaylist(tile.mix);
  });
  $("feed").addEventListener("contextmenu", (e) => {
    const node = e.target.closest(".tile-track");
    if (!node) return;
    e.preventDefault();
    openTrackMenu(node._tile.track, e.clientX, e.clientY);
  });
}
