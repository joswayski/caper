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
