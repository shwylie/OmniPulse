// Sets & podcasts: tracks over 15 minutes. The ones in your likes (any platform, plus your own
// files), then new ones from artists you follow on SoundCloud; "Liked only" leaves out the latter.

async function openSets() {
  currentPlaylist = null;
  currentArtist = null;
  showView("sets");
  $("listView").scrollTop = 0;
  const tok = ++loadToken;
  const plats = activePlats();
  if (!plats.length) { showNothing(); return; }
  say("Finding sets and podcasts...");
  await loadLikesFor(plats).catch(() => null);
  if (tok !== loadToken) return;
  const liked = []
    .concat(plats.includes("soundcloud") ? likedTracks.filter(isLikedTrack) : [])
    .concat(plats.includes("spotify") ? spLikedCache || [] : [])
    .concat(plats.includes("local") ? localTracks : [])
    .filter(isMix);
  let more = [];
  if (!prefs.setsLiked && plats.includes("soundcloud")) {
    more = (await collectPages("/me/followings/tracks?access=playable&limit=100&linked_partitioning=true", 3).catch(() => []))
      .filter((t) => isMix(t) && !isLikedTrack(t) && !hiddenRecs.has(String(t.id)));
    if (tok !== loadToken) return;
  }
  const seen = new Set();
  setLocalPool(liked.concat(more).filter((t) => !seen.has(t.id) && seen.add(t.id)));
  render();
  say(viewTracks.length ? "" : prefs.setsLiked
    ? "No sets or podcasts in your likes yet. Anything over 15 minutes counts."
    : "No sets or podcasts found. Anything over 15 minutes counts.");
}

function paintSetsChip() {
  const c = $("setsLikedChip");
  c.hidden = view !== "sets";
  c.classList.toggle("on", !!prefs.setsLiked);
  c.setAttribute("aria-pressed", prefs.setsLiked ? "true" : "false");
  c.title = prefs.setsLiked ? "Showing only the ones you've liked. Click to include artists you follow." : "Show only the ones you've liked";
}

const setsOn = () => prefs.setsOn !== false;

function applySetsOn() {
  $("setsOn").checked = setsOn();
  $("navSets").hidden = !setsOn();
  if (!setsOn() && view === "sets") openLikes();
}

function setsInit() {
  $("setsOn").addEventListener("change", () => { prefs.setsOn = $("setsOn").checked; savePrefs(); applySetsOn(); });
  applySetsOn();
  $("navSets").addEventListener("click", () => openSets());
  $("setsLikedChip").addEventListener("click", () => {
    prefs.setsLiked = !prefs.setsLiked;
    savePrefs();
    openSets();
  });
}
