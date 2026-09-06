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
  connectionState = "connected";
  iceGatheringState = "complete";
  localDescription?: { toJSON(): object };
  senders: Array<{ track: Track | null; replaceTrack(t: Track | null): Promise<void> }> = [];
  constructor() { super(); Peer.latest = this; }
  addTransceiver(track: Track) {
    const sender = { track: track as Track | null, async replaceTrack(t: Track | null) { this.track = t; } };
    this.senders.push(sender);
    return { mid: "0", sender };
  }
  async createOffer() { return { type: "offer", sdp: "v=0" }; }
  async setLocalDescription(description: object) { this.localDescription = { toJSON: () => description }; }
  async setRemoteDescription() {}
  getSenders() { return this.senders; }
  getReceivers() { return []; }
  close() { this.connectionState = "closed"; }
}

function setup(t: TestContext) {
  const track = new Track();
  const calls: string[] = [];
  const joinedNames: string[] = [];
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
      return Response.json({ token: "capability", id: "self", iceServers: [] });
    }
    if (op === "publish") return Response.json({ sessionDescription: { type: "answer", sdp: "v=0" } });
    if (op === "snapshot") return Response.json({ participants: [] });
    return new Response(null, { status: 204 });
  });
  const client = new PublicCallClient((state) => states.push(state));
  t.after(() => { client.leaveImmediately(); restore.reverse().forEach((fn) => fn()); });
  return { client, track, calls, joinedNames, states, install };
}

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
