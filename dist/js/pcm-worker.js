// The app's sound on its way to OmniPulse.exe (its own output, OmniPulse Offset, a cast), off the
// page's main thread: the sound tap (js/cast-worklet.js) hands each piece straight to this worker,
// which posts it to the app's local server (/pcm, crates/core/src/local.rs). A page busy drawing a
// long list can't hold it up, so the sound doesn't stutter. One request at a time for each output, so
// the pieces arrive in order; any that wait meanwhile go together in the next one.

const outs = {};   // "main" | "input" | "cast" -> { url, on, queue, busy }

function send(c) {
  if (c.busy || !c.queue.length) return;
  const parts = c.queue;
  c.queue = [];
  let n = 0;
  parts.forEach((p) => { n += p.length; });
  const body = new Uint8Array(n);
  let at = 0;
  parts.forEach((p) => { body.set(p, at); at += p.length; });
  c.busy = true;
  fetch(c.url, { method: "POST", body, mode: "no-cors" }).catch(() => {}).finally(() => { c.busy = false; send(c); });
}

onmessage = (e) => {
  const d = e.data || {};
  if (d.port) {
    // a new route: its address, and the port its sound comes in on
    const c = outs[d.to] = { url: d.url, on: false, queue: [], busy: false };
    d.port.onmessage = (m) => {
      if (!c.on) return;
      c.queue.push(new Uint8Array(m.data));
      // (a few seconds' worth at most, should the app stop answering)
      if (c.queue.length > 150) c.queue.splice(0, c.queue.length - 150);
      send(c);
    };
    return;
  }
  const c = outs[d.to];
  if (!c) return;
  if ("on" in d) { c.on = !!d.on; if (!c.on) c.queue = []; }
  if (d.drop) c.queue = [];   // a pause, a seek or a new track: what's waiting would be heard late
};
