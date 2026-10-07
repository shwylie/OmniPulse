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
  const seen = new Set();
  const dedupe = (list) => list.filter((t) => !seen.has(t.id) && seen.add(t.id));
  const mine = dedupe(liked);
  setLocalPool(mine);
  render();
  if (!prefs.setsLiked && plats.includes("soundcloud")) {
    if (mine.length) say("Checking artists you follow...");
    const more = (await collectPages("/me/followings/tracks?access=playable&limit=100&linked_partitioning=true", 3).catch(() => []))
      .filter((t) => isMix(t) && !isLikedTrack(t) && !hiddenRecs.has(String(t.id)));
    if (tok !== loadToken) return;
    const extra = dedupe(more);
    if (extra.length) {
      const top = $("listView").scrollTop;
      const shown = viewTracks.length;
      setLocalPool(mine.concat(extra));
      if (shown > viewTracks.length) { localPos = Math.min(localPool.length, shown); viewTracks = localPool.slice(0, localPos); nextHref = localPos < localPool.length ? "local:" : ""; }
      render();
      $("listView").scrollTop = top;
    }
  }
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
