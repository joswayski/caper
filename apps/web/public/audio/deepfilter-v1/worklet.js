import { initSync, df_create, df_get_frame_length, df_process_frame } from './df.js';

// One context per microphone: the generated WASM bindings hold module-level state.
class DeepFilterProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.stopped = false;
    try {
      if (sampleRate !== 48000) throw new Error('DeepFilterNet requires 48 kHz');
      initSync({ module: options.processorOptions.module });
      this.handle = df_create(new Uint8Array(options.processorOptions.model), 40);
      const length = df_get_frame_length(this.handle);
      if (length !== 480) throw new Error('Unexpected model frame size');
      this.frame = new Float32Array(length);
      this.inputPosition = 0;
      this.output = new Float32Array(length * 3);
      this.read = 0;
      // A full frame of initial silence gives deterministic buffering delay.
      // Without it, 128-sample worklet quanta vs 480-sample model hops cause gaps.
      this.write = length;
      this.available = length;
      this.port.postMessage('ready');
    } catch {
      this.stopped = true;
      this.port.postMessage('failed');
    }
    this.port.onmessage = ({ data }) => {
      if (data === 'stop') {
        this.stopped = true;
        // Upstream returns a raw pointer, not a wasm-bindgen DFState wrapper.
        // There is no raw-pointer destructor ABI; release the entire context/WASM
        // instance instead (owned and closed by microphone.ts).
        this.handle = 0;
        this.port.close();
      }
    };
  }

  process(inputs, outputs) {
    if (this.stopped) return false;
    const input = inputs[0]?.[0];
    const output = outputs[0]?.[0];
    if (!output) return true;
    try {
      for (let i = 0; i < output.length; i++) {
        this.frame[this.inputPosition++] = input?.[i] ?? 0;
        if (this.inputPosition === this.frame.length) {
          const processed = df_process_frame(this.handle, this.frame);
          for (let j = 0; j < processed.length; j++) {
            if (!Number.isFinite(processed[j])) throw new Error('Invalid audio');
            this.output[this.write] = processed[j];
            this.write = (this.write + 1) % this.output.length;
          }
          this.available += processed.length;
          this.inputPosition = 0;
        }
        if (this.available === 0) throw new Error('Audio buffer underrun');
        output[i] = this.output[this.read];
        this.read = (this.read + 1) % this.output.length;
        this.available--;
      }
    } catch {
      output.fill(0);
      this.stopped = true;
      this.port.postMessage('failed');
    }
    return !this.stopped;
  }
}

registerProcessor('caper-deepfilter', DeepFilterProcessor);
