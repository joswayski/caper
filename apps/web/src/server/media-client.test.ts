import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { AppGateway, setAppGatewayForTests } from "../gateway/client.ts";
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
  static stats = new Map<string, Record<string, unknown>>();
  connectionState = "connected";
  iceGatheringState = "complete";
  localDescription?: { toJSON(): object };
  remoteDescriptions: RTCSessionDescriptionInit[] = [];
  ontrack?: (event: { track: Track; transceiver: { mid: string }; streams: Stream[] }) => void;
  onconnectionstatechange?: () => void;
  senders: Array<{ track: Track | null; replaceTrack(t: Track | null): Promise<void> }> = [];
  constructor() { super(); Peer.latest = this; Peer.all.push(this); }
  configuration: RTCConfiguration = {};
  addTransceiver(track: Track | string) {
    const sender = { track: typeof track === "string" ? null : track as Track | null, async replaceTrack(t: Track | null) { this.track = t; } };
    this.senders.push(sender);
    return { mid: "0", sender };
  }
  getConfiguration() { return this.configuration; }
  setConfiguration(configuration: RTCConfiguration) { this.configuration = configuration; }
  async createOffer() { return { type: "offer", sdp: "v=0\r\na=mid:0\r\n" }; }
  async createAnswer() { return { type: "answer", sdp: "v=0" }; }
  async setLocalDescription(description: object) { this.localDescription = { toJSON: () => description }; }
  async setRemoteDescription(description: RTCSessionDescriptionInit) {
    this.remoteDescriptions.push(description);
    if (description?.type === "offer") this.ontrack?.({ track: new Track(), transceiver: { mid: "1" }, streams: [] });
  }
  getSenders() { return this.senders; }
  getReceivers() { return []; }
  async getStats() { return Peer.stats; }
  close() { this.connectionState = "closed"; }
}

const providerSdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 109\r\na=rtpmap:109 opus/48000/2\r\na=fmtp:109 useinbandfec=1;usedtx=0\r\n";
const senderSdp = "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 109\r\na=rtpmap:109 opus/48000/2\r\na=fmtp:109 useinbandfec=1;usedtx=1\r\n";

interface MediaSocket {
  event(id: string, value: object): void;
  subscribed(id: string): void;
  migrate(): void;
  disconnect(): void;
}

class MediaFeed {
  private readySent = false;
  private readonly socket: MediaSocket;
  private readonly id: string;
  private readonly revision: () => number;
  constructor(socket: MediaSocket, id: string, revision: () => number) {
    this.socket = socket;
    this.id = id;
    this.revision = revision;
  }

  enqueue(chunk: Uint8Array) {
    const payload = new TextDecoder().decode(chunk);
    const name = /^event: ([^\n]+)/m.exec(payload)?.[1];
    const data = JSON.parse(/^data: (.+)$/m.exec(payload)?.[1] ?? "{}") as Record<string, unknown>;
    if (name === "draining") { this.socket.migrate(); return; }
    if (name === "ready") { this.ready(); return; }
    if (name === "snapshot") {
      this.socket.event(this.id, { type: "snapshot", ...data, revision: data.revision ?? this.revision() });
      return;
    }
    if (name === "changed") {
      this.socket.event(this.id, { type: "snapshot", participants: [], revision: this.revision() });
    }
  }

  ready() {
    if (this.readySent) return;
    this.readySent = true;
    this.socket.event(this.id, { type: "snapshot", participants: [], revision: this.revision() });
    this.socket.subscribed(this.id);
  }

  close() { this.socket.disconnect(); }
}

function setup(t: TestContext, config: { eventsReady?: boolean } = {}) {
  Peer.all = [];
  Peer.stats = new Map();
  const track = new Track();
  const calls: string[] = [];
  const joinedNames: string[] = [];
  const stateUpdates: Array<{ muted: boolean; deafened: boolean }> = [];
  const stateSequences: number[] = [];
  const states: CallViewState[] = [];
  const events: MediaFeed[] = [];
  const audioSinks: FakeAudio[] = [];
  const restore: Array<() => void> = [];
  class FakeAudio {
    muted = false;
    srcObject: MediaStream | null = null;
    playing = false;
    constructor() { audioSinks.push(this); }
    async play() { this.playing = true; }
    pause() { this.playing = false; }
  }
  const install = (key: string, value: unknown) => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { value, configurable: true });
    restore.push(() => { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); });
  };
  install("window", globalThis);
  install("location", { protocol: "https:", host: "caper.test" });
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([track.readyState === "ended" ? new Track() : track]) } });
  install("MediaStream", Stream);
  install("RTCPeerConnection", Peer);
  install("Audio", FakeAudio);
  install("fetch", async (url: string, options: RequestInit) => {
    const op = url.split("?")[0].split("/").at(-1)!;
    calls.push(op);
    if (op === "join") {
      joinedNames.push(JSON.parse(options.body as string).name);
      const role = JSON.parse(options.body as string).monitor;
      return Response.json({ token: role ?? "capability", id: role ?? "self", iceServers: [] });
    }
    if (op === "publish") return Response.json({ trackId: "private-track", sessionDescription: { type: "answer", sdp: providerSdp } });
    if (op === "subscribe") return Response.json({ requiresImmediateRenegotiation: true, tracks: [{ mid: "1" }], sessionDescription: { type: "offer", sdp: providerSdp } });
    if (op === "snapshot") return Response.json({ participants: [] });
    if (op === "state") {
      const { muted, deafened, sequence } = JSON.parse(options.body as string);
      stateUpdates.push({ muted, deafened });
      stateSequences.push(sequence);
    }
    return new Response(null, { status: 204 });
  });
  let revision = -1;
  class Socket extends EventTarget implements MediaSocket {
    closed = false;
    constructor() {
      super();
      queueMicrotask(() => this.raw({ type: "hello", idleTimeoutSeconds: 600, serverTime: Date.now() }));
    }
    send(data: string) {
      const frame = JSON.parse(data) as Record<string, unknown>;
      if (frame.type !== "subscribe" || frame.kind !== "media") return;
      const feed = new MediaFeed(this, frame.id as string, () => ++revision);
      events.push(feed);
      if (config.eventsReady !== false) feed.ready();
    }
    close() { this.closed = true; }
    event(id: string, event: object) { this.raw({ type: "event", id, event }); }
    subscribed(id: string) { this.raw({ type: "subscribed", id }); }
    migrate() { this.raw({ type: "migrating" }); }
    disconnect() { this.dispatchEvent(new Event("close")); }
    private raw(value: unknown) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(value) })); }
  }
  const gateway = new AppGateway(() => new Socket(), () => 0);
  setAppGatewayForTests(gateway);
  const transport: typeof fetch = (input, init) => fetch(input, init);
  const client = new PublicCallClient((state) => states.push(state), "/api/media", transport);
  // These tests isolate signaling with raw mock tracks; enhanced audio is tested separately.
  void client.setNoiseSuppression("off");
  t.after(async () => {
    client.leaveImmediately();
    await tick();
    gateway.destroy();
    setAppGatewayForTests(undefined);
    restore.reverse().forEach((fn) => fn());
  });
  return { client, track, calls, joinedNames, stateUpdates, stateSequences, states, install, events, audioSinks };
}

test("state requests carry increasing sequences across mute/deafen and retries", async (t) => {
  const { client, stateSequences, install } = setup(t);
  await client.join();
  await client.setMuted(true);
  await client.setDeafened(true);
  await client.setDeafened(false);
  assert.deepEqual(stateSequences, [1, 2, 3, 4]);
  const original = fetch;
  let rejected = false;
  install("fetch", async (url: string, init: RequestInit) => {
    if (url.endsWith("/state") && !rejected) {
      rejected = true;
      await original(url, init);
      throw new TypeError("response lost after commit");
    }
    return original(url, init);
  });
  await client.setMuted(false);
  await client.setMuted(true);
  assert.deepEqual(stateSequences, [1, 2, 3, 4, 5, 6]);
});

test("DPDFNet suppression is the fixed default and prepares before capture", async (t) => {
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

test("connection diagnostics are structured and include live transport rates", async (t) => {
  const { client, states } = setup(t);
  Peer.stats = new Map([
    ["outbound", { type: "outbound-rtp", bytesSent: 1_500 }],
    ["inbound", { type: "inbound-rtp", bytesReceived: 2_500, packetsLost: 3, jitter: 0.006 }],
    ["local", { type: "local-candidate", candidateType: "relay" }],
    ["pair", { type: "candidate-pair", state: "succeeded", nominated: true, currentRoundTripTime: 0.004, localCandidateId: "local" }],
  ]);
  await client.join();
  await tick();
  const diagnostics = states.at(-1)?.diagnostics;
  assert.equal(diagnostics?.receivedBytes, 2_500);
  assert.equal(diagnostics?.sentBytes, 1_500);
  assert.equal(diagnostics?.packetsLost, 3);
  assert.equal(diagnostics?.maxJitterMs, 6);
  assert.equal(diagnostics?.roundTripMs, 4);
  assert.equal(diagnostics?.route, "relay");
  assert.equal(diagnostics?.receiveBitrate, 0, "the first sample establishes the bitrate baseline");
});

test("mode/device replacement preserves mute, releases old capture, and can select system default", async (t) => {
  const { client, states, install, calls } = setup(t);
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
  assert.equal(Peer.all.length, 1, "device replacement keeps the existing peer connection");
  assert.equal(calls.filter((operation) => operation === "join").length, 1);
  await client.leave();
  assert.ok(tracks.every((track) => track.readyState === "ended"));
});

test("failed mode replacement keeps the old microphone and rolls back selection", async (t) => {
  const { client, track, install, states } = setup(t);
  await client.join();
  const capture = (client as unknown as { captures: Map<Track, { pause(): Promise<void>; resume(): Promise<void> }> }).captures.get(track)!;
  const pause = t.mock.method(capture, "pause");
  const resume = t.mock.method(capture, "resume");
  const replacement = new Track();
  install("navigator", { mediaDevices: { getUserMedia: async () => {
    assert.equal(pause.mock.callCount(), 1, "old processing pauses before replacement capture starts");
    return new Stream([replacement]);
  } } });
  Peer.latest.senders[0].replaceTrack = async () => { throw new Error("replace failed"); };
  await assert.rejects(client.setNoiseSuppression("browser"), /replace failed/);
  assert.equal(pause.mock.callCount(), 1);
  assert.equal(resume.mock.callCount(), 1);
  assert.equal(track.readyState, "live");
  assert.equal(replacement.readyState, "ended");
  assert.equal(states.at(-1)?.noiseSuppression, "off");
});

test("guest name is trimmed, preserved on reconnect, and replaceable after leaving", async (t) => {
  const { client, joinedNames } = setup(t);
  await client.join("  Jose 🌱  ");
  await client.setMuted(true);
  await client.join(); // Already connected: do not join twice.
  assert.deepEqual(joinedNames, ["Jose 🌱"]);
  await (client as unknown as { rejoin(): Promise<void> }).rejoin();
  assert.deepEqual(joinedNames, ["Jose 🌱", "Jose 🌱"]);
  await client.leave();
  await client.join("Another guest");
  assert.deepEqual(joinedNames, ["Jose 🌱", "Jose 🌱", "Another guest"]);
});

test("join, 204 state responses, real sender mute, deafen, undeafen, and immediate device cleanup", async (t) => {
  const { client, track, calls, states } = setup(t);
  await client.join("Guest");
  assert.deepEqual(Peer.latest.remoteDescriptions, [{ type: "answer", sdp: senderSdp }]);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.localMedia?.getAudioTracks()[0], track);
  await client.setMuted(true);
  assert.equal(track.enabled, false);
  assert.equal(Peer.latest.senders[0].track, null);
  await client.setMuted(false);
  assert.equal(Peer.latest.senders[0].track, track);
  await client.setDeafened(true);
  assert.equal(track.enabled, false);
  assert.equal(Peer.latest.senders[0].track, null);
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(states.at(-1)?.deafened, true);
  await client.setDeafened(false);
  assert.equal(track.enabled, true, "undeafening restores the microphone");
  assert.equal(Peer.latest.senders[0].track, track);
  assert.equal(states.at(-1)?.muted, false);
  assert.equal(states.at(-1)?.deafened, false);
  client.leaveImmediately();
  assert.equal(track.readyState, "ended");
  assert.equal(Peer.latest.connectionState, "closed");
  assert.ok(calls.includes("leave"));
});

test("undeafen restores the microphone mute state from before deafening", async (t) => {
  const { client, track, states, stateUpdates } = setup(t);
  await client.join("Guest");
  await client.setMuted(true);
  await client.setDeafened(false);
  assert.equal(track.enabled, false, "a no-op is not an undeafen gesture");
  await client.setDeafened(true);
  await client.setDeafened(false);
  assert.equal(track.enabled, false, "undeafening must preserve the mute that was active before deafening");
  assert.equal(Peer.latest.senders[0].track, null);
  assert.equal(states.at(-1)?.muted, true);
  assert.deepEqual(stateUpdates.at(-1), { muted: true, deafened: false });
  await client.setMuted(false);
  assert.equal(track.enabled, true, "explicitly unmuting still restores the microphone");
  assert.equal(states.at(-1)?.deafened, false, "explicit unmute also undeafens");
  client.leaveImmediately();
});

test("mic test stays local, detaches channel audio, and restores prior state", async (t) => {
  const { client, track, calls, states, stateUpdates } = setup(t);
  await client.join("Guest");
  const naturalTrack = new Track();
  const capture = (client as unknown as { captures: Map<Track, { naturalTrack: Track }> }).captures.get(track)!;
  capture.naturalTrack = naturalTrack;
  const channelPeer = Peer.latest;
  await client.setMuted(true);
  await client.setDeafened(false);

  await client.setMonitoring(true);
  assert.equal(states.at(-1)?.monitoring, true);
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(states.at(-1)?.deafened, true);
  const monitorTrack = states.at(-1)?.monitorStream?.getAudioTracks()[0] as unknown as Track;
  assert.equal(monitorTrack, naturalTrack, "the test records the local denoised tap");
  assert.equal(Peer.all.length, 1, "the test must not create a private SFU connection");
  assert.equal(calls.filter((operation) => operation === "join").length, 1, "the test must not join a fake participant");
  assert.equal(track.enabled, true, "sender detachment, not track disabling, isolates the channel");
  assert.equal(channelPeer.senders[0].track, null, "the microphone must not reach the channel");
  assert.deepEqual(stateUpdates.at(-1), { muted: true, deafened: true });

  await client.setMonitoring(false);
  assert.equal(states.at(-1)?.monitoring, false);
  assert.equal(states.at(-1)?.monitorStream, undefined);
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(states.at(-1)?.deafened, false);
  assert.equal(track.enabled, false);
  assert.equal(track.readyState, "live", "stopping must not stop microphone capture");
  assert.equal(naturalTrack.readyState, "live", "the local test borrows the denoised tap without owning it");
  assert.equal(channelPeer.senders[0].track, null);
  assert.deepEqual(stateUpdates.at(-1), { muted: true, deafened: false });
});

test("pre-join mic test uses the local denoised track and releases it before joining", async (t) => {
  const { client, track, calls, states } = setup(t);
  await client.startLocalMicTest();
  assert.equal(states.at(-1)?.phase, "idle");
  assert.equal(states.at(-1)?.monitorStream?.getAudioTracks()[0], track);
  assert.deepEqual(calls, [], "starting a local test must not contact the media API");
  client.stopLocalMicTest();
  assert.equal(track.readyState, "ended");
  assert.equal(states.at(-1)?.monitorStream, undefined);
});

test("a failed join can open a local mic test", async (t) => {
  const { client, states, install } = setup(t);
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/publish")
    ? Promise.resolve(Response.json({ error: "publication unavailable" }, { status: 503 }))
    : original(url, init));
  await client.join();
  assert.equal(states.at(-1)?.phase, "failed");

  const testTrack = new Track();
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([testTrack]) } });
  await client.startLocalMicTest();
  assert.equal(states.at(-1)?.phase, "failed");
  assert.equal(states.at(-1)?.monitorStream?.getAudioTracks()[0], testTrack);
  client.stopLocalMicTest();
  assert.equal(testTrack.readyState, "ended");
});

test("stopping a pending local mic test releases capture that arrives later", async (t) => {
  const { client, states, install } = setup(t);
  const lateTrack = new Track();
  let finish!: () => void;
  install("navigator", { mediaDevices: { getUserMedia: () => new Promise<Stream>((resolve) => {
    finish = () => resolve(new Stream([lateTrack]));
  }) } });

  const starting = client.startLocalMicTest();
  await tick();
  client.stopLocalMicTest();
  await starting;
  const replacement = new Track();
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([replacement]) } });
  await client.startLocalMicTest();
  finish();
  await tick();

  assert.equal(lateTrack.readyState, "ended");
  assert.equal(states.at(-1)?.monitorStream?.getAudioTracks()[0], replacement);
  assert.equal(replacement.readyState, "live");
  client.stopLocalMicTest();
});

test("local mic test times out unanswered permission and releases a late grant", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const { client, states, install } = setup(t);
  let finish!: () => void;
  const lateTrack = new Track();
  install("navigator", { mediaDevices: { getUserMedia: () => new Promise<Stream>((resolve) => {
    finish = () => resolve(new Stream([lateTrack]));
  }) } });
  let settled = false;
  const starting = client.startLocalMicTest();
  const rejected = assert.rejects(starting, /Microphone setup timed out/).then(() => { settled = true; });
  t.mock.timers.tick(29_999);
  await tick();
  assert.equal(settled, false);
  t.mock.timers.tick(1);
  await rejected;
  finish();
  await tick();
  assert.equal(lateTrack.readyState, "ended");
  assert.equal(states.at(-1)?.monitorStream, undefined);
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([new Track()]) } });
  await client.startLocalMicTest();
  assert.ok(states.at(-1)?.monitorStream);
  client.stopLocalMicTest();
});

test("local mic test explains missing, denied, and busy devices", async (t) => {
  const { client, install } = setup(t);
  assert.equal(client.getAudioDiagnostics().captureAttempt, "not-started");
  for (const [name, expected] of [["NotFoundError", /Connect a microphone/], ["NotAllowedError", /permission was denied/], ["NotReadableError", /another app/]] as const) {
    install("navigator", { mediaDevices: { getUserMedia: async () => { throw new DOMException("Browser error", name); } } });
    await assert.rejects(client.startLocalMicTest(), expected);
    assert.equal(client.getAudioDiagnostics().captureAttempt, "failed");
    assert.equal(client.getAudioDiagnostics().captureError, name);
    assert.deepEqual(client.getAudioDiagnostics().captures, []);
  }
  client.stopLocalMicTest();
});

test("pre-join undeafen restores the prior mute intent without capturing or signaling", async (t) => {
  const { client, states, calls } = setup(t);
  await client.setMuted(true);
  await client.setDeafened(true);
  await client.setDeafened(false);
  assert.equal(states.at(-1)?.muted, true, "Undeafen preserves an explicitly muted mic");
  assert.equal(states.at(-1)?.deafened, false);
  await client.setMuted(false);
  await client.setDeafened(true);
  await client.setDeafened(false);
  assert.equal(states.at(-1)?.muted, false, "Undeafen restores a previously live mic");
  await client.setDeafened(true);
  await client.setMuted(false);
  assert.equal(states.at(-1)?.deafened, false, "Explicit unmute also undeafens");
  assert.deepEqual(calls, [], "Preferences need neither capture nor API calls");
  await client.setMuted(true);
  await client.join();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(Peer.latest.senders[0].track, null, "Pre-join mute must prevent publication of audible audio");
  await client.leave();
  assert.equal(states.at(-1)?.muted, true, "Leaving retains mute intent");
  await client.setMuted(false);
  await client.setDeafened(true);
  client.setInputVolume(145);
  client.setVoiceProcessingStrength(37);
  const nextStates: CallViewState[] = [];
  const next = new PublicCallClient((state) => nextStates.push(state), "/api/channels/next00000000/media");
  next.copyAudioPreferencesFrom(client);
  assert.equal(nextStates.at(-1)?.phase, "idle");
  assert.equal(nextStates.at(-1)?.deafened, true);
  assert.equal(nextStates.at(-1)?.inputVolume, 145);
  assert.equal(nextStates.at(-1)?.voiceProcessingStrength, 37);
  assert.equal(nextStates.at(-1)?.selfId, undefined);
  await next.setDeafened(false);
  assert.equal(nextStates.at(-1)?.muted, false, "Switching channels retains the pre-deafen intent too");
});

test("pre-join microphone selection is retained and can be supplied when testing", async (t) => {
  const { client, install } = setup(t);
  const deviceIds: Array<ConstrainDOMString | undefined> = [];
  install("navigator", { mediaDevices: { getUserMedia: async (constraints: MediaStreamConstraints) => {
    deviceIds.push((constraints.audio as MediaTrackConstraints).deviceId);
    return new Stream([new Track()]);
  } } });

  await client.changeMicrophone("desk-mic");
  await client.startLocalMicTest();
  assert.deepEqual(deviceIds[0], { exact: "desk-mic" });
  client.stopLocalMicTest();

  await client.startLocalMicTest("headset-mic");
  assert.deepEqual(deviceIds[1], { exact: "headset-mic" });
  client.stopLocalMicTest();
  await client.join();
  assert.deepEqual(deviceIds[2], { exact: "headset-mic" }, "Join without an override keeps the pre-join selection");
  await client.leave();
  await client.startLocalMicTest("");
  assert.equal(deviceIds[3], undefined, "An explicit system default clears a removed or previously selected device");
  client.stopLocalMicTest();
  await client.join("Guest", "");
  assert.equal(deviceIds[4], undefined);
});

test("voice processing defaults to 25%, clamps updates, and changes the live capture", async (t) => {
  const { client, track, states } = setup(t);
  await client.join();
  assert.equal(states.at(-1)?.voiceProcessingStrength, 25);
  const capture = (client as unknown as { captures: Map<Track, { setVoiceProcessingStrength(strength: number): void }> }).captures.get(track)!;
  const update = t.mock.method(capture, "setVoiceProcessingStrength");
  client.setVoiceProcessingStrength(125);
  assert.equal(states.at(-1)?.voiceProcessingStrength, 100);
  assert.deepEqual(update.mock.calls.map((call) => call.arguments), [[100]]);
});

test("monitor capture replacement stays private and uses the replacement local stream", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join();
  await client.setMonitoring(true);
  await client.setMuted(false);
  await client.setDeafened(false);
  assert.equal(states.at(-1)!.muted, true);
  assert.equal(states.at(-1)!.deafened, true);
  const replacement = new Track();
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([replacement]) } });
  await client.setNoiseSuppression("off");
  assert.equal(Peer.all[0].senders[0].track, null);
  assert.equal(track.readyState, "ended");
  assert.equal(states.at(-1)!.monitorStream?.getAudioTracks()[0], replacement);
  await client.setMonitoring(false);
  assert.equal(Peer.all[0].senders[0].track, replacement);
  assert.equal(Peer.all.length, 1);
});

test("mic test still starts when state synchronization is unavailable", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.join();
  const originalFetch = fetch;
  install("fetch", (url: string, options: RequestInit) => url.endsWith("/state")
    ? Promise.resolve(Response.json({ error: "state unavailable" }, { status: 503 })) : originalFetch(url, options));
  await client.setMonitoring(true);
  assert.equal(Peer.all[0].senders[0].track, null);
  assert.equal(states.at(-1)!.monitorStream?.getAudioTracks()[0], track);
  assert.equal(states.at(-1)!.monitoring, true);
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
  const { client, track, stateUpdates, states, install } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout"] });
  await client.join();
  await new Promise((resolve) => setImmediate(resolve));
  const originalFetch = fetch;
  let unavailable = true;
  install("fetch", (url: string, options: RequestInit) => unavailable && url.endsWith("/state")
    ? Promise.resolve(Response.json({}, { status: 503 })) : originalFetch(url, options));
  await client.setMuted(true);
  assert.equal(track.enabled, false);
  assert.equal(Peer.latest.senders[0].track, null);
  await client.setDeafened(true);
  assert.equal(states.at(-1)?.stateSyncPending, true);
  unavailable = false;
  t.mock.timers.tick(250);
  await tick();
  assert.deepEqual(stateUpdates.at(-1), { muted: true, deafened: true });
  assert.equal(states.at(-1)?.stateSyncPending, false);
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
  assert.equal(track.enabled, false);
  assert.equal(states.at(-1)?.muted, true);
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
  await new Promise((resolve) => setImmediate(resolve));
  finishStates.shift()!();
  await Promise.all([deafening, muting]);
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

test("failed join reports immediately while old capability cleanup remains isolated", async (t) => {
  const { client, states, install } = setup(t);
  const original = fetch;
  let joins = 0;
  let finishLeave!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/join")) return Promise.resolve(Response.json({ token: `session-${++joins}`, id: "self", iceServers: [] }));
    if (url.endsWith("/publish") && joins === 1) return Promise.resolve(Response.json({ error: "publication unavailable" }, { status: 503 }));
    if (url.endsWith("/leave") && new Headers(init.headers).get("x-caper-media-token") === "session-1") {
      return new Promise<Response>((resolve) => { finishLeave = () => resolve(new Response(null, { status: 204 })); });
    }
    return original(url, init);
  });

  await client.join("Guest"); // Must not wait for the unresolved Leave request.
  assert.equal(states.at(-1)?.phase, "failed");
  assert.equal(states.at(-1)?.error, "publication unavailable");
  assert.equal(Peer.latest.connectionState, "closed");

  await client.join("Second guest");
  const currentTrack = Peer.latest.senders[0].track!;
  assert.equal(states.at(-1)?.phase, "connected");
  finishLeave();
  await tick();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(currentTrack.readyState, "live", "old cleanup must not touch the newer generation");
});

test("automatic rejoin is not blocked by old capability cleanup", async (t) => {
  const { client, states, install } = setup(t);
  const original = fetch;
  let joins = 0;
  let finishLeave!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/join")) return Promise.resolve(Response.json({ token: `session-${++joins}`, id: "self", iceServers: [] }));
    if (url.endsWith("/leave") && new Headers(init.headers).get("x-caper-media-token") === "session-1") {
      return new Promise<Response>((resolve) => { finishLeave = () => resolve(new Response(null, { status: 204 })); });
    }
    return original(url, init);
  });
  await client.join("Guest");

  await (client as unknown as { rejoin(): Promise<void> }).rejoin();
  assert.equal(joins, 2);
  assert.equal(states.at(-1)?.phase, "connected");
  const currentTrack = Peer.latest.senders[0].track!;
  finishLeave();
  await tick();
  assert.equal(currentTrack.readyState, "live");
  assert.equal(states.at(-1)?.phase, "connected");
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

test("denied microphone permission leaves the call and shows an actionable error", async (t) => {
  const { client, calls, states, install } = setup(t);
  install("navigator", { mediaDevices: { getUserMedia: async () => {
    throw new DOMException("Permission denied", "NotAllowedError");
  } } });

  await client.join("Guest");

  assert.deepEqual(calls, ["join", "leave"]);
  assert.equal(states.at(-1)?.phase, "failed");
  assert.equal(states.at(-1)?.error, "Microphone permission was denied. Allow access and try again.");
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
    if (url.endsWith("/leave") && new Headers(init.headers).get("x-caper-media-token") === "session-1") {
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
  assert.equal(new Headers(leaveRequest.headers).get("x-caper-media-token"), "session-1");
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
    leaveToken = new Headers(init.headers).get("x-caper-media-token") ?? undefined;
    return Promise.resolve(new Response(null, { status: 204 }));
  });
  const joining = client.join("Guest");
  await new Promise((resolve) => setImmediate(resolve));
  await client.leave();
  finish(Response.json({ token: "late", id: "late", iceServers: [] }));
  await joining;
  assert.equal(track.readyState, "ended");
  assert.equal(leaveToken, "late");
});

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));
const changedEvent = () => new TextEncoder().encode("event: changed\ndata: {}\n\n");
const snapshotEvent = (participants: object[], revision?: number) => new TextEncoder().encode(
  `event: snapshot\ndata: ${JSON.stringify({ participants, ...(revision === undefined ? {} : { revision }) })}\n\n`,
);

test("Join overlaps silent publication with SSE and state with transport, but gates audio on all readiness", async (t) => {
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
  assert.equal(calls.includes("publish"), true, "silent negotiation must not wait for SSE readiness");
  assert.equal(track.enabled, false, "HTTP headers alone cannot open audio");
  Peer.latest.connectionState = "connecting";
  Peer.latest.iceGatheringState = "gathering"; // A stalled probe must not delay publication.
  events[0].enqueue(new TextEncoder().encode("event: ready\ndata: {}\n\n"));
  await tick();
  assert.equal(calls.includes("publish"), true);
  assert.equal(track.enabled, false, "publication establishes transport with silence");
  assert.equal(typeof snapshot, "undefined");
  assert.equal(typeof state, "function", "registry state updates run during the transport handshake");
  Peer.latest.connectionState = "connected";
  Peer.latest.dispatchEvent(new Event("connectionstatechange"));
  await tick();
  assert.equal(typeof snapshot, "undefined");
  assert.equal(track.enabled, false, "state synchronization must finish before audio is enabled");
  state();
  await tick();
  assert.equal(typeof snapshot, "function");
  assert.equal(track.enabled, false, "the authenticated initial roster still gates audio");
  snapshot();
  await joining;
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(track.enabled, true);
});

test("roster and subscriptions overlap transport setup, but audio waits for it", async (t) => {
  const { client, track, calls, states, install } = setup(t);
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? (calls.push("snapshot"), Promise.resolve(Response.json({ participants: [{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] }] })))
    : original(url, init));
  const applyRemote = Peer.prototype.setRemoteDescription;
  let answers = 0;
  t.mock.method(Peer.prototype, "setRemoteDescription", async function (this: Peer, description: RTCSessionDescriptionInit) {
    // Only the publication answer starts transport; later offers renegotiate a connected peer.
    if (answers++ === 0) { this.connectionState = "connecting"; return; }
    return applyRemote.call(this, description);
  });
  const joining = client.join();
  await tick();
  assert.equal(calls.includes("state"), true);
  assert.equal(calls.includes("snapshot"), true, "the lease renewal overlaps transport setup");
  assert.equal(calls.includes("subscribe"), true, "pulls overlap transport setup");
  assert.equal(calls.includes("negotiate"), true);
  assert.equal(track.enabled, false);
  assert.equal(states.at(-1)?.phase, "joining");
  assert.equal(states.some((state) => state.remoteMedia.length > 0), false, "received audio is withheld while joining");
  Peer.latest.connectionState = "connected";
  Peer.latest.dispatchEvent(new Event("connectionstatechange"));
  await joining;
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.remoteMedia[0]?.trackId, "remote");
  assert.equal(track.enabled, true);
});

test("startup renews the lease despite a pushed roster and keeps newer pushed state", async (t) => {
  const { client, events, track, states, install } = setup(t, { eventsReady: false });
  const original = fetch;
  let renewals = 0;
  let renew!: () => void;
  const initial = [{ id: "other", name: "Other", muted: false, deafened: false, tracks: [] }];
  install("fetch", (url: string, init: RequestInit) => {
    if (!url.endsWith("/snapshot")) return original(url, init);
    assert.equal(new Headers(init.headers).get("x-caper-media-token"), "capability");
    renewals++;
    return new Promise<Response>((resolve) => {
      renew = () => resolve(Response.json({ participants: initial, revision: 1 }));
    });
  });
  const joining = client.join();
  await tick();
  events[0].enqueue(new TextEncoder().encode("event: ready\ndata: {}\n\n"));
  events[0].enqueue(snapshotEvent(initial, 1));
  await tick();
  assert.equal(renewals, 1, "SSE must not suppress the authenticated startup renewal");
  assert.equal(track.enabled, false, "wait for renewal before opening audio");
  assert.equal(states.at(-1)?.phase, "joining");
  events[0].enqueue(snapshotEvent([{ ...initial[0], muted: true }], 2));
  await tick();
  renew();
  await joining;
  assert.equal(renewals, 1);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(track.enabled, true);
  assert.equal(states.at(-1)?.participants[0]?.id, "other");
  assert.equal(states.at(-1)?.participants[0]?.muted, true, "older HTTP state must not overwrite the push");
});

test("publication failure cancels an unfinished SSE handshake without enabling audio", async (t) => {
  const { client, track, states, install } = setup(t, { eventsReady: false });
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/publish")
    ? Promise.resolve(Response.json({ error: "publication unavailable" }, { status: 503 }))
    : original(url, init));
  await client.join();
  assert.equal(states.at(-1)?.phase, "failed");
  assert.equal(track.enabled, false);
  assert.equal(track.readyState, "ended");
  assert.equal(Peer.latest.connectionState, "closed");
});

test("state failure cancels an unfinished transport handshake without enabling audio", async (t) => {
  const { client, track, calls, states, install } = setup(t);
  t.mock.method(Peer.prototype, "setRemoteDescription", async () => { Peer.latest.connectionState = "connecting"; });
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/state")
    ? Promise.resolve(Response.json({ error: "state unavailable" }, { status: 503 }))
    : original(url, init));
  await client.join();
  assert.equal(states.at(-1)?.phase, "failed");
  assert.equal(calls.includes("snapshot"), false);
  assert.equal(track.enabled, false);
  assert.equal(track.readyState, "ended");
  assert.equal(Peer.latest.connectionState, "closed");
});

test("a newer unsynchronized mute change during the initial roster is repaired before opening audio", async (t) => {
  const { client, track, states, install } = setup(t);
  await client.setMuted(true);
  const original = fetch;
  let snapshot!: () => void;
  let latestState!: () => void;
  const updates: Array<{ muted: boolean }> = [];
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/snapshot")) return new Promise<Response>((resolve) => { snapshot = () => resolve(Response.json({ participants: [] })); });
    if (url.endsWith("/state")) {
      updates.push(JSON.parse(init.body as string));
      if (updates.length === 2) return Promise.resolve(Response.json({ error: "temporary state failure" }, { status: 503 }));
      if (updates.length === 3) return new Promise<Response>((resolve) => { latestState = () => resolve(new Response(null, { status: 204 })); });
    }
    return original(url, init);
  });
  const joining = client.join();
  await tick();
  await assert.rejects(client.setMuted(false), /temporary state failure/);
  snapshot();
  await tick();
  assert.deepEqual(updates.map((update) => update.muted), [true, false, false]);
  assert.equal(track.enabled, false);
  assert.equal(states.at(-1)?.phase, "joining");
  latestState();
  await joining;
  assert.equal(track.enabled, true);
  assert.equal(states.at(-1)?.phase, "connected");
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

test("subscription renegotiation may reconnect transport before Join enables audio", async (t) => {
  const { client, track, states, install } = setup(t);
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? Promise.resolve(Response.json({ participants: [{id:"other",name:"Other",muted:false,deafened:false,tracks:[{id:"remote",kind:"microphone"}]}] }))
    : original(url, init));
  t.mock.method(Peer.prototype, "setRemoteDescription", async (description: RTCSessionDescriptionInit) => {
    if (description.type === "offer") Peer.latest.connectionState = "connecting";
  });

  const joining = client.join();
  await tick();
  await tick();
  assert.equal(Peer.latest.connectionState, "connecting");
  assert.equal(states.at(-1)?.phase, "joining");
  assert.equal(track.enabled, false, "renegotiating transport must remain silent");

  Peer.latest.connectionState = "connected";
  Peer.latest.dispatchEvent(new Event("connectionstatechange"));
  await joining;
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(track.enabled, true);
});

test("leaving while subscription transport reconnects cancels Join and keeps audio closed", async (t) => {
  const { client, track, states, install } = setup(t);
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? Promise.resolve(Response.json({ participants: [{id:"other",name:"Other",muted:false,deafened:false,tracks:[{id:"remote",kind:"microphone"}]}] }))
    : original(url, init));
  t.mock.method(Peer.prototype, "setRemoteDescription", async (description: RTCSessionDescriptionInit) => {
    if (description.type === "offer") Peer.latest.connectionState = "connecting";
  });

  const joining = client.join();
  await tick();
  await tick();
  assert.equal(Peer.latest.connectionState, "connecting");
  await client.leave();
  await joining;
  assert.equal(states.at(-1)?.phase, "idle");
  assert.equal(track.enabled, false);
  assert.equal(track.readyState, "ended");
  assert.equal(Peer.latest.connectionState, "closed");
});

test("subscription transport reconnection still times out without enabling audio", async (t) => {
  const { client, track, states, install } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? Promise.resolve(Response.json({ participants: [{id:"other",name:"Other",muted:false,deafened:false,tracks:[{id:"remote",kind:"microphone"}]}] }))
    : original(url, init));
  t.mock.method(Peer.prototype, "setRemoteDescription", async (description: RTCSessionDescriptionInit) => {
    if (description.type === "offer") Peer.latest.connectionState = "connecting";
  });

  const joining = client.join();
  await tick();
  await tick();
  assert.equal(Peer.latest.connectionState, "connecting");
  t.mock.timers.tick(12_000);
  await joining;
  assert.equal(states.at(-1)?.phase, "failed");
  assert.match(states.at(-1)?.error ?? "", /Timed out waiting for connectionstatechange/);
  assert.equal(track.enabled, false);
  assert.equal(track.readyState, "ended");
});

test("a received participant track is exposed for that participant's speaking indicator", async (t) => {
  const { client, states, install } = setup(t);
  const original = fetch;
  install("fetch", (url: string, options: RequestInit) => {
    if (url.endsWith("/snapshot")) return Promise.resolve(Response.json({
      participants: [{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] }],
    }));
    return original(url, options);
  });

  await client.join();

  const remote = states.at(-1)?.remoteMedia.find((media) => media.participantId === "other");
  assert.equal(remote?.trackId, "remote");
  assert.equal(remote?.kind, "microphone");
  assert.equal(remote?.stream.getAudioTracks().length, 1);
});

test("remote audio is withheld until the join has completed", async (t) => {
  const { client, states, install } = setup(t);
  const original = fetch;
  install("fetch", (url: string, options: RequestInit) => {
    if (url.endsWith("/snapshot")) return Promise.resolve(Response.json({
      participants: [{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] }],
    }));
    return original(url, options);
  });

  await client.join();

  // Subscription happens during joining; no state before "connected" may carry it.
  assert.ok(states.some((state) => state.phase === "joining"));
  assert.deepEqual(states.filter((state) => state.phase !== "connected").map((state) => state.remoteMedia.length).filter(Boolean), []);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.remoteMedia.length, 1);
});

test("cancel before SSE readiness stops silent publication and releases capture", async (t) => {
  const { client, track, calls, states } = setup(t, { eventsReady: false });
  const joining = client.join();
  await tick();
  assert.equal(calls.includes("publish"), true);
  assert.equal(track.enabled, false);
  await client.leave();
  await joining;
  assert.equal(Peer.latest.connectionState, "closed");
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

test("gateway loss during startup and an established call preserves media while the shared socket reconnects", async (t) => {
  const { client, track, events, states, install } = setup(t);
  const original = fetch;
  let publish!: () => void;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/publish")
    ? new Promise<Response>((resolve) => { publish = () => resolve(Response.json({sessionDescription:{type:"answer",sdp:"v=0"}})); })
    : original(url, init));
  const joining = client.join();
  await tick();
  t.mock.timers.enable({ apis: ["setTimeout"] });
  events[0].close();
  await tick();
  assert.equal(track.readyState, "live");
  t.mock.timers.tick(188);
  await tick();
  publish();
  await joining;
  assert.equal(states.at(-1)?.phase, "connected");
  install("fetch", original);
  const peer = Peer.latest;
  const outgoing = Peer.latest.senders[0].track!;
  events.at(-1)!.close();
  await tick();
  assert.equal(outgoing.readyState, "live");
  assert.equal(outgoing.enabled, true);
  assert.equal(states.at(-1)?.phase, "connected");
  t.mock.timers.tick(375);
  await tick();
  await tick();
  assert.equal(events.length, 3, "reopen the gateway subscription using the existing voice session");
  assert.equal(Peer.latest, peer);
  assert.equal(outgoing.readyState, "live");
  assert.equal(states.at(-1)?.phase, "connected");
});

test("gateway-only outage never restarts healthy voice while authenticated renewals succeed", async (t) => {
  const { client, events, states } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  await client.join();
  const peer = Peer.latest;
  events[0].close();
  await tick();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.liveUpdatesPending, true);
  assert.equal(Peer.latest, peer);
  assert.equal(peer.senders[0].track?.enabled, true);
  t.mock.timers.tick(188);
  await tick();
  await tick();
  assert.equal(events.length, 2);
  assert.equal(states.at(-1)?.liveUpdatesPending, false);
  assert.equal(Peer.latest, peer);
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

test("rejoin restores local microphone testing without reattaching the public sender", async (t) => {
  const { client, states } = setup(t);
  await client.join();
  await client.setMonitoring(true);
  const previousPeers = Peer.all.length;
  await (client as unknown as { rejoin(): Promise<void> }).rejoin();
  const [publicPeer] = Peer.all.slice(previousPeers);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.monitoring, true);
  assert.equal(states.at(-1)?.muted, true);
  assert.equal(publicPeer.senders[0].track, null);
  assert.equal(Peer.all.length, previousPeers + 1);
  assert.ok(states.at(-1)?.monitorStream);
});

test("a pushed gateway snapshot subscribes without waiting for the heartbeat timer", async (t) => {
  const { client, events, calls, states } = setup(t);
  await client.join();
  events[0].enqueue(snapshotEvent([
    { id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] },
  ], 1));
  await tick();
  assert.ok(calls.includes("subscribe"));
  assert.ok(calls.includes("negotiate"));
  assert.deepEqual(Peer.latest.remoteDescriptions, [
    { type: "answer", sdp: senderSdp },
    { type: "offer", sdp: senderSdp },
  ], "subscription renegotiation must not reset the microphone's DTX preference");
  assert.equal(states.at(-1)?.remoteMedia[0]?.trackId, "remote");
});

test("pushed gateway snapshots reconcile immediately without an extra roster request", async (t) => {
  const { client, events, install, states } = setup(t);
  await client.join();
  const original = fetch;
  let snapshots = 0;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? (snapshots++, Promise.resolve(Response.json({ participants: [] })))
    : original(url, init));
  events[0].enqueue(snapshotEvent([{ id: "one", name: "One", muted: false, deafened: false, tracks: [] }], 1));
  await tick();
  events[0].enqueue(snapshotEvent([{ id: "two", name: "Two", muted: true, deafened: false, tracks: [] }], 2));
  await tick();
  assert.equal(states.at(-1)?.participants[0]?.id, "two");
  assert.equal(snapshots, 0, "full snapshots need no immediate HTTP roster fetch");
});

test("a pushed newer revision supersedes an older HTTP snapshot already in flight", async (t) => {
  const { client, events, states, install } = setup(t);
  await client.join();
  const original = fetch;
  let finish!: () => void;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/snapshot")
    ? new Promise<Response>((resolve) => { finish = () => resolve(Response.json({ participants: [], revision: 1 })); })
    : original(url, init));
  const polling = (client as unknown as { poll(): Promise<void> }).poll();
  await tick();
  events[0].enqueue(snapshotEvent([{ id: "new", name: "New", muted: false, deafened: false, tracks: [] }], 2));
  finish();
  await polling;
  await tick();
  await tick();
  assert.equal(states.at(-1)?.participants[0]?.id, "new");
});

test("draining reopens control with the same call and does not rejoin or replace the peer", async (t) => {
  const { client, events, calls, states } = setup(t);
  await client.join();
  const peer = Peer.latest;
  const joins = calls.filter((call) => call === "join").length;
  t.mock.timers.enable({ apis: ["setTimeout"] });
  events[0].enqueue(new TextEncoder().encode("event: draining\ndata: {}\n\n"));
  events[0].close();
  await tick();
  t.mock.timers.tick(50);
  await tick();
  assert.equal(events.length, 2);
  assert.equal(Peer.latest, peer);
  assert.equal(calls.filter((call) => call === "join").length, joins);
  assert.equal(states.at(-1)?.phase, "connected");
});

test("repeated gateway migrations preserve the active call and stop after leaving", async (t) => {
  const { client, events, calls, states } = setup(t);
  await client.join();
  const peer = Peer.latest;
  for (let attempt = 0; attempt < 10; attempt++) {
    events.at(-1)!.enqueue(new TextEncoder().encode("event: draining\ndata: {}\n\n"));
    await tick();
  }
  assert.equal(events.length, 11);
  events.at(-1)!.enqueue(snapshotEvent([{ id: "phone", name: "Phone", muted: true, deafened: false, tracks: [] }], 20));
  await tick();
  assert.equal(states.at(-1)?.participants[0]?.muted, true);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(Peer.latest, peer);
  assert.equal(calls.filter((op) => op === "join").length, 1);
  client.leaveImmediately();
  const count = events.length;
  events.at(-1)!.enqueue(new TextEncoder().encode("event: draining\ndata: {}\n\n"));
  await tick();
  assert.equal(events.length, count);
});

test("queued pushed snapshots never suppress a scheduled lease heartbeat", async (t) => {
  t.mock.timers.enable({ apis: ["setInterval"] });
  const { client, events, install, states } = setup(t);
  await client.join();
  const original = fetch;
  let requests = 0;
  let finish!: () => void;
  const latest = { participants: [{ id: "other", name: "Other", muted: true, deafened: false, tracks: [] }], revision: 2 };
  install("fetch", (url: string, init: RequestInit) => {
    if (!url.endsWith("/snapshot")) return original(url, init);
    requests++;
    if (requests === 1) return new Promise<Response>((resolve) => { finish = () => resolve(Response.json({ participants: [], revision: 1 })); });
    return Promise.resolve(Response.json(latest));
  });
  events[0].enqueue(changedEvent());
  await tick();
  events[0].enqueue(snapshotEvent(latest.participants, 2));
  await tick();
  t.mock.timers.tick(15_000);
  finish();
  await tick();
  await tick();
  assert.equal(requests, 2, "the queued push cannot replace the authenticated renewal");
  assert.equal(states.at(-1)?.participants[0]?.muted, true);
});

test("rapid mute changes coalesce to the latest intent behind an in-flight state write", async (t) => {
  const { client, install, states } = setup(t);
  await client.join();
  const original = fetch;
  const updates: boolean[] = [];
  let finish!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (!url.endsWith("/state")) return original(url, init);
    updates.push(JSON.parse(init.body as string).muted);
    if (updates.length === 1) return new Promise<Response>((resolve) => { finish = () => resolve(new Response(null, { status: 204 })); });
    return Promise.resolve(new Response(null, { status: 204 }));
  });
  const first = client.setMuted(true);
  await tick();
  const changes = [client.setMuted(false), client.setMuted(true), client.setMuted(false)];
  await tick();
  finish();
  await Promise.all([first, ...changes]);
  assert.deepEqual(updates, [true, false], "do not replay obsolete toggles after a slow request");
  assert.equal(states.at(-1)?.muted, false);
  assert.equal(Peer.latest.senders[0].track?.enabled, true);
});

test("mute writes and pushed roster updates cannot block the scheduled lease renewal", async (t) => {
  const { client, install, states, events, calls } = setup(t);
  t.mock.timers.enable({ apis: ["setInterval", "setTimeout"] });
  await client.join();
  const original = fetch;
  const participants = [{ id: "other", name: "Other", muted: true, deafened: false, tracks: [] }];
  let renewals = 0;
  let finish!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/snapshot")) {
      renewals++;
      return Promise.resolve(Response.json({ participants, revision: 1 }));
    }
    if (url.endsWith("/state")) return new Promise<Response>((resolve, reject) => {
      finish = () => resolve(new Response(null, { status: 204 }));
      init.signal!.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), { once: true });
    });
    return original(url, init);
  });
  const muting = client.setMuted(true).catch(() => undefined);
  await tick();
  events[0].enqueue(snapshotEvent(participants, 1));
  await tick();
  const pushedMuted = states.at(-1)?.participants[0]?.muted;
  t.mock.timers.tick(15_000);
  await tick();
  assert.ok(renewals > 0, "lease renewal must not queue behind a state request");
  assert.equal(pushedMuted, true, "show pushed state before heartbeat, without waiting for our own write");
  assert.equal(states.at(-1)?.participants[0]?.muted, true);
  assert.equal(calls.filter((call) => call === "join").length, 1);
  finish();
  await muting;
});

test("draining with failed state writes retries latest intent without replacing the voice session", async (t) => {
  const { client, events, install, states, calls } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  await client.join();
  const peer = Peer.latest;
  const original = fetch;
  const updates: Array<{ muted: boolean; deafened: boolean }> = [];
  const tokens: string[] = [];
  install("fetch", (url: string, init: RequestInit) => {
    tokens.push((init.headers as Record<string, string>)["x-caper-media-token"]);
    if (!url.endsWith("/state")) return original(url, init);
    updates.push(JSON.parse(init.body as string));
    return Promise.resolve(updates.length <= 2
      ? Response.json({}, { status: updates.length === 1 ? 503 : 504 })
      : new Response(null, { status: 204 }));
  });
  await client.setMuted(true);
  assert.equal(peer.senders[0].track, null);
  assert.equal(states.at(-1)?.stateSyncPending, true);
  events[0].enqueue(new TextEncoder().encode("event: draining\ndata: {}\n\n"));
  events[0].close();
  await tick();
  t.mock.timers.tick(50);
  await tick();
  await client.setDeafened(true);
  await Promise.all([client.setDeafened(false), client.setMuted(false)]);
  t.mock.timers.tick(300);
  await tick();
  assert.deepEqual(updates.at(-1), { muted: false, deafened: false, sequence: 5 });
  assert.equal(states.at(-1)?.stateSyncPending, false);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.some((state) => state.error !== undefined), false);
  assert.equal(events.length, 2);
  assert.equal(Peer.latest, peer);
  assert.equal(peer.getSenders()[0].track?.enabled, true);
  assert.equal(calls.filter((call) => call === "join").length, 1);
  assert.equal(calls.includes("leave"), false);
  assert.deepEqual([...new Set(tokens)], ["capability"]);
});

test("blocked subscription negotiation cannot delay roster, mute synchronization, or lease renewal", async (t) => {
  const { client, events, install, states, stateUpdates } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  await client.join();
  const original = fetch;
  let finish!: () => void;
  let renewals = 0;
  let muted = false;
  const roster = () => [{ id: "other", name: "Other", muted, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] }];
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/subscribe")) return new Promise<Response>((resolve) => {
      finish = () => { void original(url, init).then(resolve); };
    });
    if (url.endsWith("/snapshot")) {
      renewals++;
      return Promise.resolve(Response.json({ participants: roster(), revision: muted ? 2 : 1 }));
    }
    return original(url, init);
  });
  events[0].enqueue(snapshotEvent(roster(), 1));
  await tick();
  assert.equal(typeof finish, "function", "subscription is waiting on its provider response");
  muted = true;
  events[0].enqueue(snapshotEvent(roster(), 2));
  await tick();
  assert.equal(states.at(-1)?.participants[0]?.muted, true);
  await client.setMuted(true);
  assert.equal(stateUpdates.at(-1)?.muted, true);
  for (let heartbeat = 0; heartbeat < 2; heartbeat++) {
    t.mock.timers.tick(15_000);
    await tick();
  }
  assert.equal(renewals, 2, "heartbeat must run while SDP work is still pending");
  assert.equal(states.at(-1)?.phase, "connected");
  finish();
  await tick();
  await tick();
  assert.equal(states.at(-1)?.participants[0]?.muted, true);
});

test("a late timed-out mute write is repaired from a newer pushed self snapshot", async (t) => {
  const { client, events, install, states } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  await client.join();
  const original = fetch;
  const updates: boolean[] = [];
  install("fetch", (url: string, init: RequestInit) => {
    if (!url.endsWith("/state")) return original(url, init);
    updates.push(JSON.parse(init.body as string).muted);
    if (updates.length === 1) return new Promise<Response>((_resolve, reject) => {
      init.signal!.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), { once: true });
    });
    return Promise.resolve(new Response(null, { status: 204 }));
  });
  const muting = client.setMuted(true);
  await tick();
  t.mock.timers.tick(5_000);
  await muting;
  await client.setMuted(false);
  assert.deepEqual(updates, [true, false]);
  // The first request committed after the unmute, even though its HTTP wait aborted.
  events[0].enqueue(snapshotEvent([{ id: "self", name: "Self", muted: true, deafened: false, tracks: [] }], 8));
  await tick();
  assert.deepEqual(updates, [true, false, false]);
  assert.equal(states.at(-1)?.muted, false);
  assert.equal(Peer.latest.senders[0].track?.enabled, true);
  events[0].enqueue(snapshotEvent([{ id: "self", name: "Self", muted: false, deafened: false, tracks: [] }], 9));
  await tick();
  assert.equal(states.at(-1)?.participants[0]?.muted, false);
  assert.equal(states.at(-1)?.stateSyncPending, false);
  assert.equal(updates.length, 3, "an agreeing snapshot must not trigger another write");
});

test("leaving cancels pending state retries and ignores an old mute action after rejoin", async (t) => {
  const { client, install, states } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  await client.join();
  const original = fetch;
  let writes = 0;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/state")) {
      writes++;
      return Promise.resolve(Response.json({}, { status: 503 }));
    }
    return original(url, init);
  });
  await client.setMuted(true);
  assert.equal(states.at(-1)?.stateSyncPending, true);
  const oldAction = client.setMuted(false);
  await client.leave();
  await oldAction;
  install("fetch", original);
  await client.join();
  t.mock.timers.tick(1_000);
  await tick();
  assert.equal(writes, 1, "no old-generation write or retry");
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.muted, false);
  assert.equal(states.at(-1)?.stateSyncPending, false);
  assert.equal(states.some((state) => state.error !== undefined), false);
});

for (const operation of ["join", "publish", "subscribe", "negotiate", "close"]) {
  test(`${operation} retries explicit drain rejection without replacing the call`, async (t) => {
    const { client, install, events, states, calls } = setup(t);
    t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
    const startup = operation === "join" || operation === "publish";
    if (!startup) await client.join();
    const original = fetch;
    let attempts = 0;
    install("fetch", (url: string, init: RequestInit) => {
      if (url.endsWith(`/${operation}`) && ++attempts === 1) return Promise.resolve(Response.json({ error: "API is draining", code: "api_draining" }, { status: 503 }));
      return original(url, init);
    });
    const joining = startup ? client.join() : Promise.resolve();
    if (!startup) {
      events[0].enqueue(snapshotEvent([{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] }], 1));
      await tick();
      if (operation === "close") events[0].enqueue(snapshotEvent([], 2));
    }
    await tick();
    assert.equal(attempts, 1);
    t.mock.timers.tick(250);
    await tick();
    await joining;
    assert.equal(attempts, 2);
    assert.equal(states.at(-1)?.phase, "connected");
    assert.equal(Peer.all.length, 1);
    assert.equal(calls.filter((op) => op === "join").length, 1);
    assert.equal(calls.includes("leave"), false);
  });
}

for (const status of [502, 503, 504]) {
  test(`ambiguous subscription ${status} is never blindly replayed`, async (t) => {
    const { client, install, events, states } = setup(t);
    t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
    await client.join();
    const original = fetch;
    let attempts = 0;
    install("fetch", (url: string, init: RequestInit) => {
      if (url.endsWith("/subscribe")) {
        attempts++;
        return Promise.resolve(Response.json({ error: "unknown mutation outcome" }, { status }));
      }
      return original(url, init);
    });
    events[0].enqueue(snapshotEvent([{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "remote", kind: "microphone" }] }], 1));
    await tick();
    assert.equal(attempts, 1);
    assert.equal(states.at(-1)?.phase, "reconnecting", "uncertain SDP still needs a new session");
  });
}

test("a departure before subscribing skips only that track and retains the caller", async (t) => {
  const { client, install, events, states, calls } = setup(t);
  await client.join();
  const peer = Peer.latest;
  const original = fetch;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/subscribe") && JSON.parse(init.body as string).trackId === "gone"
    ? Promise.resolve(Response.json({ error: "track not found", code: "track_gone" }, { status: 404 }))
    : original(url, init));
  events[0].enqueue(snapshotEvent([{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "gone", kind: "microphone" }, { id: "live", kind: "microphone" }] }], 1));
  await tick();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(Peer.latest, peer);
  assert.equal(states.at(-1)?.remoteMedia[0]?.trackId, "live", "continue subscribing to unaffected tracks");
  assert.equal(calls.includes("leave"), false);
});

test("a source leaving during subscription completes negotiation then closes only that MID", async (t) => {
  const { client, install, events, states, calls } = setup(t);
  await client.join();
  const peer = Peer.latest;
  const original = fetch;
  let finish!: () => void;
  install("fetch", (url: string, init: RequestInit) => url.endsWith("/subscribe")
    ? new Promise<Response>((resolve) => { finish = () => { void original(url, init).then(resolve); }; })
    : original(url, init));
  events[0].enqueue(snapshotEvent([{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "departing", kind: "microphone" }] }], 1));
  await tick();
  events[0].enqueue(snapshotEvent([], 2));
  await tick();
  finish();
  await tick();
  await tick();
  assert.deepEqual(calls.filter((op) => ["negotiate", "close"].includes(op)), ["negotiate", "close"]);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(Peer.latest, peer);
  assert.equal(peer.senders[0].track?.enabled, true);
  assert.equal(states.at(-1)?.remoteMedia.length, 0);
  assert.equal(calls.includes("leave"), false);
});

test("drain retries are bounded and leave cancels retry backoff", async (t) => {
  const { client, install } = setup(t);
  t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
  await client.join();
  const original = fetch;
  let attempts = 0;
  install("fetch", (url: string, init: RequestInit) => {
    if (!url.endsWith("/subscribe")) return original(url, init);
    attempts++;
    return Promise.resolve(Response.json({ error: "API is draining", code: "api_draining" }, { status: 503 }));
  });
  const api = client as unknown as { api(operation: string): Promise<void> };
  const exhausted = assert.rejects(api.api("subscribe"), /API is draining/);
  await tick();
  for (const delay of [250, 500, 1_000]) { t.mock.timers.tick(delay); await tick(); }
  await exhausted;
  assert.equal(attempts, 4);
  const cancelled = assert.rejects(api.api("subscribe"));
  await tick();
  await client.leave();
  await cancelled;
  t.mock.timers.tick(2_000);
  await tick();
  assert.equal(attempts, 5, "no retry after leaving");
});

test("successful recovery resets the consecutive failure budget", async (t) => {
  const { client, states } = setup(t);
  await client.join();
  const recovery = client as unknown as { rejoin(): Promise<void> };
  for (let recoveryNumber = 0; recoveryNumber < 5; recoveryNumber++) {
    await recovery.rejoin();
    assert.equal(states.at(-1)?.phase, "connected", "successful recoveries must not consume a lifetime quota");
  }
});

test("a pending old microphone request cannot block controls in a replacement call", async (t) => {
  const { client, install, states, stateUpdates } = setup(t);
  await client.join();
  const capture = [...(client as unknown as { captures: Map<Track, { resume(): Promise<void> }> }).captures.values()][0];
  const resume = t.mock.method(capture, "resume", async () => { throw new Error("old pipeline was stopped"); });
  let finish!: () => void;
  let captures = 0;
  install("navigator", { mediaDevices: { getUserMedia: () => ++captures === 1
    ? new Promise<Stream>((resolve) => { finish = () => resolve(new Stream([new Track()])); })
    : Promise.resolve(new Stream([new Track()])) } });
  const replacing = assert.rejects(client.changeMicrophone("old-device"), { name: "AbortError" });
  await tick();
  await client.leave();
  await client.join();
  let applied = false;
  const muting = client.setMuted(true).then(() => { applied = true; });
  await tick();
  assert.equal(applied, true, "new media queue must not wait for uncancellable old getUserMedia");
  assert.equal(stateUpdates.at(-1)?.muted, true);
  finish();
  await replacing;
  await muting;
  assert.equal(resume.mock.callCount(), 0, "never resurrect an old pipeline or let its failed rollback restart the new call");
  assert.equal(states.at(-1)?.phase, "connected");
});

test("a late ended-track cleanup failure cannot reconnect a replacement call", async (t) => {
  const { client, install, states, track } = setup(t);
  await client.join();
  const original = fetch;
  let failClose!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/join")) return Promise.resolve(Response.json({token:"replacement-capability",id:"new-self",iceServers:[]}));
    if (url.endsWith("/close")) return new Promise<Response>((_resolve, reject) => { failClose = () => reject(new TypeError("late close failure")); });
    return original(url, init);
  });
  track.dispatchEvent(new Event("ended"));
  await tick();
  await client.leave();
  await client.join();
  const peer = Peer.latest;
  failClose();
  await tick();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(Peer.latest, peer);
  assert.equal(peer.senders[0].track?.enabled, true);
});

for (const failure of [502, 503, 504, "network", "timeout"] as const) {
  test(`remote departure with ${failure} cleanup failure keeps the existing call and retries`, async (t) => {
    const { client, install, events, states, calls, track } = setup(t);
    t.mock.timers.enable({ apis: ["setTimeout", "setInterval"] });
    await client.join();
    const peer = Peer.latest;
    const before = states.length;
    events[0].enqueue(snapshotEvent([{ id: "phone", name: "Phone", muted: false, deafened: false, tracks: [{ id: "phone-mic", kind: "microphone" }] }], 1));
    await tick();
    assert.equal(states.at(-1)?.remoteMedia.length, 1);
    const original = fetch;
    let attempts = 0;
    install("fetch", (url: string, init: RequestInit) => {
      if (url.endsWith("/close") && ++attempts === 1) {
        if (failure === "network") return Promise.reject(new TypeError("connection lost"));
        if (failure === "timeout") return new Promise<Response>((_resolve, reject) => {
          init.signal!.addEventListener("abort", () => reject(new DOMException("Aborted", "AbortError")), { once: true });
        });
        return Promise.resolve(Response.json({ error: "temporary failure" }, { status: failure }));
      }
      return original(url, init);
    });
    events[0].enqueue(snapshotEvent([], 2));
    await tick();
    assert.equal(states.at(-1)?.remoteMedia.length, 0, "remove departed audio before waiting for cleanup");
    if (failure === "timeout") { t.mock.timers.tick(5_000); await tick(); }
    t.mock.timers.tick(15_000);
    await tick();
    await tick();
    assert.equal(attempts, 2, "cleanup retries without creating another session");
    assert.ok(states.slice(before).every((state) => state.phase === "connected"));
    assert.equal(Peer.latest, peer);
    assert.equal(peer.connectionState, "connected");
    assert.equal(track.readyState, "live");
    assert.equal(track.enabled, true);
    assert.equal(calls.filter((op) => op === "join").length, 1);
    assert.equal(calls.includes("leave"), false);
  });
}

test("join carries current mute/deafen intent and synchronizes before slow publication finishes", async (t) => {
  const { client, install, stateUpdates } = setup(t);
  await client.setDeafened(true);
  const original = fetch;
  let joinBody: unknown;
  let finish!: () => void;
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/join")) joinBody = JSON.parse(init.body as string);
    if (url.endsWith("/publish")) return new Promise<Response>((resolve) => {
      finish = () => { void original(url, init).then(resolve); };
    });
    return original(url, init);
  });
  const joining = client.join("Laptop");
  await tick();
  try {
    assert.deepEqual(joinBody, {
      name: "Laptop", muted: true, deafened: true,
      publish: { mid: "0", sessionDescription: { type: "offer", sdp: "v=0\r\na=mid:0\r\n" } },
    });
    assert.deepEqual(stateUpdates, [{ muted: true, deafened: true }]);
  } finally { finish(); await joining; }
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

test("join publishes the microphone in the same request when the API answers it", async (t) => {
  const { client, install, calls, track, states } = setup(t);
  const original = fetch;
  const iceServers = [{ urls: "turn:turn.example:3478", username: "u", credential: "c" }];
  install("fetch", async (url: string, init: RequestInit) => {
    if (!url.endsWith("/join")) return original(url, init);
    calls.push("join");
    return Response.json({ token: "capability", id: "self", iceServers, publish: { trackId: "mine", tracks: [{ mid: "0" }], sessionDescription: { type: "answer", sdp: providerSdp } } });
  });
  await client.join();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(calls.includes("publish"), false, "no second signaling request");
  assert.deepEqual(Peer.latest.configuration.iceServers, iceServers, "TURN is configured before the offer is applied");
  assert.equal(Peer.latest.remoteDescriptions[0]?.sdp, senderSdp);
  assert.equal(Peer.latest.senders[0].track, track);
  assert.equal(track.enabled, true);
});

test("an API without combined publication falls back to a separate publish", async (t) => {
  const { client, install, calls, states } = setup(t);
  const original = fetch;
  const bodies: Array<Record<string, unknown>> = [];
  install("fetch", async (url: string, init: RequestInit) => {
    if (url.endsWith("/join")) {
      const body = JSON.parse(init.body as string);
      bodies.push(body);
      if (body.publish) return Response.json({ error: "unknown field `publish`" }, { status: 422 });
    }
    return original(url, init);
  });
  await client.join();
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(bodies.length, 2);
  assert.equal(bodies[1].publish, undefined);
  assert.equal(calls.filter((op) => op === "publish").length, 1);
  assert.equal(calls.includes("leave"), false);
});

test("a listed source that cannot be pulled yet is retried shortly", async (t) => {
  const { client, install, events, states } = setup(t);
  await client.join();
  const original = fetch;
  let refusals = 0;
  const roster = [{ id: "other", name: "Other", muted: false, deafened: false, tracks: [{ id: "fresh", kind: "microphone" }] }];
  install("fetch", (url: string, init: RequestInit) => {
    if (url.endsWith("/subscribe") && refusals++ === 0) {
      return Promise.resolve(Response.json({ error: "track not found", code: "track_gone" }, { status: 404 }));
    }
    if (url.endsWith("/snapshot")) return Promise.resolve(Response.json({ participants: roster }));
    return original(url, init);
  });
  events[0].enqueue(snapshotEvent(roster, 1));
  await tick();
  assert.equal(states.at(-1)?.remoteMedia.length, 0);
  await new Promise((resolve) => setTimeout(resolve, 650));
  await tick();
  assert.equal(refusals, 2);
  assert.equal(states.at(-1)?.phase, "connected");
  assert.equal(states.at(-1)?.remoteMedia[0]?.trackId, "fresh");
});
