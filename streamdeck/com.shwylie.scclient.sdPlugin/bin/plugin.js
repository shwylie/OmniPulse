// The SC Client Stream Deck plugin. Each key calls the app's remote control (Settings > Stream Deck
// in SC Client, served on 127.0.0.1 by src-tauri/src/remote.rs). The app writes its port and key
// to %APPDATA%\sc-client\remote.json while remote control is on, so there's nothing to set up here.
// Toggle keys (play/pause, like, favourite, shuffle, repeat, sets & podcasts) show the app's state.

import streamDeck from "@elgato/streamdeck";
import { readFile } from "node:fs/promises";
import { join } from "node:path";

const PREFIX = "com.shwylie.scclient.";
const INFO = join(process.env.APPDATA || "", "sc-client", "remote.json");
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

const visible = new Map();   // keys on screen: action id -> action
let last = null;

async function call(action) {
  let info;
  try {
    info = JSON.parse(await readFile(INFO, "utf8"));
  } catch {
    throw new Error("SC Client isn't running, or Stream Deck is switched off in its Settings.");
  }
  const path = action ? `control?action=${encodeURIComponent(action)}&` : "state?";
  const res = await fetch(`http://127.0.0.1:${info.port}/${path}key=${encodeURIComponent(info.key)}`, { signal: AbortSignal.timeout(6000) });
  const state = await res.json();
  if (!res.ok || !state.ok) throw new Error(state.message || `SC Client said ${res.status}`);
  return state;
}

const nameOf = (action) => action.manifestId.slice(PREFIX.length);

function paint(state) {
  last = state;
  for (const action of visible.values()) {
    const show = STATES[nameOf(action)];
    if (show && action.isKey()) action.setState(state && show(state) ? 1 : 0).catch(() => {});
  }
}

streamDeck.actions.onWillAppear((ev) => {
  visible.set(ev.action.id, ev.action);
  if (last) paint(last);
});
streamDeck.actions.onWillDisappear((ev) => visible.delete(ev.action.id));

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

// keep the toggle keys in step with the app (only while there are any on screen)
setInterval(async () => {
  if (![...visible.values()].some((a) => STATES[nameOf(a)])) return;
  try { paint(await call(null)); } catch { paint(null); }
}, POLL_MS);

streamDeck.connect();
