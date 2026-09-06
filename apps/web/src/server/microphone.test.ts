import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { captureMicrophone } from "../media/microphone.ts";
import { NoiseAssets } from "../media/noise-assets.ts";

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

class Context {
  static latest: Context | undefined;
  readonly sampleRate = 48_000;
  state: AudioContextState = "running";
  readonly source = new SourceNode();
  readonly processed = new Track();
  readonly destination = { stream: new Stream([this.processed]) };
  closeCalls = 0;
  audioWorklet = { addModule: async (_url: string) => undefined };
  constructor(..._args: unknown[]) { Context.latest = this; }
  async resume() { this.state = "running"; }
  createMediaStreamSource(_stream: MediaStream) { return this.source; }
  createMediaStreamDestination() { return this.destination; }
  async close() { this.closeCalls++; this.state = "closed"; }
}

function setup(t: TestContext, options: {
  fetch?: typeof fetch;
  getUserMedia?: () => Promise<Stream>;
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

for (const variant of ["2", "8"] as const) test(`DPDFNet-${variant} readiness comes from its worker; runtime failure stops the published track`, async (t) => {
  const { install } = setup(t);
  let worker!: { onmessage?: (event: { data: unknown }) => void; terminated: boolean };
  install("Worker", class {
    onmessage?: (event: { data: unknown }) => void;
    terminated = false;
    constructor(url: string) { assert.equal(url, `/audio/dpdfnet${variant}-v1/worker.js`); worker = this; }
    terminate() { this.terminated = true; }
  });
  const capturing = captureMicrophone(undefined, variant === "2" ? "dpdfnet2" : "dpdfnet8", new AbortController().signal, () => undefined);
  await tick();
  worker.onmessage!({ data: { type: "ready" } });
  const microphone = await capturing;
  assert.equal(microphone.track, Context.latest!.processed);
  assert.ok(microphone.status.startsWith(`DPDFNet-${variant} HR active`));
  microphone.track.enabled = false;
  WorkletNode.latest!.port.emit("failed");
  await tick();
  assert.equal(microphone.track, Context.latest!.processed);
  assert.equal(microphone.track.enabled, false);
  assert.match(microphone.status, /failed — microphone stopped/);
  assert.equal(microphone.track.readyState, "ended");
  assert.equal(worker.terminated, true);
  microphone.stop();
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

test("a ready acknowledgement connects raw input to DeepFilter and returns only its destination track", async (t) => {
  const { raw } = setup(t);
  const capturing = captureMicrophone(undefined, "deepfilter", new AbortController().signal, () => undefined);
  await tick();
  WorkletNode.latest!.port.emit("ready");
  const microphone = await capturing;
  const context = Context.latest!;
  assert.notEqual(microphone.track, raw);
  assert.equal(microphone.track, context.processed);
  assert.deepEqual(context.source.connections, [WorkletNode.latest]);
  assert.deepEqual(WorkletNode.latest!.connections, [context.destination]);
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
  assert.equal(microphone.status, "DeepFilterNet failed — microphone stopped");
  assert.equal(changes, 1);
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
  microphone.stop();
  microphone.stop();
  assert.equal(raw.stopCalls, 1);
  assert.equal(processed.stopCalls, 1);
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
