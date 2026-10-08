// The app's sound as 16-bit stereo PCM, a piece at a time (a tenth of a second unless told otherwise):
// for casting's live stream (crates/core/src/cast.rs) and Offset OmniPulse (omni_input.rs). Silence keeps flowing while
// nothing plays, so the device's stream never runs dry.
class CastTap extends AudioWorkletProcessor {
  constructor(options) {
    super();
    const ms = (options && options.processorOptions && options.processorOptions.ms) || 100;
    this.frames = Math.round(sampleRate * ms / 1000);
    this.buf = new Int16Array(this.frames * 2);
    this.n = 0;
    // the pieces can go somewhere else: the sound worker's port (js/pcm.js), past the page's main thread
    this.out = this.port;
    this.port.onmessage = (e) => { if (e.data && e.data.port) this.out = e.data.port; };
  }
  process(inputs) {
    const inp = inputs[0] || [];
    const l = inp[0], r = inp[1] || inp[0];
    const len = l ? l.length : 128;
    for (let i = 0; i < len; i++) {
      const a = l ? Math.max(-1, Math.min(1, l[i])) : 0;
      const b = r ? Math.max(-1, Math.min(1, r[i])) : 0;
      this.buf[this.n++] = a < 0 ? a * 32768 : a * 32767;
      this.buf[this.n++] = b < 0 ? b * 32768 : b * 32767;
      if (this.n >= this.buf.length) {
        this.out.postMessage(this.buf.buffer, [this.buf.buffer]);
        this.buf = new Int16Array(this.frames * 2);
        this.n = 0;
      }
    }
    return true;
  }
}
registerProcessor("cast-tap", CastTap);
