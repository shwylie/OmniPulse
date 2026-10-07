// A second output (Settings > Sound > Second output): the same sound on another device as well, with an
// offset to line the two up, like OBS's sync offset. Above 0 the second output is later; below 0 the
// main output is held back instead. It has its own volume, and plays whether or not the main one is
// muted (for a stream or a recording, say).
//
//   ... limiter -> master (volume) -> main delay -> your output device
//          \-> second delay -> second volume -> a stream -> an <audio> on the second device

const OUT2_MAX_MS = 5000;
const out2 = { delay: null, gain: null, dest: null, el: null, mainDelay: null };

// The main path's own delay sits in the chain from the start (it's 0 unless the offset is negative)
function out2Wire(ctx, master) {
  const mainDelay = ctx.createDelay(OUT2_MAX_MS / 1000 + 0.1);
  mainDelay.delayTime.value = 0;
  master.connect(mainDelay);
  mainDelay.connect(ctx.destination);
  out2.mainDelay = mainDelay;
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
    $("out2Note").textContent = id === (prefs.sink || "default") ? "That's the same device as the main output, so you'll hear it twice." : "";
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
