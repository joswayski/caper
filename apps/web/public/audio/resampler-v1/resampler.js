// Streaming band-limited sample-rate conversion for the noise models, which run
// at 48 kHz, when the capture context runs at the device's route rate (an iPhone
// on a Bluetooth headset runs at 24 kHz). Loaded with audioWorklet.addModule
// before a worklet that uses it; AudioWorklet modules share one global scope.
// Windowed-sinc interpolation (Blackman window, 16 zero crossings) at an
// arbitrary ratio, with the cutoff at 95% of the lower rate's Nyquist frequency.
class CaperResampler {
  constructor(from, to) {
    this.step = from / to;
    const cutoff = Math.min(1, to / from) * 0.95;
    this.half = Math.ceil(16 / cutoff);
    this.resolution = 256;
    this.table = new Float32Array(this.half * this.resolution + 2);
    for (let i = 0; i < this.table.length; i++) {
      const x = i / this.resolution;
      const y = cutoff * x;
      const sinc = y === 0 ? 1 : Math.sin(Math.PI * y) / (Math.PI * y);
      const r = x / this.half;
      const window = r >= 1 ? 0 : 0.42 + 0.5 * Math.cos(Math.PI * r) + 0.08 * Math.cos(2 * Math.PI * r);
      this.table[i] = cutoff * sinc * window;
    }
    // Zero history before the first sample; `time` is the next output position.
    this.buffer = new Float32Array(this.half * 2 + 8192);
    this.length = this.half;
    this.time = this.half;
    this.output = new Float32Array(Math.ceil(8192 / this.step) + 8);
  }

  /** Input samples of delay, in the input rate. */
  get latency() { return this.half; }

  kernel(x) {
    const at = Math.abs(x) * this.resolution;
    const index = at | 0;
    if (index >= this.table.length - 1) return 0;
    const fraction = at - index;
    return this.table[index] + (this.table[index + 1] - this.table[index]) * fraction;
  }

  /** Returns a view that is valid until the next call. Accepts up to 8192 samples. */
  process(input) {
    if (this.length + input.length > this.buffer.length) {
      // Keep only the history the kernel still needs.
      const keep = Math.floor(this.time) - this.half + 1;
      this.buffer.copyWithin(0, keep, this.length);
      this.length -= keep;
      this.time -= keep;
    }
    this.buffer.set(input, this.length);
    this.length += input.length;
    let count = 0;
    while (Math.floor(this.time) + this.half < this.length) {
      const base = Math.floor(this.time);
      const fraction = this.time - base;
      let sum = 0;
      for (let k = 1 - this.half; k <= this.half; k++) sum += this.buffer[base + k] * this.kernel(k - fraction);
      this.output[count++] = sum;
      this.time += this.step;
    }
    return this.output.subarray(0, count);
  }
}

/** Fixed-latency bridge from a variable-count producer to fixed render quanta. */
class CaperSampleQueue {
  constructor(prime) {
    this.buffer = new Float32Array(16384);
    this.read = 0;
    this.available = prime;
    this.write = prime;
  }
  push(samples) {
    for (let i = 0; i < samples.length; i++) {
      this.buffer[this.write] = samples[i];
      this.write = (this.write + 1) % this.buffer.length;
    }
    this.available = Math.min(this.available + samples.length, this.buffer.length);
  }
  /** Fills `target`; a shortfall (never expected after priming) is silence. */
  pull(target) {
    for (let i = 0; i < target.length; i++) {
      if (this.available === 0) { target[i] = 0; continue; }
      target[i] = this.buffer[this.read];
      this.read = (this.read + 1) % this.buffer.length;
      this.available--;
    }
  }
}

/**
 * Wraps a 48 kHz one-in-one-out processor for a context at another rate.
 * `core(input, output)` must write output.length samples for input.length.
 */
class CaperRateBridge {
  constructor(rate) {
    this.up = new CaperResampler(rate, 48000);
    this.down = new CaperResampler(48000, rate);
    this.scratch = new Float32Array(this.up.output.length);
    // Both conversions' delay in context samples, plus ratio jitter headroom.
    this.queue = new CaperSampleQueue(Math.ceil(this.up.latency + this.down.latency * rate / 48000) + 4);
  }
  process(source, target, core) {
    const high = this.up.process(source);
    const processed = this.scratch.subarray(0, high.length);
    core(high, processed);
    this.queue.push(this.down.process(processed));
    this.queue.pull(target);
  }
}

globalThis.CaperResampler = CaperResampler;
globalThis.CaperRateBridge = CaperRateBridge;
