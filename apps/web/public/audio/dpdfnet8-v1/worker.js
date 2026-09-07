import * as ort from './ort.wasm.bundle.min.mjs';
import { DpdfnetStream } from './dsp.js';

ort.env.wasm.numThreads = 1;
ort.env.wasm.wasmPaths = './';

let stream;
try {
  const [metadata, session] = await Promise.all([
    fetch('./metadata.json').then((response) => {
      if (!response.ok) throw new Error('metadata download failed');
      return response.json();
    }),
    ort.InferenceSession.create('./dpdfnet8_48khz_hr.onnx', {
      executionProviders: ['wasm'], graphOptimizationLevel: 'all',
    }),
  ]);
  const initialState = () => {
    const state = new Float32Array(metadata.stateSize);
    state.set(metadata.erbNormInit, 0);
    state.set(metadata.specNormInit, metadata.erbNormStateSize);
    return state;
  };
  const infer = async (spec, currentState) => {
    const output = await session.run({
      spec: new ort.Tensor('float32', spec, [1, 1, 481, 2]),
      state_in: new ort.Tensor('float32', currentState, [metadata.stateSize]),
    });
    return { spec: output.spec_e.data, state: output.state_out.data };
  };
  // Pay ORT's first-run compilation/allocation cost before exposing the engine.
  // Neither recurrent nor overlap-add warm-up state is allowed into live capture.
  await infer(new Float32Array(481 * 2), initialState());
  stream = new DpdfnetStream(infer, initialState());
  postMessage({ type: 'ready' });
} catch {
  postMessage({ type: 'failed' });
}

let processing = Promise.resolve();
onmessage = ({ data }) => {
  if (data.type !== 'process' || !stream) return;
  // Serialize hops: recurrent state and overlap-add must never run concurrently.
  processing = processing.then(async () => {
    try {
      const started = performance.now();
      const output = await stream.process(new Float32Array(data.samples));
      if (!output.every(Number.isFinite)) throw new Error('Invalid processed audio');
      postMessage({ type: 'output', samples: output.buffer, duration: performance.now() - started }, [output.buffer]);
    } catch {
      stream = undefined;
      postMessage({ type: 'failed' });
    }
  });
};
