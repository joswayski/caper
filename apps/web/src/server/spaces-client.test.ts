import assert from "node:assert/strict";
import { test } from "node:test";
import { createSpaceNavigation } from "../spaces/navigation.ts";
import {
  SpacesApiError,
  channelNameError,
  createChannel,
  createSpace,
  deleteSpace,
  spaceNameError,
} from "../spaces/client.ts";

test("space and channel names enforce the browser-visible API rules", () => {
  assert.equal(spaceNameError("  Studio  "), undefined);
  assert.match(spaceNameError("\u0000")!, /control/);
  assert.match(spaceNameError("x".repeat(81))!, /80/);
  assert.equal(channelNameError("launch-plans"), undefined);
  for (const name of ["Launch-plans", "launch--plans", "launch-2", "launch-"]) {
    assert.match(channelNameError(name)!, /lowercase/);
  }
});

test("CRUD sends only the specified payload and preserves server error details", async (t) => {
  const requests: Array<{ path: string; init?: RequestInit }> = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    requests.push({ path: String(input), init });
    if (String(input) === "/api/spaces") return Response.json({ id: "space1234567", name: "Studio", ownerId: "owner1234567" });
    if (init?.method === "DELETE") return Response.json({ error: "The default space cannot be deleted." }, { status: 409 });
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
  space: spaceDetail.space, channel: { id, name: id }, messages: [], cursor: "7", hasMore: false,
});

test("hover and click share one read, wait for history, and consume the snapshot only once", async (t) => {
  const paths: string[] = [];
  let release!: (response: Response) => void;
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    const path = String(input); paths.push(path);
    if (path === "/api/spaces/space1234567") return Response.json(spaceDetail);
    return new Promise<Response>((resolve) => { release = resolve; });
  });
  const navigation = createSpaceNavigation();
  const hover = navigation.prepare("space1234567", "other1234567");
  assert.equal(navigation.take("space1234567", "other1234567"), hover);
  let settled = false;
  void hover.then(() => { settled = true; });
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

test("expired and invalidated speculation is not reused; genuinely empty spaces do not fetch history", async (t) => {
  let now = 100;
  let requests = 0;
  t.mock.method(Date, "now", () => now);
  t.mock.method(globalThis, "fetch", async () => {
    requests++;
    return Response.json({ ...spaceDetail, channels: [] });
  });
  const navigation = createSpaceNavigation();
  const first = navigation.prepare("space1234567");
  assert.deepEqual(await first, { detail: { ...spaceDetail, channels: [] }, channelId: undefined, history: undefined, historyError: undefined });
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

test("failed speculation retries and rejects history for another space", async (t) => {
  let denied = true;
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    if (denied) return Response.json({ error: "Access removed" }, { status: 404 });
    if (String(input).startsWith("/api/spaces/")) return Response.json(spaceDetail);
    return Response.json({ ...history("first1234567"), space: { id: "wrong1234567", name: "Wrong" } });
  });
  const navigation = createSpaceNavigation();
  await assert.rejects(navigation.prepare("space1234567"), /Access removed/);
  denied = false;
  await assert.rejects(navigation.take("space1234567"), /wrong space/);
});

test("a message outage preserves channel navigation and space management", async (t) => {
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) =>
    String(input).startsWith("/api/spaces/") ? Response.json(spaceDetail)
      : Response.json({ error: "Messaging unavailable" }, { status: 503 }));
  const next = await createSpaceNavigation().take("space1234567");
  assert.deepEqual(next.detail, spaceDetail);
  assert.equal(next.channelId, "first1234567");
  assert.equal(next.history, undefined);
  assert.equal(next.historyError, "Messaging unavailable");
});

test("returning restores each channel snapshot, rechecks access, and forgets revoked data", async (t) => {
  const paths: string[] = [];
  let denied = false;
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    const path = String(input); paths.push(path);
    if (path.startsWith("/api/spaces/")) return denied
      ? Response.json({ error: "Access removed" }, { status: 404 }) : Response.json(spaceDetail);
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
  assert.equal(paths.filter((path) => path.startsWith("/api/chat/")).length, 2, "visited channels resume replay instead of fetching another first page");
  assert.equal(paths.filter((path) => path.startsWith("/api/spaces/")).length, 3);
  assert.equal(navigation.peek("space1234567", "other1234567")?.history?.cursor, "7");
  denied = true;
  await assert.rejects(navigation.take("space1234567", "first1234567"), /Access removed/);
  navigation.rememberHistory(liveSnapshot);
  assert.equal(navigation.peek("space1234567"), undefined, "cleanup must not resurrect revoked snapshots");
});

test("public demo uses real history IDs and shares navigation without account-space requests", async (t) => {
  t.mock.method(globalThis, "fetch", () => { throw new Error("Public demo must not call the membership-only API"); });
  const navigation = createSpaceNavigation();
  const demo = navigation.setDemo(history("demoChannel1"));
  assert.equal(demo.detail.space.demo, true);
  assert.equal(demo.detail.space.id, "space1234567");
  assert.equal(demo.detail.channels[0].private, false);
  assert.deepEqual((await navigation.take("space1234567")).history, history("demoChannel1"));
  navigation.clear();
  assert.equal(navigation.peek("space1234567"), undefined);
});
