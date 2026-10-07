// The OmniPulse input (Settings > Streaming): the same sound sent to a second device as well, with an
// offset to line the two up, like OBS's sync offset. Above 0 it's later; below 0 the main output is
// held back instead. It has its own volume, and plays whether or not the main one is muted.
//
// Windows only adds an input device through a driver, so the sound goes to a virtual cable that has
// one: a Wave Link channel, VB-CABLE or the like. Its input side is what OBS or Discord picks up, and
// it can carry the name OmniPulse (a Wave Link channel of that name, or the cable renamed in Windows).
//
//   ... limiter -> master (volume) -> main delay -> your output device
//          \-> second delay -> second volume -> a stream -> an <audio> on the second device

const OUT2_MAX_MS = 5000;
const out2 = { delay: null, gain: null, dest: null, el: null, mainDelay: null, mainGain: null };

// The main path's own delay sits in the chain from the start (it's 0 unless the offset is negative)
// The main output, too, can need an <audio> on the device: the audio context can't open every device
// (some virtual ones refuse it with an AbortError), and an <audio> playing a stream of it can.
const mainOut = { dest: null, el: null };

async function mainSink(id) {
  const ctx = norm.ctx;
  if (!ctx || !out2.mainDelay) return;
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

// Devices that are virtual cables, so the help can point at them
const OUT2_CABLE = /omnipulse|cable input|vb-audio|voicemeeter|wave ?link|elgato virtual|virtual audio/i;

function out2Help() {
  const opts = [...$("out2Sel").options];
  const cables = opts.filter((o) => OUT2_CABLE.test(o.text));
  const named = cables.find((o) => /omnipulse/i.test(o.text));
  $("out2Help").textContent = named
    ? "Sending to " + named.text + ": pick its input in OBS (Audio Input Capture) or Discord."
    : cables.length
      ? "Pick a virtual cable below (" + cables.slice(0, 2).map((o) => o.text).join(", ") + "). To have it called OmniPulse: in Wave Link, add a channel named OmniPulse and pick it here; or rename the cable's recording device to OmniPulse in Windows' Sound settings. Then use that input in OBS or Discord."
      : "Windows needs a driver to add an input device, so this sends to a virtual cable: add a channel named OmniPulse in Elgato Wave Link, or install VB-CABLE (free, from vb-audio.com) and rename its recording device to OmniPulse in Windows' Sound settings. Then pick it below and restart the app if it isn't listed.";
}

// The first time, start on a device called OmniPulse if there is one
function out2Default() {
  if (prefs.out2Sink) return;
  const o = [...$("out2Sel").options].find((x) => /omnipulse/i.test(x.text));
  if (o) { $("out2Sel").value = o.value; prefs.out2Sink = o.value; savePrefs(); }
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

function out2MainMute(muted) {
  if (out2.mainGain) out2.mainGain.gain.setTargetAtTime(muted ? 0 : 1, norm.ctx.currentTime, 0.05);
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
  if (out2.mainDelay) out2.mainDelay.delayTime.setValueAtTime(on && offset < 0 ? -offset / 1000 : 0, now);
  if (!on) {
    if (out2.el) { out2.el.pause(); out2.el.srcObject = null; }
    if (out2.delay) { try { norm.limiter.disconnect(out2.delay); } catch (e) {} }
    out2.delay = out2.gain = out2.dest = out2.el = null;
    $("out2Note").textContent = "";
    return;
  }
  out2Default();
  out2Help();
  if (!out2.delay) {
    out2.delay = ctx.createDelay(OUT2_MAX_MS / 1000 + 0.1);
    out2.gain = ctx.createGain();
    out2.dest = ctx.createMediaStreamDestination();
    norm.limiter.connect(out2.delay);
    out2.delay.connect(out2.gain);
    out2.gain.connect(out2.dest);
    out2.el = new Audio();
    out2.el.srcObject = out2.dest.stream;
  }
  out2.delay.delayTime.setValueAtTime(offset > 0 ? offset / 1000 : 0, now);
  out2.gain.gain.setTargetAtTime(prefs.out2Volume ?? 1, now, 0.02);
  try {
    if (!out2.el.setSinkId) throw new Error("this engine can't choose a device for it");
    const id = prefs.out2Sink || "default";
    if (out2.el.sinkId !== id) await out2.el.setSinkId(id);
    await out2.el.play();
    $("out2Note").textContent = id === (prefs.sink || "default") ? "That's your main output too, so you'll hear it twice. Pick a virtual cable instead." : "";
  } catch (e) {
    $("out2Note").textContent = "Couldn't play on that device: " + (e && e.message ? e.message : e);
  }
}

function out2Init() {
  $("out2On").addEventListener("change", () => { prefs.out2On = $("out2On").checked; savePrefs(); out2Apply(); });
  $("out2Sel").addEventListener("change", () => { prefs.out2Sink = $("out2Sel").value; savePrefs(); out2Apply(); });
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
