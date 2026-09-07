import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { DpdfnetPreparation } from "../media/dpdfnet-preparation.ts";

class WorkerMock {
  static instances: WorkerMock[] = [];
  readonly url: string;
  onmessage: ((event: { data: unknown }) => void) | null = null;
  onerror: (() => void) | null = null;
  terminateCalls = 0;
  constructor(url: string) { this.url = url; WorkerMock.instances.push(this); }
  terminate() { this.terminateCalls++; }
  postMessage() { assert.fail("Preparation must not process microphone samples"); }
  emit(type: string) { this.onmessage?.({ data: { type } }); }
}

function setup(t: TestContext) {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "Worker");
  Object.defineProperty(globalThis, "Worker", { value: WorkerMock, configurable: true });
  WorkerMock.instances = [];
  const preparation = new DpdfnetPreparation();
  t.after(() => {
    preparation.stop();
    if (descriptor) Object.defineProperty(globalThis, "Worker", descriptor);
    else Reflect.deleteProperty(globalThis, "Worker");
  });
  return { preparation, workers: WorkerMock.instances };
}

test("preparation shares one idle worker and only resolves after its warm-up acknowledgement", async (t) => {
  const { preparation, workers } = setup(t);
  let ready = false;
  const first = preparation.prepare("dpdfnet8").then(() => { ready = true; });
  const second = preparation.prepare("dpdfnet8");
  assert.equal(workers.length, 1);
  assert.equal(workers[0].url, "/audio/dpdfnet8-v1/worker.js");
  await Promise.resolve();
  assert.equal(ready, false);
  workers[0].emit("ready");
  await Promise.all([first, second]);
  await preparation.prepare("dpdfnet8");
  assert.equal(workers.length, 1);
  const owned = preparation.take("dpdfnet8");
  await owned.ready;
  assert.equal(owned.worker, workers[0]);
  preparation.stop();
  assert.equal(workers[0].terminateCalls, 0, "page cache no longer owns the captured worker");
  owned.stop();
  owned.stop();
  assert.equal(workers[0].terminateCalls, 1);
});

test("simultaneous captures and later joins never share recurrent or DSP state", async (t) => {
  const { preparation, workers } = setup(t);
  const first = preparation.take("dpdfnet8");
  const second = preparation.take("dpdfnet8");
  assert.notEqual(first.worker, second.worker);
  workers.forEach((worker) => worker.emit("ready"));
  await Promise.all([first.ready, second.ready]);
  first.stop();
  second.stop();
  const later = preparation.take("dpdfnet8");
  assert.equal(workers.length, 3);
  assert.notEqual(later.worker, first.worker);
  workers[2].emit("ready");
  await later.ready;
  later.stop();
});

test("stopping unfinished preparation terminates the worker and permits a fresh retry", async (t) => {
  const { preparation, workers } = setup(t);
  const pending = preparation.prepare("dpdfnet8");
  const rejected = assert.rejects(pending, /stopped/);
  preparation.stop();
  await rejected;
  assert.equal(workers[0].terminateCalls, 1);
  const retry = preparation.prepare("dpdfnet8");
  workers[1].emit("ready");
  await retry;
});

for (const failure of ["message", "error", "timeout"] as const) test(`failed preparation is terminated and evicted (${failure})`, async (t) => {
  const { preparation, workers } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const pending = preparation.prepare("dpdfnet8");
  const rejected = assert.rejects(pending, /failed|timed out/);
  if (failure === "message") workers[0].emit("failed");
  else if (failure === "error") workers[0].onerror!();
  else t.mock.timers.tick(60_000);
  await rejected;
  assert.equal(workers[0].terminateCalls, 1);
  const retry = preparation.prepare("dpdfnet8");
  workers[1].emit("ready");
  await retry;
  t.mock.timers.tick(60_000);
  assert.equal(workers[1].terminateCalls, 0, "ready workers must not time out");
});

test("an idle worker failure after readiness is also evicted", async (t) => {
  const { preparation, workers } = setup(t);
  const pending = preparation.prepare("dpdfnet8");
  workers[0].emit("ready");
  await pending;
  workers[0].onerror!();
  const retry = preparation.prepare("dpdfnet8");
  assert.equal(workers[0].terminateCalls, 1);
  assert.equal(workers.length, 2);
  workers[1].emit("ready");
  await retry;
});

test("a capture can cancel an exclusively transferred worker before readiness", async (t) => {
  const { preparation, workers } = setup(t);
  const owned = preparation.take("dpdfnet8");
  const rejected = assert.rejects(owned.ready, /stopped/);
  const lateReady = workers[0].onmessage!;
  owned.stop();
  lateReady({ data: { type: "ready" } });
  await rejected;
  assert.equal(workers[0].terminateCalls, 1);
});
