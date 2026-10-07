// The audio chain: equalizer, loudness leveler, limiter, volume. Everything the app plays goes in at
// norm.input: the <audio> element (SoundCloud, local files) and the Spotify worklet.

// ---------- loudness leveler ----------
// A compressor in the spirit of RoughRider, on the audio thread (js/leveler-worklet.js), with one
// Strength control: Gentle and Strong are fixed points on it, Custom is your own.
const LEVELER_STRENGTH = { gentle: 0.3, strong: 0.65 };
const norm = { ctx: null, input: null, leveler: null, slot: null, limiter: null, master: null };

const levelerStrength = () => (prefs.normalize === "custom"
  ? Math.max(0, Math.min(100, Number(prefs.normStrength ?? 50))) / 100
  : LEVELER_STRENGTH[prefs.normalize] ?? 0);

// Once the player is routed through the chain it stays routed; "Off" simply passes it through.
function normEnsure() {
  if (norm.ctx) return true;
  try {
    // 44.1 kHz to match Spotify's stream; the engine converts to the device's own rate
    const ctx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 44100 });
    const input = ctx.createGain();
    ctx.createMediaElementSource(audio).connect(input);
    // where the leveler goes, once its worklet has loaded (straight through until then)
    const slot = ctx.createGain();
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
    let node = input;
    eq.forEach((b) => { node.connect(b); node = b; });
    node.connect(slot);
    slot.connect(limiter);
    limiter.connect(master);
    master.connect(ctx.destination);
    Object.assign(norm, { ctx, input, slot, limiter, master, eq });
    if (prefs.sink && prefs.sink !== "default" && ctx.setSinkId) ctx.setSinkId(prefs.sink).catch(() => {});
    ctx.audioWorklet.addModule("js/leveler-worklet.js").then(() => {
      const lev = new AudioWorkletNode(ctx, "leveler", { outputChannelCount: [2] });
      lev.port.onmessage = (e) => { if (e.data.type === "meter") paintLevelerMeter(e.data); };
      slot.disconnect();
      slot.connect(lev);
      lev.connect(limiter);
      norm.leveler = lev;
      normApply();
    }).catch((e) => say("The loudness leveler couldn't start: " + e));
    normApply();
    return true;
  } catch (e) {
    say("Could not start the audio chain: " + e);
    return false;
  }
}

function normApply() {
  if (!norm.ctx) return;
  if (norm.leveler) norm.leveler.port.postMessage({ type: "set", on: prefs.normalize && prefs.normalize !== "off" && levelerStrength() > 0, strength: levelerStrength() });
  setLevel(prefs.volume);
  eqApply();
}

// A new track: let the leveler settle on its level quickly
function levelerNewTrack() {
  if (norm.leveler) norm.leveler.port.postMessage({ type: "newTrack" });
}

// The meter under Custom: how much it's turning down right now, and the rider's gain
function paintLevelerMeter(m) {
  const gr = Math.min(0, m.gr);
  $("levMeterBar").style.width = Math.min(100, (-gr / 18) * 100).toFixed(1) + "%";
  $("levMeterVal").textContent = (gr > -0.05 ? "0.0" : gr.toFixed(1)) + " dB" +
    (Math.abs(m.ride) >= 0.1 ? ", level " + (m.ride > 0 ? "+" : "") + m.ride.toFixed(1) + " dB" : "");
}

function setLevelerMetering(on) {
  if (norm.leveler) norm.leveler.port.postMessage({ type: "meter", on });
}

// The volume slider sits after the leveler, so leveling doesn't fight it
function setLevel(v) {
  if (norm.ctx) {
    audio.volume = 1;
    norm.master.gain.setTargetAtTime(audio.muted ? 0 : v, norm.ctx.currentTime, 0.02);
  } else {
    audio.volume = v;
  }
}

function normMute(muted) {
  if (norm.ctx) norm.master.gain.setTargetAtTime(muted ? 0 : prefs.volume, norm.ctx.currentTime, 0.02);
}

const NORM_NOTES = {
  off: "Brings quiet tracks up and loud ones down.",
  gentle: "Light, transparent levelling.",
  strong: "Tighter: everything sits at a similar loudness.",
  custom: "Set the strength yourself.",
};
function paintLeveler() {
  const mode = prefs.normalize || "off";
  $("normSel").value = mode;
  $("normNote").textContent = NORM_NOTES[mode] || NORM_NOTES.off;
  $("levCustom").hidden = mode !== "custom";
  const s = Math.round(levelerStrength() * 100);
  $("levStrength").value = String(mode === "custom" ? s : Math.round((prefs.normStrength ?? 50)));
  $("levStrengthVal").textContent = $("levStrength").value + "%";
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
  // Route the player through the equalizer / leveler from the start, so changes are heard
  // immediately rather than from the next track (it has to be in place before audio loads)
  audio.crossOrigin = "anonymous";
  normEnsure();

  // Loudness leveler
  paintLeveler();
  $("normSel").addEventListener("change", () => {
    prefs.normalize = $("normSel").value;
    savePrefs();
    paintLeveler();
    normApply();
    setLevelerMetering(prefs.normalize === "custom" && $("settings").open);
  });
  $("levStrength").addEventListener("input", () => {
    prefs.normStrength = Number($("levStrength").value);
    savePrefs();
    $("levStrengthVal").textContent = prefs.normStrength + "%";
    normApply();
  });
  // the meter only runs while you can see it
  new MutationObserver(() => setLevelerMetering(prefs.normalize === "custom" && $("settings").open))
    .observe($("settings"), { attributes: true, attributeFilter: ["open"] });

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
      $("eqNote").textContent = "Ten bands. Changes are heard straight away.";
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
