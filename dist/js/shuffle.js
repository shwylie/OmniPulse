// Shuffling that doesn't keep serving the same songs. Tracks you've never heard come first, then
// the rest from longest ago to most recent, in four groups shuffled within themselves, so a song
// you just heard goes to the back. It goes by the play history (js/history.js), which is kept
// between sessions. Tracks you queued yourself and chat requests aren't reordered.

function fairShuffle(list) {
  const never = [];
  const heard = [];
  list.forEach((t) => (lastHeard(t) ? heard : never).push(t));
  heard.sort((a, b) => lastHeard(a) - lastHeard(b));
  const size = Math.max(1, Math.ceil(heard.length / 4));
  const groups = [];
  for (let i = 0; i < heard.length; i += size) groups.push(shuffleInPlace(heard.slice(i, i + size)));
  return shuffleInPlace(never).concat(...groups);
}

// Takes turns between platforms: A, B, A, B... (or A, B, C... with three). Each platform's own
// order is kept. Once only one platform has tracks left the list stops there, so the same platform
// never plays twice in a row, unless it's the only one in the list.
function alternatePlatforms(list) {
  const by = new Map();
  list.forEach((t) => {
    const p = srcOf(t);
    if (!by.has(p)) by.set(p, []);
    by.get(p).push(t);
  });
  if (by.size < 2) return list.slice();
  const lists = [...by.values()];
  const rounds = Math.min(...lists.map((l) => l.length));   // an even split
  const out = [];
  for (let i = 0; i < rounds; i++) lists.forEach((l) => out.push(l[i]));
  return out;
}

// Everything a Shuffle button should shuffle: on Library, all your likes (with the Tracks / Sets &
// podcasts filter); on pages the app holds in full, the whole list; otherwise what's loaded
async function shuffleSource() {
  if (view === "likes") {
    const plats = activePlats("likes");
    await loadLikesFor(plats).catch(() => null);
    const all = []
      .concat(plats.includes("soundcloud") ? likedTracks.filter(isLikedTrack) : [])
      .concat(plats.includes("spotify") ? spLikedCache || [] : [])
      .concat(plats.includes("local") ? localTracks : []);
    if (all.length) return applyListFilter(all);
  }
  if (localPool && localPool.length > viewTracks.length && nextHref === "local:") return localPool.slice();
  return viewTracks.slice();
}

// The order a shuffled list plays in: fair, and on Rediscover still taking turns between platforms
function shuffledOrder(list) {
  if (view !== "rediscover") return fairShuffle(list);
  const by = new Map();
  list.forEach((t) => { const p = srcOf(t); if (!by.has(p)) by.set(p, []); by.get(p).push(t); });
  return alternatePlatforms([].concat(...[...by.values()].map(fairShuffle)));
}
