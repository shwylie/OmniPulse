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
    discoverSrc, listFilter, plat: activePlats(v).join(","),
  };
}

function pageKey(p) {
  return [p.view, p.query, p.playlist && p.playlist.id, p.artist && p.artist.id, p.discoverSrc, p.listFilter, p.plat].join("|");
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
  listFilter = p.listFilter || "all";
  switch (p.view) {
    case "likes": openLikes(); break;
    case "favourites": openFavourites(); break;
    case "sets": openSets(); break;
    case "artists": openArtistsPage(); break;
    case "search": if (p.query) openSearch(p.query); else openSearchPage(); break;
    case "playlist": if (p.playlist) openPlaylist(p.playlist); else openLikes(); break;
    case "artist": if (p.artist) openArtist(p.artist); else openLikes(); break;   // the merged artist (js/artists.js)
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

// ---------- the sidebar's pages, in your order ----------
// Drag a page up or down the list; the order is kept in prefs.navOrder (pages added later go last).
function navOrderApply() {
  const box = $("navList");
  const order = prefs.navOrder || [];
  const buttons = [...box.querySelectorAll(":scope > .nav")];
  const rank = (b) => { const i = order.indexOf(b.id); return i < 0 ? order.length + buttons.indexOf(b) : i; };
  buttons.slice().sort((a, b) => rank(a) - rank(b)).forEach((b) => box.append(b));
}

function navOrderInit() {
  const box = $("navList");
  let dragged = null;
  box.querySelectorAll(":scope > .nav").forEach((b) => { b.draggable = true; });
  box.addEventListener("dragstart", (e) => {
    const b = e.target.closest(".nav");
    if (!b) return;
    dragged = b;
    b.classList.add("dragging");
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", b.id);
  });
  box.addEventListener("dragover", (e) => {
    if (!dragged) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = "move";
    const over = e.target.closest(".nav");
    if (!over || over === dragged) return;
    const r = over.getBoundingClientRect();
    box.insertBefore(dragged, e.clientY < r.top + r.height / 2 ? over : over.nextSibling);
  });
  const done = () => {
    if (!dragged) return;
    dragged.classList.remove("dragging");
    dragged = null;
    prefs.navOrder = [...box.querySelectorAll(":scope > .nav")].map((b) => b.id);
    savePrefs();
  };
  box.addEventListener("drop", (e) => { if (dragged) { e.preventDefault(); done(); } });
  box.addEventListener("dragend", done);
  navOrderApply();
}

function navInit() {
  $("backBtn").addEventListener("click", goBack);
  navOrderInit();
}
