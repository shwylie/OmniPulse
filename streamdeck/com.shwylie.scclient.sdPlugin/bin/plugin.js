// The OmniPulse Stream Deck plugin. Each key calls the app's remote control (Settings >
// Streaming > Stream Deck & Remote in OmniPulse, served on 127.0.0.1 by src-tauri/src/remote.rs). The app writes its port and key
// to %APPDATA%\OmniPulse\remote.json while remote control is on, so there's nothing to set up here.
// Toggle keys (play/pause, like, favourite, shuffle, repeat, sets & podcasts) show the app's state.
// The Play / Pause keys show the cover of what's playing (each key's settings: ui/art.html).

import streamDeck from "@elgato/streamdeck";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const PREFIX = "com.shwylie.scclient.";
const INFOS = ["OmniPulse", "sc-client"].map((d) => join(process.env.APPDATA || "", d, "remote.json"));
// the app's remote.json: in its folder, or the folder older versions used
async function readInfo() {
  for (const f of INFOS) { try { return await readFile(f, "utf8"); } catch (e) { /* try the next */ } }
  throw new Error("OmniPulse isn't running with Stream Deck control on");
}
const POLL_MS = 2000;

// which state each toggle key shows (1 = on), from the app's state
const STATES = {
  toggle: (s) => s.playing,
  like: (s) => s.liked,
  favourite: (s) => s.favourite,
  shuffle: (s) => s.shuffle,
  repeat: (s) => s.repeat && s.repeat !== "off",
  sets: (s) => s.setsAllowed,
};
// keys that can show the cover
const ART_KEYS = new Set(["toggle", "play", "pause"]);

const visible = new Map();    // keys on screen: action id -> action
const settings = new Map();   // action id -> its settings ({ art, sign }: both on unless switched off)
const shown = new Map();      // action id -> the cover image it shows ("" for its own image)
let last = null;

async function call(action) {
  let info;
  try {
    info = JSON.parse(await readInfo());
  } catch {
    throw new Error("OmniPulse isn't running, or Stream Deck is switched off in its Settings.");
  }
  const path = action ? `control?action=${encodeURIComponent(action)}&` : "state?";
  const res = await fetch(`http://127.0.0.1:${info.port}/${path}key=${encodeURIComponent(info.key)}`, { signal: AbortSignal.timeout(6000) });
  const state = await res.json();
  if (!res.ok || !state.ok) throw new Error(state.message || `OmniPulse said ${res.status}`);
  return state;
}

const nameOf = (action) => action.manifestId.slice(PREFIX.length);
const artOn = (id) => (settings.get(id) || {}).art !== false;
const signOn = (id) => (settings.get(id) || {}).sign !== false;

// ---------- covers ----------
const covers = new Map();   // its address -> a data URL, or null (it couldn't be had)
const fetching = new Set();
async function coverFetch(url) {
  if (covers.has(url) || fetching.has(url)) return;
  fetching.add(url);
  try {
    const res = await fetch(url, { signal: AbortSignal.timeout(8000) });
    if (!res.ok) throw new Error(String(res.status));
    const type = (res.headers.get("content-type") || "image/jpeg").split(";")[0];
    covers.set(url, `data:${type};base64,${Buffer.from(await res.arrayBuffer()).toString("base64")}`);
  } catch (e) {
    covers.set(url, null);
  } finally {
    fetching.delete(url);
  }
  // keep a few
  while (covers.size > 24) covers.delete(covers.keys().next().value);
  if (last) paint(last);
}

const ICON = {
  play: "M8 5v14l11-7z",
  pause: "M6 19h4V5H6v14zm8-14v14h4V5h-4z",
};
// The cover on a key: dimmed while paused, and (if wanted) the key's sign in the corner, in the app
// icon's colours
function coverKey(cover, sign, dim) {
  const svg = `<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="144" height="144" viewBox="0 0 144 144">` +
    `<defs><clipPath id="c"><rect width="144" height="144" rx="22"/></clipPath>` +
    `<linearGradient id="on" x1="0" y1="1" x2="1" y2="0"><stop offset="0" stop-color="#18e0ff"/><stop offset="1" stop-color="#ff2bd6"/></linearGradient></defs>` +
    `<g clip-path="url(#c)"><rect width="144" height="144" fill="#1b1e26"/>` +
    `<image width="144" height="144" preserveAspectRatio="xMidYMid slice" href="${cover}" xlink:href="${cover}"/>` +
    (dim ? `<rect width="144" height="144" fill="#000000" opacity="0.45"/>` : "") + `</g>` +
    (sign ? `<circle cx="110" cy="110" r="30" fill="url(#on)"/><path transform="translate(93.2 93.2) scale(1.4)" d="${ICON[sign]}" fill="#ffffff"/>` : "") +
    `</svg>`;
  return `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;
}

function paintCover(action, state) {
  const id = action.id, name = nameOf(action);
  let image = "";
  if (state && state.art && artOn(id)) {
    const cover = covers.get(state.art);
    if (cover === undefined) coverFetch(state.art);
    if (cover) {
      // the sign: what the key does (Play / Pause: what a press would do now)
      const sign = !signOn(id) ? "" : name === "toggle" ? (state.playing ? "pause" : "play") : name;
      image = coverKey(cover, sign, !state.playing);
    }
  }
  if (shown.get(id) === image) return;
  shown.set(id, image);
  // (nothing: back to the key's own image)
  action.setImage(image || undefined).catch(() => {});
}

function paint(state) {
  last = state;
  for (const action of visible.values()) {
    if (!action.isKey()) continue;
    const name = nameOf(action);
    const show = STATES[name];
    if (show) action.setState(state && show(state) ? 1 : 0).catch(() => {});
    if (ART_KEYS.has(name)) paintCover(action, state);
  }
}

streamDeck.actions.onWillAppear((ev) => {
  visible.set(ev.action.id, ev.action);
  settings.set(ev.action.id, ev.payload.settings || {});
  shown.delete(ev.action.id);
  if (last) paint(last);
});
streamDeck.actions.onWillDisappear((ev) => {
  visible.delete(ev.action.id);
  shown.delete(ev.action.id);
});
streamDeck.settings.onDidReceiveSettings((ev) => {
  settings.set(ev.action.id, ev.payload.settings || {});
  if (last) paint(last);
});

streamDeck.actions.onKeyDown(async (ev) => {
  const name = nameOf(ev.action);
  try {
    paint(await call(name));
    if (!STATES[name]) await ev.action.showOk();   // toggles show their new state instead
  } catch (e) {
    streamDeck.logger.warn(`${name}: ${e.message}`);
    await ev.action.showAlert();
  }
});

// keep the toggle keys and covers in step with the app (only while there are any on screen)
setInterval(async () => {
  if (![...visible.values()].some((a) => STATES[nameOf(a)] || (ART_KEYS.has(nameOf(a)) && artOn(a.id)))) return;
  try { paint(await call(null)); } catch { paint(null); }
}, POLL_MS);

streamDeck.connect();
