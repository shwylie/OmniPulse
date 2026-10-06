// The Back button and page cache.

// ---------- back button ----------
const navStack = [];
const pageCache = new Map();   // Discover / Rediscover lists, so going back shows the same thing

function useCachedPage() {
  const hit = curPage && pageCache.get(pageKey(curPage));
  if (!hit) return false;
  ++loadToken;   // cancel anything still loading
  viewTracks = hit.tracks.slice();
  nextHref = hit.nextHref;
  localPool = hit.pool || null;
  localPos = hit.pos || 0;
  render();
  say("");
  requestAnimationFrame(() => { $("listView").scrollTop = hit.scroll; });
  return true;
}
let curPage = null;
let restoring = false;

function pageSnapshot(v) {
  return {
    view: v, query, playlist: currentPlaylist, artist: currentArtist,
    discoverSrc, likesFilter, plat: curPlat(v),
  };
}

function pageKey(p) {
  return [p.view, p.query, p.playlist && p.playlist.id, p.artist && p.artist.id, p.discoverSrc, p.likesFilter, p.plat].join("|");
}

// Called from showView: remembers where you were before this page
function navTrack(v) {
  const now = pageSnapshot(v);
  if (curPage && pageKey(curPage) !== pageKey(now) && (curPage.view === "discover" || curPage.view === "rediscover")) {
    pageCache.set(pageKey(curPage), { tracks: viewTracks.slice(), nextHref, scroll: $("listView").scrollTop, pool: localPool, pos: localPos });
  }
  if (curPage && !restoring && pageKey(curPage) !== pageKey(now)) {
    navStack.push(curPage);
    if (navStack.length > 50) navStack.shift();
  }
  curPage = now;
  albumsOnPage(now);
  $("backBtn").disabled = navStack.length === 0;
  renderPlatChips();
}

function restorePage(p) {
  likesFilter = p.likesFilter || "all";
  switch (p.view) {
    case "likes": openLikes(); break;
    case "search": if (p.query) openSearch(p.query); else openSearchPage(); break;
    case "playlist": if (p.playlist) openPlaylist(p.playlist); else openLikes(); break;
    case "artist": if (p.artist) openArtist(p.artist); else openLikes(); break;
    case "discover": openDiscover(p.discoverSrc); break;
    case "rediscover": openRediscover(); break;
    case "stats": showView("stats"); break;
    case "now": showView("now"); break;
    default: openLikes();
  }
}

function goBack() {
  const p = navStack.pop();
  if (!p) return;
  restoring = true;
  try { restorePage(p); } finally { restoring = false; }
  $("backBtn").disabled = navStack.length === 0;
}

function navInit() {
  $("backBtn").addEventListener("click", goBack);
}
