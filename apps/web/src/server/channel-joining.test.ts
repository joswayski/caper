import assert from "node:assert/strict";
import { test } from "node:test";
import { createSpaceNavigation } from "../spaces/navigation.ts";
import { acceptChannelInvitation, declineChannelInvitation, joinChannel, leaveChannel } from "../spaces/client.ts";

const space = { id: "space1234567", name: "Studio", ownerId: "owner1234567" };
const preview = { id: "other1234567", spaceId: space.id, name: "design", private: false, joined: false };
const joined = { id: "first1234567", spaceId: space.id, name: "general", private: false, joined: true };
const detail = { space, channels: [preview, joined], members: [], channelInvitations: [] };
const history = (channel: typeof joined) => ({ space, channel, messages: [], cursor: "0", hasMore: false });

test("landing selects a joined channel; explicit preview performs reads without silently joining", async (t) => {
  const paths: string[] = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    assert.ok(!init?.method, "navigation must never issue a membership write");
    const path = String(input); paths.push(path);
    return Response.json(path.startsWith("/api/spaces/") ? detail : history(path.includes(preview.id) ? preview : joined));
  });
  assert.equal((await createSpaceNavigation().take(space.id)).channelId, joined.id);
  assert.equal((await createSpaceNavigation().take(space.id, preview.id)).channelId, preview.id);
  assert.deepEqual(paths, [`/api/spaces/${space.id}`, `/api/chat/channels/${joined.id}/messages`, `/api/spaces/${space.id}`, `/api/chat/channels/${preview.id}/messages`]);
});

test("returning to a space does not restore an unjoined preview's channel or history as the landing conversation", async (t) => {
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    const path = String(input);
    return Response.json(path.startsWith("/api/spaces/") ? detail : history(path.includes(preview.id) ? preview : joined));
  });
  const navigation = createSpaceNavigation();
  navigation.remember(await navigation.take(space.id, preview.id));
  const landing = await navigation.take(space.id);
  assert.equal(landing.channelId, joined.id);
  assert.equal(landing.history?.channel.id, joined.id);
});

test("no joined channels leaves browsing available without choosing or loading an unjoined channel", async (t) => {
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request) => {
    assert.equal(String(input), `/api/spaces/${space.id}`);
    return Response.json({ ...detail, channels: [preview], channelInvitations: [{ channel: { ...preview, private: true }, inviter: { username: "owner", displayName: "Owner" } }] });
  });
  const view = await createSpaceNavigation().take(space.id);
  assert.equal(view.channelId, undefined);
  assert.equal(view.history, undefined);
});

test("join, leave and invitation responses use separate explicit mutation endpoints", async (t) => {
  const writes: Array<{ path: string; method?: string; body?: BodyInit | null }> = [];
  t.mock.method(globalThis, "fetch", async (input: string | URL | Request, init?: RequestInit) => {
    assert.equal(init?.credentials, "same-origin");
    writes.push({ path: String(input), method: init?.method, body: init?.body });
    return init?.method === "DELETE" ? new Response(null, { status: 204 }) : Response.json(joined);
  });
  assert.equal((await joinChannel(space.id, joined.id)).joined, true);
  await leaveChannel(space.id, joined.id);
  await acceptChannelInvitation(space.id, joined.id);
  await declineChannelInvitation(space.id, joined.id);
  const root = `/api/spaces/${space.id}/channels/${joined.id}`;
  assert.deepEqual(writes, [
    { path: `${root}/membership`, method: "POST", body: undefined },
    { path: `${root}/membership`, method: "DELETE", body: undefined },
    { path: `${root}/invitation`, method: "POST", body: undefined },
    { path: `${root}/invitation`, method: "DELETE", body: undefined },
  ]);
});
