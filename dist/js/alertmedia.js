// Alerts' pictures and sounds, in the app (Settings > Stream Widget > Alerts > an alert's settings):
//   - its picture cropped: zoomed, moved, filling its tile or fitting in it (the widget does the same)
//   - its collage: the icons and up to six pictures of its own that its tiles show
//   - its sound's loudness, measured when it's added, so every alert peaks about as loud as the
//     others; your own adjustment on top, a delay, and a preview with its levels drawn as it plays
//   - a musical alert: the app plays its sound itself, through its own output, pausing the music for
//     it and fading the music back in after (the widget tells the app when it shows one: /shown)

const ALERT_COLLAGE_MAX = 6;
// The icons, as the widget draws them (crates/core/widget/widget.html ALERT_ICONS)
const ALERT_ICON_PATHS = {
  follow: "M12 21.35l-1.45-1.32C5.4 15.36 2 12.28 2 8.5 2 5.42 4.42 3 7.5 3c1.74 0 3.41.81 4.5 2.09C13.09 3.81 14.76 3 16.5 3 19.58 3 22 5.42 22 8.5c0 3.78-3.4 6.86-8.55 11.54L12 21.35z",
  sub: "M12 17.27L18.18 21l-1.64-7.03L22 9.24l-7.19-.61L12 2 9.19 8.63 2 9.24l5.46 4.73L5.82 21z",
  gift: "M20 6h-2.18c.11-.31.18-.65.18-1 0-1.66-1.34-3-3-3-1.05 0-1.96.54-2.5 1.35l-.5.67-.5-.68C10.96 2.54 10.05 2 9 2 7.34 2 6 3.34 6 5c0 .35.07.69.18 1H4c-1.11 0-1.99.89-1.99 2L2 19c0 1.11.89 2 2 2h16c1.11 0 2-.89 2-2V8c0-1.11-.89-2-2-2zm-5-2c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zM9 4c.55 0 1 .45 1 1s-.45 1-1 1-1-.45-1-1 .45-1 1-1zm11 15H4v-2h16v2zm0-5H4V8h5.08L7 10.83 8.62 12 12 7.4l3.38 4.6L17 10.83 14.92 8H20v6z",
  bits: "M12 2L4 9l8 13 8-13-8-7zm0 3.2L16.9 9H7.1L12 5.2z",
  donation: "M11.8 10.9c-2.27-.59-3-1.2-3-2.15 0-1.09 1.01-1.85 2.7-1.85 1.78 0 2.44.85 2.5 2.1h2.21c-.07-1.72-1.12-3.3-3.21-3.81V3h-3v2.16c-1.94.42-3.5 1.68-3.5 3.61 0 2.31 1.91 3.46 4.7 4.13 2.5.6 3 1.48 3 2.41 0 .69-.49 1.79-2.7 1.79-2.06 0-2.87-.92-2.98-2.1h-2.2c.12 2.19 1.76 3.42 3.68 3.83V21h3v-2.15c1.95-.37 3.5-1.5 3.5-3.55 0-2.84-2.43-3.81-4.7-4.4z",
  hypetrain: "M12 2c-4 0-8 .5-8 4v9.5C4 17.43 5.57 19 7.5 19L6 20.5v.5h2.23l2-2H14l2 2h2v-.5L16.5 19c1.93 0 3.5-1.57 3.5-3.5V6c0-3.5-3.58-4-8-4zM7.5 17c-.83 0-1.5-.67-1.5-1.5S6.67 14 7.5 14s1.5.67 1.5 1.5S8.33 17 7.5 17zm3.5-7H6V6h5v4zm2 0V6h5v4h-5zm3.5 7c-.83 0-1.5-.67-1.5-1.5s.67-1.5 1.5-1.5 1.5.67 1.5 1.5-.67 1.5-1.5 1.5z",
  poll: "M19 3H5c-1.1 0-2 .9-2 2v14c0 1.1.9 2 2 2h14c1.1 0 2-.9 2-2V5c0-1.1-.9-2-2-2zM9 17H7v-7h2v7zm4 0h-2V7h2v10zm4 0h-2v-4h2v4z",
  ban: "M12 2C6.48 2 2 6.48 2 12s4.48 10 10 10 10-4.48 10-10S17.52 2 12 2zM4 12c0-4.42 3.58-8 8-8 1.85 0 3.55.63 4.9 1.69L5.69 16.9C4.63 15.55 4 13.85 4 12zm8 8c-1.85 0-3.55-.63-4.9-1.69L18.31 7.1C19.37 8.45 20 10.15 20 12c0 4.42-3.58 8-8 8z",
  watchstreak: "M13.5.67s.74 2.65.74 4.8c0 2.06-1.35 3.73-3.41 3.73-2.07 0-3.63-1.67-3.63-3.73l.03-.36C5.21 7.51 4 10.62 4 14c0 4.42 3.58 8 8 8s8-3.58 8-8C20 8.61 17.41 3.8 13.5.67zM11.71 19c-1.78 0-3.22-1.4-3.22-3.14 0-1.62 1.05-2.76 2.81-3.12 1.77-.36 3.6-1.21 4.62-2.58.39 1.29.59 2.65.59 4.04 0 2.65-2.15 4.8-4.8 4.8z",
  raid: "M16 11c1.66 0 2.99-1.34 2.99-3S17.66 5 16 5c-1.66 0-3 1.34-3 3s1.34 3 3 3zm-8 0c1.66 0 2.99-1.34 2.99-3S9.66 5 8 5C6.34 5 5 6.34 5 8s1.34 3 3 3zm0 2c-2.33 0-7 1.17-7 3.5V19h14v-2.5c0-2.33-4.67-3.5-7-3.5zm8 0c-.29 0-.62.02-.97.05 1.16.84 1.97 1.97 1.97 3.45V19h6v-2.5c0-2.33-4.67-3.5-7-3.5z",
  kicks: "M7 2v11h3v9l7-12h-4l4-8z",
  bell: "M12 22c1.1 0 2-.9 2-2h-4c0 1.1.89 2 2 2zm6-6v-5c0-3.07-1.64-5.64-4.5-6.32V4c0-.83-.67-1.5-1.5-1.5s-1.5.67-1.5 1.5v.68C7.63 5.36 6 7.92 6 11v5l-2 2v1h16v-1l-2-2z",
  trophy: "M19 5h-2V3H7v2H5c-1.1 0-2 .9-2 2v1c0 2.55 1.92 4.63 4.39 4.94.63 1.5 1.98 2.63 3.61 2.96V19H7v2h10v-2h-4v-3.1c1.63-.33 2.98-1.46 3.61-2.96C19.08 12.63 21 10.55 21 8V7c0-1.1-.9-2-2-2zM5 8V7h2v3.82C5.84 10.4 5 9.3 5 8zm14 0c0 1.3-.84 2.4-2 2.82V7h2v1z",
  hammer: "M2 19.63L13.43 8.2l-.71-.71 1.42-1.41-2.13-2.12 3.53-3.54 2.13 2.12 1.41-1.41 2.83 2.83-1.41 1.41 1.41 1.42-2.12 2.12-1.42-1.41-.7.7L5.24 21.63z",
  rocket: "M12 2.5s4.5 2.04 4.5 10.5c0 2.49-1.04 5.57-1.6 7H9.1c-.56-1.43-1.6-4.51-1.6-7C7.5 4.54 12 2.5 12 2.5zm2 8.5c0-1.1-.9-2-2-2s-2 .9-2 2 .9 2 2 2 2-.9 2-2zm-6.31 9.52c-.48-1.23-1.52-4.17-1.67-6.87l-1.13.75c-.56.38-.89 1-.89 1.67V22l3.69-1.48zM20 22v-5.93c0-.67-.33-1.29-.89-1.66l-1.13-.75c-.15 2.69-1.2 5.64-1.67 6.87L20 22z",
  music: "M12 3v10.55c-.59-.34-1.27-.55-2-.55-2.21 0-4 1.79-4 4s1.79 4 4 4 4-1.79 4-4V7h4V3h-6z",
  sparkle: "M19 9l1.25-2.75L23 5l-2.75-1.25L19 1l-1.25 2.75L15 5l2.75 1.25L19 9zm-7.5.5L9 4 6.5 9.5 1 12l5.5 2.5L9 20l2.5-5.5L17 12l-5.5-2.5zM19 15l-1.25 2.75L15 19l2.75 1.25L19 23l1.25-2.75L23 19l-2.75-1.25L19 15z",
};

const ALERT_LOUD_DB = -16;   // where every sound's loudest moment is brought to (a 400 ms average)
const ALERT_PEAK_DB = -1;    // and no higher than this at its very peak

// ---------- its files, as the app keeps them ----------
const alertBlobs = new Map();   // "kind|slot|at" -> a blob address for the picture or sound
const MEDIA_TYPES = { gif: "image/gif", png: "image/png", webp: "image/webp", jpg: "image/jpeg", jpeg: "image/jpeg" };
async function alertMediaUrl(kind, slot, at, name) {
  const key = kind + "|" + slot + "|" + at;
  if (alertBlobs.has(key)) return alertBlobs.get(key);
  const bytes = await invoke("alert_media_bytes", { kind, slot });
  const ext = String(name || "").split(".").pop().toLowerCase();
  const url = URL.createObjectURL(new Blob([bytes], MEDIA_TYPES[ext] ? { type: MEDIA_TYPES[ext] } : {}));
  alertBlobs.set(key, url);
  return url;
}

// ---------- the sound's loudness ----------
const alertLevels = (k) => (prefs.alertLevels || {})[k] || null;
function alertLevelSet(kind, v) {
  const all = Object.assign({}, prefs.alertLevels);
  if (v) all[kind] = Object.assign({}, all[kind], v); else delete all[kind];
  prefs.alertLevels = all;
  alertsSave();
}
// what the widget plays it at: the measured level plus yours, in dB
const alertGainDb = (kind) => { const l = alertLevels(kind); return l ? (Number(l.auto) || 0) + (Number(l.adj) || 0) : 0; };

const alertDecoded = new Map();   // kind|at -> AudioBuffer
async function alertDecode(kind) {
  const snd = (prefs.alertSounds || {})[kind];
  if (!snd) throw new Error("no sound");
  const key = kind + "|" + snd.at;
  if (alertDecoded.has(key)) return alertDecoded.get(key);
  normEnsure();
  const bytes = await invoke("alert_media_bytes", { kind, slot: "sound" });
  const buf = await norm.ctx.decodeAudioData(bytes.slice ? bytes.slice(0) : new Uint8Array(bytes).buffer);
  alertDecoded.set(key, buf);
  return buf;
}

// Its peak, its loudest 400 ms, its length, and its shape (for drawing)
function alertMeasure(buf) {
  const n = buf.length, ch = [];
  for (let c = 0; c < buf.numberOfChannels; c++) ch.push(buf.getChannelData(c));
  const at = (i) => { let v = 0; ch.forEach((d) => { v = Math.max(v, Math.abs(d[i])); }); return v; };
  let peak = 0;
  const env = new Array(160).fill(0);
  const per = Math.max(1, Math.ceil(n / env.length));
  for (let i = 0; i < n; i++) { const v = at(i); if (v > peak) peak = v; const b = Math.floor(i / per); if (v > env[b]) env[b] = v; }
  const win = Math.round(buf.sampleRate * 0.4), hop = Math.round(buf.sampleRate * 0.1);
  let loud = 0;
  for (let s = 0; s + 1 < n; s += hop) {
    let sum = 0, cnt = 0;
    for (let i = s; i < Math.min(n, s + win); i++) { ch.forEach((d) => { sum += d[i] * d[i]; cnt++; }); }
    if (cnt) loud = Math.max(loud, Math.sqrt(sum / cnt));
    if (s + win >= n) break;
  }
  const db = (v) => (v > 0 ? 20 * Math.log10(v) : -96);
  const peakDb = db(peak), loudDb = db(loud);
  const auto = Math.round(Math.max(-24, Math.min(18, ALERT_LOUD_DB - loudDb, ALERT_PEAK_DB - peakDb)) * 2) / 2;
  return { auto, peak: Math.round(peakDb * 10) / 10, loud: Math.round(loudDb * 10) / 10, ms: Math.round(buf.duration * 1000), env: env.map((v) => Math.round(v * 1000) / 1000) };
}

// Measured once, when the sound's added (and for one added before this was here, when it's opened)
async function alertAnalyse(kind, keepAdj) {
  try {
    const m = alertMeasure(await alertDecode(kind));
    const old = alertLevels(kind);
    alertLevelSet(kind, Object.assign(m, { adj: keepAdj && old ? old.adj || 0 : 0 }));
    return true;
  } catch (e) {
    say("Couldn't measure that sound: " + (e && e.message ? e.message : e));
    return false;
  }
}

// ---------- playing a sound in the app ----------
// Through the app's own output, after the music's leveller and EQ (so neither touches it), at the
// alerts' volume and its gain. Returns { stop, done, analyser, started }.
function alertPlayHere(kind, buf, delayMs) {
  normEnsure();
  const ctx = norm.ctx;
  const vol = prefs.alertVol === undefined ? 0.8 : Number(prefs.alertVol);
  const src = ctx.createBufferSource();
  src.buffer = buf;
  const g = ctx.createGain();
  g.gain.value = vol * Math.pow(10, alertGainDb(kind) / 20);
  const an = ctx.createAnalyser();
  an.fftSize = 1024;
  src.connect(g).connect(an).connect(norm.limiter);
  if (ctx.state === "suspended") ctx.resume().catch(() => {});
  const started = ctx.currentTime + Math.max(0, delayMs || 0) / 1000;
  src.start(started);
  const done = new Promise((r) => { src.onended = r; });
  return { stop: () => { try { src.stop(); } catch (e) { /* already stopped */ } }, done, analyser: an, started, ctx };
}

// ---------- musical alerts: the music pauses for them ----------
let alertMusicChain = Promise.resolve();
window.__alertShown = (kind) => {
  if (!alertEdits(kind).music || !(prefs.alertSounds || {})[kind]) return;
  alertMusicChain = alertMusicChain.then(() => alertPlayMusical(kind)).catch(() => {});
};

async function alertPlayMusical(kind) {
  const buf = await alertDecode(kind);
  const wasPlaying = nowPlaying();
  if (wasPlaying) {
    fadeTo(0, 350);
    await new Promise((r) => setTimeout(r, 380));
    if (nowPlaying()) toggle();
  }
  const p = alertPlayHere(kind, buf, Number(alertEdits(kind).sdelay) || 0);
  await p.done;
  // back in, unless you started or stopped it meanwhile
  if (wasPlaying && !nowPlaying()) {
    fadeTo(0, 0);
    toggle();
    setTimeout(() => fadeTo(1, 1500), 120);
  } else if (wasPlaying) fadeTo(1, 600);
}

// ---------- the editors ----------
// The sound: its levels as it'll play (red past the top), the measured level and your adjustment,
// when it starts, whether it's musical, and a preview that draws where it is and how loud
function alertSoundEditor(kind) {
  const wrap = el("div", "alertSoundEd");
  const snd = (prefs.alertSounds || {})[kind];
  if (!snd) { wrap.append(el("div", "note", "No sound. Add one on the alert's row.")); return wrap; }
  const lv = alertLevels(kind);
  if (!lv || !lv.env) {
    wrap.append(el("div", "note", "Measuring its loudness..."));
    alertAnalyse(kind, true).then((ok) => { if (ok) alertsPaintList(); });
    return wrap;
  }
  const cv = el("canvas", "alertLevels");
  cv.width = 320;
  cv.height = 56;
  const meter = el("div", "alertMeter");
  const fillM = el("span");
  meter.append(fillM);
  const view = el("div", "alertLevelView");
  view.append(cv, meter);
  const draw = (pos) => {
    const g = cv.getContext("2d");
    const w = cv.width, h = cv.height, k = Math.pow(10, alertGainDb(kind) / 20);
    const css = getComputedStyle(document.documentElement);
    g.clearRect(0, 0, w, h);
    const top = Math.pow(10, ALERT_PEAK_DB / 20);
    lv.env.forEach((v, i) => {
      const a = Math.min(1.2, v * k);
      const bh = Math.max(1, (a / 1.2) * h);
      g.fillStyle = a > 1 ? "#ff5a5a" : a > top ? "#f2b544" : css.getPropertyValue("--accent").trim() || "#8e9bff";
      g.fillRect((i / lv.env.length) * w, (h - bh) / 2, Math.max(1, w / lv.env.length - 1), bh);
    });
    // the top: where it would distort
    g.fillStyle = "rgba(255,255,255,0.35)";
    const y = (1 - 1 / 1.2) * h / 2;
    g.fillRect(0, y, w, 1);
    g.fillRect(0, h - y, w, 1);
    if (pos !== undefined) { g.fillStyle = "#fff"; g.fillRect(pos * w, 0, 2, h); }
  };
  draw();
  const info = el("div", "note");
  const paintInfo = () => {
    const peak = lv.peak + alertGainDb(kind);
    info.textContent = "Measured: " + (lv.auto >= 0 ? "+" : "") + lv.auto + " dB to match the others. Loudest moment " +
      (peak > -0.05 ? "would distort" : "peaks at " + peak.toFixed(1) + " dB") + ". " + (lv.ms / 1000).toFixed(1) + " s long.";
  };
  paintInfo();
  const adj = el("input");
  adj.type = "range";
  adj.min = "-12";
  adj.max = "12";
  adj.step = "0.5";
  adj.value = String(lv.adj || 0);
  adj.setAttribute("aria-label", "Your adjustment, in dB");
  const adjVal = el("span", "alertAdjVal", (lv.adj > 0 ? "+" : "") + (lv.adj || 0) + " dB");
  adj.addEventListener("input", () => {
    lv.adj = Number(adj.value);
    adjVal.textContent = (lv.adj > 0 ? "+" : "") + lv.adj + " dB";
    draw();
    paintInfo();
  });
  adj.addEventListener("change", () => alertLevelSet(kind, { adj: Number(adj.value) }));
  const adjRow = el("div", "alertInline");
  adjRow.append(adj, adjVal);
  const e = alertEdits(kind);
  const delay = el("select", "narrow");
  [0, 250, 500, 1000, 1500, 2000, 3000].forEach((ms) => { const o = el("option", "", ms ? (ms / 1000) + " s after" : "With the alert"); o.value = String(ms); delay.append(o); });
  delay.value = String(Number(e.sdelay) || 0);
  delay.addEventListener("change", () => alertEditSet(kind, "sdelay", Number(delay.value) || ""));
  const music = el("input");
  music.type = "checkbox";
  music.checked = !!e.music;
  music.addEventListener("change", () => alertEditSet(kind, "music", music.checked || ""));
  const musicLbl = el("label", "alertCheck");
  musicLbl.append(music, document.createTextNode(" Musical"));
  const musicNote = el("div", "note", "Musical: your music pauses for it, and it plays through OmniPulse (not the widget), then the music fades back in.");
  const play = el("button", "soft", "Preview");
  const auto = el("button", "soft", "Measure Again");
  auto.title = "Work out its level again (your adjustment stays)";
  auto.addEventListener("click", async () => { if (await alertAnalyse(kind, true)) alertsPaintList(); });
  let playing = null;
  play.addEventListener("click", async () => {
    if (playing) { playing.stop(); return; }
    try {
      const buf = await alertDecode(kind);
      playing = alertPlayHere(kind, buf, Number(alertEdits(kind).sdelay) || 0);
      play.textContent = "Stop";
      const data = new Float32Array(playing.analyser.fftSize);
      const frame = () => {
        if (!playing) return;
        const t = (playing.ctx.currentTime - playing.started) / buf.duration;
        playing.analyser.getFloatTimeDomainData(data);
        let pk = 0;
        for (let i = 0; i < data.length; i++) pk = Math.max(pk, Math.abs(data[i]));
        fillM.style.height = Math.min(100, pk * 100) + "%";
        fillM.classList.toggle("hot", pk > 0.99);
        draw(Math.max(0, Math.min(1, t)));
        requestAnimationFrame(frame);
      };
      frame();
      await playing.done;
    } catch (err) {
      say("Couldn't play it: " + (err && err.message ? err.message : err));
    }
    playing = null;
    play.textContent = "Preview";
    fillM.style.height = "0";
    draw();
  });
  const btns = el("div", "alertInline");
  btns.append(play, auto);
  const r = (label, input) => { const x = el("div", "alertField"); x.append(el("label", "", label), input); return x; };
  wrap.append(view, r("Level", adjRow), info, r("Starts", delay), r("", musicLbl), musicNote, r("", btns));
  return wrap;
}

// The picture, cropped: drag it about, zoom in, and fill its tile or fit in it
function alertCropImg(img, c) {
  c = c || {};
  const x = c.x === undefined ? 50 : c.x, y = c.y === undefined ? 50 : c.y, z = c.z || 1;
  img.style.objectFit = c.fit === "cover" ? "cover" : "contain";
  img.style.objectPosition = x + "% " + y + "%";
  img.style.transformOrigin = x + "% " + y + "%";
  img.style.transform = z > 1 ? "scale(" + z + ")" : "";
}

function alertCropEditor(kind) {
  const wrap = el("div", "alertCrop");
  const img = (prefs.alertImages || {})[kind];
  if (!img) return wrap;
  const crop = Object.assign({ x: 50, y: 50, z: 1, fit: "contain" }, alertEdits(kind).crop);
  const round = ["pill", "vinyl"].includes(prefs.widgetDesign);
  const frame = el("div", "cropFrame" + (round ? " round" : ""));
  const pic = el("img");
  pic.alt = "";
  pic.draggable = false;
  frame.append(pic);
  alertMediaUrl(kind, "image", img.at, img.name).then((u) => { pic.src = u; }).catch(() => { frame.append(el("span", "note", "Couldn't load it")); });
  const paint = () => alertCropImg(pic, crop);
  paint();
  const save = () => alertEditSet(kind, "crop", { x: Math.round(crop.x), y: Math.round(crop.y), z: Math.round(crop.z * 100) / 100, fit: crop.fit });
  // dragging moves the picture under the frame
  frame.addEventListener("pointerdown", (ev) => {
    frame.setPointerCapture(ev.pointerId);
    const sx = ev.clientX, sy = ev.clientY, ox = crop.x, oy = crop.y, size = frame.clientWidth;
    const move = (m) => {
      crop.x = Math.max(0, Math.min(100, ox - ((m.clientX - sx) / size) * 100 / crop.z));
      crop.y = Math.max(0, Math.min(100, oy - ((m.clientY - sy) / size) * 100 / crop.z));
      paint();
    };
    const up = () => { frame.removeEventListener("pointermove", move); frame.removeEventListener("pointerup", up); save(); };
    frame.addEventListener("pointermove", move);
    frame.addEventListener("pointerup", up);
  });
  frame.addEventListener("wheel", (ev) => {
    ev.preventDefault();
    crop.z = Math.max(1, Math.min(4, crop.z * (ev.deltaY < 0 ? 1.08 : 1 / 1.08)));
    zoom.value = String(crop.z);
    paint();
    clearTimeout(frame._t);
    frame._t = setTimeout(save, 300);
  }, { passive: false });
  const zoom = el("input");
  zoom.type = "range";
  zoom.min = "1";
  zoom.max = "4";
  zoom.step = "0.05";
  zoom.value = String(crop.z);
  zoom.setAttribute("aria-label", "Zoom");
  zoom.addEventListener("input", () => { crop.z = Number(zoom.value); paint(); });
  zoom.addEventListener("change", save);
  const fit = el("select", "narrow");
  [["contain", "Fit in the Tile"], ["cover", "Fill the Tile"]].forEach(([v, n]) => { const o = el("option", "", n); o.value = v; fit.append(o); });
  fit.value = crop.fit;
  fit.addEventListener("change", () => { crop.fit = fit.value; paint(); save(); });
  const reset = el("button", "soft", "Reset");
  reset.addEventListener("click", () => { Object.assign(crop, { x: 50, y: 50, z: 1, fit: "contain" }); zoom.value = "1"; fit.value = "contain"; paint(); alertEditSet(kind, "crop", ""); });
  const side = el("div", "cropSide");
  side.append(fit, el("label", "note", "Zoom"), zoom, el("div", "note", "Drag the picture to move it; scroll to zoom."), reset);
  wrap.append(frame, side);
  return wrap;
}

// The collage: which icons, and up to six pictures of its own
function alertCollageEditor(kind) {
  const wrap = el("div", "alertCollageEd");
  const e = alertEdits(kind);
  const chosen = new Set(e.cicons || []);
  const chips = el("div", "alertIconChips");
  ALERT_ICON_NAMES.forEach(([id, name]) => {
    const b = el("button", "alertIconChip" + (chosen.has(id) ? " on" : ""));
    b.type = "button";
    b.title = name;
    b.setAttribute("aria-label", name);
    b.setAttribute("aria-pressed", chosen.has(id) ? "true" : "false");
    b.append(icon(ALERT_ICON_PATHS[id] || ALERT_ICON_PATHS.bell));
    b.addEventListener("click", () => {
      if (chosen.has(id)) chosen.delete(id); else chosen.add(id);
      alertEditSet(kind, "cicons", chosen.size ? [...chosen] : "");
      alertsPaintList();
    });
    chips.append(b);
  });
  const pics = (prefs.alertCollage || {})[kind] || [];
  const row = el("div", "alertCollagePics");
  for (let i = 0; i < ALERT_COLLAGE_MAX; i++) {
    const p = pics[i];
    const slot = el("div", "collageSlot");
    if (p) {
      const im = el("img");
      im.alt = "";
      alertMediaUrl(kind + "-" + (i + 1), "collage", p.at, p.name).then((u) => { im.src = u; }).catch(() => {});
      const x = el("button", "", "×");
      x.title = "Take it out";
      x.setAttribute("aria-label", "Remove picture " + (i + 1));
      x.addEventListener("click", () => alertCollageClear(kind, i));
      slot.append(im, x);
    } else {
      const add = el("button", "collageAdd", "+");
      add.title = "Add a picture or GIF to its collage";
      add.setAttribute("aria-label", add.title);
      add.addEventListener("click", () => alertCollagePick(kind, i));
      slot.append(add);
    }
    row.append(slot);
  }
  wrap.append(chips, row, el("div", "note", (chosen.size || pics.some(Boolean)
    ? "Its tiles show its own picture, then these."
    : "Nothing chosen: its icon, with a couple that go with it.") + " With the Collage transition."));
  return wrap;
}

async function alertCollagePick(kind, i) {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg) { say("Picking a file needs the app."); return; }
  try {
    const path = await dlg.open({ multiple: false, title: "Choose a picture or GIF for this alert's collage", filters: [{ name: "Pictures", extensions: ALERT_IMAGE_EXTS }] });
    if (!path) return;
    const name = await invoke("alert_media_set", { kind: kind + "-" + (i + 1), slot: "collage", path: String(path) });
    const all = Object.assign({}, prefs.alertCollage);
    const list = (all[kind] || []).slice();
    list[i] = { name, at: Date.now() };
    all[kind] = list;
    prefs.alertCollage = all;
    alertsSave();
    alertsPaintList();
  } catch (e) {
    say(String(e && e.message ? e.message : e));
  }
}

async function alertCollageClear(kind, i) {
  await invoke("alert_media_clear", { kind: kind + "-" + (i + 1), slot: "collage" }).catch(() => {});
  const all = Object.assign({}, prefs.alertCollage);
  const list = (all[kind] || []).slice();
  list[i] = null;
  if (list.some(Boolean)) all[kind] = list; else delete all[kind];
  prefs.alertCollage = all;
  alertsSave();
  alertsPaintList();
}

// All of an alert's collage pictures, gone (it was deleted, or its trigger renamed)
function alertCollageDrop(kind) {
  for (let i = 0; i < ALERT_COLLAGE_MAX; i++) invoke("alert_media_clear", { kind: kind + "-" + (i + 1), slot: "collage" }).catch(() => {});
  if (prefs.alertCollage && prefs.alertCollage[kind]) { prefs.alertCollage = Object.assign({}, prefs.alertCollage); delete prefs.alertCollage[kind]; }
}
