// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test, vi } from "vitest";
import NotificationsInbox from "./NotificationsInbox";
import type { InboxItem } from "./client";
import { rememberSavedAccount } from "../account/client";
import type { ComposerInput } from "../chat/MentionTextarea";

test("inbox lists without reading, preserves failed reads, filters history and opens the exact reply", async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const reply: InboxItem = {
    id: "m:12",
    kind: "reply",
    actorId: "actor0000001",
    actorName: "Maya",
    avatarId: null,
    body: "A reply in an older thread",
    createdAt: "2026-11-04T12:00:00Z",
    unread: true,
    spaceId: "space0000001",
    spaceName: "Studio",
    channelId: "channel00001",
    channelName: "ideas",
    conversationId: null,
    messageId: "message00000012",
    threadRootId: "message00000001",
    emoji: null,
  };
  let rows: InboxItem[] = [
    reply,
    { ...reply, id: "r:31", kind: "reaction", actorName: "Kai", body: "Your post", emoji: "🎉" },
  ];
  let failRead = true;
  let opened: InboxItem | undefined;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    if (init?.method === "POST") {
      if (failRead) return Response.json({ error: "Couldn’t save. Try again." }, { status: 503 });
      const { ids } = JSON.parse(String(init.body)) as { ids: string[] };
      rows = rows.map((row) => (ids.includes(row.id) ? { ...row, unread: false } : row));
      return Response.json({ unreadCount: rows.filter((row) => row.unread).length });
    }
    const query = new URL(String(input), "http://localhost").searchParams;
    return Response.json({
      items: rows.filter(
        (row) =>
          (query.get("unread") !== "true" || row.unread) &&
          (query.get("filter") !== "reactions" || row.kind === "reaction"),
      ),
      nextCursor: null,
      unreadCount: rows.filter((row) => row.unread).length,
    });
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const button = (label: string) =>
    Array.from(container.querySelectorAll("button")).find((element) => element.textContent === label)!;
  const click = async (element: HTMLElement) => act(async () => element.click());
  try {
    await act(async () =>
      root.render(
        <NotificationsInbox
          accountId="self00000001"
          onClose={() => {}}
          onCount={() => {}}
          onOpen={(item) => {
            opened = item;
          }}
        />,
      ),
    );
    expect(container.querySelectorAll(".inbox-list li")).toHaveLength(2);
    expect(container.querySelector<HTMLInputElement>('[role="switch"]')?.checked).toBe(true);
    expect(container.querySelector(".inbox-toolbar")?.textContent).toContain("Unread only(2)");
    expect(container.querySelector('[aria-label="Refresh notifications"]')).toBeNull();
    expect(rows.every((row) => row.unread)).toBe(true);
    expect(document.title).toBe("(2) Caper");
    await click(container.querySelector<HTMLButtonElement>('[aria-label="Mark notification from Kai read"]')!);
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Couldn’t save");
    expect(container.querySelectorAll(".inbox-list li")).toHaveLength(2);
    failRead = false;
    await click(container.querySelector<HTMLButtonElement>('[aria-label="Mark notification from Kai read"]')!);
    expect(container.querySelectorAll(".inbox-list li")).toHaveLength(1);
    expect(document.title).toBe("(1) Caper");
    await click(button("Reactions"));
    expect(container.textContent).toContain("You’re all caught up");
    await click(container.querySelector<HTMLInputElement>('input[type="checkbox"]')!);
    expect(container.querySelectorAll(".inbox-list li")).toHaveLength(1);
    expect(container.textContent).toContain("Reacted 🎉 to your message");
    expect(container.querySelector(".inbox-read")).toBeNull();
    await click(button("All"));
    await click(container.querySelector<HTMLButtonElement>(".inbox-open")!);
    expect(opened).toEqual(reply);
    expect(document.title).toBe("Caper");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  }
});

test("automatic updates revalidate older pages without gaps, even when the oldest row was read elsewhere", async () => {
  vi.useFakeTimers();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  const make = (id: number): InboxItem => ({
    id: `m:${id}`,
    kind: "message",
    actorId: "actor0000001",
    actorName: `Person ${id}`,
    avatarId: null,
    body: `Message ${id}`,
    createdAt: `2026-11-04T12:00:0${id}Z`,
    unread: true,
    spaceId: null,
    spaceName: null,
    channelId: null,
    channelName: null,
    conversationId: "direct000001",
    messageId: `message0000${id}`,
    threadRootId: null,
    emoji: null,
  });
  let rows = [4, 3, 2, 1, 0].map(make);
  let failContinuation = false;
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input) => {
    const query = new URL(String(input), "http://localhost").searchParams;
    const cursor = query.get("cursor");
    if (cursor && failContinuation) return Response.json({ error: "Offline" }, { status: 503 });
    const offset = cursor ? rows.findIndex((row) => row.id === cursor) + 1 : 0;
    const items = rows.slice(offset, offset + 2);
    return Response.json({
      items,
      unreadCount: rows.length,
      nextCursor: offset + 2 < rows.length ? items.at(-1)!.id : null,
    });
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const previews = () => Array.from(container.querySelectorAll(".inbox-preview"), (node) => node.textContent);
  try {
    await act(async () =>
      root.render(
        <NotificationsInbox accountId="self00000001" onClose={() => {}} onCount={() => {}} onOpen={() => {}} />,
      ),
    );
    await act(async () => container.querySelector<HTMLButtonElement>(".inbox-more")!.click());
    expect(previews()).toEqual(["Message 4", "Message 3", "Message 2", "Message 1"]);
    rows = [6, 5, 4, 2, 0].map(make); // New arrivals, removed 3, acknowledged oldest 1.
    failContinuation = true;
    await act(async () => vi.advanceTimersByTimeAsync(15_000));
    expect(container.querySelector('[role="alert"]')?.textContent).toContain("Offline");
    expect(previews()).toEqual(["Message 4", "Message 3", "Message 2", "Message 1"]);
    failContinuation = false;
    await act(async () => vi.advanceTimersByTimeAsync(15_000));
    expect(previews()).toEqual(["Message 6", "Message 5", "Message 4", "Message 2", "Message 0"]);
    expect(container.querySelector(".inbox-more")).toBeNull();
    expect(container.querySelector('[role="alert"]')).toBeNull();
    expect(container.querySelector(".inbox-toolbar")?.textContent).toContain("(5)");
  } finally {
    await act(async () => root.unmount());
    container.remove();
    vi.useRealTimers();
    vi.restoreAllMocks();
  }
});

test("inline drafts survive collapse and live reads; replies keep their destination and retry identity", async () => {
  const account = { id: "replyself001", username: "alex", displayName: "Alex" };
  let signedIn = account;
  rememberSavedAccount(account);
  const make = (id: string, threadRootId: string | null): InboxItem => ({
    id: `m:${id}`,
    kind: "reply",
    actorId: "maya00000001",
    actorName: "Maya",
    avatarId: null,
    body: `Message ${id}`,
    createdAt: "2026-11-04T12:00:00Z",
    unread: true,
    spaceId: "studio000001",
    spaceName: "Studio",
    channelId: "ideas0000001",
    channelName: "ideas",
    conversationId: null,
    messageId: id,
    threadRootId,
    emoji: null,
  });
  let rows = [make("original0001", null), make("threadreply1", "threadroot01")];
  const commands: Record<string, unknown>[] = [];
  let loseConfirmation = true;
  const opened = vi.fn();
  const closed = vi.fn();
  const reads: string[] = [];
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  vi.spyOn(globalThis, "fetch").mockImplementation(async (input, init) => {
    const path = String(input);
    if (path.startsWith("/api/notifications/inbox?"))
      return Response.json({ items: rows, nextCursor: null, unreadCount: rows.length });
    if (path === "/api/account/me") return Response.json(signedIn);
    if (path === "/api/chat/session")
      return Response.json({ token: "test-reply-session", author: { id: account.id, name: "Alex", isGuest: false } });
    if (path === "/api/chat/channels/ideas0000001/messages" && init?.method === "POST") {
      const command = JSON.parse(String(init.body));
      commands.push(command);
      if (loseConfirmation) throw new TypeError("Connection lost after commit");
      return Response.json({
        id: "sent00000001",
        channelId: "ideas0000001",
        seq: "20",
        clientMessageId: command.clientMessageId,
        author: { id: account.id, name: "Alex", isGuest: false },
        content: { version: 1, type: "text", text: command.text },
        createdAt: "2026-11-04T12:01:00Z",
        threadRootId: command.threadRootId,
        replyTo: command.replyToId
          ? {
              id: command.replyToId,
              author: { id: "maya00000001", name: "Maya", isGuest: false },
              text: "Original",
              hasAttachments: false,
            }
          : undefined,
      });
    }
    if (path.includes("/read") || path.includes("/direct")) reads.push(path);
    return Response.json({}, { status: 404 }); // Uploads/GIFs unavailable, no other capabilities granted.
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const button = (label: string) =>
    Array.from(container.querySelectorAll("button")).find((node) => node.textContent === label)!;
  const composer = () => container.querySelector<ComposerInput>('[role="textbox"]')!;
  const type = async (text: string) =>
    act(async () => {
      const input = composer();
      input.replaceText(0, input.value.length, text);
    });
  const click = async (node: HTMLElement) => act(async () => node.click());
  try {
    await act(async () =>
      root.render(<NotificationsInbox accountId={account.id} onClose={closed} onCount={() => {}} onOpen={opened} />),
    );
    await click(button("Reply"));
    await type("A separate inbox draft");
    await click(button("Close reply"));
    await click(button("Reply"));
    expect(composer().value).toBe("A separate inbox draft");
    await click(container.querySelector<HTMLButtonElement>('[aria-label="Insert emoji"]')!);
    const escape = new KeyboardEvent("keydown", { key: "Escape", bubbles: true, cancelable: true });
    await act(async () => container.querySelector('[aria-label="Search emoji"]')!.dispatchEvent(escape));
    expect(escape.defaultPrevented).toBe(true); // The native dialog must not also cancel.
    expect(closed).not.toHaveBeenCalled();
    expect(container.querySelector(".chat-reaction-picker")).toBeNull();
    // Another device reads this notification while this composer is in use.
    rows = rows.slice(1);
    await act(async () => window.dispatchEvent(new Event("focus")));
    expect(composer().value).toBe("A separate inbox draft");
    await click(container.querySelector<HTMLButtonElement>('[aria-label="Send reply"]')!);
    expect(container.textContent).toContain("Couldn’t confirm delivery");
    expect(commands[0]).toMatchObject({ replyToId: "original0001", text: "A separate inbox draft" });
    expect(commands[0]).not.toHaveProperty("threadRootId");
    loseConfirmation = false;
    await click(button("Retry"));
    expect(commands[1]).toEqual(commands[0]);
    expect(container.textContent).toContain("Reply sent");
    expect(composer().value).toBe("");
    await click(button("Close reply"));
    await click(button("Reply"));
    await type("Continue the existing thread");
    await click(container.querySelector<HTMLButtonElement>('[aria-label="Send reply"]')!);
    expect(commands[2]).toMatchObject({ threadRootId: "threadroot01", text: "Continue the existing thread" });
    expect(commands[2]).not.toHaveProperty("replyToId");
    signedIn = { id: "other0000001", username: "kai", displayName: "Kai" };
    await type("Never send as another account");
    await click(container.querySelector<HTMLButtonElement>('[aria-label="Send reply"]')!);
    expect(container.textContent).toContain("Your account changed");
    expect(commands).toHaveLength(3);
    expect(opened).not.toHaveBeenCalled();
    expect(reads).toEqual([]);
  } finally {
    await act(async () => root.unmount());
    container.remove();
    vi.restoreAllMocks();
  }
});
