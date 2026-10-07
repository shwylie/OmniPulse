// AudioWorklet for Spotify playback: plays the 44.1 kHz stereo PCM the Rust side streams in
// (see src-tauri/src/spotify_engine.rs), and reports back how much it has played so the
// sender can stay just ahead. Runs in the audio thread; spotify.js talks to it via its port.

const CAPACITY = 44100 * 8;   // frames; the sender stays well under a second ahead
const PRIME = 4410;           // wait for 100 ms of audio before starting, so it doesn't stutter
const REPORT_EVERY = 2205;    // frames between progress reports (50 ms)

class SpotifyPcm extends AudioWorkletProcessor {
  constructor() {
    super();
    this.left = new Float32Array(CAPACITY);
    this.right = new Float32Array(CAPACITY);
    this.read = 0;
    this.count = 0;
    this.gen = 0;
    this.played = 0;
    this.sinceReport = 0;
    this.playing = false;
    this.primed = false;
    this.eos = false;   // the track has been fully sent: play out the rest without waiting to prime
    this.port.onmessage = (e) => this.onMessage(e.data);
  }

  onMessage(m) {
    if (m.type === "pcm") {
      if (m.gen !== this.gen) return;
      const s = m.samples;
      let w = (this.read + this.count) % CAPACITY;
      for (let i = 0; i + 1 < s.length; i += 2) {
        if (this.count === CAPACITY) { this.read = (this.read + 1) % CAPACITY; this.count--; }   // overflow: drop the oldest
        this.left[w] = s[i];
        this.right[w] = s[i + 1];
        w = (w + 1) % CAPACITY;
        this.count++;
      }
    } else if (m.type === "flush") {
      this.gen = m.gen;
      this.read = 0;
      this.count = 0;
      this.played = 0;
      this.sinceReport = 0;
      this.primed = false;
      this.eos = false;
    } else if (m.type === "play") {
      this.playing = true;
    } else if (m.type === "pause") {
      this.playing = false;
    } else if (m.type === "eos") {
      if (m.gen === this.gen) this.eos = true;
    }
  }

  process(_inputs, outputs) {
    const out = outputs[0];
    const L = out[0];
    const R = out[1] || out[0];
    const n = L.length;
    let k = 0;
    if (this.playing && (this.primed || this.count >= PRIME || (this.eos && this.count > 0))) {
      this.primed = true;
      const take = Math.min(n, this.count);
      for (; k < take; k++) {
        L[k] = this.left[this.read];
        R[k] = this.right[this.read];
        this.read = (this.read + 1) % CAPACITY;
      }
      this.count -= take;
      this.played += take;
      this.sinceReport += take;
      if (this.count === 0) {
        this.primed = false;   // ran dry: the end of the track, or a hiccup (re-prime before going on)
        this.port.postMessage({ type: "empty", gen: this.gen, frames: this.played });
      }
    }
    for (; k < n; k++) { L[k] = 0; R[k] = 0; }
    if (this.sinceReport >= REPORT_EVERY) {
      this.sinceReport = 0;
      this.port.postMessage({ type: "played", gen: this.gen, frames: this.played, level: this.count });
    }
    return true;
  }
}

registerProcessor("spotify-pcm", SpotifyPcm);
