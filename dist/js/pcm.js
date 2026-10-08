// Routes a sound tap (js/cast-worklet.js) to OmniPulse.exe through the sound worker (js/pcm-worker.js),
// so the page's main thread isn't in the way. Returns a switch: gate(true) sends, gate(false) stops.
// Where a worker can't be made, the page sends the pieces itself, as it used to (fallback).

let pcmWorker = null;

async function pcmRoute(node, to, rate, fallback) {
  try {
    if (!window.Worker || !window.MessageChannel) throw new Error("no workers");
    if (!localInfo) localInfo = await invoke("local_server");
    if (!pcmWorker) pcmWorker = new Worker("js/pcm-worker.js");
    const ch = new MessageChannel();
    const url = "http://127.0.0.1:" + localInfo.port + "/pcm?k=" + encodeURIComponent(localInfo.token) + "&to=" + to + "&rate=" + rate;
    pcmWorker.postMessage({ to, url, port: ch.port2 }, [ch.port2]);
    node.port.postMessage({ port: ch.port1 }, [ch.port1]);
    return (on) => pcmWorker.postMessage({ to, on: !!on });
  } catch (e) {
    let on = false;
    node.port.onmessage = (m) => { if (on) fallback(m.data); };
    return (v) => { on = !!v; };
  }
}

// What's waiting in the worker for this output, dropped (a pause, a seek, a new track)
function pcmDrop(to) {
  if (pcmWorker) pcmWorker.postMessage({ to, drop: true });
}
