import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import ThreadPanel from "./ThreadPanel.tsx";
import { initialChatView, type ThreadViewState } from "./client.ts";
import type { ChatMessage } from "./types.ts";

const root: ChatMessage = {
  id: "root",
  channelId: "channel",
  seq: "1",
  clientMessageId: "root-client",
  author: { id: "author", name: "Fixture Author", isGuest: false },
  content: { version: 1, type: "text", text: "Thread parent" },
  createdAt: "2026-10-08T07:00:00Z",
};

function render(messages: ChatMessage[], thread: Partial<ThreadViewState> = {}) {
  return renderToStaticMarkup(
    <ThreadPanel
      state={{
        ...initialChatView(),
        phase: "ready",
        messages,
        thread: { rootId: root.id, loading: false, loadingOlder: false, hasMore: false, ...thread },
      }}
      channelName="general"
      readOnly={false}
      renderMessage={(_index, message) => <p>{message.content.text}</p>}
      onClose={() => {}}
    />,
  );
}

test("empty threads show the parent and empty state without a zero-reply divider", () => {
  const markup = render([root]);
  expect(markup).toContain("Thread parent");
  expect(markup).toContain("No replies yet. Start the thread.");
  expect(markup).not.toContain("chat-thread-divider");
  expect(markup).not.toContain("0 replies");
});

test.each([
  [1, "1 reply"],
  [3, "3 replies"],
])("populated threads keep the full %i-reply count, even with partial history", (replyCount, label) => {
  const markup = render(
    [
      { ...root, thread: { replyCount, participants: [root.author], seq: "4" } },
      {
        ...root,
        id: "reply",
        clientMessageId: "reply-client",
        seq: "4",
        threadRootId: root.id,
        content: { version: 1, type: "text", text: "Latest reply" },
      },
    ],
    { hasMore: replyCount > 1 },
  );
  expect(markup).toContain(`<div class="chat-thread-divider">${label}</div>`);
  expect(markup).toContain("Latest reply");
  expect(markup).not.toContain("No replies yet.");
});

test.each([{ loading: true }, { error: "Thread could not be loaded." }])(
  "unavailable thread history does not show a zero count or an empty state: %j",
  (thread) => {
    const markup = render([root], thread);
    expect(markup).toContain(thread.loading ? "Loading thread…" : thread.error!);
    expect(markup).not.toContain("chat-thread-divider");
    expect(markup).not.toContain("No replies yet.");
  },
);
