import assert from "node:assert/strict";
import test, { type TestContext } from "node:test";
import { hasVoiceActivity, watchVoiceActivity } from "../media/voice-activity.ts";
import { playbackDiagnostics } from "../media/audio-context.ts";

function signal(rms: number) {
  return Float32Array.from({ length: 256 }, (_, index) => index % 2 ? rms : -rms);
}

test("quiet audible speech activates the speaking indicator", () => {
  assert.equal(hasVoiceActivity(signal(0.006)), true);
});

test("suppressed background noise does not activate the speaking indicator", () => {
  assert.equal(hasVoiceActivity(signal(0.002)), false);
});

function audioEnvironment(t: TestContext) {
  const document = Object.assign(new EventTarget(), { visibilityState: "visible" });
  const timers = new Map<number, () => void>();
  let nextTimer = 0;
  let now = 0;
  let rms = 0;
  let reads = 0;
  let closed = 0;
  let disconnected = 0;
  let failSetup = false;
  const changes: boolean[] = [];
  const stream = {} as MediaStream;
  class Context {
    createMediaStreamSource() { return { connect() {}, disconnect() { disconnected++; } }; }
    createAnalyser() {
      if (failSetup) throw new Error("analyser unavailable");
      return {
        fftSize: 256,
        getFloatTimeDomainData(samples: Float32Array) { samples.set(signal(rms)); reads++; },
        disconnect() { disconnected++; },
      };
    }
    async resume() {}
    async close() { closed++; }
  }
  for (const [name, value] of Object.entries({
    document, AudioContext: Context,
    window: {
      setInterval(callback: () => void, delay: number) {
        assert.equal(delay, 32);
        const id = nextTimer++;
        timers.set(id, callback);
        return id;
      },
      clearInterval(id: number) { timers.delete(id); },
    },
  })) {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, name);
    Object.defineProperty(globalThis, name, { configurable: true, value });
    t.after(() => {
      if (descriptor) Object.defineProperty(globalThis, name, descriptor);
      else Reflect.deleteProperty(globalThis, name);
    });
  }
  t.mock.method(performance, "now", () => now);
  return {
    stream, changes, timers,
    get reads() { return reads; }, get closed() { return closed; }, get disconnected() { return disconnected; },
    failSetup() { failSetup = true; },
    sample(at: number, level: number) { now = at; rms = level; for (const callback of timers.values()) callback(); },
    visibility(state: string) { document.visibilityState = state; document.dispatchEvent(new Event("visibilitychange")); },
    watch: (muted = false, input: MediaStream | undefined = stream) => watchVoiceActivity(input, muted, (active) => changes.push(active)),
  };
}

test("muted or missing streams and failed setup allocate no sampling loop", (t) => {
  const env = audioEnvironment(t);
  env.watch(true)();
  watchVoiceActivity(undefined, false, () => assert.fail("missing stream cannot speak"))();
  assert.equal(playbackDiagnostics().contextUsers, 0);
  assert.equal(env.timers.size, 0);
  env.failSetup();
  env.watch()();
  assert.equal(env.timers.size, 0);
  assert.equal(env.closed, 1);
  assert.equal(env.disconnected, 1, "disconnect the source when analyser setup fails");
  assert.equal(playbackDiagnostics().contextUsers, 0);
});

test("speech uses the same threshold and 180ms release, and cleanup clears a speaking ring", (t) => {
  const env = audioEnvironment(t);
  const stop = env.watch();
  env.sample(32, 0.006);
  env.sample(64, 0.006);
  env.sample(243, 0.002);
  assert.deepEqual(env.changes, [true], "hold quiet gaps shorter than 180ms without repeated UI updates");
  env.sample(244, 0.002);
  assert.deepEqual(env.changes, [true, false]);
  env.sample(256, 0.006);
  stop();
  assert.deepEqual(env.changes, [true, false, true, false]);
  assert.equal(env.timers.size, 0);
  assert.equal(env.disconnected, 2);
  assert.equal(env.closed, 1);
});

test("hidden tabs stop UI sampling without closing audio, then resume with one loop", (t) => {
  const env = audioEnvironment(t);
  env.visibility("hidden");
  const stop = env.watch();
  assert.equal(env.timers.size, 0);
  assert.equal(env.reads, 0);
  env.visibility("visible");
  env.visibility("visible");
  assert.equal(env.timers.size, 1);
  env.sample(32, 0.006);
  env.visibility("hidden");
  const reads = env.reads;
  env.sample(1_000, 0.006);
  assert.equal(env.reads, reads);
  assert.deepEqual(env.changes, [true, false]);
  assert.equal(env.closed, 0, "hiding UI must not close a context also used by playback");
  env.sample(1_100, 0.002);
  env.visibility("visible");
  assert.equal(env.timers.size, 1);
  assert.deepEqual(env.changes, [true, false], "do not restore a stale speaking ring");
  stop();
  env.visibility("visible");
  assert.equal(env.timers.size, 0, "cleanup removes the visibility listener");
  assert.equal(env.closed, 1);
});
