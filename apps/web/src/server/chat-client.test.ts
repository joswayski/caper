import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { ChatClient, initialChatView, loadChatHistory, type ChatViewState } from "../chat/client.ts";
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
    assert.deepEqual(Object.keys(frame.body as object), ["typing"], "draft text and claimed identity never enter typing commands");
    const id = frame.id as string;
    this.commands.push({
      active: (frame.body as { typing?: boolean }).typing === true,
      resolve: (response) => this.raw({ type: "result", id, status: response.status, body: response.status < 300 ? {} : { error: response.statusText || "Typing failed." } }),
      reject: (error) => this.raw({ type: "result", id, status: 500, body: { error: error.message } }),
    });
  }
  private raw(value: unknown) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(value) })); }
  subscriptions() { return this.sent.filter((frame) => frame.type === "subscribe" && frame.kind === "chat"); }
  frame(event: ChatEvent | { type: "migrating" }, subscriptionId?: string) {
    if (event.type === "migrating") { this.raw(event); return; }
    const id = subscriptionId ?? this.subscriptions().at(-1)?.id;
    assert.equal(typeof id, "string", "chat event requires an active gateway subscription");
    this.raw({ type: "event", id, event });
    if (event.type === "ready") this.raw({ type: "subscribed", id });
  }
  message(message: ChatMessage, subscriptionId?: string) {
    this.frame({ type: "message.created", channelId: message.channelId, seq: message.seq, message }, subscriptionId);
  }
  close() { this.closed = true; }
}

function installBrowser(t: TestContext) {
  const originals = { window: globalThis.window, localStorage: globalThis.localStorage, WebSocket: globalThis.WebSocket };
  const values = new Map<string, string>();
  const sockets: TestSocket[] = [];
  Object.defineProperties(globalThis, {
    window: { configurable: true, value: Object.assign(new EventTarget(), { location: { protocol: "https:", host: "caper.test" } }) },
    localStorage: { configurable: true, value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    } },
    WebSocket: { configurable: true, value: class extends TestSocket {
      constructor(url: string) { super(url); sockets.push(this); }
    } },
  });
  const gateway = new AppGateway((url) => {
    const socket = new TestSocket(url);
    sockets.push(socket);
    return socket;
  }, () => 0);
  setAppGatewayForTests(gateway);
  t.after(() => {
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
  t.mock.method(globalThis.crypto, "randomUUID", () => "00000000-0000-4000-8000-000000000001");
  const sentBodies: string[] = [];
  let sendAttempts = 0;
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    const url = String(input);
    if (url === "/api/chat/session") return Response.json({ token: "opaque-token", author: { id: "guest", name: "Test Guest", isGuest: true } });
    if (url === "/api/chat/general") return Response.json({
      space: { id: "space", name: "Caper" }, channel: { id: "general", name: "General" }, messages: [], cursor: "0", hasMore: false,
    });
    assert.equal(url, "/api/chat/channels/general/messages");
    assert.equal(new Headers(init?.headers).get("x-caper-chat-token"), "opaque-token");
    sentBodies.push(String(init?.body));
    if (sendAttempts++ === 0) return Response.json({ error: "temporary" }, { status: 503 });
    const body = JSON.parse(String(init?.body)) as { clientMessageId: string; text: string };
    const message: ChatMessage = {
      id: "message-1", channelId: "general", seq: "1", author: { id: "guest", name: "Test Guest", isGuest: true },
      content: { version: 1, type: "text", text: body.text }, createdAt: "2026-09-21T12:00:00Z", clientMessageId: body.clientMessageId,
    };
    return Response.json(message);
  });

  let state!: ChatViewState;
  const client = new ChatClient((next) => { state = next; });
  t.after(() => client.stop());
  client.start();
  client.identify("Test Guest");
  await tick(); await tick();

  assert.equal(await client.send("original text"), false);
  assert.ok(state.pendingSend);
  assert.equal(await client.send("edited draft"), true);
  assert.deepEqual(sentBodies.map((body) => JSON.parse(body)), [
    { clientMessageId: "00000000-0000-4000-8000-000000000001", text: "original text" },
    { clientMessageId: "00000000-0000-4000-8000-000000000001", text: "original text" },
  ]);
  assert.equal(state.messages.length, 1);
});

test("signed-in startup does not reuse another account's capability with the same display name", async (t) => {
  installBrowser(t);
  localStorage.setItem("caper.chat.session", JSON.stringify({
    token: "old-account-token", author: { id: "old-account", name: "Shared Name", isGuest: false },
  }));
  let sessions = 0;
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    if (String(input) === "/api/chat/session") {
      sessions++;
      return Response.json({ token: "current-account-token", author: { id: "current-account", name: "Shared Name", isGuest: false } });
    }
    assert.equal(String(input), "/api/chat/general");
    return Response.json({
      space: { id: "space", name: "Caper" }, channel: { id: "general", name: "General" }, messages: [], cursor: "0", hasMore: false,
    });
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => { state = next; });
  t.after(() => client.stop());
  client.start();
  client.identify("Shared Name", true);
  await tick(); await tick();
  assert.equal(sessions, 1);
  assert.equal(state.author?.id, "current-account");
  assert.equal(JSON.parse(localStorage.getItem("caper.chat.session")!).token, "current-account-token");
});

test("public history loads before identity and remains visible while the send session resolves", async (t) => {
  const sockets = installBrowser(t);
  const message: ChatMessage = {
    id: "existing", channelId: "general", seq: "7", author: { id: "other", name: "Other Guest", isGuest: true },
    content: { version: 1, type: "text", text: "Already here" }, createdAt: "2026-09-21T12:00:00Z", clientMessageId: "old-command",
  };
  const requests: string[] = [];
  let finishSession!: (response: Response) => void;
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    const path = String(input);
    requests.push(path);
    if (path === "/api/chat/session") return new Promise<Response>((resolve) => { finishSession = resolve; });
    assert.equal(path, "/api/chat/general");
    return Response.json({
      space: { id: "space", name: "Caper" }, channel: { id: "general", name: "General" }, messages: [message], cursor: "7", hasMore: false,
    });
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => { state = next; });
  t.after(() => client.stop());
  client.start();
  await tick(); await tick();
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
  await tick(); await tick();
  assert.deepEqual(state.author, { id: "new", name: "New Guest", isGuest: true });
  assert.equal(state.typingAuthors.length, 0, "late identity removes own typing from another tab immediately");
  assert.deepEqual(state.messages, [message]);
  assert.deepEqual(requests, ["/api/chat/general", "/api/chat/session"]);
});

interface SendBody { clientMessageId: string; text: string }

function committed(body: SendBody, seq: string): ChatMessage {
  return {
    id: `message-${seq}`, channelId: "general", seq,
    author: { id: "guest", name: "Test Guest", isGuest: true },
    content: { version: 1, type: "text", text: body.text },
    createdAt: "2026-09-21T12:00:00Z", clientMessageId: body.clientMessageId,
  };
}

async function sendingFixture(t: TestContext) {
  const sockets = installBrowser(t);
  const history: ChatMessage[] = [];
  const posts: { body: SendBody; resolve: (response: Response) => void; reject: (error: Error) => void }[] = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    if (String(input) === "/api/chat/session") return Response.json({ token: "opaque", author: { id: "guest", name: "Test Guest", isGuest: true } });
    if (String(input) === "/api/chat/general") return Response.json({
      space: { id: "space", name: "Caper" }, channel: { id: "general", name: "General" },
      messages: history, cursor: history.at(-1)?.seq ?? "0", hasMore: false,
    });
    assert.equal(String(input), "/api/chat/channels/general/messages");
    const body = JSON.parse(String(init?.body)) as SendBody;
    assert.deepEqual(Object.keys(body).sort(), ["clientMessageId", "text"], "local metadata never enters the wire contract");
    return new Promise<Response>((resolve, reject) => posts.push({ body, resolve, reject }));
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => { state = next; });
  t.after(() => client.stop());
  client.start(); client.identify("Test Guest");
  await tick();
  sockets[0].frame({ type: "ready", cursor: "0" });
  return {
    client, sockets, posts, history,
    get typingPosts() { return sockets.flatMap((socket) => socket.commands); },
    get state() { return state; },
  };
}

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
  for (const seq of ["1", "2"]) f.sockets[0].message(committed({ clientMessageId: `other-${seq}`, text: "local text" }, seq));
  f.sockets[0].message(accepted);
  f.sockets[0].message(accepted);
  assert.deepEqual(f.state.messages.map((message) => message.seq), ["1", "2", "3"]);
  assert.deepEqual(f.state.messages[2], accepted);
});

test("WebSocket-first confirmation requires the sender and cannot be undone by a late failed HTTP response", async (t) => {
  const f = await sendingFixture(t);
  const sending = f.client.send("same text");
  const other = committed(f.posts[0].body, "1");
  other.author = { id: "someone-else", name: "Test Guest", isGuest: true };
  f.sockets[0].message(other);
  assert.ok(f.state.pendingSend, "matching text, name and UUID from a different author is not our acknowledgement");
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
  assert.deepEqual(f.state.messages.map((message) => message.seq), ["1", "2", "3"]);
});

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
  return { type: "typing.updated", channelId: "general", author: { id, name: "Shared Name", isGuest: true }, revision, typing };
}

test("typing is opt-in, author-deduplicated, expires independently, and never advances replay", async (t) => {
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1_000 });
  const f = await sendingFixture(t);
  assert.equal(chatSubscription(f.sockets[0]).channelId, "general");
  assert.equal(new URL(f.sockets[0].url).search, "", "typing is multiplexed rather than enabled through URL credentials");
  f.sockets[0].frame(typingEvent("guest"));
  f.sockets[0].frame({ ...typingEvent("wrong-channel"), channelId: "private" });
  assert.equal(f.state.typingAuthors.length, 0);
  f.sockets[0].frame(typingEvent("a"));
  f.sockets[0].frame(typingEvent("a"));
  t.mock.timers.tick(4_000);
  f.sockets[0].frame(typingEvent("b"));
  f.sockets[0].frame(typingEvent("a"));
  assert.deepEqual(f.state.typingAuthors.map((author) => author.id), ["a", "b"], "same names are not the same identity");
  t.mock.timers.tick(2_000);
  assert.deepEqual(f.state.typingAuthors.map((author) => author.id), ["b"], "duplicates do not prolong a stale indicator");
  assert.deepEqual(f.state.messages, []);
  f.sockets[0].frame({ type: "migrating" });
  await tick();
  assert.equal(chatSubscription(f.sockets[1]).after, "0");
  f.sockets[1].frame(typingEvent("b", "9007199254740993", false));
  f.sockets[0].frame(typingEvent("b"));
  assert.deepEqual(f.state.typingAuthors, [], "an older overlapping start cannot undo a stop, even above JS's safe integer limit");
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
  t.mock.timers.enable({ apis: ["setTimeout", "Date"], now: 1_000 });
  const f = await sendingFixture(t);
  f.client.setTyping(true);
  f.typingPosts[0].resolve(new Response(null, { status: 204 }));
  await tick();
  for (let index = 0; index < 100; index++) f.client.setTyping(true);
  t.mock.timers.tick(249);
  f.client.setTyping(true);
  t.mock.timers.tick(249);
  f.client.setTyping(true);
  assert.equal(f.typingPosts.length, 1);
  t.mock.timers.tick(2);
  f.client.setTyping(true);
  assert.equal(f.typingPosts.length, 2);
  f.typingPosts[1].reject(new TypeError("broker unavailable"));
  await tick();
  t.mock.timers.tick(499);
  assert.deepEqual(f.typingPosts.map((post) => post.active), [true, true]);
  t.mock.timers.tick(1);
  assert.deepEqual(f.typingPosts.map((post) => post.active), [true, true, false]);
  f.typingPosts[2].resolve(new Response(null, { status: 429 }));
  await tick();
  assert.equal(f.state.sendError, undefined);
  assert.equal(f.state.sessionError, undefined);
  f.client.stop();
  f.client.setTyping(true);
  t.mock.timers.tick(10_000);
  assert.equal(f.typingPosts.length, 3);
});

test("a stop waits for its start but an optimistic send never waits for typing", async (t) => {
  const f = await sendingFixture(t);
  f.client.setTyping(true);
  f.client.setTyping(false);
  assert.deepEqual(f.typingPosts.map((post) => post.active), [true]);
  const sending = f.client.send("instant");
  assert.equal(f.state.pendingSend?.text, "instant");
  f.posts[0].resolve(Response.json(committed(f.posts[0].body, "1")));
  assert.equal(await sending, true);
  f.typingPosts[0].resolve(new Response(null, { status: 204 }));
  await tick();
  assert.deepEqual(f.typingPosts.map((post) => post.active), [true, false]);
  f.typingPosts[1].resolve(new Response(null, { status: 204 }));
  await tick();
  f.client.setTyping(false);
  assert.equal(f.typingPosts.length, 2);
});

async function paginationFixture(t: TestContext) {
  const sockets = installBrowser(t);
  const base = 9_007_199_254_740_990n;
  const message = (offset: number) => committed({ clientMessageId: `command-${offset}`, text: `Message ${offset}` }, String(base + BigInt(offset)));
  const history = { messages: [message(4), message(5)], cursor: message(5).seq, hasMore: true };
  const requests: { url: string; resolve: (response: Response) => void }[] = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    const url = String(input);
    if (url === "/api/chat/general") return Response.json({ ...history, space: { id: "space", name: "Caper" }, channel: { id: "general", name: "General" } });
    return new Promise<Response>((resolve) => requests.push({ url, resolve }));
  });
  let state!: ChatViewState;
  const client = new ChatClient((next) => { state = next; });
  t.after(() => client.stop());
  client.start();
  await tick();
  sockets[0].frame({ type: "ready", cursor: history.cursor });
  return { client, sockets, message, history, requests, get state() { return state; } };
}

test("older pages serialize, merge with live delivery, retry the same cursor, and stop at the beginning", async (t) => {
  const f = await paginationFixture(t);
  const loading = f.client.loadOlder();
  await f.client.loadOlder();
  assert.equal(f.requests.length, 1, "scroll callbacks cannot start duplicate requests");
  assert.equal(f.requests[0].url, `/api/chat/channels/general/messages?before=${f.message(4).seq}`);
  f.sockets[0].message(f.message(6));
  f.requests[0].resolve(Response.json({ messages: [f.message(2), f.message(3), f.message(4)], cursor: f.message(5).seq, hasMore: true }));
  await loading;
  assert.deepEqual(f.state.messages, [2, 3, 4, 5, 6].map(f.message), "overlap is deduplicated without losing a concurrent live message");
  f.sockets[0].frame({ type: "migrating" });
  await tick();
  assert.equal(chatSubscription(f.sockets[1]).after, f.message(6).seq, "older history never rewinds the live replay cursor");

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
    f.requests[0].resolve(status === 200
      ? Response.json({ messages: [f.message(1)], cursor: f.message(5).seq, hasMore: false })
      : Response.json({ error: "obsolete failure" }, { status }));
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
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    const url = String(input);
    requests.push(url);
    const channelId = url.includes("alpha") ? "alphaChannel" : "bravoChannel";
    return Response.json({
      space: { id: "space1234567", name: "Studio" },
      channel: { id: channelId, name: channelId === "alphaChannel" ? "alpha" : "bravo" },
      messages: [], cursor: "0", hasMore: false,
    });
  });

  let alpha!: ChatViewState;
  const first = new ChatClient((state) => { alpha = state; }, "alphaChannel");
  first.start();
  await tick();
  assert.equal(requests[0], "/api/chat/channels/alphaChannel/messages");
  const alphaSubscription = chatSubscription(sockets[0]);
  assert.equal(alphaSubscription.channelId, "alphaChannel");
  first.stop();
  assert.equal(sockets[0].closed, false, "the shared socket remains available for the next channel");
  assert.equal(sockets[0].sent.some((frame) => frame.type === "unsubscribe" && frame.id === alphaSubscription.id), true);

  let bravo!: ChatViewState;
  const second = new ChatClient((state) => { bravo = state; }, "bravoChannel");
  t.after(() => second.stop());
  second.start();
  await tick();
  assert.equal(requests[1], "/api/chat/channels/bravoChannel/messages");
  assert.equal(sockets.length, 1);
  assert.equal(chatSubscription(sockets[0]).channelId, "bravoChannel");

  const stale: ChatMessage = {
    id: "stale", channelId: "alphaChannel", seq: "1", author: { id: "peer", name: "Peer", isGuest: false },
    content: { version: 1, type: "text", text: "wrong room" }, createdAt: "2026-09-22T12:00:00Z", clientMessageId: "stale-command",
  };
  sockets[0].message(stale, alphaSubscription.id as string);
  assert.deepEqual(alpha.messages, []);
  assert.deepEqual(bravo.messages, [], "an old channel cannot leak messages into the replacement client");
});

test("prepared history is ready on the first render and starts live replay without another history fetch", async (t) => {
  const sockets = installBrowser(t);
  t.mock.method(globalThis, "fetch", () => { throw new Error("Unexpected duplicate history fetch"); });
  const message: ChatMessage = {
    id: "seven", channelId: "alphaChannel", seq: "7", author: { id: "peer", name: "Peer", isGuest: false },
    content: { version: 1, type: "text", text: "Prepared message" }, createdAt: "2026-09-23T12:00:00Z", clientMessageId: "command-seven",
  };
  const history: GeneralChatHistory = {
    space: { id: "space", name: "Studio" }, channel: { id: "alphaChannel", name: "general" },
    messages: [message], cursor: "7", hasMore: true,
  };
  const firstRender = initialChatView(history);
  assert.equal(firstRender.phase, "ready");
  assert.deepEqual(firstRender.messages, [message]);
  const states: ChatViewState[] = [];
  const client = new ChatClient((state) => states.push(state), "alphaChannel");
  t.after(() => client.stop());
  client.start(history);
  assert.ok(states.every((state) => state.phase === "ready"));
  await tick();
  assert.equal(chatSubscription(sockets[0]).after, "7");
  sockets[0].message({ ...message, id: "eight", seq: "8", clientMessageId: "command-eight", content: { version: 1, type: "text", text: "Arrived during navigation" } });
  assert.deepEqual(states.at(-1)?.messages.map((item) => item.content.text), ["Prepared message", "Arrived during navigation"]);
  client.stop();
  assert.equal(sockets[0].closed, false);
});

test("prefetch rejects a history payload containing another channel's messages", async (t) => {
  t.mock.method(globalThis, "fetch", async () => Response.json({
    space: { id: "space", name: "Studio" }, channel: { id: "alphaChannel", name: "general" },
    messages: [{ id: "one", channelId: "bravoChannel", seq: "1", author: { id: "peer", name: "Peer", isGuest: false },
      content: { version: 1, type: "text", text: "Wrong channel" }, createdAt: "2026-09-23T12:00:00Z", clientMessageId: "command-one" }],
    cursor: "1", hasMore: false,
  }));
  await assert.rejects(loadChatHistory("alphaChannel"), /another channel/);
});

test("a prepared history failure renders once and retries only when requested", async (t) => {
  const sockets = installBrowser(t);
  let requests = 0;
  t.mock.method(globalThis, "fetch", async () => {
    requests++;
    return Response.json({ space: { id: "space", name: "Studio" }, channel: { id: "alphaChannel", name: "general" }, messages: [], cursor: "0", hasMore: false });
  });
  let view = initialChatView(undefined, "Messaging unavailable");
  assert.equal(view.phase, "error");
  const client = new ChatClient((next) => { view = next; }, "alphaChannel");
  t.after(() => client.stop());
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
  f.requests[0].resolve(Response.json({ messages: [f.message(2), f.message(3)], cursor: f.message(5).seq, hasMore: false }));
  await older;
  f.sockets[0].message(f.message(6));
  const snapshot = f.client.snapshotHistory()!;
  f.client.stop();
  assert.deepEqual(snapshot.messages, [2, 3, 4, 5, 6].map(f.message));
  assert.equal(snapshot.cursor, f.message(6).seq);
  assert.equal(snapshot.hasMore, false);
  const states: ChatViewState[] = [];
  const returning = new ChatClient((state) => states.push(state), "general");
  t.after(() => returning.stop());
  returning.start(snapshot);
  assert.equal(chatSubscription(f.sockets[0]).after, f.message(6).seq);
  assert.deepEqual(states.at(-1)?.messages, snapshot.messages);
  f.sockets[0].message(f.message(7));
  assert.deepEqual(states.at(-1)?.messages, [2, 3, 4, 5, 6, 7].map(f.message));
  assert.ok(states.every((state) => state.phase === "ready"));
  assert.equal(f.requests.length, 1, "return does not request another history page");
});

test("resync retains visible messages through transient failures but clears them on access denial", async (t) => {
  const f = await paginationFixture(t);
  let finish!: (response: Response) => void;
  t.mock.method(globalThis, "fetch", () => new Promise<Response>((resolve) => { finish = resolve; }));
  f.sockets[0].frame({ type: "resync_required" });
  assert.equal(f.state.phase, "ready");
  assert.deepEqual(f.state.messages, [4, 5].map(f.message));
  finish(Response.json({ error: "Temporary outage" }, { status: 503 }));
  await tick();
  assert.equal(f.state.phase, "ready");
  assert.equal(f.state.error, "Temporary outage");
  assert.deepEqual(f.state.messages, [4, 5].map(f.message));
  f.client.retryLoad();
  finish(Response.json({ ...f.client.snapshotHistory(), messages: [f.message(6)], cursor: f.message(6).seq, hasMore: true }));
  await tick();
  assert.deepEqual(f.state.messages, [4, 5, 6].map(f.message), "a contiguous refresh retains saved pages");
  assert.equal(f.state.error, undefined);
  f.client.retryLoad();
  finish(Response.json({ error: "Access removed" }, { status: 403 }));
  await tick();
  assert.equal(f.state.phase, "error");
  assert.deepEqual(f.state.messages, []);
  assert.equal(f.client.snapshotHistory(), undefined);
});
