// SponsorBlock (Settings > Playback): what plays from YouTube skips the non-music parts of music
// videos (a skit before the song, an interview after it) that people have marked there. The Rust
// side asks by a hash prefix, so SponsorBlock doesn't learn what's playing (crates/core/src/sponsorblock.rs).

const sbOn = () => prefs.sbOn !== false;
const sbSegs = new Map();   // video id -> [[start, end], ...] in seconds, or a promise while it's asked
let sbSkipped = "";         // the last segment skipped ("id:start"), so a seek back into it isn't fought

// The YouTube video a track is playing from (itself, or its match)
const sbVideoOf = (t) => (t && (srcOf(t) === "youtube" ? ytIdOf(t) : t.ytVia)) || "";

function sbLoad(id) {
  if (!id || sbSegs.has(id)) return;
  sbSegs.set(id, invoke("sb_segments", { videoId: id }).then((s) => { sbSegs.set(id, s || []); }).catch(() => { sbSegs.delete(id); }));
}

function sbTick() {
  const t = curTrack();
  if (!sbOn() || !t || !nowPlaying() || switching) return;
  // listening along: the host's app skips them (or doesn't), and this follows it
  if (typeof together !== "undefined" && together.role === "listener") return;
  const id = sbVideoOf(t);
  if (!id) return;
  const segs = sbSegs.get(id);
  if (segs === undefined) { sbLoad(id); return; }
  if (!Array.isArray(segs)) return;
  const pos = curPosMs() / 1000;
  const seg = segs.find(([a, b]) => pos >= a && pos < b - 0.5);
  if (!seg) return;
  const key = id + ":" + seg[0];
  if (sbSkipped === key && pos > seg[0] + 1.5) return;   // you went back into it yourself
  sbSkipped = key;
  const secs = Math.round(seg[1] - Math.max(pos, seg[0]));
  // a part that runs to the end: the song's over
  if (curDurMs() && seg[1] * 1000 >= curDurMs() - 1500) {
    say("Skipped the last " + secs + " s (not music, marked on SponsorBlock).");
    onEnded();
    return;
  }
  fadeJump(450, () => {
    audio.currentTime = seg[1];
    if (typeof hostPush === "function") hostPush(true);   // (hosting: listeners jump with it)
  });
  say("Skipped " + secs + " s that isn't music (marked on SponsorBlock).");
}

function sbInit() {
  $("sbOn").checked = sbOn();
  $("sbOn").addEventListener("change", () => { prefs.sbOn = $("sbOn").checked; savePrefs(); });
  setInterval(sbTick, 250);
}
