import assert from "node:assert/strict";
import { test } from "vitest";
import { parseDiffFromFile } from "@pierre/diffs";
import { ChatTimeline } from "../chat/timeline.ts";
import { isChatEditEvent, isChatMessage, type ChatEditEvent, type ChatMessage } from "../chat/types.ts";

test("code diffs retain full before/after Unicode text and separate changes around unchanged lines", () => {
  const before = "Meet Friday 🙂\nKeep this unchanged\nAt 9";
  const after = "Meet Saturday 🚀\nKeep this unchanged\nAt 11";
  const diff = parseDiffFromFile({ name: "message.txt", contents: before }, { name: "message.txt", contents: after });
  assert.equal(diff.deletionLines.join(""), before);
  assert.equal(diff.additionLines.join(""), after);
  const chunks = diff.hunks.flatMap((hunk) => hunk.hunkContent);
  assert.equal(chunks.filter((chunk) => chunk.type === "change").length, 2);
  assert.ok(
    chunks.some(
      (chunk) => chunk.type === "context" && diff.additionLines[chunk.additionLineIndex] === "Keep this unchanged\n",
    ),
  );
});

const root: ChatMessage = {
  id: "root",
  channelId: "room",
  seq: "1",
  clientMessageId: "send-root",
  createdAt: "2026-10-06T12:00:00Z",
  author: { id: "author", name: "Author", isGuest: false },
  content: { version: 1, type: "text", text: "Meet Friday" },
};
const edit = (message: ChatMessage, revision: number, seq: string, text: string): ChatEditEvent => ({
  type: "message.edited",
  schemaVersion: 1,
  channelId: "room",
  seq,
  message: {
    ...message,
    revision,
    editSeq: seq,
    editedAt: "2026-10-06T13:00:00Z",
    content: { version: 1, type: "text", text },
  },
});

test("edit acknowledgements preserve message identity, independent metadata and replay cursor", () => {
  const timeline = new ChatTimeline();
  const pinned = {
    ...root,
    reactionSeq: "8",
    reactions: [{ emoji: "🚀", authorIds: ["peer"] }],
    pinSeq: "9",
    pin: { author: root.author, createdAt: root.createdAt },
    thread: { replyCount: 3, participants: [root.author], seq: "7" },
  };
  timeline.reset([pinned], "9", [pinned]);
  const before = timeline.messages;
  const update = edit(root, 2, "10", "Meet Saturday");
  timeline.mergeEdit(update.message);
  assert.equal(timeline.cursor, "9");
  assert.equal(timeline.messages.length, 1);
  assert.equal(before[0].content.text, "Meet Friday", "old render snapshots are immutable");
  assert.deepEqual(timeline.messages[0], {
    ...pinned,
    content: update.message.content,
    revision: 2,
    editSeq: "10",
    editedAt: update.message.editedAt,
  });
  assert.equal(timeline.pinnedMessages[0].content.text, "Meet Saturday");
  timeline.applyEvent(update);
  assert.equal(timeline.cursor, "10");
  timeline.prepend([root]);
  timeline.mergePin({
    type: "message.pin",
    schemaVersion: 1,
    channelId: "room",
    seq: "11",
    message: { ...pinned, pinSeq: "11" },
  });
  assert.equal(timeline.messages[0].revision, 2, "late history/pin payloads never revert content");
  assert.equal(timeline.pinnedMessages[0].revision, 2);
  assert.equal(timeline.cursor, "10", "pin ack also remains cursor-neutral");
});

test("edits of unloaded roots/replies do not insert rows and hydrate late pages", () => {
  const timeline = new ChatTimeline();
  const hidden = { ...root, id: "reply", seq: "2", threadRootId: root.id, broadcast: false };
  timeline.reset([], "2");
  assert.equal(timeline.applyEvent(edit(hidden, 2, "3", "Corrected hidden reply")), "applied");
  assert.equal(timeline.messages.length, 0);
  timeline.prepend([hidden]);
  assert.equal(timeline.messages[0].content.text, "Corrected hidden reply");
  assert.equal(timeline.messages[0].threadRootId, root.id);
  assert.equal(timeline.messages[0].broadcast, false);
  timeline.reset([], "0");
  timeline.prepend([hidden]);
  assert.equal(timeline.messages[0].revision, undefined, "reset on access/account loss clears mutation cache");
});

test("edit event ordering uses editSeq, not creation seq, and rejects stale versions", () => {
  const timeline = new ChatTimeline();
  timeline.reset([root], "1");
  const version3 = edit(root, 3, "3", "Version three");
  assert.equal(timeline.applyEvent(version3), "buffered");
  assert.equal(timeline.messages[0].content.text, "Meet Friday");
  timeline.applyEvent(edit(root, 2, "2", "Version two"));
  assert.equal(timeline.cursor, "3");
  assert.equal(timeline.messages[0].content.text, "Version three");
  timeline.mergeEdit(edit(root, 2, "2", "Version two").message);
  assert.equal(timeline.messages[0].revision, 3);
  timeline.reset([edit(root, 2, "2", "Version two").message, ...timeline.messages], "2");
  assert.equal(timeline.messages[0].revision, 3, "newer HTTP snapshot survives an older captured history");
  assert.equal(timeline.cursor, "2");
});

test("unchanged and unloaded edits retain the sorted render snapshot", () => {
  const timeline = new ChatTimeline();
  timeline.reset([root], "1");
  const version2 = edit(root, 2, "2", "Saturday");
  timeline.mergeEdit(version2.message);
  const rendered = timeline.messages;
  assert.equal(timeline.applyEvent(version2), "applied");
  assert.equal(timeline.messages, rendered, "confirming an HTTP edit must not re-sort history");
  assert.equal(timeline.cursor, "2", "an unchanged snapshot still advances replay");
  timeline.prepend([version2.message, root]);
  assert.equal(timeline.messages, rendered, "overlapping history must not re-sort unchanged rows");
  const hidden = { ...root, id: "hidden", seq: "3" };
  timeline.mergeEdit(edit(hidden, 2, "4", "Hidden correction").message);
  assert.equal(timeline.messages, rendered, "an unloaded edit must not invalidate visible history");
  timeline.mergeEdit(edit(root, 3, "5", "Sunday").message);
  assert.notEqual(timeline.messages, rendered);
  assert.equal(timeline.messages[0].content.text, "Sunday");
  assert.equal(rendered[0].content.text, "Saturday", "previous render snapshots stay immutable");
  timeline.mergeEdit(version2.message);
  const latest = timeline.messages;
  timeline.mergeEdit(version2.message);
  assert.equal(timeline.messages, latest, "stale edits must not re-sort history");
  timeline.prepend([hidden]);
  assert.equal(timeline.messages[1].content.text, "Hidden correction", "unloaded edits are still retained");
});

test("edit validators require matching channel/event revision and tolerate legacy messages", () => {
  assert.ok(isChatMessage(root));
  const event = edit(root, 2, "7", "Changed");
  assert.ok(isChatEditEvent(event));
  for (const invalid of [
    { ...event, seq: root.seq },
    { ...event, channelId: "elsewhere" },
    { ...event, schemaVersion: 2 },
    { ...event, message: { ...event.message, revision: 1 } },
    { ...event, message: { ...event.message, revision: 2.5 } },
    { ...event, message: { ...event.message, editSeq: undefined } },
  ])
    assert.equal(isChatEditEvent(invalid), false);
});

test("unloaded edit overflow requests resync instead of silently discarding a stale-page overlay", () => {
  const timeline = new ChatTimeline();
  timeline.reset([], "1");
  for (let index = 0; index < 256; index++) {
    assert.equal(
      timeline.applyEvent(edit({ ...root, id: `unloaded-${index}` }, 2, String(index + 2), "corrected")),
      "applied",
    );
  }
  assert.equal(timeline.applyEvent(edit({ ...root, id: "overflow" }, 2, "258", "corrected")), "overflow");
  assert.equal(timeline.messages.length, 0);
  timeline.prepend([{ ...root, id: "unloaded-0" }]);
  assert.equal(
    timeline.messages[0].content.text,
    "corrected",
    "overflow must not silently revert the oldest cached edit",
  );

  const loaded = Array.from({ length: 257 }, (_, index) => ({ ...root, id: `loaded-${index}` }));
  const visible = new ChatTimeline();
  visible.reset(loaded, "1");
  loaded.forEach((message, index) =>
    assert.equal(visible.applyEvent(edit(message, 2, String(index + 2), "corrected")), "applied"),
  );
  visible.mergePin({
    type: "message.pin",
    schemaVersion: 1,
    channelId: "room",
    seq: "259",
    message: { ...loaded[0], pinSeq: "259", pin: { author: root.author, createdAt: root.createdAt } },
  });
  assert.equal(
    visible.pinnedMessages[0].content.text,
    "corrected",
    "loaded edits survive old pin payloads beyond the unseen-cache limit",
  );
});
