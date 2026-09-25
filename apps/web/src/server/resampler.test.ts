import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { runInNewContext } from "node:vm";
import { test } from "node:test";

const assets = new URL("../../public/audio/", import.meta.url);

type Resampler = { process(input: Float32Array): Float32Array; latency: number };
type Bridge = { process(source: Float32Array, target: Float32Array, core: (input: Float32Array, output: Float32Array) => void): void };
interface Scope {
  CaperResampler: new (from: number, to: number) => Resampler;
  CaperRateBridge: new (rate: number) => Bridge;
}

/** Evaluates resampler-v1 as an AudioWorklet module would: into a shared global scope. */
async function loadResampler(): Promise<Scope> {
  const scope: Record<string, unknown> = {};
  scope.globalThis = scope;
  runInNewContext(await readFile(new URL("resampler-v1/resampler.js", assets), "utf8"), scope);
  return scope as unknown as Scope;
}

const tone = (hz: number, rate: number, length: number, amplitude = 0.5) =>
  Float32Array.from({ length }, (_, i) => amplitude * Math.sin(2 * Math.PI * hz * i / rate));
/** Speech-band tones whose sum does not repeat within the delay search. */
const voice = (rate: number, length: number) => {
  const mix = new Float32Array(length);
  for (const hz of [217, 1_013, 3_307]) {
    const part = tone(hz, rate, length, 0.25);
    for (let i = 0; i < length; i++) mix[i] += part[i];
  }
  return mix;
};

/** Runs `input` through a bridge in 128-sample render quanta. */
function render(bridge: Bridge, input: Float32Array, core = (source: Float32Array, target: Float32Array) => target.set(source)) {
  const output = new Float32Array(input.length);
  for (let at = 0; at + 128 <= input.length; at += 128) {
    bridge.process(input.subarray(at, at + 128), output.subarray(at, at + 128), core);
  }
  return output;
}

/** The delay that best aligns output with input, and the worst error at it. */
function alignment(input: Float32Array, output: Float32Array, from: number, maxDelay = 400) {
  let best = { delay: -1, error: Infinity };
  for (let delay = 0; delay < maxDelay; delay++) {
    let error = 0;
    for (let i = from; i < output.length; i++) error = Math.max(error, Math.abs(output[i] - input[i - delay]));
    if (error < best.error) best = { delay, error };
  }
  return best;
}

for (const rate of [24_000, 44_100, 16_000]) test(`a ${rate / 1000} kHz context round-trips speech-band audio through 48 kHz with a fixed delay and no holes`, async () => {
  const { CaperRateBridge } = await loadResampler();
  const input = new Float32Array(Math.floor(rate / 128) * 128); // About one second of whole render quanta.
  for (const hz of [220, 1_000, 3_300, rate * 0.35]) {
    const part = tone(hz, rate, input.length, 0.2);
    for (let i = 0; i < input.length; i++) input[i] += part[i];
  }
  let coreSamples = 0;
  const output = render(new CaperRateBridge(rate), input, (source, target) => { coreSamples += source.length; target.set(source); });
  const { delay, error } = alignment(input, output, 2_000);
  assert.ok(delay > 0 && delay < 60, `delay ${delay} samples`);
  assert.ok(error < 0.01, `max error ${error.toFixed(5)} at ${delay} samples`);
  // The model sees 48 kHz: 48,000 samples for each second of context audio.
  assert.ok(Math.abs(coreSamples - 48_000 * input.length / rate) < 60, `core received ${coreSamples}`);
});

test("converting 48 kHz down removes content above the lower rate's band instead of aliasing it", async () => {
  const { CaperResampler } = await loadResampler();
  const down = new CaperResampler(48_000, 24_000);
  const passed = down.process(tone(18_000, 48_000, 8_192));
  let energy = 0;
  for (let i = 200; i < passed.length; i++) energy += passed[i] ** 2;
  assert.ok(Math.sqrt(energy / (passed.length - 200)) < 0.005, "an 18 kHz tone must not fold down to 6 kHz");
  const kept = new CaperResampler(48_000, 24_000).process(tone(1_000, 48_000, 8_192));
  let keptEnergy = 0;
  for (let i = 200; i < kept.length; i++) keptEnergy += kept[i] ** 2;
  assert.ok(Math.abs(Math.sqrt(keptEnergy / (kept.length - 200)) - 0.5 / Math.SQRT2) < 0.01, "in-band level is preserved");
});

test("RNNoise/DeepFilter worklet v2 runs its 48 kHz engine through the bridge at 24 kHz", async () => {
  const scope = await loadResampler();
  const code = (await readFile(new URL("noise-v1/worklet-v2.js", assets), "utf8")).replace(/^import .*;\n/gm, "");
  const messages: string[] = [];
  let Processor: any;
  const frames: number[] = [];
  const sandbox: Record<string, unknown> = {
    AudioWorkletProcessor: class { port = { postMessage: (x: string) => messages.push(x), close() {} }; },
    sampleRate: 24_000, CaperRateBridge: scope.CaperRateBridge,
    RNNoise: class { process(frame: Float32Array) { frames.push(frame.length); return frame; } destroy() {} },
    registerProcessor: (_name: string, value: unknown) => { Processor = value; },
  };
  sandbox.globalThis = sandbox;
  runInNewContext(code, sandbox);
  const processor = new Processor({ processorOptions: { engine: "rnnoise" } });
  assert.deepEqual(messages, ["ready"], "24 kHz is no longer refused");
  const input = voice(24_000, 187 * 128);
  const output = new Float32Array(input.length);
  for (let at = 0; at + 128 <= input.length; at += 128) {
    assert.equal(processor.process([[input.subarray(at, at + 128)]], [[output.subarray(at, at + 128)]]), true);
  }
  assert.ok(frames.length >= 95 && frames.every((length) => length === 480), "the engine gets 480-sample 48 kHz frames");
  // The engine's one-frame (10 ms) buffer is 240 samples at 24 kHz, plus the bridge.
  const { delay, error } = alignment(input, output, 2_000);
  assert.ok(delay >= 240 && delay < 300, `delay ${delay}`);
  assert.ok(error < 0.01, `max error ${error}`);
});

test("DPDFNet worklet v4 feeds its worker 48 kHz hops at 24 kHz and plays the results back", async () => {
  const scope = await loadResampler();
  const code = await readFile(new URL("dpdfnet8-v2/worklet-v4.js", assets), "utf8");
  const messages: any[] = [];
  let Processor: any;
  const sandbox: Record<string, unknown> = {
    AudioWorkletProcessor: class { port = { onmessage: null, postMessage: (message: unknown) => messages.push(message), close() {} }; },
    sampleRate: 24_000, CaperRateBridge: scope.CaperRateBridge,
    registerProcessor: (_name: string, value: unknown) => { Processor = value; },
  };
  sandbox.globalThis = sandbox;
  runInNewContext(code, sandbox);
  const processor = new Processor();
  const input = voice(24_000, 187 * 128);
  const output = new Float32Array(input.length);
  let handled = 0;
  for (let at = 0; at + 128 <= input.length; at += 128) {
    assert.equal(processor.process([[input.subarray(at, at + 128)]], [[output.subarray(at, at + 128)]]), true);
    // An identity "model" answering between callbacks.
    for (; handled < messages.length; handled++) {
      const message = messages[handled];
      if (message?.type !== "process") continue;
      assert.equal(new Float32Array(message.samples).length, 480);
      processor.port.onmessage({ data: { type: "output", samples: message.samples } });
    }
  }
  assert.ok(!messages.includes("bypassed"));
  assert.ok(messages.filter((message) => message?.type === "process").length >= 95, "48 kHz hops: ~100 per second");
  // Three prebuffered 480-sample 48 kHz hops (720 samples here) plus the bridge.
  const { delay, error } = alignment(input, output, 6_000, 1_200);
  assert.ok(delay >= 720 && delay < 800, `delay ${delay}`);
  assert.ok(error < 0.01, `max error ${error} at ${delay}`);
});
