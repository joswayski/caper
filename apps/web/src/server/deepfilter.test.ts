import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { test } from "node:test";

const assets = new URL("../../public/audio/deepfilter-v1/", import.meta.url);

test("pinned DeepFilterNet3 WASM/model execute actual neural inference", async () => {
  const wasm = await readFile(new URL("df_bg.wasm", assets));
  const model = await readFile(new URL("DeepFilterNet3.bin", assets));
  assert.equal(createHash("sha256").update(wasm).digest("hex"), "440b5d12b6ea7d95008736f844221d7874ee15de5cb10d3015002470fdba0432");
  assert.equal(createHash("sha256").update(model).digest("hex"), "c94d91f70911001c946e0fabb4aa9adc37045f45a03b56008cb0c8244cb63616");
  const bindings = await import(new URL("df.js", assets).href);
  bindings.initSync({ module: new WebAssembly.Module(wasm) });
  const handle = bindings.df_create(model, 40);
  assert.equal(bindings.df_get_frame_length(handle), 480);
  let inputEnergy = 0;
  let outputEnergy = 0;
  let seed = 1;
  // Deterministic broadband noise, not a speech-quality benchmark.
  for (let frame = 0; frame < 200; frame++) {
    const input = Float32Array.from({ length: 480 }, () => {
      seed = (Math.imul(seed, 1664525) + 1013904223) >>> 0;
      return (seed / 4294967296 - 0.5) * 0.1;
    });
    const output = bindings.df_process_frame(handle, input);
    assert.equal(output.length, 480);
    assert.ok(output.every(Number.isFinite));
    if (frame > 100) {
      for (let i = 0; i < 480; i++) { inputEnergy += input[i] ** 2; outputEnergy += output[i] ** 2; }
    }
  }
  assert.ok(outputEnergy > 0, "filter should produce actual samples, not permanent silence");
  assert.ok(outputEnergy < inputEnergy * 0.25, "model should attenuate stationary noise by more than 6 dB");
});
