// Favourites: a star on any track, from any platform. Kept on this PC (the same small snapshots
// as playlists on this PC), newest first, and listed on the Favourites page.

const FAV_KEY = "sc.favourites";
const ICON_STAR = "M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z";
const ICON_STAR_OUT = "M22 9.24l-7.19-.62L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21 12 17.27 18.18 21l-1.63-7.03L22 9.24zM12 15.4l-3.76 2.27 1-4.28-3.32-2.88 4.38-.38L12 6.1l1.71 4.04 4.38.38-3.32 2.88 1 4.28L12 15.4z";

let favs = [];
try { favs = JSON.parse(localStorage.getItem(FAV_KEY) || "[]"); } catch (e) { favs = []; }
let favIds = new Set(favs.map((s) => s.id));

const isFav = (t) => !!t && favIds.has(t.id);

function favSave() {
  try { localStorage.setItem(FAV_KEY, JSON.stringify(favs)); } catch (e) { say("Could not save your favourites: " + e); }
}

function setStar(btn, on) {
  btn.classList.toggle("on", on);
  btn.replaceChildren(icon(on ? ICON_STAR : ICON_STAR_OUT));
  btn.title = on ? "Remove from Favourites" : "Add to Favourites";
  btn.setAttribute("aria-label", btn.title);
}

function toggleFav(t) {
  if (!t) return;
  if (isFav(t)) {
    favs = favs.filter((s) => s.id !== t.id);
    favIds.delete(t.id);
    say("Removed from Favourites.");
  } else {
    favs.unshift(xplSnap(t));
    favIds.add(t.id);
    say("Added to Favourites.");
  }
  favSave();
  if (view === "favourites") {
    viewTracks = viewTracks.filter((x) => isFav(x));
    render();
  }
  refreshFavUI();
}

function refreshFavUI() {
  const t = curTrack();
  $("favBtn").hidden = !t;
  if (t) setStar($("favBtn"), isFav(t));
  const nb = $("npFav");
  nb.hidden = !t;
  if (t) {
    nb.textContent = isFav(t) ? "Favourite" : "Add to Favourites";
    nb.classList.toggle("on", isFav(t));
  }
  document.querySelectorAll("#list li").forEach((li) => {
    const t2 = viewTracks[Number(li.dataset.i)];
    const b = li.querySelector('button[data-act="fav"]');
    if (t2 && b && b.classList.contains("on") !== isFav(t2)) setStar(b, isFav(t2));
  });
}

function openFavourites() {
  currentPlaylist = null;
  currentArtist = null;
  showView("favourites");
  ++loadToken;
  $("listView").scrollTop = 0;
  setLocalPool(xplTracks({ tracks: favs }));
  render();
  say(viewTracks.length ? "" : "Nothing here yet. Star a track (next to its heart) to add it.");
}

function favouritesInit() {
  $("navFavourites").addEventListener("click", openFavourites);
  $("favBtn").addEventListener("click", (e) => { e.stopPropagation(); toggleFav(curTrack()); });
  $("npFav").addEventListener("click", () => toggleFav(curTrack()));
}
