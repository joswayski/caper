import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { NoiseAssets } from "../media/noise-assets.ts";

function setup(t: TestContext) {
  const requests: string[] = [];
  const module = {} as WebAssembly.Module;
  t.mock.method(globalThis, "fetch", async (url: string) => {
    requests.push(url);
    return new Response(new Uint8Array([1, 2, 3]));
  });
  const compile = t.mock.method(WebAssembly, "compile", async () => module);
  return { assets: new NoiseAssets(), requests, compile, module };
}

test("preparation shares downloads and compiled code across concurrent callers and later captures", async (t) => {
  const { assets, requests, compile, module } = setup(t);
  const [first, second] = await Promise.all([assets.load("deepfilter"), assets.load("deepfilter")]);
  const later = await assets.load("deepfilter");
  assert.equal(first, second);
  assert.equal(first, later);
  assert.equal(first.module, module);
  assert.equal(first.model!.byteLength, 3);
  assert.equal(requests.length, 2);
  assert.equal(compile.mock.callCount(), 1);
});

test("RNNoise uses its own cached module and never downloads a DeepFilter model", async (t) => {
  const { assets, requests, compile } = setup(t);
  assert.equal((await assets.load("rnnoise")).model, undefined);
  await assets.load("rnnoise");
  assert.deepEqual(requests, ["/audio/rnnoise-v1/rnnoise.wasm"]);
  await assets.load("deepfilter");
  assert.equal(compile.mock.callCount(), 2);
});

test("a failed preload is evicted so a later join can retry", async (t) => {
  const { assets, compile } = setup(t);
  compile.mock.mockImplementationOnce(async () => { throw new Error("compile failed"); });
  await assert.rejects(assets.load("deepfilter"), /compile failed/);
  const prepared = await assets.load("deepfilter");
  assert.ok(prepared.module);
  assert.equal(compile.mock.callCount(), 2);
});

test("aborting one capture does not cancel shared preparation or another capture", async (t) => {
  const { assets, compile } = setup(t);
  let finish!: (module: WebAssembly.Module) => void;
  compile.mock.mockImplementation(() => new Promise<WebAssembly.Module>((resolve) => { finish = resolve; }));
  const controller = new AbortController();
  const cancelled = assets.load("deepfilter", controller.signal);
  const other = assets.load("deepfilter");
  await new Promise<void>((resolve) => setImmediate(resolve));
  controller.abort();
  await assert.rejects(cancelled, { name: "AbortError" });
  const module = {} as WebAssembly.Module;
  finish(module);
  assert.equal((await other).module, module);
  assert.equal((await assets.load("deepfilter")).module, module);
  assert.equal(compile.mock.callCount(), 1);
});

test("an already cancelled capture does not initiate downloads", async (t) => {
  const { assets, requests } = setup(t);
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(assets.load("deepfilter", controller.signal), { name: "AbortError" });
  assert.equal(requests.length, 0);
});
