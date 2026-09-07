import assert from "node:assert/strict";
import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { performance } from "node:perf_hooks";
import { test } from "node:test";
import * as ort from "onnxruntime-web";
import { DpdfnetStream } from "../../public/audio/dpdfnet8-v2/dsp.js";

const assets = new URL("../../public/audio/dpdfnet8-v2/", import.meta.url);

test("shipped browser ONNX runtime is the intact pinned WASM binary", async () => {
  // Model inference below uses node_modules; also verify the bytes actually served to browsers.
  const wasm = await readFile(new URL("ort-wasm-simd-threaded.wasm", assets));
  assert.equal(wasm.length, 11_905_541);
  assert.equal(createHash("sha256").update(wasm).digest("hex"), "45eaee27761ad883742a8d4b8fce1538d60ce43b51adf1726fafccc59b8c1a15");
  assert.equal(WebAssembly.validate(wasm), true);
});

test("pinned DPDFNet-8 model performs stateful inference on real spectra", async () => {
  ort.env.wasm.numThreads = 1;
  const model = await readFile(new URL("dpdfnet8_48khz_hr.onnx", assets));
  assert.equal(model.length, 14_857_107);
  assert.equal(createHash("sha256").update(model).digest("hex"), "7b3afbb260a08fe9af3d16e3bda992971be1e7e951d1dee7c2d235f5c43f5631");
  const metadata = JSON.parse(await readFile(new URL("metadata.json", assets), "utf8"));
  assert.equal(metadata.erbNormInit.length, 481);
  assert.equal(metadata.specNormInit.length, 96);
  const state = new Float32Array(metadata.stateSize);
  state.set(metadata.erbNormInit);
  state.set(metadata.specNormInit, metadata.erbNormStateSize);
  const session = await ort.InferenceSession.create(model, { executionProviders: ["wasm"] });
  let calls = 0;
  const stream = new DpdfnetStream(async (spec: Float32Array, current: Float32Array) => {
    const result = await session.run({
      spec: new ort.Tensor("float32", spec, [1, 1, 481, 2]),
      state_in: new ort.Tensor("float32", current, [metadata.stateSize]),
    });
    calls++;
    return { spec: result.spec_e.data as Float32Array, state: result.state_out.data as Float32Array };
  }, state);
  const started = performance.now();
  let energy = 0;
  for (let frame = 0; frame < 8; frame++) {
    const input = Float32Array.from({ length: 480 }, (_, i) => 0.05 * Math.sin(2 * Math.PI * 440 * (frame * 480 + i) / 48_000));
    const output = await stream.process(input);
    assert.equal(output.length, 480);
    assert.ok(output.every(Number.isFinite));
    if (frame > 1) for (const sample of output) energy += sample * sample;
  }
  const elapsed = performance.now() - started;
  assert.equal(calls, 8);
  assert.ok(energy > 0, "actual model output must not remain silent");
  // Diagnostic only: shared CI/orb CPU speed is not a correctness gate.
  console.log(`DPDFNet-8: ${(elapsed / calls).toFixed(1)} ms/hop (${elapsed.toFixed(0)} ms total; includes startup, not a sustained benchmark)`);
  await session.release();
});

async function loadWorklet() {
  const code = await readFile(new URL("worklet.js", assets), "utf8");
  const messages: unknown[] = [];
  let Processor: any;
  new Function("AudioWorkletProcessor", "registerProcessor", code)(
    class { port = { onmessage: null, postMessage: (message: unknown) => messages.push(message), close() {} }; },
    (_name: string, value: unknown) => { Processor = value; },
  );
  return { processor: new Processor(), messages };
}

test("DPDFNet identity STFT has one-hop alignment and unity gain", async () => {
  const identity = new DpdfnetStream(async (spec, state) => ({ spec, state }), new Float32Array());
  const input = Float32Array.from({ length: 480 * 8 }, (_, i) =>
    0.13 * Math.sin(2 * Math.PI * 997 * i / 48_000) + (i === 731 ? 0.7 : 0));
  const output = new Float32Array(input.length);
  for (let at = 0; at < input.length; at += 480)
    output.set(await identity.process(input.subarray(at, at + 480)), at);
  assert.ok(output.subarray(0, 480).every((sample) => sample === 0));
  let maxError = 0;
  for (let i = 480; i < output.length; i++) maxError = Math.max(maxError, Math.abs(output[i] - input[i - 480]));
  assert.ok(maxError < 2e-6, `identity reconstruction error ${maxError}`);
});

test("DPDFNet adapter prebuffers three hops and stays continuous across variable callbacks and response jitter", async () => {
  const { processor, messages } = await loadWorklet();
  let handled = 0;
  const submitted: number[] = [];
  const respond = () => {
    for (; handled < messages.length; handled++) {
      const message = messages[handled] as any;
      if (message?.type !== "process") continue;
      const samples = new Float32Array(message.samples);
      submitted.push(...samples);
      processor.port.onmessage({ data: { type: "output", samples: samples.buffer } });
    }
  };
  const callbackSizes = [73, 211, 64, 389, 127, 96, 480, 31, 257, 192, 480, 128, 352, 480];
  let next = 1;
  const rendered: number[] = [];
  let accumulated = 0;
  for (const size of callbackSizes) {
    const source = Float32Array.from({ length: size }, () => next++);
    const target = new Float32Array(size);
    assert.equal(processor.process([[source]], [[target]]), true);
    rendered.push(...target);
    accumulated += size;
    // Deterministic jitter: hold all responses through startup, then deliver only
    // between callbacks. The three-hop reserve absorbs that scheduling delay.
    if (accumulated >= 1_440) respond();
  }
  const firstAudio = rendered.findIndex((sample) => sample !== 0);
  assert.ok(firstAudio >= 1_440, `startup was only ${firstAudio} samples`);
  const live = rendered.slice(firstAudio);
  assert.deepEqual(live, submitted.slice(0, live.length));
  assert.ok(live.every((sample) => sample !== 0), "no holes after startup");
});

test("DPDFNet adapter fails explicitly on overload and underrun", async () => {
  const overloaded = await loadWorklet();
  for (let i = 0; i < 35; i++) overloaded.processor.process([[new Float32Array(128)]], [[new Float32Array(128)]]);
  assert.equal(overloaded.messages.filter((message) => (message as any)?.type === "process").length, 8);
  assert.equal(overloaded.messages.at(-1), "failed");

  const underrun = await loadWorklet();
  for (let i = 0; i < 3; i++) underrun.processor.port.onmessage({
    data: { type: "output", samples: new Float32Array(480).fill(i + 1).buffer },
  });
  assert.equal(underrun.processor.process([[new Float32Array(1_440)]], [[new Float32Array(1_440)]]), true);
  assert.equal(underrun.processor.process([[new Float32Array(1)]], [[new Float32Array(1)]]), false);
  assert.equal(underrun.messages.at(-1), "failed");
});

test("DPDFNet adapter keeps silence and missing input deterministic", async () => {
  const { processor, messages } = await loadWorklet();
  const missing = new Float32Array(128).fill(1);
  assert.equal(processor.process([[]], [[missing]]), true);
  assert.ok(missing.every((sample) => sample === 0));
  for (let i = 0; i < 12; i++) processor.process([[new Float32Array(128)]], [[new Float32Array(128)]]);
  for (const message of messages) {
    if ((message as any)?.type === "process") {
      const samples = new Float32Array((message as any).samples);
      assert.ok(samples.every((sample) => sample === 0));
      processor.port.onmessage({ data: { type: "output", samples: samples.buffer } });
    }
  }
  const target = new Float32Array(480);
  assert.equal(processor.process([[new Float32Array(480)]], [[target]]), true);
  assert.ok(target.every((sample) => sample === 0));
  assert.notEqual(messages.at(-1), "failed");
});
