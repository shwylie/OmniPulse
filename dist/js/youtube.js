// YouTube Music, without signing in: search and radio from YouTube Music itself (crates/core/src/youtube.rs),
// audio through yt-dlp, played via the local server's /yt (which fetches it in pieces). Likes are kept on
// this PC, like favourites, so they show in your Library and seed autoplay and Rediscover.

let ytInfo = { ytdlp: null, deno: false, node: false, checked: false };
const ytAvailable = () => !!ytInfo.ytdlp && (ytInfo.deno || ytInfo.node);

// A YouTube track's video id: its own, or from its id ("yt:<video>"). Tracks kept on this PC (likes,
// playlists, the queue) were saved without ytId before, so it's never taken for granted.
const ytIdOf = (t) => (t && (t.ytId || (String(t.id || "").startsWith("yt:") ? String(t.id).slice(3) : ""))) || "";

function ytTrack(x) {
  return tidyTrack({
    source: "youtube",
    id: "yt:" + x.id,
    ytId: x.id,
    title: x.title,
    user: { username: x.artists || "Unknown artist" },
    ytArtistId: x.artist_id || "",
    album: x.album || "",
    duration: x.duration_ms || 0,
    artwork_url: x.thumb || "",
    permalink_url: "https://music.youtube.com/watch?v=" + x.id,
  });
}

async function ytSearch(q) {
  return (await invoke("yt_search", { query: q })).map(ytTrack);
}

// YouTube Music's radio for a song (the song itself is left out)
async function ytRadio(videoId) {
  return (await invoke("yt_radio", { id: videoId })).map(ytTrack).filter((t) => t.ytId !== videoId);
}

// The YouTube Music song for any track: itself, or the best search match for its artist and title
async function ytFind(t) {
  if (srcOf(t) === "youtube") return t;
  const found = await ytSearch((seedArtist(t) + " " + guessNames(t).title).trim()).catch(() => []);
  return found[0] || null;
}

// ---------- the same song on YouTube Music ----------
// For tracks that can't be played where they're from (Go+ previews, Spotify without Premium). The
// search results are scored on title words, artist, length and version tags (remix, live, ...), and
// a weak best match counts as none rather than playing the wrong song. Matches are remembered.
const YT_MATCH_KEY = "sc.ytMatch";
let ytMatches = null;   // track id -> YouTube track snapshot, or null for "none found"

const ytOn = () => ytWanted() && ytAvailable();
// What uploads add to a title that says nothing about the song: "(Official Audio)", "[Lyric Video]",
// "| Official Music Video". Only bracketed or trailing parts go, so "Video Games" stays a title.
const UPLOAD_TAG = /\b(official|audio|video|lyrics?|visuali[sz]er|hd|hq|4k|mv|m\/v)\b/i;
const uploadTitle = (s) => String(s || "")
  .replace(/[(\[]([^)\]]*)[)\]]/g, (m, inner) => (UPLOAD_TAG.test(inner) ? " " : m))
  .replace(/\s[|\uFF5C]\s.*$/, " ");
const VERSION_WORDS = ["remix", "edit", "mix", "vip", "flip", "bootleg", "live", "acoustic", "instrumental", "remaster", "cover", "sped", "slowed", "nightcore", "karaoke"];

// A track's title for matching: without "Artist - " in front (SoundCloud and uploads put it there) or
// upload tags, but with its version: "(Remix)", "(Extended Mix)", Spotify's " - Radio Edit"
function matchTitle(t) {
  let title = String((t && t.title) || "").trim();
  if (srcOf(t) !== "spotify") {
    const m = title.match(/^(.+?)\s+[-\u2013\u2014]\s+(.+)$/);
    if (m) title = m[2];
  }
  return uploadTitle(title);
}

function matchWords(s) {
  return String(s || "").replace(/[(\[]\s*original( mix| version)?\s*[)\]]|\boriginal mix\b/gi, " ").toLowerCase().normalize("NFKD").replace(/[\u0300-\u036f]/g, "")
    .replace(/\b(feat|ft|featuring|prod|with)\b\.?/g, " ").replace(/[^a-z0-9]+/g, " ").trim().split(" ").filter(Boolean);
}

function ytMatchScore(t, c) {
  const names = guessNames(t);
  const artist = matchWords(names.artist || artistName(t));
  const want = new Set(matchWords(matchTitle(t)));
  // the artist's name in an upload's title ("Artist - Song") isn't part of the song's title
  const got = new Set(matchWords(uploadTitle(c.title)).filter((w) => !(artist.includes(w) && !want.has(w))));
  if (!want.size || !got.size) return 0;
  let shared = 0;
  want.forEach((w) => { if (got.has(w)) shared++; });
  // the title has to be (nearly) the same: "Track 7" is not "Track 8"
  let score = shared / Math.max(want.size, got.size);
  if (score < 0.75) return 0;
  // and so does the artist (credited, or named in the title): otherwise it's likely a cover
  const where = new Set(matchWords(artistName(c) + " " + c.title));
  if (artist.length && artist.some((w) => where.has(w))) score += 0.3;
  else score -= 0.6;
  // a remix isn't the original (and the other way round)
  VERSION_WORDS.forEach((v) => { if (want.has(v) !== got.has(v)) score -= 0.6; });
  // the length, when both are known
  const d1 = t.full_duration || t.duration || 0;
  const d2 = c.duration || 0;
  if (d1 > 40000 && d2) {
    const off = Math.abs(d1 - d2) / d1;
    score += off < 0.03 ? 0.3 : off < 0.1 ? 0.1 : off > 0.25 ? -0.5 : 0;
  }
  return score;
}

const YT_NO_MATCH_TTL = 86400000;   // "no match" is looked for again after a day
let ytLastMiss = null;              // { id, closest }: why the last match came up empty

const ytSnap = (c) => ({ id: ytIdOf(c), title: c.title, artists: artistName(c), album: c.album, duration_ms: c.duration, thumb: c.artwork_url });
function ytMatchesLoad() {
  if (!ytMatches) { try { ytMatches = JSON.parse(localStorage.getItem(YT_MATCH_KEY) || "{}"); } catch (e) { ytMatches = {}; } }
  return ytMatches;
}
function ytMatchesSave() { try { localStorage.setItem(YT_MATCH_KEY, JSON.stringify(ytMatches)); } catch (e) {} }

// The YouTube Music search for a track, best match first
async function ytCandidates(t, query) {
  const names = guessNames(t);
  const found = await ytSearch(query || (names.artist || artistName(t)) + " " + names.title);
  return found.slice(0, 10).map((c) => ({ c, score: ytMatchScore(t, c) })).sort((a, b) => b.score - a.score);
}

async function ytMatch(t) {
  if (srcOf(t) === "youtube") return t;
  const saved = ytMatchesLoad()[t.id];
  if (saved && saved.id) return ytTrack(saved);
  // (an old "none" without a date, or one more than a day old, is looked for again)
  if (saved && saved.none && Date.now() - (saved.at || 0) < YT_NO_MATCH_TTL) { ytLastMiss = { id: t.id, closest: saved.closest || "" }; return null; }
  let ranked;
  try { ranked = await ytCandidates(t); } catch (e) { return null; }   // couldn't search: try again next time
  const top = ranked[0];
  const ok = top && top.score >= 0.8 ? top.c : null;
  const closest = top ? '"' + top.c.title + '" by ' + artistName(top.c) : "";
  ytMatches[t.id] = ok ? ytSnap(ok) : { none: true, at: Date.now(), closest };
  ytMatchesSave();
  if (!ok) ytLastMiss = { id: t.id, closest };
  return ok;
}

// Track preferences > Plays from YouTube Music: your choice wins over the automatic match
function ytChooseMatch(t, c) {
  ytMatchesLoad();
  ytMatches[t.id] = Object.assign(ytSnap(c), { chosen: true });
  ytMatchesSave();
  delete t.ytVia;
}

// Why a track that should come from YouTube Music didn't
function ytMissNote(t) {
  if (!ytOn()) return "YouTube Music is off or not set up";
  const miss = ytLastMiss && ytLastMiss.id === t.id ? ytLastMiss : null;
  return "YouTube Music had no close match" + (miss && miss.closest ? " (the closest was " + miss.closest + ")" : "") +
    ". Right-click > Track preferences to choose the version";
}

// Plays a track by its YouTube Music match; false if there's none (or YouTube Music is off)
async function playViaYouTube(t, mine, note) {
  if (!ytOn()) return false;
  const m = await ytMatch(t).catch(() => null);
  if (!m || mine !== streamToken) return false;
  t.ytVia = m.ytId;   // loudness and silences are measured on what's actually playing
  const url = await ytPlay(m);
  if (mine !== streamToken) return true;
  await tryStream(url, false, null);
  if (mine !== streamToken) return true;
  say(note);
  loudAnalyse(t).then(() => { if (curTrack() === t) loudApply(t, false); });
  return true;
}

// ---------- likes, kept on this PC ----------
const YT_LIKES_KEY = "sc.ytLikes";
let ytLikes = [];   // newest first

function ytLikesLoad() {
  try { ytLikes = JSON.parse(localStorage.getItem(YT_LIKES_KEY) || "[]"); } catch (e) { ytLikes = []; }
  ytLikes.forEach((t) => likeState.set(t.id, true));
}

function ytSetLiked(t, on) {
  ytLikes = ytLikes.filter((x) => x.id !== t.id);
  if (on) ytLikes.unshift(xplSnap(t));
  try { localStorage.setItem(YT_LIKES_KEY, JSON.stringify(ytLikes)); } catch (e) {}
  // with your YouTube account connected, the like is made there too (the one here stays either way)
  if (ytAcc.connected) ytRate(t, on, ytIdOf(t));
}

// ---------- your YouTube account (optional) ----------
// Connected (Settings > Integrations), liking a YouTube track likes it on YouTube and YouTube Music;
// with "Like Other Tracks There Too", liking any other track likes the same song there as well. The
// video a track was matched to is remembered, so unliking takes back the same like.
const YT_MIRROR_KEY = "sc.ytMirror";
const ytAcc = { connected: false, name: "", configured: false, busy: false };
let ytMirrorIds = {};
try { ytMirrorIds = JSON.parse(localStorage.getItem(YT_MIRROR_KEY) || "{}"); } catch (e) { ytMirrorIds = {}; }

async function ytRate(t, on, videoId) {
  const n = guessNames(t);
  try {
    const id = await invoke(on ? "like_youtube_track" : "unlike_youtube_track", { videoId: videoId || ytMirrorIds[t.id] || null, title: n.title || t.title || "", artist: n.artist || artistName(t) || "" });
    if (srcOf(t) !== "youtube") {
      if (on) ytMirrorIds[t.id] = id; else delete ytMirrorIds[t.id];
      try { localStorage.setItem(YT_MIRROR_KEY, JSON.stringify(ytMirrorIds)); } catch (e) {}
    }
  } catch (e) {
    say((on ? "Liked here, but not on YouTube: " : "Unliked here, but not on YouTube: ") + e);
    if (/connect your youtube account/i.test(String(e))) ytAccRefresh();
  }
}

// Liking a track from another platform: the same song on YouTube too, if you've asked for that
function ytMirrorLike(t, on) {
  if (ytAcc.connected && prefs.ytMirror && srcOf(t) !== "youtube" && srcOf(t) !== "local") ytRate(t, on, null);
}

async function ytAccRefresh() {
  try { Object.assign(ytAcc, await invoke("youtube_status")); } catch (e) { ytAcc.connected = false; }
  ytAccPaint();
}

function ytAccPaint() {
  $("ytAccOn").checked = ytAcc.connected;
  $("ytAccOn").disabled = ytAcc.busy;
  $("ytMirrorRow").hidden = !ytAcc.connected;
  $("ytMirror").checked = !!prefs.ytMirror;
  $("ytAccNote").textContent = ytAcc.busy ? "Finish signing in in your browser, then come back here..."
    : ytAcc.connected ? "Connected" + (ytAcc.name ? " as " + ytAcc.name : "") + ". Likes here are likes on YouTube and YouTube Music too."
    : ytAcc.configured === false ? "Optional. Needs youtube_client_id and youtube_client_secret in config.json (see the README)."
    : "Optional. Connected, a like here is a like on YouTube and YouTube Music too.";
}

function ytAccInit() {
  $("ytAccOn").addEventListener("change", async () => {
    if ($("ytAccOn").checked) {
      ytAcc.busy = true;
      ytAccPaint();
      try {
        Object.assign(ytAcc, await invoke("youtube_connect"));
        say("YouTube account connected" + (ytAcc.name ? " (" + ytAcc.name + ")" : "") + ".");
      } catch (e) {
        say("Couldn't connect your YouTube account: " + e);
      }
      ytAcc.busy = false;
      await ytAccRefresh();
    } else {
      if (!confirm("Disconnect your YouTube account? Likes stay on YouTube; new ones won't be sent there.")) { ytAccPaint(); return; }
      try { await invoke("youtube_disconnect"); } catch (e) { say(String(e)); }
      await ytAccRefresh();
      say("YouTube account disconnected.");
    }
  });
  $("ytMirror").addEventListener("change", () => { prefs.ytMirror = $("ytMirror").checked; savePrefs(); });
  ytAccRefresh();
}

const ytLikedTracks = () => ytLikes.map((x) => Object.assign({}, x, { user: Object.assign({}, x.user) }));

// ---------- playing ----------
// Finds the audio first (so a missing yt-dlp gets a clear message), then plays it through the local server
async function ytPlay(t) {
  const vid = ytIdOf(t);
  if (!vid) throw new Error("this YouTube track has no video id");
  await invoke("yt_prepare", { id: vid });
  if (!localInfo) localInfo = await invoke("local_server");
  return localUrl("yt", vid);
}

// The next track's audio is looked up while this one plays, so it starts straight away
function ytPrepareNext() {
  const next = queue[qi + 1];
  if (next && srcOf(next) === "youtube" && ytAvailable()) invoke("yt_prepare", { id: ytIdOf(next) }).catch(() => {});
}

// ---------- autoplay and Discover ----------
async function apYouTube(seed) {
  const song = await ytFind(seed);
  return song ? shuffleInPlace(await ytRadio(song.ytId)) : [];
}

// A genre on YouTube Music: its most played songs
async function ytDiscover(src) {
  const g = String(src).replace(/^genre:/, "");
  return { list: await ytSearch(g + " music") };
}

// ---------- settings: setting up, switching off, removing ----------
// With YouTube Music switched on (the default), yt-dlp and Deno are set up by themselves the first
// time; switched off, YouTube isn't used anywhere. Remove takes them (and their caches) off this PC.
let ytSetupTimer = null;
let ytAutoTried = false;   // set up by itself once per run; after a failure it waits for Try again

const ytWanted = () => prefs.ytOn !== false;
const mbOf = (n) => (n / 1048576).toFixed(0);

function ytPaint() {
  const note = $("ytNote");
  const setupBtn = $("ytSetup");
  const removeBtn = $("ytRemove");
  $("ytOn").checked = ytWanted();
  const setup = ytInfo.setup || {};
  removeBtn.hidden = !ytInfo.checked || setup.busy || !ytInfo.diskUse;
  removeBtn.title = ytInfo.diskUse ? "Remove yt-dlp, Deno and their caches (" + mbOf(ytInfo.diskUse) + " MB)" : "";
  setupBtn.hidden = true;
  if (!ytInfo.checked) { note.textContent = "Checking..."; return; }
  if (setup.busy) {
    note.textContent = "Setting up: " + setup.step.toLowerCase() + (setup.total ? ", " + mbOf(setup.done) + " of " + mbOf(setup.total) + " MB" : "") + "...";
    return;
  }
  if (!ytWanted()) {
    note.textContent = "Off" + (ytInfo.diskUse ? ". yt-dlp and Deno are still on this PC (" + mbOf(ytInfo.diskUse) + " MB): Remove takes them off." : ". Switch on to set it up (about 58 MB).");
    return;
  }
  if (ytAvailable()) {
    note.textContent = ytInfo.builtIn
      ? "Ready (" + mbOf(ytInfo.diskUse) + " MB on this PC). Nothing to sign in to: likes are kept on this PC."
      : "Ready, with your own yt-dlp " + ytInfo.ytdlp + ". Nothing to sign in to: likes are kept on this PC.";
    return;
  }
  if (setup.error) {
    note.textContent = "Couldn't set up: " + setup.error;
    setupBtn.textContent = "Try again";
    setupBtn.hidden = false;
  } else {
    note.textContent = "Needs yt-dlp and Deno to play (about 58 MB from GitHub).";
    setupBtn.textContent = "Set up";
    setupBtn.hidden = false;
  }
}

async function ytStartSetup() {
  await invoke("yt_setup").catch((e) => say(String(e)));
  say("Setting up YouTube Music (about 58 MB)...");
  ytCheck();
}

async function ytCheck() {
  try { ytInfo = Object.assign(await invoke("yt_status"), { checked: true }); } catch (e) { ytInfo = { ytdlp: null, deno: false, node: false, checked: true }; }
  const setup = ytInfo.setup || {};
  ytPaint();
  syncSpotifySettings();   // a free Spotify account plays through YouTube Music
  renderPlatChips();
  clearTimeout(ytSetupTimer);
  if (setup.busy) { ytSetupTimer = setTimeout(ytCheck, 700); return; }   // follow the download
  if (setup.step === "Done" && ytAvailable()) {
    ytInfo.setup = {};
    say("YouTube Music is ready.");
    return;
  }
  if (setup.error) { say("YouTube Music couldn't be set up: " + setup.error); return; }
  // the first time (or after Remove and switching on again): set up without being asked
  if (ytWanted() && !ytAvailable() && !ytAutoTried) {
    ytAutoTried = true;
    ytStartSetup();
  }
}

async function ytRemove() {
  const mb = mbOf(ytInfo.diskUse || 0);
  if (!confirm("Remove YouTube Music's yt-dlp and Deno from this PC (" + mb + " MB, caches included)? YouTube Music will be switched off. Your YouTube likes are kept, in case you set it up again.")) return;
  try {
    const freed = await invoke("yt_remove");
    prefs.ytOn = false;
    savePrefs();
    ytAutoTried = true;   // switched off now: nothing to set up
    await setPlatformOn("youtube", false);
    say("Removed yt-dlp and Deno (" + mbOf(freed) + " MB freed).");
  } catch (e) {
    say("Couldn't remove them: " + e);
  }
  ytCheck();
}

function ytInit() {
  ytLikesLoad();
  ytPaint();
  $("ytOn").addEventListener("change", async () => {
    await setPlatformOn("youtube", $("ytOn").checked);
    ytAutoTried = false;   // switched on: set up if it isn't yet
    ytCheck();
  });
  $("ytSetup").addEventListener("click", ytStartSetup);
  $("ytRemove").addEventListener("click", ytRemove);
  ytCheck();
}
