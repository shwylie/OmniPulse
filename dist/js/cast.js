// Casting (the output button in the player bar): plays on a Chromecast, a Nest speaker or display, or a
// TV or speaker with Cast built in, on the same network.
//
// The app stays the player. Its sound, after the EQ, loudness match, limiter and volume, goes to the
// device as one live stream (crates/core/src/cast.rs), so every source, trims and the queue work as
// they do here; the PC's own output is muted meanwhile. A device with a screen shows Google's own
// player, with the title, artist and cover.
// There's a short delay (the device's buffer) between pressing play here and hearing it there.
// While casting, the volume slider is the device's own (it starts at the level the device was at).

const cast = { on: false, device: null, node: null, pre: null, sink: null, devPaused: false, starting: false, level: null, volAt: 0 };

async function castDevices() {
  try { return await invoke("cast_devices"); } catch (e) { say(String(e)); return []; }
}

// The sound, tapped before this PC's volume (the device has its own), muted with the mute button
async function castTapStart() {
  if (!normEnsure()) throw new Error("the audio chain isn't running");
  const ctx = norm.ctx;
  if (!cast.node) {
    await ctx.audioWorklet.addModule("js/cast-worklet.js");
    cast.node = new AudioWorkletNode(ctx, "cast-tap", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2] });
    cast.sink = ctx.createGain();
    cast.sink.gain.value = 0;
    cast.node.connect(cast.sink);
    cast.pre = ctx.createGain();
    cast.pre.connect(cast.node);
    cast.sink.connect(ctx.destination);
    const rate = String(ctx.sampleRate);
    cast.gate = await pcmRoute(cast.node, "cast", rate, (d) => invoke("cast_pcm", new Uint8Array(d), { headers: { "x-rate": rate } }).catch(() => {}));
  }
  cast.pre.gain.value = audio.muted ? 0 : 1;
  try { norm.limiter.connect(cast.pre); } catch (e) {}
  if (ctx.state === "suspended") ctx.resume().catch(() => {});
}

function castTapStop() {
  if (cast.pre) { try { norm.limiter.disconnect(cast.pre); } catch (e) {} }
}

function castMute(muted) {
  if (cast.pre) cast.pre.gain.setTargetAtTime(muted ? 0 : 1, norm.ctx.currentTime, 0.02);
}

function castMeta() {
  const s = widgetState();
  const t = curTrack();
  return Object.assign(s, { album: t ? albumName(t) : "", art: s.artwork });
}

const albumName = (t) => (t && (t.album || (t.publisher_metadata && t.publisher_metadata.album_title))) || "";

async function castStart(d) {
  if (cast.starting) return;
  cast.starting = true;
  say("Connecting to " + d.name + "...");
  try {
    await castTapStart();
    cast.gate(true);
    const ch = new window.__TAURI__.core.Channel();
    ch.onmessage = castEvent;
    cast.device = await invoke("cast_start", { id: d.id, meta: castMeta(), events: ch });
    cast.on = true;
    cast.devPaused = false;
    out2MainMute(true);
    castPaint();
    say("Casting to " + d.name + ". It plays there a moment after it does here.");
  } catch (e) {
    castTapStop();
    cast.on = false;
    if (cast.gate) cast.gate(false);
    say("Couldn't cast to " + d.name + ": " + (e && e.message ? e.message : e));
  } finally {
    cast.starting = false;
  }
}

function castEnd(note) {
  if (!cast.on) return;
  cast.on = false;
  if (cast.gate) cast.gate(false);
  castTapStop();
  out2MainMute(false);
  cast.level = null;
  const name = cast.device ? cast.device.name : "the device";
  cast.device = null;
  castPaint();
  say(note || "Stopped casting to " + name + ".");
}

function castStop() {
  invoke("cast_stop").catch(() => {});
  castEnd();
}

// What the device says
function castEvent(m) {
  if (!m) return;
  // the device's level comes as it connects, before casting is on: the slider starts there
  if (m.type === "volume") { cast.level = m.muted ? 0 : m.level; castVolPaint(); return; }
  if (!cast.on) return;
  if (m.type === "ended") castEnd(m.message ? "Casting stopped: " + m.message : "");
  else if (m.type === "error") say(m.message);
  else if (m.type === "media") {
    // paused or played with the device's own controls (Google's player)
    if (m.state === "PAUSED" && nowPlaying()) { cast.devPaused = true; toggle(); }
    else if (m.state === "PAUSED") cast.devPaused = true;
    else if (m.state === "PLAYING" && cast.devPaused) {
      cast.devPaused = false;
      invoke("cast_reload").catch(() => {});   // what it had buffered is old
      if (!nowPlaying()) toggle();
    } else if (m.state === "IDLE" && m.idleReason === "ERROR") say((cast.device ? cast.device.name : "The device") + " couldn't play the stream.");
  }
}

// Track changes and play/pause, from trackStarted and widgetPush
function castTrack() {
  if (!cast.on) return;
  invoke("cast_track", { meta: castMeta() }).catch(() => {});
}

// ---------- the device's volume ----------
// It takes the volume slider's place while casting, even when the slider is hidden for playing here
function castVolPaint() {
  const wrap = document.querySelector(".volWrap");
  wrap.hidden = !!prefs.hideVolume && !cast.on;
  $("vol").hidden = cast.on || !!prefs.hideVolume;
  $("castVolRow").hidden = !cast.on;
  $("mute").title = cast.on && cast.device ? "Mute " + cast.device.name : "Mute";
  // the device's own level, unless it's being dragged here
  if (cast.level != null && Date.now() - cast.volAt > 1500) $("castVol").value = String(Math.round(cast.level * 100));
}

let castVolSent = 0;
function castVolInput() {
  cast.level = Number($("castVol").value) / 100;
  cast.volAt = Date.now();
  const send = () => { castVolSent = Date.now(); invoke("cast_volume", { level: cast.level }).catch(() => {}); };
  clearTimeout(cast.volTimer);
  if (Date.now() - castVolSent > 120) send();
  else cast.volTimer = setTimeout(send, 130);
}

function castPaint() {
  const b = $("outputBtn");
  b.classList.toggle("on", cast.on);
  b.title = cast.on && cast.device ? "Casting to " + cast.device.name : "Output device and casting";
  castVolPaint();
}

// The output button's menu: where the sound plays, this PC's devices first, then Cast devices
let castSearching = false;

async function outputMenu() {
  const r = $("outputBtn").getBoundingClientRect();
  const at = () => showMenuAt(r.left, r.top - 4, items);
  let outs = [];
  try { outs = (await navigator.mediaDevices.enumerateDevices()).filter((d) => d.kind === "audiooutput"); } catch (e) {}
  const items = ["This PC"];
  const cur = prefs.sink || "default";
  if (!outs.length) outs = [{ deviceId: "default", label: "System default" }];
  outs.forEach((d, i) => {
    const name = d.label || (d.deviceId === "default" ? "System default" : "Output " + (i + 1));
    items.push([name, () => outputPick(d.deviceId), !cast.on && d.deviceId === cur ? "menuOn" : ""]);
  });
  if (outs.some((d) => !d.label) && outs.length > 1) items.push(["Show device names", () => { $("namesBtn").click(); setTimeout(outputMenu, 600); }]);
  items.push("sep", "Chromecast");
  if (!castEnabled()) {
    items.push(["Turn on Chromecast", () => { castSetOn(true); castSearch(); }, "menuNew"]);
    items.push("sep", ["Sound settings...", () => openSettings("audioSection")]);
    at();
    return;
  }
  if (cast.on && cast.device) items.push([cast.device.name, () => {}, "menuOn"], ["Stop casting", castStop]);
  const list = (await castDevices()).filter((d) => !cast.device || d.id !== cast.device.id);
  list.forEach((d) => items.push([d.name + (d.group ? " (group)" : d.display ? " (display)" : ""), () => castStart(d)]));
  if (castSearching) items.push("Searching for Cast devices...");
  else {
    if (!list.length && !cast.on) items.push("No Cast devices found. They need to be on the same network as this PC, and Windows may ask to let OmniPulse through its firewall.");
    items.push(["Search for Cast devices", castSearch, "menuNew"]);
  }
  items.push("sep", ["Sound settings...", () => openSettings("audioSection")]);
  at();
}

// Looks again for a few seconds (devices answer within a second or two), keeping the menu open
async function castSearch() {
  castSearching = true;
  await outputMenu();
  await castDevices();
  await new Promise((ok) => setTimeout(ok, 2500));
  castSearching = false;
  await outputMenu();
}

async function outputPick(id) {
  if (cast.on) castStop();
  await setOutput(id);
  if ($("outputSel")) $("outputSel").value = id;
  if ($("outputNote").textContent) say($("outputNote").textContent);
}

const castEnabled = () => !!prefs.castOn;

function castOnPaint() {
  $("castOn").checked = castEnabled();
}

function castSetOn(on) {
  prefs.castOn = on;
  savePrefs();
  castOnPaint();
  if (on) invoke("cast_devices").catch(() => {});
  else if (cast.on) castStop();
}

function castInit() {
  castOnPaint();
  $("castOn").addEventListener("change", () => castSetOn($("castOn").checked));
  $("castVol").addEventListener("input", castVolInput);
  // Keep Playing on This PC and its delay are gone
  if ("castLocal" in prefs || "castLocalDelay" in prefs || "castLocalAuto" in prefs) {
    delete prefs.castLocal; delete prefs.castLocalDelay; delete prefs.castLocalAuto;
    savePrefs();
  }
  castPaint();
  // start listening for devices early, so the menu has them
  if (castEnabled()) setTimeout(() => invoke("cast_devices").catch(() => {}), 3000);
}
