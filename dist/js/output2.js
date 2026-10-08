// OmniPulse Offset (Settings > Streaming; its program is "Offset OmniPulse"): the same sound played a second time, as its own app
// called "Offset OmniPulse" (crates/core/src/omni_input.rs plays it on the default playback device),
// with its own volume and an offset to line the two up, like OBS's sync offset. Above 0 it's later;
// below 0 the main output is held back instead. In Elgato Wave Link, Windows' mixer or the like it
// shows apart from the app's main sound, so it can go to a stream mix only. No driver is involved.
//
//   ... limiter -> master (volume) -> main delay -> your output device
//          \-> input delay -> input volume -> a tap (js/cast-worklet.js) -> Offset OmniPulse

const OUT2_MAX_MS = 5000;
const out2 = { delay: null, gain: null, tap: null, sink: null, running: false, mainDelay: null, mainGain: null };

// The main path's own delay sits in the chain from the start (it's 0 unless the offset is negative)
// The main output, too, can need an <audio> on the device: the audio context can't open every device
// (some virtual ones refuse it with an AbortError), and an <audio> playing a stream of it can.
const mainOut = { dest: null, el: null };

async function mainSink(id) {
  const ctx = norm.ctx;
  if (!ctx || !out2.mainDelay) return;
  // played from OmniPulse.exe: that's what moves (js/mainout.js)
  if (nativeOut.on) {
    await invoke("main_out_start", { device: await sinkLabel(id) });
    setLevel(appVolume());
    return;
  }
  const want = id && id !== "default" ? id : "";
  let first = null;
  if (ctx.setSinkId) {
    try {
      if (ctx.sinkId !== want) await ctx.setSinkId(want);
      mainViaElement("");
      return;
    } catch (e) { first = e; }
  }
  if (!want) { if (first) throw first; return; }
  try {
    await mainViaElement(want);
  } catch (e) {
    mainViaElement("");
    throw first || e;
  }
}

async function mainViaElement(id) {
  const ctx = norm.ctx, d = out2.mainDelay;
  if (!id) {
    if (!mainOut.el) return;
    mainOut.el.pause();
    mainOut.el.srcObject = null;
    try { d.disconnect(mainOut.dest); } catch (e) {}
    d.connect(ctx.destination);
    mainOut.el = mainOut.dest = null;
    return;
  }
  if (!mainOut.el) {
    mainOut.dest = ctx.createMediaStreamDestination();
    mainOut.el = new Audio();
    mainOut.el.srcObject = mainOut.dest.stream;
  }
  if (!mainOut.el.setSinkId) throw new Error("this engine can't choose a device");
  if (mainOut.el.sinkId !== id) await mainOut.el.setSinkId(id);
  d.connect(mainOut.dest);
  try { d.disconnect(ctx.destination); } catch (e) {}
  await mainOut.el.play();
}

function out2Wire(ctx, master) {
  const mainDelay = ctx.createDelay(OUT2_MAX_MS / 1000 + 0.1);
  mainDelay.delayTime.value = 0;
  const mainGain = ctx.createGain();   // 0 while casting (js/cast.js): the sound plays on the device instead
  master.connect(mainGain);
  mainGain.connect(mainDelay);
  out2.mainGain = mainGain;
  mainDelay.connect(ctx.destination);
  out2.mainDelay = mainDelay;
}

// The PC's own delay: the OmniPulse input's offset when that's below 0, plus, while casting with this PC
// playing too, the delay that lines it up with the device
function out2MainDelay() {
  if (!out2.mainDelay) return;
  const offset = prefs.out2On ? Math.max(-OUT2_MAX_MS, Math.min(OUT2_MAX_MS, Number(prefs.out2Offset) || 0)) : 0;
  const ms = Math.min(OUT2_MAX_MS, offset < 0 ? -offset : 0);
  out2.mainDelay.delayTime.setValueAtTime(ms / 1000, norm.ctx.currentTime);
}

function out2MainMute(muted) {
  if (out2.mainGain) out2.mainGain.gain.setTargetAtTime(muted ? 0 : 1, norm.ctx.currentTime, 0.05);
}

const out2StartMuted = () => prefs.out2Muted !== false;

const OUT2_ROUTE = "Please route OmniPulse appropriately first to prevent playback echo.\n\nTurn on OmniPulse Offset?";
const OUT2_ROUTE_MUTED = "OmniPulse Offset starts muted in the Windows Sound Mixer (it's called Offset OmniPulse there). Route it (in Wave Link: the Stream mix only), then unmute it there.\n\nTurn it on?";

function out2Teardown() {
  if (out2.tap) { out2.tap.port.onmessage = null; try { out2.tap.disconnect(); } catch (e) {} }
  if (out2.delay) { try { norm.limiter.disconnect(out2.delay); } catch (e) {} }
  out2.delay = out2.gain = out2.tap = out2.sink = null;
  if (out2.running) invoke("omni_input_stop").catch(() => {});
  out2.running = false;
  if (out2.gate) out2.gate(false);
}

async function out2Apply() {
  const on = !!prefs.out2On;
  $("out2On").checked = on;
  $("out2Body").hidden = !on;
  const offset = Math.max(-OUT2_MAX_MS, Math.min(OUT2_MAX_MS, Number(prefs.out2Offset) || 0));
  $("out2Offset").value = String(offset);
  $("out2Vol").value = String(Math.round((prefs.out2Volume ?? 1) * 100));
  $("out2VolVal").textContent = $("out2Vol").value + "%";
  if (!normEnsure()) return;
  const ctx = norm.ctx;
  const now = ctx.currentTime;
  out2MainDelay();
  if (!on) { out2Teardown(); $("out2Note").textContent = ""; return; }
  try {
    if (!out2.delay) {
      await ctx.audioWorklet.addModule("js/cast-worklet.js");
      out2.delay = ctx.createDelay(OUT2_MAX_MS / 1000 + 0.1);
      out2.gain = ctx.createGain();
      // small pieces, so it stays close to the main sound
      out2.tap = new AudioWorkletNode(ctx, "cast-tap", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2], processorOptions: { ms: 20 } });
      out2.sink = ctx.createGain();
      out2.sink.gain.value = 0;
      norm.limiter.connect(out2.delay);
      out2.delay.connect(out2.gain);
      out2.gain.connect(out2.tap);
      out2.tap.connect(out2.sink);
      out2.sink.connect(ctx.destination);
      const rate = String(ctx.sampleRate);
      out2.gate = await pcmRoute(out2.tap, "input", rate, (d) => invoke("omni_input_pcm", new Uint8Array(d), { headers: { "x-rate": rate } }).catch(() => {}));
    }
    out2.delay.delayTime.setValueAtTime(offset > 0 ? offset / 1000 : 0, now);
    out2.gain.gain.setTargetAtTime(prefs.out2Volume ?? 1, now, 0.02);
    if (!out2.running) {
      const device = await invoke("omni_input_start", { muted: out2StartMuted() });
      out2.running = true;
      if (out2.gate) out2.gate(true);
      $("out2Note").textContent = "";
      $("out2Help").textContent = "Playing on " + device + (out2StartMuted() ? ", muted in the Sound Mixer" : "") + ".";
    }
    if (ctx.state === "suspended") ctx.resume().catch(() => {});
  } catch (e) {
    out2Teardown();
    $("out2Note").textContent = (e && e.message) || String(e);
  }
}

function out2Init() {
  // it used to send to another device; that was a different thing, so it starts off this time
  if (!prefs.out2Input) { prefs.out2On = false; prefs.out2Input = 1; savePrefs(); }
  $("out2Help").textContent = "";
  $("out2On").addEventListener("change", () => {
    if ($("out2On").checked && !confirm(out2StartMuted() ? OUT2_ROUTE_MUTED : OUT2_ROUTE)) { $("out2On").checked = false; return; }
    prefs.out2On = $("out2On").checked;
    savePrefs();
    out2Apply();
  });
  $("out2Muted").checked = out2StartMuted();
  $("out2Muted").addEventListener("change", () => { prefs.out2Muted = $("out2Muted").checked; savePrefs(); });
  $("out2Offset").addEventListener("change", () => {
    const v = Math.round(Number($("out2Offset").value));
    prefs.out2Offset = Number.isFinite(v) ? Math.max(-OUT2_MAX_MS, Math.min(OUT2_MAX_MS, v)) : 0;
    savePrefs();
    out2Apply();
  });
  $("out2Vol").addEventListener("input", () => {
    prefs.out2Volume = Number($("out2Vol").value) / 100;
    savePrefs();
    $("out2VolVal").textContent = $("out2Vol").value + "%";
    if (out2.gain) out2.gain.gain.setTargetAtTime(prefs.out2Volume, norm.ctx.currentTime, 0.02);
  });
  out2Apply();
}
