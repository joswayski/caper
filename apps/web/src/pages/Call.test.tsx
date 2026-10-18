import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import type { GeneralChatHistory } from "../chat/types";
import { Route as SpacesRoute } from "../routes/spaces";
import Call from "./Call";
import { STRIP_FACES, callCardFaces, liveNames } from "../media/live-now";

test("a direct spaces visit renders a visible server fallback before account data or hydration", () => {
  const Pending = SpacesRoute.options.pendingComponent!;
  const markup = renderToStaticMarkup(<Pending />);
  expect(markup).toContain('aria-busy="true"');
  expect(markup).toMatch(/<p[^>]*role="status"[^>]*>Loading your spaces…<\/p>/);
  expect(markup).toContain('aria-label="Loading messages"');
  expect(markup).not.toContain("sr-only");
  expect(markup).not.toContain("<button");
  expect(markup).not.toContain("<textarea");
});

test("the signed-in composer reserves a disabled attachment control before uploads or session are ready", () => {
  const markup = renderToStaticMarkup(
    <Call
      initialAccount={{ id: "me", username: "me", displayName: "Me" }}
      channel={{ id: "channel", name: "general", spaceName: "Space" }}
    />,
  );
  expect(markup).toContain('aria-label="Loading messages"');
  const attach = markup.match(/<button[^>]*aria-label="Attach files"[^>]*>/)?.[0];
  expect(attach).toBeDefined();
  expect(attach).toContain('disabled=""');
  expect(attach).toContain('title="Checking file uploads…"');
});

test("audio controls remain available before joining; joining is a channel row action, not the header", () => {
  const markup = renderToStaticMarkup(<Call />);
  for (const label of ["Mute microphone", "Deafen audio", "Input Options", "Output Options"]) {
    const button = markup.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0];
    expect(button).toBeDefined();
    expect(button).not.toContain('disabled=""');
    expect(button).not.toContain('aria-disabled="true"');
  }
  // Voice lives in the sidebar now: the conversation header carries none of it.
  const header = markup.match(/<header class="chat-heading">(.*?)<\/header>/)?.[1];
  expect(header).toBeDefined();
  expect(header).not.toContain("Join voice");
  expect(header).not.toContain("Leave voice");
  // The channel row keeps a one-click, icon-only join.
  const shortcut = markup.match(/<button[^>]*aria-label="Join voice in #general"[^>]*>(.*?)<\/button>/)?.[1];
  expect(shortcut).toContain("<svg");
  expect(shortcut).not.toContain("Join");
  // Nobody is talking yet, so no row is a live card and there is nothing to leave.
  expect(markup).not.toContain("live-card");
  expect(markup).not.toContain('aria-label="Live now"');
  expect(markup).not.toContain('aria-label="Leave voice"');
});

test.each([
  [[], ""],
  [["Maya"], "Maya"],
  [["Maya", "Alex"], "Maya and Alex"],
  [["Maya", "Alex", "Sam"], "Maya, Alex and Sam"],
  [["Rosa", "Ben", "Mina", "Leo", "Ivy", "Omar", "June", "Kai"], "Rosa, Ben and 6 more"],
])("a live room names its people in one line: %j", (names, line) => {
  expect(liveNames(names)).toBe(line);
});

test("your call card shows up to ten faces, then nine and a +N tile", () => {
  const people = (count: number) => Array.from({ length: count }, (_, index) => index);
  expect(callCardFaces(people(3))).toEqual({ shown: people(3), hidden: [] });
  expect(callCardFaces(people(10))).toEqual({ shown: people(10), hidden: [] });
  const twelve = callCardFaces(people(12));
  expect(twelve.shown).toEqual(people(9));
  expect(twelve.hidden).toEqual([9, 10, 11]);
  expect(callCardFaces(people(50)).shown).toHaveLength(9);
});

test("the phone strip shows the card's faces in its order, folding after five", () => {
  const people = (count: number) => Array.from({ length: count }, (_, index) => index);
  // Up to five, the strip and the card show exactly the same people.
  for (const count of [1, 2, 3, 5])
    expect(callCardFaces(people(count), STRIP_FACES).shown).toEqual(callCardFaces(people(count)).shown);
  const eight = callCardFaces(people(8), STRIP_FACES);
  expect(eight.shown).toEqual(people(4));
  expect(eight.hidden).toEqual([4, 5, 6, 7]);
  expect(callCardFaces(people(8)).shown.slice(0, 4)).toEqual(eight.shown);
});

test("members start open in the initial markup, before responsive hydration", () => {
  const markup = renderToStaticMarkup(<Call membersPanel={() => <aside id="space-member-list">Members</aside>} />);

  expect(markup).toContain('id="space-member-list"');
  expect(markup).toContain('aria-label="Hide member list" aria-expanded="true"');
});

test.each([
  [true, "me", "me", "You can message yourself here to keep notes, reminders, and ideas."],
  [true, "other", "me", "Only you and Same name can read this conversation."],
  [true, undefined, undefined, "Only you and Same name can read this conversation."],
  [false, "me", "me", "Start the conversation in #same name."],
])("empty conversation copy uses account IDs, not display names: %j %j %j", (direct, directPeerId, accountId, copy) => {
  const markup = renderToStaticMarkup(
    <Call
      initialAccount={accountId ? { id: accountId, username: "me", displayName: "Same name" } : undefined}
      channel={{ id: "conversation", name: "Same name", spaceName: "Space", direct, directPeerId }}
      initialHistory={{
        space: { id: "space", name: "Space" },
        channel: { id: "conversation", name: "Same name", direct },
        cursor: "0",
        hasMore: false,
        messages: [],
      }}
    />,
  );
  expect(markup).toContain(copy);
  if (direct && directPeerId === "me" && accountId === "me") {
    expect(markup).not.toContain("No messages yet.");
    expect(markup).not.toContain("Only you and");
  } else {
    expect(markup).toContain("No messages yet.");
    expect(markup).not.toContain("You can message yourself here");
  }
});

test("reaction ownership is correct in the first markup, before a chat session exists", () => {
  const history: GeneralChatHistory = {
    space: { id: "space", name: "Space" },
    channel: { id: "channel", name: "general" },
    cursor: "1",
    hasMore: false,
    messages: [
      {
        id: "message",
        channelId: "channel",
        seq: "1",
        clientMessageId: "client-message",
        author: { id: "other", name: "Same name", isGuest: false },
        content: { version: 1, type: "text", text: "Reaction test" },
        createdAt: "2026-10-08T00:00:00Z",
        reactions: [
          { emoji: "👍", authorIds: ["other", "me"] },
          { emoji: "🚀", authorIds: ["other"] },
        ],
      },
    ],
  };
  for (const id of ["me", "other", undefined]) {
    const markup = renderToStaticMarkup(
      <Call
        initialAccount={id ? { id, username: id, displayName: "Same name" } : undefined}
        initialHistory={history}
      />,
    );
    const chips = [...markup.matchAll(/<button[^>]*class="chat-reaction"[^>]*>/g)].map(([tag]) => tag);
    expect(chips).toHaveLength(2);
    expect(chips[0]).toContain(`aria-pressed="${!!id}"`);
    expect(chips[1]).toContain(`aria-pressed="${id === "other"}"`);
    // Knowing the viewer does not grant a sending capability.
    expect(chips.every((tag) => tag.includes('aria-disabled="true"'))).toBe(true);
    expect(markup.match(/<button[^>]*class="chat-add-reaction"[^>]*>/)?.[0]).toContain('disabled=""');
  }
});
