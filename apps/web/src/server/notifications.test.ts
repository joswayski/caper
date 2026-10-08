import assert from "node:assert/strict";
import { test, vi } from "vitest";
import {
  SpacesApiError,
  getNotificationSettings,
  setChannelNotifications,
  setDirectNotifications,
  setSpaceNotifications,
  updateNotificationSettings,
} from "../spaces/client.ts";
import { isMuted, muteLabel, muteUntil } from "../spaces/notifications.ts";

// Outside a component the store's hook reads its snapshot directly, and its
// subscribe function is kept so a test can listen the way React would.
let subscribe: (listener: () => void) => () => void = () => () => undefined;
vi.mock("react", () => ({
  useSyncExternalStore: (listen: typeof subscribe, snapshot: () => unknown) => {
    subscribe = listen;
    return snapshot();
  },
}));

const settings = {
  level: "all",
  mobile: "whenInactive",
  overrides: [
    { spaceId: "space0000001", level: "mentions", mutedUntil: null },
    { spaceId: "space0000001", channelId: "chan00000002", level: null, mutedUntil: "forever" },
  ],
};

// Each store test gets a fresh module, so one test's state never leaks into the next.
async function freshStore() {
  vi.resetModules();
  return import("../spaces/notifications.ts");
}

function subscribeTo(store: Awaited<ReturnType<typeof freshStore>>, listener: () => void) {
  store.useNotificationSettings();
  return subscribe(listener);
}

function deferred() {
  let resolve!: (response: Response) => void;
  const promise = new Promise<Response>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test("notification routes send only the changed fields and read unknown levels as mentions", async () => {
  const calls: Array<{ path: string; method?: string; body?: unknown }> = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    const path = String(input);
    calls.push({ path, method: init?.method, body: init?.body ? JSON.parse(String(init.body)) : undefined });
    if (path === "/api/notifications/settings")
      return Response.json({ ...settings, level: "someday", overrides: [{ conversationId: "dm0000000001" }] });
    return Response.json({ ...JSON.parse(String(init?.body)), spaceId: "space0000001", level: "loud" });
  });

  assert.deepEqual(await getNotificationSettings(), {
    level: "mentions",
    mobile: "whenInactive",
    overrides: [{ conversationId: "dm0000000001", level: null, mutedUntil: null }],
  });
  await updateNotificationSettings({ level: "nothing" });
  assert.equal((await setSpaceNotifications("space0000001", { level: null })).level, "mentions");
  await setChannelNotifications("space0000001", "chan00000002", { mutedUntil: "forever" });
  await setDirectNotifications("dm0000000001", { level: "nothing", mutedUntil: null });
  assert.deepEqual(calls, [
    { path: "/api/notifications/settings", method: undefined, body: undefined },
    { path: "/api/notifications/settings", method: "PUT", body: { level: "nothing" } },
    { path: "/api/spaces/space0000001/notifications", method: "PUT", body: { level: null } },
    {
      path: "/api/spaces/space0000001/channels/chan00000002/notifications",
      method: "PUT",
      body: { mutedUntil: "forever" },
    },
    { path: "/api/dms/dm0000000001/notifications", method: "PUT", body: { level: "nothing", mutedUntil: null } },
  ]);
});

test("notification route errors keep the server's message and IDs are checked before sending", async () => {
  vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
    Response.json({ error: "mutedUntil must be forever or a time within the next year" }, { status: 400 }),
  );
  await assert.rejects(setSpaceNotifications("space0000001", { mutedUntil: "2020-01-01T00:00:00Z" }), (error) => {
    assert.ok(error instanceof SpacesApiError);
    assert.equal(error.status, 400);
    assert.equal(error.message, "mutedUntil must be forever or a time within the next year");
    return true;
  });
  await assert.rejects(setDirectNotifications("../account", { level: null }), /Invalid/);
});

test("mute presets send whole UTC seconds, and labels use local time with the date when it isn't today", () => {
  const now = new Date(2026, 9, 8, 9, 30, 15, 250);
  assert.equal(muteUntil(15, now.getTime()), new Date(2026, 9, 8, 9, 45, 15).toISOString().replace(".000Z", "Z"));
  assert.match(muteUntil(60, now.getTime())!, /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$/);
  assert.equal(muteUntil(undefined), "forever");

  const label = (mutedUntil: string | null) => muteLabel(mutedUntil, now, "en-US")?.replace(/\s/g, " ");
  assert.equal(label(new Date(2026, 9, 8, 17, 0).toISOString()), "Muted until 5:00 PM");
  assert.equal(label(new Date(2026, 9, 9, 1, 0).toISOString()), "Muted until Oct 9, 1:00 AM");
  assert.equal(label(new Date(2027, 0, 2, 8, 5).toISOString()), "Muted until Jan 2, 2027, 8:05 AM");
  assert.equal(label("forever"), "Muted");
  assert.equal(label(null), undefined);
  assert.equal(label(new Date(2026, 9, 8, 9, 30).toISOString()), undefined, "an expired mute reads as unmuted");
  assert.equal(isMuted("not a time"), false);
});

test("a failed change reverts to the saved state and rethrows; a success keeps the server's copy", async () => {
  const store = await freshStore();
  const put = deferred();
  vi.spyOn(globalThis, "fetch").mockImplementation(async (_input: string | URL | Request, init?: RequestInit) =>
    init?.method ? put.promise : Response.json(settings),
  );
  await store.refreshNotificationSettings();
  const channel = { spaceId: "space0000001", channelId: "chan00000002" };
  const read = () => store.overrideFor(store.useNotificationSettings(), channel);
  assert.equal(read()?.mutedUntil, "forever");

  const unmute = store.changeChannelNotifications("space0000001", "chan00000002", { mutedUntil: null });
  assert.equal(read(), undefined, "applied before the server answers; an empty override is dropped");
  put.resolve(Response.json({ error: "channel not found" }, { status: 404 }));
  await assert.rejects(unmute, /channel not found/);
  assert.deepEqual(read(), { ...channel, level: null, mutedUntil: "forever" });

  vi.mocked(globalThis.fetch).mockImplementation(async () =>
    Response.json({ ...channel, level: "all", mutedUntil: null }),
  );
  await store.changeChannelNotifications("space0000001", "chan00000002", { level: "nothing" });
  assert.deepEqual(read(), { ...channel, level: "all", mutedUntil: null }, "the server's answer wins");
});

test("the account level is optimistic and reverts on failure", async () => {
  const store = await freshStore();
  const put = deferred();
  vi.spyOn(globalThis, "fetch").mockImplementation(async (_input: string | URL | Request, init?: RequestInit) =>
    init?.method ? put.promise : Response.json(settings),
  );
  const changes: string[] = [];
  const listener = () => changes.push(store.useNotificationSettings().level);
  await store.refreshNotificationSettings();
  assert.equal(store.useNotificationSettings().loaded, true);
  const unsubscribe = subscribeTo(store, listener);

  const change = store.setNotificationLevel("nothing");
  assert.equal(store.useNotificationSettings().level, "nothing");
  put.resolve(new Response("upstream down", { status: 502 }));
  await assert.rejects(change, (error) => error instanceof Error && error.message === "That request did not work.");
  assert.equal(store.useNotificationSettings().level, "all");
  assert.deepEqual(changes, ["nothing", "all"]);
  unsubscribe();
});

test("a failure only undoes fields it still owns, and a slower refresh never undoes a newer change", async () => {
  const store = await freshStore();
  const responses: Array<ReturnType<typeof deferred>> = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => {
    const next = deferred();
    responses.push(next);
    return next.promise;
  });
  const space = { spaceId: "space0000001" };
  const read = () => store.overrideFor(store.useNotificationSettings(), space);

  const refresh = store.refreshNotificationSettings();
  await Promise.resolve();
  const level = store.changeSpaceNotifications("space0000001", { level: "nothing" });
  const mute = store.changeSpaceNotifications("space0000001", { mutedUntil: "forever" });
  assert.deepEqual(read(), { ...space, level: "nothing", mutedUntil: "forever" });
  await new Promise((resolve) => setImmediate(resolve));
  const [loaded, levelSaved, muteSaved] = responses;
  loaded.resolve(Response.json(settings));
  await refresh;
  assert.deepEqual(read(), { ...space, level: "nothing", mutedUntil: "forever" }, "the stale refresh is ignored");

  levelSaved.resolve(Response.json({ error: "space not found" }, { status: 404 }));
  await assert.rejects(level, /space not found/);
  assert.deepEqual(read(), { ...space, level: null, mutedUntil: "forever" }, "the pending mute stays");
  muteSaved.resolve(Response.json({ ...space, level: null, mutedUntil: "forever" }));
  await mute;
  assert.deepEqual(read(), { ...space, level: null, mutedUntil: "forever" });

  const older = store.changeSpaceNotifications("space0000001", { level: "all" });
  const newer = store.changeSpaceNotifications("space0000001", { level: "mentions" });
  const [olderSaved, newerSaved] = responses.slice(3);
  olderSaved.resolve(Response.json({ error: "space not found" }, { status: 404 }));
  await assert.rejects(older);
  assert.equal(read()?.level, "mentions", "an older failure leaves the newer choice alone");
  newerSaved.resolve(Response.json({ ...space, level: "mentions", mutedUntil: "forever" }));
  await newer;
  assert.equal(read()?.level, "mentions");
});

test("a timed mute ends on its own, as it does on the server", async () => {
  vi.useFakeTimers({ now: new Date("2026-10-08T12:00:00Z") });
  const store = await freshStore();
  vi.spyOn(globalThis, "fetch").mockImplementation(async () =>
    Response.json({
      level: "all",
      mobile: "whenInactive",
      overrides: [{ conversationId: "dm0000000001", level: "nothing", mutedUntil: "2026-10-08T12:15:00Z" }],
    }),
  );
  await store.refreshNotificationSettings();
  const read = () => store.overrideFor(store.useNotificationSettings(), { conversationId: "dm0000000001" });
  assert.equal(read()?.mutedUntil, "2026-10-08T12:15:00Z");
  vi.advanceTimersByTime(15 * 60_000 + 1_000);
  assert.deepEqual(read(), { conversationId: "dm0000000001", level: "nothing", mutedUntil: null });
});
