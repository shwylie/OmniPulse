// The audio chain: loudness match, equalizer, limiter, volume. Everything the app plays goes in at
// norm.input: the <audio> element (SoundCloud, local files) and the Spotify worklet.
//
//   input -> fade (js/fade.js) -> match (each track's gain, js/loudness.js) -> 10-band EQ -> limiter -> master (volume)
//      -> probe (an analyser: how loud it is right now, for skipping silence)
const norm = { ctx: null, input: null, fade: null, match: null, eq: null, limiter: null, master: null, probe: null };

// Once the player is routed through the chain it stays routed
function normEnsure() {
  if (norm.ctx) return true;
  try {
    // 44.1 kHz to match Spotify's stream; the engine converts to the device's own rate
    const ctx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 44100 });
    const input = ctx.createGain();
    ctx.createMediaElementSource(audio).connect(input);
    const match = ctx.createGain();
    // a safety net: quiet tracks turned up to match can't clip
    const limiter = ctx.createDynamicsCompressor();
    limiter.threshold.value = -1.5;
    limiter.knee.value = 0;
    limiter.ratio.value = 20;
    limiter.attack.value = 0.002;
    limiter.release.value = 0.12;
    const master = ctx.createGain();
    // 10-band equalizer (flat unless switched on)
    const eq = EQ_FREQS.map((f, i) => {
      const b = ctx.createBiquadFilter();
      b.type = i === 0 ? "lowshelf" : i === EQ_FREQS.length - 1 ? "highshelf" : "peaking";
      b.frequency.value = f;
      b.Q.value = 1.1;
      b.gain.value = 0;
      return b;
    });
    // fading between tracks (js/fade.js), before everything else
    const fade = ctx.createGain();
    input.connect(fade);
    fade.connect(match);
    let node = match;
    eq.forEach((b) => { node.connect(b); node = b; });
    node.connect(limiter);
    limiter.connect(master);
    out2Wire(ctx, master);   // master -> main delay -> the device (js/output2.js)
    // the probe has to lead somewhere to be run: into a silent gain
    const probe = ctx.createAnalyser();
    probe.fftSize = 2048;
    const sinkhole = ctx.createGain();
    sinkhole.gain.value = 0;
    input.connect(probe);
    probe.connect(sinkhole);
    sinkhole.connect(ctx.destination);
    Object.assign(norm, { ctx, input, fade, match, limiter, master, eq, probe });
    if (prefs.sink && prefs.sink !== "default" && !nativeOutWanted()) mainSink(prefs.sink).catch(() => {});
    normApply();
    nativeOutStart();   // OmniPulse.exe plays the sound (js/mainout.js)
    return true;
  } catch (e) {
    say("Could not start the audio chain: " + e);
    return false;
  }
}

// The volume the app plays at: the slider's, or 100% with the volume control hidden (when it's set
// on Windows' mixer, a Stream Deck or the like instead)
const appVolume = () => prefs.volume;

function normApply() {
  if (!norm.ctx) return;
  setLevel(appVolume());
  eqApply();
}

// How loud the input is right now (before any gain): { rms, peak } in dBFS
const probeBuf = new Float32Array(2048);
function probeLevel() {
  if (!norm.probe) return null;
  norm.probe.getFloatTimeDomainData(probeBuf);
  let sq = 0, peak = 0;
  for (let i = 0; i < probeBuf.length; i++) {
    const v = probeBuf[i];
    sq += v * v;
    if (Math.abs(v) > peak) peak = Math.abs(v);
  }
  const db = (x) => 20 * Math.log10(Math.max(x, 1e-10));
  return { rms: db(Math.sqrt(sq / probeBuf.length)), peak: db(peak) };
}

// The volume slider sits at the end of the chain
function setLevel(v) {
  if (norm.ctx && nativeOut.on) {
    // played from OmniPulse.exe: the volume is applied there, at once (js/mainout.js)
    audio.volume = 1;
    norm.master.gain.setTargetAtTime(1, norm.ctx.currentTime, 0.02);
    nativeOutVolume(audio.muted ? 0 : v);
  } else if (norm.ctx) {
    audio.volume = 1;
    norm.master.gain.setTargetAtTime(audio.muted ? 0 : v, norm.ctx.currentTime, 0.02);
  } else {
    audio.volume = v;
  }
}

function normMute(muted) {
  castMute(muted);
  if (nativeOut.on) { nativeOutVolume(muted ? 0 : appVolume()); return; }
  if (norm.ctx) norm.master.gain.setTargetAtTime(muted ? 0 : appVolume(), norm.ctx.currentTime, 0.02);
}

// ---------- equalizer ----------
const EQ_FREQS = [31, 62, 125, 250, 500, 1000, 2000, 4000, 8000, 16000];
const EQ_PRESETS = {
  flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  bass: [6, 5, 4, 2, 0, 0, 0, 0, 0, 0],
  vocal: [-2, -2, -1, 1, 3, 3, 2, 1, 0, -1],
  treble: [0, 0, 0, 0, 0, 1, 2, 4, 5, 6],
  smile: [5, 4, 2, 0, -1, -1, 0, 2, 4, 5],
};
const eqGains = () => (Array.isArray(prefs.eq) && prefs.eq.length === 10 ? prefs.eq : EQ_PRESETS.flat.slice());

function eqApply() {
  if (!norm.ctx || !norm.eq) return;
  const g = eqGains();
  norm.eq.forEach((b, i) => b.gain.setTargetAtTime(prefs.eqOn ? g[i] : 0, norm.ctx.currentTime, 0.05));
}

function renderEq() {
  const box = $("eqBands");
  box.replaceChildren();
  const g = eqGains();
  EQ_FREQS.forEach((f, i) => {
    const band = el("div", "eqBand");
    const val = el("span", "gain", (g[i] > 0 ? "+" : "") + g[i]);
    const r = el("input");
    r.type = "range";
    r.min = -12;
    r.max = 12;
    r.step = 1;
    r.value = g[i];
    r.dataset.i = i;
    r.setAttribute("aria-label", (f >= 1000 ? f / 1000 + " kHz" : f + " Hz") + " gain");
    band.append(val, r, el("span", "", f >= 1000 ? f / 1000 + "k" : String(f)));
    box.append(band);
  });
}

function setEq(gains) {
  prefs.eq = gains.slice();
  savePrefs();
  renderEq();
  eqApply();
}

function audioInit() {
  // Route the player through the chain from the start, so changes are heard
  // immediately rather than from the next track (it has to be in place before audio loads)
  audio.crossOrigin = "anonymous";
  normEnsure();

  // Equalizer
  $("eqOn").checked = !!prefs.eqOn;
  $("eqSection").hidden = !prefs.eqOn;
  renderEq();
  $("eqOn").addEventListener("change", () => {
    prefs.eqOn = $("eqOn").checked;
    savePrefs();
    $("eqSection").hidden = !prefs.eqOn;
    if (prefs.eqOn) {
      $("eqSection").open = true;
      $("eqNote").textContent = "Ten bands.";
    }
    eqApply();
  });
  $("eqBands").addEventListener("input", (e) => {
    const i = Number(e.target.dataset.i);
    if (Number.isNaN(i)) return;
    const g = eqGains();
    g[i] = Number(e.target.value);
    prefs.eq = g;
    savePrefs();
    e.target.parentElement.querySelector(".gain").textContent = (g[i] > 0 ? "+" : "") + g[i];
    eqApply();
  });
  $("eqPreset").addEventListener("change", () => {
    const p = EQ_PRESETS[$("eqPreset").value];
    if (p) setEq(p);
    $("eqPreset").value = "";
  });
  $("eqReset").addEventListener("click", () => setEq(EQ_PRESETS.flat));
}
