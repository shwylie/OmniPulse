// Casting: the app's sound (after volume) as 16-bit stereo PCM, about a tenth of a second at a time,
// for the live stream the device plays (crates/core/src/cast.rs). Silence keeps flowing while
// nothing plays, so the device's stream never runs dry.
class CastTap extends AudioWorkletProcessor {
  constructor() {
    super();
    this.frames = Math.round(sampleRate / 10);
    this.buf = new Int16Array(this.frames * 2);
    this.n = 0;
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
        this.port.postMessage(this.buf.buffer, [this.buf.buffer]);
        this.buf = new Int16Array(this.frames * 2);
        this.n = 0;
      }
    }
    return true;
  }
}
registerProcessor("cast-tap", CastTap);
