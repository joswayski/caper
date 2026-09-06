// Minimal host for the pinned Emscripten RNNoise binary. See README.md/COPYING.
export class RNNoise {
  constructor(module) {
    this.wasm = new WebAssembly.Instance(module, {
      env: {
        __assert_fail() { throw new Error('RNNoise assertion failed'); },
        // Keep memory bounded on the render thread. The initial heap holds one
        // model/state/frame; allocation failure is handled by capture fallback.
        emscripten_resize_heap() { return 0; },
      },
      wasi_snapshot_preview1: { fd_write() { return 5; } }, // No audio/log output.
    }).exports;
    this.wasm.emscripten_stack_init();
    this.wasm.__wasm_call_ctors();
    if (this.wasm.rnnoise_get_frame_size() !== 480) throw new Error('Unexpected RNNoise frame size');
    this.handle = this.wasm.rnnoise_create(0);
    this.pointer = this.wasm.malloc(480 * 4);
    if (!this.handle || !this.pointer) { this.destroy(); throw new Error('RNNoise allocation failed'); }
    this.samples = new Float32Array(this.wasm.memory.buffer, this.pointer, 480);
  }

  process(frame) {
    // RNNoise uses float storage at signed 16-bit PCM scale, unlike DeepFilter.
    for (let i = 0; i < 480; i++) this.samples[i] = frame[i] * 32768;
    this.wasm.rnnoise_process_frame(this.handle, this.pointer, this.pointer);
    for (let i = 0; i < 480; i++) frame[i] = this.samples[i] / 32768;
    // Do not use the returned VAD probability to gate speech: quiet syllables
    // and word endings must not be deliberately cut off by this adapter.
    return frame;
  }

  destroy() {
    if (this.handle) this.wasm.rnnoise_destroy(this.handle);
    if (this.pointer) this.wasm.free(this.pointer);
    this.handle = this.pointer = 0;
  }
}
