import { initSync, df_create, df_get_frame_length, df_process_frame, df_set_post_filter_beta } from '../deepfilter-v1/df.js';
import { RNNoise } from '../rnnoise-v1/rnnoise.js';

// One context per capture; DeepFilter's generated bindings hold module state.
class NoiseProcessor extends AudioWorkletProcessor {
  constructor(options) {
    super();
    this.stopped = false;
    try {
      if (sampleRate !== 48000) throw new Error('Noise suppression requires 48 kHz');
      const { engine, module, model, attenuationLimit } = options.processorOptions;
      if (engine === 'rnnoise') {
        this.rnnoise = new RNNoise(module);
        this.filter = (frame) => this.rnnoise.process(frame);
      } else if (engine === 'deepfilter' && [12, 20, 40].includes(attenuationLimit)) {
        initSync({ module });
        this.handle = df_create(new Uint8Array(model), attenuationLimit);
        if (df_get_frame_length(this.handle) !== 480) throw new Error('Unexpected model frame size');
        df_set_post_filter_beta(this.handle, 0);
        this.filter = (frame) => df_process_frame(this.handle, frame);
      } else throw new Error('Unknown noise suppression configuration');
      this.frame = new Float32Array(480);
      this.inputPosition = 0;
      this.output = new Float32Array(480 * 3);
      this.read = 0;
      // One frame of initial silence prevents 128/480 adaptation gaps. This
      // adds 10 ms on top of each engine's own lookahead/STFT latency.
      this.write = this.available = 480;
      this.port.postMessage('ready');
    } catch {
      this.stopped = true;
      this.rnnoise?.destroy();
      this.port.postMessage('failed');
    }
    this.port.onmessage = ({ data }) => {
      if (data === 'stop') {
        this.stopped = true;
        this.rnnoise?.destroy();
        // DeepFilter has no raw-pointer destructor ABI; closing its dedicated
        // AudioContext releases the entire worklet/WASM instance.
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
        if (this.inputPosition === 480) {
          const processed = this.filter(this.frame);
          for (let j = 0; j < 480; j++) {
            if (!Number.isFinite(processed[j])) throw new Error('Invalid audio');
            this.output[this.write] = processed[j];
            this.write = (this.write + 1) % this.output.length;
          }
          this.available += 480;
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
      this.rnnoise?.destroy();
      this.port.postMessage('failed');
    }
    return !this.stopped;
  }
}

registerProcessor('caper-noise', NoiseProcessor);
