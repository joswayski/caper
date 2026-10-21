// @vitest-environment happy-dom
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { ChatViewState } from "./client.ts";
import type { ChatMessage, GeneralChatHistory } from "./types.ts";

// Every message row renders its text through LinkedText exactly once, so its
// renders count the row's renders, keyed by the message's text.
const fixture = vi.hoisted(() => ({
  renders: new Map<string, number>(),
  clients: [] as { emit: (change: Partial<ChatViewState>) => void }[],
}));

vi.mock("./LinkedText.tsx", () => ({
  default: ({ text }: { text: string }) => {
    fixture.renders.set(text, (fixture.renders.get(text) ?? 0) + 1);
    return text;
  },
}));

// The conversation's network client, replaced by one the test drives.
vi.mock("./client.ts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./client.ts")>();
  class ChatClient {
    private state = actual.initialChatView();
    constructor(private onChange: (state: ChatViewState) => void) {
      fixture.clients.push(this);
    }
    start(history?: GeneralChatHistory) {
      this.emit(actual.initialChatView(history));
    }
    emit(change: Partial<ChatViewState>) {
      this.state = { ...this.state, ...change };
      this.onChange(this.state);
    }
    current() {
      return this.state;
    }
    stop() {}
    setSilencedAuthors() {}
    snapshotHistory() {
      return undefined;
    }
    readCursor() {
      return undefined;
    }
    identify() {}
    async refreshUnreadThreads() {}
    setSounds() {}
    refreshProfiles() {}
    setTyping() {}
    async openThread(rootId: string) {
      this.emit({ thread: { rootId, loading: true, loadingOlder: false, hasMore: false } });
    }
    async prefetchThread() {}
    closeThread() {
      this.emit({ thread: undefined });
    }
  }
  return { ...actual, ChatClient };
});

const { default: Chat } = await import("./Chat.tsx");

const COUNT = 12;
const text = (index: number) => `Fixture message ${index}`;
// An hour apart, so no row groups with its neighbours and a new message
// changes no other row's grouping.
const message = (index: number): ChatMessage => ({
  id: `message-${index}`,
  channelId: "channel",
  seq: String(index),
  clientMessageId: `client-${index}`,
  author: { id: `author-${index % 3}`, name: `Fixture Author ${index % 3}`, isGuest: false },
  content: { version: 1, type: "text", text: text(index) },
  createdAt: new Date(Date.UTC(2026, 9, 19, 0, index)).toISOString(),
});
const history: GeneralChatHistory = {
  space: { id: "space", name: "Fixture space" },
  channel: { id: "channel", name: "general" },
  messages: Array.from({ length: COUNT }, (_, index) => message(index * 60 + 1)),
  hasMore: false,
} as GeneralChatHistory;

// Stands in for Call: every render rebuilds header content, callbacks and the
// mention directory, as a speaking change or a presence tick does there.
function Page({ speaking }: { speaking: string }) {
  return (
    <Chat
      name="Viewer"
      signedIn={false}
      identityReady={false}
      channelId="channel"
      initialHistory={history}
      headerBelow={<p className="fixture-voice">{speaking}</p>}
      headerActions={<button type="button">{speaking}</button>}
      composerBanner={<span>{speaking}</span>}
      onAuthorChange={() => {}}
      onOnlineChange={() => {}}
      mentionDirectory={[{ id: "author-1", username: "fixture1", displayName: "Fixture Author 1" }]}
    />
  );
}

let container: HTMLElement;
let root: Root;
const render = (node: ReactNode) => act(() => root.render(node));
const counts = () => history.messages.map((entry) => fixture.renders.get(text(Number(entry.seq))) ?? 0);

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  vi.spyOn(globalThis, "fetch").mockImplementation(async () => new Response("{}", { status: 404 }));
  fixture.renders.clear();
  fixture.clients.length = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

test("voice, presence, typing and composer updates leave unchanged message rows alone", async () => {
  await render(<Page speaking="Nobody is speaking" />);
  // Let mount effects (hydration, the client's first state) settle.
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
  expect(container.querySelectorAll(".chat-message[data-message-key]")).toHaveLength(COUNT);
  const settled = counts();
  expect(settled.every((count) => count > 0)).toBe(true);

  // Someone starts and stops speaking; a presence tick re-renders the page.
  await render(<Page speaking="Fixture Author 1 is speaking" />);
  await render(<Page speaking="Nobody is speaking" />);
  expect(container.querySelector(".fixture-voice")?.textContent).toBe("Nobody is speaking");
  // Someone types in the conversation.
  const client = fixture.clients.at(-1)!;
  await act(async () => client.emit({ typingAuthors: [history.messages[0].author] }));
  expect(container.querySelector(".chat-typing")?.textContent).toContain("Fixture Author");
  await act(async () => client.emit({ typingAuthors: [] }));
  // The viewer types a draft.
  const textarea = container.querySelector<HTMLTextAreaElement>("#chat-message")!;
  await act(async () => {
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")!.set!.call(textarea, "a draft");
    textarea.dispatchEvent(new Event("input", { bubbles: true }));
  });
  expect(textarea.value).toBe("a draft");
  expect(counts()).toEqual(settled);

  // A row whose own message changes still re-renders, and only that row.
  const current = (client as unknown as { current(): ChatViewState }).current();
  const edited = current.messages.map((entry, index) =>
    index === 4 ? { ...entry, reactions: [{ emoji: "👍", authorIds: ["author-2"] }] } : entry,
  );
  await act(async () => client.emit({ messages: edited }));
  expect(counts()).toEqual(settled.map((count, index) => (index === 4 ? count + 1 : count)));
  const afterEdit = counts();

  // A message arriving at the live end animates in without re-rendering the rest.
  const arriving = message(COUNT * 60 + 1);
  await act(async () => client.emit({ messages: [...edited, arriving] }));
  const row = container.querySelector(`.chat-message[data-message-key="${arriving.clientMessageId}"]`);
  expect(row?.classList.contains("chat-message-arrive")).toBe(true);
  expect(counts()).toEqual(afterEdit);
});

test.each([false, true])("Reply focuses within the click, including a repeated target (menu: %s)", async (menu) => {
  await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
  await act(async () => fixture.clients.at(-1)!.emit({ phase: "ready", author: history.messages[0].author }));
  const row = container.querySelector(".chat-message[data-message-key]")!;
  const input = container.querySelector<HTMLTextAreaElement>("#chat-message")!;
  for (let repeat = 0; repeat < 2; repeat++) {
    input.blur();
    if (menu) {
      await act(async () => row.querySelector<HTMLButtonElement>(".chat-message-actions-trigger")!.click());
    }
    const button = menu
      ? [...document.querySelectorAll<HTMLButtonElement>(".chat-message-actions button")].find(
          (button) => button.textContent?.trim() === "Reply",
        )!
      : row.querySelector<HTMLButtonElement>(".chat-reply-message")!;
    await act(async () => {
      button.click();
      // Waiting for a frame would pass on desktop but lose iOS's keyboard activation.
      expect(document.activeElement).toBe(input);
    });
    expect(document.activeElement).toBe(input);
    expect(container.querySelector(".chat-reply-draft")?.textContent).toContain(history.messages[0].content.text);
  }
});

test.each([false, true])("Reply in thread focuses before history arrives (menu: %s)", async (menu) => {
  await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
  await act(async () => fixture.clients.at(-1)!.emit({ phase: "ready", author: history.messages[0].author }));
  const row = container.querySelector(".chat-message[data-message-key]")!;
  if (menu) {
    await act(async () => row.querySelector<HTMLButtonElement>(".chat-message-actions-trigger")!.click());
  }
  const button = menu
    ? [...document.querySelectorAll<HTMLButtonElement>(".chat-message-actions button")].find(
        (button) => button.textContent?.trim() === "Reply in thread",
      )!
    : row.querySelector<HTMLButtonElement>(".chat-reply-thread")!;
  await act(async () => {
    button.click();
    const input = container.querySelector<HTMLTextAreaElement>("#chat-thread-reply")!;
    expect(input).not.toBeNull();
    expect(input.disabled).toBe(false);
    expect(document.activeElement).toBe(input);
  });
  expect(document.activeElement?.id).toBe("chat-thread-reply");
  expect(container.querySelector(".chat-thread-messages")?.getAttribute("aria-busy")).toBe("true");
  expect(container.querySelector<HTMLButtonElement>('button[aria-label="Send reply"]')!.disabled).toBe(true);
});
