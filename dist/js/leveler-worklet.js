// The loudness leveler: a compressor in the spirit of Audio Damage's RoughRider, running on the
// audio thread (an AudioWorklet), so it never competes with the interface. Written from scratch:
//
//  - the detector listens through a 100 Hz high-pass, linked across both channels, so bass hits
//    don't make the whole mix pump
//  - a soft-knee gain computer, smoothed in decibels: quick to catch peaks, and a release that
//    depends on the music (fast after short peaks, slow while it's been loud a while)
//  - a parallel blend with the dry signal, which keeps the punch
//  - a slow "loudness rider" that brings every track toward the same level
// One Strength value (0-1) sets all of it. The app's limiter comes after this.
// Messages in: { type: "set", on, strength }, { type: "newTrack" }, { type: "meter", on }.
// Messages out (while metering): { type: "meter", gr, ride } in dB.

const DB = 8.685889638;            // 20 / ln(10)
const fromDb = (db) => Math.exp(db / DB);
const coef = (ms) => Math.exp(-1 / (sampleRate * ms / 1000));
const TARGET_DB = -17;             // where the rider aims (short-term RMS, dBFS)

class Leveler extends AudioWorkletProcessor {
  constructor() {
    super();
    this.on = false;
    this.hpL = 0; this.hpR = 0; this.xL = 0; this.xR = 0;
    this.ms = 1e-6;                // detector power
    this.gr = 0;                   // gain reduction now, dB (<= 0)
    this.held = 0;                 // slow average of gr, for the program-dependent release
    this.ride = 0;                 // rider gain, dB
    this.blockPow = 0;
    this.fastFor = 0;              // samples left of quick settling after a new track
    this.meterOn = false;
    this.sinceMeter = 0;
    this.hpCoef = Math.exp(-2 * Math.PI * 100 / sampleRate);
    this.rmsCoef = coef(8);
    this.heldCoef = coef(1200);
    this.set(0.5);
    this.port.onmessage = (e) => {
      const m = e.data;
      if (m.type === "set") { this.on = !!m.on; this.set(m.strength); if (!this.on) { this.gr = 0; this.ride = 0; } }
      else if (m.type === "newTrack") this.fastFor = sampleRate * 4;
      else if (m.type === "meter") this.meterOn = !!m.on;
    };
  }

  set(strength) {
    const s = Math.max(0, Math.min(1, Number(strength) || 0));
    this.thr = -10 - 28 * s;               // threshold: -10 dB (gentle) to -38 dB (hard)
    this.ratio = 1.6 + 7.4 * s * s;        // 1.6:1 up to 9:1
    this.knee = 12 - 6 * s;                // a wide knee, narrowing as it gets stronger
    this.mix = 0.45 + 0.55 * Math.min(1, s * 1.5);   // parallel: some dry signal at low settings
    this.atk = coef(30 - 24 * s);          // 30 ms down to 6 ms
    this.relFast = coef(70);
    this.relSlow = coef(700 - 300 * s);
    this.rideRange = 4 + 8 * s;            // how far the rider may move, either way
    this.rideRate = 0.0006 + 0.0012 * s;   // per block
  }

  // How much to turn down for a detector level, in dB (soft knee)
  computer(level) {
    const over = level - this.thr;
    const slope = 1 / this.ratio - 1;
    if (2 * over < -this.knee) return 0;
    if (2 * Math.abs(over) <= this.knee) {
      const k = over + this.knee / 2;
      return slope * k * k / (2 * this.knee);
    }
    return slope * over;
  }

  process(inputs, outputs) {
    const inp = inputs[0];
    const out = outputs[0];
    if (!inp || !inp.length) return true;
    const L = inp[0];
    const R = inp[1] || inp[0];
    const oL = out[0];
    const oR = out[1] || out[0];
    const n = L.length;
    if (!this.on) {
      oL.set(L);
      if (oR !== oL) oR.set(R);
      return true;
    }
    const fast = this.fastFor > 0;
    const hp = this.hpCoef;
    let pow = 0;
    for (let i = 0; i < n; i++) {
      const l = L[i], r = R[i];
      // sidechain high-pass (one pole), linked
      this.hpL = hp * (this.hpL + l - this.xL); this.xL = l;
      this.hpR = hp * (this.hpR + r - this.xR); this.xR = r;
      const x = Math.max(Math.abs(this.hpL), Math.abs(this.hpR));
      this.ms = this.rmsCoef * this.ms + (1 - this.rmsCoef) * x * x;
      const level = 10 * Math.log10(this.ms + 1e-12) + 3;   // RMS of a sine reads as its peak
      const target = this.computer(level);
      if (target < this.gr) {
        this.gr = this.atk * this.gr + (1 - this.atk) * target;
      } else {
        // the longer it's been working hard, the slower it lets go
        const w = Math.min(1, -this.held / 6);
        const rel = this.relFast + (this.relSlow - this.relFast) * w;
        this.gr = rel * this.gr + (1 - rel) * target;
      }
      this.held = this.heldCoef * this.held + (1 - this.heldCoef) * this.gr;
      const wet = fromDb(this.gr);
      const g = fromDb(this.ride) * (1 - this.mix + this.mix * wet);
      const a = l * g, b = r * g;
      oL[i] = a;
      if (oR !== oL) oR[i] = b;
      pow += 0.5 * (a * a + b * b);
    }
    // the rider, once per block: toward the target, ignoring silence and quiet intros
    const blockDb = 10 * Math.log10(pow / n + 1e-12);
    this.blockPow = 0.99 * this.blockPow + 0.01 * (pow / n);   // about a third of a second
    const avgDb = 10 * Math.log10(this.blockPow + 1e-12);
    if (blockDb > -50) {
      const want = Math.max(-this.rideRange, Math.min(this.rideRange, this.ride + (TARGET_DB - avgDb)));
      this.ride += (want - this.ride) * (fast ? this.rideRate * 12 : this.rideRate);
    }
    if (fast) this.fastFor -= n;
    if (this.meterOn && (this.sinceMeter += n) >= 2205) {
      this.sinceMeter = 0;
      this.port.postMessage({ type: "meter", gr: this.gr, ride: this.ride });
    }
    return true;
  }
}

registerProcessor("leveler", Leveler);
