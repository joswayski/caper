import { renderToStaticMarkup } from "react-dom/server";
import { expect, test, vi } from "vitest";
import type { GeneralChatHistory } from "../chat/types";
import Call from "./Call";

test("members start closed in the initial markup, before responsive hydration", () => {
  const membersPanel = vi.fn(() => <aside id="space-member-list">Members</aside>);
  const markup = renderToStaticMarkup(<Call membersPanel={membersPanel} />);

  expect(membersPanel).not.toHaveBeenCalled();
  expect(markup).not.toContain('id="space-member-list"');
  expect(markup).toContain('aria-label="Show member list" aria-expanded="false"');
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
