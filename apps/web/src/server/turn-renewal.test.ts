import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { TurnRenewal } from "../media/turn-renewal.ts";
import type { TurnGeneration, TurnResponse } from "../media/types.ts";

type Request = <T>(operation: string, body: object, token: string, signal: AbortSignal) => Promise<T>;

const initial: TurnGeneration = { generation: "old", refreshAfterMs: 1_000, expiresInMs: 60_000 };
const renewed: TurnResponse = {
  iceServers: [{ urls: "turn:new.example.test", username: "new-user", credential: "new-pass" }],
  turn: { generation: "new", refreshAfterMs: 60_000, expiresInMs: 120_000 },
};
const offer = { type: "offer" as const, sdp: "v=0\r\na=ice-ufrag:one\r\n" };
const answer = { type: "answer" as const, sdp: "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\na=rtpmap:111 opus/48000/2\r\n" };

function deferred<T>() {
  let resolve!: (value: T | PromiseLike<T>) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function flush() {
  for (let i = 0; i < 8; i++) await Promise.resolve();
}

class Peer {
  configuration: RTCConfiguration = { bundlePolicy: "max-bundle", iceServers: [{ urls: "turn:old.example.test" }] };
  localDescription: { toJSON(): RTCSessionDescriptionInit } | null = null;
  offers: RTCOfferOptions[] = [];
  locals: RTCSessionDescriptionInit[] = [];
  remotes: RTCSessionDescriptionInit[] = [];
  remoteHook?: (description: RTCSessionDescriptionInit) => Promise<void>;
  getConfiguration() { return this.configuration; }
  setConfiguration(value: RTCConfiguration) { this.configuration = value; }
  async createOffer(options: RTCOfferOptions) { this.offers.push(options); return offer; }
  async setLocalDescription(value: RTCSessionDescriptionInit) {
    this.locals.push(value);
    this.localDescription = { toJSON: () => value };
  }
  async setRemoteDescription(value: RTCSessionDescriptionInit) {
    this.remotes.push(value);
    await this.remoteHook?.(value);
  }
}

function setup(t: TestContext) {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const oldWindow = Object.getOwnPropertyDescriptor(globalThis, "window");
  Object.defineProperty(globalThis, "window", { value: globalThis, configurable: true });
  t.after(() => {
    if (oldWindow) Object.defineProperty(globalThis, "window", oldWindow);
    else Reflect.deleteProperty(globalThis, "window");
  });
  const start = async () => { t.mock.timers.tick(1_000); await flush(); };
  return { start };
}

function make(pc: Peer, request: Request, serialize: <T>(operation: () => Promise<T>) => Promise<T>, invalid: () => void = () => {}) {
  return new TurnRenewal(pc as unknown as RTCPeerConnection, "token", initial, request, serialize,
    () => true, invalid, performance.now());
}

test("replays the exact restart offer after a lost answer or 503", async (t) => {
  const { start } = setup(t);
  const pc = new Peer();
  const restartBodies: object[] = [];
  let attempts = 0;
  const request: Request = async <T>(operation: string, body: object) => {
    if (operation === "turn") return renewed as T;
    if (operation === "restart-ice") {
      restartBodies.push(body);
      if (attempts++ === 0) throw Object.assign(new Error("answer was lost"), { status: 503 });
      return { sessionDescription: answer } as T;
    }
    return undefined as T;
  };
  const renewal = make(pc, request, (operation) => operation());
  await start();
  assert.equal(restartBodies.length, 1);
  assert.equal(pc.offers.length, 1);
  t.mock.timers.tick(1_000);
  await flush();
  assert.equal(restartBodies.length, 2);
  assert.strictEqual(restartBodies[1], restartBodies[0], "retry must replay the same body object");
  assert.deepEqual(restartBodies[1], { generation: "new", sequence: 1, sessionDescription: offer });
  assert.deepEqual(pc.offers, [{ iceRestart: true }]);
  assert.deepEqual(pc.locals, [offer]);
  assert.equal(pc.remotes.length, 1);
  renewal.stop();
});

test("holds the signaling queue through ACK and retries ACK without renegotiating", async (t) => {
  const { start } = setup(t);
  const pc = new Peer();
  const ack = deferred<void>();
  const operations: Array<{ operation: string; body: object }> = [];
  let ackAttempts = 0;
  const request: Request = async <T>(operation: string, body: object) => {
    operations.push({ operation, body });
    if (operation === "turn") return renewed as T;
    if (operation === "restart-ice") return { sessionDescription: answer } as T;
    if (ackAttempts++ === 0) throw Object.assign(new Error("busy"), { status: 503 });
    return ack.promise as T;
  };
  const admitted = deferred<void>();
  let released = false;
  const serialize = async <T>(operation: () => Promise<T>) => {
    await admitted.promise;
    try { return await operation(); } finally { released = true; }
  };
  const renewal = make(pc, request, serialize);
  await start();
  assert.equal(pc.offers.length, 0, "offer creation must wait for queue admission");
  admitted.resolve();
  await flush();
  assert.equal(pc.offers.length, 1);
  assert.equal(pc.remotes.length, 1);
  assert.equal(released, false);
  t.mock.timers.tick(1_000);
  await flush();
  assert.equal(operations.filter(({ operation }) => operation === "restart-ice-ack").length, 2);
  assert.equal(operations.filter(({ operation }) => operation === "restart-ice").length, 1);
  assert.equal(pc.offers.length, 1);
  assert.equal(pc.remotes.length, 1, "ACK retry must not reapply the answer");
  assert.equal(released, false, "queue must remain occupied until ACK completes");
  ack.resolve();
  await flush();
  assert.equal(released, true);
  renewal.stop();
});

test("same-generation credentials are a cache hit without an ICE restart", async (t) => {
  const { start } = setup(t);
  const pc = new Peer();
  const calls: string[] = [];
  let serialized = 0;
  const renewal = make(pc, async <T>(operation: string, body: object) => {
    calls.push(operation);
    assert.deepEqual(body, { generation: "old" });
    return { ...renewed, turn: { ...renewed.turn, generation: "old" } } as T;
  }, async (operation) => { serialized++; return operation(); });
  await start();
  assert.deepEqual(calls, ["turn"]);
  assert.equal(serialized, 0);
  assert.equal(pc.offers.length, 0);
  assert.deepEqual(pc.configuration.iceServers, [{ urls: "turn:old.example.test" }]);
  renewal.stop();
});

test("cancellation while queued and while applying the answer has no stale failure callback", async (t) => {
  const { start } = setup(t);
  const queuedPc = new Peer();
  const gate = deferred<void>();
  let invalids = 0;
  const request: Request = async <T>(operation: string) => operation === "turn" ? renewed as T : { sessionDescription: answer } as T;
  const queued = make(queuedPc, request, async (operation) => { await gate.promise; return operation(); }, () => invalids++);
  await start();
  queued.stop();
  gate.resolve();
  await flush();
  assert.equal(queuedPc.offers.length, 0);
  assert.equal(invalids, 0);

  const applyingPc = new Peer();
  const remote = deferred<void>();
  applyingPc.remoteHook = () => remote.promise;
  const applying = make(applyingPc, request, (operation) => operation(), () => invalids++);
  // A fresh helper has its own initial timer.
  t.mock.timers.tick(1_000);
  await flush();
  assert.equal(applyingPc.remotes.length, 1);
  applying.stop();
  remote.resolve();
  await flush();
  assert.equal(invalids, 0);
});

for (const scenario of ["mismatch", "invalid answer"] as const) {
  test(`permanent ${scenario} invokes owner recovery once without retrying`, async (t) => {
    const { start } = setup(t);
    const pc = new Peer();
    let invalids = 0;
    let restarts = 0;
    const renewal = make(pc, async <T>(operation: string) => {
      if (operation === "turn") return renewed as T;
      if (operation === "restart-ice") {
        restarts++;
        if (scenario === "mismatch") throw Object.assign(new Error("generation mismatch"), { status: 409, code: "ice_restart_mismatch" });
        return {} as T;
      }
      throw new Error("unexpected ACK");
    }, (operation) => operation(), () => invalids++);
    await start();
    assert.equal(invalids, 1);
    assert.equal(restarts, 1);
    t.mock.timers.tick(120_000);
    await flush();
    assert.equal(invalids, 1);
    assert.equal(restarts, 1);
    assert.equal(pc.offers.length, 1);
    renewal.stop();
  });
}

test("independent monitor sender and receiver renewal queues do not block each other", async (t) => {
  const { start } = setup(t);
  const sender = new Peer();
  const receiver = new Peer();
  const senderGate = deferred<void>();
  const calls: string[] = [];
  const request = (role: string): Request => async <T>(operation: string) => {
    calls.push(`${role}:${operation}`);
    if (operation === "turn") return renewed as T;
    if (operation === "restart-ice") return { sessionDescription: answer } as T;
    return undefined as T;
  };
  const senderRenewal = make(sender, request("sender"), async (operation) => { await senderGate.promise; return operation(); });
  const receiverRenewal = make(receiver, request("receiver"), (operation) => operation());
  await start();
  assert.equal(sender.offers.length, 0);
  assert.equal(receiver.offers.length, 1);
  assert.deepEqual(calls.filter((call) => call.startsWith("receiver:")),
    ["receiver:turn", "receiver:restart-ice", "receiver:restart-ice-ack"]);
  senderGate.resolve();
  await flush();
  assert.equal(sender.offers.length, 1);
  senderRenewal.stop();
  receiverRenewal.stop();
});
