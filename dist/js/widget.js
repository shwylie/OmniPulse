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
    align: prefs.widgetAlign === "right" ? "right" : "left",
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
    align: look.align,
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
    if (prefs.botRemote && !prefs.alertKey) { prefs.alertKey = reqKeyNew() + reqKeyNew(); savePrefs(); }
    const url = await invoke("widget_start", { port: Number(prefs.widgetPort) || 7878, remote: !!prefs.botRemote && alertsOn(), key: prefs.alertKey || "" });
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
  // Alignment: left or right
  const paintAlign = () => document.querySelectorAll("#widgetAlignSeg button").forEach((b) => b.classList.toggle("on", b.dataset.v === (prefs.widgetAlign === "right" ? "right" : "left")));
  $("widgetAlignSeg").addEventListener("click", (e) => {
    const b = e.target.closest("button");
    if (!b) return;
    prefs.widgetAlign = b.dataset.v;
    savePrefs();
    paintAlign();
    widgetPush();
  });
  paintAlign();
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
// tried with Test; dragging sets which goes first when several are waiting. The pencil by a name
// opens its own settings: its name and words, colour, length, animation, icon or picture. + adds a
// custom alert, set off by a web request with its trigger name.
//   [id, name in Settings, test values, heading, main line, icon]
const ALERT_KINDS = [
  ["follow", "Follows", { user: "A new friend" }, "New Follower", "{user}"],
  ["sub", "Subscriptions", { user: "A loyal viewer", amount: "6", message: "Six months already!" }, "New Subscriber", "{user}"],
  ["gift", "Gifted Subs", { user: "A generous viewer", amount: "5" }, "Gifted Subs", "{user} gifted {amount} subs"],
  ["bits", "Bits", { user: "A cheerful viewer", amount: "500", message: "gg!" }, "{amount} Bits", "{user}"],
  ["kicks", "Kicks", { user: "A kind viewer", amount: "100", message: "Love it" }, "{amount} Kicks", "{user}"],
  ["donation", "Donations", { user: "A kind viewer", amount: "$10.00", message: "Love the stream" }, "Donation", "{user} · {amount}"],
  ["raid", "Raids & Hosts", { user: "A fellow streamer", amount: "42" }, "{amount} Raiders", "{user} is here!"],
  ["hypetrain", "Hype Trains", { amount: "3" }, "Hype Train", "Level {amount}"],
  ["poll", "Polls", { user: "Which track next?", message: "Vote now!" }, "Poll", "{user}"],
  ["ban", "Bans", { user: "A troll" }, "Banned", "{user}"],
  ["watchstreak", "Watch Streaks", { user: "A regular", amount: "10" }, "Watch Streak", "{user} · {amount} streams"],
];
const ALERT_ICON_NAMES = [
  ["follow", "Heart"], ["sub", "Star"], ["gift", "Gift"], ["bits", "Gem"], ["kicks", "Lightning"], ["donation", "Money"],
  ["raid", "People"], ["hypetrain", "Train"], ["poll", "Chart"], ["ban", "No Entry"], ["watchstreak", "Flame"],
  ["bell", "Bell"], ["trophy", "Trophy"], ["hammer", "Hammer"], ["rocket", "Rocket"], ["music", "Music"], ["sparkle", "Sparkles"],
];
const ALERT_SOUND_EXTS = ["mp3", "wav", "ogg", "m4a", "aac", "flac"];
const ALERT_IMAGE_EXTS = ["gif", "png", "webp", "jpg", "jpeg"];
const ALERT_SECS = [3, 5, 8, 12, 20];
const ALERT_ANIMS = [["slide", "Slide"], ["pop", "Pop"], ["fade", "Fade"], ["bounce", "Bounce"], ["flip", "Flip"]];

const alertsOn = () => !!prefs.alertsOn;
const alertBuiltin = (k) => ALERT_KINDS.find((x) => x[0] === k);
const alertCustom = () => (prefs.alertCustom || []).filter((c) => c && c.id);
const alertEdits = (k) => (prefs.alertEdit || {})[k] || {};
function alertName(k) {
  const b = alertBuiltin(k), c = alertCustom().find((x) => x.id === k);
  return alertEdits(k).name || (b ? b[1] : c ? c.name : k);
}
function alertOrder() {
  const all = ALERT_KINDS.map((k) => k[0]).concat(alertCustom().map((c) => c.id));
  const saved = (prefs.alertOrder || []).filter((k) => all.includes(k));
  return saved.concat(all.filter((k) => !saved.includes(k)));
}

// What the widget needs: the switched-on kinds in priority order, how each looks, and their sounds
function alertsCfg() {
  if (!alertsOn()) return { on: false };
  const off = new Set(prefs.alertOff || []);
  const sounds = {}, kinds = {};
  Object.entries(prefs.alertSounds || {}).forEach(([k, v]) => { if (v && v.at) sounds[k] = String(v.at); });
  alertOrder().forEach((k) => {
    const e = alertEdits(k), img = (prefs.alertImages || {})[k];
    const c = {};
    ["head", "main", "color", "anim", "icon"].forEach((f) => { if (e[f]) c[f] = e[f]; });
    if (e.secs) c.secs = Number(e.secs);
    if (img && img.at) c.image = String(img.at);
    if (!alertBuiltin(k)) c.name = alertName(k);
    if (Object.keys(c).length) kinds[k] = c;
  });
  return {
    on: true,
    order: alertOrder().filter((k) => !off.has(k)),
    custom: alertCustom().map((c) => c.id),
    kinds,
    anim: prefs.alertAnim || "slide",
    transition: prefs.alertTrans || "seamless",
    secs: Number(prefs.alertSecs) || 5,
    // their own colours (else the widget's, from the album art), and which way the gradient runs
    accent: prefs.alertOwnCol ? prefs.alertCol1 || "#8e9bff" : "",
    accent2: prefs.alertOwnCol ? prefs.alertCol2 || prefs.alertCol1 || "#b38cff" : "",
    dir: prefs.alertDir || "115deg",
    icon_scale: alertIconScale(),
    volume: prefs.alertVol === undefined ? 0.8 : Number(prefs.alertVol),
    sounds,
    sb: { on: prefs.sbOn !== false, host: prefs.sbHost || "127.0.0.1:8080" },
  };
}

// The icons' size: 60% to 200% of how they started (200% fills the tile), in steps of 10%
const alertIconScale = () => Math.min(2, Math.max(0.6, Number(prefs.alertIcon) || 1));

function alertsSave() {
  savePrefs();
  widgetPush();
}

function alertEditSet(kind, field, value) {
  const all = Object.assign({}, prefs.alertEdit);
  const e = Object.assign({}, all[kind]);
  if (value === "" || value === null || value === undefined) delete e[field]; else e[field] = value;
  all[kind] = e;
  prefs.alertEdit = all;
  alertsSave();
}

let alertOpen = null;   // the alert whose settings are open

function alertRow(kind) {
  const off = new Set(prefs.alertOff || []);
  const snd = (prefs.alertSounds || {})[kind];
  const li = el("li", "alertRow" + (off.has(kind) ? " alertIsOff" : ""));
  li.draggable = true;
  li.dataset.kind = kind;
  const grip = el("span", "alertGrip", "☰");
  grip.title = "Drag to reorder";
  const label = el("span", "alertName");
  label.append(el("span", "", alertName(kind)));
  const pen = el("button", "alertPen");
  pen.title = "Edit this alert";
  pen.setAttribute("aria-label", "Edit " + alertName(kind));
  pen.append(icon("M3 17.25V21h3.75L17.81 9.94l-3.75-3.75L3 17.25zM20.71 7.04a1 1 0 0 0 0-1.41l-2.34-2.34a1 1 0 0 0-1.41 0l-1.83 1.83 3.75 3.75 1.83-1.83z"));
  pen.addEventListener("click", () => { alertOpen = alertOpen === kind ? null : kind; alertsPaintList(); });
  label.append(pen);
  const sound = el("button", "soft alertSound", snd ? snd.name : "Add Sound");
  sound.title = snd ? "Change the sound" : "Pick a sound to play with this alert";
  sound.addEventListener("click", () => alertPickMedia(kind, "sound"));
  const parts = [grip, label, sound];
  if (snd) {
    const clear = el("button", "soft alertClear", "×");
    clear.title = "No sound";
    clear.setAttribute("aria-label", "Remove the sound");
    clear.addEventListener("click", () => alertClearMedia(kind, "sound"));
    parts.push(clear);
  }
  const test = el("button", "soft", "Test");
  test.title = "Show this alert on the widget now";
  test.addEventListener("click", () => alertTest(kind));
  const sw = el("label", "switch");
  const box = el("input");
  box.type = "checkbox";
  box.checked = !off.has(kind);
  box.setAttribute("aria-label", alertName(kind));
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

// ---------- one alert's own settings, under its row ----------
function alertEditor(kind) {
  const b = alertBuiltin(kind);
  const custom = !b;
  const e = alertEdits(kind);
  const box = el("li", "alertEdit");
  box.dataset.for = kind;
  const field = (label, input, note) => {
    const r = el("div", "alertField");
    r.append(el("label", "", label), input);
    if (note) r.append(el("div", "note", note));
    return r;
  };
  const text = (value, placeholder, onChange) => {
    const i = el("input");
    i.type = "text";
    i.value = value || "";
    i.placeholder = placeholder || "";
    i.spellcheck = false;
    i.addEventListener("change", () => onChange(i.value.trim()));
    return i;
  };
  const select = (opts, value, onChange) => {
    const s = el("select", "narrow");
    opts.forEach(([v, n]) => { const o = el("option", "", n); o.value = v; s.append(o); });
    s.value = value;
    s.addEventListener("change", () => onChange(s.value));
    return s;
  };
  const defName = b ? b[1] : (alertCustom().find((c) => c.id === kind) || {}).name || "";
  box.append(field("Name", text(custom ? alertName(kind) : e.name, defName, (v) => {
    if (custom) {
      prefs.alertCustom = alertCustom().map((c) => (c.id === kind ? Object.assign({}, c, { name: v || c.name }) : c));
      alertsSave();
    } else alertEditSet(kind, "name", v);
    alertsPaintList();
  })));
  if (custom) {
    const url = ($("alertUrl").value || "").replace(/type=[^&]*/, "type=" + kind);
    const trig = text(kind, "", (v) => alertRename(kind, v));
    box.append(field("Trigger", trig, "Its web request: " + url));
  }
  box.append(
    field("Heading", text(e.head, b ? b[3] : alertName(kind), (v) => alertEditSet(kind, "head", v))),
    field("Main Line", text(e.main, b ? b[4] : "{user}", (v) => alertEditSet(kind, "main", v)), "{user}, {amount} and {message} are filled in. Empty: as shown faintly."),
  );
  // colour: its own, or the widget's
  const colRow = el("div", "alertInline");
  const own = el("input");
  own.type = "checkbox";
  own.checked = !!e.color;
  const col = el("input");
  col.type = "color";
  col.value = e.color || "#8e9bff";
  col.disabled = !e.color;
  own.addEventListener("change", () => { col.disabled = !own.checked; alertEditSet(kind, "color", own.checked ? col.value : ""); });
  col.addEventListener("change", () => alertEditSet(kind, "color", col.value));
  const ownLbl = el("label", "alertCheck");
  ownLbl.append(own, document.createTextNode(" Its own colour"));
  colRow.append(ownLbl, col);
  box.append(field("Colour", colRow));
  box.append(
    field("Show For", select([["", "As set above"]].concat(ALERT_SECS.map((s) => [String(s), s + " s"])), String(e.secs || ""), (v) => alertEditSet(kind, "secs", v ? Number(v) : ""))),
    field("Animation", select([["", "As set above"]].concat(ALERT_ANIMS), e.anim || "", (v) => alertEditSet(kind, "anim", v))),
  );
  // the picture: an icon, or your own image or GIF
  const img = (prefs.alertImages || {})[kind];
  const picRow = el("div", "alertInline");
  if (img) {
    picRow.append(el("span", "alertFile", img.name));
    const change = el("button", "soft", "Change");
    change.addEventListener("click", () => alertPickMedia(kind, "image"));
    const rm = el("button", "soft", "Use an Icon");
    rm.addEventListener("click", () => alertClearMedia(kind, "image"));
    picRow.append(change, rm);
  } else {
    picRow.append(select(ALERT_ICON_NAMES, e.icon || (b ? kind : "bell"), (v) => alertEditSet(kind, "icon", v === (b ? kind : "bell") ? "" : v)));
    const pick = el("button", "soft", "Image or GIF...");
    pick.title = "Show your own picture (a GIF, say) in place of the icon";
    pick.addEventListener("click", () => alertPickMedia(kind, "image"));
    picRow.append(pick);
  }
  box.append(field("Picture", picRow));
  const foot = el("div", "alertInline alertFoot");
  if (custom) {
    const del = el("button", "soft danger", "Delete This Alert");
    del.addEventListener("click", () => alertDelete(kind));
    foot.append(del);
  } else if (Object.keys(e).length) {
    const reset = el("button", "soft", "Back to How It Was");
    reset.addEventListener("click", () => {
      const all = Object.assign({}, prefs.alertEdit);
      delete all[kind];
      prefs.alertEdit = all;
      alertsSave();
      alertsPaintList();
    });
    foot.append(reset);
  }
  const done = el("button", "soft", "Done");
  done.addEventListener("click", () => { alertOpen = null; alertsPaintList(); });
  foot.append(done);
  box.append(foot);
  return box;
}

// A trigger name: what a web request's type= says (letters, digits, - and _)
const alertSlug = (s) => String(s).toLowerCase().normalize("NFKD").replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 32);

function alertRename(kind, v) {
  const id = alertSlug(v);
  if (!id || id === kind) { alertsPaintList(); return; }
  if (alertBuiltin(id) || alertCustom().some((c) => c.id === id)) { say("\"" + id + "\" is already an alert's trigger."); alertsPaintList(); return; }
  const move = (o) => { if (o && o[kind] !== undefined) { o = Object.assign({}, o); o[id] = o[kind]; delete o[kind]; } return o; };
  prefs.alertCustom = alertCustom().map((c) => (c.id === kind ? Object.assign({}, c, { id }) : c));
  prefs.alertEdit = move(prefs.alertEdit);
  prefs.alertOrder = (prefs.alertOrder || []).map((k) => (k === kind ? id : k));
  prefs.alertOff = (prefs.alertOff || []).map((k) => (k === kind ? id : k));
  // its sound and picture are files named after the trigger: those are picked again
  ["sound", "image"].forEach((slot) => {
    const key = slot === "sound" ? "alertSounds" : "alertImages";
    if (prefs[key] && prefs[key][kind]) { invoke("alert_media_clear", { kind, slot }).catch(() => {}); prefs[key] = Object.assign({}, prefs[key]); delete prefs[key][kind]; }
  });
  alertOpen = id;
  alertsSave();
  alertsPaintList();
}

function alertAdd() {
  let n = 1, id = "custom";
  while (alertBuiltin(id) || alertCustom().some((c) => c.id === id)) id = "custom-" + ++n;
  prefs.alertCustom = alertCustom().concat([{ id, name: "New Alert" + (n > 1 ? " " + n : "") }]);
  alertOpen = id;
  alertsSave();
  alertsPaintList();
  const ed = document.querySelector('#alertList .alertEdit[data-for="' + id + '"]');
  if (ed) { ed.scrollIntoView({ block: "nearest" }); const first = ed.querySelector("input"); if (first) { first.focus(); first.select(); } }
}

function alertDelete(kind) {
  prefs.alertCustom = alertCustom().filter((c) => c.id !== kind);
  ["alertEdit", "alertSounds", "alertImages"].forEach((key) => { if (prefs[key] && prefs[key][kind]) { prefs[key] = Object.assign({}, prefs[key]); delete prefs[key][kind]; } });
  prefs.alertOrder = (prefs.alertOrder || []).filter((k) => k !== kind);
  prefs.alertOff = (prefs.alertOff || []).filter((k) => k !== kind);
  ["sound", "image"].forEach((slot) => invoke("alert_media_clear", { kind, slot }).catch(() => {}));
  alertOpen = null;
  alertsSave();
  alertsPaintList();
}

function alertsPaintList() {
  const items = [];
  alertOrder().forEach((k) => {
    items.push(alertRow(k));
    if (alertOpen === k) items.push(alertEditor(k));
  });
  $("alertList").replaceChildren(...items);
}

async function alertPickMedia(kind, slot) {
  const dlg = window.__TAURI__ && window.__TAURI__.dialog;
  if (!dlg) { say("Picking a file needs the app."); return; }
  const image = slot === "image";
  try {
    const path = await dlg.open({ multiple: false, title: image ? "Choose a picture or GIF for this alert" : "Choose a sound for this alert",
      filters: [{ name: image ? "Pictures" : "Sounds", extensions: image ? ALERT_IMAGE_EXTS : ALERT_SOUND_EXTS }] });
    if (!path) return;
    const name = await invoke("alert_media_set", { kind, slot, path: String(path) });
    const key = image ? "alertImages" : "alertSounds";
    prefs[key] = Object.assign({}, prefs[key], { [kind]: { name, at: Date.now() } });
    alertsSave();
    alertsPaintList();
  } catch (e) {
    say(String(e && e.message ? e.message : e));
  }
}

async function alertClearMedia(kind, slot) {
  await invoke("alert_media_clear", { kind, slot }).catch(() => {});
  const key = slot === "image" ? "alertImages" : "alertSounds";
  if (prefs[key]) { prefs[key] = Object.assign({}, prefs[key]); delete prefs[key][kind]; }
  alertsSave();
  alertsPaintList();
}

async function alertTest(kind) {
  if (!prefs.widget) { say("Switch on the widget first."); return; }
  const b = alertBuiltin(kind);
  const sample = b ? b[2] : { user: "Someone", amount: "1", message: "" };
  widgetPush();   // the widget has this alert's newest settings first
  try {
    await invoke("widget_alert", { alert: Object.assign({ type: kind, user: "", amount: "", message: "" }, sample, { test: true }) });
    say("Sent a test alert to the widget.");
  } catch (e) {
    say(String(e));
  }
}

// Dragging a row to reorder them (an open editor closes first, so it doesn't get left behind)
function alertsDrag() {
  const list = $("alertList");
  let dragging = null;
  list.addEventListener("dragstart", (e) => {
    dragging = e.target.closest && e.target.closest(".alertRow");
    if (!dragging || e.target.closest("input, select, button")) { dragging = null; return; }
    if (alertOpen) { const ed = list.querySelector(".alertEdit"); if (ed) ed.remove(); alertOpen = null; }
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

// ---------- bots that aren't on this PC (Generic) ----------
// Off unless switched on: then the alerts and song request servers listen to other devices too
// (alerts need a key from them; requests always do). An online bot needs the port forwarded on your
// router, and your public IP address, which is yours to keep to yourself.
let lanIp = "";
async function botRemoteSet(on) {
  prefs.botRemote = on;
  if (on && !prefs.alertKey) prefs.alertKey = reqKeyNew() + reqKeyNew();
  savePrefs();
  await widgetApply();
  if (typeof reqApply === "function") await reqApply();
  document.querySelectorAll(".botRemoteOn").forEach((b) => { b.checked = on; });
  alertsPaint();
  if (typeof reqRenderBot === "function") reqRenderBot();
}

// The part of a bot's setup about bots elsewhere: what, how, and a word of warning. what: "alerts" or "requests"
function botRemoteBlock(what, port, path) {
  const box = el("div", "botRemote");
  const row = el("div", "row");
  const txt = el("div");
  txt.append(el("div", "lbl", "Bots on Other Devices"), el("div", "note", "Off: only programs on this PC can reach these addresses. On: other devices can too" + (what === "alerts" ? " (with the key in their address)." : " (with the key).")));
  const sw = el("label", "switch");
  const cb = el("input", "botRemoteOn");
  cb.type = "checkbox";
  cb.checked = !!prefs.botRemote;
  cb.addEventListener("change", () => botRemoteSet(cb.checked));
  sw.append(cb, el("span"));
  row.append(txt, sw);
  box.append(row);
  if (prefs.botRemote) {
    if (!lanIp) invoke("lan_ip").then((ip) => { if (ip) { lanIp = ip; alertsPaint(); if (typeof reqRenderBot === "function") reqRenderBot(); } }).catch(() => {});
    const ip = lanIp || "THIS-PC";
    const l = el("ol");
    const li = (...parts) => { const x = el("li"); parts.forEach((p) => x.append(typeof p === "string" ? document.createTextNode(p) : p)); return x; };
    l.append(
      li("A bot on another computer at home: give it ", el("code", "", "http://" + ip + ":" + port + path), " in place of 127.0.0.1."),
      li("An online bot (one that isn't on your network): on your router, forward port ", el("code", "", String(port)), " (TCP) to this PC, ", el("code", "", ip), ". Then give the bot the same address with your public IP address in place of ", el("code", "", ip), "."),
    );
    box.append(l);
    box.append(el("p", "note warn", "Careful: your public IP address points to your home. Don't show it on stream or in screenshots, and don't share it; keep Streamer Mode on while you set this up. With it and the key, anyone could " + (what === "alerts" ? "send alerts to your stream." : "request songs.")));
  }
  return box;
}

function alertsPaint() {
  $("alertsOn").checked = alertsOn();
  $("alertsBody").hidden = !alertsOn();
  $("sbOn").checked = prefs.sbOn !== false;
  $("sbHost").value = prefs.sbHost || "127.0.0.1:8080";
  $("alertAnim").value = prefs.alertAnim || "slide";
  $("alertTrans").value = prefs.alertTrans || "seamless";
  $("alertSecs").value = String(Number(prefs.alertSecs) || 5);
  $("alertIconVal").textContent = Math.round(alertIconScale() * 100) + "%";
  $("alertIconDown").disabled = alertIconScale() <= 0.6;
  $("alertIconUp").disabled = alertIconScale() >= 2;
  $("alertOwnCol").checked = !!prefs.alertOwnCol;
  $("alertColRow").hidden = !prefs.alertOwnCol;
  $("alertCol1").value = prefs.alertCol1 || "#8e9bff";
  $("alertCol2").value = prefs.alertCol2 || "#b38cff";
  $("alertDir").value = prefs.alertDir || "115deg";
  const v = prefs.alertVol === undefined ? 0.8 : Number(prefs.alertVol);
  $("alertVol").value = String(Math.round(v * 100));
  $("alertVolVal").textContent = Math.round(v * 100) + "%";
  const port = Number(prefs.widgetPort) || 7878;
  const bot = prefs.alertBot || (prefs.reqBot === "mixitup" ? "mixitup" : prefs.reqBot === "other" ? "generic" : "streamerbot");
  $("alertBot").value = bot;
  $("sbRow").hidden = bot !== "streamerbot";
  $("alertWebRow").hidden = bot === "streamerbot";
  const vars = bot === "mixitup" ? "type=follow&user=$username" : "type=follow&user=NAME&amount=&message=";
  const keyPart = prefs.botRemote && prefs.alertKey ? "key=" + prefs.alertKey + "&" : "";
  $("alertUrl").value = "http://127.0.0.1:" + port + "/alert?" + keyPart + vars;
  $("alertWebNote").textContent = bot === "mixitup" ? "In Mix It Up, its Web Request action, with this address." : "Your bot opens this address when something happens.";
  const help = $("alertBotHelp");
  help.replaceChildren();
  const code = (s) => el("code", "", s);
  const li = (...parts) => { const x = el("li"); parts.forEach((p) => x.append(typeof p === "string" ? document.createTextNode(p) : p)); return x; };
  const ol = el("ol");
  const TYPES = "follow, sub, gift, bits, kicks, donation, raid, hypetrain, poll, ban, watchstreak";
  if (bot === "streamerbot") {
    ol.append(
      li("In Streamer.bot: Servers/Clients > WebSocket Server. Tick Auto Start, then Start Server."),
      li("Put its address and port here (", code("127.0.0.1:8080"), " unless you've changed it), and switch it on."),
      li("That's all: follows, subs, bits, raids and the rest come through by themselves."),
    );
    help.append(ol);
  } else if (bot === "mixitup") {
    ol.append(
      li("In Mix It Up, open the event to alert on (a follow, a sub, bits...) and add a Web Request action."),
      li("Paste the address above, with ", code("type="), " set to that event: ", code(TYPES), "."),
      li("Put Mix It Up's special identifiers in: ", code("user=$username"), ", and the amount and message for that event (its list of special identifiers has the right names, such as the bits or months)."),
    );
    help.append(ol);
  } else {
    ol.append(
      li("When something happens, have your bot open the address above."),
      li("Swap ", code("NAME"), " for your bot's name for the viewer, and fill in ", code("amount"), " and ", code("message"), " if it has them (or leave them empty)."),
      li(code("type"), " says what happened: ", code(TYPES), ", or one of your own alerts' triggers."),
    );
    help.append(ol, botRemoteBlock("alerts", port, "/alert?" + keyPart + vars));
  }
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
  $("alertBot").addEventListener("change", () => { prefs.alertBot = $("alertBot").value; savePrefs(); alertsPaint(); });
  $("alertTrans").addEventListener("change", () => { prefs.alertTrans = $("alertTrans").value; alertsSave(); });
  $("alertSecs").addEventListener("change", () => { prefs.alertSecs = Number($("alertSecs").value); alertsSave(); });
  const stepIcon = (d) => { prefs.alertIcon = Math.round(Math.min(2, Math.max(0.6, alertIconScale() + d)) * 10) / 10; alertsSave(); alertsPaint(); };
  $("alertIconDown").addEventListener("click", () => stepIcon(-0.1));
  $("alertIconUp").addEventListener("click", () => stepIcon(0.1));
  $("alertOwnCol").addEventListener("change", () => { prefs.alertOwnCol = $("alertOwnCol").checked; alertsSave(); alertsPaint(); });
  $("alertCol1").addEventListener("change", () => { prefs.alertCol1 = $("alertCol1").value; alertsSave(); });
  $("alertCol2").addEventListener("change", () => { prefs.alertCol2 = $("alertCol2").value; alertsSave(); });
  $("alertDir").addEventListener("change", () => { prefs.alertDir = $("alertDir").value; alertsSave(); });
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
  $("alertAdd").addEventListener("click", alertAdd);
  // the section folds away, and stays as you left it
  $("alertsBody").open = prefs.alertsOpen !== false;
  $("alertsBody").addEventListener("toggle", () => { prefs.alertsOpen = $("alertsBody").open; savePrefs(); });
  alertsDrag();
  alertsPaint();
  alertsPaintList();
}
