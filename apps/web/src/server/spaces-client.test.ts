import assert from "node:assert/strict";
import { test, vi } from "vitest";
import {
  createDirectNavigation,
  createSpaceNavigation,
  prepareSpaceLanding,
  rememberSpaceLanding,
} from "../spaces/navigation.ts";
import {
  SpacesApiError,
  channelNameError,
  createChannel,
  createSpace,
  createDirectConversation,
  listDirectConversations,
  readDirectConversation,
  directUnread,
  deleteSpace,
  normalizeChannelName,
  refreshedFeedback,
  spaceNameError,
  type FeedbackReport,
} from "../spaces/client.ts";

test("DM requests are global and unread comparisons preserve large sequence values", async () => {
  const calls: Array<{ path: string; body?: string }> = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    calls.push({ path: String(input), body: init?.body as string | undefined });
    return init?.method === "POST" && String(input).endsWith("/read")
      ? new Response(null, { status: 204 })
      : Response.json({ conversations: [] });
  });
  await listDirectConversations();
  await createDirectConversation(" @Mira ");
  await readDirectConversation("direct000001", "9007199254740993");
  assert.deepEqual(calls, [
    { path: "/api/dms", body: undefined },
    { path: "/api/dms", body: '{"username":"@Mira"}' },
    { path: "/api/dms/direct000001/read", body: '{"seq":"9007199254740993"}' },
  ]);
  const peer = { id: "account00002", username: "mira", displayName: "Mira" };
  assert.equal(
    directUnread({ id: "direct000001", peer, lastSeq: "9007199254740993", readSeq: "9007199254740992" }),
    true,
  );
  assert.equal(
    directUnread({ id: "direct000001", peer, lastSeq: "9007199254740992", readSeq: "9007199254740993" }),
    false,
  );
});

test("space and channel names enforce the browser-visible API rules", () => {
  assert.equal(spaceNameError("  Studio  "), undefined);
  assert.match(spaceNameError("\u0000")!, /control/);
  assert.match(spaceNameError("x".repeat(81))!, /80/);
  assert.equal(channelNameError("launch-plans"), undefined);
  for (const name of ["Launch-plans", "launch--plans", "launch-2", "launch-"]) {
    assert.match(channelNameError(name)!, /lowercase/);
  }
});

test("channel input normalizes pasted names while allowing a word separator during typing", () => {
  assert.equal(normalizeChannelName("  Launch--PLANS 42! & updates"), "launch-plans-updates");
  assert.equal(normalizeChannelName("---123!?"), "");
  assert.equal(normalizeChannelName("launch-"), "launch-");
  assert.equal(normalizeChannelName("launch--p"), "launch-p");
  assert.equal(normalizeChannelName("UPDATES".repeat(12)), "updates".repeat(11) + "upd");
});

test("CRUD sends only the specified payload and preserves server error details", async () => {
  const requests: Array<{ path: string; init?: RequestInit }> = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    requests.push({ path: String(input), init });
    if (String(input) === "/api/spaces")
      return Response.json({ id: "space1234567", name: "Studio", ownerId: "owner1234567" });
    if (init?.method === "DELETE")
      return Response.json({ error: "The default space cannot be deleted." }, { status: 409 });
    return Response.json({ id: "chanl1234567", spaceId: "space1234567", name: "launch-plans", private: true });
  });

  await createSpace("  Studio  ");
  await createChannel("space1234567", "launch-plans", true);
  await assert.rejects(deleteSpace("space1234567"), (error) => {
    assert.ok(error instanceof SpacesApiError);
    assert.equal(error.status, 409);
    assert.equal(error.message, "The default space cannot be deleted.");
    return true;
  });
  assert.deepEqual(JSON.parse(String(requests[0].init?.body)), { name: "Studio" });
  assert.deepEqual(JSON.parse(String(requests[1].init?.body)), { name: "launch-plans", private: true });
  assert.equal(requests[1].path, "/api/spaces/space1234567/channels");
  assert.equal(requests[2].path, "/api/spaces/space1234567");
  assert.ok(requests.every(({ init }) => init?.credentials === "same-origin"));
});

const spaceDetail = {
  space: { id: "space1234567", name: "Studio", ownerId: "owner1234567" },
  channels: [
    { id: "first1234567", spaceId: "space1234567", name: "general", private: false },
    { id: "other1234567", spaceId: "space1234567", name: "private", private: true },
  ],
  members: [],
};
const history = (id: string) => ({
  space: spaceDetail.space,
  channel: { id, name: id },
  messages: [],
  cursor: "7",
  hasMore: false,
});

const directHistory = (id: string) => ({
  ...history(id),
  space: { id: "", name: "Direct messages" },
  channel: { id, name: id, direct: true },
});

test("web landing restores the account's last joined conversation, with explicit links and access taking precedence", async () => {
  const storage = new Map<string, string>();
  vi.stubGlobal("localStorage", {
    getItem: (key: string) => storage.get(key) ?? null,
    setItem: (key: string, value: string) => storage.set(key, value),
  });
  const firstSpace = { id: "earlier12345" };
  let channels = spaceDetail.channels.map((channel) => ({ ...channel, joined: true }));
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    assert.equal(init?.method, undefined, "preparation is read-only");
    const path = String(input);
    return Response.json(
      path.startsWith("/api/spaces/")
        ? { ...spaceDetail, space: { ...spaceDetail.space, id: path.split("/").at(-1)! }, channels }
        : history(path.includes("other") ? "other1234567" : "first1234567"),
    );
  });
  try {
    rememberSpaceLanding("owner", { detail: spaceDetail, channelId: "other1234567", history: history("other1234567") });
    assert.deepEqual(JSON.parse([...storage.values()][0]), { spaceId: "space1234567", channelId: "other1234567" });
    const spaces = [firstSpace, spaceDetail.space];
    assert.deepEqual((await prepareSpaceLanding("owner", spaces, new URLSearchParams())).initialSelection, {
      spaceId: "space1234567",
      channelId: "other1234567",
    });
    assert.deepEqual(
      (await prepareSpaceLanding("owner", spaces, new URLSearchParams("space=space1234567&channel=first1234567")))
        .initialSelection,
      { spaceId: "space1234567", channelId: "first1234567" },
    );

    channels[1].joined = false;
    assert.equal(
      (await prepareSpaceLanding("owner", spaces, new URLSearchParams())).initialView?.channelId,
      "first1234567",
      "a remembered channel the user left must not become the landing conversation",
    );
    channels = channels.filter((channel) => channel.id !== "other1234567");
    assert.equal(
      (await prepareSpaceLanding("owner", spaces, new URLSearchParams())).initialView?.channelId,
      "first1234567",
    );
    const explicit = await prepareSpaceLanding(
      "owner",
      spaces,
      new URLSearchParams("space=space1234567&channel=other1234567"),
    );
    assert.equal(explicit.initialView, undefined, "a stale explicit link must not silently open another conversation");
    assert.match(explicit.initialSpaceError!, /no longer accessible/);

    // Another account, or a removed space, cannot restore the saved space.
    // Use no channels so history cannot accidentally mask a wrong space choice.
    channels = [];
    assert.equal(
      (await prepareSpaceLanding("other", spaces, new URLSearchParams())).initialView?.detail.space.id,
      "earlier12345",
    );
    assert.equal(
      (await prepareSpaceLanding("owner", [firstSpace], new URLSearchParams())).initialView?.detail.space.id,
      "earlier12345",
    );
    assert.equal((await prepareSpaceLanding("owner", [], new URLSearchParams())).initialView, undefined);
    storage.set("caper:space-landing:owner", "not JSON");
    assert.equal(
      (await prepareSpaceLanding("owner", spaces, new URLSearchParams())).initialView?.detail.space.id,
      "earlier12345",
    );
  } finally {
    vi.unstubAllGlobals();
  }
});

test("web landing keeps explicit invitation, direct and feedback destinations without loading a different conversation", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(() => {
    throw new Error("must not load space history");
  });
  for (const query of ["space=invite12345", "dm=direct12345", "feedback=report12345"]) {
    const result = await prepareSpaceLanding("owner", [spaceDetail.space], new URLSearchParams(query));
    assert.equal(result.initialView, undefined);
    assert.equal(result.initialSpaceError, undefined);
    assert.equal(result.initialSelection.spaceId, query.startsWith("space=") ? "invite12345" : "space1234567");
  }
});

test("DM hover/click shares a read and revisits retain each live history and replay cursor", async () => {
  const paths: string[] = [];
  let release!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    assert.equal(init?.method, undefined, "speculation never creates sessions or marks messages read");
    paths.push(String(input));
    return new Promise<Response>((resolve) => {
      release = resolve;
    });
  });
  const navigation = createDirectNavigation();
  navigation.prefetch("direct000001");
  navigation.prefetch("direct000001");
  const first = navigation.take("direct000001", new AbortController().signal);
  assert.deepEqual(paths, ["/api/chat/channels/direct000001/messages"]);
  assert.equal(navigation.peek("direct000001"), undefined, "unfinished history cannot replace the current chat");
  release(Response.json(directHistory("direct000001")));
  navigation.remember("direct000001", await first);
  const second = navigation.take("direct000002", new AbortController().signal);
  release(Response.json(directHistory("direct000002")));
  navigation.remember("direct000002", await second);
  const live = { ...directHistory("direct000001"), cursor: "9007199254740993", hasMore: true };
  navigation.rememberHistory(live);
  navigation.prefetch("direct000001");
  assert.deepEqual((await navigation.take("direct000001", new AbortController().signal)).history, live);
  assert.equal((await navigation.take("direct000002", new AbortController().signal)).history?.cursor, "7");
  assert.equal(paths.length, 2, "visited DMs resume replay without another first-page read");
  navigation.rememberHistory({ ...live, hasNewer: true });
  assert.equal(navigation.peek("direct000001"), undefined, "an older search window cannot seed latest navigation");
  navigation.remember("direct000001", { history: live });
  navigation.forget("direct000001");
  navigation.rememberHistory(live);
  assert.equal(navigation.peek("direct000001"), undefined, "cleanup cannot resurrect revoked data");
  navigation.clear();
  assert.equal(navigation.peek("direct000002"), undefined, "account/page cleanup releases private snapshots");
});

test("DM preparation cancels abandoned reads, preserves explicit load errors, and rejects inaccessible history", async () => {
  const navigation = createDirectNavigation();
  const controller = new AbortController();
  vi.spyOn(globalThis, "fetch").mockImplementation(
    async (_input, init) =>
      new Promise<Response>((_resolve, reject) => {
        init?.signal?.addEventListener("abort", () => reject(init.signal!.reason), { once: true });
      }),
  );
  navigation.prefetch("direct000001");
  const abandoned = navigation.take("direct000001", controller.signal);
  controller.abort();
  await assert.rejects(abandoned, { name: "AbortError" });
  vi.spyOn(globalThis, "fetch").mockResolvedValue(Response.json({ error: "Unavailable" }, { status: 503 }));
  assert.deepEqual(await navigation.take("direct000001", new AbortController().signal), {
    historyError: "Unavailable",
  });
  vi.spyOn(globalThis, "fetch").mockResolvedValue(Response.json({ error: "Access removed" }, { status: 404 }));
  await assert.rejects(navigation.take("direct000001", new AbortController().signal), /Access removed/);
  vi.spyOn(globalThis, "fetch").mockResolvedValue(Response.json(history("direct000001")));
  assert.deepEqual(await navigation.take("direct000001", new AbortController().signal), {
    historyError: "The chat service returned the wrong conversation.",
  });
  let prefetchSignal: AbortSignal | undefined;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (_input, init) => {
    prefetchSignal = init?.signal ?? undefined;
    return new Promise<Response>((_resolve, reject) => {
      prefetchSignal?.addEventListener("abort", () => reject(prefetchSignal!.reason), { once: true });
    });
  });
  navigation.prefetch("direct000002");
  navigation.clear();
  assert.equal(prefetchSignal?.aborted, true, "page cleanup cancels speculative reads");
});

test("hover and click share one read, wait for history, and consume the snapshot only once", async () => {
  const paths: string[] = [];
  let release!: (response: Response) => void;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    const path = String(input);
    paths.push(path);
    if (path === "/api/spaces/space1234567") return Response.json(spaceDetail);
    return new Promise<Response>((resolve) => {
      release = resolve;
    });
  });
  const navigation = createSpaceNavigation();
  const hover = navigation.prepare("space1234567", "other1234567");
  assert.equal(navigation.take("space1234567", "other1234567"), hover);
  let settled = false;
  void hover.then(() => {
    settled = true;
  });
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(settled, false, "a channel list alone must not replace the visible space");
  assert.deepEqual(paths, ["/api/spaces/space1234567", "/api/chat/channels/other1234567/messages"]);
  release(Response.json(history("other1234567")));
  assert.equal((await hover).channelId, "other1234567");
  const again = navigation.take("space1234567", "other1234567");
  assert.notEqual(again, hover);
  await new Promise((resolve) => setImmediate(resolve));
  release(Response.json(history("other1234567")));
  await again;
  assert.equal(paths.length, 4, "revisits must recheck access and current history");
});

test("expired and invalidated speculation is not reused; genuinely empty spaces do not fetch history", async () => {
  let now = 100;
  let requests = 0;
  vi.spyOn(Date, "now").mockImplementation(() => now);
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => {
    requests++;
    return Response.json({ ...spaceDetail, channels: [] });
  });
  const navigation = createSpaceNavigation();
  const first = navigation.prepare("space1234567");
  assert.deepEqual(await first, {
    detail: { ...spaceDetail, channels: [] },
    channelId: undefined,
    history: undefined,
    historyError: undefined,
  });
  now = 5_099;
  assert.equal(navigation.prepare("space1234567"), first);
  now = 5_100;
  const expired = navigation.prepare("space1234567");
  assert.notEqual(expired, first);
  await expired;
  navigation.clear();
  await navigation.take("space1234567");
  assert.equal(requests, 3);
});

test("failed speculation retries and rejects history for another space", async () => {
  let denied = true;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    if (denied) return Response.json({ error: "Access removed" }, { status: 404 });
    if (String(input).startsWith("/api/spaces/")) return Response.json(spaceDetail);
    return Response.json({ ...history("first1234567"), space: { id: "wrong1234567", name: "Wrong" } });
  });
  const navigation = createSpaceNavigation();
  await assert.rejects(navigation.prepare("space1234567"), /Access removed/);
  denied = false;
  await assert.rejects(navigation.take("space1234567"), /wrong space/);
});

test("a message outage preserves channel navigation and space management", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) =>
    String(input).startsWith("/api/spaces/")
      ? Response.json(spaceDetail)
      : Response.json({ error: "Messaging unavailable" }, { status: 503 }),
  );
  const next = await createSpaceNavigation().take("space1234567");
  assert.deepEqual(next.detail, spaceDetail);
  assert.equal(next.channelId, "first1234567");
  assert.equal(next.history, undefined);
  assert.equal(next.historyError, "Messaging unavailable");
});

test("returning restores each channel snapshot, rechecks access, and forgets revoked data", async () => {
  const paths: string[] = [];
  let denied = false;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request) => {
    const path = String(input);
    paths.push(path);
    if (path.startsWith("/api/spaces/"))
      return denied ? Response.json({ error: "Access removed" }, { status: 404 }) : Response.json(spaceDetail);
    return Response.json(history(path.includes("other") ? "other1234567" : "first1234567"));
  });
  const navigation = createSpaceNavigation();
  navigation.remember(await navigation.take("space1234567", "first1234567"));
  navigation.remember(await navigation.take("space1234567", "other1234567"));
  const liveSnapshot = { ...history("first1234567"), cursor: "19", hasMore: true };
  navigation.rememberHistory(liveSnapshot);
  assert.deepEqual(navigation.peek("space1234567", "first1234567")?.history, liveSnapshot);
  const returning = await navigation.take("space1234567", "first1234567");
  assert.deepEqual(returning.history, liveSnapshot);
  assert.equal(
    paths.filter((path) => path.startsWith("/api/chat/")).length,
    2,
    "visited channels resume replay instead of fetching another first page",
  );
  assert.equal(paths.filter((path) => path.startsWith("/api/spaces/")).length, 3);
  assert.equal(navigation.peek("space1234567", "other1234567")?.history?.cursor, "7");
  denied = true;
  await assert.rejects(navigation.take("space1234567", "first1234567"), /Access removed/);
  navigation.rememberHistory(liveSnapshot);
  assert.equal(navigation.peek("space1234567"), undefined, "cleanup must not resurrect revoked snapshots");
});

test("a feedback refresh keeps older pages already loaded with Show more", () => {
  const report = (id: string) => ({ id }) as FeedbackReport;
  const ids = (page: { reports: FeedbackReport[] }) => page.reports.map((item) => item.id);
  const loaded = { reports: ["f", "e", "d", "c", "b"].map(report), cursor: "older" };
  // A new report arrived and "e" was archived: the newest page shifts by one.
  const refreshed = refreshedFeedback(loaded, { reports: ["g", "f", "d"].map(report), cursor: "page" });
  assert.deepEqual(ids(refreshed), ["g", "f", "d", "c", "b"]);
  assert.equal(refreshed.cursor, "older");
  // Only the first page was loaded: the refresh replaces it outright.
  const first = { reports: ["c", "b"].map(report), cursor: "x" };
  assert.deepEqual(refreshedFeedback(first, { reports: ["d", "c", "b"].map(report), cursor: "y" }), {
    reports: ["d", "c", "b"].map(report),
    cursor: "y",
  });
  // A complete newest page, or one sharing nothing with the loaded list, replaces it.
  assert.deepEqual(ids(refreshedFeedback(loaded, { reports: ["g"].map(report), cursor: null })), ["g"]);
  assert.deepEqual(ids(refreshedFeedback(loaded, { reports: ["z"].map(report), cursor: "p" })), ["z"]);
  assert.deepEqual(ids(refreshedFeedback(undefined, { reports: ["a"].map(report), cursor: "p" })), ["a"]);
});
