import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { ChatClient, type ChatViewState } from "../chat/client.ts";
import type { ChatEvent, ChatMessage } from "../chat/types.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

class TestSocket extends EventTarget {
  url: string;
  constructor(url: string) { super(); this.url = url; }
  frame(event: ChatEvent) { this.dispatchEvent(new MessageEvent("message", { data: JSON.stringify(event) })); }
  message(message: ChatMessage) { this.frame({ type: "message.created", channelId: message.channelId, seq: message.seq, message }); }
  close() {}
}

function installBrowser(t: TestContext) {
  const originals = { window: globalThis.window, localStorage: globalThis.localStorage, WebSocket: globalThis.WebSocket };
  const values = new Map<string, string>();
  const sockets: TestSocket[] = [];
  Object.defineProperties(globalThis, {
    window: { configurable: true, value: { location: { protocol: "https:", host: "caper.test" } } },
    localStorage: { configurable: true, value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    } },
    WebSocket: { configurable: true, value: class extends TestSocket {
      constructor(url: string) { super(url); sockets.push(this); }
    } },
  });
  t.after(() => {
    Object.defineProperties(globalThis, {
      window: { configurable: true, value: originals.window },
      localStorage: { configurable: true, value: originals.localStorage },
      WebSocket: { configurable: true, value: originals.WebSocket },
    });
  });
  return sockets;
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
  installBrowser(t);
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
  finishSession(Response.json({ token: "new-capability", author: { id: "new", name: "New Guest", isGuest: true } }));
  await tick(); await tick();
  assert.deepEqual(state.author, { id: "new", name: "New Guest", isGuest: true });
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
  return { client, sockets, posts, history, get state() { return state; } };
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
  assert.equal(new URL(f.sockets[1].url).searchParams.get("after"), "0");
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
