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
  if (t) applyFix(t);
  return t;
}

// ---------- your own title / artist fixes (right-click > Edit title and artist) ----------
function applyFix(t) {
  const fx = (prefs.trackFixes || {})[t.id];
  if (!t._auto) t._auto = { title: t.title, artist: t.displayArtist || null };
  if (!fx) return;
  if (fx.title) t.title = fx.title;
  if (fx.artist) t.displayArtist = fx.artist;
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
  $("fixReset").hidden = !fx;
  $("fixDialog").showModal();
  $("fixTitle").focus();
  $("fixTitle").select();
}

function saveFix(e) {
  e.preventDefault();
  const t = fixTarget;
  if (!t) return;
  const title = $("fixTitle").value.trim();
  const artist = $("fixArtist").value.trim();
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
