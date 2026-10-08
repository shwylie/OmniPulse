// Track titles: promo-tag cleanup, "Artist - Title" splitting and your own fixes.

// ---------- tidy titles: drop promo tags like "FREE DL" ----------
const JUNK_RE = /\b(?:free\s*(?:dl|d\/l|download|dwnld)|free\s*=?\s*buy|buy\s*=\s*free|click\s*buy|out\s*now|premiere|exclusive|supported\s+by\b.*|played\s+by\b.*|download\s+(?:link|in\s+desc\w*)|link\s+in\s+(?:bio|desc\w*)|hypeddit|toneden)/i;
const JUNK_G = new RegExp(JUNK_RE.source, "gi");
const SEP_JUNK = /\b(?:free\s*(?:dl|d\/l|download|dwnld)|click\s*buy|buy\s*=\s*free|out\s*now|supported\s+by|played\s+by|link\s+in)\b/i;
const KEEP_RE = /\b(?:re-?mix|mix|edit|bootleg|flip|vip|feat\.?|ft\.?|prod\.?|rework|cover|version|instrumental|acapella|live|remaster\w*)\b/i;
const ALWAYS_JUNK_G = /\s*\b(?:free\s*(?:dl|d\/l|download|dwnld)|click\s*buy|buy\s*=\s*free|free\s*=\s*buy|out\s*now|link\s+in\s+(?:bio|desc\w*))\b[!.]*/gi;

function cleanTitle(title) {
  if (!title) return title;
  let t = String(title);
  // "Premiere: Artist - Title"
  t = t.replace(/^\s*(?:premiere|exclusive)\s*[:|\-\u2013\u2014]\s*/i, "");
  // bracketed tags: (FREE DL), [OUT NOW], {Premiere}; "(Exclusive Remix)" keeps "(Remix)"
  t = t.replace(/[\(\[\{\u3010]([^\)\]\}\u3011]*)[\)\]\}\u3011]/g, (m, inner) => {
    if (!JUNK_RE.test(inner)) return m;
    if (!KEEP_RE.test(inner)) return " ";
    const kept = inner.replace(JUNK_G, " ").replace(/\s{2,}/g, " ").trim();
    return kept ? m[0] + kept + m[m.length - 1] : " ";
  });
  // promo text after a separator: "Title - FREE DL", "Title | Supported by ..."
  t = t.replace(/\s+(?:\||\/\/|-{1,2}|\u2013|\u2014|~)\s+([^|\-\u2013\u2014~]+)$/, (m, rest) =>
    SEP_JUNK.test(rest) && !KEEP_RE.test(rest) ? "" : m);
  // **FREE DOWNLOAD**
  t = t.replace(/\*+([^*]*)\*+/g, (m, inner) => (JUNK_RE.test(inner) ? " " : m));
  // phrases that are never part of a title
  t = t.replace(ALWAYS_JUNK_G, " ");
  t = t.replace(/\s{2,}/g, " ").replace(/\s+([,\)\]])/g, "$1")
    .replace(/[\s\-\u2013\u2014|~:]+$/, "").replace(/^[\s\-\u2013\u2014|~:]+/, "").trim();
  return t || String(title);
}

function tidyTrack(t) {
  if (t && t.title) t.title = cleanTitle(t.title);
  if (t && t.title && !t.displayArtist) splitArtist(t);
  if (t) tidyFeat(t);
  if (t) applyFix(t);
  return t;
}

// ---------- featured artists ----------
// "Song (feat. X) (feat. X)": the repeat goes, and X joins the artists (if they aren't there already),
// so the title is just the song and X is a link like the others. Repeated tags ("(Remix) (Remix)")
// are kept once.
const FEAT_RE = /\s*[(\[]\s*(?:feat\.?|ft\.?|featuring)\s+([^)\]]+)[)\]]|\s+(?:feat\.?|ft\.?|featuring)\s+([^()\[\]]+?)\s*$/gi;
const nameKey = (s) => String(s).toLowerCase().normalize("NFKD").replace(/[^a-z0-9\u00c0-\u024f\u0400-\u04ff\u3040-\u9fff]/g, "");
const splitNames = (s) => String(s).split(/\s*(?:,|&|\band\b|\bx\b|\bvs\.?\b)\s*/i).map((x) => x.trim()).filter(Boolean);

function tidyFeat(t) {
  if (!t || !t.title) return t;
  const featured = [];
  let title = t.title.replace(FEAT_RE, (m, a, b) => { featured.push(...splitNames(a || b)); return ""; });
  // the same tag twice
  const seen = new Set();
  title = title.replace(/\s*([(\[])([^)\]]+)([)\]])/g, (m, o, inner) => {
    const k = nameKey(inner);
    if (seen.has(k)) return "";
    seen.add(k);
    return m;
  }).replace(/\s{2,}/g, " ").trim();
  if (!featured.length) { if (title && title !== t.title) t.title = title; return t; }
  const artist = artistName(t);
  const have = new Set(splitNames(artist).map(nameKey));
  const add = [];
  featured.forEach((n) => { const k = nameKey(n); if (k && !have.has(k)) { have.add(k); add.push(n); } });
  if (title) t.title = title;
  if (add.length) t.displayArtist = [artist].concat(add).filter(Boolean).join(", ");
  return t;
}

// The album line, only when it says something the title and artist don't: "Artist - Song" or the
// song's own name as its album (a single) aren't shown
const ALBUM_FILLER = new Set(["single", "ep", "the", "a", "feat", "ft", "featuring", "x", "and", "vs"]);
function albumShown(t) {
  const album = (t && (t.album || (t.spAlbum && t.spAlbum.name))) || "";
  if (!album) return "";
  const words = (s) => String(s).toLowerCase().normalize("NFKD").split(/[^a-z0-9\u00c0-\u024f\u0400-\u04ff\u3040-\u9fff]+/).filter(Boolean);
  const have = new Set(words((t.title || "") + " " + artistName(t)));
  return words(album).every((w) => have.has(w) || ALBUM_FILLER.has(w)) ? "" : album;
}

// ---------- your own title / artist fixes (right-click > Edit title and artist) ----------
// Applied wherever a track is shown (a list row, the queue, now playing, a tile), not only when it's
// loaded: copies held elsewhere (YouTube Music and Spotify likes, your files, playlists on this PC, a
// page kept from before) show the change too, and go back when it's undone.
function applyFix(t) {
  if (!t) return t;
  const fx = (prefs.trackFixes || {})[t.id];
  if (!t._auto) t._auto = { title: t.title, artist: t.displayArtist || null };
  if (fx) {
    if (fx.title) t.title = fx.title;
    t.displayArtist = fx.artist || t._auto.artist || undefined;
    t._fixed = true;
  } else if (t._fixed) {
    t.title = t._auto.title;
    t.displayArtist = t._auto.artist || undefined;
    t._fixed = false;
  }
  return t;
}

// Every copy of a track the app is holding (lists, queue, likes) gets the change
function forEachCopy(id, fn) {
  const seen = new Set();
  [viewTracks, queue, likedTracks].forEach((list) => list.forEach((x) => {
    if (x && x.id === id && !seen.has(x)) { seen.add(x); fn(x); }
  }));
}

function refreshAfterFix(id) {
  render();
  renderQueue();
  const cur = curTrack();
  if (cur && cur.id === id) { fillNowPlaying(cur); presenceUpdate(); widgetPush(); }
}

let fixTarget = null;

// A guess for titles that are really file names, e.g. "thescript-forthefirsttime(8barzremix).mp3"
function guessFromFilename(t) {
  let s = (t.title || "").replace(/\.(mp3|wav|flac|m4a|aac|ogg|aiff?)$/i, "").replace(/_/g, " ").trim();
  const m = s.match(/^([^-]+?)\s*-\s*(.+)$/);
  return m ? { artist: m[1].trim(), title: m[2].trim() } : { artist: artistName(t), title: s };
}

function openFixDialog(t) {
  fixTarget = t;
  const fx = (prefs.trackFixes || {})[t.id];
  const looksLikeFile = /\.(mp3|wav|flac|m4a|aac|ogg|aiff?)$/i.test(t.title || "") || (!/\s/.test(t.title || "") && /[-_]/.test(t.title || ""));
  const guess = !fx && looksLikeFile ? guessFromFilename(t) : { title: t.title, artist: artistName(t) };
  $("fixTitle").value = guess.title || "";
  $("fixArtist").value = guess.artist || "";
  $("fixReset").hidden = !fx && !trimOf(t);
  trimDialogOpen(t);
  $("fixDialog").showModal();
  $("fixTitle").focus();
  $("fixTitle").select();
}

function saveFix(e) {
  e.preventDefault();
  const t = fixTarget;
  if (!t) return;
  if (!trimDialogSave()) return;   // the start and end times (js/trims.js)
  const title = $("fixTitle").value.trim();
  const artist = $("fixArtist").value.trim();
  // the title and artist only count as an edit if they were changed
  if (!(prefs.trackFixes || {})[t.id] && title === (t.title || "") && artist === artistName(t)) {
    $("fixDialog").close();
    say("Saved.");
    return;
  }
  prefs.trackFixes = prefs.trackFixes || {};
  prefs.trackFixes[t.id] = { title: title || t._auto.title, artist: artist || null };
  savePrefs();
  forEachCopy(t.id, (x) => {
    if (!x._auto) x._auto = { title: x.title, artist: x.displayArtist || null };
    x.title = title || x._auto.title;
    x.displayArtist = artist || x._auto.artist || undefined;
  });
  $("fixDialog").close();
  refreshAfterFix(t.id);
  say("Saved. It will show like this from now on.");
}

function resetFix() {
  const t = fixTarget;
  if (!t) return;
  if (prefs.trackFixes) delete prefs.trackFixes[t.id];
  trimReset(t);
  savePrefs();
  forEachCopy(t.id, (x) => {
    if (x._auto) { x.title = x._auto.title; x.displayArtist = x._auto.artist || undefined; }
  });
  $("fixDialog").close();
  refreshAfterFix(t.id);
  say("Back to the original title and artist.");
}

// "Artist - Title": show the artist from the title instead of the uploader, and drop it from the title.
// "Song - Extended Mix" is left alone, since the part after the dash is just a version name.
const DESCRIPTOR_ONLY = /^[^()\[\]]*\b(?:mix|edit|remix|version|vip|bootleg|flip|rework|dub|instrumental|acapella|remaster(?:ed)?)\s*$/i;

function splitArtist(t) {
  const m = t.title.match(/^(.+?)\s+[-\u2013\u2014]\s+(.+)$/);
  if (!m) return;
  const left = m[1].trim();
  const right = m[2].trim();
  if (!left || !right || left.length > 80 || /[\(\[]/.test(left)) return;
  // "Song - Extended Mix" is a version name, but "Artist - Song - VIP" still has a song in it
  if (!/\s[-\u2013\u2014]\s/.test(right) && DESCRIPTOR_ONLY.test(right)) return;
  t.displayArtist = left;
  t.title = right;
}

// The artist shown for a track (from the title when there is one, otherwise the uploader)
function artistName(t) {
  return (t && (t.displayArtist || (t.user && t.user.username))) || "";
}

// Hover text on the artist: who actually uploaded it, when that's someone else
function uploaderNote(t) {
  const up = t && t.user && t.user.username;
  if (t && t.displayArtist && up && up.toLowerCase() !== t.displayArtist.toLowerCase()) {
    return "Uploaded by " + up + ". Click to open their page.";
  }
  return "Open artist page";
}

function titlesInit() {
  $("fixForm").addEventListener("submit", saveFix);
  $("fixCancel").addEventListener("click", () => $("fixDialog").close());
  $("fixReset").addEventListener("click", resetFix);
}
