import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { fakerEN as faker } from "@faker-js/faker";
import { PublicCallClient, waitFor } from "../media/client.ts";
import type { CallViewState } from "../media/types.ts";

class Track extends EventTarget {
  kind = "audio";
  enabled = true;
  readyState = "live";
  stop() { this.readyState = "ended"; }
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
  ontrack?: (event: { track: Track }) => void;
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
    if (description?.type === "offer") this.ontrack?.({ track: new Track() });
  }
  getSenders() { return this.senders; }
  getReceivers() { return []; }
  close() { this.connectionState = "closed"; }
}

function setup(t: TestContext) {
  Peer.all = [];
  const track = new Track();
  const calls: string[] = [];
  const joinedNames: string[] = [];
  const stateUpdates: Array<{ muted: boolean; deafened: boolean }> = [];
  const states: CallViewState[] = [];
  const restore: Array<() => void> = [];
  const install = (key: string, value: unknown) => {
    const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
    Object.defineProperty(globalThis, key, { value, configurable: true });
    restore.push(() => { if (descriptor) Object.defineProperty(globalThis, key, descriptor); else Reflect.deleteProperty(globalThis, key); });
  };
  install("window", globalThis);
  install("navigator", { mediaDevices: { getUserMedia: async () => new Stream([track]) } });
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
    if (op === "publish") return Response.json({ trackId: "private-track", sessionDescription: { type: "answer", sdp: "v=0" } });
    if (op === "subscribe") return Response.json({ requiresImmediateRenegotiation: true, tracks: [{ mid: "1" }], sessionDescription: { type: "offer", sdp: "v=0" } });
    if (op === "snapshot") return Response.json({ participants: [] });
    if (op === "state") stateUpdates.push(JSON.parse(options.body as string));
    return new Response(null, { status: 204 });
  });
  const client = new PublicCallClient((state) => states.push(state));
  t.after(() => { client.leaveImmediately(); restore.reverse().forEach((fn) => fn()); });
  return { client, track, calls, joinedNames, stateUpdates, states, install };
}

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
  await client.setMuted(true);
  await client.setNoiseSuppression("off");
  assert.equal(Peer.latest.senders[0].track, null);
  assert.equal(tracks[0].readyState, "ended");
  assert.equal(tracks[1].enabled, false);
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
  await assert.rejects(client.setNoiseSuppression("off"), /replace failed/);
  assert.equal(track.readyState, "live");
  assert.equal(replacement.readyState, "ended");
  assert.equal(states.at(-1)?.noiseSuppression, "deepfilter");
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

test("leave returns the view to idle before a slow server cleanup completes", async (t) => {
  const { client, states, install } = setup(t);
  let completeLeave!: () => void;
  install("fetch", (url: string) => {
    if (url.endsWith("/leave")) return new Promise<Response>((resolve) => { completeLeave = () => resolve(new Response(null, { status: 204 })); });
    if (url.endsWith("/join")) return Promise.resolve(Response.json({ token: "capability", id: "self", iceServers: [] }));
    if (url.endsWith("/publish")) return Promise.resolve(Response.json({ sessionDescription: { type: "answer", sdp: "v=0" } }));
    if (url.endsWith("/state")) return Promise.resolve(new Response(null, { status: 204 }));
    return Promise.resolve(Response.json({ participants: [] }));
  });
  await client.join("Guest");
  const leaving = client.leave();
  assert.equal(states.at(-1)?.phase, "idle");
  completeLeave();
  await leaving;
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
