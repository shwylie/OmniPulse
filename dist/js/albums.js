// Album tiles mixed into lists.

// ---------- albums mixed into lists ----------
// Album tiles sit among the tracks (one every 6 rows) on Likes, Search, Discover, Rediscover and artist pages.
// The Albums button beside the title hides them; tracks from albums always stay in the list.
const ALBUM_PAGES = ["likes", "search", "discover", "rediscover", "artist"];
const PLAY_ICON = "M8 5v14l11-7z";
let viewAlbums = [];
let albumsKey = "";

const albumsShown = () => prefs.showAlbums !== false;
const asItems = (d) => (Array.isArray(d) ? d : (d && d.collection) || []);
const isScAlbum = (p) => !!p && (/album|ep|compilation/i.test(p.playlist_type || p.set_type || "") || p.is_album === true);

function scAlbumTile(p) {
  return {
    source: "soundcloud", id: "sca:" + p.id, raw: p, title: p.title || "Untitled album",
    artist: (p.user && p.user.username) || "", count: p.track_count || 0, art: artUrl(p, "t200x200"),
  };
}

function spAlbumTile(a) {
  const imgs = a.images || [];
  return {
    source: "spotify", id: "spa:" + a.id, spAlbumId: a.id, title: a.name || "Untitled album",
    artist: (a.artists || []).map((x) => x.name).join(", "), count: a.total_tracks || 0,
    art: imgs.length ? (imgs[1] || imgs[0]).url : "",
  };
}

function localAlbumTiles(page, q) {
  const map = new Map();
  localTracks.forEach((t) => {
    const a = (t.album || "").trim();
    if (!a) return;
    if (!map.has(a)) map.set(a, { source: "local", id: "loca:" + a, title: a, artist: artistName(t), count: 0, art: t.artwork_url });
    map.get(a).count++;
  });
  let list = [...map.values()].filter((a) => a.count > 1);
  if (page === "search" && q) {
    const n = q.toLowerCase();
    list = list.filter((a) => (a.title + " " + a.artist).toLowerCase().includes(n));
  }
  if (page === "rediscover" || page === "discover") list = shuffleInPlace(list).slice(0, 6);
  return list;
}

async function scUserAlbums(uid) {
  if (!uid) return [];
  const d = await api("/users/" + uid + "/playlists?limit=50&linked_partitioning=true&show_tracks=false");
  return asItems(d).filter(isScAlbum).map(scAlbumTile);
}

async function fetchAlbums(p) {
  if (p.view === "artist") return scUserAlbums(p.artist && p.artist.id);
  const plat = curPlat(p.view);
  if (plat === "local") return localAlbumTiles(p.view, p.query);
  if (plat === "spotify") {
    if (p.view === "likes" || p.view === "rediscover") {
      const d = await spApi("GET", "/me/albums?limit=50");
      const list = (d.items || []).map((x) => x.album).filter((a) => a && a.id).map(spAlbumTile);
      return p.view === "rediscover" ? shuffleInPlace(list).slice(0, 6) : list;
    }
    if (p.view === "search" && p.query) {
      const d = await spApi("GET", "/search?type=album&limit=10&q=" + encodeURIComponent(p.query));
      return ((d.albums && d.albums.items) || []).filter((a) => a && a.id).map(spAlbumTile);
    }
    return [];   // Discover: built from the recommended tracks' own albums as the list draws
  }
  if (plat !== "soundcloud") return [];
  if (p.view === "likes" || p.view === "rediscover") {
    const list = asItems(await api("/me/likes/playlists?limit=50")).filter(isScAlbum).map(scAlbumTile);
    return p.view === "rediscover" ? shuffleInPlace(list).slice(0, 6) : list;
  }
  if (p.view === "search" && p.query) {
    const d = await api("/playlists?q=" + encodeURIComponent(p.query) + "&limit=30&linked_partitioning=true&show_tracks=false");
    return asItems(d).filter(isScAlbum).map(scAlbumTile);
  }
  if (p.view === "discover") {
    const src = p.discoverSrc || "";
    if (src.startsWith("genre:")) {
      const d = await api("/playlists?q=" + encodeURIComponent(src.slice(6)) + "&limit=30&linked_partitioning=true&show_tracks=false");
      return shuffleInPlace(asItems(d).filter(isScAlbum)).slice(0, 6).map(scAlbumTile);
    }
    // albums by the artists in the recommendations (wait briefly for the list)
    for (let i = 0; i < 30 && !viewTracks.length; i++) await new Promise((r) => setTimeout(r, 200));
    const ids = [...new Set(viewTracks.map((t) => t.user && t.user.id).filter(Boolean))].slice(0, 4);
    const lists = await Promise.all(ids.map((id) => scUserAlbums(id).catch(() => [])));
    return shuffleInPlace(lists.flat()).slice(0, 6);
  }
  return [];
}

// Called from navTrack whenever the page changes
function albumsOnPage(p) {
  const key = pageKey(p);
  if (key === albumsKey) return;
  albumsKey = key;
  viewAlbums = [];
  if (!albumsShown() || !ALBUM_PAGES.includes(p.view)) return;
  fetchAlbums(p).then((list) => {
    if (albumsKey !== key) return;
    viewAlbums = list || [];
    render();
  }).catch(() => {});
}

function paintAlbumToggle() {
  const b = $("albumToggle");
  b.hidden = !ALBUM_PAGES.includes(view);
  b.classList.toggle("on", albumsShown());
  b.setAttribute("aria-pressed", albumsShown() ? "true" : "false");
  b.title = albumsShown() ? "Hide albums (their tracks stay in the list)" : "Show albums";
}

function albumTileEl(a) {
  const li = el("li", "albumTile");
  li._album = a;
  const img = el("img", "albumArt");
  img.alt = "";
  img.loading = "lazy";
  if (a.art) img.src = a.art;
  const info = el("div", "albumInfo");
  const kind = el("div", "albumKind");
  kind.append(platformBadge({ source: a.source }), document.createTextNode("Album"));
  info.append(
    kind,
    el("div", "albumTitle", a.title),
    el("div", "albumArtist", a.artist || ""),
    el("div", "albumCount", a.count ? a.count + (a.count === 1 ? " track" : " tracks") : ""),
  );
  const play = el("button", "albumPlay");
  play.title = "Play album";
  play.setAttribute("aria-label", "Play " + a.title);
  play.append(icon(PLAY_ICON));
  li.append(img, info, play);
  return li;
}

function injectAlbums() {
  paintAlbumToggle();
  if (!albumsShown() || !ALBUM_PAGES.includes(view)) return;
  if (view === "likes" && likesFilter !== "all") return;
  let tiles = viewAlbums.slice();
  if ((view === "discover" || view === "search") && curPlat(view) === "spotify") {
    const seen = new Set(tiles.map((a) => a.id));
    viewTracks.forEach((t) => {
      const a = t.spAlbum;
      if (a && a.album_type === "album" && a.total_tracks > 1 && !seen.has("spa:" + a.id)) {
        seen.add("spa:" + a.id);
        tiles.push(spAlbumTile(a));
      }
    });
    tiles = tiles.slice(0, 10);
  }
  if (!tiles.length) return;
  const box = $("list");
  const rows = [...box.children];
  // one tile every 6 tracks; a couple more at the end, and more appear as the list grows
  tiles.slice(0, Math.ceil(rows.length / 6) + 2).forEach((a, k) => {
    const before = rows[(k + 1) * 6];
    if (before) box.insertBefore(albumTileEl(a), before);
    else box.append(albumTileEl(a));
  });
}

function albumAsPlaylist(a) {
  if (a.source === "soundcloud") return Object.assign({}, a.raw);
  if (a.source === "spotify") return { source: "spotify", id: a.id, spAlbumId: a.spAlbumId, title: a.title, user: { username: a.artist } };
  return { source: "local", id: "album:" + a.title, album: a.title, title: a.title, user: { username: a.artist } };
}

async function albumTracks(a) {
  if (a.source === "local") return localTracks.filter((t) => (t.album || "").trim() === a.title);
  if (a.source === "spotify") return spotifyAlbumTracks(a.spAlbumId);
  return asList(await api("/playlists/" + a.raw.id + "/tracks?access=playable&limit=200&linked_partitioning=true"));
}

async function playAlbum(a) {
  say("Loading " + a.title + "...");
  try {
    const list = await albumTracks(a);
    if (!list.length) { say("No playable tracks in this album."); return; }
    say("");
    startList(list, 0, false);
  } catch (e) {
    say(String(e));
  }
}

function albumsInit() {
  $("albumToggle").addEventListener("click", () => {
    prefs.showAlbums = !albumsShown();
    savePrefs();
    albumsKey = "";
    if (albumsShown() && curPage) albumsOnPage(curPage);
    render();
  });
  // Album tiles handle their own clicks, ahead of the track-row handlers
  const tileOf = (e) => e.target.closest && e.target.closest(".albumTile");
  $("list").addEventListener("click", (e) => {
    const li = tileOf(e);
    if (!li) return;
    e.stopPropagation();
    if (e.target.closest(".albumPlay")) playAlbum(li._album);
    else openPlaylist(albumAsPlaylist(li._album));
  }, true);
  ["dblclick", "contextmenu"].forEach((type) => $("list").addEventListener(type, (e) => {
    if (!tileOf(e)) return;
    e.stopPropagation();
    if (type === "contextmenu") e.preventDefault();
  }, true));
}
