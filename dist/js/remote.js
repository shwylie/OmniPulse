// Remote control (for the Stream Deck plugin in streamdeck/): the Rust side (src-tauri/src/remote.rs)
// listens on 127.0.0.1 and hands each button press to window.__remote, which answers with the state.

function remoteState() {
  const t = curTrack();
  return {
    ok: true,
    playing: nowPlaying(),
    title: t ? t.title || "" : "",
    artist: t ? artistName(t) : "",
    shuffle: shuffleOn,
    repeat: repeatMode,
    liked: isLiked(t),
    canLike: canLike(t),
    favourite: favOn() && isFav(t),
    setsAllowed: !!prefs.queueSets,
    requests: reqPendingList().length,
  };
}

const REMOTE_ACTIONS = {
  play: () => { if (!nowPlaying()) toggle(); },
  pause: () => { if (nowPlaying()) toggle(); },
  toggle: () => toggle(),
  next: () => step(1),
  previous: () => step(-1),
  like: () => toggleLike(curTrack()),
  favourite: () => { if (favOn()) toggleFav(curTrack()); },
  shuffle: () => toggleShuffle(),
  repeat: () => cycleRepeat(),
  "clear-requests": () => clearRequests(),
  "clear-queue": () => {
    queue = qi >= 0 ? [queue[qi]] : [];
    qi = queue.length ? 0 : -1;
    renderQueue();
  },
  // sets & podcasts in (or out of) both the queue and autoplay
  sets: () => {
    const on = !prefs.queueSets;
    prefs.queueSets = on;
    savePrefs();
    $("queueSets").checked = on;
    autoplayFailedFor = null;
    say(on ? "Sets & Podcasts are back in the queue and autoplay." : "Sets & Podcasts are out of the queue and autoplay.");
  },
  state: () => {},
};

window.__remote = async (id, action) => {
  let state;
  try {
    await REMOTE_ACTIONS[action]();
    if (action !== "state") await new Promise((r) => setTimeout(r, 150));   // let toggles settle
    state = remoteState();
  } catch (e) {
    state = Object.assign(remoteState(), { ok: false, message: String(e && e.message ? e.message : e) });
  }
  invoke("remote_result", { id, state }).catch(() => {});
};

async function remoteApply() {
  const on = !!prefs.remoteOn;
  $("remoteOn").checked = on;
  $("remoteSection").hidden = !on;
  if (!on) { invoke("remote_stop").catch(() => {}); return; }
  if (!prefs.remoteKey) { prefs.remoteKey = reqKeyNew() + reqKeyNew(); savePrefs(); }
  const port = Number(prefs.remotePort) || 7880;
  $("remotePort").value = port;
  try {
    const url = await invoke("remote_start", { port, key: prefs.remoteKey });
    $("remoteUrl").value = url + "control?key=" + prefs.remoteKey + "&action=toggle";
    $("remoteNote").textContent = "";
  } catch (e) {
    $("remoteNote").textContent = String(e);
  }
}

function remoteInit() {
  $("remoteOn").addEventListener("change", () => { prefs.remoteOn = $("remoteOn").checked; savePrefs(); remoteApply(); });
  $("remotePort").addEventListener("change", () => {
    const p = Math.round(Number($("remotePort").value));
    if (p >= 1024 && p <= 65535) { prefs.remotePort = p; savePrefs(); remoteApply(); }
    else $("remotePort").value = prefs.remotePort || 7880;
  });
  $("remoteNewKey").addEventListener("click", () => {
    prefs.remoteKey = reqKeyNew() + reqKeyNew();
    savePrefs();
    remoteApply();
    say("New remote key made. The Stream Deck plugin picks it up by itself.");
  });
  $("remoteCopy").addEventListener("click", async () => {
    try { await navigator.clipboard.writeText($("remoteUrl").value); $("remoteCopy").textContent = "Copied"; setTimeout(() => { $("remoteCopy").textContent = "Copy"; }, 1500); }
    catch (e) { $("remoteUrl").select(); }
  });
  remoteApply();
}
