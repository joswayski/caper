import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { fakerEN as faker } from "@faker-js/faker";
import { PublicCallClient, waitFor } from "../media/client.ts";
import { NoiseAssets } from "../media/noise-assets.ts";
import { DpdfnetPreparation } from "../media/dpdfnet-preparation.ts";
import type { CallViewState } from "../media/types.ts";

class Track extends EventTarget {
  kind = "audio";
  enabled = true;
  readyState = "live";
  onended?: () => void;
  stop() { this.readyState = "ended"; }
  getSettings() { return { noiseSuppression: true }; }
}
class Stream {
  tracks: Track[];
  constructor(tracks: Track[]) { this.tracks = tracks; }
  getTracks() { return this.tracks; }
  getAudioTracks() { return this.tracks.filter((t) => t.kind === "audio"); }
}
class Peer extends EventTarget {
  static latest: Peer;
  static all: Peer[] = [];
  connectionState = "connected";
  iceGatheringState = "complete";
  localDescription?: { toJSON(): object };
  ontrack?: (event: { track: Track; transceiver: { mid: string }; streams: Stream[] }) => void;
  onconnectionstatechange?: () => void;
  senders: Array<{ track: Track | null; replaceTrack(t: Track | null): Promise<void> }> = [];
  constructor() { super(); Peer.latest = this; Peer.all.push(this); }
  addTransceiver(track: Track) {
    const sender = { track: track as Track | null, async replaceTrack(t: Track | null) { this.track = t; } };
    this.senders.push(sender);
    return { mid: "0", sender };
  }
  async createOffer() { return { type: "offer", sdp: "v=0" }; }
  async createAnswer() { return { type: "answer", sdp: "v=0" }; }
  async setLocalDescription(description: object) { this.localDescription = { toJSON: () => description }; }
  async setRemoteDescription(description?: { type: string }) {
    if (description?.type === "offer") this.ontrack?.({ track: new Track(), transceiver: { mid: "1" }, streams: [] });
  }
  getSenders() { return this.senders; }
  getReceivers() { return []; }
  close() { this.connectionState = "closed"; }
}

function setup(t: TestContext, config: { eventsReady?: boolean } = {}) {
  Peer.all = [];
  const track = new Track();
  const calls: string[] = [];
  const joinedNames: string[] = [];
  const stateUpdates: Array<{ muted: boolean; deafened: boolean }> = [];
  const states: CallViewState[] = [];
  const events: ReadableStreamDefaultController<Uint8Array>[] = [];
  const restore: Array<() => void> = [];
  const install = (key: string, value: unknown) => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { value, configurable: true });
    restore.push(() => { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); });
  };
  install("window", globalThis);
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([track.readyState === "ended" ? new Track() : track]) } });
  install("MediaStream", Stream);
  install("RTCPeerConnection", Peer);
  install("fetch", async (url: string, options: RequestInit) => {
    const op = url.split("/").at(-1)!;
    calls.push(op);
    if (op === "join") {
      joinedNames.push(JSON.parse(options.body as string).name);
      const role = JSON.parse(options.body as string).monitor;
      return Response.json({ token: role ?? "capability", id: role ?? "self", iceServers: [] });
    }
    if (op === "events") {
      let controller: ReadableStreamDefaultController<Uint8Array>;
      const abort = () => controller.error(options.signal!.reason);
      return new Response(new ReadableStream<Uint8Array>({
        start(value) {
          controller = value;
          events.push(controller);
          if (config.eventsReady !== false) controller.enqueue(new TextEncoder().encode("event: ready\ndata: {}\n\n"));
          options.signal!.addEventListener("abort", abort, { once: true });
        },
        cancel() { options.signal!.removeEventListener("abort", abort); },
      }), { headers: { "content-type": "text/event-stream" } });
    }
    if (op === "publish") return Response.json({ trackId: "private-track", sessionDescription: { type: "answer", sdp: "v=0" } });
    if (op === "subscribe") return Response.json({ requiresImmediateRenegotiation: true, tracks: [{ mid: "1" }], sessionDescription: { type: "offer", sdp: "v=0" } });
    if (op === "snapshot") return Response.json({ participants: [] });
    if (op === "state") stateUpdates.push(JSON.parse(options.body as string));
    return new Response(null, { status: 204 });
  });
  const client = new PublicCallClient((state) => states.push(state));
  // These tests isolate signaling with raw mock tracks; enhanced audio is tested separately.
  void client.setNoiseSuppression("off");
  t.after(async () => { client.leaveImmediately(); await tick(); restore.reverse().forEach((fn) => fn()); });
  return { client, track, calls, joinedNames, stateUpdates, states, install, events };
}

test("DPDFNet suppression is the default; browser suppression remains opt-in", async (t) => {
  const dpdfnet = t.mock.method(DpdfnetPreparation.prototype, "prepare", async () => undefined);
  const engines: string[] = [];
  t.mock.method(NoiseAssets.prototype, "load", async (engine: string) => {
    engines.push(engine);
    return { module: {} as WebAssembly.Module };
  });
  const states: CallViewState[] = [];
  const client = new PublicCallClient((state) => states.push(state));
  await client.setAudioSetup("headphones");
  assert.equal(states.at(-1)?.noiseSuppression, "dpdfnet8");
  client.prepareMicrophone();
  assert.equal(dpdfnet.mock.callCount(), 1);
  assert.deepEqual(engines, []);
  await client.setNoiseSuppression("browser");
  assert.equal(states.at(-1)?.noiseSuppression, "browser");
  client.prepareMicrophone();
  assert.equal(dpdfnet.mock.callCount(), 1);
  assert.deepEqual(engines, []);
  await client.setNoiseSuppression("deepfilter-gentle");
  client.prepareMicrophone();
  await client.setNoiseSuppression("rnnoise");
  client.prepareMicrophone();
  assert.deepEqual(engines, ["deepfilter", "rnnoise"]);
});

test("preparation does not allocate a spare during a call; Leave warms the next join, page exit does not", async (t) => {
  const { client, track } = setup(t);
  const prepare = t.mock.method(DpdfnetPreparation.prototype, "prepare", async () => undefined);
  const stop = t.mock.method(DpdfnetPreparation.prototype, "stop", () => undefined);
  // Isolate client lifecycle here; real processed capture and worker handoff have separate tests.
  t.mock.method(client as unknown as { openMicrophone(): Promise<MediaStreamTrack> }, "openMicrophone", async () => track as unknown as MediaStreamTrack);
  await client.setNoiseSuppression("dpdfnet8");
  client.prepareMicrophone();
  await client.join();
  client.prepareMicrophone();
  assert.equal(prepare.mock.callCount(), 1);
  await client.leave();
  assert.equal(prepare.mock.callCount(), 2);
  assert.equal(stop.mock.callCount(), 1);
  client.leaveImmediately();
  assert.equal(prepare.mock.callCount(), 2);
  assert.equal(stop.mock.callCount(), 2);
});

test("mode/device replacement preserves mute, releases old capture, and can select system default", async (t) => {
  const { client, states, install } = setup(t);
  const tracks: Track[] = [];
  const constraints: MediaTrackConstraints[] = [];
  install("navigator", { mediaDevices: { getUserMedia: async (value: MediaStreamConstraints) => {
    constraints.push(value.audio as MediaTrackConstraints);
    const track = new Track(); tracks.push(track);
    return new Stream([track]);
  } } });
  await client.join("Guest", "usb");
  assert.equal(states.at(-1)?.noiseSuppression, "off");
  assert.equal(states.at(-1)?.audioSetup, "headphones");
  assert.equal(constraints[0].echoCancellation, false);
  assert.equal(constraints[0].autoGainControl, false);
  assert.equal(constraints[0].noiseSuppression, false);
  await client.setMuted(true);
  await client.setNoiseSuppression("off");
  assert.equal(Peer.latest.senders[0].track, null);
  assert.equal(tracks[0].readyState, "ended");
  assert.equal(tracks[1].enabled, false);
  assert.equal(states.at(-1)?.localMedia?.getAudioTracks()[0], tracks[1]);
  assert.equal(states.at(-1)?.noiseSuppressionStatus, "Noise suppression off");
  await client.changeMicrophone("");
  assert.equal(constraints[2].deviceId, undefined);
  assert.equal(tracks[1].readyState, "ended");
  await client.setMuted(false);
  assert.equal(Peer.latest.senders[0].track, tracks[2]);
  await client.leave();
  assert.ok(tracks.every((track) => track.readyState === "ended"));
});

test("failed mode replacement keeps the old microphone and rolls back selection", async (t) => {
  const { client, track, install, states } = setup(t);
  await client.join();
  const replacement = new Track();
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([replacement]) } });
  Peer.latest.senders[0].replaceTrack = async () => { throw new Error("replace failed"); };
  await assert.rejects(client.setNoiseSuppression("browser"), /replace failed/);
  assert.equal(track.readyState, "live");
  assert.equal(replacement.readyState, "ended");
  assert.equal(states.at(-1)?.noiseSuppression, "off");
});

test("random nicknames are submitted once per explicit join", async (t) => {
  const { client, joinedNames } = setup(t);
  let generated = 0;
  t.mock.method(faker.word, "adjective", () => ++generated === 1 ? "mellow" : "brave");
  t.mock.method(faker.animal, "type", () => "otter");
  await client.join();
  await client.setMuted(true);
  await client.join(); // Already connected: do not regenerate.
  assert.deepEqual(joinedNames, ["mellow otter"]);
  assert.equal(generated, 1);
  await client.leave();
  await client.join();
  assert.deepEqual(joinedNames, ["mellow otter", "brave otter"]);
});

test("join, 204 state responses, real sender mute, deafen and immediate device cleanup", async (t) => {
  const { client, track, calls, states } = setup(t);
  await client.join("Guest");
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.localMedia?.getAudioTracks()[0], track);
  await client.setMuted(true);
  assert.equal(track.enabled, false);
  assert.equal(Peer.latest.senders[0].track, null);
  await client.setMuted(false);
  assert.equal(Peer.latest.senders[0].track, track);
  await client.setDeafened(true);
  client.leaveImmediately();
  assert.equal(track.readyState, "ended");
  assert.equal(Peer.latest.connectionState, "closed");
  assert.ok(calls.includes("leave"));
});

test("mic test detaches channel audio, plays a separately received track, and restores prior state", async (t) => {
  const { client, track, states, stateUpdates } = setup(t);
  await client.join("Guest");
  const channelPeer = Peer.latest;
  await client.setMuted(true);
  await client.setDeafened(false);

  await client.setMonitoring(true);
  assert.equal(states.at(-1)?.monitoring, true);
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(states.at(-1)?.deafened, true);
  const monitorTrack = states.at(-1)?.monitorStream?.getAudioTracks()[0] as unknown as Track;
  assert.notEqual(monitorTrack, track, "only the separately received/decoded track should feed playback");
  assert.equal(monitorTrack.enabled, true, "the local loopback track must remain audible");
  assert.equal(track.enabled, true, "sender detachment, not track disabling, isolates the channel");
  assert.equal(channelPeer.senders[0].track, null, "the microphone must not reach the channel");
  assert.deepEqual(stateUpdates.at(-1), { muted: true, deafened: true });

  await client.setMonitoring(false);
  assert.equal(states.at(-1)?.monitoring, false);
  assert.equal(states.at(-1)?.monitorStream, undefined);
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(states.at(-1)?.deafened, false);
  assert.equal(track.enabled, false);
  assert.equal(monitorTrack.readyState, "ended", "stopping releases the received track");
  assert.equal(track.readyState, "live", "stopping must not stop microphone capture");
  assert.equal(channelPeer.senders[0].track, null);
  assert.deepEqual(stateUpdates.at(-1), { muted: true, deafened: false });
});

test("monitor capture replacement stays private and retains the same received stream", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join();
  await client.setMonitoring(true);
  const received = states.at(-1)!.monitorStream;
  await client.setMuted(false);
  await client.setDeafened(false);
  assert.equal(states.at(-1)!.muted, true);
  assert.equal(states.at(-1)!.deafened, true);
  const replacement = new Track();
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([replacement]) } });
  await client.setNoiseSuppression("off");
  assert.equal(Peer.all[0].senders[0].track, null);
  assert.equal(Peer.all[1].senders[0].track, replacement);
  assert.equal(track.readyState, "ended");
  assert.equal(states.at(-1)!.monitorStream, received);
  await client.setMonitoring(false);
  assert.equal(Peer.all[0].senders[0].track, replacement);
  assert.ok(Peer.all.slice(1).every((peer) => peer.connectionState === "closed"));
});

test("failed private join keeps channel isolated until explicit stop", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join();
  const originalFetch = fetch;
  install("fetch", (url: string, options: RequestInit) => url.endsWith("/join")
    ? Promise.resolve(Response.json({ error: "test unavailable" }, { status: 503 })) : originalFetch(url, options));
  await assert.rejects(client.setMonitoring(true), /test unavailable/);
  assert.equal(Peer.all[0].senders[0].track, null);
  assert.equal(states.at(-1)!.monitorStream, undefined);
  assert.equal(states.at(-1)!.monitoring, true);
  assert.equal(states.at(-1)!.monitorConnecting, false);
  await client.setMonitoring(false);
  assert.equal(Peer.all[0].senders[0].track, track);
});

test("transient heartbeat failures preserve audio, recover the same session, and reset the grace window", async (t) => {
  const { client, track, states, calls, install } = setup(t);
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  const peer = Peer.latest;
  const polling = client as unknown as { poll(): Promise<void> };
  const originalFetch = fetch;
  let now = 0;
  let status = 503;
  t.mock.method(performance, "now", () => now);
  install("fetch", (url: string, options: RequestInit) => url.endsWith("/snapshot") && status !== 200
    ? Promise.resolve(Response.json({}, { status })) : originalFetch(url, options));
  for (status of [503, 502, 429, 408]) {
    now += 3_000;
    await polling.poll();
    assert.equal(states.at(-1)?.phase, "connected");
    assert.equal(track.readyState, "live");
    assert.equal(peer.connectionState, "connected");
  }
  status = 200;
  await polling.poll();
  now += 30_000;
  status = 503;
  await polling.poll();
  assert.equal(states.at(-1)?.phase, "connected", "a later outage gets its own grace window");
  assert.equal(Peer.all.length, 1);
  assert.equal(calls.filter((op) => op === "join").length, 1);
  assert.ok(!calls.includes("leave"));
});

test("prolonged control outage still triggers rejoin", async (t) => {
  const { client, states, install } = setup(t);
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  const polling = client as unknown as { poll(): Promise<void> };
  let now = 0;
  t.mock.method(performance, "now", () => now);
  install("fetch", async () => { throw new TypeError("Network unavailable"); });
  await polling.poll();
  assert.equal(states.at(-1)?.phase, "connected");
  now = 30_000;
  await polling.poll();
  assert.equal(states.at(-1)?.phase, "reconnecting");
});

test("invalid session is not treated as a transient outage", async (t) => {
  const { client, states, install } = setup(t);
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  install("fetch", async () => Response.json({ error: "Session expired" }, { status: 401 }));
  await (client as unknown as { poll(): Promise<void> }).poll();
  assert.equal(states.at(-1)?.phase, "reconnecting");
});

test("heartbeat timeout covers response bodies without closing healthy media", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  t.mock.timers.enable({ apis: ["setTimeout"] });
  install("fetch", async (_url: string, options: RequestInit) => ({
    ok: true, status: 200, headers: new Headers(),
    text: () => new Promise((_resolve, reject) => options.signal!.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")))),
  }));
  const pending = (client as unknown as { poll(): Promise<void> }).poll();
  await Promise.resolve();
  t.mock.timers.tick(5_000);
  await pending;
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(track.readyState, "live");
});

test("failed mute state synchronization retries the latest state after API recovery", async (t) => {
  const { client, track, stateUpdates, install } = setup(t);
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  const originalFetch = fetch;
  let unavailable = true;
  install("fetch", (url: string, options: RequestInit) => unavailable && url.endsWith("/state")
    ? Promise.resolve(Response.json({}, { status: 503 })) : originalFetch(url, options));
  await assert.rejects(client.setMuted(true));
  assert.equal(track.enabled, false);
  assert.equal(Peer.latest.senders[0].track, null);
  await assert.rejects(client.setDeafened(true));
  unavailable = false;
  await (client as unknown as { poll(): Promise<void> }).poll();
  assert.deepEqual(stateUpdates.at(-1), { muted: true, deafened: true });
  assert.equal(Peer.all.length, 1);
});

test("temporary RTC disconnect recovers without rejoin, but persistent disconnect does not", async (t) => {
  const { client, states } = setup(t);
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const peer = Peer.latest;
  peer.connectionState = "disconnected";
  peer.onconnectionstatechange!();
  t.mock.timers.tick(9_000);
  assert.equal(states.at(-1)?.phase, "connected");
  peer.connectionState = "connected";
  peer.onconnectionstatechange!();
  t.mock.timers.tick(1_000);
  assert.equal(states.at(-1)?.phase, "connected");
  peer.connectionState = "disconnected";
  peer.onconnectionstatechange!();
  t.mock.timers.tick(10_000);
  assert.equal(states.at(-1)?.phase, "reconnecting");
});

for (const intermediate of ["connecting", "disconnected"]) {
  test(`RTC recovery keeps the original deadline through ${intermediate} events`, async (t) => {
    const { client, states } = setup(t);
    await client.join();
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const peer = Peer.latest;
    peer.connectionState = "disconnected";
    peer.onconnectionstatechange!();
    t.mock.timers.tick(9_000);
    peer.connectionState = intermediate;
    peer.onconnectionstatechange!();
    await (client as unknown as { poll(): Promise<void> }).poll();
    assert.equal(states.at(-1)?.phase, "connected", "a healthy heartbeat does not prove transport recovery");
    t.mock.timers.tick(1_000);
    assert.equal(states.at(-1)?.phase, "reconnecting", "recover at the original ten-second deadline");
  });
}

test("RTC failure immediately schedules recovery and leave cancels delayed recovery", async (t) => {
  const { client, states, calls } = setup(t);
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const peer = Peer.latest;
  peer.connectionState = "failed";
  peer.onconnectionstatechange!();
  assert.equal(states.at(-1)?.phase, "reconnecting");
  await client.leave();
  t.mock.timers.tick(20_000);
  assert.equal(states.at(-1)?.phase, "idle");
  assert.equal(calls.filter((op) => op === "join").length, 1);
});

test("terminal reconnect failure stops mic monitoring before an explicit join", async (t) => {
  const { client, states, install } = setup(t);
  await client.join();
  await client.setMonitoring(true);

  const reconnectable = client as unknown as { reconnects: number; rejoin(): Promise<void> };
  reconnectable.reconnects = 3;
  await reconnectable.rejoin();
  assert.equal(states.at(-1)?.phase, "failed");
  assert.equal(states.at(-1)?.monitoring, false);
  assert.equal(states.at(-1)?.muted, false);
  assert.equal(states.at(-1)?.deafened, false);

  const retryTrack = new Track();
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([retryTrack]) } });
  await client.join();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(Peer.latest.senders[0].track, retryTrack);
});

test("stop during pending test joins releases late capabilities without publishing", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join();
  const originalFetch = fetch;
  const grants: Array<() => void> = [];
  const left: string[] = [];
  let published = 0;
  install("fetch", (url: string, options: RequestInit) => {
    if (url.endsWith("/join")) return new Promise<Response>((resolve) => {
      const role = JSON.parse(options.body as string).monitor;
      grants.push(() => resolve(Response.json({ token: role, iceServers: [] })));
    });
    if (url.endsWith("/publish")) published++;
    if (url.endsWith("/leave")) left.push((options.headers as Record<string, string>).authorization);
    return originalFetch(url, options);
  });
  const starting = client.setMonitoring(true);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(grants.length, 2);
  await client.setMonitoring(false);
  grants.forEach((grant) => grant());
  await starting;
  assert.equal(published, 0);
  assert.deepEqual(left.sort(), ["Bearer receiver", "Bearer sender"]);
  assert.equal(states.at(-1)!.monitorStream, undefined);
  assert.equal(Peer.all[0].senders[0].track, track);
});

test("mute and deafen update local media and view state without waiting for roster sync", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join("Guest");
  let finishState!: () => void;
  install("fetch", async (url: string) => {
    if (url.endsWith("/state")) return new Promise<Response>((resolve) => { finishState = () => resolve(new Response(null, { status: 204 })); });
    return new Response(null, { status: 204 });
  });

  const muting = client.setMuted(true);
  assert.equal(track.enabled, false);
  assert.equal(states.at(-1)?.muted, true);
  await new Promise((resolve) => setImmediate(resolve));
  finishState();
  await muting;

  const deafening = client.setDeafened(true);
  assert.equal(states.at(-1)?.deafened, true);
  await new Promise((resolve) => setImmediate(resolve));
  finishState();
  await deafening;
});

test("mute media changes are not queued behind roster synchronization", async (t) => {
  const { client, install } = setup(t);
  await client.join("Guest");
  const finishStates: Array<() => void> = [];
  install("fetch", async (url: string) => {
    if (url.endsWith("/state")) return new Promise<Response>((resolve) => { finishStates.push(() => resolve(new Response(null, { status: 204 }))); });
    return new Response(null, { status: 204 });
  });

  const deafening = client.setDeafened(true);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(finishStates.length, 1);
  const muting = client.setMuted(true);
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(Peer.latest.senders[0].track, null);
  assert.equal(finishStates.length, 1);

  finishStates.shift()!();
  await deafening;
  await new Promise((resolve) => setImmediate(resolve));
  finishStates.shift()!();
  await muting;
});

test("unmute during a pending microphone switch attaches the new track", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join("Guest");
  await client.setMuted(true);

  const nextTrack = new Track();
  const replacements: Array<{ next: Track | null; finish: () => void }> = [];
  const flush = () => new Promise((resolve) => setImmediate(resolve));
  Peer.latest.senders[0].replaceTrack = (next) => new Promise<void>((resolve) => {
    replacements.push({
      next,
      finish: () => {
        Peer.latest.senders[0].track = next;
        resolve();
      },
    });
  });
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([nextTrack]) } });

  const switching = client.changeMicrophone("mic-2");
  for (let i = 0; i < 20 && replacements.length === 0; i++) await flush();
  assert.equal(replacements.length, 1);
  assert.equal(replacements[0]?.next, null);

  const unmuting = client.setMuted(false);
  assert.equal(states.at(-1)?.muted, false);
  for (let i = 0; i < 10; i++) await flush();
  assert.equal(replacements.length, 1);

  replacements[0]?.finish();
  await switching;
  await flush();
  assert.equal(track.readyState, "ended");
  assert.equal(replacements.length, 2);
  assert.equal(replacements[1]?.next, nextTrack);

  replacements[1]?.finish();
  await unmuting;
  assert.equal(Peer.latest.senders[0].track, nextTrack);
  assert.equal(nextTrack.enabled, true);
  assert.equal(nextTrack.readyState, "live");
});

test("provider join failure releases microphone acquired before publication", async (t) => {
  const { client, track, states, install } = setup(t);
  install("fetch", async () => Response.json({ error: "unavailable" }, { status: 503 }));
  await client.join("Guest");
  assert.equal(track.readyState, "ended");
  assert.equal(states.at(-1)?.phase, "failed");
  assert.equal(states.at(-1)?.error, "unavailable");
});

test("join provisioning overlaps permission and leave cleans up both late results", async (t) => {
  const { client, track, calls, states, install } = setup(t);
  let grant!: (s: Stream) => void;
  install("navigator", { mediaDevices: { getUserMedia: () => new Promise((resolve) => { grant = resolve; }) } });
  const joining = client.join("Guest");
  await new Promise((resolve) => setImmediate(resolve));
  assert.deepEqual(calls, ["join"]);
  await client.leave();
  grant(new Stream([track]));
  await joining;
  assert.equal(track.readyState, "ended");
  assert.deepEqual(calls, ["join", "leave"]);
  assert.equal(states.at(-1)?.phase, "idle");
});

for (const cleanupFails of [false, true]) test(`leave releases local media and permits rejoin before old cleanup ${cleanupFails ? "fails" : "finishes"}`, async (t) => {
  const { client, track, states, install } = setup(t);
  let completeLeave!: () => void;
  let joins = 0;
  let leaveRequest!: RequestInit;
  let cleanupRequests = 0;
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/join")) return Promise.resolve(Response.json({ token: `session-${++joins}`, id: "self", iceServers: [] }));
    if (url.endsWith("/leave") && new Headers(init.headers).get("authorization") === "Bearer session-1") {
      leaveRequest = init;
      cleanupRequests++;
      return new Promise<Response>((resolve, reject) => {
        completeLeave = () => cleanupFails ? reject(new Error("cleanup unavailable")) : resolve(new Response(null, { status: 204 }));
      });
    }
    return original(url, init);
  });
  await client.join("Guest");
  const oldPeer = Peer.latest;
  const leaving = client.leave();
  assert.equal(track.readyState, "ended", "capture stops synchronously");
  assert.equal(oldPeer.connectionState, "closed");
  assert.equal(states.at(-1)?.localMedia, undefined);
  assert.deepEqual(states.at(-1)?.remoteMedia, []);
  assert.deepEqual(states.at(-1)?.participants, []);
  assert.equal(states.at(-1)?.selfId, undefined);
  await leaving; // The old HTTP request is deliberately unresolved.
  assert.equal(states.at(-1)?.phase, "idle");
  assert.equal(leaveRequest.keepalive, true, "cleanup can outlive navigation");
  await client.leave(); // Repeated Leave must not send another cleanup request.
  assert.equal(cleanupRequests, 1);
  await client.join("Second guest");
  const currentTrack = Peer.latest.senders[0].track!;
  assert.equal(joins, 2);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(leaveRequest.signal?.aborted, false, "new capture must not cancel old cleanup");
  completeLeave();
  await tick();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(currentTrack.readyState, "live");
  assert.equal(currentTrack.enabled, true);
  assert.equal(new Headers(leaveRequest.headers).get("authorization"), "Bearer session-1");
});

test("old peer events after leave cannot reconnect or replace the next call's audio", async (t) => {
  const { client, states, install } = setup(t);
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? Promise.resolve(Response.json({ participants: [{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] }] }))
    : original(url, init));
  await client.join();
  const oldPeer = Peer.latest;
  const oldTrack = states.at(-1)!.remoteMedia[0].stream.getAudioTracks()[0] as unknown as Track;
  await client.leave();
  await client.join();
  const currentMedia = states.at(-1)!.remoteMedia[0];
  oldPeer.connectionState = "disconnected";
  oldPeer.onconnectionstatechange!();
  oldTrack.onended!();
  const lateTrack = new Track();
  oldPeer.ontrack!({ track: lateTrack, transceiver: { mid: "1" }, streams: [] });
  assert.equal(lateTrack.readyState, "ended");
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.remoteMedia[0], currentMedia);
});

test("a microphone replacement finishing after leave cannot overwrite the new call", async (t) => {
  const { client, states, install } = setup(t);
  await client.join();
  const replacement = new Track();
  const nextCapture = new Track();
  let captures = 0;
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([captures++ === 0 ? replacement : nextCapture]) } });
  let finishReplacement!: () => void;
  Peer.latest.senders[0].replaceTrack = () => new Promise<void>((resolve) => { finishReplacement = resolve; });
  const replacing = assert.rejects(client.changeMicrophone("old-device"), /Call session changed/);
  await tick();
  await client.leave();
  await client.join();
  finishReplacement();
  await replacing;
  assert.equal(replacement.readyState, "ended");
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.localMedia?.getAudioTracks()[0], nextCapture);
  assert.equal(Peer.latest.senders[0].track, nextCapture);
  assert.equal(nextCapture.readyState, "live");
});

test("leave during join closes the late capability and never creates a PeerConnection", async (t) => {
  const { client, track, install } = setup(t);
  let finish!: (r: Response) => void;
  let leaveToken: string | undefined;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/join")) return new Promise((resolve) => { finish = resolve; });
    leaveToken = new Headers(init.headers).get("authorization") ?? undefined;
    return Promise.resolve(new Response(null, { status: 204 }));
  });
  const joining = client.join("Guest");
  await new Promise((resolve) => setImmediate(resolve));
  await client.leave();
  finish(Response.json({ token: "late", id: "late", iceServers: [] }));
  await joining;
  assert.equal(track.readyState, "ended");
  assert.equal(leaveToken, "Bearer late");
});

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const changedEvent = () => new TextEncoder().encode("event: changed\ndata: {}\n\n");

test("Join gates publication on SSE and audio on transport, initial roster, and state readiness", async (t) => {
  const { client, track, calls, states, events, install } = setup(t, { eventsReady: false });
  const original = fetch;
  let snapshot!: () => void;
  let state!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/snapshot")) return new Promise<Response>((resolve) => { snapshot = () => resolve(Response.json({ participants: [] })); });
    if (url.endsWith("/state")) return new Promise<Response>((resolve) => { state = () => resolve(new Response(null, { status: 204 })); });
    return original(url, init);
  });
  const joining = client.join();
  await tick();
  assert.equal(states.at(-1)?.phase, "joining");
  assert.equal(calls.includes("publish"), false, "HTTP headers alone are not an SSE handshake");
  Peer.latest.connectionState = "connecting";
  Peer.latest.iceGatheringState = "gathering"; // A stalled probe must not delay publication.
  events[0].enqueue(new TextEncoder().encode("event: ready\ndata: {}\n\n"));
  await tick();
  assert.equal(calls.includes("publish"), true);
  assert.equal(track.enabled, false, "publication establishes transport with silence");
  assert.equal(typeof snapshot, "undefined");
  Peer.latest.connectionState = "connected";
  Peer.latest.dispatchEvent(new Event("connectionstatechange"));
  await tick();
  assert.equal(track.enabled, false);
  snapshot();
  await tick();
  assert.equal(track.enabled, false, "state synchronization must finish before audio is enabled");
  assert.equal(states.at(-1)?.phase, "joining");
  events[0].enqueue(changedEvent());
  await tick();
  state();
  await tick();
  assert.equal(track.enabled, false, "an update during state sync must be reconciled before opening audio");
  snapshot();
  await joining;
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(track.enabled, true);
});

test("initial subscription negotiation completes before microphone audio is enabled", async (t) => {
  const { client, track, states, install } = setup(t);
  const original = fetch;
  let negotiate!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/snapshot")) return Promise.resolve(Response.json({ participants: [{id:"other",name:"Other",muted:false,deafened:false,tracks:[{id:"remote",kind:"microphone"}]}] }));
    if (url.endsWith("/negotiate")) return new Promise<Response>((resolve) => { negotiate = () => resolve(new Response(null,{status:204})); });
    return original(url, init);
  });
  const joining = client.join();
  await tick();
  assert.equal(typeof negotiate, "function");
  assert.equal(track.enabled, false);
  assert.equal(states.at(-1)?.phase, "joining");
  negotiate();
  await joining;
  assert.equal(track.enabled, true);
  assert.equal(states.at(-1)?.phase, "connected");
});

test("cancel before SSE readiness never publishes and releases capture", async (t) => {
  const { client, track, calls, states } = setup(t, { eventsReady: false });
  const joining = client.join();
  await tick();
  await client.leave();
  await joining;
  assert.equal(calls.includes("publish"), false);
  assert.equal(track.readyState, "ended");
  assert.equal(states.at(-1)?.phase, "idle");
});

test("an unavailable selected enhancer fails Join without publishing a fallback", async (t) => {
  const { client, track, states, calls } = setup(t);
  await client.setNoiseSuppression("deepfilter");
  await client.join();
  assert.equal(states.at(-1)?.phase, "failed");
  assert.match(states.at(-1)!.error!, /DeepFilterNet could not start/);
  assert.equal(calls.includes("publish"), false);
  assert.equal(track.readyState, "ended");
});

test("SSE loss during startup fails closed but an established call recovers its stream without stopping audio", async (t) => {
  const { client, track, events, states, install } = setup(t);
  const original = fetch;
  let publish!: () => void;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/publish")
    ? new Promise<Response>((resolve) => { publish = () => resolve(Response.json({sessionDescription:{type:"answer",sdp:"v=0"}})); })
    : original(url, init));
  const joining = client.join();
  await tick();
  events[0].close();
  await tick();
  assert.equal(track.readyState, "ended");
  publish();
  await joining;
  assert.equal(states.at(-1)?.phase, "failed");
  install("fetch", original);
  await client.join();
  assert.equal(states.at(-1)?.phase, "connected");
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const peer = Peer.latest;
  const outgoing = Peer.latest.senders[0].track!;
  events.at(-1)!.close();
  await tick();
  assert.equal(outgoing.readyState, "live");
  assert.equal(outgoing.enabled, true);
  assert.equal(states.at(-1)?.phase, "connected");
  t.mock.timers.tick(3_000);
  await tick();
  assert.equal(events.length, 3, "reopen SSE using the existing voice session");
  assert.equal(Peer.latest, peer);
  assert.equal(outgoing.readyState, "live");
  assert.equal(states.at(-1)?.phase, "connected");
});

test("persistent SSE outage is bounded even while snapshots succeed", async (t) => {
  const { client, events, states, install } = setup(t);
  await client.join();
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/events")
    ? Promise.resolve(Response.json({}, { status: 503 })) : original(url, init));
  t.mock.timers.enable({ apis: ["setTimeout"] });
  events[0].close();
  await tick();
  t.mock.timers.tick(3_000);
  await tick();
  await (client as unknown as { poll(): Promise<void> }).poll();
  assert.equal(states.at(-1)?.phase, "connected");
  t.mock.timers.tick(27_000);
  await tick();
  assert.equal(states.at(-1)?.phase, "reconnecting");
});

test("automatic rejoin reopens SSE and preserves mute until explicitly unmuted", async (t) => {
  const { client, track, events, states } = setup(t);
  await client.join();
  await client.setMuted(true);
  await (client as unknown as { rejoin(): Promise<void> }).rejoin();
  assert.equal(events.length, 2);
  assert.equal(track.readyState, "ended");
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(Peer.latest.senders[0].track, null);
  await client.setMuted(false);
  assert.equal(Peer.latest.getSenders()[0].track?.enabled, true);
});

test("rejoin restores private microphone testing without reattaching the public sender", async (t) => {
  const { client, states } = setup(t);
  await client.join();
  await client.setMonitoring(true);
  const previousPeers = Peer.all.length;
  await (client as unknown as { rejoin(): Promise<void> }).rejoin();
  const [publicPeer, privateSender] = Peer.all.slice(previousPeers);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.monitoring, true);
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(publicPeer.senders[0].track, null);
  assert.equal(privateSender.senders[0].track?.enabled, true);
  assert.ok(states.at(-1)?.monitorStream);
});

test("an SSE track notification subscribes without waiting for the heartbeat timer", async (t) => {
  const { client, events, calls, states, install } = setup(t);
  await client.join();
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? Promise.resolve(Response.json({ participants: [{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{id:"remote",kind:"microphone"}] }] }))
    : original(url, init));
  events[0].enqueue(changedEvent());
  await tick();
  assert.ok(calls.includes("subscribe"));
  assert.ok(calls.includes("negotiate"));
  assert.equal(states.at(-1)?.remoteMedia[0]?.trackId, "remote");
});

test("SSE invalidations reconcile immediately and retain changes arriving during a snapshot", async (t) => {
  const { client, events, install } = setup(t);
  await client.join();
  const original = fetch;
  const snapshots: Array<() => void> = [];
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? new Promise<Response>((resolve) => snapshots.push(() => resolve(Response.json({participants:[]}))))
    : original(url, init));
  events[0].enqueue(changedEvent());
  await tick();
  assert.equal(snapshots.length, 1);
  events[0].enqueue(changedEvent());
  events[0].enqueue(changedEvent());
  await tick();
  assert.equal(snapshots.length, 1, "coalesce while a fetch is pending");
  snapshots[0]();
  await tick();
  assert.equal(snapshots.length, 2, "do not lose invalidation during an in-flight snapshot");
  snapshots[1]();
  await tick();
});

test("connection event wait ignores intermediate states", async (t) => {
  setup(t);
  const target = new EventTarget();
  let ready = false;
  let resolved = false;
  const waiting = waitFor(target, "change", 1000, () => ready).then(() => { resolved = true; });
  target.dispatchEvent(new Event("change"));
  await Promise.resolve();
  assert.equal(resolved, false);
  ready = true;
  target.dispatchEvent(new Event("change"));
  await waiting;
  assert.equal(resolved, true);
});
