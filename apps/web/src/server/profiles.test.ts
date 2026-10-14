import assert from "node:assert/strict";
import { test } from "vitest";
import { getRememberedAccount, rememberSavedAccount, withLatestProfile, type Account } from "../account/client.ts";
import { applyLiveUpdate } from "../account/live-updates.ts";
import {
  getProfiles,
  liveUpdate,
  observeProfiles,
  profileRevision,
  receiveProfile,
  resetProfilesForTests,
  subscribeProfiles,
  withAuthorProfile,
  withMessageAuthors,
  withProfile,
  withSpaceUpdate,
  withSpaceUpdates,
  type ProfileUpdate,
  type SpaceUpdate,
} from "../account/profiles.ts";
import type { ChatAuthor, ChatMessage } from "../chat/types.ts";

const maya: ProfileUpdate = {
  id: "maya",
  username: "maya_l",
  displayName: "Maya Lopez",
  avatarId: 312,
  avatarAsset: "pictureasset000000001",
  avatarUrl: "https://cdn.test/assets/pictureasset000000001?sig=a",
};

test("profile and space events are validated; malformed or unknown ones are ignored", () => {
  assert.deepEqual(liveUpdate({ type: "profile.updated", person: maya }), { type: "profile.updated", person: maya });
  const { avatarAsset: _asset, avatarUrl: _url, ...removed } = maya;
  assert.deepEqual(liveUpdate({ type: "profile.updated", person: removed }), {
    type: "profile.updated",
    person: removed,
  });
  // A URL without its asset, or one that isn't http(s), is never shown.
  assert.deepEqual(liveUpdate({ type: "profile.updated", person: { ...removed, avatarUrl: maya.avatarUrl } }), {
    type: "profile.updated",
    person: removed,
  });
  assert.equal(
    (
      liveUpdate({ type: "profile.updated", person: { ...maya, avatarUrl: "javascript:alert(1)" } }) as {
        person: ProfileUpdate;
      }
    ).person.avatarUrl,
    undefined,
  );
  for (const person of [{ ...maya, displayName: "" }, { ...maya, id: 4 }, { ...maya, avatarId: "7" }, null])
    assert.equal(liveUpdate({ type: "profile.updated", person }), undefined);
  assert.equal(liveUpdate({ type: "presence.updated", person: maya }), undefined);
  assert.equal(liveUpdate("profile.updated"), undefined);
  const space = {
    id: "space1234567",
    name: "Friends",
    ownerId: "owner",
    iconAsset: "icon",
    iconUrl: "https://cdn.test/i",
  };
  assert.deepEqual(liveUpdate({ type: "space.updated", space }), { type: "space.updated", space });
  assert.deepEqual(liveUpdate({ type: "space.updated", space: { id: "s", name: "Friends", ownerId: "o" } }), {
    type: "space.updated",
    space: { id: "s", name: "Friends", ownerId: "o" },
  });
  assert.equal(liveUpdate({ type: "space.updated", space: { id: "s", ownerId: "o" } }), undefined);
});

test("a profile update renames and re-pictures the same account only, and a removal clears the picture", () => {
  const member = { id: "maya", username: "maya", displayName: "Maya", avatarId: 312, owner: false };
  assert.deepEqual(withProfile(member, maya), { ...member, ...maya });
  const pictured = withProfile(member, maya);
  const { avatarAsset: _asset, avatarUrl: _url, ...removed } = maya;
  assert.deepEqual(withProfile(pictured, removed), { ...member, username: "maya_l", displayName: "Maya Lopez" });
  const other = { ...member, id: "sam" };
  assert.equal(withProfile(other, maya), other);
  assert.equal(withProfile(member, undefined), member);
});

test("message authors follow live profiles everywhere they are nested, without touching content", () => {
  const author = (id: string, name: string): ChatAuthor => ({ id, name, isGuest: false, avatarId: 1 });
  const base = (id: string, from: ChatAuthor): ChatMessage => ({
    id,
    channelId: "general",
    seq: "4",
    author: from,
    content: { version: 1, type: "text", text: `${id} text` },
    createdAt: "2026-10-14T12:00:00Z",
    clientMessageId: `${id}-client`,
  });
  const message: ChatMessage = {
    ...base("reply", author("sam", "Sam")),
    replyTo: { id: "root", author: author("maya", "Maya"), text: "quoted", hasAttachments: false },
    pin: { author: author("maya", "Maya"), createdAt: "2026-10-14T12:00:00Z" },
    thread: { replyCount: 2, participants: [author("maya", "Maya"), author("sam", "Sam")], seq: "9" },
    forward: { message: base("original", author("maya", "Maya")), seq: "5" },
  };
  const profiles = new Map([["maya", maya]]);
  const updated = withMessageAuthors(message, (value) => withAuthorProfile(value, profiles.get(value.id)));
  const latest = {
    id: "maya",
    name: "Maya Lopez",
    isGuest: false,
    avatarId: 312,
    avatarAsset: maya.avatarAsset,
    avatarUrl: maya.avatarUrl,
  };
  assert.deepEqual(updated.author, message.author);
  assert.deepEqual(updated.replyTo!.author, latest);
  assert.deepEqual(updated.pin!.author, latest);
  assert.deepEqual(updated.thread!.participants, [latest, message.author]);
  assert.deepEqual(updated.forward!.message!.author, latest);
  assert.deepEqual(
    [updated.content, updated.seq, updated.replyTo!.text, updated.forward!.message!.content],
    [message.content, message.seq, "quoted", message.forward!.message!.content],
  );
  const guest = { id: "maya", name: "Maya", isGuest: true };
  assert.equal(withAuthorProfile(guest, maya), guest, "guests are never accounts");
});

test("a space update renames and re-icons that space and ignores spaces you don't have", () => {
  const space = { id: "space1234567", name: "Studio", ownerId: "o", iconAsset: "old", iconUrl: "https://cdn.test/old" };
  const update: SpaceUpdate = { id: "space1234567", name: "Friends", ownerId: "o" };
  assert.deepEqual(withSpaceUpdate(space, update), { id: "space1234567", name: "Friends", ownerId: "o" });
  assert.deepEqual(withSpaceUpdate(space, { ...update, iconAsset: "new", iconUrl: "https://cdn.test/new" }), {
    ...space,
    name: "Friends",
    iconAsset: "new",
    iconUrl: "https://cdn.test/new",
  });
  const spaces = [space, { id: "other1234567", name: "Garden", ownerId: "o" }];
  assert.deepEqual(
    withSpaceUpdates(spaces, update).map((item) => item.name),
    ["Friends", "Garden"],
  );
  assert.equal(withSpaceUpdates(spaces, { ...update, id: "missing12345" }), spaces);
});

test("live profiles are applied by account ID; your own update goes to the signed-in account", (t) => {
  resetProfilesForTests();
  t.onTestFinished(resetProfilesForTests);
  const self: Account = { id: "self", username: "me", displayName: "Me", avatarId: 4, feedbackTeam: true };
  rememberSavedAccount(self);
  let notified = 0;
  t.onTestFinished(subscribeProfiles(() => notified++));
  const spaces: SpaceUpdate[] = [];
  applyLiveUpdate({ type: "profile.updated", person: maya }, (space) => spaces.push(space));
  assert.deepEqual(getProfiles().get("maya"), maya);
  assert.equal(notified, 1);
  const member = { id: "maya", username: "maya", displayName: "Maya", avatarId: 312 };
  assert.deepEqual(withLatestProfile(member, self, getProfiles()), maya);

  applyLiveUpdate(
    {
      type: "profile.updated",
      person: {
        id: "self",
        username: "me_now",
        displayName: "Me Now",
        avatarId: 4,
        avatarAsset: "mine",
        avatarUrl: "https://cdn.test/mine",
      },
    },
    (space) => spaces.push(space),
  );
  assert.deepEqual(getRememberedAccount(), {
    ...self,
    username: "me_now",
    displayName: "Me Now",
    avatarAsset: "mine",
    avatarUrl: "https://cdn.test/mine",
  });
  assert.equal(getProfiles().has("self"), false, "the saved account is the authority for you");
  applyLiveUpdate({ type: "space.updated", space: { id: "s", name: "Friends", ownerId: "self" } }, (space) =>
    spaces.push(space),
  );
  applyLiveUpdate({ type: "space.updated", space: { id: "s" } }, (space) => spaces.push(space));
  assert.deepEqual(spaces, [{ id: "s", name: "Friends", ownerId: "self" }]);
});

test("a refetch that postdates an update repairs it; one already in flight can't undo it", (t) => {
  resetProfilesForTests();
  t.onTestFinished(resetProfilesForTests);
  const inFlight = profileRevision();
  receiveProfile(maya);
  const stale = { id: "maya", username: "maya", displayName: "Maya", avatarId: 312 };
  observeProfiles([stale], inFlight);
  assert.deepEqual(getProfiles().get("maya"), maya, "an older response loses to the live update");
  // Missed while disconnected (no replay): the next refetch is the repair.
  const fresh = {
    id: "maya",
    username: "maya_l",
    displayName: "Maya L.",
    avatarId: 312,
    avatarAsset: "newer",
    avatarUrl: "https://cdn.test/newer",
  };
  observeProfiles([fresh, { id: "unknown", username: "u", displayName: "U" }], profileRevision());
  assert.deepEqual(getProfiles().get("maya"), fresh);
  assert.equal(getProfiles().has("unknown"), false, "only people with live updates are tracked");
  const before = getProfiles();
  observeProfiles([fresh], profileRevision());
  assert.equal(getProfiles(), before, "an unchanged refetch notifies no one");
});
