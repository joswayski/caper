import assert from "node:assert/strict";
import { test, vi } from "vitest";
import { blockedLabel, blockedRuns } from "../chat/blocked.ts";
import {
  SpacesApiError,
  acceptDirectRequest,
  blockAccount,
  createDirectConversation,
  declineDirectRequest,
  directControls,
  directFeedback,
  directStatus,
  directTag,
  directUnread,
  getDirectPrivacy,
  listBlocks,
  listDirectConversations,
  setDirectPrivacy,
  unblockAccount,
  type DirectConversation,
} from "../spaces/client.ts";

const conversation = (status?: DirectConversation["status"]): DirectConversation => ({
  id: "dm0000000003",
  peer: { id: "stranger0001", username: "jordan", displayName: "Jordan" },
  lastSeq: "2",
  readSeq: "0",
  status,
});

test("requests never count as unread and older servers read as accepted", () => {
  assert.equal(directStatus(conversation()), "accepted");
  assert.equal(directUnread(conversation()), true);
  assert.equal(directUnread(conversation("outgoing")), true);
  assert.equal(directUnread(conversation("incoming")), false);
});

test("feedback conversations decode tolerantly and older servers read as ordinary DMs", async () => {
  const team = { id: "teamaccount1", username: "caper", displayName: "Caper", avatarId: 1 };
  vi.spyOn(globalThis, "fetch").mockResolvedValue(
    Response.json({
      conversations: [
        { ...conversation("accepted"), id: "dm0000000004", peer: team, feedback: true, blocked: false },
        { ...conversation("accepted"), feedback: false },
        conversation(),
        { ...conversation(), feedback: "yes" },
      ],
    }),
  );
  const { conversations } = await listDirectConversations();
  assert.deepEqual(conversations.map(directFeedback), [true, false, false, false]);
  assert.equal(conversations[0].peer.username, "caper", "the team's username comes from the server");
});

test("DM rows tag feedback conversations ahead of the local block list and requests", () => {
  const feedback = { ...conversation(), feedback: true };
  assert.equal(directTag(feedback, false), "Feedback");
  assert.equal(directTag(feedback, true), "Feedback", "a team member's personal block doesn't relabel it");
  assert.equal(directTag({ ...conversation("outgoing"), feedback: true }, false), "Feedback");
  assert.equal(directTag(conversation(), true), "Blocked");
  assert.equal(directTag(conversation("outgoing"), false), "Request sent");
  assert.equal(directTag(conversation("accepted"), false), undefined);
});

test("feedback conversations offer no Block, request or blocked bar and never lock the composer", () => {
  const none = { actions: undefined, locked: false, waiting: false, blockAuthors: false };
  for (const status of [undefined, "accepted", "outgoing", "incoming"] as const) {
    for (const blocked of [false, true])
      assert.deepEqual(directControls({ ...conversation(status), feedback: true }, "member000001", blocked), none);
  }
  assert.deepEqual(directControls(conversation(), "member000001", false), {
    actions: "block",
    locked: false,
    waiting: false,
    blockAuthors: true,
  });
  assert.deepEqual(directControls(conversation("outgoing"), "member000001", false), {
    actions: "block",
    locked: false,
    waiting: true,
    blockAuthors: true,
  });
  assert.equal(directControls(conversation(), "member000001", true).actions, "unblock");
  assert.equal(directControls(conversation(), "member000001", true).locked, true);
  assert.equal(directControls(conversation("incoming"), "member000001", false).actions, "request");
  assert.equal(directControls(conversation("incoming"), "member000001", false).locked, true);
  assert.deepEqual(directControls(conversation(), "stranger0001", false), {
    actions: undefined,
    locked: false,
    waiting: false,
    blockAuthors: true,
  });
});

test("request, block and privacy calls use the API paths and readable refusals", async () => {
  const calls: string[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input: string | URL | Request, init?: RequestInit) => {
    const path = String(input);
    calls.push(`${init?.method ?? "GET"} ${path}${init?.body ? ` ${String(init.body)}` : ""}`);
    if (path === "/api/dms")
      return Response.json(
        { error: "this person isn't accepting direct messages", code: "dm_not_accepted" },
        { status: 403 },
      );
    if (path.endsWith("/accept")) return Response.json({ ...conversation("accepted") });
    if (path === "/api/blocks")
      return Response.json({ blocks: [{ id: "member000001", username: "maya", displayName: "Maya" }] });
    if (path === "/api/blocks/teamaccount1")
      return Response.json({ error: "the Caper team can't be blocked", code: "team_account" }, { status: 400 });
    if (path === "/api/account/privacy")
      return Response.json({ directMessages: init?.body ? JSON.parse(String(init.body)).directMessages : "anyone" });
    return new Response(null, { status: 204 });
  });
  await assert.rejects(
    createDirectConversation("jordan"),
    (error) =>
      error instanceof SpacesApiError &&
      error.status === 403 &&
      error.message === "This person isn’t accepting direct messages.",
  );
  assert.equal((await acceptDirectRequest("dm0000000003")).status, "accepted");
  await declineDirectRequest("dm0000000003");
  assert.equal((await listBlocks()).blocks[0].username, "maya");
  await blockAccount("member000001");
  await assert.rejects(
    blockAccount("teamaccount1"),
    (error) =>
      error instanceof SpacesApiError && error.status === 400 && error.message === "The Caper team can’t be blocked.",
  );
  await unblockAccount("member000001");
  assert.deepEqual(await getDirectPrivacy(), { directMessages: "anyone" });
  assert.deepEqual(await setDirectPrivacy("spaces"), { directMessages: "spaces" });
  assert.throws(() => blockAccount("../me"), /Invalid/);
  assert.deepEqual(calls, [
    'POST /api/dms {"username":"jordan"}',
    "POST /api/dms/dm0000000003/accept",
    "POST /api/dms/dm0000000003/decline",
    "GET /api/blocks",
    "PUT /api/blocks/member000001",
    "PUT /api/blocks/teamaccount1",
    "DELETE /api/blocks/member000001",
    "GET /api/account/privacy",
    'PUT /api/account/privacy {"directMessages":"spaces"}',
  ]);
});

test("consecutive blocked messages collapse into runs; guests and you never do", () => {
  const message = (key: string, id: string, isGuest = false) => ({ clientMessageId: key, author: { id, isGuest } });
  const list = [
    message("1", "maya"),
    message("2", "maya"),
    message("3", "alex"),
    message("4", "maya"),
    message("5", "maya", true),
    message("6", "me"),
    message("7", "jordan"),
    { clientMessageId: "8" },
  ];
  const runs = blockedRuns(list, new Set(["maya", "jordan", "me"]), "me");
  assert.deepEqual(
    [...runs],
    [
      ["1", { first: "1", count: 2 }],
      ["2", { first: "1", count: 2 }],
      ["4", { first: "4", count: 1 }],
      ["7", { first: "7", count: 1 }],
    ],
  );
  assert.equal(blockedRuns(list, new Set(), "me").size, 0);
  assert.equal(blockedLabel(1), "1 blocked message");
  assert.equal(blockedLabel(2), "2 blocked messages");
});
