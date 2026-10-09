// Artists across platforms.
//
// An artist page is one page whichever platform you came from: the artist is found by name on every
// connected platform (exact name matches only, remembered for a week), and shows their music from all
// of them, plus your files. The same song on several platforms shows once; it counts as liked if it's
// liked on any of them. Following follows on every platform the artist is on.
//
// The Artists page is everyone you follow anywhere (YouTube Music follows are kept on this PC), with
// right-click to follow them on the other platforms too, one at a time or all at once.
// Artists you newly follow on one platform are followed on the others as well (Settings > Library).

const ARTIST_PLATS = ["soundcloud", "spotify", "youtube"];
const SP_FOLLOW_NOTE = "Spotify hasn't let OmniPulse see or change who you follow yet: press Give Spotify permission.";
const ARTIST_LINKS_KEY = "sc.artistLinks";
const ARTIST_LINK_TTL = 7 * 86400000;
const sameArtist = (a, b) => !!artistKey(a) && artistKey(a) === artistKey(b);

// ---------- the same song once ----------
// A song's key: its first artist and its title (upload tags like "(Official Audio)" aside). Versions stay
// apart ("(Remix)" is part of the title), so only the same recording on another platform counts.
function songKey(t) {
  const n = guessNames(t);
  const artist = String(n.artist || artistName(t)).split(/,|&|\bx\b|\band\b|\bfeat\b|\bft\b/i)[0];
  const title = matchWords(matchTitle(t)).join(" ");
  return title ? matchWords(artist).join("") + "|" + title : "";
}

// Keeps the first of each song (lists come best-to-play first) and hangs the others off it as .dupes
function mergeDupes(list, keys) {
  keys = keys || new Map();
  const out = [];
  list.forEach((t) => {
    const k = songKey(t);
    const kept = k && keys.get(k);
    if (kept) {
      if (kept.id !== t.id && !(kept.dupes || []).some((d) => d.id === t.id)) (kept.dupes = kept.dupes || []).push(t);
      return;
    }
    const c = Object.assign({}, t, { dupes: undefined });
    if (k) keys.set(k, c);
    out.push(c);
  });
  return out;
}

// The order the copies of a song are kept in: what plays best first
function bestFirst(byPlat) {
  const order = ["local", spotifyPremium ? "spotify" : null, "soundcloud", "youtube", spotifyPremium ? null : "spotify"].filter(Boolean);
  return order.map((p) => byPlat[p] || []);
}

// ---------- finding an artist on each platform ----------
let artistLinks = null;
function linksLoad() {
  if (!artistLinks) { try { artistLinks = JSON.parse(localStorage.getItem(ARTIST_LINKS_KEY) || "{}"); } catch (e) { artistLinks = {}; } }
  return artistLinks;
}
function linksSave() {
  const now = Date.now();
  Object.keys(artistLinks).forEach((k) => { if (now - (artistLinks[k].at || 0) > ARTIST_LINK_TTL) delete artistLinks[k]; });
  try { localStorage.setItem(ARTIST_LINKS_KEY, JSON.stringify(artistLinks)); } catch (e) {}
}

async function findArtistOn(plat, name) {
  if (plat === "soundcloud") {
    const list = asItems(await api("/users?q=" + encodeURIComponent(name) + "&limit=10&linked_partitioning=true"));
    const u = list.filter((x) => sameArtist(x.username, name) || sameArtist(x.full_name, name))
      .sort((a, b) => (b.followers_count || 0) - (a.followers_count || 0))[0];
    return u ? { id: u.id, name: u.username, img: (u.avatar_url || "").replace("-large", "-t500x500"), followers: u.followers_count || 0, url: u.permalink_url || "" } : null;
  }
  if (plat === "spotify") {
    const d = await spApi("GET", "/search?type=artist&limit=10&q=" + encodeURIComponent(name));
    const a = ((d.artists && d.artists.items) || []).filter((x) => sameArtist(x.name, name))
      .sort((p, q) => ((q.followers && q.followers.total) || 0) - ((p.followers && p.followers.total) || 0))[0];
    return a ? { id: a.id, name: a.name, img: (a.images && a.images[0] && a.images[0].url) || "", followers: spFollowers(a), url: (a.external_urls && a.external_urls.spotify) || "" } : null;
  }
  if (plat === "youtube") {
    const a = (await invoke("yt_artists", { query: name })).find((x) => sameArtist(x.name, name));
    return a ? { id: a.id, name: a.name, img: a.thumb, followers: a.subscribers, url: "https://www.youtube.com/channel/" + a.id } : null;
  }
  return null;
}

// An artist: { id, key, name, on: { soundcloud, spotify, youtube } }, where each is the profile there,
// null if they're not on it, or missing if it hasn't been looked up
function artistFrom(seed) {
  if (seed && seed.key && seed.on) return seed;
  const name = (seed && (seed.name || seed.username)) || "Unknown artist";
  const a = { key: artistKey(name), name, on: {} };
  a.id = "a:" + a.key;
  a.username = name;   // what older code shows as the title
  const links = linksLoad()[a.key];
  if (links) ARTIST_PLATS.forEach((p) => { if (links[p] !== undefined) a.on[p] = links[p]; });
  // what we came with beats what was remembered
  if (seed && seed.sc) a.on.soundcloud = seed.sc;
  if (seed && seed.sp) a.on.spotify = Object.assign({}, a.on.spotify || {}, seed.sp);
  if (seed && seed.yt) a.on.youtube = Object.assign({}, a.on.youtube || {}, seed.yt);
  return a;
}

// The artist of a track, on the platform it's from
function artistOfTrack(t) {
  const src = srcOf(t);
  if (src === "soundcloud" && t.user) {
    const u = t.user;
    return artistFrom({ name: u.username, sc: { id: u.id, name: u.username, img: (u.avatar_url || "").replace("-large", "-t500x500"), followers: u.followers_count || 0, url: u.permalink_url || "" } });
  }
  // YouTube: the artist it was uploaded under (an edited artist line doesn't change who uploaded it)
  const ytUploader = src === "youtube" && t.user && t.user.username ? String(t.user.username).split(/\s*,\s*/)[0] : "";
  const name = src === "spotify" ? firstArtist(t) : ytUploader || guessNames(t).artist || artistName(t);
  return artistFrom({
    name,
    sp: src === "spotify" && t.spArtistId ? { id: t.spArtistId, name } : undefined,
    yt: src === "youtube" && t.ytArtistId ? { id: t.ytArtistId, name } : undefined,
  });
}

// Looks the artist up wherever that hasn't been done (or a profile is only half known)
async function resolveArtist(a) {
  const plats = ARTIST_PLATS.filter((p) => activePlats().includes(p));
  await Promise.all(plats.map(async (p) => {
    const have = a.on[p];
    if (have === null || (have && have.img !== undefined && have.followers !== undefined)) return;
    try {
      const found = await findArtistOn(p, a.name);
      // an id we came with wins over a different same-named profile
      a.on[p] = have && have.id && found && String(found.id) !== String(have.id) ? Object.assign({ img: "", followers: null }, have) : found;
    } catch (e) {
      if (have === undefined) delete a.on[p];   // try again next time
    }
  }));
  const links = linksLoad();
  links[a.key] = Object.assign({ at: Date.now() }, a.on);
  linksSave();
  return a;
}

// ---------- an artist's music, from every platform ----------
async function artistTracksOn(plat, info, a) {
  if (plat === "soundcloud") return asList(await api("/users/" + info.id + "/tracks?access=playable&limit=50&linked_partitioning=true"));
  if (plat === "spotify") {
    const list = await spArtistTracks(a.name);
    return list.filter((t) => (t.user.username || "").split(", ").some((n) => sameArtist(n, a.name)));
  }
  if (plat === "youtube") {
    const page = await invoke("yt_artist", { id: info.id });
    const more = page.all_songs ? await invoke("yt_playlist", { id: page.all_songs }).catch(() => []) : [];
    return page.songs.concat(more).map(ytTrack).filter((t) => (t.user.username || "").split(", ").some((n) => sameArtist(n, a.name)));
  }
  return [];
}

function artistFiles(a) {
  return localTracks.filter((t) => sameArtist(artistName(t), a.name) || sameArtist(guessNames(t).artist, a.name));
}

async function artistAlbums(a) {
  if (!a || !a.on) return [];
  const lists = await Promise.all([
    a.on.soundcloud ? scUserAlbums(a.on.soundcloud.id).catch(() => []) : [],
    a.on.spotify && activePlats().includes("spotify")
      ? spApi("GET", "/artists/" + a.on.spotify.id + "/albums?include_groups=album,single&limit=20")
        .then((d) => (d.items || []).filter((x) => x && x.id).map(spAlbumTile)).catch(() => [])
      : [],
    a.on.youtube && activePlats().includes("youtube")
      ? invoke("yt_artist", { id: a.on.youtube.id }).then((p) => p.albums.map(ytAlbumTile)).catch(() => [])
      : [],
    activePlats().includes("local") ? localAlbumTiles("likes", "").filter((t) => sameArtist(t.artist, a.name)) : [],
  ]);
  // the same album once, from whichever platform comes first
  const seen = new Set();
  return interleave(lists).filter((x) => { const k = matchWords(x.title).join(" "); return k && !seen.has(k) && seen.add(k); });
}

// ---------- the artist page ----------
let artistFollow = {};   // platform -> true / false, or missing while unknown
let artistFollowDone = false;   // the platforms have been asked (what they couldn't say stays unknown)

async function openArtist(seed) {
  const a = artistFrom(seed);
  currentArtist = a;
  currentPlaylist = null;
  artistFollow = {};
  artistFollowDone = false;
  showView("artist");
  $("listView").scrollTop = 0;
  paintFollow();
  const onlyAlbums = albumsOnly();
  const tok = loadToken;
  if (!onlyAlbums) { viewTracks = []; nextHref = ""; render(); say("Looking for " + a.name + " everywhere..."); }
  await resolveArtist(a);
  if (currentArtist !== a) return;
  renderHero();
  albumsKey = "";   // the albums can come from every platform now
  if (curPage) albumsOnPage(curPage);
  artistFollowCheck(a);
  if (onlyAlbums || tok !== loadToken) return;
  const merged = await artistAllTracks(a, true);
  if (tok !== loadToken || currentArtist !== a) return;
  setLocalPool(groupVersions(applyListFilter(merged)));
  render();
  say(viewTracks.length ? "" : "No tracks by " + a.name + " found.");
  setTimeout(nearBottomCheck, 50);
}

// ---------- versions of the same song, together (artist pages) ----------
// A song's versions (its radio edit, extended mix, remixes, a live one...) under one row: the original
// shows, and the rest open beneath it (js/app.js: versionsPanel), with More versions to search for it.
// "Song (Radio Edit)", "Song - Extended Mix" and "Song [VIP]" are all "song".
// The title, without an "Artist - " in front (only when that part is the artist: "Song - Extended Mix" stays)
function versionTitle(t) {
  const title = String(t.title || "");
  const m = title.match(/^(.+?)\s+[-–—]\s+(.+)$/);
  if (!m) return title;
  // (the track's own artist and uploader, not one guessed from this same title)
  const who = new Set(matchWords(artistName(t) + " " + ((t.user && t.user.username) || "") + " " + ((currentArtist && currentArtist.name) || "")));
  return matchWords(m[1]).every((w) => who.has(w)) ? m[2] : title;
}
// What a version adds (in brackets, or after " - "): only these words make a part of a title its version
// ("Pt. II (Dusk)" is another song)
const VERSION_WORDS_RX = /\b(remix|rmx|mix|edit|version|remaster(ed)?|live|vip|flip|bootleg|mash ?up|cover|acoustic|instrumental|extended|radio|rework|refix|dub|demo|sped ?up|slowed|nightcore|club|original|re-?edit|session)\b/i;
// The title without its version parts: the song's own name
function versionStrip(title) {
  return String(title || "")
    .replace(/\s*[(\[]([^)\]]*)[)\]]/g, (m, inner) => (VERSION_WORDS_RX.test(inner) || /^\s*(feat|ft|featuring|with)\b/i.test(inner) ? " " : m))
    .replace(/\s+[-–—|]\s+(.*)$/, (m, rest) => (VERSION_WORDS_RX.test(rest) ? " " : m))
    .replace(/\s+(feat|ft|featuring)\b.*$/i, " ");
}
function versionBase(t) {
  return versionStrip(versionTitle(t)).toLowerCase().replace(/[^a-z0-9À-ɏ]+/g, " ").trim();
}
// Which goes on top: the song itself (or its original mix), then edits of it, then other versions
function versionRank(t) {
  const title = versionTitle(t).toLowerCase();
  // what's added to the song's name (a featured artist isn't a version)
  const bare = title.replace(/[(\[]\s*(feat|ft|featuring|with)\b[^)\]]*[)\]]/g, " ").replace(/\s+(feat|ft|featuring)\b.*$/, " ");
  const extra = bare.replace(/[^a-z0-9À-ɏ]+/g, " ").trim().replace(versionBase(t), "").trim();
  if (!extra || /\boriginal (mix|version)\b/.test(title) && !/\b(remix|rmx|bootleg|vip|flip|live)\b/.test(title)) return 0;
  if (/\b(radio|single|album|extended|clean|explicit|remaster(ed)?|edit)\b/.test(extra) && !/\b(remix|rmx|bootleg|vip|flip|mash ?up|live|acoustic|cover|rework|refix|dub)\b/.test(extra)) return 1;
  return 2;
}
function groupVersions(list) {
  const groups = new Map();
  const order = [];
  list.forEach((t, k) => {
    const base = versionBase(t);
    const key = base.length >= 2 ? base : "\u0000" + k;   // (nothing to go by: on its own)
    if (!groups.has(key)) { groups.set(key, []); order.push(key); }
    groups.get(key).push(t);
  });
  return order.map((key) => {
    const g = groups.get(key);
    if (g.length < 2) return g[0];
    const sorted = g.map((t, k) => [t, versionRank(t), k]).sort((a, b) => a[1] - b[1] || a[2] - b[2]).map((x) => x[0]);
    return Object.assign({}, sorted[0], { _versions: sorted.slice(1) });
  });
}

// Everything by an artist, from every platform, each song once
async function artistAllTracks(a, resolved) {
  if (!resolved) await resolveArtist(a);
  const byPlat = {};
  await Promise.all(ARTIST_PLATS.filter((p) => a.on[p] && activePlats().includes(p)).map(async (p) => {
    byPlat[p] = await artistTracksOn(p, a.on[p], a).catch(() => []);
  }));
  if (activePlats().includes("local")) byPlat.local = artistFiles(a);
  return mergeDupes(interleave(bestFirst(byPlat)));
}

function openArtistFor(t) {
  if (t) openArtist(artistOfTrack(t));
}

// "SoundCloud 12,345 followers · Spotify 1.2m · YouTube Music 1.37m subscribers", each with its logo
function artistSubline(a) {
  const parts = [];
  ARTIST_PLATS.forEach((p) => {
    const info = a.on[p];
    if (!info) return;
    const n = typeof info.followers === "number" && (info.followers > 0 || p !== "spotify") ? info.followers.toLocaleString() + " followers" : info.followers ? info.followers + " subscribers" : "";
    const s = el("span", "artistOn");
    s.title = PLATFORMS[p].name + (artistFollow[p] ? ": following" : "");
    s.append(platIcon(p), el("span", "", n || PLATFORMS[p].name));
    if (artistFollow[p]) s.classList.add("following");
    parts.push(s);
  });
  const files = artistFiles(a).length;
  if (files) { const s = el("span", "artistOn"); s.append(platIcon("local"), el("span", "", files + " files")); parts.push(s); }
  return parts;
}

const artistImage = (a) => (a && a.on && ((a.on.soundcloud && a.on.soundcloud.img) || (a.on.spotify && a.on.spotify.img) || (a.on.youtube && a.on.youtube.img))) || "";

// ---------- following, on every platform the artist is on ----------
const YT_FOLLOWS_KEY = "sc.ytFollows";
let ytFollows = null;
function ytFollowsLoad() {
  if (!ytFollows) { try { ytFollows = JSON.parse(localStorage.getItem(YT_FOLLOWS_KEY) || "[]"); } catch (e) { ytFollows = []; } }
  return ytFollows;
}

async function followStateOn(plat, info) {
  if (plat === "soundcloud") return api("/me/followings/" + info.id).then(() => true, (e) => (/404/.test(String(e)) ? false : null));
  if (plat === "spotify") {
    const r = await spApi("GET", "/me/library/contains?uris=" + encodeURIComponent("spotify:artist:" + info.id)).catch(() => null);
    return Array.isArray(r) ? !!r[0] : null;
  }
  if (plat === "youtube") return ytFollowsLoad().some((x) => x.id === info.id);
  return null;
}

async function setFollowOn(plat, info, on) {
  if (plat === "soundcloud") return invoke("set_follow", { id: String(info.id), follow: on });
  if (plat === "spotify") {
    try {
      await spApi(on ? "PUT" : "DELETE", "/me/library?uris=" + encodeURIComponent("spotify:artist:" + info.id));
    } catch (e) {
      if (spScopeMissing(e) || /\b40[13]\b/.test(String(e))) throw new Error(SP_FOLLOW_NOTE);
      throw e;
    }
    return;
  }
  if (plat === "youtube") {
    ytFollows = ytFollowsLoad().filter((x) => x.id !== info.id);
    if (on) ytFollows.unshift({ id: info.id, name: info.name, img: info.img || "" });
    try { localStorage.setItem(YT_FOLLOWS_KEY, JSON.stringify(ytFollows)); } catch (e) {}
  }
}

async function artistFollowCheck(a) {
  await Promise.all(ARTIST_PLATS.filter((p) => a.on[p] && activePlats().includes(p)).map(async (p) => {
    const s = await followStateOn(p, a.on[p]).catch(() => null);
    if (s !== null) artistFollow[p] = s;
  }));
  if (currentArtist === a) { artistFollowDone = true; paintFollow(); renderHero(); }
}

const followPlats = (a) => ARTIST_PLATS.filter((p) => a && a.on[p] && activePlats().includes(p));

// Follow (everywhere) or Unfollow (wherever you follow them); right-click for one platform at a time.
// Followed on some of their platforms but not all: a second button follows them on the rest.
const platNames = (list) => andList(list.map((p) => PLATFORMS[p].name));

function paintFollow() {
  const b = $("followBtn");
  const more = $("followMoreBtn");
  const a = currentArtist;
  const plats = followPlats(a);
  const yes = plats.filter((p) => artistFollow[p] === true);
  const rest = plats.filter((p) => artistFollow[p] !== true);
  // only while the platforms are being asked; one that can't say doesn't hold it up
  b.disabled = !plats.length || !artistFollowDone;
  b.classList.toggle("on", yes.length > 0);
  b.textContent = yes.length ? "Unfollow" : "Follow";
  b.title = !plats.length ? "" : (yes.length
    ? "Following on " + platNames(yes) + ". Click to unfollow there"
    : "Follow on " + platNames(plats)) + "; right-click for one platform.";
  // followed on some, not all: the rest, in one go
  more.hidden = b.hidden || !artistFollowDone || !yes.length || !rest.length;
  if (!more.hidden) {
    more.textContent = rest.length === 1 ? "Also follow on " + PLATFORMS[rest[0]].name : "Also follow on " + rest.length + " more";
    more.title = "Follow " + a.name + " on " + platNames(rest) + " too.";
  }
}

// Follows or unfollows on the platforms given, and says how that went
async function followSet(a, targets, on) {
  if (!targets.length) return;
  if (!on && !confirm("Unfollow " + a.name + " on " + platNames(targets) + "?")) return;
  const failed = [];
  await Promise.all(targets.map(async (p) => {
    try { await setFollowOn(p, a.on[p], on); artistFollow[p] = on; } catch (e) { failed.push(PLATFORMS[p].name + ": " + (e.message || e)); }
  }));
  if (currentArtist === a) { paintFollow(); renderHero(); }
  followedCache = null;
  sidebarArtists();
  if (failed.some((f) => f.includes(SP_FOLLOW_NOTE)) && confirm("Spotify hasn't let OmniPulse change who you follow yet. Ask Spotify for permission now? (Your browser opens; playback stays signed in.)")) spotifyGrant();
  say(failed.length ? "Couldn't " + (on ? "follow" : "unfollow") + " everywhere. " + failed.join(" ") : (on ? "Following " : "Unfollowed ") + a.name + " on " + platNames(targets) + ".");
}

// The button: unfollows wherever you follow them; if nowhere, follows everywhere
function toggleFollowEverywhere(a) {
  const plats = followPlats(a);
  const yes = plats.filter((p) => artistFollow[p] === true);
  return yes.length ? followSet(a, yes, false) : followSet(a, plats, true);
}

// Right-click: each platform on its own
function followMenu(a, x, y) {
  const items = [a.name];
  followPlats(a).forEach((p) => {
    const on = artistFollow[p] === true;
    items.push([(on ? "Unfollow on " : "Follow on ") + PLATFORMS[p].name, () => followSet(a, [p], !on)]);
  });
  showMenuAt(x, y, items);
}

// ---------- the Artists page ----------
let followedCache = null;   // [{ key, name, img, on: {plat: info}, followed: {plat: true} }]
let artistsSideTok = 0;

async function followedOn(plat) {
  if (plat === "soundcloud") {
    const users = await collectPages("/me/followings?limit=200&linked_partitioning=true", 10);
    return users.map((u) => ({ id: u.id, name: u.username, img: (u.avatar_url || "").replace("-large", "-t200x200"), followers: u.followers_count || 0, url: u.permalink_url || "" }));
  }
  if (plat === "spotify") {
    const out = [];
    let after = "";
    for (let i = 0; i < 20; i++) {
      const d = await spApi("GET", "/me/following?type=artist&limit=50" + (after ? "&after=" + after : ""));
      const page = d.artists || {};
      (page.items || []).forEach((a) => out.push({ id: a.id, name: a.name, img: (a.images && a.images[a.images.length - 1] && a.images[a.images.length - 1].url) || "", followers: spFollowers(a), url: (a.external_urls && a.external_urls.spotify) || "" }));
      after = page.cursors && page.cursors.after;
      if (!after || !page.next) break;
    }
    return out;
  }
  if (plat === "youtube") return ytFollowsLoad().slice();
  return [];
}

async function loadFollowed() {
  const plats = ARTIST_PLATS.filter((p) => activePlats().includes(p));
  const by = new Map();
  const notes = [];
  await Promise.all(plats.map(async (p) => {
    let list = [];
    try { list = await followedOn(p); } catch (e) {
      notes.push(p === "spotify" && spScopeMissing(e) ? SP_FOLLOW_NOTE : PLATFORMS[p].name + ": " + (e.message || e));
      return;
    }
    list.forEach((info) => {
      const k = artistKey(info.name);
      if (!k) return;
      if (!by.has(k)) by.set(k, { key: k, name: info.name, img: "", on: {}, followed: {} });
      const a = by.get(k);
      a.on[p] = info;
      a.followed[p] = true;
      if (!a.img) a.img = info.img;
    });
  }));
  const list = [...by.values()].sort((x, y) => x.name.localeCompare(y.name, undefined, { sensitivity: "base" }));
  return { list, notes, plats };
}

let followedNotes = [];

// Who you follow (loaded once, and again when the platforms change); new follows are followed
// everywhere. Paints the Artists page if it's open.
async function sidebarArtists() {
  const tok = ++artistsSideTok;
  const plats = ARTIST_PLATS.filter((p) => activePlats().includes(p));
  $("navArtists").hidden = !plats.length;
  if (!plats.length) return;
  if (!followedCache) {
    if (view === "artists") paintArtistsPage();
    const got = await loadFollowed();
    if (tok !== artistsSideTok) return;
    followedCache = got.list;
    followedNotes = got.notes;
    followNewEverywhere(got);
  }
  if (view === "artists") paintArtistsPage();
}

const seedOf = (a) => artistFrom({ name: a.name, sc: a.on.soundcloud, sp: a.on.spotify, yt: a.on.youtube });

function paintArtistsPage() {
  const box = $("artistsPage");
  if (!followedCache) { box.replaceChildren(el("p", "feedNote", "Loading the artists you follow...")); return; }
  const list = followedCache.filter((a) => listFilter === "all" || a.followed[listFilter]);
  const tiles = list.map((a) => ({
    kind: "artist", name: a.name, art: a.img, hue: hueOf(a.name), plats: Object.keys(a.followed), _artist: a,
    sub: Object.keys(a.followed).map((p) => PLATFORMS[p].name).join(" \u00b7 "),
    open: () => openArtist(seedOf(a)),
    play: async () => {
      say("Gathering " + a.name + "'s music...");
      const tracks = await artistAllTracks(seedOf(a));
      if (!tracks.length) { say("No tracks by " + a.name + " found."); return; }
      say("");
      startList(tracks, 0, false, a.name);
    },
  }));
  const shelf = shelfEl({ title: list.length + (list.length === 1 ? " artist" : " artists"), tiles });
  shelf.classList.add("expanded");
  shelf.querySelector(".shelfAll").hidden = true;
  box.replaceChildren(shelf);
  if (followedNotes.length) {
    const note = el("p", "feedNote", followedNotes.join(" "));
    if (followedNotes.includes(SP_FOLLOW_NOTE)) {
      const b = el("button", "soft grantBtn", "Give Spotify permission");
      b.addEventListener("click", async () => { await spotifyGrant(); followedCache = null; sidebarArtists(); });
      note.append(" ", b);
    }
    box.append(note);
  }
  if (!list.length) box.append(el("p", "feedNote", listFilter === "all" ? "Artists you follow show here. Follow them from their pages." : "You don't follow anyone on " + PLATFORMS[listFilter].name + " yet."));
}

function openArtistsPage() {
  currentPlaylist = null;
  currentArtist = null;
  showView("artists");
  ++loadToken;
  viewTracks = [];
  nextHref = "";
  render();
  say("");
  $("listView").scrollTop = 0;
  if (!followedCache) sidebarArtists(); else paintArtistsPage();
}

// ---------- following the same artists on other platforms ----------
// Each artist is found on the other platform by exact name; ones that aren't there are left alone.
async function followOnToo(list, plat, progress) {
  let done = 0, followed = 0, missing = 0, failed = "";
  const queueList = list.slice();
  const worker = async () => {
    while (queueList.length && !failed) {
      const a = queueList.shift();
      try {
        const info = a.on[plat] || await findArtistOn(plat, a.name);
        if (!info) { missing++; } else {
          await setFollowOn(plat, info, true);
          a.on[plat] = info;
          a.followed[plat] = true;
          followed++;
        }
      } catch (e) {
        failed = e.message || String(e);
      }
      done++;
      if (progress) progress(done, list.length);
    }
  };
  await Promise.all([worker(), worker(), worker()]);
  return { followed, missing, failed };
}

async function batchFollow(from, to) {
  const list = (followedCache || []).filter((a) => a.followed[from] && !a.followed[to]);
  if (!list.length) { say("Everyone you follow on " + PLATFORMS[from].name + " is followed on " + PLATFORMS[to].name + " already."); return; }
  if (!confirm("Follow on " + PLATFORMS[to].name + " the " + list.length + " artists you follow on " + PLATFORMS[from].name + "? Each is matched by name; any that aren't on " + PLATFORMS[to].name + " are skipped.")) return;
  const r = await followOnToo(list, to, (d, n) => say("Following on " + PLATFORMS[to].name + ": " + d + " of " + n + "..."));
  sidebarArtists();
  say((r.failed ? "Stopped: " + r.failed + " " : "") + "Followed " + r.followed + " on " + PLATFORMS[to].name + "." + (r.missing ? " " + r.missing + (r.missing === 1 ? " isn't" : " aren't") + " on " + PLATFORMS[to].name + "." : ""));
}

// New follows: an artist newly followed on a platform (since the last look at that platform) gets
// followed on the others. Each platform is remembered on its own: one seen for the first time (just
// signed in, or ready a moment later than the others) only has its follows noted, so a whole list
// never counts as new. More than a handful at once is left to you (right-click Artists).
const FOLLOW_SEEN_KEY = "sc.followSeen";
const FOLLOW_AUTO_MAX = 10;
async function followNewEverywhere(got) {
  if (prefs.followEverywhere === false || got.notes.length) return;
  let seen = null;
  try { seen = JSON.parse(localStorage.getItem(FOLLOW_SEEN_KEY) || "null"); } catch (e) {}
  if (!seen || Array.isArray(seen) || typeof seen !== "object") seen = {};
  const snapshot = () => {
    const now = Object.assign({}, seen);   // platforms that aren't on right now keep what was noted
    got.plats.forEach((p) => { now[p] = got.list.filter((a) => a.followed[p]).map((a) => a.key); });
    try { localStorage.setItem(FOLLOW_SEEN_KEY, JSON.stringify(now)); } catch (e) {}
  };
  const fresh = got.list.filter((a) => got.plats.some((p) => Array.isArray(seen[p]) && a.followed[p] && !seen[p].includes(a.key)));
  if (!fresh.length) { snapshot(); return; }
  if (fresh.length > FOLLOW_AUTO_MAX) {
    snapshot();
    say(fresh.length + " new follows found. Right-click Artists in the sidebar to follow them on your other platforms too.");
    return;
  }
  let followed = 0;
  for (const p of got.plats) {
    const todo = fresh.filter((a) => !a.followed[p]);
    if (todo.length) followed += (await followOnToo(todo, p)).followed;
  }
  snapshot();
  if (followed) {
    say("Followed " + fresh.map((a) => a.name).slice(0, 3).join(", ") + (fresh.length > 3 ? " and others" : "") + " on your other platforms too.");
    sidebarArtists();
  }
}

// ---------- menus ----------
let menuFns = [];
function showMenuAt(x, y, items) {
  const m = $("plMenu");
  m.replaceChildren();
  menuFns = [];
  items.forEach((it) => {
    if (it === "sep") { m.append(el("div", "menuSep")); return; }
    if (typeof it === "string") { m.append(el("div", "menuHead", it)); return; }
    const b = el("button", "menuItem", it[0]);
    if (it[2]) b.classList.add(it[2]);
    b.dataset.fn = String(menuFns.length);
    menuFns.push(it[1]);
    m.append(b);
  });
  m.hidden = false;
  const w = m.offsetWidth;
  const h = m.offsetHeight;
  m.style.left = Math.max(8, Math.min(window.innerWidth - w - 8, x)) + "px";
  m.style.top = Math.max(8, y + h + 8 > window.innerHeight ? y - h : y) + "px";
}

function artistMenu(a, x, y) {
  const items = [a.name, ["Open", () => openArtist(seedOf(a))]];
  const missing = ARTIST_PLATS.filter((p) => activePlats().includes(p) && !a.followed[p]);
  if (missing.length) items.push("sep");
  missing.forEach((p) => items.push(["Follow on " + PLATFORMS[p].name + " too", async () => {
    const r = await followOnToo([a], p);
    say(r.failed ? "Couldn't follow on " + PLATFORMS[p].name + ": " + r.failed : r.followed ? "Following " + a.name + " on " + PLATFORMS[p].name + "." : a.name + " isn't on " + PLATFORMS[p].name + ".");
    sidebarArtists();
  }]));
  items.push("sep", ["Unfollow everywhere", async () => {
    if (!confirm("Unfollow " + a.name + " on " + Object.keys(a.followed).map((p) => PLATFORMS[p].name).join(", ") + "?")) return;
    await Promise.all(Object.keys(a.followed).map((p) => setFollowOn(p, a.on[p], false).catch(() => {})));
    followedCache = null;
    sidebarArtists();
  }]);
  showMenuAt(x, y, items);
}

function artistsHeaderMenu(x, y) {
  const plats = ARTIST_PLATS.filter((p) => activePlats().includes(p));
  const items = ["Follow everyone you follow on one platform on another too"];
  plats.forEach((from) => plats.filter((to) => to !== from).forEach((to) => {
    const n = (followedCache || []).filter((a) => a.followed[from] && !a.followed[to]).length;
    if (n) items.push([PLATFORMS[from].name + " → " + PLATFORMS[to].name + " (" + n + (n === 1 ? " artist)" : " artists)"), () => batchFollow(from, to)]);
  }));
  if (items.length === 1) items.push(["Nothing to do: everyone's followed everywhere", () => {}]);
  items.push("sep", ["Refresh", () => { followedCache = null; sidebarArtists(); }]);
  showMenuAt(x, y, items);
}

function artistsInit() {
  $("followBtn").addEventListener("click", () => { if (currentArtist && currentArtist.on) toggleFollowEverywhere(currentArtist); });
  $("followBtn").addEventListener("contextmenu", (e) => {
    e.preventDefault();
    e.stopPropagation();
    if (currentArtist && currentArtist.on && !$("followBtn").disabled) followMenu(currentArtist, e.clientX, e.clientY);
  });
  $("followMoreBtn").addEventListener("click", () => {
    const a = currentArtist;
    if (a && a.on) followSet(a, followPlats(a).filter((p) => artistFollow[p] !== true), true);
  });
  $("navArtists").addEventListener("click", openArtistsPage);
  wireShelves($("artistsPage"));
  $("artistsPage").addEventListener("contextmenu", (e) => {
    const node = e.target.closest(".tile-artist");
    if (!node || !node._tile._artist) return;
    e.preventDefault();
    e.stopPropagation();
    artistMenu(node._tile._artist, e.clientX, e.clientY);
  });
  $("artistsBatch").addEventListener("click", (e) => { e.stopPropagation(); const r = $("artistsBatch").getBoundingClientRect(); artistsHeaderMenu(r.left, r.bottom + 4); });
  $("plMenu").addEventListener("click", (e) => {
    const b = e.target.closest(".menuItem[data-fn]");
    if (!b) return;
    e.stopPropagation();
    const fn = menuFns[Number(b.dataset.fn)];
    closeMenu();
    if (fn) fn();
  }, true);
  $("followEverywhereOn").checked = prefs.followEverywhere !== false;
  $("followEverywhereOn").addEventListener("change", () => { prefs.followEverywhere = $("followEverywhereOn").checked; savePrefs(); });
}

// Spotify stopped sending follower counts (February 2026), so an artist without one shows just the
// platform's name rather than "0 followers"
function spFollowers(a) {
  return a && a.followers && typeof a.followers.total === "number" && a.followers.total > 0 ? a.followers.total : null;
}
