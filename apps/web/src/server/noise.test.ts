import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { runInNewContext } from "node:vm";
import { test } from "node:test";

const assets = new URL("../../public/audio/", import.meta.url);

test("RNNoise pinned binary performs finite, nonzero inference and suppresses stationary noise", async () => {
  const bytes = await readFile(new URL("rnnoise-v1/rnnoise.wasm", assets));
  assert.equal(createHash("sha256").update(bytes).digest("hex"), "b3b67c9eae8f0791aad468c708659e0850bb37b0fb9c8a8666f2d7b0b6869bc4");
  const { RNNoise } = await import(new URL("rnnoise-v1/rnnoise.js", assets).href);
  const filter = new RNNoise(new WebAssembly.Module(bytes));
  let inputEnergy = 0, outputEnergy = 0, seed = 1;
  for (let n = 0; n < 200; n++) {
    const frame = Float32Array.from({ length: 480 }, () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return (seed / 4294967296 - 0.5) * 0.1;
    });
    if (n > 100) inputEnergy += frame.reduce((sum, x) => sum + x * x, 0);
    const output = filter.process(frame);
    assert.ok(output.every(Number.isFinite));
    if (n > 100) outputEnergy += output.reduce((sum: number, x: number) => sum + x * x, 0);
  }
  assert.ok(outputEnergy > 0 && outputEnergy < inputEnergy * 0.25);
  filter.destroy();
  filter.destroy();
  assert.equal(filter.handle, 0);
  assert.equal(filter.pointer, 0);
});

test("shared worklet preserves frame order for both engines, selects DF blend and disables post-filter", async () => {
  const code = (await readFile(new URL("noise-v1/worklet.js", assets), "utf8")).replace(/^import .*;\n/gm, "");
  for (const engine of ["deepfilter", "rnnoise"]) {
    const messages: string[] = [];
    let Processor: any, attenuation: number | undefined, beta: number | undefined, destroyed = 0;
    runInNewContext(code, {
      AudioWorkletProcessor: class { port = { postMessage: (x: string) => messages.push(x), close() {} }; },
      sampleRate: 48000,
      initSync() {}, df_create: (_model: unknown, value: number) => { attenuation = value; return 1; },
      df_get_frame_length: () => 480, df_set_post_filter_beta: (_h: number, value: number) => { beta = value; },
      df_process_frame: (_h: number, frame: Float32Array) => frame,
      RNNoise: class { process(frame: Float32Array) { return frame; } destroy() { destroyed++; } },
      registerProcessor: (_name: string, value: unknown) => { Processor = value; },
    });
    const processor = new Processor({ processorOptions: { engine, attenuationLimit: 20 } });
    assert.deepEqual(messages, ["ready"]);
    if (engine === "deepfilter") { assert.equal(attenuation, 20); assert.equal(beta, 0); }
    let offset = 0;
    for (const quantum of [128, 256, 64, ...Array(1000).fill(128)]) {
      const input = Float32Array.from({ length: quantum }, (_, i) => offset + i + 1);
      const output = new Float32Array(quantum);
      assert.equal(processor.process([[input]], [[output]]), true);
      for (let i = 0; i < quantum; i++) assert.equal(output[i], offset + i < 480 ? 0 : offset + i - 479);
      offset += quantum;
    }
    processor.port.onmessage({ data: "stop" });
    assert.equal(processor.process([], []), false);
    assert.equal(destroyed, engine === "rnnoise" ? 1 : 0);
  }
});
