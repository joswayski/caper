import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { appleMobileWebKit, captureMicrophone, type NoiseSuppression } from "../media/microphone.ts";
import { NoiseAssets } from "../media/noise-assets.ts";
import { DpdfnetPreparation } from "../media/dpdfnet-preparation.ts";

class Track {
  enabled = true;
  readyState = "live";
  settings: MediaTrackSettings = { noiseSuppression: false };
  stopCalls = 0;
  stop() { this.stopCalls++; this.readyState = "ended"; }
  async applyConstraints(constraints: MediaTrackConstraints) {
    if (typeof constraints.noiseSuppression === "boolean") {
      this.settings.noiseSuppression = constraints.noiseSuppression;
    }
  }
  getSettings() { return this.settings; }
}

class Stream {
  readonly tracks: Track[];
  constructor(tracks: Track[]) { this.tracks = tracks; }
  getTracks() { return this.tracks; }
  getAudioTracks() { return this.tracks; }
}

class Port {
  onmessage: ((event: { data: unknown }) => void) | null = null;
  messages: unknown[] = [];
  closed = false;
  addEventListener() {}
  start() {}
  postMessage(message: unknown) { this.messages.push(message); }
  close() { this.closed = true; }
  emit(data: unknown) { this.onmessage?.({ data }); }
}

class WorkletNode {
  static latest: WorkletNode | undefined;
  readonly port = new Port();
  onprocessorerror: (() => void) | null = null;
  connections: unknown[] = [];
  disconnectCalls = 0;
  readonly options: any;
  constructor(...args: unknown[]) { WorkletNode.latest = this; this.options = args[2]; }
  connect(target: unknown) { this.connections.push(target); return target; }
  disconnect() { this.disconnectCalls++; this.connections = []; }
}

class SourceNode {
  connections: unknown[] = [];
  disconnectCalls = 0;
  connect(target: unknown) { this.connections.push(target); return target; }
  disconnect() { this.disconnectCalls++; this.connections = []; }
}

class Parameter {
  value: number;
  constructor(value: number) { this.value = value; }
  cancelScheduledValues() {}
  setTargetAtTime(value: number) { this.value = value; }
  setValueAtTime(value: number) { this.value = value; }
}

class GainNode extends SourceNode {
  readonly gain = new Parameter(1);
  readonly context: Context;
  constructor(context: Context) { super(); this.context = context; }
}

class FilterNode extends SourceNode {
  type = "lowpass";
  readonly frequency = new Parameter(350);
  readonly Q = new Parameter(1);
  readonly gain = new Parameter(0);
}

class CompressorNode extends SourceNode {
  readonly threshold = new Parameter(-24);
  readonly knee = new Parameter(30);
  readonly ratio = new Parameter(12);
  readonly attack = new Parameter(0.003);
  readonly release = new Parameter(0.25);
}

class Context {
  static latest: Context | undefined;
  readonly sampleRate: number = 48_000;
  state: AudioContextState = "running";
  readonly source = new SourceNode();
  readonly gain = new GainNode(this);
  readonly voiceInput = new GainNode(this);
  readonly makeup = new GainNode(this);
  readonly gains = [this.gain, this.voiceInput, this.makeup];
  readonly filters = [new FilterNode(), new FilterNode(), new FilterNode()];
  readonly compressors = [new CompressorNode(), new CompressorNode()];
  filterIndex = 0;
  compressorIndex = 0;
  readonly processed = new Track();
  readonly natural = new Track();
  readonly destination = { stream: new Stream([this.processed]) };
  readonly naturalDestination = { stream: new Stream([this.natural]) };
  readonly destinations = [this.destination, this.naturalDestination];
  closeCalls = 0;
  suspendCalls = 0;
  resumeCalls = 0;
  audioWorklet = { addModule: async (_url: string) => undefined };
  constructor(..._args: unknown[]) { Context.latest = this; }
  async suspend() { this.suspendCalls++; this.state = "suspended"; }
  async resume() { this.resumeCalls++; this.state = "running"; }
  createMediaStreamSource(_stream: MediaStream) { return this.source; }
  createGainCalls = 0;
  createGain() { return this.gains[this.createGainCalls++]; }
  createBiquadFilter() { return this.filters[this.filterIndex++]; }
  createDynamicsCompressor() { return this.compressors[this.compressorIndex++]; }
  destinationIndex = 0;
  createMediaStreamDestination() { return this.destinations[this.destinationIndex++]; }
  async close() { this.closeCalls++; this.state = "closed"; }
}

function setup(t: TestContext, options: {
  fetch?: typeof fetch;
  getUserMedia?: (constraints?: { audio: MediaTrackConstraints }) => Promise<Stream>;
  context?: unknown;
  worklet?: unknown;
  compile?: (bytes: BufferSource) => Promise<WebAssembly.Module>;
} = {}) {
  const raw = new Track();
  const stream = new Stream([raw]);
  const fetches: Array<{ url: string; init?: RequestInit }> = [];
  const restore: Array<() => void> = [];
  const install = (key: string, value: unknown) => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { value, configurable: true });
    restore.push(() => descriptor
      ? Object.defineProperty(globalThis, key, descriptor)
      : Reflect.deleteProperty(globalThis, key));
  };
  WorkletNode.latest = undefined;
  Context.latest = undefined;
  install("navigator", { mediaDevices: { getUserMedia: options.getUserMedia ?? (async () => stream) } });
  install("AudioContext", options.context ?? Context);
  install("AudioWorkletNode", options.worklet ?? WorkletNode);
  install("fetch", options.fetch ?? (async (url: string | URL | Request, init?: RequestInit) => {
    fetches.push({ url: String(url), init });
    return new Response(new Uint8Array([0, 97, 115, 109]));
  }));
  const compile = options.compile ?? (async () => ({}) as WebAssembly.Module);
  t.mock.method(WebAssembly, "compile", compile);
  t.after(() => restore.reverse().forEach((fn) => fn()));
  return { raw, stream, fetches, install };
}

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

for (const browserSuppression of [true, false]) test(`DPDFNet runtime overload keeps the track live when browser suppression is ${browserSuppression ? "available" : "unavailable"}`, async (t) => {
  const { install, raw } = setup(t);
  if (!browserSuppression) raw.applyConstraints = async () => undefined;
  let changes = 0;
  let worker!: { onmessage?: (event: { data: unknown }) => void; terminated: boolean };
  install("Worker", class {
    onmessage?: (event: { data: unknown }) => void;
    terminated = false;
    constructor(url: string) {
      if (url === "/audio/dpdfnet8-v2/worker.js") worker = this;
      else {
        assert.equal(url, "/audio/dpdfnet2-v1/worker.js");
        queueMicrotask(() => this.onmessage?.({ data: { type: "failed" } }));
      }
    }
    terminate() { this.terminated = true; }
  });
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => { changes++; });
  await tick();
  worker.onmessage!({ data: { type: "ready" } });
  const microphone = await capturing;
  assert.equal(microphone.track, Context.latest!.processed);
  assert.ok(microphone.status.startsWith("DPDFNet-8 HR active"));
  microphone.track.enabled = false;
  const original = WorkletNode.latest!;
  original.port.emit("bypassed");
  await tick();
  assert.equal(microphone.track, Context.latest!.processed);
  assert.equal(microphone.track.enabled, false);
  assert.match(microphone.status, browserSuppression ? /unavailable · browser suppression active/ : /unavailable - noise suppression bypassed/);
  assert.equal(raw.getSettings().noiseSuppression, browserSuppression);
  assert.equal(microphone.track.readyState, "live");
  assert.equal(worker.terminated, true);
  assert.equal(changes, 2);
  const replacement = WorkletNode.latest!;
  assert.notEqual(replacement, original);
  assert.equal(replacement.options.processorOptions.engine, "rnnoise");
  replacement.port.emit("ready");
  await tick();
  assert.match(microphone.status, /DPDFNet unavailable · RNNoise active/);
  assert.equal(microphone.track, Context.latest!.processed);
  assert.equal(microphone.naturalTrack, Context.latest!.natural);
  assert.equal(microphone.track.enabled, false, "fallback must not unmute outgoing audio");
  assert.equal(raw.getSettings().noiseSuppression, false);
  assert.deepEqual(Context.latest!.gain.connections, [replacement]);
  assert.deepEqual(replacement.connections, [Context.latest!.naturalDestination, Context.latest!.voiceInput]);
  assert.equal(original.port.closed, true);
  assert.equal(changes, 3);
  microphone.stop();
});

function preparedDpdfnet(t: TestContext) {
  const { install, raw } = setup(t);
  const workers: Array<{ onmessage?: (event: { data: unknown }) => void; onerror?: () => void; terminateCalls: number; url: string }> = [];
  install("Worker", class {
    onmessage?: (event: { data: unknown }) => void;
    onerror?: () => void;
    terminateCalls = 0;
    url: string;
    constructor(url: string) { this.url = url; workers.push(this); }
    terminate() { this.terminateCalls++; }
  });
  const preparation = new DpdfnetPreparation();
  t.after(() => preparation.stop());
  return { preparation, workers, raw };
}

for (const end of ["cancel", "failed", "timeout"] as const) test(`RNNoise fallback ${end} never reconnects stale audio`, async (t) => {
  const { preparation, workers } = preparedDpdfnet(t);
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "headphones", undefined, preparation);
  await tick();
  workers[0].onmessage!({ data: { type: "ready" } });
  const microphone = await capturing;
  const original = WorkletNode.latest!;
  t.mock.timers.enable({ apis: ["setTimeout"] });
  original.port.emit("bypassed");
  await tick();
  workers[1].onmessage!({ data: { type: "failed" } });
  await tick();
  workers[2].onmessage!({ data: { type: "failed" } });
  await tick();
  const replacement = WorkletNode.latest!;
  assert.notEqual(replacement, original);
  if (end === "cancel") microphone.stop();
  else if (end === "failed") replacement.port.emit("failed");
  else t.mock.timers.tick(15_000);
  await tick();
  replacement.port.emit("ready");
  await tick();
  if (end !== "cancel") {
    const retry = WorkletNode.latest!;
    assert.notEqual(retry, replacement);
    retry.port.emit("failed");
    await tick();
    assert.equal(retry.port.closed, true);
  }
  assert.equal(replacement.port.closed, true);
  assert.ok(!Context.latest!.gain.connections.includes(replacement));
  if (end === "cancel") assert.equal(microphone.track.readyState, "ended");
  else {
    assert.equal(microphone.track.readyState, "live");
    assert.match(microphone.status, /browser suppression active/);
  }
  microphone.stop();
});

for (const warmed of [false, true]) test(`capture exclusively consumes DPDFNet preparation (already ready: ${warmed})`, async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  raw.settings = { sampleRate: 48000, deviceId: "private-device", groupId: "private-group" };
  const warming = preparation.prepare();
  assert.equal(Context.latest, undefined, "preparation must not open an audio context");
  if (warmed) {
    workers[0].onmessage!({ data: { type: "ready" } });
    await warming;
  }
  let ready = false;
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "headphones", new NoiseAssets(), preparation)
    .then((microphone) => { ready = true; return microphone; });
  await tick();
  assert.equal(workers.length, 1, "Join must not create another model instance");
  if (!warmed) {
    WorkletNode.latest!.port.emit("ready");
    await tick();
    assert.equal(ready, false, "worklet readiness cannot bypass worker initialization");
    assert.equal(Context.latest!.source.connections.length, 0);
    workers[0].onmessage!({ data: { type: "ready" } });
    await warming;
  }
  const microphone = await capturing;
  assert.equal(microphone.track, Context.latest!.processed);
  assert.match(microphone.status, /DPDFNet-8 HR active/);
  for (const duration of [4, 14, 6]) workers[0].onmessage!({ data: { type: "output", duration, samples: new ArrayBuffer(4) } });
  const report = JSON.parse(JSON.stringify(microphone.diagnostics()));
  assert.deepEqual(report.dpdfnet, { profile: 8, processedHops: 3, meanProcessingMs: 8, maxProcessingMs: 14, hopBudgetMs: 10 });
  assert.equal(workers.length, 1, "healthy 8 HR never initializes a fallback");
  assert.deepEqual(report.capture, { sampleRate: 48000 });
  assert.equal(report.status, microphone.status);
  preparation.stop();
  assert.equal(workers[0].terminateCalls, 0);
  microphone.stop();
  assert.equal(workers[0].terminateCalls, 1);
});

for (const readyThenAbort of [false, true]) test(`cancel releases the transferred DPDFNet worker (ready/abort race: ${readyThenAbort})`, async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  const controller = new AbortController();
  const capturing = captureMicrophone(undefined, "dpdfnet8", controller.signal, () => undefined, "headphones", new NoiseAssets(), preparation);
  const rejected = assert.rejects(capturing, { name: "AbortError" });
  await tick();
  if (readyThenAbort) workers[0].onmessage!({ data: { type: "ready" } });
  controller.abort();
  await rejected;
  assert.equal(workers[0].terminateCalls, 1);
  assert.equal(raw.readyState, "ended");
  assert.equal(Context.latest!.state, "closed");
  assert.equal(Context.latest!.source.connections.length, 0);
});

test("all local initialization failures stop capture rather than returning raw audio", async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "headphones", new NoiseAssets(), preparation);
  const rejected = assert.rejects(capturing, /DPDFNet-8 HR could not start/);
  await tick();
  for (let i = 0; i < 4; i++) {
    assert.equal(workers[i].url, i < 2 ? "/audio/dpdfnet8-v2/worker.js" : "/audio/dpdfnet2-v1/worker.js");
    workers[i].onmessage!({ data: { type: "failed" } });
    await tick();
    assert.equal(Context.latest!.source.connections.length, 0);
    assert.equal(workers[i].terminateCalls, 1);
  }
  for (let i = 0; i < 2; i++) {
    assert.equal(WorkletNode.latest!.options.processorOptions.engine, "rnnoise");
    WorkletNode.latest!.port.emit("failed");
    await tick();
  }
  await rejected;
  assert.equal(workers[0].terminateCalls, 1);
  assert.equal(raw.readyState, "ended");
  assert.equal(Context.latest!.source.connections.length, 0);
});

for (const failure of ["startup", "overload", "processorerror", "workererror", "inference"] as const) test(`DPDFNet ${failure} retries only errors, with a per-capture budget`, async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "headphones", undefined, preparation, 73);
  await tick();
  workers[0].onmessage!({ data: { type: failure === "startup" ? "failed" : "ready" } });
  if (failure !== "startup") {
    const microphone = await capturing;
    microphone.track.enabled = false;
    if (failure === "overload") WorkletNode.latest!.port.emit("bypassed");
    else if (failure === "processorerror") WorkletNode.latest!.onprocessorerror!();
    else if (failure === "workererror") workers[0].onerror!();
    else workers[0].onmessage!({ data: { type: "failed" } });
  }
  await tick();
  assert.equal(workers.length, 2);
  if (failure !== "overload") {
    assert.equal(workers[1].url, "/audio/dpdfnet8-v2/worker.js", "first error must retry 8 HR");
    workers[1].onmessage!({ data: { type: "ready" } });
    const microphone = await capturing;
    await tick();
    assert.match(microphone.status, /DPDFNet-8 HR active/);
    assert.equal(microphone.track.enabled, failure === "startup");
    WorkletNode.latest!.onprocessorerror!();
    await tick();
    assert.equal(workers.length, 3, "retry budget does not reset after successful recovery");
  }
  const smallerWorker = workers.at(-1)!;
  assert.equal(smallerWorker.url, "/audio/dpdfnet2-v1/worker.js");
  const smaller = WorkletNode.latest!;
  smallerWorker.onmessage!({ data: { type: "ready" } });
  await tick();
  const microphone = await capturing;
  assert.match(microphone.status, /DPDFNet-2 HR active/);
  assert.equal(raw.settings.noiseSuppression, false);
  assert.equal(Context.latest!.gain.gain.value, .73);
  assert.equal(microphone.track, Context.latest!.processed);
  assert.equal(microphone.naturalTrack, Context.latest!.natural);
  assert.equal(microphone.track.enabled, failure === "startup");
  assert.deepEqual(Context.latest!.gain.connections, [smaller]);
  smallerWorker.onmessage!({ data: { type: "output", duration: 2, samples: new ArrayBuffer(4) } });
  assert.deepEqual((microphone.diagnostics() as any).dpdfnet, { profile: 2, processedHops: 1, meanProcessingMs: 2, maxProcessingMs: 2, hopBudgetMs: 10 });
  assert.equal(workers.length, failure === "overload" ? 2 : 3);
  smaller.port.emit("bypassed");
  await tick();
  const rnnoise = WorkletNode.latest!;
  assert.equal(rnnoise.options.processorOptions.engine, "rnnoise");
  rnnoise.port.emit("ready");
  await tick();
  assert.match(microphone.status, /RNNoise active/);
  assert.equal(smallerWorker.terminateCalls, 1);
  assert.equal(microphone.track.enabled, failure === "startup");
  assert.equal(Context.latest!.gain.gain.value, .73);
  microphone.stop();
});

test("2 HR and RNNoise each get one crash retry, then advance or stop", async (t) => {
  const { preparation, workers } = preparedDpdfnet(t);
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "headphones", undefined, preparation);
  await tick();
  workers[0].onmessage!({ data: { type: "ready" } });
  const microphone = await capturing;
  microphone.track.enabled = false;
  WorkletNode.latest!.port.emit("bypassed");
  await tick();
  workers[1].onmessage!({ data: { type: "ready" } });
  await tick();
  WorkletNode.latest!.onprocessorerror!();
  await tick();
  assert.equal(workers[2].url, "/audio/dpdfnet2-v1/worker.js");
  assert.equal(workers[1].terminateCalls, 1);
  workers[2].onmessage!({ data: { type: "ready" } });
  await tick();
  assert.match(microphone.status, /DPDFNet-2 HR active/);
  WorkletNode.latest!.onprocessorerror!();
  await tick();
  const firstRnnoise = WorkletNode.latest!;
  assert.equal(firstRnnoise.options.processorOptions.engine, "rnnoise");
  firstRnnoise.port.emit("ready");
  await tick();
  firstRnnoise.onprocessorerror!();
  await tick();
  const retry = WorkletNode.latest!;
  assert.notEqual(retry, firstRnnoise);
  assert.equal(retry.options.processorOptions.engine, "rnnoise");
  retry.port.emit("ready");
  await tick();
  assert.equal(microphone.track.enabled, false);
  assert.equal(microphone.track, Context.latest!.processed);
  assert.equal(firstRnnoise.port.closed, true);
  retry.onprocessorerror!();
  await tick();
  assert.equal(WorkletNode.latest, retry, "no third RNNoise instance");
  assert.match(microphone.status, /RNNoise failed - microphone stopped/);
  assert.equal(microphone.track.readyState, "ended");
});

test("DPDFNet startup timeout retries once; aborting the retry releases every track", async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const controller = new AbortController();
  const capturing = captureMicrophone(undefined, "dpdfnet8", controller.signal, () => undefined, "headphones", undefined, preparation);
  const rejected = assert.rejects(capturing, { name: "AbortError" });
  await tick();
  t.mock.timers.tick(60_000);
  await tick();
  assert.equal(workers.length, 2);
  assert.equal(workers[1].url, "/audio/dpdfnet8-v2/worker.js");
  assert.equal(workers[0].terminateCalls, 1);
  assert.deepEqual(Context.latest!.source.connections, []);
  const lateReady = workers[1].onmessage!;
  controller.abort();
  lateReady({ data: { type: "ready" } });
  await rejected;
  assert.equal(workers.length, 2);
  assert.equal(workers[1].terminateCalls, 1);
  assert.equal(raw.readyState, "ended");
  assert.equal(Context.latest!.processed.readyState, "ended");
  assert.equal(WorkletNode.latest!.port.closed, true);
});

test("stopping during 2 HR initialization cannot reconnect or start RNNoise", async (t) => {
  const { preparation, workers } = preparedDpdfnet(t);
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "headphones", undefined, preparation);
  await tick();
  workers[0].onmessage!({ data: { type: "ready" } });
  const microphone = await capturing;
  WorkletNode.latest!.port.emit("bypassed");
  await tick();
  const smaller = WorkletNode.latest!;
  const ready = workers[1].onmessage!;
  microphone.stop();
  ready({ data: { type: "ready" } });
  await tick();
  assert.equal(workers[1].terminateCalls, 1);
  assert.equal(smaller.port.closed, true);
  assert.equal(WorkletNode.latest, smaller);
  assert.equal(microphone.track.readyState, "ended");
  assert.deepEqual(Context.latest!.gain.connections, []);
});

test("headphones opt out of echo and automatic level processing; speakers retain both", async (t) => {
  setup(t);
  const captured: MediaTrackConstraints[] = [];
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: {
    mediaDevices: { getUserMedia: async ({ audio }: MediaStreamConstraints) => {
      captured.push(audio as MediaTrackConstraints);
      return new Stream([new Track()]);
    } },
  } });
  for (const setup of ["headphones", "speakers"] as const) {
    const microphone = await captureMicrophone(undefined, "off", new AbortController().signal, () => undefined, setup);
    microphone.stop();
  }
  assert.deepEqual(captured.map((c) => [c.echoCancellation, c.autoGainControl, c.noiseSuppression]), [[false, false, false], [true, true, false]]);
});

test("every enhanced mode selects the intended engine and preset", async (t) => {
  for (const [mode, attenuation] of [["deepfilter", 20], ["deepfilter-gentle", 12], ["deepfilter-strong", 40], ["rnnoise", 20]] as const) {
    await t.test(mode, async (t) => {
      const { fetches } = setup(t);
      const capturing = captureMicrophone(undefined, mode, new AbortController().signal, () => undefined);
      await tick();
      const { engine, attenuationLimit, model } = WorkletNode.latest!.options.processorOptions;
      assert.equal(engine, mode === "rnnoise" ? "rnnoise" : "deepfilter");
      assert.equal(attenuationLimit, attenuation);
      if (mode === "rnnoise") {
        assert.equal(model, undefined);
        assert.deepEqual(fetches.map(({ url }) => url), ["/audio/rnnoise-v1/rnnoise.wasm"]);
      }
      WorkletNode.latest!.port.emit("ready");
      const microphone = await capturing;
      assert.match(microphone.status, /active/);
      microphone.stop();
    });
  }
});

test("browser baseline skips WASM and honestly reports unsupported suppression", async (t) => {
  const { raw, fetches } = setup(t);
  const microphone = await captureMicrophone(undefined, "browser", new AbortController().signal, () => undefined);
  assert.equal(microphone.track, raw);
  assert.equal(fetches.length, 0);
  assert.equal(Context.latest, undefined);
  assert.match(microphone.status, /unavailable/);
  microphone.stop();
});

const IPHONE_BRAVE = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1";

test("iPhone and iPad are detected in every WebKit browser, including iPadOS desktop mode", () => {
  assert.equal(appleMobileWebKit({ userAgent: IPHONE_BRAVE, platform: "iPhone", maxTouchPoints: 5 }), true);
  assert.equal(appleMobileWebKit({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15", platform: "MacIntel", maxTouchPoints: 5 }), true);
  assert.equal(appleMobileWebKit({ userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15", platform: "MacIntel", maxTouchPoints: 0 }), false);
  assert.equal(appleMobileWebKit({ userAgent: "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 Chrome/140.0 Mobile Safari/537.36", platform: "Linux armv8l", maxTouchPoints: 5 }), false);
  assert.equal(appleMobileWebKit(undefined), false);
});

const report = (microphone: { diagnostics(): object }) => microphone.diagnostics() as { requested?: string; routeSampleRate?: number };

/** An iPhone whose default context (its audio route) runs at routeRate. */
function iphone(t: TestContext, routeRate: number, userAgent = IPHONE_BRAVE) {
  const requested: MediaTrackConstraints[] = [];
  const raw = new Track();
  const env = setup(t, { getUserMedia: async (constraints?: { audio: MediaTrackConstraints }) => {
    requested.push(constraints!.audio);
    raw.settings.noiseSuppression = !!constraints!.audio.noiseSuppression;
    return new Stream([raw]);
  } });
  const getUserMedia = (globalThis.navigator as Navigator).mediaDevices.getUserMedia;
  env.install("navigator", { userAgent, platform: userAgent === IPHONE_BRAVE ? "iPhone" : "MacIntel", maxTouchPoints: 5, mediaDevices: { getUserMedia } });
  const contexts: RoutedContext[] = [];
  class RoutedContext extends Context {
    readonly sampleRate: number;
    constructor(options?: AudioContextOptions) { super(); this.sampleRate = options?.sampleRate ?? routeRate; contexts.push(this); }
  }
  env.install("AudioContext", RoutedContext);
  let workers = 0;
  env.install("Worker", class {
    onmessage?: (event: { data: unknown }) => void;
    constructor() { workers++; queueMicrotask(() => this.onmessage?.({ data: { type: "ready" } })); }
    terminate() {}
  });
  return { ...env, raw, requested, contexts, workers: () => workers };
}

for (const mode of ["dpdfnet8", "rnnoise", "deepfilter"] as NoiseSuppression[]) test(`on an iPhone 24 kHz route, ${mode} sends the browser-processed capture without Web Audio`, async (t) => {
  const { raw, requested, contexts, fetches, workers } = iphone(t, 24_000);
  const dpdfnet = new DpdfnetPreparation();
  const microphone = await captureMicrophone(undefined, mode, new AbortController().signal, () => undefined, "speakers", undefined, dpdfnet);
  assert.equal(microphone.track, raw);
  assert.equal(microphone.naturalTrack, raw);
  // Only the rate probe, closed at once; no capture graph.
  assert.deepEqual(contexts.map((context) => [context.sampleRate, context.closeCalls]), [[24_000, 1]]);
  assert.equal(fetches.length, 0);
  assert.equal(workers(), 0);
  // An unprepared DPDFNet already asks for browser suppression with the capture.
  assert.equal(requested[0].noiseSuppression, mode === "dpdfnet8");
  assert.equal(raw.settings.noiseSuppression, true);
  assert.equal(report(microphone).requested, mode);
  assert.equal(report(microphone).routeSampleRate, 24_000);
  assert.equal(microphone.status, "Browser suppression active · 24 kHz audio route, on-device models need 48 kHz");
  microphone.stop();
  assert.equal(raw.readyState, "ended");
});

test("on an iPhone 24 kHz route without browser suppression, the status says suppression is off", async (t) => {
  const { raw } = iphone(t, 24_000);
  raw.applyConstraints = async () => undefined;
  const microphone = await captureMicrophone(undefined, "rnnoise", new AbortController().signal, () => undefined);
  assert.equal(microphone.track, raw);
  assert.equal(microphone.status, "Browser suppression unavailable - noise suppression off · 24 kHz audio route, on-device models need 48 kHz");
  microphone.stop();
});

test("on an iPhone 48 kHz route the on-device model still runs", async (t) => {
  const { raw, contexts } = iphone(t, 48_000);
  const capturing = captureMicrophone(undefined, "rnnoise", new AbortController().signal, () => undefined);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  assert.notEqual(microphone.track, raw);
  assert.equal(microphone.status, "RNNoise active · on-device");
  assert.equal(report(microphone).routeSampleRate, 48_000);
  assert.equal(contexts.length, 2);
  microphone.stop();
});

test("other devices keep the model on a non-48 kHz output, which they resample", async (t) => {
  const { raw, contexts } = iphone(t, 44_100, "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Version/18.6 Safari/605.1.15");
  (globalThis.navigator as { maxTouchPoints: number }).maxTouchPoints = 0;
  const capturing = captureMicrophone(undefined, "rnnoise", new AbortController().signal, () => undefined);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  assert.notEqual(microphone.track, raw);
  assert.equal(microphone.status, "RNNoise active · on-device");
  assert.deepEqual(contexts.map((context) => context.sampleRate), [48_000]);
  microphone.stop();
});

for (const mode of ["off", "browser"] as NoiseSuppression[]) test(`on an iPhone, ${mode} needs no route probe`, async (t) => {
  const { raw, contexts } = iphone(t, 24_000);
  const microphone = await captureMicrophone(undefined, mode, new AbortController().signal, () => undefined);
  assert.equal(microphone.track, raw);
  assert.equal(contexts.length, 0);
  microphone.stop();
});

test("a missing capture stream reports a microphone access error", async (t) => {
  setup(t, { getUserMedia: async () => null as unknown as Stream });
  await assert.rejects(
    captureMicrophone(undefined, "off", new AbortController().signal, () => undefined),
    /Microphone access was not granted/,
  );
});

test("preloading never captures audio; warm capture still waits for the selected processor", async (t) => {
  let captures = 0;
  const { fetches } = setup(t, { getUserMedia: async () => { captures++; return new Stream([new Track()]); } });
  const assets = new NoiseAssets();
  await assets.load("deepfilter");
  assert.equal(captures, 0);
  assert.equal(Context.latest, undefined);
  assert.equal(WorkletNode.latest, undefined);
  let settled = false;
  const capturing = captureMicrophone(undefined, "deepfilter-gentle", new AbortController().signal, () => undefined, "speakers", assets)
    .then((microphone) => { settled = true; return microphone; });
  await tick();
  assert.equal(captures, 1);
  assert.equal(fetches.length, 2, "joining must reuse the preloaded assets");
  assert.equal(settled, false, "never return temporary raw audio while the filter is loading");
  assert.equal(WorkletNode.latest!.options.processorOptions.attenuationLimit, 12);
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  assert.equal(microphone.track, Context.latest!.processed);
  assert.match(microphone.status, /DeepFilterNet active · gentle/);
  microphone.stop();
});

test("DeepFilter waits for a valid ready acknowledgement", async (t) => {
  const { raw, fetches } = setup(t);
  const controller = new AbortController();
  let settled = false;
  const capturing = captureMicrophone(undefined, "deepfilter", controller.signal, () => undefined)
    .then((microphone) => { settled = true; return microphone; });

  await tick();
  assert.ok(WorkletNode.latest, "worklet should be constructed after local assets load");
  assert.equal(settled, false, "capture must not resolve before the processor acknowledges readiness");
  WorkletNode.latest.port.emit("not-ready");
  await assert.rejects(capturing, /DeepFilterNet could not start/);
  assert.equal(settled, false, "invalid readiness must not yield a lower-quality track");
  assert.equal(fetches.every(({ url }) => url.startsWith("/audio/deepfilter-v1/")), true);
  assert.equal(raw.readyState, "ended");
});

test("a ready acknowledgement exposes processed output and a natural mic-test tap", async (t) => {
  const { raw } = setup(t);
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  const context = Context.latest!;
  assert.notEqual(microphone.track, raw);
  assert.equal(microphone.track, context.processed);
  assert.equal(microphone.naturalTrack, context.natural);
  assert.deepEqual(context.source.connections, [context.gain]);
  assert.deepEqual(context.gain.connections, [WorkletNode.latest]);
  assert.deepEqual(WorkletNode.latest!.connections, [context.naturalDestination, context.voiceInput]);
  assert.deepEqual(context.voiceInput.connections, [context.filters[0]]);
  assert.deepEqual(context.compressors[1].connections, [context.destination]);
  assert.equal(context.filters[0].frequency.value, 18.75);
  assert.equal(context.filters[1].gain.value, 0.5);
  assert.equal(context.compressors[0].ratio.value, 1.5);
  assert.equal(context.makeup.gain.value, 1.35 ** 0.25);
  assert.equal(context.compressors[1].threshold.value, -2);
  assert.equal(microphone.status, "DeepFilterNet active · balanced · on-device");
  microphone.stop();
});

test("off mode skips every asset and retains echo cancellation capture constraints", async (t) => {
  let constraints: MediaStreamConstraints | undefined;
  const raw = new Track();
  const stream = new Stream([raw]);
  const { fetches } = setup(t, { getUserMedia: async () => stream });
  Object.defineProperty(globalThis, "navigator", { configurable: true, value: {
    mediaDevices: { getUserMedia: async (value: MediaStreamConstraints) => { constraints = value; return stream; } },
  } });
  const microphone = await captureMicrophone("device", "off", new AbortController().signal, () => undefined);
  assert.equal(fetches.length, 0);
  assert.equal((constraints!.audio as MediaTrackConstraints).echoCancellation, true);
  assert.deepEqual((constraints!.audio as MediaTrackConstraints).deviceId, { exact: "device" });
  assert.equal(microphone.track, raw);
  assert.equal(microphone.status, "Noise suppression off");
  microphone.stop();
});

test("input volume clamps to 0–200% and changes the outgoing gain without replacing the track", async (t) => {
  setup(t);
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined, "headphones", undefined, undefined, 200);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  const track = microphone.track;
  assert.equal(Context.latest!.gain.gain.value, 2);
  microphone.setInputVolume(-10);
  assert.equal(Context.latest!.gain.gain.value, 0);
  microphone.setInputVolume(250);
  assert.equal(Context.latest!.gain.gain.value, 2);
  assert.equal(microphone.track, track);
  microphone.stop();
});

test("voice processing strength scales smoothly and zero bypasses without replacing the outgoing track", async (t) => {
  setup(t);
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined, "headphones", undefined, undefined, 100, 0);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  const context = Context.latest!;
  const track = microphone.track;
  assert.deepEqual(context.voiceInput.connections, [context.destination], "zero strength bypasses tone and dynamics processing");

  microphone.setVoiceProcessingStrength(100);
  assert.equal(microphone.track, track);
  assert.equal(context.voiceInput.connections[0], context.filters[0]);
  assert.equal(context.filters[0].frequency.value, 75);
  assert.equal(context.filters[1].gain.value, 2);
  assert.equal(context.filters[2].gain.value, 1.5);
  assert.equal(context.compressors[0].ratio.value, 3);
  assert.equal(context.makeup.gain.value, 1.35);

  microphone.setVoiceProcessingStrength(-10);
  assert.deepEqual(context.voiceInput.connections, [context.destination]);
  microphone.stop();
});

test("initialization, download, and worklet failures stop capture rather than downgrade", async (t) => {
  class BrokenContext { constructor() { throw new Error("init"); } }
  class BrokenWorkletContext extends Context {
    override audioWorklet = { addModule: async () => { throw new Error("worklet"); } };
  }
  for (const [name, options] of [
    ["AudioContext initialization", { context: BrokenContext }],
    ["asset download", { fetch: async () => new Response(null, { status: 503 }) }],
    ["audio worklet module", { context: BrokenWorkletContext }],
  ] as const) {
    await t.test(name, async (t) => {
      const { raw } = setup(t, options);
      await assert.rejects(captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined), /DeepFilterNet could not start/);
      assert.equal(raw.readyState, "ended");
      assert.equal(raw.settings.noiseSuppression, false, "never request a fallback filter");
      if (Context.latest) assert.equal(Context.latest.state, "closed");
    });
  }
});

test("runtime processor failure stops output without replacing or unmuting the outgoing track", async (t) => {
  setup(t);
  let changes = 0;
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => changes++);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  const outgoing = microphone.track;
  outgoing.enabled = false;
  WorkletNode.latest!.onprocessorerror!();
  await tick();
  assert.equal(microphone.track, outgoing);
  assert.equal(outgoing.enabled, false);
  assert.deepEqual(Context.latest!.source.connections, []);
  assert.equal(outgoing.readyState, "ended");
  assert.equal(microphone.status, "DeepFilterNet failed - microphone stopped");
  assert.equal(changes, 1);
  microphone.stop();
});

test("enhanced capture can pause during replacement and resume after rollback", async (t) => {
  setup(t);
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  const context = Context.latest!;

  await microphone.pause();
  assert.equal(context.state, "suspended");
  assert.equal(context.suspendCalls, 1);
  await microphone.pause();
  assert.equal(context.suspendCalls, 1);

  await microphone.resume();
  assert.equal(context.state, "running");
  assert.equal(context.resumeCalls, 2, "capture startup and rollback each resume once");
  await microphone.resume();
  assert.equal(context.resumeCalls, 2);
  microphone.stop();
});

test("abort during asset download releases capture and abort after a late permission grant releases raw audio", async (t) => {
  await t.test("download", async (t) => {
    const controller = new AbortController();
    const { raw } = setup(t, { fetch: async (_url, init) => new Promise((_resolve, reject) => {
      init!.signal!.addEventListener("abort", () => reject(init!.signal!.reason), { once: true });
    }) });
    const capturing = captureMicrophone(undefined, "deepfilter", controller.signal, () => undefined);
    await tick();
    controller.abort();
    await assert.rejects(capturing, { name: "AbortError" });
    assert.equal(raw.readyState, "ended");
  });
  await t.test("permission", async (t) => {
    let grant!: (stream: Stream) => void;
    const controller = new AbortController();
    const raw = new Track();
    setup(t, { getUserMedia: () => new Promise((resolve) => { grant = resolve; }) });
    const capturing = captureMicrophone(undefined, "off", controller.signal, () => undefined);
    controller.abort();
    grant(new Stream([raw]));
    await assert.rejects(capturing, { name: "AbortError" });
    assert.equal(raw.readyState, "ended");
  });
});

test("stop is idempotent and releases raw and destination tracks and the audio context", async (t) => {
  const { raw } = setup(t);
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  const processed = Context.latest!.processed;
  const natural = Context.latest!.natural;
  microphone.stop();
  microphone.stop();
  assert.equal(raw.stopCalls, 1);
  assert.equal(processed.stopCalls, 1);
  assert.equal(natural.stopCalls, 1);
  assert.equal(Context.latest!.closeCalls, 1);
  assert.equal(WorkletNode.latest!.port.closed, true);
});

test("DeepFilter setup only downloads same-origin public assets without bodies or credentials", async (t) => {
  const { fetches } = setup(t);
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  assert.deepEqual(fetches.map(({ url }) => url).sort(), [
    "/audio/deepfilter-v1/DeepFilterNet3.bin",
    "/audio/deepfilter-v1/df_bg.wasm",
  ]);
  for (const { init } of fetches) {
    assert.equal(init?.body, undefined);
    assert.equal(init?.credentials, undefined);
  }
  microphone.stop();
});

/** getUserMedia that honours the requested browser noise suppression, like a real browser. */
function honourSuppression(raw: Track) {
  const original = navigator.mediaDevices.getUserMedia.bind(navigator.mediaDevices);
  navigator.mediaDevices.getUserMedia = (async (constraints: MediaStreamConstraints) => {
    raw.settings.noiseSuppression = (constraints.audio as MediaTrackConstraints).noiseSuppression === true;
    return original(constraints);
  }) as typeof navigator.mediaDevices.getUserMedia;
}

test("a compiling DPDFNet model does not hold Join: browser suppression carries audio until it swaps in", async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  honourSuppression(raw);
  void preparation.prepare(); // Still compiling, as right after page load or a leave.
  let changes = 0;
  const microphone = await captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => { changes++; }, "speakers", undefined, preparation);
  const context = Context.latest!;
  const node = WorkletNode.latest!;
  assert.match(microphone.status, /DPDFNet-8 HR loading · browser suppression active/);
  assert.equal(microphone.startup.interim, true);
  assert.equal(raw.getSettings().noiseSuppression, true, "never raw: the browser suppresses until DPDFNet is ready");
  assert.equal(microphone.track, context.processed);
  assert.deepEqual(context.source.connections, [context.gain]);
  assert.ok(context.gain.connections.includes(context.naturalDestination) && context.gain.connections.includes(context.voiceInput));
  assert.equal(node.connections.length, 0, "the model is not in the path yet");
  assert.equal(workers.length, 1, "the prepared worker is taken, not duplicated");

  workers[0].onmessage!({ data: { type: "ready" } });
  await tick();
  await tick();
  assert.match(microphone.status, /DPDFNet-8 HR active/);
  assert.equal(raw.getSettings().noiseSuppression, false, "browser suppression is released for DPDFNet");
  assert.deepEqual(context.gain.connections, [node]);
  assert.ok(node.connections.includes(context.naturalDestination) && node.connections.includes(context.voiceInput));
  assert.equal(microphone.track, context.processed, "the published track never changes");
  assert.equal(changes, 1);
  assert.equal(workers.length, 1);
  microphone.stop();
  assert.equal(workers[0].terminateCalls, 1);
});

test("without browser suppression, a compiling model still holds startup rather than publish raw audio", async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  void preparation.prepare();
  let ready = false;
  const capturing = captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "speakers", undefined, preparation)
    .then((microphone) => { ready = true; return microphone; });
  await tick();
  await tick();
  assert.equal(ready, false);
  assert.equal(raw.getSettings().noiseSuppression, false);
  assert.equal(Context.latest!.source.connections.length, 0, "no audio path before the model is ready");
  workers[0].onmessage!({ data: { type: "ready" } });
  const microphone = await capturing;
  assert.match(microphone.status, /DPDFNet-8 HR active/);
  assert.equal(microphone.startup.interim, false);
  microphone.stop();
});

test("a model that fails while loading hands over to the fallback chain on the same track", async (t) => {
  const { preparation, workers, raw } = preparedDpdfnet(t);
  honourSuppression(raw);
  const preparing = preparation.prepare().catch(() => undefined); // This model fails.
  const microphone = await captureMicrophone(undefined, "dpdfnet8", new AbortController().signal, () => undefined, "speakers", undefined, preparation);
  const track = microphone.track;
  workers[0].onmessage!({ data: { type: "failed" } });
  await tick();
  await tick();
  assert.equal(microphone.track, track);
  assert.equal(microphone.track.readyState, "live");
  assert.match(microphone.status, /browser suppression active/);
  assert.equal(raw.getSettings().noiseSuppression, true);
  assert.equal(workers[0].terminateCalls, 1);
  assert.equal(workers.length, 2, "one fresh DPDFNet-8 retry, as for startup errors");
  microphone.stop();
  await preparing;
});
