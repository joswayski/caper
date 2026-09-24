import assert from "node:assert/strict";
import { test } from "node:test";
import { getSystemSoundsEnabled, playSliderTick, playSound, preloadSoundEffects, setSystemSoundsEnabled } from "../audio/effects.ts";

const flush = () => new Promise<void>((resolve) => setImmediate(resolve));

test("effects preload decoded buffers and bound immediate Web Audio playback", async (t) => {
  class Source extends EventTarget {
    buffer?: AudioBuffer;
    playbackRate = { value: 1 };
    started = false;
    stopped = false;
    connect() { return this; }
    disconnect() {}
    start() { this.started = true; }
    stop() { this.stopped = true; }
  }
  class Gain {
    gain = { value: 1 };
    connect() { return this; }
    disconnect() {}
  }
  const sources: Source[] = [];
  const gains: Gain[] = [];
  const decoded: ArrayBuffer[] = [];
  let currentContext!: Context;
  class Context {
    state: AudioContextState = "suspended";
    destination = {};
    resumeCalls = 0;
    constructor(_options: AudioContextOptions) { currentContext = this; }
    resume() { this.resumeCalls++; this.state = "running"; return Promise.resolve(); }
    decodeAudioData(data: ArrayBuffer) { decoded.push(data); return Promise.resolve({} as AudioBuffer); }
    createBufferSource() { const source = new Source(); sources.push(source); return source; }
    createGain() { const gain = new Gain(); gains.push(gain); return gain; }
  }
  const requests: string[] = [];
  let warningAttempts = 0;
  let resolveWarning!: (response: Response) => void;
  const originalContext = Object.getOwnPropertyDescriptor(globalThis, "AudioContext");
  Object.defineProperty(globalThis, "AudioContext", { configurable: true, value: Context });
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    const url = String(input);
    requests.push(url);
    if (url.endsWith("/warning.wav")) {
      warningAttempts++;
      if (warningAttempts === 1) return new Response(null, { status: 500 });
      return new Promise<Response>((resolve) => { resolveWarning = resolve; });
    }
    return new Response(new Uint8Array([requests.length]));
  });
  t.after(() => {
    if (originalContext) Object.defineProperty(globalThis, "AudioContext", originalContext);
    else Reflect.deleteProperty(globalThis, "AudioContext");
  });

  await preloadSoundEffects();
  assert.equal(requests.length, 9, "every effect, including disconnect, is fetched once");
  assert.equal(decoded.length, 8, "failed downloads are not decoded");
  assert.ok(requests.includes("/audio/effects/disconnect.wav"));
  assert.ok(requests.includes("/audio/effects/channel-leave.wav"));

  const originalMatchMedia = Object.getOwnPropertyDescriptor(globalThis, "matchMedia");
  Object.defineProperty(globalThis, "matchMedia", { configurable: true, value: (query: string) => ({ matches: query === "(pointer: coarse)" }) });
  try {
    await preloadSoundEffects();
    playSound("toggle-on");
    playSound("channel-leave");
    playSliderTick(.5);
    await flush();
    assert.equal(sources.length, 0, "mobile never starts a sound, including cached effects");
    assert.equal(currentContext.resumeCalls, 0, "mobile never unlocks audio for decorative effects");
    assert.equal(requests.length, 9, "mobile preloading makes no requests");
  } finally {
    if (originalMatchMedia) Object.defineProperty(globalThis, "matchMedia", originalMatchMedia);
    else Reflect.deleteProperty(globalThis, "matchMedia");
  }

  let now = 1_000;
  t.mock.method(performance, "now", () => now);
  playSound("channel-join", { volume: 2, playbackRate: 3 });
  await flush();
  assert.equal(requests.filter((url) => url.endsWith("/channel-join.wav")).length, 1, "decoded buffers are reused");
  assert.equal(sources[0].playbackRate.value, 2, "rate is capped");
  assert.equal(gains[0].gain.value, 0.6, "volume is capped then reduced by 40%");
  assert.equal((sources[0] as unknown as { stopped: boolean }).stopped, false);
  assert.equal((sources[0].buffer as AudioBuffer | undefined) !== undefined, true);

  playSliderTick(0);
  await flush();
  assert.equal(sources.at(-1)?.playbackRate.value, 0.75);
  assert.equal(gains.at(-1)?.gain.value, 0.06);
  now += 39;
  playSliderTick(1);
  await flush();
  const throttledCount = sources.length;
  now++;
  playSliderTick(1);
  await flush();
  assert.equal(sources.length, throttledCount + 1, "slider ticks are throttled at 40ms");
  assert.equal(sources.at(-1)?.playbackRate.value, 1.35);
  assert.equal(gains.at(-1)?.gain.value, 0.192);

  for (let i = 0; i < 4; i++) playSound("toggle-on");
  await flush();
  assert.equal(sources.filter((source) => source.started && !source.stopped).length, 4, "oldest voices are stopped at the overlap cap");

  playSound("warning");
  const beforeStale = sources.length;
  now += 121;
  resolveWarning(new Response(new Uint8Array([9])));
  await flush();
  assert.equal(sources.length, beforeStale, "a slow decode does not play a stale interaction");
  playSound("warning");
  await flush();
  assert.equal(sources.length, beforeStale + 1, "a failed preload is retryable and its decoded retry is cached");
  let resume!: () => void;
  currentContext.state = "suspended";
  t.mock.method(currentContext, "resume", () => new Promise<void>((resolve) => { resume = () => { currentContext.state = "running"; resolve(); }; }));
  const beforeResume = sources.length;
  playSound("toggle-on");
  await flush();
  assert.equal(sources.length, beforeResume, "never queue a source in a suspended context");
  now += 121;
  resume();
  await flush();
  assert.equal(sources.length, beforeResume, "late autoplay permission cannot replay stale clicks");
  playSound("toggle-on");
  await flush();
  assert.equal(sources.length, beforeResume + 1);
  const beforeDisable = sources.length;
  playSound("toggle-on");
  setSystemSoundsEnabled(false);
  assert.equal(getSystemSoundsEnabled(), false);
  assert.equal(sources.filter(source => source.started && !source.stopped).length, 0, "disabling stops currently playing effects");
  playSound("delete");
  await preloadSoundEffects();
  setSystemSoundsEnabled(true);
  await flush();
  assert.equal(sources.length, beforeDisable, "a queued sound stays cancelled even after re-enabling");
  playSound("toggle-on");
  await flush();
  assert.equal(sources.length, beforeDisable + 1);
  assert.equal(gains.at(-1)?.gain.value, 0.27, "default effect gain is also reduced by 40%");
});

test("effects remain best-effort when Web Audio is unavailable", () => {
  assert.doesNotThrow(() => playSound("delete"));
});
