// YouTube Music, without signing in: search and radio from YouTube Music itself (crates/core/src/youtube.rs),
// audio through yt-dlp, played via the local server's /yt (which fetches it in pieces). Likes are kept on
// this PC, like favourites, so they show in your Library and seed autoplay and Rediscover.

let ytInfo = { ytdlp: null, deno: false, node: false, checked: false };
const ytAvailable = () => !!ytInfo.ytdlp && (ytInfo.deno || ytInfo.node);

function ytTrack(x) {
  return tidyTrack({
    source: "youtube",
    id: "yt:" + x.id,
    ytId: x.id,
    title: x.title,
    user: { username: x.artists || "Unknown artist" },
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
}

const ytLikedTracks = () => ytLikes.map((x) => Object.assign({}, x, { user: Object.assign({}, x.user) }));

// ---------- playing ----------
// Finds the audio first (so a missing yt-dlp gets a clear message), then plays it through the local server
async function ytPlay(t) {
  await invoke("yt_prepare", { id: t.ytId });
  if (!localInfo) localInfo = await invoke("local_server");
  return localUrl("yt", t.ytId);
}

// The next track's audio is looked up while this one plays, so it starts straight away
function ytPrepareNext() {
  const next = queue[qi + 1];
  if (next && srcOf(next) === "youtube" && ytAvailable()) invoke("yt_prepare", { id: next.ytId }).catch(() => {});
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

// ---------- settings: setting up yt-dlp and Deno ----------
let ytSetupTimer = null;

function ytPaint() {
  const note = $("ytNote");
  const btn = $("ytSetup");
  $("ytOn").checked = prefs.ytOn !== false;
  if (!ytInfo.checked) { note.textContent = "Checking..."; btn.hidden = true; return; }
  const setup = ytInfo.setup || {};
  const mb = (n) => (n / 1048576).toFixed(0);
  if (setup.busy) {
    note.textContent = setup.step + (setup.total ? ": " + mb(setup.done) + " of " + mb(setup.total) + " MB" : "") + "...";
    btn.hidden = true;
    return;
  }
  if (ytAvailable()) {
    note.textContent = "Ready" + (ytInfo.builtIn ? "" : ", with your own yt-dlp " + ytInfo.ytdlp) + ". Nothing to sign in to: likes are kept on this PC.";
    btn.hidden = true;
    return;
  }
  note.textContent = setup.error
    ? "Couldn't set up: " + setup.error
    : "Needs yt-dlp and Deno to play (about 58 MB, downloaded from GitHub). Set up gets them.";
  btn.textContent = setup.error ? "Try again" : "Set up";
  btn.hidden = false;
}

async function ytCheck() {
  try { ytInfo = Object.assign(await invoke("yt_status"), { checked: true }); } catch (e) { ytInfo = { ytdlp: null, deno: false, node: false, checked: true }; }
  ytPaint();
  renderPlatChips();
  const busy = ytInfo.setup && ytInfo.setup.busy;
  clearTimeout(ytSetupTimer);
  if (busy) ytSetupTimer = setTimeout(ytCheck, 700);   // follow the download
  else if (ytAvailable() && ytInfo.setup && ytInfo.setup.step === "Done") say("YouTube Music is ready.");
}

function ytInit() {
  ytLikesLoad();
  ytPaint();
  $("ytOn").addEventListener("change", () => setPlatformOn("youtube", $("ytOn").checked));
  $("ytSetup").addEventListener("click", async () => {
    await invoke("yt_setup").catch((e) => say(String(e)));
    ytCheck();
  });
  ytCheck();
}
