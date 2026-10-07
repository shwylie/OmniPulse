// The audio chain: loudness leveling and the equalizer. Everything the app plays goes in at
// norm.input: the <audio> element (SoundCloud, local files) and the Spotify worklet.

// ---------- loudness leveling ----------
// Measures how loud the track is, slowly rides a gain toward a target level, then a
// compressor smooths what's left and a limiter stops peaks clipping. It's built from the
// browser engine's own audio nodes, so it costs very little.
const NORM_PRESETS = {
  gentle: { target: -18, maxBoost: 9, maxCut: 12, thr: -20, ratio: 3, knee: 12, attack: 0.02, release: 0.3 },
  strong: { target: -15, maxBoost: 12, maxCut: 15, thr: -26, ratio: 5, knee: 10, attack: 0.008, release: 0.2 },
};
const norm = { ctx: null, input: null, agc: null, comp: null, master: null, analyser: null, buf: null, pow: 0, gainDb: 0, fastUntil: 0 };

// Once the player is routed through the leveler it stays routed; "Off" simply sets it flat.
function normEnsure() {
  if (norm.ctx) return true;
  try {
    // 44.1 kHz to match Spotify's stream; the engine converts to the device's own rate
    const ctx = new (window.AudioContext || window.webkitAudioContext)({ sampleRate: 44100 });
    const input = ctx.createGain();
    ctx.createMediaElementSource(audio).connect(input);
    const analyser = ctx.createAnalyser();
    analyser.fftSize = 2048;
    const agc = ctx.createGain();
    const comp = ctx.createDynamicsCompressor();
    const limiter = ctx.createDynamicsCompressor();
    limiter.threshold.value = -1.5;
    limiter.knee.value = 0;
    limiter.ratio.value = 20;
    limiter.attack.value = 0.002;
    limiter.release.value = 0.12;
    const master = ctx.createGain();
    input.connect(analyser);   // measure the track as it arrives
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
    node.connect(agc);
    agc.connect(comp);
    comp.connect(limiter);
    limiter.connect(master);
    master.connect(ctx.destination);
    Object.assign(norm, { ctx, input, agc, comp, master, analyser, buf: new Float32Array(analyser.fftSize) });
    norm.eq = eq;
    if (prefs.sink && prefs.sink !== "default" && ctx.setSinkId) ctx.setSinkId(prefs.sink).catch(() => {});
    normApply();
    return true;
  } catch (e) {
    say("Could not start loudness leveling: " + e);
    return false;
  }
}

function normApply() {
  if (!norm.ctx) return;
  const p = NORM_PRESETS[prefs.normalize];
  const now = norm.ctx.currentTime;
  const c = norm.comp;
  if (p) {
    c.threshold.setValueAtTime(p.thr, now);
    c.ratio.setValueAtTime(p.ratio, now);
    c.knee.setValueAtTime(p.knee, now);
    c.attack.setValueAtTime(p.attack, now);
    c.release.setValueAtTime(p.release, now);
  } else {
    c.threshold.setValueAtTime(0, now);
    c.ratio.setValueAtTime(1, now);
    norm.gainDb = 0;
    norm.agc.gain.setTargetAtTime(1, now, 0.1);
  }
  setLevel(prefs.volume);
  eqApply();
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

function normTick() {
  if (!norm.ctx || !nowPlaying()) return;
  const p = NORM_PRESETS[prefs.normalize];
  if (!p) return;
  norm.analyser.getFloatTimeDomainData(norm.buf);
  let sum = 0;
  for (let i = 0; i < norm.buf.length; i++) sum += norm.buf[i] * norm.buf[i];
  const fast = performance.now() < norm.fastUntil;
  norm.pow = norm.pow * (fast ? 0.6 : 0.9) + (sum / norm.buf.length) * (fast ? 0.4 : 0.1);
  const db = 10 * Math.log10(norm.pow + 1e-12);
  if (db < -50) return;   // silence or a very quiet intro: leave the gain where it is
  const want = Math.max(-p.maxCut, Math.min(p.maxBoost, p.target - db));
  norm.gainDb += (want - norm.gainDb) * (fast ? 0.25 : 0.04);
  norm.agc.gain.setTargetAtTime(Math.pow(10, norm.gainDb / 20), norm.ctx.currentTime, 0.25);
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
  setInterval(normTick, 100);

  // Loudness leveling
  $("normSel").value = prefs.normalize || "off";
  $("normSel").addEventListener("change", () => {
    prefs.normalize = $("normSel").value;
    savePrefs();
    if (norm.ctx) {
      normApply();
      $("normNote").textContent = "Brings quiet tracks up and loud ones down, like a leveling compressor.";
    } else if (prefs.normalize !== "off") {
      $("normNote").textContent = "Starts with the next track you play.";
    }
  });

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
      $("eqNote").textContent = "Ten bands, for everything the app plays. Changes are heard straight away.";
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
