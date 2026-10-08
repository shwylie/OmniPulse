// The now-playing widget for OBS.

// ---------- now playing widget for OBS ----------
const WIDGET_NOTE = "A now-playing card for OBS.";

let widgetReady = false;
let widgetCleared = false;   // "Clear" hides the card until the next track starts

function widgetState() {
  const t = curTrack();
  // the look and the alerts' settings go along even with nothing playing (alerts still show)
  const look = {
    accent: accentNow[0],
    accent2: accentNow[1],
    design: prefs.widgetDesign || "card",
    rounded: prefs.widgetRounded !== false,
    text_scale: prefs.widgetText || 1,
    alerts_cfg: alertsCfg(),
  };
  if (!t || widgetCleared) return Object.assign({ playing: false }, look);
  return {
    alerts_cfg: look.alerts_cfg,
    playing: nowPlaying(),
    title: t.title || "",
    artist: artistName(t),
    artwork: artUrl(t, "t500x500") || "",
    url: t.permalink_url || "",
    platform: srcOf(t),
    duration_ms: viewDur(t, t.duration || curDurMs()),
    position_ms: viewPos(t, curPosMs()),
    updated_at: Date.now(),
    accent: accentNow[0],
    accent2: accentNow[1],
    design: prefs.widgetDesign || "card",
    rounded: prefs.widgetRounded !== false,
    text_scale: prefs.widgetText || 1,
    anim: {
      fade: prefs.wFade !== false,
      scroll: prefs.wScroll !== false,
      spin: prefs.wSpin !== false,
      color: prefs.wColor === undefined ? 1.2 : Number(prefs.wColor),
    },
  };
}

function widgetPush() {
  hostPush();
  if (prefs.widget || prefs.req) invoke("widget_update", { state: widgetState() }).catch(() => {});
}

async function widgetApply() {
  const on = !!prefs.widget;
  $("widgetSection").hidden = !on;
  if (!on) {
    invoke("widget_stop").catch(() => {});
    $("widgetNote").textContent = WIDGET_NOTE;
    return;
  }
  try {
    const url = await invoke("widget_start", { port: Number(prefs.widgetPort) || 7878 });
    $("widgetUrl").value = url;
    alertsPaint();   // its web request address goes by the same port
    $("widgetNote").textContent = WIDGET_NOTE;
    widgetPush();
  } catch (e) {
    $("widgetUrl").value = "";
    $("widgetNote").textContent = String(e);
  }
}

// Animation options that only some designs use (the rest, fades, scrolling and colour speed, apply
// to every design). Options for other designs are hidden.
const WIDGET_ANIM_DESIGNS = { wSpin: ["vinyl"] };

function paintWidgetAnims() {
  const d = prefs.widgetDesign || "card";
  Object.entries(WIDGET_ANIM_DESIGNS).forEach(([id, designs]) => { $(id).closest(".row").hidden = !designs.includes(d); });
}

function widgetInit() {
  $("widgetToggle").checked = !!prefs.widget;
  $("widgetPort").value = prefs.widgetPort || 7878;
  $("widgetDesign").value = prefs.widgetDesign || "card";
  $("widgetRounded").checked = prefs.widgetRounded !== false;
  $("widgetDesign").addEventListener("change", () => {
    prefs.widgetDesign = $("widgetDesign").value;
    savePrefs();
    paintWidgetAnims();
    widgetPush();
  });
  $("widgetRounded").addEventListener("change", () => {
    prefs.widgetRounded = $("widgetRounded").checked;
    savePrefs();
    widgetPush();
  });
  $("widgetToggle").addEventListener("change", () => {
    prefs.widget = $("widgetToggle").checked;
    savePrefs();
    widgetApply();
  });
  $("widgetPort").addEventListener("change", () => {
    const p = Math.round(Number($("widgetPort").value));
    if (p >= 1024 && p <= 65535) { prefs.widgetPort = p; savePrefs(); widgetApply(); }
    else $("widgetPort").value = prefs.widgetPort || 7878;
  });
  $("widgetClear").addEventListener("click", () => {
    widgetCleared = true;
    widgetPush();
  });
  $("widgetCopy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("widgetUrl").value);
      $("widgetCopy").textContent = "Copied";
      setTimeout(() => { $("widgetCopy").textContent = "Copy"; }, 1500);
    } catch (e) {
      $("widgetUrl").select();
    }
  });
  audio.addEventListener("seeked", widgetPush);   // Spotify seeks push from spotifySeek
  setInterval(() => { if ((prefs.widget || prefs.req) && nowPlaying()) widgetPush(); }, 5000);
  const paintText = () => { $("widgetTextVal").textContent = Math.round((prefs.widgetText || 1) * 100) + "%"; };
  const stepText = (d) => {
    prefs.widgetText = Math.min(1.6, Math.max(0.7, Math.round(((prefs.widgetText || 1) + d) * 10) / 10));
    savePrefs();
    paintText();
    widgetPush();
  };
  $("widgetTextDown").addEventListener("click", () => stepText(-0.1));
  $("widgetTextUp").addEventListener("click", () => stepText(0.1));
  paintText();

  // Animation options
  paintWidgetAnims();
  $("wFade").checked = prefs.wFade !== false;
  $("wScroll").checked = prefs.wScroll !== false;
  $("wSpin").checked = prefs.wSpin !== false;
  $("wColor").value = String(prefs.wColor === undefined ? 1.2 : prefs.wColor);
  ["wFade", "wScroll", "wSpin"].forEach((key) =>
    $(key).addEventListener("change", () => { prefs[key] = $(key).checked; savePrefs(); widgetPush(); }));
  $("wColor").addEventListener("change", () => { prefs.wColor = Number($("wColor").value); savePrefs(); widgetPush(); });

  alertsInit();
  widgetReady = true;
  widgetApply();
}

// ---------- stream alerts, shown by the widget (crates/core/widget/widget.html) ----------
// From Streamer.bot (the widget connects to its WebSocket server) or web requests to /alert (Mix It
// Up's Web Request action, Streamer.bot's Fetch URL...). Each kind can be off, have a sound, and be
// tried with Test; dragging sets which goes first when several are waiting.
const ALERT_KINDS = [
  ["follow", "Follows", { user: "A new friend" }],
  ["sub", "Subscriptions", { user: "A loyal viewer", amount: "6", message: "Six months already!" }],
  ["gift", "Gifted Subs", { user: "A generous viewer", amount: "5" }],
  ["bits", "Bits", { user: "A cheerful viewer", amount: "500", message: "gg!" }],
  ["donation", "Donations", { user: "A kind viewer", amount: "$10.00", message: "Love the stream" }],
  ["hypetrain", "Hype Trains", { amount: "3" }],
  ["poll", "Polls", { user: "Which track next?", message: "Vote now!" }],
  ["ban", "Bans", { user: "A troll" }],
  ["watchstreak", "Watch Streaks", { user: "A regular", amount: "10" }],
];
const ALERT_SOUND_EXTS = ["mp3", "wav", "ogg", "m4a", "aac", "flac"];

const alertsOn = () => !!prefs.alertsOn;
function alertOrder() {
  const all = ALERT_KINDS.map((k) => k[0]);
  const saved = (prefs.alertOrder || []).filter((k) => all.includes(k));
  return saved.concat(all.filter((k) => !saved.includes(k)));
}

// What the widget needs: the switched-on kinds in priority order, the look of them, and their sounds
function alertsCfg() {
  if (!alertsOn()) return { on: false };
  const off = new Set(prefs.alertOff || []);
  const sounds = {};
  Object.entries(prefs.alertSounds || {}).forEach(([k, v]) => { if (v && v.at) sounds[k] = String(v.at); });
  return {
    on: true,
    order: alertOrder().filter((k) => !off.has(k)),
    anim: prefs.alertAnim || "slide",
    secs: Number(prefs.alertSecs) || 5,
    volume: prefs.alertVol === undefined ? 0.8 : Number(prefs.alertVol),
    sounds,
    sb: { on: prefs.sbOn !== false, host: prefs.sbHost || "127.0.0.1:8080" },
  };
}

function alertsSave() {
  savePrefs();
  widgetPush();
}

function alertRow(kind, name) {
  const off = new Set(prefs.alertOff || []);
  const snd = (prefs.alertSounds || {})[kind];
  const li = el("li", "alertRow" + (off.has(kind) ? " alertIsOff" : ""));
  li.draggable = true;
  li.dataset.kind = kind;
  const grip = el("span", "alertGrip", "☰");
  grip.title = "Drag to reorder";
  const label = el("span", "alertName", name);
  const sound = el("button", "soft alertSound", snd ? snd.name : "Add Sound");
  sound.title = snd ? "Change the sound" : "Pick a sound to play with this alert";
  sound.addEventListener("click", () => alertPickSound(kind));
  const parts = [grip, label, sound];
  if (snd) {
    const clear = el("button", "soft alertClear", "×");
    clear.title = "No sound";
    clear.setAttribute("aria-label", "Remove the sound");
    clear.addEventListener("click", async () => {
      await invoke("alert_sound_clear", { kind }).catch(() => {});
      delete prefs.alertSounds[kind];
      alertsSave();
      alertsPaintList();
    });
    parts.push(clear);
  }
  const test = el("button", "soft", "Test");
  test.title = "Show this alert on the widget now";
  test.addEventListener("click", () => alertTest(kind));
  const sw = el("label", "switch");
  const box = el("input");
  box.type = "checkbox";
  box.checked = !off.has(kind);
  box.setAttribute("aria-label", name);
  box.addEventListener("change", () => {
    const s = new Set(prefs.alertOff || []);
    if (box.checked) s.delete(kind); else s.add(kind);
    prefs.alertOff = [...s];
    li.classList.toggle("alertIsOff", !box.checked);
    alertsSave();
  });
  sw.append(box, el("span"));
  parts.push(test, sw);
  li.append(...parts);
  return li;
}

function alertsPaintList() {
  const names = Object.fromEntries(ALERT_KINDS.map((k) => [k[0], k[1]]));
  $("alertList").replaceChildren(...alertOrder().map((k) => alertRow(k, names[k])));
}

async function alertPickSound(kind) {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg) { say("Picking a sound needs the app."); return; }
  try {
    const path = await dlg.open({ multiple: false, title: "Choose a sound for this alert", filters: [{ name: "Sounds", extensions: ALERT_SOUND_EXTS }] });
    if (!path) return;
    const name = await invoke("alert_sound_set", { kind, path: String(path) });
    prefs.alertSounds = Object.assign({}, prefs.alertSounds, { [kind]: { name, at: Date.now() } });
    alertsSave();
    alertsPaintList();
  } catch (e) {
    say(String(e && e.message ? e.message : e));
  }
}

async function alertTest(kind) {
  if (!prefs.widget) { say("Switch on the widget first."); return; }
  const sample = (ALERT_KINDS.find((k) => k[0] === kind) || [])[2] || {};
  try {
    await invoke("widget_alert", { alert: Object.assign({ type: kind, user: "", amount: "", message: "" }, sample, { test: true }) });
    say("Sent a test alert to the widget.");
  } catch (e) {
    say(String(e));
  }
}

// Dragging a row to reorder them
function alertsDrag() {
  const list = $("alertList");
  let dragging = null;
  list.addEventListener("dragstart", (e) => {
    dragging = e.target.closest(".alertRow");
    if (!dragging) return;
    dragging.classList.add("dragging");
    e.dataTransfer.effectAllowed = "move";
    e.dataTransfer.setData("text/plain", dragging.dataset.kind);
  });
  list.addEventListener("dragover", (e) => {
    if (!dragging) return;
    e.preventDefault();
    const rows = [...list.querySelectorAll(".alertRow:not(.dragging)")];
    const after = rows.find((r) => { const b = r.getBoundingClientRect(); return e.clientY < b.top + b.height / 2; });
    if (after) list.insertBefore(dragging, after); else list.append(dragging);
  });
  const done = () => {
    if (!dragging) return;
    dragging.classList.remove("dragging");
    dragging = null;
    prefs.alertOrder = [...list.querySelectorAll(".alertRow")].map((r) => r.dataset.kind);
    alertsSave();
  };
  list.addEventListener("drop", (e) => { e.preventDefault(); done(); });
  list.addEventListener("dragend", done);
}

function alertsPaint() {
  $("alertsOn").checked = alertsOn();
  $("alertsBody").hidden = !alertsOn();
  $("sbOn").checked = prefs.sbOn !== false;
  $("sbHost").value = prefs.sbHost || "127.0.0.1:8080";
  $("alertAnim").value = prefs.alertAnim || "slide";
  $("alertSecs").value = String(Number(prefs.alertSecs) || 5);
  const v = prefs.alertVol === undefined ? 0.8 : Number(prefs.alertVol);
  $("alertVol").value = String(Math.round(v * 100));
  $("alertVolVal").textContent = Math.round(v * 100) + "%";
  const base = ($("widgetUrl").value || "http://127.0.0.1:" + (Number(prefs.widgetPort) || 7878) + "/").replace(/\/$/, "");
  $("alertUrl").value = base + "/alert?type=follow&user=NAME&amount=&message=";
  if (typeof stateNotes === "function") stateNotes();
}

function alertsInit() {
  $("alertsOn").addEventListener("change", () => { prefs.alertsOn = $("alertsOn").checked; alertsSave(); alertsPaint(); });
  $("sbOn").addEventListener("change", () => { prefs.sbOn = $("sbOn").checked; alertsSave(); });
  $("sbHost").addEventListener("change", () => {
    prefs.sbHost = $("sbHost").value.trim().replace(/^wss?:\/\//, "").replace(/\/+$/, "") || "127.0.0.1:8080";
    $("sbHost").value = prefs.sbHost;
    alertsSave();
  });
  $("alertAnim").addEventListener("change", () => { prefs.alertAnim = $("alertAnim").value; alertsSave(); });
  $("alertSecs").addEventListener("change", () => { prefs.alertSecs = Number($("alertSecs").value); alertsSave(); });
  $("alertVol").addEventListener("input", () => {
    prefs.alertVol = Number($("alertVol").value) / 100;
    $("alertVolVal").textContent = $("alertVol").value + "%";
  });
  $("alertVol").addEventListener("change", alertsSave);
  $("alertCopy").addEventListener("click", async () => {
    try {
      await navigator.clipboard.writeText($("alertUrl").value);
      $("alertCopy").textContent = "Copied";
      setTimeout(() => { $("alertCopy").textContent = "Copy"; }, 1500);
    } catch (e) {
      $("alertUrl").select();
    }
  });
  alertsDrag();
  alertsPaintList();
  alertsPaint();
}
