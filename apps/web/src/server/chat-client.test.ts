import assert from "node:assert/strict";
import { test, type TestContext, vi } from "vitest";
import {
  ChatClient,
  initialChatView,
  loadChatHistory,
  resetChatSessionsForTests,
  type ChatViewState,
} from "../chat/client.ts";
import { AppGateway, setAppGatewayForTests } from "../gateway/client.ts";
import type { ChatEvent, ChatMessage, ChatTypingEvent, GeneralChatHistory } from "../chat/types.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

class TestSocket extends EventTarget {
  url: string;
  closed = false;
  sent: Array<Record<string, unknown>> = [];
  commands: Array<{
    active: boolean;
    resolve: (response: Response) => void;
    reject: (error: Error) => void;
  }> = [];
  constructor(url: string) {
    super();
    this.url = url;
    queueMicrotask(() => this.raw({ type: "hello", idleTimeoutSeconds: 600, serverTime: Date.now() }));
  }
  send(data: string) {
    const frame = JSON.parse(data) as Record<string, unknown>;
    this.sent.push(frame);
    if (frame.type !== "command") return;
    assert.equal(frame.method, "typing");
    assert.equal(frame.channelId, "general");
    assert.equal(frame.chatToken, "opaque");
    assert.deepEqual(
      Object.keys(frame.body as object),
      ["typing"],
      "draft text and claimed identity never enter typing commands",
    );
    const id = frame.id as string;
    this.commands.push({
      active: (frame.body as { typing?: boolean }).typing === true,
      resolve: (response) =>
        this.raw({
          type: "result",
          id,
          status: response.status,
          body: response.status < 300 ? {} : { error: response.statusText || "Typing failed." },
        }),
      reject: (error) => this.raw({ type: "result", id, status: 500, body: { error: error.message } }),
    });
  }
  private raw(value: unknown) {
    this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(value) }));
  }
  subscriptions() {
    return this.sent.filter((frame) => frame.type === "subscribe" && frame.kind === "chat");
  }
  frame(event: ChatEvent | { type: "migrating" }, subscriptionId?: string) {
    if (event.type === "migrating") {
      this.raw(event);
      return;
    }
    const id = subscriptionId ?? this.subscriptions().at(-1)?.id;
    assert.equal(typeof id, "string", "chat event requires an active gateway subscription");
    this.raw({ type: "event", id, event });
    if (event.type === "ready") this.raw({ type: "subscribed", id });
  }
  message(message: ChatMessage, subscriptionId?: string) {
    this.frame({ type: "message.created", channelId: message.channelId, seq: message.seq, message }, subscriptionId);
  }
  close() {
    this.closed = true;
  }
}

function installBrowser(t: TestContext) {
  resetChatSessionsForTests();
  t.onTestFinished(resetChatSessionsForTests);
  const originals = {
    window: globalThis.window,
    localStorage: globalThis.localStorage,
    WebSocket: globalThis.WebSocket,
  };
  const values = new Map<string, string>();
  const sockets: TestSocket[] = [];
  Object.defineProperties(globalThis, {
    window: {
      configurable: true,
      value: Object.assign(new EventTarget(), { location: { protocol: "https:", host: "caper.test" } }),
    },
    localStorage: {
      configurable: true,
      value: {
        getItem: (key: string) => values.get(key) ?? null,
        setItem: (key: string, value: string) => values.set(key, value),
        removeItem: (key: string) => values.delete(key),
      },
    },
    WebSocket: {
      configurable: true,
      value: class extends TestSocket {
        constructor(url: string) {
          super(url);
          sockets.push(this);
        }
      },
    },
  });
  const gateway = new AppGateway(
    (url) => {
      const socket = new TestSocket(url);
      sockets.push(socket);
      return socket;
    },
    () => 0,
  );
  setAppGatewayForTests(gateway);
  t.onTestFinished(() => {
    gateway.destroy();
    setAppGatewayForTests(undefined);
    Object.defineProperties(globalThis, {
      window: { configurable: true, value: originals.window },
      localStorage: { configurable: true, value: originals.localStorage },
      WebSocket: { configurable: true, value: originals.WebSocket },
    });
  });
  return sockets;
}

function chatSubscription(socket: TestSocket, offset = -1) {
  const frame = socket.subscriptions().at(offset);
  assert.ok(frame, "expected a chat subscription frame");
  return frame;
}

test("pressing Send again after an unknown outcome preserves the original UUID and text", async (t) => {
  installBrowser(t);
  vi.spyOn(globalThis.crypto, "randomUUID").mockImplementation(() => "00000000-0000-4000-8000-000000000001");
  const sentBodies: string[] = [];
  let sendAttempts = 0;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/chat/session")
      return Response.json({ token: "opaque-token", author: { id: "guest", name: "Test Guest", isGuest: true } });
    if (url === "/api/chat/general")
      return Response.json({
        space: { id: "space", name: "Caper" },
        channel: { id: "general", name: "General" },
        messages: [],
        cursor: "0",
        hasMore: false,
      });
    assert.equal(url, "/api/chat/channels/general/messages");
    assert.equal(new Headers(init?.headers).get("x-caper-chat-token"), "opaque-token");
    sentBodies.push(String(init?.body));
    if (sendAttempts++ === 0) return Response.json({ error: "temporary" }, { status: 503 });
    const body = JSON.parse(String(init?.body)) as { clientMessageId: string; text: string };
    const message: ChatMessage = {
      id: "message-1",
      channelId: "general",
      seq: "1",
      author: { id: "guest", name: "Test Guest", isGuest: true },
      content: { version: 1, type: "text", text: body.text },
      createdAt: "2026-09-21T12:00:00Z",
      clientMessageId: body.clientMessageId,
    };
    return Response.json(message);
  });

  let state!: ChatViewState;
  const client = new ChatClient((next) => {
    state = next;
  });
  t.onTestFinished(() => client.stop());
  client.start();
  client.identify("Test Guest");
  await tick();
  await tick();

  assert.equal(await client.send("original text"), false);
  assert.ok(state.pendingSend);
  assert.equal(await client.send("edited draft"), true);
  assert.deepEqual(
    sentBodies.map((body) => JSON.parse(body)),
    [
      { clientMessageId: "00000000-0000-4000-8000-000000000001", text: "original text" },
      { clientMessageId: "00000000-0000-4000-8000-000000000001", text: "original text" },
    ],
  );
  assert.equal(state.messages.length, 1);
});

test("signed-in startup does not reuse another account's capability with the same display name", async (t) => {
  installBrowser(t);
  localStorage.setItem(
    "caper.chat.session",
    JSON.stringify({
      token: "old-account-token",
      author: { id: "old-account", name: "Shared Name", isGuest: false },
    }),
  );
  let sessions = 0;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    if (String(input) === "/api/chat/session") {
      sessions++;
      return Response.json({
        token: "current-account-token",
        author: { id: "current-account", name: "Shared Name", isGuest: false },
      });
    }
    assert.equal(String(input), "/api/chat/general");
    return Response.json({
      space: { id: "space", name: "Caper" },
      channel: { id: "general", name: "General" },
      messages: [],
      cursor: "0",
      hasMore: false,
    });
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => {
    state = next;
  });
  t.onTestFinished(() => client.stop());
  client.start();
  client.identify("Shared Name", true);
  await tick();
  await tick();
  assert.equal(sessions, 1);
  assert.equal(state.author?.id, "current-account");
  assert.equal(JSON.parse(localStorage.getItem("caper.chat.session")!).token, "current-account-token");
});

test("public history loads before identity and remains visible while the send session resolves", async (t) => {
  const sockets = installBrowser(t);
  const message: ChatMessage = {
    id: "existing",
    channelId: "general",
    seq: "7",
    author: { id: "other", name: "Other Guest", isGuest: true },
    content: { version: 1, type: "text", text: "Already here" },
    createdAt: "2026-09-21T12:00:00Z",
    clientMessageId: "old-command",
  };
  const requests: string[] = [];
  let finishSession!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    const path = String(input);
    requests.push(path);
    if (path === "/api/chat/session")
      return new Promise<Response>((resolve) => {
        finishSession = resolve;
      });
    assert.equal(path, "/api/chat/general");
    return Response.json({
      space: { id: "space", name: "Caper" },
      channel: { id: "general", name: "General" },
      messages: [message],
      cursor: "7",
      hasMore: false,
    });
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => {
    state = next;
  });
  t.onTestFinished(() => client.stop());
  client.start();
  await tick();
  await tick();
  assert.deepEqual(requests, ["/api/chat/general"]);
  assert.equal(state.phase, "ready");
  assert.deepEqual(state.messages, [message]);
  assert.equal(state.author, undefined);

  client.identify("New Guest");
  assert.equal(state.phase, "ready");
  assert.deepEqual(state.messages, [message]);
  sockets[0].frame({ type: "ready", cursor: "7" });
  sockets[0].frame(typingEvent("new"));
  assert.equal(state.typingAuthors.length, 1, "identity is not known until the session resolves");
  finishSession(Response.json({ token: "new-capability", author: { id: "new", name: "New Guest", isGuest: true } }));
  await tick();
  await tick();
  assert.deepEqual(state.author, { id: "new", name: "New Guest", isGuest: true });
  assert.equal(state.typingAuthors.length, 0, "late identity removes own typing from another tab immediately");
  assert.deepEqual(state.messages, [message]);
  assert.deepEqual(requests, ["/api/chat/general", "/api/chat/session"]);
});

interface SendBody {
  clientMessageId: string;
  text: string;
}

function committed(body: SendBody, seq: string): ChatMessage {
  return {
    id: `message-${seq}`,
    channelId: "general",
    seq,
    author: { id: "guest", name: "Test Guest", isGuest: true },
    content: { version: 1, type: "text", text: body.text },
    createdAt: "2026-09-21T12:00:00Z",
    clientMessageId: body.clientMessageId,
  };
}

async function sendingFixture(t: TestContext) {
  const sockets = installBrowser(t);
  const history: ChatMessage[] = [];
  const posts: {
    body: SendBody;
    signal: AbortSignal;
    resolve: (response: Response) => void;
    reject: (error: Error) => void;
  }[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    if (String(input) === "/api/chat/session")
      return Response.json({ token: "opaque", author: { id: "guest", name: "Test Guest", isGuest: true } });
    if (String(input) === "/api/chat/general")
      return Response.json({
        space: { id: "space", name: "Caper" },
        channel: { id: "general", name: "General" },
        messages: history,
        cursor: history.at(-1)?.seq ?? "0",
        hasMore: false,
      });
    assert.equal(String(input), "/api/chat/channels/general/messages");
    const body = JSON.parse(String(init?.body)) as SendBody;
    assert.deepEqual(
      Object.keys(body).sort(),
      ["clientMessageId", "text"],
      "local metadata never enters the wire contract",
    );
    assert.ok(init?.signal);
    const signal = init.signal;
    return new Promise<Response>((resolve, reject) => posts.push({ body, signal, resolve, reject }));
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => {
    state = next;
  });
  t.onTestFinished(() => client.stop());
  client.start();
  client.identify("Test Guest");
  await tick();
  sockets[0].frame({ type: "ready", cursor: "0" });
  return {
    client,
    sockets,
    posts,
    history,
    get typingPosts() {
      return sockets.flatMap((socket) => socket.commands);
    },
    get state() {
      return state;
    },
  };
}

test("live updates reuse unaffected message objects without mutating previous snapshots", async (t) => {
  const f = await sendingFixture(t);
  f.sockets[0].message(committed({ clientMessageId: "first", text: "First" }, "1"));
  const first = f.state.messages[0];
  f.sockets[0].message(committed({ clientMessageId: "second", text: "Second" }, "2"));
  assert.strictEqual(f.state.messages[0], first, "inserting a message must not clone every previous row");
  const second = f.state.messages[1];
  f.sockets[0].frame({
    type: "message.reactions",
    schemaVersion: 1,
    channelId: "general",
    seq: "3",
    messageId: first.id,
    reactions: [{ emoji: "👍", authorIds: ["other"] }],
  });
  assert.strictEqual(f.state.messages[1], second);
  assert.notStrictEqual(f.state.messages[0], first);
  assert.equal(first.reactions, undefined, "previous states stay immutable");
  assert.deepEqual(f.state.messages[0].reactions, [{ emoji: "👍", authorIds: ["other"] }]);
});

test("pending edits and pins only copy affected rows, including when a newer edit supersedes the draft", async (t) => {
  const f = await sendingFixture(t);
  for (const seq of ["1", "2", "3"])
    f.sockets[0].message(committed({ clientMessageId: seq, text: `Message ${seq}` }, seq));
  const [original, untouched, pinTarget] = f.state.messages;
  const responses: Array<(response: Response) => void> = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(() => new Promise<Response>((resolve) => responses.push(resolve)));
  const editing = f.client.editMessage(original.id, "Draft", 1);
  assert.strictEqual(f.state.messages[1], untouched);
  assert.strictEqual(f.state.messages[2], pinTarget);
  assert.equal(f.state.messages[0].content.text, "Draft");
  assert.equal(original.content.text, "Message 1");
  const pinning = f.client.setPin(pinTarget.id, true);
  assert.strictEqual(f.state.messages[1], untouched);
  assert.notStrictEqual(f.state.messages[2], pinTarget);
  assert.equal(f.state.pinnedMessages[0].id, pinTarget.id);
  assert.equal(pinTarget.pin, undefined);
  const remote = {
    ...original,
    revision: 2,
    editSeq: "4",
    editedAt: original.createdAt,
    content: { ...original.content, text: "Remote edit" },
  };
  f.sockets[0].frame({ type: "message.edited", schemaVersion: 1, channelId: "general", seq: "4", message: remote });
  assert.equal(f.state.messages[0].content.text, "Remote edit");
  assert.strictEqual(
    f.state.messages[0],
    f.client.snapshotHistory()?.messages[0],
    "a superseded draft needs no projection copy",
  );
  responses[0](Response.json({ error: "message changed" }, { status: 409 }));
  await assert.rejects(editing, /message changed/);
  responses[1](Response.json({ error: "try again" }, { status: 503 }));
  await assert.rejects(pinning, /try again/);
  assert.strictEqual(f.state.messages[1], untouched);
  assert.strictEqual(f.state.messages[2], pinTarget);
  assert.equal(f.state.pinnedMessages.length, 0);
});

test("pin PUTs share live state without advancing HTTP replay or restoring a later unpin", async (t) => {
  const f = await sendingFixture(t);
  const original = committed({ clientMessageId: "pin-target", text: "Shared pin" }, "1");
  f.sockets[0].message(original);
  const pinned = { ...original, pinSeq: "3", pin: { author: original.author, createdAt: original.createdAt } };
  const event = {
    type: "message.pin" as const,
    schemaVersion: 1 as const,
    channelId: "general",
    seq: "3",
    message: pinned,
  };
  let finish!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    assert.equal(String(input), "/api/chat/channels/general/messages/message-1/pin");
    assert.equal(init?.method, "PUT");
    assert.equal(new Headers(init?.headers).get("x-caper-chat-token"), "opaque");
    assert.deepEqual(JSON.parse(String(init?.body)), { active: true });
    return new Promise<Response>((resolve) => {
      finish = resolve;
    });
  });
  const saving = f.client.setPin(original.id, true);
  assert.equal(f.state.messages[0].pin?.author.id, "guest", "pin appears before the request resolves");
  assert.equal(f.state.pinnedMessages[0].id, original.id);
  assert.equal(f.client.snapshotHistory()?.messages[0].pin, undefined, "history never caches a speculative pin");
  assert.equal(f.client.snapshotHistory()?.cursor, "1");
  f.sockets[0].message(committed({ clientMessageId: "between", text: "Between pin updates" }, "2"));
  f.sockets[0].frame(event);
  assert.equal(f.state.pinnedMessages[0].id, original.id);
  f.sockets[0].frame({ ...event, seq: "4", message: { ...pinned, pin: null, pinSeq: "4" } });
  finish(Response.json(event));
  await saving;
  assert.equal(f.client.snapshotHistory()?.cursor, "4");
  assert.equal(f.state.pinnedMessages.length, 0);
  assert.equal(f.state.messages[0].pin, null, "a late pin HTTP acknowledgement cannot revert a later shared unpin");
  vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
    Response.json({ error: "Pins temporarily unavailable" }, { status: 503 }),
  );
  await assert.rejects(f.client.setPin(original.id, true), /Pins temporarily unavailable/);
  assert.equal(f.state.pinnedMessages.length, 0);
});

test("optimistic unpin rolls back to the latest shared pin and content, not the pre-click row", async (t) => {
  const f = await sendingFixture(t);
  const original = committed({ clientMessageId: "pin-target", text: "Before" }, "1");
  const pin = { author: original.author, createdAt: original.createdAt };
  f.sockets[0].message({ ...original, pin, pinSeq: "1" });
  let finish!: (response: Response) => void;
  let requests = 0;
  vi.spyOn(globalThis, "fetch").mockImplementation(() => {
    requests++;
    return new Promise<Response>((resolve) => {
      finish = resolve;
    });
  });
  const saving = f.client.setPin(original.id, false);
  assert.equal(f.state.messages[0].pin, null);
  assert.equal(f.state.pinnedMessages.length, 0);
  assert.equal(f.client.snapshotHistory()?.pinnedMessages?.[0].id, original.id);
  await f.client.setPin(original.id, true);
  assert.equal(requests, 1, "a duplicate pin cannot race the pending write");
  f.sockets[0].frame({
    type: "message.reactions",
    schemaVersion: 1,
    channelId: "general",
    seq: "2",
    messageId: original.id,
    reactions: [{ emoji: "🚀", authorIds: ["peer"] }],
  });
  const corrected = {
    ...original,
    revision: 2,
    editSeq: "3",
    editedAt: original.createdAt,
    content: { ...original.content, text: "Remote correction" },
  };
  f.sockets[0].frame({ type: "message.edited", schemaVersion: 1, channelId: "general", seq: "3", message: corrected });
  const remotePin = { author: { id: "peer", name: "Peer", isGuest: false }, createdAt: "2026-10-08T12:00:00Z" };
  f.sockets[0].frame({
    type: "message.pin",
    schemaVersion: 1,
    channelId: "general",
    seq: "4",
    message: { ...corrected, pin: remotePin, pinSeq: "4" },
  });
  assert.equal(f.state.pinnedMessages.length, 0, "local intent survives shared updates until confirmation");
  finish(Response.json({ error: "Try again" }, { status: 503 }));
  await assert.rejects(saving, /Try again/);
  assert.deepEqual(f.state.messages[0].pin, remotePin);
  assert.equal(f.state.messages[0].content.text, "Remote correction");
  assert.deepEqual(f.state.messages[0].reactions, [{ emoji: "🚀", authorIds: ["peer"] }]);
  assert.equal(f.state.pinnedMessages[0].content.text, "Remote correction");
  assert.equal(f.client.snapshotHistory()?.cursor, "4");
});

test("a failed refresh clears abandoned pin and edit overlays and ignores their late acknowledgements", async (t) => {
  const f = await sendingFixture(t);
  const original = committed({ clientMessageId: "mutation-target", text: "Confirmed" }, "1");
  f.sockets[0].message(original);
  const responses: ((response: Response) => void)[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation((_input, init) =>
    init?.method === "PUT"
      ? new Promise<Response>((resolve) => responses.push(resolve))
      : Promise.resolve(Response.json({ error: "Temporary outage" }, { status: 503 })),
  );
  const pinning = f.client.setPin(original.id, true);
  const editing = f.client.editMessage(original.id, "Unconfirmed", 1);
  assert.equal(f.state.pinnedMessages[0].content.text, "Unconfirmed");
  f.client.retryLoad();
  await tick();
  assert.equal(f.state.phase, "ready");
  assert.equal(f.state.messages[0].content.text, "Confirmed");
  assert.equal(f.state.messages[0].pin, undefined);
  assert.equal(f.state.pinnedMessages.length, 0);
  const refreshed = f.state;
  responses[0](
    Response.json({
      type: "message.pin",
      schemaVersion: 1,
      channelId: "general",
      seq: "2",
      message: { ...original, pinSeq: "2", pin: { author: original.author, createdAt: original.createdAt } },
    }),
  );
  responses[1](
    Response.json({
      ...original,
      revision: 2,
      editSeq: "3",
      editedAt: original.createdAt,
      content: { ...original.content, text: "Unconfirmed" },
    }),
  );
  await pinning;
  await assert.rejects(editing, /conversation changed/);
  assert.equal(f.state, refreshed, "abandoned acknowledgements cannot restore a local overlay");
  assert.equal(f.client.snapshotHistory()?.cursor, "1");
});

test("HTTP-only pin acknowledgement updates the collection but not its durable cursor", async (t) => {
  const f = await sendingFixture(t);
  const original = committed({ clientMessageId: "pin-target", text: "Old pin outside the page" }, "1");
  const event = {
    type: "message.pin",
    schemaVersion: 1,
    channelId: "general",
    seq: "2",
    message: { ...original, pinSeq: "2", pin: { author: original.author, createdAt: original.createdAt } },
  };
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => Response.json(event));
  await f.client.setPin(original.id, true);
  assert.equal(f.client.snapshotHistory()?.cursor, "0");
  assert.equal(f.state.messages.length, 0, "pins stay independent of pagination");
  assert.equal(f.state.pinnedMessages[0].id, original.id);
});

for (const lifecycle of ["stopped", "denied"] as const) {
  test(`a late send acknowledgement cannot restore a ${lifecycle} chat`, async (t) => {
    const f = await sendingFixture(t);
    const sending = f.client.send("In flight before departure");
    if (lifecycle === "stopped") f.client.stop();
    else {
      vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
        Response.json({ error: "Access removed" }, { status: 403 }),
      );
      f.client.retryLoad();
      await tick();
      assert.equal(f.state.phase, "error");
      assert.equal(f.state.channelId, undefined);
    }
    const departed = f.state;
    f.posts[0].resolve(Response.json(committed(f.posts[0].body, "1")));
    assert.equal(await sending, false, "an obsolete acknowledgement must not update the departed view");
    assert.equal(f.state, departed);
    assert.deepEqual(f.state.messages, []);
  });

  test(`a late send rejection cannot reset the session of a ${lifecycle} chat`, async (t) => {
    const f = await sendingFixture(t);
    const sending = f.client.send("In flight before departure");
    const fetch = vi
      .spyOn(globalThis, "fetch")
      .mockImplementation(async () => Response.json({ error: "Access removed" }, { status: 403 }));
    if (lifecycle === "stopped") f.client.stop();
    else {
      f.client.retryLoad();
      await tick();
    }
    const departed = f.state;
    const session = localStorage.getItem("caper.chat.session");
    const requests = fetch.mock.calls.length;
    f.posts[0].resolve(Response.json({ error: "Obsolete rejection" }, { status: 401 }));
    assert.equal(await sending, false);
    await tick();
    assert.equal(f.state, departed);
    assert.equal(localStorage.getItem("caper.chat.session"), session);
    assert.equal(fetch.mock.calls.length, requests, "obsolete sends cannot mint new sessions");
  });
}

for (const status of [200, 503]) {
  test(`a send acknowledgement remains valid after a same-channel refresh returns ${status}`, async (t) => {
    const f = await sendingFixture(t);
    const sending = f.client.send("Still in this channel");
    const snapshot = f.client.snapshotHistory();
    vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
      status === 200 ? Response.json(snapshot) : Response.json({ error: "Temporary outage" }, { status }),
    );
    f.client.retryLoad();
    await tick();
    const accepted = committed(f.posts[0].body, "1");
    f.posts[0].resolve(Response.json(accepted));
    assert.equal(await sending, true);
    assert.deepEqual(f.state.messages, [accepted]);
    assert.equal(f.state.pendingSend, undefined);
    assert.equal(f.client.snapshotHistory()?.cursor, "0", "HTTP still cannot advance the replay cursor");
  });
}

test("stopped clients cannot send or retry history", async (t) => {
  const f = await sendingFixture(t);
  f.client.stop();
  const departed = f.state;
  const fetch = vi
    .spyOn(globalThis, "fetch")
    .mockClear()
    .mockImplementation(async () => {
      throw new Error("Unexpected request after stop");
    });
  assert.equal(await f.client.send("Too late"), false);
  f.client.retryLoad();
  await tick();
  assert.equal(fetch.mock.calls.length, 0);
  assert.equal(f.state, departed);
});

test("reaction HTTP snapshots and sequenced delivery agree without skipping messages", async (t) => {
  const f = await sendingFixture(t);
  const target = committed({ clientMessageId: "target", text: "React here" }, "1");
  f.sockets[0].message(target);
  let finish!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation((input: string | URL | Request, init?: RequestInit) => {
    assert.equal(String(input), "/api/chat/channels/general/messages/message-1/reactions");
    assert.equal(init?.method, "PUT");
    assert.equal(new Headers(init?.headers).get("x-caper-chat-token"), "opaque");
    assert.deepEqual(JSON.parse(String(init?.body)), { emoji: "👍", active: true });
    return new Promise<Response>((resolve) => {
      finish = resolve;
    });
  });
  const adding = f.client.setReaction(target.id, "👍", true);
  const added = {
    type: "message.reactions" as const,
    schemaVersion: 1 as const,
    channelId: "general",
    messageId: target.id,
    seq: "2",
    reactions: [{ emoji: "👍", authorIds: ["guest"] }],
  };
  assert.deepEqual(f.state.messages[0].reactions, added.reactions, "reaction appears before any server response");
  assert.equal(f.client.snapshotHistory()?.messages[0].reactions, undefined, "cached history stays authoritative");
  f.sockets[0].frame(added);
  f.sockets[0].frame({ ...added, seq: "3", reactions: [] });
  assert.deepEqual(
    f.state.messages[0].reactions,
    added.reactions,
    "pending own intent stays projected over live snapshots",
  );
  finish(Response.json(added));
  await adding;
  assert.deepEqual(f.state.messages[0].reactions, [], "late HTTP cannot undo a newer removal");
  assert.equal(f.client.snapshotHistory()?.cursor, "3");
  f.sockets[0].message(committed({ clientMessageId: "next", text: "After the reactions" }, "4"));
  assert.equal(f.state.messages.length, 2);
  assert.equal(f.client.snapshotHistory()?.cursor, "4");
  const failed = f.client.setReaction(target.id, "👍", true);
  finish(Response.json({ error: "Please try again" }, { status: 503 }));
  await assert.rejects(failed, /Please try again/);
  assert.deepEqual(f.state.messages[0].reactions, []);
  const retry = f.client.setReaction(target.id, "👍", true);
  finish(Response.json({ ...added, seq: "5" }));
  await retry;
  assert.deepEqual(f.state.messages[0].reactions, added.reactions);
  assert.equal(f.client.snapshotHistory()?.cursor, "4", "HTTP acknowledgement alone does not advance replay");
});

test("rapid reaction intents stay visible, serialize writes and roll back only the failed membership", async (t) => {
  const f = await sendingFixture(t);
  const target = {
    ...committed({ clientMessageId: "target", text: "React here" }, "1"),
    reactionSeq: "1",
    reactions: [{ emoji: "👍", authorIds: ["other"] }],
  };
  f.sockets[0].message(target);
  const requests: { body: { emoji: string; active: boolean }; finish: (response: Response) => void }[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(
    (_input: unknown, init?: RequestInit) =>
      new Promise<Response>((finish) => {
        requests.push({ body: JSON.parse(String(init?.body)), finish });
      }),
  );
  const add = f.client.setReaction(target.id, "👍", true);
  assert.deepEqual(f.state.messages[0].reactions, [{ emoji: "👍", authorIds: ["other", "guest"] }]);
  const remove = f.client.setReaction(target.id, "👍", false);
  const celebrate = f.client.setReaction(target.id, "🎉", true);
  assert.equal(requests.length, 1, "one in-flight request per message");
  assert.deepEqual(f.state.messages[0].reactions, [
    { emoji: "👍", authorIds: ["other"] },
    { emoji: "🎉", authorIds: ["guest"] },
  ]);
  requests[0].finish(Response.json({ error: "superseded add failed" }, { status: 503 }));
  await add;
  await tick();
  assert.deepEqual(requests[1].body, { emoji: "👍", active: false });
  const update = {
    type: "message.reactions" as const,
    schemaVersion: 1 as const,
    channelId: "general",
    messageId: target.id,
    seq: "2",
    reactions: [{ emoji: "👍", authorIds: ["other", "new-person"] }],
  };
  f.sockets[0].frame(update);
  requests[1].finish(Response.json(update));
  await remove;
  await tick();
  assert.deepEqual(requests[2].body, { emoji: "🎉", active: true });
  assert.deepEqual(f.state.messages[0].reactions, [...update.reactions, { emoji: "🎉", authorIds: ["guest"] }]);
  requests[2].finish(Response.json({ error: "celebration failed" }, { status: 503 }));
  await assert.rejects(celebrate, /celebration failed/);
  assert.deepEqual(f.state.messages[0].reactions, update.reactions, "rollback retains the latest other-person count");

  const last = f.client.setReaction(target.id, "🎉", true);
  requests[3].finish(
    Response.json({ ...update, seq: "3", reactions: [...update.reactions, { emoji: "🎉", authorIds: ["guest"] }] }),
  );
  await last;
  const removingLast = f.client.setReaction(target.id, "🎉", false);
  assert.deepEqual(
    f.state.messages[0].reactions,
    update.reactions,
    "the last membership disappears before acknowledgement",
  );
  requests[4].finish(Response.json({ ...update, seq: "4" }));
  await removingLast;
  assert.deepEqual(f.state.messages[0].reactions, update.reactions);
});

test("a late reaction rejection after stopping cannot recreate a chat session", async (t) => {
  const f = await sendingFixture(t);
  let finish!: (response: Response) => void;
  const fetch = vi
    .spyOn(globalThis, "fetch")
    .mockClear()
    .mockImplementation(
      () =>
        new Promise<Response>((resolve) => {
          finish = resolve;
        }),
    );
  const request = f.client.setReaction("message-1", "👍", true);
  const before = f.state;
  f.client.stop();
  finish(Response.json({ error: "expired" }, { status: 401 }));
  await request;
  assert.equal(fetch.mock.calls.length, 1, "do not mint a session for an abandoned conversation");
  assert.equal(f.state, before);
  await assert.rejects(f.client.setReaction("message-1", "👍", true), /unavailable/);
  assert.equal(fetch.mock.calls.length, 1);
});

test("message sounds exclude history, own messages, and duplicate replay", async (t) => {
  const sounds: string[] = [];
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "Audio");
  Object.defineProperty(globalThis, "Audio", {
    configurable: true,
    value: class extends EventTarget {
      volume = 1;
      playbackRate = 1;
      preservesPitch = true;
      constructor(src: string) {
        super();
        sounds.push(src);
      }
      play() {
        return Promise.resolve();
      }
    },
  });
  t.onTestFinished(() => {
    if (descriptor) Object.defineProperty(globalThis, "Audio", descriptor);
    else Reflect.deleteProperty(globalThis, "Audio");
  });
  const f = await sendingFixture(t);
  const message = (seq: string, own = false) => ({
    ...committed({ clientMessageId: `command-${seq}`, text: "Hello" }, seq),
    author: { id: own ? "guest" : "other", name: "Sender", isGuest: true },
  });
  f.history.push(message("5"));
  f.client.retryLoad();
  await tick();
  f.sockets[0].frame({ type: "ready", cursor: "5" });
  assert.equal(sounds.length, 0);
  f.sockets[0].message(message("6"));
  assert.deepEqual(sounds, ["/audio/effects/new-message.wav"]);
  f.sockets[0].message(message("7", true));
  f.sockets[0].message(message("6"));
  f.sockets[0].message(message("4"));
  assert.equal(sounds.length, 1, "own, duplicate and older replay events are silent");
  f.sockets[0].message(message("8"));
  assert.equal(sounds.length, 2, "the next genuinely new incoming message should notify");
  await tick();
});

test("a client created without sounds stays silent for new messages", async (t) => {
  const sounds: string[] = [];
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "Audio");
  Object.defineProperty(globalThis, "Audio", {
    configurable: true,
    value: class extends EventTarget {
      constructor(src: string) {
        super();
        sounds.push(src);
      }
      play() {
        return Promise.resolve();
      }
    },
  });
  t.onTestFinished(() => {
    if (descriptor) Object.defineProperty(globalThis, "Audio", descriptor);
    else Reflect.deleteProperty(globalThis, "Audio");
  });
  const sockets = installBrowser(t);
  let state!: ChatViewState;
  const client = new ChatClient(
    (next) => {
      state = next;
    },
    undefined,
    { sounds: false },
  );
  t.onTestFinished(() => client.stop());
  client.start({
    space: { id: "space", name: "Caper" },
    channel: { id: "general", name: "general" },
    messages: [],
    cursor: "0",
    hasMore: false,
  });
  await tick();
  sockets[0].frame({ type: "ready", cursor: "0" });
  sockets[0].message({
    ...committed({ clientMessageId: "incoming", text: "Hi" }, "1"),
    author: { id: "other", name: "Other", isGuest: true },
  });
  assert.equal(state.messages.length, 1, "the message is still delivered");
  assert.equal(sounds.length, 0);
  await tick();
});

test("optimistic send is immediate; HTTP-first confirmation uses server content/order without skipping replay", async (t) => {
  const f = await sendingFixture(t);
  const sending = f.client.send("local text");
  assert.equal(f.state.pendingSend?.text, "local text");
  assert.equal(f.state.pendingSend?.author?.id, "guest");
  assert.ok(f.state.pendingSend?.createdAt);
  assert.equal(f.state.messages.length, 0, "provisional rows stay out of the ordered timeline");
  assert.equal(await f.client.send("double click"), false);
  assert.equal(f.posts.length, 1);
  const accepted = committed({ ...f.posts[0].body, text: "server-transformed text" }, "3");
  f.posts[0].resolve(Response.json(accepted));
  assert.equal(await sending, true);
  assert.equal(f.state.pendingSend, undefined);
  assert.deepEqual(f.state.messages, [accepted]);

  f.sockets[0].frame({ type: "migrating" });
  await tick();
  assert.equal(chatSubscription(f.sockets[1]).after, "0");
  for (const seq of ["1", "2"])
    f.sockets[0].message(committed({ clientMessageId: `other-${seq}`, text: "local text" }, seq));
  f.sockets[0].message(accepted);
  f.sockets[0].message(accepted);
  assert.deepEqual(
    f.state.messages.map((message) => message.seq),
    ["1", "2", "3"],
  );
  assert.deepEqual(f.state.messages[2], accepted);
});

test("WebSocket-first confirmation requires the sender and cannot be undone by a late failed HTTP response", async (t) => {
  const f = await sendingFixture(t);
  const sending = f.client.send("same text");
  const other = committed(f.posts[0].body, "1");
  other.author = { id: "someone-else", name: "Test Guest", isGuest: true };
  f.sockets[0].message(other);
  assert.ok(f.state.pendingSend, "matching text, name and UUID from a different author is not our acknowledgement");
  assert.equal(f.posts[0].signal.aborted, false, "another sender must not cancel the in-flight POST");
  const accepted = committed(f.posts[0].body, "2");
  f.sockets[0].message(accepted);
  assert.equal(await sending, true, "does not wait for HTTP once delivery is confirmed");
  assert.equal(Boolean(f.state.pendingSend), false);
  const nextSend = f.client.send("next message");
  f.posts[0].resolve(Response.json({ error: "late auth error" }, { status: 401 }));
  await tick();
  assert.equal(f.state.pendingSend?.text, "next message");
  assert.equal(f.state.sendError, undefined);
  assert.equal(f.state.sessionError, undefined);
  assert.ok(localStorage.getItem("caper.chat.session"), "late response cannot revoke a confirmed sender capability");
  f.posts[1].resolve(Response.json(committed(f.posts[1].body, "3")));
  assert.equal(await nextSend, true);
  assert.deepEqual(
    f.state.messages.map((message) => message.seq),
    ["1", "2", "3"],
  );
});

for (const transport of ["WebSocket", "history"] as const) {
  test(`${transport} confirmation cancels the obsolete POST without cancelling the next send`, async (t) => {
    const f = await sendingFixture(t);
    const sending = f.client.send("confirmed without an HTTP response");
    const post = f.posts[0];
    let aborts = 0;
    post.signal.addEventListener(
      "abort",
      () => {
        aborts++;
        post.reject(post.signal.reason);
      },
      { once: true },
    );
    assert.equal(post.signal.aborted, false, "the POST stays alive until delivery is confirmed");
    const accepted = committed(post.body, "1");
    if (transport === "WebSocket") f.sockets[0].message(accepted);
    else {
      f.history.push(accepted);
      f.client.retryLoad();
    }
    assert.equal(await sending, true);
    assert.equal(aborts, 1, "release the outstanding request immediately, not at its timeout");
    assert.deepEqual(f.state.messages, [accepted]);
    assert.equal(f.state.pendingSend, undefined);
    assert.equal(f.state.sendError, undefined);

    const next = f.client.send("a separate command");
    assert.equal(f.posts[1].signal.aborted, false, "cancellation is scoped to the completed command");
    assert.notEqual(f.posts[1].body.clientMessageId, post.body.clientMessageId);
    f.posts[1].resolve(Response.json(committed(f.posts[1].body, "2")));
    assert.equal(await next, true);
    await tick();
    assert.equal(f.state.sendError, undefined, "the old request's abort cannot become a send error");
    assert.deepEqual(
      f.state.messages.map((message) => message.seq),
      ["1", "2"],
    );
  });
}

test("failed optimistic row retries its exact command and replay can confirm during that retry", async (t) => {
  const f = await sendingFixture(t);
  const sending = f.client.send("original");
  f.posts[0].reject(new TypeError("lost response"));
  assert.equal(await sending, false);
  assert.equal(f.state.pendingSend?.text, "original");
  assert.equal(f.client.discardRejected(), undefined, "unknown outcome cannot silently become a new command");
  const retry = f.client.send("new draft must not replace retry text");
  assert.deepEqual(f.posts[1].body, f.posts[0].body);
  f.sockets[0].message(committed(f.posts[0].body, "1"));
  assert.equal(await retry, true);
  f.posts[1].resolve(Response.json({ error: "late conflict" }, { status: 409 }));
  await tick();
  assert.equal(f.state.pendingSend, undefined);
  assert.equal(f.state.sendError, undefined);
  assert.equal(f.state.sendRejected, undefined);
  assert.equal(f.state.messages.length, 1);
});

test("definitive rejection preserves text for editing and edited send receives a new UUID", async (t) => {
  const f = await sendingFixture(t);
  await assert.rejects(f.client.send("bad\u0000text"), /control characters/);
  assert.equal(Boolean(f.state.pendingSend), false);
  assert.equal(f.posts.length, 0);
  const sending = f.client.send("rejected text");
  f.posts[0].resolve(Response.json({ error: "rejected" }, { status: 422 }));
  assert.equal(await sending, false);
  assert.equal(f.state.sendRejected, true);
  assert.equal(f.state.pendingSend?.text, "rejected text");
  assert.equal(await f.client.send("edited"), false);
  assert.equal(f.client.discardRejected(), "rejected text");
  assert.equal(f.state.pendingSend, undefined);
  const edited = f.client.send("edited");
  assert.notEqual(f.posts[0].body.clientMessageId, f.posts[1].body.clientMessageId);
  assert.equal(f.posts[1].body.text, "edited");
  f.posts[1].resolve(Response.json(committed(f.posts[1].body, "1")));
  assert.equal(await edited, true);
});

test("history resync reconciles a failed optimistic row after a lost acknowledgement", async (t) => {
  const f = await sendingFixture(t);
  const sending = f.client.send("saved but not acknowledged");
  f.posts[0].reject(new TypeError("connection lost"));
  assert.equal(await sending, false);
  const accepted = committed(f.posts[0].body, "7");
  f.history.push(accepted);
  f.client.retryLoad();
  await tick();
  assert.equal(f.state.phase, "ready");
  assert.equal(f.state.pendingSend, undefined);
  assert.equal(f.state.sendError, undefined);
  assert.deepEqual(f.state.messages, [accepted]);
});

test("confirmation during HTTP rejection propagation cannot resurrect an optimistic row", async (t) => {
  const f = await sendingFixture(t);
  // Exercise confirmation before, inside, and after the fetch/race/catch
  // microtask chain, rather than only well-separated network callbacks.
  for (let delay = 0; delay < 8; delay++) {
    const sending = f.client.send(`message ${delay}`);
    f.posts[delay].reject(new TypeError("response lost"));
    for (let turn = 0; turn < delay; turn++) await Promise.resolve();
    f.sockets[0].message(committed(f.posts[delay].body, String(delay + 1)));
    await sending;
    await tick();
    assert.equal(f.state.pendingSend, undefined, `confirmed send resurrected at microtask ${delay}`);
    assert.equal(f.state.sendError, undefined);
    assert.equal(f.state.messages.length, delay + 1);
  }
});

function typingEvent(id: string, revision = "9007199254740992", typing = true): ChatTypingEvent {
  return {
    type: "typing.updated",
    channelId: "general",
    author: { id, name: "Shared Name", isGuest: true },
    revision,
    typing,
  };
}

test("typing is opt-in, author-deduplicated, expires independently, and never advances replay", async (t) => {
  vi.useFakeTimers({ toFake: ["setTimeout", "Date", "clearTimeout"], now: 1_000 });
  const f = await sendingFixture(t);
  assert.equal(chatSubscription(f.sockets[0]).channelId, "general");
  assert.equal(
    new URL(f.sockets[0].url).search,
    "",
    "typing is multiplexed rather than enabled through URL credentials",
  );
  f.sockets[0].frame(typingEvent("guest"));
  f.sockets[0].frame({ ...typingEvent("wrong-channel"), channelId: "private" });
  assert.equal(f.state.typingAuthors.length, 0);
  f.sockets[0].frame(typingEvent("a"));
  f.sockets[0].frame(typingEvent("a"));
  vi.advanceTimersByTime(4_000);
  f.sockets[0].frame(typingEvent("b"));
  f.sockets[0].frame(typingEvent("a"));
  assert.deepEqual(
    f.state.typingAuthors.map((author) => author.id),
    ["a", "b"],
    "same names are not the same identity",
  );
  vi.advanceTimersByTime(2_000);
  assert.deepEqual(
    f.state.typingAuthors.map((author) => author.id),
    ["b"],
    "duplicates do not prolong a stale indicator",
  );
  assert.deepEqual(f.state.messages, []);
  f.sockets[0].frame({ type: "migrating" });
  await tick();
  assert.equal(chatSubscription(f.sockets[1]).after, "0");
  f.sockets[1].frame(typingEvent("b", "9007199254740993", false));
  f.sockets[0].frame(typingEvent("b"));
  assert.deepEqual(
    f.state.typingAuthors,
    [],
    "an older overlapping start cannot undo a stop, even above JS's safe integer limit",
  );
  f.sockets[1].frame({ type: "ready", cursor: "0" });
  assert.equal(f.state.phase, "ready");
});

test("typing clears on message, offline, and history resync; unique typers are bounded", async (t) => {
  const f = await sendingFixture(t);
  f.sockets[0].frame(typingEvent("a"));
  const message = committed({ clientMessageId: "peer", text: "hello" }, "1");
  message.author.id = "a";
  f.sockets[0].message(message);
  assert.deepEqual(f.state.typingAuthors, []);
  f.sockets[0].frame(typingEvent("a", "9007199254740993"));
  f.sockets[0].dispatchEvent(new Event("close"));
  assert.deepEqual(f.state.typingAuthors, []);
  f.client.retryLoad();
  await new Promise((resolve) => setTimeout(resolve, 200));
  const current = f.sockets.at(-1)!;
  current.frame({ type: "ready", cursor: "0" });
  for (let index = 0; index < 100; index++) current.frame(typingEvent(`person-${index}`));
  assert.equal(f.state.typingAuthors.length, 64);
  f.client.retryLoad();
  assert.deepEqual(f.state.typingAuthors, []);
  await tick();
});

test("typing pulses are throttled, stop after inactivity, and failures stay out of send state", async (t) => {
  vi.useFakeTimers({ toFake: ["setTimeout", "Date", "clearTimeout"], now: 1_000 });
  const f = await sendingFixture(t);
  f.client.setTyping(true);
  f.typingPosts[0].resolve(new Response(null, { status: 204 }));
  await tick();
  for (let index = 0; index < 100; index++) f.client.setTyping(true);
  vi.advanceTimersByTime(249);
  f.client.setTyping(true);
  vi.advanceTimersByTime(249);
  f.client.setTyping(true);
  assert.equal(f.typingPosts.length, 1);
  vi.advanceTimersByTime(2);
  f.client.setTyping(true);
  assert.equal(f.typingPosts.length, 2);
  f.typingPosts[1].reject(new TypeError("broker unavailable"));
  await tick();
  vi.advanceTimersByTime(499);
  assert.deepEqual(
    f.typingPosts.map((post) => post.active),
    [true, true],
  );
  vi.advanceTimersByTime(1);
  assert.deepEqual(
    f.typingPosts.map((post) => post.active),
    [true, true, false],
  );
  f.typingPosts[2].resolve(new Response(null, { status: 429 }));
  await tick();
  assert.equal(f.state.sendError, undefined);
  assert.equal(f.state.sessionError, undefined);
  f.client.stop();
  f.client.setTyping(true);
  vi.advanceTimersByTime(10_000);
  assert.equal(f.typingPosts.length, 3);
});

test("a stop waits for its start but an optimistic send never waits for typing", async (t) => {
  const f = await sendingFixture(t);
  f.client.setTyping(true);
  f.client.setTyping(false);
  assert.deepEqual(
    f.typingPosts.map((post) => post.active),
    [true],
  );
  const sending = f.client.send("instant");
  assert.equal(f.state.pendingSend?.text, "instant");
  f.posts[0].resolve(Response.json(committed(f.posts[0].body, "1")));
  assert.equal(await sending, true);
  f.typingPosts[0].resolve(new Response(null, { status: 204 }));
  await tick();
  assert.deepEqual(
    f.typingPosts.map((post) => post.active),
    [true, false],
  );
  f.typingPosts[1].resolve(new Response(null, { status: 204 }));
  await tick();
  f.client.setTyping(false);
  assert.equal(f.typingPosts.length, 2);
});

async function paginationFixture(t: TestContext) {
  const sockets = installBrowser(t);
  const base = 9_007_199_254_740_990n;
  const message = (offset: number) =>
    committed({ clientMessageId: `command-${offset}`, text: `Message ${offset}` }, String(base + BigInt(offset)));
  const history = { messages: [message(4), message(5)], cursor: message(5).seq, hasMore: true };
  const requests: { url: string; resolve: (response: Response) => void }[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    const url = String(input);
    if (url === "/api/chat/general")
      return Response.json({
        ...history,
        space: { id: "space", name: "Caper" },
        channel: { id: "general", name: "General" },
      });
    return new Promise<Response>((resolve) => requests.push({ url, resolve }));
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => {
    state = next;
  });
  t.onTestFinished(() => client.stop());
  client.start();
  await tick();
  sockets[0].frame({ type: "ready", cursor: history.cursor });
  return {
    client,
    sockets,
    message,
    history,
    requests,
    get state() {
      return state;
    },
  };
}

test("history gaps discard older rows without erasing concurrent HTTP reaction snapshots", async (t) => {
  const f = await sendingFixture(t);
  const message = (seq: string) => committed({ clientMessageId: `command-${seq}`, text: `Message ${seq}` }, seq);
  for (const seq of ["1", "2", "3"]) f.sockets[0].message(message(seq));
  const fresh = { ...message("3"), author: { ...message("3").author, name: "Fresh author" } };
  const history = { ...f.client.snapshotHistory(), messages: [fresh, message("4")], cursor: "5", hasMore: true };
  const update = {
    type: "message.reactions" as const,
    schemaVersion: 1 as const,
    channelId: "general",
    messageId: fresh.id,
    seq: "6",
    reactions: [{ emoji: "👍", authorIds: ["guest"] }],
  };
  let finish!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation((input: string | URL | Request) => {
    if (String(input).endsWith("/reactions")) return Promise.resolve(Response.json(update));
    return new Promise<Response>((resolve) => {
      finish = resolve;
    });
  });
  f.sockets[0].frame({ type: "resync_required" });
  await f.client.setReaction(fresh.id, "👍", true);
  finish(Response.json(history));
  await tick();
  assert.deepEqual(
    f.state.messages.map((value) => value.seq),
    ["3", "4"],
    "missing event 5 may affect an omitted older row",
  );
  assert.equal(f.state.messages[0].author.name, "Fresh author");
  assert.deepEqual(f.state.messages[0].reactions, update.reactions);
  assert.equal(f.state.messages[0].reactionSeq, "6");
  assert.equal(f.client.snapshotHistory()?.cursor, "5", "HTTP acknowledgement cannot bridge missing replay events");
  assert.equal(f.state.hasMore, true);
});

test("older pages serialize, merge with live delivery, retry the same cursor, and stop at the beginning", async (t) => {
  const f = await paginationFixture(t);
  const loading = f.client.loadOlder();
  await f.client.loadOlder();
  assert.equal(f.requests.length, 1, "scroll callbacks cannot start duplicate requests");
  assert.equal(f.requests[0].url, `/api/chat/channels/general/messages?before=${f.message(4).seq}`);
  f.sockets[0].message(f.message(6));
  f.requests[0].resolve(
    Response.json({ messages: [f.message(2), f.message(3), f.message(4)], cursor: f.message(5).seq, hasMore: true }),
  );
  await loading;
  assert.deepEqual(
    f.state.messages,
    [2, 3, 4, 5, 6].map(f.message),
    "overlap is deduplicated without losing a concurrent live message",
  );
  f.sockets[0].frame({ type: "migrating" });
  await tick();
  assert.equal(
    chatSubscription(f.sockets[1]).after,
    f.message(6).seq,
    "older history never rewinds the live replay cursor",
  );

  const failed = f.client.loadOlder();
  f.requests[1].resolve(Response.json({ error: "temporary history outage" }, { status: 503 }));
  await failed;
  assert.equal(f.state.olderError, "temporary history outage");
  assert.equal(f.state.phase, "ready");
  assert.equal(f.state.loadingOlder, false);
  const retry = f.client.loadOlder();
  assert.equal(f.state.olderError, undefined);
  assert.equal(f.requests[2].url, f.requests[1].url);
  assert.equal(f.requests[2].url, `/api/chat/channels/general/messages?before=${f.message(2).seq}`);
  f.requests[2].resolve(Response.json({ messages: [f.message(1)], cursor: f.message(6).seq, hasMore: false }));
  await retry;
  assert.deepEqual(f.state.messages, [1, 2, 3, 4, 5, 6].map(f.message));
  await f.client.loadOlder();
  assert.equal(f.requests.length, 3);
});

for (const status of [200, 503]) {
  test(`an older page returning ${status} after resync cannot overwrite the new history request`, async (t) => {
    const f = await paginationFixture(t);
    const obsolete = f.client.loadOlder();
    f.history.messages = [f.message(8), f.message(9)];
    f.history.cursor = f.message(9).seq;
    f.sockets[0].frame({ type: "resync_required" });
    await tick();
    assert.equal(f.state.loadingOlder, false);
    const current = f.client.loadOlder();
    f.requests[0].resolve(
      status === 200
        ? Response.json({ messages: [f.message(1)], cursor: f.message(5).seq, hasMore: false })
        : Response.json({ error: "obsolete failure" }, { status }),
    );
    await obsolete;
    assert.deepEqual(f.state.messages, [f.message(8), f.message(9)]);
    assert.equal(f.state.hasMore, true);
    assert.equal(f.state.olderError, undefined);
    assert.equal(f.state.loadingOlder, true, "obsolete completion must not unlock a current request");
    f.requests[1].resolve(Response.json({ messages: [f.message(7)], cursor: f.message(9).seq, hasMore: true }));
    await current;
    assert.deepEqual(f.state.messages, [f.message(7), f.message(8), f.message(9)]);
  });
}

test("channel clients isolate history, gateway subscriptions, and late events across a switch", async (t) => {
  const sockets = installBrowser(t);
  const requests: string[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    const url = String(input);
    requests.push(url);
    const channelId = url.includes("alpha") ? "alphaChannel" : "bravoChannel";
    return Response.json({
      space: { id: "space1234567", name: "Studio" },
      channel: { id: channelId, name: channelId === "alphaChannel" ? "alpha" : "bravo" },
      messages: [],
      cursor: "0",
      hasMore: false,
    });
  });

  let alpha!: ChatViewState;
  const first = new ChatClient((state) => {
    alpha = state;
  }, "alphaChannel");
  first.start();
  await tick();
  assert.equal(requests[0], "/api/chat/channels/alphaChannel/messages");
  const alphaSubscription = chatSubscription(sockets[0]);
  assert.equal(alphaSubscription.channelId, "alphaChannel");
  first.stop();
  assert.equal(sockets[0].closed, false, "the shared socket remains available for the next channel");
  assert.equal(
    sockets[0].sent.some((frame) => frame.type === "unsubscribe" && frame.id === alphaSubscription.id),
    true,
  );

  let bravo!: ChatViewState;
  const second = new ChatClient((state) => {
    bravo = state;
  }, "bravoChannel");
  t.onTestFinished(() => second.stop());
  second.start();
  await tick();
  assert.equal(requests[1], "/api/chat/channels/bravoChannel/messages");
  assert.equal(sockets.length, 1);
  assert.equal(chatSubscription(sockets[0]).channelId, "bravoChannel");

  const stale: ChatMessage = {
    id: "stale",
    channelId: "alphaChannel",
    seq: "1",
    author: { id: "peer", name: "Peer", isGuest: false },
    content: { version: 1, type: "text", text: "wrong room" },
    createdAt: "2026-09-22T12:00:00Z",
    clientMessageId: "stale-command",
  };
  sockets[0].message(stale, alphaSubscription.id as string);
  assert.deepEqual(alpha.messages, []);
  assert.deepEqual(bravo.messages, [], "an old channel cannot leak messages into the replacement client");
});

test("prepared history is ready on the first render and starts live replay without another history fetch", async (t) => {
  const sockets = installBrowser(t);
  vi.spyOn(globalThis, "fetch").mockImplementation(() => {
    throw new Error("Unexpected duplicate history fetch");
  });
  const message: ChatMessage = {
    id: "seven",
    channelId: "alphaChannel",
    seq: "7",
    author: { id: "peer", name: "Peer", isGuest: false },
    content: { version: 1, type: "text", text: "Prepared message" },
    createdAt: "2026-09-23T12:00:00Z",
    clientMessageId: "command-seven",
  };
  const history: GeneralChatHistory = {
    space: { id: "space", name: "Studio" },
    channel: { id: "alphaChannel", name: "general" },
    messages: [message],
    cursor: "7",
    hasMore: true,
  };
  const firstRender = initialChatView(history);
  assert.equal(firstRender.phase, "ready");
  assert.deepEqual(firstRender.messages, [message]);
  const states: ChatViewState[] = [];
  const client = new ChatClient((state) => states.push(state), "alphaChannel");
  t.onTestFinished(() => client.stop());
  client.start(history);
  assert.ok(states.every((state) => state.phase === "ready"));
  await tick();
  assert.equal(chatSubscription(sockets[0]).after, "7");
  sockets[0].message({
    ...message,
    id: "eight",
    seq: "8",
    clientMessageId: "command-eight",
    content: { version: 1, type: "text", text: "Arrived during navigation" },
  });
  assert.deepEqual(
    states.at(-1)?.messages.map((item) => item.content.text),
    ["Prepared message", "Arrived during navigation"],
  );
  client.stop();
  assert.equal(sockets[0].closed, false);
});

test("prefetch rejects a history payload containing another channel's messages", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
    Response.json({
      space: { id: "space", name: "Studio" },
      channel: { id: "alphaChannel", name: "general" },
      messages: [
        {
          id: "one",
          channelId: "bravoChannel",
          seq: "1",
          author: { id: "peer", name: "Peer", isGuest: false },
          content: { version: 1, type: "text", text: "Wrong channel" },
          createdAt: "2026-09-23T12:00:00Z",
          clientMessageId: "command-one",
        },
      ],
      cursor: "1",
      hasMore: false,
    }),
  );
  await assert.rejects(loadChatHistory("alphaChannel"), /another channel/);
});

test("a prepared history failure renders once and retries only when requested", async (t) => {
  const sockets = installBrowser(t);
  let requests = 0;
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => {
    requests++;
    return Response.json({
      space: { id: "space", name: "Studio" },
      channel: { id: "alphaChannel", name: "general" },
      messages: [],
      cursor: "0",
      hasMore: false,
    });
  });
  let view = initialChatView(undefined, "Messaging unavailable");
  assert.equal(view.phase, "error");
  const client = new ChatClient((next) => {
    view = next;
  }, "alphaChannel");
  t.onTestFinished(() => client.stop());
  client.start(undefined, "Messaging unavailable");
  assert.equal(requests, 0);
  assert.equal(sockets.length, 0);
  client.retryLoad();
  await tick();
  assert.equal(requests, 1);
  assert.equal(view.phase, "ready");
  assert.equal(sockets.length, 1);
});

test("snapshots include older pages and live messages; returning replays the missing tail without loading", async (t) => {
  const f = await paginationFixture(t);
  const older = f.client.loadOlder();
  f.requests[0].resolve(
    Response.json({ messages: [f.message(2), f.message(3)], cursor: f.message(5).seq, hasMore: false }),
  );
  await older;
  f.sockets[0].message(f.message(6));
  const snapshot = f.client.snapshotHistory()!;
  f.client.stop();
  assert.deepEqual(snapshot.messages, [2, 3, 4, 5, 6].map(f.message));
  assert.equal(snapshot.cursor, f.message(6).seq);
  assert.equal(snapshot.hasMore, false);
  const states: ChatViewState[] = [];
  const returning = new ChatClient((state) => states.push(state), "general");
  t.onTestFinished(() => returning.stop());
  returning.start(snapshot);
  assert.equal(chatSubscription(f.sockets[0]).after, f.message(6).seq);
  assert.deepEqual(states.at(-1)?.messages, snapshot.messages);
  f.sockets[0].message(f.message(7));
  assert.deepEqual(states.at(-1)?.messages, [2, 3, 4, 5, 6, 7].map(f.message));
  assert.ok(states.every((state) => state.phase === "ready"));
  assert.equal(f.requests.length, 1, "return does not request another history page");
});

test("unscoped direct-message history can be retained in a timeline snapshot", async (t) => {
  installBrowser(t);
  const history: GeneralChatHistory = {
    space: { id: "", name: "Direct messages" },
    channel: { id: "direct000001", name: "Mira", direct: true },
    messages: [],
    cursor: "0",
    hasMore: false,
  };
  const client = new ChatClient(() => undefined, "direct000001");
  t.onTestFinished(() => client.stop());
  client.start(history);
  assert.equal(client.snapshotHistory()?.space.id, "");
  assert.equal(client.snapshotHistory()?.channel.name, "Mira");
  assert.deepEqual(client.snapshotHistory(), history, "snapshots preserve direct-message channel metadata");
});

test("resync retains visible messages through transient failures but clears them on access denial", async (t) => {
  const f = await paginationFixture(t);
  let finish!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(
    () =>
      new Promise<Response>((resolve) => {
        finish = resolve;
      }),
  );
  f.sockets[0].frame({ type: "resync_required" });
  assert.equal(f.state.phase, "ready");
  assert.deepEqual(f.state.messages, [4, 5].map(f.message));
  finish(Response.json({ error: "Temporary outage" }, { status: 503 }));
  await tick();
  assert.equal(f.state.phase, "ready");
  assert.equal(f.state.error, "Temporary outage");
  assert.deepEqual(f.state.messages, [4, 5].map(f.message));
  f.client.retryLoad();
  finish(
    Response.json({ ...f.client.snapshotHistory(), messages: [f.message(6)], cursor: f.message(7).seq, hasMore: true }),
  );
  await tick();
  assert.deepEqual(f.state.messages, [f.message(6)], "missing sequence 7 may be a reaction on an older cached row");
  assert.equal(f.state.hasMore, true, "older pages can be loaded again from authoritative history");
  assert.equal(f.state.error, undefined);
  f.client.retryLoad();
  finish(Response.json({ error: "Access removed" }, { status: 403 }));
  await tick();
  assert.equal(f.state.phase, "error");
  assert.deepEqual(f.state.messages, []);
  assert.equal(f.client.snapshotHistory(), undefined);
});

test("older history from another channel is rejected without contaminating the saved timeline", async (t) => {
  const f = await paginationFixture(t);
  const loading = f.client.loadOlder();
  f.requests[0].resolve(
    Response.json({
      messages: [f.message(2), { ...f.message(3), channelId: "other-channel" }],
      cursor: f.message(5).seq,
      hasMore: false,
    }),
  );
  await loading;
  assert.deepEqual(f.state.messages, [4, 5].map(f.message), "reject the whole page, including valid rows");
  assert.match(f.state.olderError!, /another channel/);
  assert.equal(f.state.loadingOlder, false);
  assert.equal(f.state.hasMore, true, "the failed page must remain retryable");
  assert.deepEqual(f.client.snapshotHistory()?.messages, [4, 5].map(f.message));
});

test("prepared history receives the same channel isolation checks as fetched history", async (t) => {
  const sockets = installBrowser(t);
  let state!: ChatViewState;
  const client = new ChatClient((next) => {
    state = next;
  }, "general");
  t.onTestFinished(() => client.stop());
  client.start({
    space: { id: "space", name: "Caper" },
    channel: { id: "general", name: "general" },
    messages: [
      { ...committed({ clientMessageId: "foreign", text: "Wrong channel" }, "1"), channelId: "other-channel" },
    ],
    cursor: "1",
    hasMore: false,
  });
  assert.equal(state.phase, "error");
  assert.match(state.error!, /another channel/);
  assert.deepEqual(state.messages, []);
  assert.equal(sockets.length, 0);
});

for (const status of [200, 503]) {
  test(`an obsolete session response (${status}) cannot overwrite a newer identity or error state`, async (t) => {
    installBrowser(t);
    const requests: Array<(response: Response) => void> = [];
    vi.spyOn(globalThis, "fetch").mockImplementation(() => new Promise<Response>((resolve) => requests.push(resolve)));
    let state!: ChatViewState;
    const client = new ChatClient((next) => {
      state = next;
    });
    t.onTestFinished(() => client.stop());
    client.identify("Old", true);
    client.identify("Current", true);
    const current = { token: "current", author: { id: "current", name: "Current", isGuest: false } };
    requests[1](Response.json(current));
    await tick();
    requests[0](
      status === 200
        ? Response.json({ token: "obsolete", author: { id: "old", name: "Old", isGuest: false } })
        : Response.json({ error: "Obsolete session failure" }, { status }),
    );
    await tick();
    assert.deepEqual(state.author, current.author);
    assert.equal(state.sessionError, undefined);
    assert.deepEqual(JSON.parse(localStorage.getItem("caper.chat.session")!), current);
  });
}

test("stopped clients cannot persist a late session or start a session retry", async (t) => {
  installBrowser(t);
  let finish!: (response: Response) => void;
  let requests = 0;
  vi.spyOn(globalThis, "fetch").mockImplementation(() => {
    requests++;
    return new Promise<Response>((resolve) => {
      finish = resolve;
    });
  });
  const states: ChatViewState[] = [];
  const client = new ChatClient((state) => states.push(state));
  client.identify("Departed", true);
  client.stop();
  const updatesBeforeCompletion = states.length;
  finish(Response.json({ token: "obsolete", author: { id: "departed", name: "Departed", isGuest: false } }));
  await tick();
  client.retrySession();
  assert.equal(localStorage.getItem("caper.chat.session"), null);
  assert.equal(states.length, updatesBeforeCompletion);
  assert.equal(requests, 1);
});

test("resync keeps paginated history while refreshing overlapping author metadata", async (t) => {
  const f = await paginationFixture(t);
  const older = f.client.loadOlder();
  f.requests[0].resolve(
    Response.json({ messages: [f.message(2), f.message(3)], cursor: f.message(5).seq, hasMore: false }),
  );
  await older;
  let finish!: (response: Response) => void;
  let requests = 0;
  vi.spyOn(globalThis, "fetch").mockImplementation(() => {
    requests++;
    return new Promise<Response>((resolve) => {
      finish = resolve;
    });
  });
  const snapshot = f.client.snapshotHistory()!;
  f.client.retryLoad();
  await f.client.loadOlder();
  assert.equal(requests, 1);
  assert.deepEqual(
    f.state.messages.map((message) => message.seq),
    [2, 3, 4, 5].map((offset) => f.message(offset).seq),
  );
  const fresh = { ...f.message(5), author: { ...f.message(5).author, name: "Updated name", avatarId: 42 } };
  finish(Response.json({ ...snapshot, messages: [fresh, f.message(6)], cursor: f.message(6).seq, hasMore: true }));
  await tick();
  assert.deepEqual(f.state.messages, [f.message(2), f.message(3), f.message(4), fresh, f.message(6)]);
  assert.equal(f.state.hasMore, false, "the retained prefix has already reached the beginning");
});

test("pagination cannot start during a refresh even when older pages remain", async (t) => {
  const f = await paginationFixture(t);
  const snapshot = f.client.snapshotHistory()!;
  const requests: ((response: Response) => void)[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(
    () =>
      new Promise<Response>((resolve) => {
        requests.push(resolve);
      }),
  );
  f.client.retryLoad();
  const blockedPage = f.client.loadOlder();
  assert.equal(
    requests.length,
    1,
    "a page started after the resync generation would otherwise be lost or corrupt hasMore",
  );
  await blockedPage;
  requests[0](Response.json({ ...snapshot, messages: [f.message(6)], cursor: f.message(6).seq }));
  await tick();
  const older = f.client.loadOlder();
  assert.equal(requests.length, 2, "pagination resumes when refresh finishes");
  requests[1](Response.json({ messages: [f.message(3)], cursor: f.message(6).seq, hasMore: false }));
  await older;
  assert.deepEqual(f.state.messages, [3, 4, 5, 6].map(f.message));
});

test("resync replaces a disconnected range so pagination can fill its gap", async (t) => {
  const f = await paginationFixture(t);
  const snapshot = f.client.snapshotHistory()!;
  vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
    Response.json({
      ...snapshot,
      messages: [f.message(7)],
      cursor: f.message(7).seq,
      hasMore: true,
    }),
  );
  f.client.retryLoad();
  await tick();
  assert.deepEqual(f.state.messages, [f.message(7)]);
  assert.equal(f.state.hasMore, true);
});

test("older history from another channel is rejected without polluting the timeline", async (t) => {
  const f = await paginationFixture(t);
  const loading = f.client.loadOlder();
  f.requests[0].resolve(
    Response.json({
      messages: [f.message(2), { ...f.message(3), channelId: "other" }],
      cursor: f.message(5).seq,
      hasMore: false,
    }),
  );
  await loading;
  assert.deepEqual(f.state.messages, [4, 5].map(f.message));
  assert.equal(f.state.hasMore, true);
  assert.equal(f.state.olderError, "The chat service returned messages from another channel.");
});

test("old thread parents and broadcast replies do not move channel paging or survive a resync as channel rows", async (t) => {
  const f = await paginationFixture(t);
  const root = f.message(1);
  const reply = { ...f.message(2), threadRootId: root.id, broadcast: true };
  const loading = f.client.openThread(root.id);
  f.requests[0].resolve(Response.json({ root, messages: [reply], cursor: f.history.cursor, hasMore: false }));
  await loading;
  assert.deepEqual(
    f.state.channelMessages?.map((message) => message.id),
    [f.message(4).id, f.message(5).id],
  );
  assert.deepEqual(
    f.client.snapshotHistory()?.messages.map((message) => message.id),
    [f.message(4).id, f.message(5).id],
  );
  const older = f.client.loadOlder();
  assert.equal(f.requests[1].url, `/api/chat/channels/general/messages?before=${f.message(4).seq}`);
  f.requests[1].resolve(Response.json({ messages: [f.message(3)], cursor: f.history.cursor, hasMore: true }));
  await older;
  f.client.retryLoad();
  await tick();
  assert.deepEqual(
    f.state.channelMessages?.map((message) => message.id),
    [3, 4, 5].map((offset) => f.message(offset).id),
  );
  const threadReload = f.requests.at(-1)!;
  threadReload.resolve(Response.json({ root, messages: [reply], cursor: f.history.cursor, hasMore: false }));
  await tick();
  assert.deepEqual(
    f.state.channelMessages?.map((message) => message.id),
    [3, 4, 5].map((offset) => f.message(offset).id),
  );
  const page = f.client.loadOlder();
  f.requests.at(-1)!.resolve(Response.json({ messages: [root, reply], cursor: f.history.cursor, hasMore: false }));
  await page;
  assert.deepEqual(
    f.state.channelMessages?.map((message) => message.id),
    [1, 2, 3, 4, 5].map((offset) => f.message(offset).id),
  );
});

test("late thread responses cannot reopen a closed or different thread", async (t) => {
  const f = await paginationFixture(t);
  const first = f.client.openThread(f.message(4).id);
  const second = f.client.openThread(f.message(5).id);
  f.requests[0].resolve(Response.json({ root: f.message(4), messages: [], cursor: f.history.cursor, hasMore: false }));
  await first;
  assert.equal(f.state.thread?.rootId, f.message(5).id);
  f.client.closeThread();
  f.requests[1].resolve(Response.json({ root: f.message(5), messages: [], cursor: f.history.cursor, hasMore: false }));
  await second;
  assert.equal(f.state.thread, undefined);
  assert.equal(f.client.snapshotHistory()?.cursor, f.history.cursor);
});

test("repeated thread clicks share the pending request and leave a loaded view untouched", async (t) => {
  const f = await paginationFixture(t);
  const root = f.message(4);
  const first = f.client.openThread(root.id);
  const loadingState = f.state;
  const second = f.client.openThread(root.id);
  assert.equal(f.requests.length, 1);
  assert.equal(f.state, loadingState, "a repeated click must not publish a new loading state");
  f.requests[0].resolve(Response.json({ root, messages: [], cursor: f.history.cursor, hasMore: false }));
  await Promise.all([first, second]);
  const loadedState = f.state;
  await f.client.openThread(root.id);
  assert.equal(f.requests.length, 1);
  assert.equal(f.state, loadedState);
});

test("cached thread reopening preserves older-page boundaries and receives live replies", async (t) => {
  const f = await paginationFixture(t);
  const root = f.message(1);
  const reply = (offset: number) => ({ ...f.message(offset), threadRootId: root.id, broadcast: false });
  const open = f.client.openThread(root.id);
  f.requests[0].resolve(Response.json({ root, messages: [reply(3)], cursor: f.history.cursor, hasMore: true }));
  await open;
  const older = f.client.loadOlderThread();
  const repeated = f.client.openThread(root.id);
  assert.equal(f.state.thread?.loadingOlder, true, "reopening cannot reset a pending older page");
  f.requests[1].resolve(Response.json({ root, messages: [reply(2)], cursor: f.history.cursor, hasMore: true }));
  await Promise.all([older, repeated]);
  f.client.closeThread();
  f.sockets[0].message(reply(6));
  await f.client.openThread(root.id);
  assert.equal(f.requests.length, 2, "reopening uses loaded data, including live updates");
  assert.equal(f.state.thread?.loading, false);
  assert.deepEqual(
    f.state.messages.filter((message) => message.threadRootId === root.id),
    [2, 3, 6].map(reply),
  );
  const next = f.client.loadOlderThread();
  assert.equal(f.requests[2].url, `/api/chat/channels/general/messages/${root.id}/thread?before=${reply(2).seq}`);
  f.requests[2].resolve(Response.json({ root, messages: [], cursor: f.history.cursor, hasMore: false }));
  await next;
});

test("hover prefetch and click share a request without opening or replacing another thread", async (t) => {
  const f = await paginationFixture(t);
  const first = f.message(4);
  const second = f.message(5);
  const prefetch = f.client.prefetchThread(first.id);
  const duplicate = f.client.prefetchThread(first.id);
  assert.equal(Boolean(f.state.thread), false);
  const click = f.client.openThread(first.id);
  assert.equal(f.requests.length, 1);
  const other = f.client.openThread(second.id);
  f.requests[0].resolve(Response.json({ root: first, messages: [], cursor: f.history.cursor, hasMore: false }));
  await Promise.all([prefetch, duplicate, click]);
  assert.equal(f.state.thread?.rootId, second.id);
  assert.equal(f.state.thread?.loading, true);
  f.requests[1].resolve(Response.json({ root: second, messages: [], cursor: f.history.cursor, hasMore: false }));
  await other;
  await f.client.openThread(first.id);
  assert.equal(f.requests.length, 2, "completed background fetch is cached for an instant open");
  assert.equal(f.state.thread?.loading, false);
});

test("failed prefetch is silent and does not prevent explicit load and retry", async (t) => {
  const f = await paginationFixture(t);
  const root = f.message(4);
  const prefetch = f.client.prefetchThread(root.id);
  f.requests[0].resolve(Response.json({ error: "temporary" }, { status: 503 }));
  await prefetch;
  assert.equal(Boolean(f.state.thread), false);
  assert.equal(f.state.error, undefined);
  const open = f.client.openThread(root.id);
  f.requests[1].resolve(Response.json({ error: "try again" }, { status: 503 }));
  await open;
  assert.equal(f.state.thread?.error, "try again");
  const retry = f.client.retryThread();
  f.requests[2].resolve(Response.json({ root, messages: [], cursor: f.history.cursor, hasMore: false }));
  await retry;
  assert.equal(f.state.thread?.error, undefined);
  assert.equal(f.state.thread?.loading, false);
});

test("resync invalidates thread pages and discards prefetches from the previous generation", async (t) => {
  const f = await paginationFixture(t);
  const root = f.message(4);
  const oldPrefetch = f.client.prefetchThread(root.id);
  f.client.retryLoad();
  await tick();
  f.requests[0].resolve(Response.json({ root, messages: [], cursor: f.history.cursor, hasMore: false }));
  await oldPrefetch;
  const open = f.client.openThread(root.id);
  assert.equal(f.requests.length, 2, "a stale response cannot restore the invalidated cache");
  f.requests[1].resolve(Response.json({ root, messages: [], cursor: f.history.cursor, hasMore: false }));
  await open;
  f.client.closeThread();
  f.client.retryLoad();
  await tick();
  const reopened = f.client.openThread(root.id);
  assert.equal(f.requests.length, 3, "completed pages are also invalidated on resync");
  f.requests[2].resolve(Response.json({ root, messages: [], cursor: f.history.cursor, hasMore: false }));
  await reopened;
});

test("thread send retry freezes root and broadcast and confirms one shared reply", async (t) => {
  const f = await sendingFixture(t);
  const bodies: Array<{ clientMessageId: string; text: string; threadRootId: string; broadcast: boolean }> = [];
  let fail = true;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (_input: unknown, init?: RequestInit) => {
    const body = JSON.parse(String(init?.body));
    bodies.push(body);
    if (fail) return Response.json({ error: "temporary" }, { status: 503 });
    return Response.json({ ...committed(body, "1"), threadRootId: body.threadRootId, broadcast: body.broadcast });
  });
  assert.equal(await f.client.send("broadcast reply", { threadRootId: "parent", broadcast: true }), false);
  assert.equal(await f.client.send("unrelated channel draft"), false, "channel composer cannot retry a thread command");
  fail = false;
  assert.equal(await f.client.send("different text", { threadRootId: "parent", broadcast: false }), true);
  assert.deepEqual(bodies[1], bodies[0]);
  assert.equal(f.state.pendingSend, undefined);
  assert.equal(f.state.channelMessages?.length, 1);
  const reply = f.state.messages[0];
  f.sockets[0].message(reply);
  assert.equal(f.state.messages.length, 1);
  assert.equal(f.client.snapshotHistory()?.cursor, "1");
});

test("duplicate delivery and local edits preserve unchanged message references", async (t) => {
  const f = await sendingFixture(t);
  const first = committed({ clientMessageId: "first", text: "First" }, "1");
  const second = committed({ clientMessageId: "second", text: "Second" }, "2");
  f.sockets[0].message(first);
  f.sockets[0].message(second);
  const rendered = f.state.messages;
  f.sockets[0].message(second);
  assert.equal(f.state.messages, rendered, "duplicate delivery must retain the render snapshot");
  let finish!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(
    () =>
      new Promise<Response>((resolve) => {
        finish = resolve;
      }),
  );
  const saving = f.client.editMessage(first.id, "Changed first", 1);
  assert.notEqual(f.state.messages[0], rendered[0]);
  assert.equal(f.state.messages[0].content.text, "Changed first");
  assert.equal(f.state.messages[1], rendered[1], "an edit must not clone unrelated rows");
  assert.equal(rendered[0].content.text, "First", "optimism must not mutate the confirmed row");
  finish(Response.json({ error: "not saved" }, { status: 503 }));
  await assert.rejects(saving, /not saved/);
  assert.equal(f.state.messages, rendered, "rollback reuses the unchanged authoritative snapshot");
});

for (const loaded of [true, false]) {
  test(`pin reaction intents roll back to live state without cloning unrelated messages (loaded=${loaded})`, async (t) => {
    const f = await sendingFixture(t);
    const target = committed({ clientMessageId: "pinned", text: "Pinned" }, "1");
    const pinned = { ...target, pin: { author: target.author, createdAt: target.createdAt }, pinSeq: "2" };
    const unrelated = committed({ clientMessageId: "other", text: "Unchanged" }, "3");
    f.client.start({
      ...f.client.snapshotHistory()!,
      messages: loaded ? [pinned, unrelated] : [unrelated],
      pinnedMessages: [pinned],
      cursor: "3",
    });
    await tick();
    const stable = f.state.messages.at(-1);
    let finish!: (response: Response) => void;
    vi.spyOn(globalThis, "fetch").mockImplementation(
      () =>
        new Promise<Response>((resolve) => {
          finish = resolve;
        }),
    );
    const saving = f.client.setReaction(target.id, "👍", true);
    assert.deepEqual(f.state.pinnedMessages[0].reactions, [{ emoji: "👍", authorIds: ["guest"] }]);
    assert.equal(f.client.snapshotHistory()?.pinnedMessages?.[0].reactions, undefined);
    assert.strictEqual(f.state.messages.at(-1), stable, "local intents must not clone unrelated rows");
    f.sockets[0].frame({
      type: "message.reactions",
      schemaVersion: 1,
      channelId: "general",
      messageId: target.id,
      seq: "4",
      reactions: [{ emoji: "🎉", authorIds: ["peer"] }],
    });
    assert.deepEqual(f.state.pinnedMessages[0].reactions, [
      { emoji: "🎉", authorIds: ["peer"] },
      { emoji: "👍", authorIds: ["guest"] },
    ]);
    finish(Response.json({ error: "temporary" }, { status: 503 }));
    await assert.rejects(saving, /temporary/);
    assert.deepEqual(f.state.pinnedMessages[0].reactions, [{ emoji: "🎉", authorIds: ["peer"] }]);
    assert.strictEqual(f.state.messages.at(-1), stable);
    assert.equal(f.state.messages.length, loaded ? 2 : 1);
    assert.equal(f.client.snapshotHistory()?.cursor, "4");
  });
}

test("edit PUTs keep expected revision, merge live updates and reject late abandoned acknowledgements", async (t) => {
  const f = await sendingFixture(t);
  const original = committed({ clientMessageId: "edit-root", text: "Friday" }, "1");
  f.sockets[0].message(original);
  const version2 = {
    ...original,
    revision: 2,
    editSeq: "2",
    editedAt: original.createdAt,
    content: { ...original.content, text: "Saturday" },
  };
  const responses: Array<(response: Response) => void> = [];
  vi.spyOn(globalThis, "fetch").mockImplementation((input: unknown, init?: RequestInit) => {
    assert.equal(String(input), "/api/chat/channels/general/messages/message-1");
    assert.equal(new Headers(init?.headers).get("x-caper-chat-token"), "opaque");
    assert.equal(init?.method, "PUT");
    assert.deepEqual(Object.keys(JSON.parse(String(init?.body))).sort(), ["expectedRevision", "text"]);
    return new Promise<Response>((resolve) => responses.push(resolve));
  });
  const save = f.client.editMessage(original.id, "Saturday", 1);
  assert.equal(f.state.messages[0].content.text, "Saturday", "edit text projects before HTTP resolves");
  assert.equal(f.state.messages[0].revision, undefined, "pending edits do not invent a confirmed revision");
  assert.equal(f.client.snapshotHistory()?.messages[0].content.text, "Friday");
  responses[0](Response.json(version2));
  await save;
  assert.equal(f.state.messages[0].content.text, "Saturday");
  assert.equal(f.client.snapshotHistory()?.cursor, "1");
  f.sockets[0].frame({ type: "message.edited", schemaVersion: 1, channelId: "general", seq: "2", message: version2 });
  assert.equal(f.client.snapshotHistory()?.cursor, "2");
  assert.equal(f.state.messages.length, 1);
  const conflict = f.client.editMessage(original.id, "Old draft", 1);
  responses[1](Response.json({ error: "message changed" }, { status: 409 }));
  await assert.rejects(conflict, /message changed/);
  assert.equal(f.state.messages[0].content.text, "Saturday");
  const late = f.client.editMessage(original.id, "Sunday", 2);
  const before = f.state;
  f.client.stop();
  responses[2](
    Response.json({ ...version2, revision: 3, editSeq: "3", content: { ...version2.content, text: "Sunday" } }),
  );
  await assert.rejects(late, /conversation changed/);
  assert.equal(f.state, before);
});

test("failed optimistic edit reveals a newer remote revision while keeping independent pins", async (t) => {
  const f = await sendingFixture(t);
  const original = committed({ clientMessageId: "edit-target", text: "@peer Original" }, "1");
  original.content.mentions = [{ type: "user", id: "peer", username: "peer" }];
  const pinned = { ...original, pin: { author: original.author, createdAt: original.createdAt }, pinSeq: "1" };
  f.sockets[0].message(pinned);
  let finish!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(
    () =>
      new Promise<Response>((resolve) => {
        finish = resolve;
      }),
  );
  const saving = f.client.editMessage(original.id, "Local draft", 1);
  assert.equal(f.state.messages[0].content.text, "Local draft");
  assert.equal(f.state.pinnedMessages[0].content.text, "Local draft");
  assert.deepEqual(f.state.messages[0].content.mentions, [], "only the server resolves mentions for the new text");
  assert.deepEqual(f.client.snapshotHistory()?.messages[0].content.mentions, original.content.mentions);
  const remote = {
    ...original,
    revision: 2,
    editSeq: "2",
    editedAt: original.createdAt,
    content: { ...original.content, text: "Other tab" },
  };
  f.sockets[0].frame({ type: "message.edited", schemaVersion: 1, channelId: "general", seq: "2", message: remote });
  assert.equal(f.state.messages[0].content.text, "Other tab", "a confirmed newer revision supersedes a stale draft");
  finish(Response.json({ error: "message changed" }, { status: 409 }));
  await assert.rejects(saving, /message changed/);
  assert.equal(f.state.messages[0].content.text, "Other tab");
  assert.equal(f.state.pinnedMessages[0].content.text, "Other tab");
  assert.deepEqual(f.state.pinnedMessages[0].pin, pinned.pin);
  assert.equal(f.client.snapshotHistory()?.cursor, "2");
});

test("typing refreshes from the same people do not re-render the chat", async (t) => {
  vi.useFakeTimers({ toFake: ["setTimeout", "Date", "clearTimeout"], now: 1_000 });
  const f = await sendingFixture(t);
  f.sockets[0].frame(typingEvent("a"));
  const shown = f.state;
  assert.deepEqual(
    shown.typingAuthors.map((author) => author.id),
    ["a"],
  );
  vi.advanceTimersByTime(500);
  f.sockets[0].frame(typingEvent("a", "9007199254740993"));
  assert.equal(f.state, shown, "an unchanged typer list publishes no new state");
  f.sockets[0].frame(typingEvent("a", "9007199254740994", false));
  assert.deepEqual(f.state.typingAuthors, []);
});

test("signed-in conversations share one chat session per account and replace it only on 401", async (t) => {
  installBrowser(t);
  let sessions = 0;
  const reactions: number[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    const path = String(input);
    if (path === "/api/chat/session") {
      sessions++;
      return Response.json({
        token: `token-${sessions}`,
        author: { id: "account", name: "Person", isGuest: false },
      });
    }
    if (path.endsWith("/reactions")) {
      const status = reactions.shift()!;
      return Response.json(
        { error: "this person isn't accepting direct messages", code: "dm_not_accepted" },
        { status },
      );
    }
    return Response.json({
      space: { id: "space", name: "Caper" },
      channel: { id: "general", name: "General" },
      messages: [],
      cursor: "0",
      hasMore: false,
    });
  });
  const open = () => {
    let state!: ChatViewState;
    const client = new ChatClient((next) => {
      state = next;
    });
    t.onTestFinished(() => client.stop());
    client.start();
    client.identify("Person", true, "account");
    return { client, state: () => state };
  };
  const first = open();
  const second = open();
  await tick();
  await tick();
  assert.equal(sessions, 1, "switching conversations reuses the account's session");
  assert.equal(first.state().author?.id, "account");
  assert.equal(second.state().author?.id, "account");

  // A refusal such as a block is not an invalid session.
  reactions.push(403);
  await assert.rejects(first.client.setReaction("message", "👍", true), /accepting direct messages/);
  await tick();
  assert.equal(sessions, 1);

  reactions.push(401);
  await assert.rejects(first.client.setReaction("message", "👍", true));
  await tick();
  await tick();
  assert.equal(sessions, 2, "an expired capability is replaced once");
  const third = open();
  await tick();
  await tick();
  assert.equal(sessions, 2, "later conversations reuse the replacement");
  assert.equal(third.state().author?.id, "account");
});

test("a DM send refused by a block is final, like on the native clients", async (t) => {
  const f = await sendingFixture(t);
  const sending = f.client.send("are you there?");
  await tick();
  f.posts[0].resolve(
    Response.json({ error: "this person isn't accepting direct messages", code: "dm_not_accepted" }, { status: 403 }),
  );
  await sending.catch(() => undefined);
  await tick();
  assert.equal(f.state.sendRejected, true);
  assert.equal(f.state.sendError, "This person isn’t accepting direct messages.");
});
