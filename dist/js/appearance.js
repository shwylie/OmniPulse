// Custom CSS, accent colours (including from album art), title marquee, fades, animations and the app name.

const cssEnabled = () => (prefs.cssOn === undefined ? !!(prefs.css || "").trim() : !!prefs.cssOn);

// ---------- custom CSS ----------
function applyUserCss() {
  let st = document.getElementById("userCss");
  if (!st) {
    st = document.createElement("style");
    st.id = "userCss";
    document.head.append(st);
  }
  st.textContent = cssEnabled() ? prefs.css || "" : "";
}

// ---------- accent from album art ----------
let artToken = 0;

function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  let h = 0, s = 0;
  const l = (max + min) / 2;
  if (max !== min) {
    const d = max - min;
    s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
    if (max === r) h = (g - b) / d + (g < b ? 6 : 0);
    else if (max === g) h = (b - r) / d + 2;
    else h = (r - g) / d + 4;
    h /= 6;
  }
  return [h, s, l];
}

function hslToHex(h, s, l) {
  const f = (n) => {
    const k = (n + h * 12) % 12;
    const a = s * Math.min(l, 1 - l);
    const c = l - a * Math.max(-1, Math.min(k - 3, 9 - k, 1));
    return Math.round(c * 255).toString(16).padStart(2, "0");
  };
  return "#" + f(0) + f(8) + f(4);
}

let artColors = null;   // colours taken from the current cover, when that option is on

// Current accent colours (the second one comes from the cover when "Match accent to album art" is on)
let accentNow = ["#8e9bff", "#8e9bff"];

function setAccentColors(a1, a2) {
  const root = document.documentElement;
  accentNow = [a1, a2 || a1];
  root.style.setProperty("--accent", a1);
  root.style.setProperty("--accent-2", a2 || a1);
  root.style.setProperty("--on-accent", onAccent(a1));
  if (widgetReady) widgetPush();
}

// Two colours from the cover: the strongest one, and a clearly different one if there is one
function artPalette(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = "anonymous";
    img.onload = () => {
      try {
        const c = document.createElement("canvas");
        c.width = c.height = 32;
        const ctx = c.getContext("2d", { willReadFrequently: true });
        ctx.drawImage(img, 0, 0, 32, 32);
        const d = ctx.getImageData(0, 0, 32, 32).data;
        const buckets = Array.from({ length: 12 }, (_, k) => ({ r: 0, g: 0, b: 0, w: 0, k }));
        for (let i = 0; i < d.length; i += 4) {
          const [h, s, l] = rgbToHsl(d[i], d[i + 1], d[i + 2]);
          const w = s * Math.max(0, 1 - Math.abs(l - 0.5) * 1.8);
          if (w < 0.05) continue;
          const bk = buckets[Math.floor(h * 12) % 12];
          bk.r += d[i] * w; bk.g += d[i + 1] * w; bk.b += d[i + 2] * w; bk.w += w;
        }
        const order = buckets.filter((b) => b.w > 0).sort((a, b) => b.w - a.w);
        if (!order.length) { reject(new Error("no colour")); return; }
        const first = order[0];
        const apart = (a, b) => Math.min(Math.abs(a - b), 12 - Math.abs(a - b));
        const second = order.find((b) => apart(b.k, first.k) >= 2 && b.w >= first.w * 0.15);
        const theme = document.documentElement.dataset.theme;
        const lightTheme = theme === "light" || theme === "paper";
        const tune = (b, shade) => {
          let [h, s, l] = rgbToHsl(b.r / b.w, b.g / b.w, b.b / b.w);
          s = Math.max(s, 0.5);
          l = lightTheme ? Math.min(Math.max(l, 0.3), 0.45) : Math.min(Math.max(l, 0.58), 0.72);
          if (shade) l = lightTheme ? Math.min(0.55, l + 0.1) : Math.max(0.45, l - 0.13);
          return hslToHex(h, s, l);
        };
        resolve([tune(first), second ? tune(second) : tune(first, true)]);
      } catch (e) {
        reject(e);
      }
    };
    img.onerror = () => reject(new Error("cover could not be read"));
    img.src = url;
  });
}

async function updateArtAccent(t) {
  const tok = ++artToken;
  if (!prefs.artAccent) { artColors = null; applyPrefs(); return; }
  const url = t ? artUrl(t, "t120x120") : "";
  if (!url) { artColors = null; applyPrefs(); return; }
  try {
    const [c1, c2] = await artPalette(url);
    if (tok !== artToken || !prefs.artAccent) return;
    artColors = [c1, c2];
    setAccentColors(c1, c2);
  } catch (e) {
    if (tok === artToken) { artColors = null; applyPrefs(); }
  }
}

// ---------- long titles scroll on hover ----------
function marqueeInit() {
  document.addEventListener("mouseover", (e) => {
    const box = e.target.closest(".title");
    if (!box || box.classList.contains("marqOn")) return;
    const over = box.scrollWidth - box.clientWidth;
    if (over <= 2) return;
    let span = box.firstElementChild;
    if (!span || !span.classList.contains("marqText")) {
      span = el("span", "marqText", box.textContent);
      box.replaceChildren(span);
    }
    span.style.setProperty("--shift", -(over + 6) + "px");
    span.style.setProperty("--dur", Math.max(4, over / 40 + 3) + "s");
    box.classList.add("marqOn");
    box.addEventListener("mouseleave", () => box.classList.remove("marqOn"), { once: true });
  });
}

// ---------- fades between tracks ----------
// Fade the old cover out, wait for the new one to load, fade it in
function fadeImg(img, src) {
  img._tok = (img._tok || 0) + 1;
  const tok = img._tok;
  if (!src) { img.removeAttribute("src"); img.classList.remove("artFade"); return; }
  if (img.getAttribute("src") === src) return;
  if (!img.getAttribute("src")) { img.src = src; return; }
  img.classList.add("artFade");
  const started = performance.now();
  const swap = () => {
    setTimeout(() => {
      if (tok !== img._tok) return;
      img.src = src;
      requestAnimationFrame(() => img.classList.remove("artFade"));
    }, Math.max(0, 250 - (performance.now() - started)));
  };
  const pre = new Image();
  pre.onload = swap;
  pre.onerror = swap;
  pre.src = src;
}

function fadeIn(node) {
  if (!node) return;
  node.classList.remove("fadeIn");
  void node.offsetWidth;   // restart the animation
  node.classList.add("fadeIn");
}

// ---------- second accent colour ----------
function buildSwatches2() {
  const box = $("swatches2");
  box.replaceChildren();
  const off = el("button", "swatch none");
  off.dataset.c = "";
  off.title = "One colour";
  off.setAttribute("aria-label", "No second colour");
  ACCENTS.forEach((c) => {
    const b = el("button", "swatch");
    b.style.background = c;
    b.dataset.c = c;
    b.setAttribute("aria-label", "Second accent " + c);
    box.append(b);
  });
  const custom = el("input");
  custom.type = "color";
  custom.id = "accent2Custom";
  custom.setAttribute("aria-label", "Custom second colour");
  box.append(custom, off);
}

// Marks the chosen second accent, and greys out both accent rows while album-art colours are on
function syncAccent2() {
  document.querySelectorAll("#swatches2 .swatch").forEach((b) => {
    b.classList.toggle("on", (b.dataset.c || "") === (prefs.accent2 || ""));
  });
  const custom = $("accent2Custom");
  if (custom) custom.value = prefs.accent2 || prefs.accent;
  const art = !!prefs.artAccent;
  ["accentRow", "accent2Row"].forEach((id) => {
    const row = $(id);
    row.classList.toggle("disabledRow", art);
    row.title = art ? "Turn off \"Match accent to album art\" to choose your own colours" : "";
    row.querySelectorAll("button, input").forEach((x) => { x.disabled = art; });
  });
}

function setAccent2(c) {
  prefs.accent2 = c || null;
  savePrefs();
  applyPrefs();
  syncAccent2();
}

// ---------- animations (app only; the widget has its own) ----------
function applyAnim() {
  document.documentElement.classList.toggle("noAnim", prefs.animOn === false);
}

// ---------- v35: platform icons and app name in the title bar ----------
function refreshTitle() {
  updateTitle(lastSignedIn);
}

function editAppName() {
  const span = $("titleText");
  if (span.hidden) return;
  const inp = el("input", "titleEdit");
  inp.value = (prefs.appName || "").trim() || span.textContent;
  inp.maxLength = 60;
  inp.setAttribute("aria-label", "App name");
  span.hidden = true;
  span.after(inp);
  inp.focus();
  inp.select();
  let finished = false;
  const done = (save) => {
    if (finished) return;
    finished = true;
    if (save) {
      prefs.appName = inp.value.trim();
      savePrefs();
      $("appNameInput").value = prefs.appName;
    }
    inp.remove();
    span.hidden = false;
    refreshTitle();
  };
  inp.addEventListener("keydown", (ev) => {
    if (ev.key === "Enter") done(true);
    else if (ev.key === "Escape") done(false);
  });
  inp.addEventListener("blur", () => done(true));
}

function appearanceInit() {
  $("artAccentToggle").addEventListener("change", () => {
    prefs.artAccent = $("artAccentToggle").checked;
    savePrefs();
    syncAccent2();
    updateArtAccent(curTrack());
  });

  // Second accent
  buildSwatches2();
  $("swatches2").addEventListener("click", (e) => {
    const b = e.target.closest(".swatch");
    if (b && !b.disabled) setAccent2(b.dataset.c);
  });
  $("swatches2").addEventListener("input", (e) => {
    if (e.target.id === "accent2Custom") setAccent2(e.target.value);
  });
  syncAccent2();

  // Custom CSS
  $("customCss").addEventListener("input", () => {
    prefs.css = $("customCss").value;
    savePrefs();
    applyUserCss();
  });
  $("cssOn").checked = cssEnabled();
  $("cssSection").hidden = !cssEnabled();
  $("cssOn").addEventListener("change", () => {
    prefs.cssOn = $("cssOn").checked;
    savePrefs();
    $("cssSection").hidden = !prefs.cssOn;
    if (prefs.cssOn) $("cssSection").open = true;
    applyUserCss();
  });

  // Animations
  $("animOn").checked = prefs.animOn !== false;
  $("animOn").addEventListener("change", () => { prefs.animOn = $("animOn").checked; savePrefs(); applyAnim(); });
  applyAnim();

  // App name: Settings > Appearance, or right-click the name in the title bar
  $("titleText").addEventListener("contextmenu", (e) => { e.preventDefault(); editAppName(); });
  $("appNameInput").value = prefs.appName || "";
  $("appNameInput").placeholder = "OmniPulse";
  $("appNameInput").addEventListener("input", () => {
    prefs.appName = $("appNameInput").value;
    savePrefs();
    refreshTitle();
  });

  marqueeInit();
}
