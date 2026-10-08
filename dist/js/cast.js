// Casting (the output button in the player bar): plays on a Chromecast, a Nest speaker or display, or a
// TV or speaker with Cast built in, on the same network.
//
// The app stays the player. Its sound, after the EQ, loudness match, limiter and volume, goes to the
// device as one live stream (crates/core/src/cast.rs), so every source, trims and the queue work as
// they do here; the PC's own output is muted meanwhile (unless Keep Playing on This PC is on). A device
// with a screen shows Google's own player, with the title, artist and cover.
// There's a short delay (the device's buffer) between pressing play here and hearing it there.

const cast = { on: false, device: null, node: null, sink: null, devPaused: false, starting: false };

async function castDevices() {
  try { return await invoke("cast_devices"); } catch (e) { say(String(e)); return []; }
}

// The sound, tapped after the volume
async function castTapStart() {
  if (!normEnsure()) throw new Error("the audio chain isn't running");
  const ctx = norm.ctx;
  if (!cast.node) {
    await ctx.audioWorklet.addModule("js/cast-worklet.js");
    cast.node = new AudioWorkletNode(ctx, "cast-tap", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2] });
    cast.sink = ctx.createGain();
    cast.sink.gain.value = 0;
    cast.node.connect(cast.sink);
    cast.sink.connect(ctx.destination);
    const rate = String(ctx.sampleRate);
    cast.node.port.onmessage = (e) => {
      if (cast.on || cast.starting) invoke("cast_pcm", new Uint8Array(e.data), { headers: { "x-rate": rate } }).catch(() => {});
    };
  }
  try { norm.master.connect(cast.node); } catch (e) {}
  if (ctx.state === "suspended") ctx.resume().catch(() => {});
}

function castTapStop() {
  if (cast.node) { try { norm.master.disconnect(cast.node); } catch (e) {} }
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
    const ch = new window.__TAURI__.core.Channel();
    ch.onmessage = castEvent;
    cast.device = await invoke("cast_start", { id: d.id, meta: castMeta(), events: ch });
    cast.on = true;
    cast.devPaused = false;
    castLocalApply();
    castPaint();
    say("Casting to " + d.name + "." + (prefs.castLocal ? " Playing here too." : " It plays there a moment after it does here."));
  } catch (e) {
    castTapStop();
    cast.on = false;
    say("Couldn't cast to " + d.name + ": " + (e && e.message ? e.message : e));
  } finally {
    cast.starting = false;
  }
}

function castEnd(note) {
  if (!cast.on) return;
  cast.on = false;
  castTapStop();
  castLocalApply();
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
  if (!m || !cast.on) return;
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
  if (cast.on) invoke("cast_track", { meta: castMeta() }).catch(() => {});
}


function castPaint() {
  const b = $("outputBtn");
  b.classList.toggle("on", cast.on);
  b.title = cast.on && cast.device ? "Casting to " + cast.device.name : "Output device and casting";
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

// Playing on this PC as well while casting, held back to line up with the device (its buffer)
const castLocalDelay = () => (cast.on && prefs.castLocal ? Math.max(0, Math.min(5000, Number(prefs.castLocalDelay) || 0)) : 0);

function castLocalApply() {
  out2MainMute(cast.on && !prefs.castLocal);
  out2MainDelay();
}

function castLocalPaint() {
  $("castLocal").checked = !!prefs.castLocal;
  $("castLocalRow").hidden = !prefs.castLocal;
  $("castLocalDelay").value = String(Number(prefs.castLocalDelay) || 0);
}

const castEnabled = () => !!prefs.castOn;

function castOnPaint() {
  $("castOn").checked = castEnabled();
  $("castBody").hidden = !castEnabled();
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
  castLocalPaint();
  $("castLocal").addEventListener("change", () => { prefs.castLocal = $("castLocal").checked; savePrefs(); castLocalPaint(); castLocalApply(); });
  $("castLocalDelay").addEventListener("change", () => {
    const v = Math.round(Number($("castLocalDelay").value));
    prefs.castLocalDelay = Number.isFinite(v) ? Math.max(0, Math.min(5000, v)) : 0;
    savePrefs();
    castLocalPaint();
    castLocalApply();
  });
  castPaint();
  // start listening for devices early, so the menu has them
  if (castEnabled()) setTimeout(() => invoke("cast_devices").catch(() => {}), 3000);
}
