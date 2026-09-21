import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { ChatClient, type ChatViewState } from "../chat/client.ts";
import type { ChatMessage } from "../chat/types.ts";

const tick = () => new Promise<void>((resolve) => setImmediate(resolve));

class SilentSocket {
  addEventListener() {}
  close() {}
}

function installBrowser(t: TestContext) {
  const originals = { window: globalThis.window, localStorage: globalThis.localStorage, WebSocket: globalThis.WebSocket };
  const values = new Map<string, string>();
  Object.defineProperties(globalThis, {
    window: { configurable: true, value: { location: { protocol: "https:", host: "caper.test" } } },
    localStorage: { configurable: true, value: {
      getItem: (key: string) => values.get(key) ?? null,
      setItem: (key: string, value: string) => values.set(key, value),
      removeItem: (key: string) => values.delete(key),
    } },
    WebSocket: { configurable: true, value: SilentSocket },
  });
  t.after(() => {
    Object.defineProperties(globalThis, {
      window: { configurable: true, value: originals.window },
      localStorage: { configurable: true, value: originals.localStorage },
      WebSocket: { configurable: true, value: originals.WebSocket },
    });
  });
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
