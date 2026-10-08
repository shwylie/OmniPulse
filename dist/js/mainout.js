// Discord & OBS Capture (Settings > Sound, on to start with): OmniPulse's sound is played from
// OmniPulse.exe itself (crates/core/src/main_out.rs) instead of the WebView. Programs that capture an
// app's sound by its window (Discord's screen share with sound, OBS's Application Audio Capture) only
// hear OmniPulse.exe, and the WebView plays from a process they don't count as the app.
//
//   ... master -> main delay -> a tap (js/cast-worklet.js, 20 ms pieces) -> OmniPulse.exe -> the device
//
// The volume is applied there, and what's waiting is dropped on a pause, a seek or a new track, so
// those are heard at once; the sound itself is about a tenth of a second behind the page.

const nativeOut = { on: false, tap: null, sink: null, starting: null };
const nativeOutWanted = () => !!window.__TAURI__ && prefs.nativeOut !== false;

// The device's name as Windows has it, from the page's device id ("" for the default)
async function sinkLabel(id) {
  if (!id || id === "default") return "";
  try {
    const d = (await navigator.mediaDevices.enumerateDevices()).find((x) => x.kind === "audiooutput" && x.deviceId === id);
    return d && d.label ? d.label : "";
  } catch (e) { return ""; }
}

function nativeOutStart() {
  if (nativeOut.on || nativeOut.starting || !nativeOutWanted() || !norm.ctx || !out2.mainDelay) return nativeOut.starting;
  nativeOut.starting = (async () => {
    try {
      await invoke("main_out_start", { device: await sinkLabel(prefs.sink) });
      const ctx = norm.ctx;
      await ctx.audioWorklet.addModule("js/cast-worklet.js");
      const tap = new AudioWorkletNode(ctx, "cast-tap", { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2], processorOptions: { ms: 20 } });
      const sink = ctx.createGain();
      sink.gain.value = 0;
      tap.connect(sink);
      sink.connect(ctx.destination);
      const rate = String(ctx.sampleRate);
      tap.port.onmessage = (e) => {
        if (nativeOut.on) invoke("main_out_pcm", new Uint8Array(e.data), { headers: { "x-rate": rate } }).catch(() => {});
      };
      // the WebView's own output goes quiet; the sound goes to the tap instead
      await mainViaElement("").catch(() => {});
      out2.mainDelay.connect(tap);
      try { out2.mainDelay.disconnect(ctx.destination); } catch (e) {}
      Object.assign(nativeOut, { on: true, tap, sink });
      setLevel(appVolume());
    } catch (e) {
      say("Discord & OBS Capture couldn't start, so the sound plays as before: " + (e && e.message ? e.message : e));
    } finally {
      nativeOut.starting = null;
    }
  })();
  return nativeOut.starting;
}

function nativeOutStop() {
  if (!nativeOut.on) return;
  nativeOut.on = false;
  try { out2.mainDelay.disconnect(nativeOut.tap); } catch (e) {}
  out2.mainDelay.connect(norm.ctx.destination);
  nativeOut.tap.port.onmessage = null;
  try { nativeOut.tap.disconnect(); } catch (e) {}
  nativeOut.tap = nativeOut.sink = null;
  invoke("main_out_stop").catch(() => {});
  setLevel(appVolume());
  if (prefs.sink && prefs.sink !== "default") mainSink(prefs.sink).catch(() => {});
}

// What's waiting would be heard late: dropped on a pause, a seek or a new track
function nativeOutFlush() {
  if (nativeOut.on) invoke("main_out_flush").catch(() => {});
}

function nativeOutVolume(v) {
  if (nativeOut.on) invoke("main_out_volume", { volume: v }).catch(() => {});
}

function nativeOutInit() {
  $("nativeOutOn").checked = prefs.nativeOut !== false;
  $("nativeOutOn").addEventListener("change", () => {
    prefs.nativeOut = $("nativeOutOn").checked;
    savePrefs();
    if (prefs.nativeOut) nativeOutStart(); else nativeOutStop();
  });
  audio.addEventListener("seeking", nativeOutFlush);
}
