// @vitest-environment happy-dom
import { act, type ReactNode } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { ChatViewState } from "./client.ts";
import type { ChatMessage, GeneralChatHistory } from "./types.ts";
import type { ComposerInput } from "./MentionTextarea.tsx";

// Every message row renders its text through LinkedText exactly once, so its
// renders count the row's renders, keyed by the message's text.
const fixture = vi.hoisted(() => ({
  renders: new Map<string, number>(),
  clients: [] as { emit: (change: Partial<ChatViewState>) => void; sent: unknown[][] }[],
  uploads: [] as { name: string; released: boolean }[],
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
    snapshotHistory() {
      return undefined;
    }
    readCursor() {
      return undefined;
    }
    identify() {}
    async refreshUnreadThreads() {}
    refreshProfiles() {}
    setTyping() {}
    async openThread(rootId: string) {
      this.emit({ thread: { rootId, loading: true, loadingOlder: false, hasMore: false } });
    }
    async prefetchThread() {}
    async loadMessageContext() {
      await fetch("/api/chat/channels/channel/messages?around=");
      return true;
    }
    closeThread() {
      this.emit({ thread: undefined });
    }
    sent: unknown[][] = [];
    async send(...command: unknown[]) {
      this.sent.push(command);
      return true;
    }
  }
  return { ...actual, ChatClient };
});

// Uploads are configured, and each staged file's upload is recorded instead of sent.
vi.mock("./uploads.ts", async (importOriginal) => {
  const actual = await importOriginal<typeof import("./uploads.ts")>();
  const { DEFAULT_COMPRESSION } = await import("./prepare.ts");
  class AttachmentUpload {
    state: import("./uploads.ts").UploadState;
    private record: { name: string; released: boolean };
    constructor(_channel: string, file: File) {
      this.state = { name: file.name, sourceSize: file.size, progress: 0 };
      this.record = { name: file.name, released: false };
      fixture.uploads.push(this.record);
    }
    get stale() {
      return false;
    }
    get source() {
      return undefined;
    }
    ensure() {
      return new Promise(() => {});
    }
    release() {
      this.record.released = true;
    }
  }
  return {
    ...actual,
    AttachmentUpload,
    rememberedUploadSettings: () => DEFAULT_COMPRESSION,
    loadUploadSettings: async () => DEFAULT_COMPRESSION,
  };
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
  fixture.uploads.length = 0;
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(() => {
  act(() => root.unmount());
  container.remove();
  vi.restoreAllMocks();
});

test.each([false, true])(
  "deleted rows disappear but their quoted replies remain (channel window: %s)",
  async (windowed) => {
    await render(<Page speaking="Nobody is speaking" />);
    const client = fixture.clients.at(-1)!;
    const original = message(1);
    const answer: ChatMessage = {
      ...message(2),
      replyTo: { id: original.id, author: original.author, text: "", hasAttachments: false, deleted: true },
    };
    const tombstone = { ...original, deletedAt: "2026-10-19T00:05:00Z" };
    const messages = [tombstone, answer];
    await act(async () => client.emit({ messages, channelMessages: windowed ? messages : undefined }));
    expect(container.querySelector(`[data-message-key="${original.clientMessageId}"]`)).toBeNull();
    expect(container.querySelectorAll(".chat-message[data-message-key]")).toHaveLength(1);
    expect(container.textContent).toContain("Original message was deleted.");
    expect(container.textContent).toContain(answer.content.text);
    expect(container.textContent).not.toContain("This message was deleted.");
    expect(container.querySelector(".chat-reply-source")?.tagName).toBe("DIV");
    expect(container.querySelector(".chat-reply-context button")).toBeNull();
    expect(container.querySelector(".chat-reply-view")).toBeNull();

    await act(async () => client.emit({ messages: [tombstone], channelMessages: windowed ? [tombstone] : undefined }));
    expect(container.querySelector(".chat-message[data-message-key]")).toBeNull();
    expect(container.querySelector(".chat-date-divider")).toBeNull();
    expect(container.textContent).toContain("No messages yet.");
    await act(async () => client.emit({ hasMore: true }));
    expect(container.textContent).toContain("Load older messages");
    expect(container.textContent).not.toContain("No messages yet.");
  },
);

test("opening a reply's loaded original highlights it in place without reloading the list", async () => {
  await render(<Page speaking="Nobody is speaking" />);
  const client = fixture.clients.at(-1)!;
  const original = message(1);
  const answer: ChatMessage = {
    ...message(2),
    replyTo: { id: original.id, author: original.author, text: text(1), hasAttachments: false },
  };
  await act(async () => client.emit({ messages: [original, answer] }));
  const row = () => container.querySelector(`.chat-message[data-message-key="${original.clientMessageId}"]`);
  const before = row();
  expect(before).not.toBeNull();
  const fetches = vi.mocked(fetch).mock.calls.length;

  await act(async () => container.querySelector<HTMLButtonElement>("button.chat-reply-source")!.click());
  expect(row()).toBe(before);
  expect(row()?.classList).toContain("chat-message-jump-target");
  expect(vi.mocked(fetch).mock.calls.length).toBe(fetches);
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
  const textarea = container.querySelector<ComposerInput>("#chat-message")!;
  await act(async () => {
    const clipboardData = new DataTransfer();
    clipboardData.setData("text/plain", "a draft");
    textarea.dispatchEvent(new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }));
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

  // The viewer's own send, pending and then confirmed, re-renders no other row.
  const sent = [...edited, arriving];
  await act(async () =>
    client.emit({
      pendingSend: { clientMessageId: "client-pending", text: "a pending send", createdAt: arriving.createdAt },
    }),
  );
  expect(container.textContent).toContain("a pending send");
  await act(async () => client.emit({ pendingSend: undefined }));
  expect(counts()).toEqual(afterEdit);

  // Someone's video processing ticks re-render only the row that shows the percentage.
  const file = { kind: "file", contentType: "application/pdf", size: 1 } as const;
  const withFiles = sent.map((entry, index) =>
    index === 2
      ? { ...entry, content: { ...entry.content, attachments: [{ ...file, id: "ready", name: "ready.pdf" }] } }
      : index === 6
        ? {
            ...entry,
            content: {
              ...entry.content,
              attachments: [{ ...file, id: "processing", name: "clip.mov", kind: "video", status: "processing" }],
            },
          }
        : entry,
  ) as ChatMessage[];
  await act(async () => client.emit({ messages: withFiles }));
  const beforeTicks = counts();
  await act(async () => client.emit({ attachmentProgress: { processing: 10 } }));
  await act(async () => client.emit({ attachmentProgress: { processing: 11 } }));
  expect(counts()).toEqual(beforeTicks.map((count, index) => (index === 6 ? count + 2 : count)));
});

test("a send that failed offline is retried once when the connection returns, unless the server rejected it", async () => {
  await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
  const client = fixture.clients.at(-1)!;
  const pendingSend = {
    clientMessageId: "client-pending",
    text: "sent in a tunnel",
    createdAt: history.messages[0].createdAt,
  };
  await act(async () => client.emit({ phase: "ready", online: true, author: history.messages[0].author }));
  await act(async () => client.emit({ online: false, pendingSend, sendError: "Not confirmed yet." }));
  expect(client.sent).toEqual([]);
  await act(async () => client.emit({ online: true }));
  expect(client.sent).toEqual([["sent in a tunnel", { threadRootId: undefined, broadcast: undefined }]]);
  // Staying online does not send it again; a rejected message waits for the person.
  await act(async () => client.emit({ typingAuthors: [] }));
  await act(async () => client.emit({ online: false, sendRejected: true }));
  await act(async () => client.emit({ online: true }));
  expect(client.sent).toHaveLength(1);
});

test("an unconfirmed send goes again by itself after 3, 10 and 30 seconds, then waits; a refusal never does", async () => {
  vi.useFakeTimers();
  try {
    await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
    const client = fixture.clients.at(-1)!;
    const pendingSend = {
      clientMessageId: "client-pending",
      text: "sent during a deploy",
      createdAt: history.messages[0].createdAt,
    };
    const fail = async (change: Partial<ChatViewState> = {}) => {
      // Each attempt clears the error while it is in flight, then fails again.
      await act(async () => client.emit({ pendingSend, sendError: undefined, ...change }));
      await act(async () => client.emit({ sendError: "Message could not be sent." }));
    };
    await act(async () => client.emit({ phase: "ready", online: true, author: history.messages[0].author }));
    await fail();
    for (const [attempt, delay] of [3_000, 10_000, 30_000].entries()) {
      await act(async () => vi.advanceTimersByTime(delay - 1));
      expect(client.sent).toHaveLength(attempt);
      await act(async () => vi.advanceTimersByTime(1));
      expect(client.sent).toHaveLength(attempt + 1);
      await fail();
    }
    await act(async () => vi.advanceTimersByTime(300_000));
    expect(client.sent).toHaveLength(3);
    // A new message starts over, but the server refusing it is final.
    await fail({ pendingSend: { ...pendingSend, clientMessageId: "client-refused" }, sendRejected: true });
    await act(async () => vi.advanceTimersByTime(300_000));
    expect(client.sent).toHaveLength(3);
  } finally {
    vi.useRealTimers();
  }
});

test("Enter during a send in flight sends the next draft once it is confirmed, unless edited or the send failed", async () => {
  await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
  const client = fixture.clients.at(-1)!;
  const pendingSend = { clientMessageId: "client-first", text: "first", createdAt: history.messages[0].createdAt };
  const input = container.querySelector<ComposerInput>("#chat-message")!;
  const type = (text: string) =>
    act(async () => {
      const clipboardData = new DataTransfer();
      clipboardData.setData("text/plain", text);
      input.dispatchEvent(new ClipboardEvent("paste", { clipboardData, bubbles: true, cancelable: true }));
    });
  const enter = () =>
    act(async () =>
      input.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true })),
    );
  const confirmed = () => act(async () => client.emit({ pendingSend: undefined, sendError: undefined }));
  await act(async () => client.emit({ phase: "ready", online: true, author: history.messages[0].author }));

  await act(async () => client.emit({ pendingSend }));
  await type("second");
  await enter();
  expect(client.sent).toEqual([]);
  await confirmed();
  expect(client.sent.map((command) => command[0])).toEqual(["second"]);

  // Typing after Enter means the draft is no longer what Enter approved.
  await act(async () => client.emit({ pendingSend }));
  await enter();
  await type(" and more");
  await confirmed();
  expect(client.sent).toHaveLength(1);

  // A failed send keeps the composer for its own retry.
  await act(async () => client.emit({ pendingSend }));
  await enter();
  await act(async () => client.emit({ sendError: "Message could not be sent." }));
  await confirmed();
  expect(client.sent).toHaveLength(1);
});

test.each([false, true])("Reply focuses within the click, including a repeated target (menu: %s)", async (menu) => {
  await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
  await act(async () => fixture.clients.at(-1)!.emit({ phase: "ready", author: history.messages[0].author }));
  const row = container.querySelector(".chat-message[data-message-key]")!;
  const input = container.querySelector<ComposerInput>("#chat-message")!;
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
    const input = container.querySelector<ComposerInput>("#chat-thread-reply")!;
    expect(input).not.toBeNull();
    expect(input.disabled).toBe(false);
    expect(document.activeElement).toBe(input);
  });
  expect(document.activeElement?.id).toBe("chat-thread-reply");
  expect(container.querySelector(".chat-thread-messages")?.getAttribute("aria-busy")).toBe("true");
  expect(container.querySelector<HTMLButtonElement>('button[aria-label="Send reply"]')!.disabled).toBe(true);
});

test("a mouse row keeps every action button but draws their icons only for hover, focus or an open menu", async () => {
  const mouse = (query: string) =>
    ({ matches: query.includes("hover: hover"), addEventListener() {}, removeEventListener() {} }) as never;
  vi.spyOn(window, "matchMedia").mockImplementation(mouse);
  await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
  await act(async () => fixture.clients.at(-1)!.emit({ phase: "ready", author: history.messages[0].author }));
  const [row, other] = container.querySelectorAll<HTMLElement>(".chat-message[data-message-key]");
  const buttons = (item: Element) => item.querySelectorAll(".chat-message-tools > button").length;
  const icons = (item: Element) => item.querySelectorAll(".chat-message-tools :is(svg, img)").length;
  // Three quick reactions, Reply in thread, Reply, Add reaction and the actions menu.
  expect(buttons(row)).toBe(7);
  // Only the actions menu, which is all a touch layout shows.
  expect(icons(row)).toBe(1);

  await act(async () => row.dispatchEvent(new MouseEvent("pointerover", { bubbles: true })));
  expect(icons(row)).toBe(7);
  expect(icons(other)).toBe(1);
  await act(async () => row.dispatchEvent(new MouseEvent("mouseout", { bubbles: true, relatedTarget: document.body })));
  expect(icons(row)).toBe(1);

  // Tab reaches a button the pointer is nowhere near.
  await act(async () => row.querySelector<HTMLButtonElement>(".chat-reply-message")!.focus());
  expect(icons(row)).toBe(7);
  await act(async () => row.querySelector<HTMLButtonElement>(".chat-message-actions-trigger")!.click());
  await act(async () => (document.activeElement as HTMLElement).blur());
  expect(document.querySelector(".chat-message-actions")).not.toBeNull();
  expect(icons(row)).toBe(7);
});

test("rows follow the account's quick reactions without re-rendering for an unchanged list", async () => {
  const { getAccount } = await import("../account/client.ts");
  let quickReactions = ["🔥", "🎉", "🙌"];
  vi.mocked(globalThis.fetch).mockImplementation(async (input) =>
    String(input).endsWith("/api/account/me")
      ? Response.json({ id: "viewer", username: "viewer", displayName: "Viewer", quickReactions })
      : new Response("{}", { status: 404 }),
  );
  await render(<Page speaking="Nobody is speaking" />);
  await act(async () => {
    await getAccount();
    await new Promise((resolve) => setTimeout(resolve, 50));
  });
  const settled = counts();
  // A refresh returns the same list as a new array: no row re-renders.
  await act(async () => void (await getAccount()));
  expect(counts()).toEqual(settled);
  // A new most-used emoji reaches every row.
  quickReactions = ["🎉", "🔥", "🙌"];
  await act(async () => void (await getAccount()));
  expect(counts()).toEqual(settled.map((count) => count + 1));
});

test("Up in an empty composer edits your newest message, and Escape closes the editor", async () => {
  await render(<Chat name="Viewer" signedIn identityReady channelId="channel" initialHistory={history} />);
  await act(async () => fixture.clients.at(-1)!.emit({ phase: "ready", author: history.messages[0].author }));
  const key = (target: Element, key: string, keyCode: number) =>
    act(async () => {
      target.dispatchEvent(new KeyboardEvent("keydown", { key, keyCode, bubbles: true, cancelable: true }));
    });
  await key(container.querySelector("#chat-message")!, "ArrowUp", 38);
  const editor = document.querySelector<ComposerInput>("dialog.chat-edit-dialog #chat-edit-text");
  expect(editor?.value).toBe(history.messages.at(-1)!.content.text);
  // ProseMirror claims every Escape; the dialog must still see it as unhandled.
  await key(editor!, "Escape", 27);
  expect(document.querySelector("dialog.chat-edit-dialog")).toBeNull();
});

test("a removed file added back within the grace period keeps its upload; later it is released", async () => {
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    await render(
      <Chat name="Viewer" signedIn identityReady accountId="viewer" channelId="channel" initialHistory={history} />,
    );
    await act(async () => fixture.clients.at(-1)!.emit({ phase: "ready", author: history.messages[0].author }));
    const photo = new File(["pixels"], "photo.png", { type: "image/png", lastModified: 1 });
    const other = new File(["other pixels"], "photo.png", { type: "image/png", lastModified: 2 });
    const drop = (files: File[]) =>
      act(async () => {
        const event = new Event("drop", { bubbles: true, cancelable: true });
        Object.defineProperty(event, "dataTransfer", { value: { files, types: ["Files"] } });
        container.querySelector(".chat-panel")!.dispatchEvent(event);
      });
    const remove = () =>
      act(async () => container.querySelector<HTMLButtonElement>('[aria-label="Remove photo.png"]')!.click());
    const tiles = () => container.querySelectorAll(".chat-drafts li").length;

    await drop([photo]);
    expect(tiles()).toBe(1);
    expect(container.querySelector(".chat-draft-ring")).not.toBeNull();
    await remove();
    expect(tiles()).toBe(0);
    expect(fixture.uploads).toEqual([{ name: "photo.png", released: false }]);

    // The same file comes back with the same upload; a different one with that name does not.
    await drop([photo]);
    expect(tiles()).toBe(1);
    expect(fixture.uploads).toHaveLength(1);
    await drop([other]);
    expect(fixture.uploads).toHaveLength(2);

    // Removed and left alone, it is released once the grace period ends, and adding it then starts again.
    await remove();
    await act(async () => vi.advanceTimersByTime(29_000));
    expect(fixture.uploads[0].released).toBe(false);
    await act(async () => vi.advanceTimersByTime(1_000));
    expect(fixture.uploads[0].released).toBe(true);
    await drop([photo]);
    expect(fixture.uploads).toHaveLength(3);
  } finally {
    vi.useRealTimers();
  }
});
