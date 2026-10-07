// Casting (the cast button in the player bar): plays on a Chromecast, a Nest speaker or display, or a
// TV or speaker with Cast built in, on the same network.
//
// The app stays the player. Its sound, after the EQ, loudness match, limiter and volume, goes to the
// device as one live stream (crates/core/src/cast.rs), so every source, trims and the queue work as
// they do here; the PC's own output is muted meanwhile. A device with a screen shows what's playing:
// OmniPulse's own display (cast-receiver/, once it's set up in Settings > Sound > Casting) with play,
// pause and skip buttons, or otherwise Google's player with the title, artist and cover.
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
    const app = d.display && prefs.castApp ? prefs.castApp : null;
    cast.device = await invoke("cast_start", { id: d.id, app, meta: castMeta(), events: ch });
    cast.on = true;
    cast.devPaused = false;
    out2MainMute(true);
    castPaint();
    castPush();
    say("Casting to " + d.name + ". It plays there a moment after it does here.");
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
  out2MainMute(false);
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
  } else if (m.type === "app" && m.message) {
    const a = m.message;
    if (a.type === "hello") { invoke("cast_track", { meta: castMeta() }).catch(() => {}); castPush(); }
    else if (a.type === "command") {
      if (a.action === "toggle") toggle();
      else if (a.action === "next") step(1);
      else if (a.action === "prev") step(-1);
    }
  }
}

// Track changes and play/pause, from trackStarted and widgetPush
function castTrack() {
  if (cast.on) invoke("cast_track", { meta: castMeta() }).catch(() => {});
}
function castPush() {
  if (cast.on) invoke("cast_state", { state: widgetState() }).catch(() => {});
}

function castPaint() {
  const b = $("castBtn");
  b.classList.toggle("on", cast.on);
  b.title = cast.on && cast.device ? "Casting to " + cast.device.name : "Cast to a device";
}

async function castMenu() {
  const r = $("castBtn").getBoundingClientRect();
  const items = [cast.on && cast.device ? "Casting to " + cast.device.name : "Cast to a device on your network"];
  if (cast.on) items.push(["Stop casting", castStop], "sep");
  showMenuAt(r.left, r.top - 4, items.concat(["Looking for devices..."]));
  // devices answer within a second or two of the first look
  let list = await castDevices();
  if (!list.length) { await new Promise((ok) => setTimeout(ok, 1800)); list = await castDevices(); }
  if ($("plMenu").hidden) return;
  const devs = list.filter((d) => !cast.device || d.id !== cast.device.id).map((d) => [
    d.name + (d.group ? " (group)" : d.display ? " (display)" : ""), () => castStart(d),
  ]);
  if (!devs.length) devs.push(list.length ? "No other devices found." : "No devices found. They need to be on the same network as this PC, and Windows may ask to let OmniPulse through its firewall.");
  devs.push("sep", ["Refresh", () => castMenu()]);
  showMenuAt(r.left, r.top - 4, items.concat(devs));
}

function castInit() {
  $("castBtn").addEventListener("click", (e) => { e.stopPropagation(); castMenu(); });
  $("castApp").value = prefs.castApp || "";
  $("castApp").addEventListener("change", () => {
    prefs.castApp = $("castApp").value.trim().toUpperCase().replace(/[^0-9A-F]/g, "");
    $("castApp").value = prefs.castApp;
    savePrefs();
  });
  castPaint();
  // start listening for devices early, so the menu has them
  setTimeout(() => invoke("cast_devices").catch(() => {}), 3000);
}
