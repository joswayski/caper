import { renderToStaticMarkup } from "react-dom/server";
import { expect, test, vi } from "vitest";
import type { GeneralChatHistory } from "../chat/types";
import Call from "./Call";

// Records which special `@` names each composer (`chat`, `chat-thread`) offers.
const offeredSpecials = vi.hoisted(() => new Map<string, readonly string[]>());
vi.mock("../chat/ComposerSuggestions.tsx", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../chat/ComposerSuggestions.tsx")>();
  return {
    ...actual,
    useComposerSuggestions: (options: Parameters<typeof actual.useComposerSuggestions>[0]) => {
      offeredSpecials.set(options.id, options.specialMentions);
      return actual.useComposerSuggestions(options);
    },
  };
});

const emptyHistory = (direct: boolean): GeneralChatHistory => ({
  space: { id: "space", name: "Space" },
  channel: { id: "conversation", name: "Same name", direct },
  cursor: "0",
  hasMore: false,
  messages: [],
});

test("audio controls remain available before joining, with a text-only Join action", () => {
  const markup = renderToStaticMarkup(<Call />);
  for (const label of ["Mute microphone", "Deafen audio", "Input Options", "Output Options"]) {
    const button = markup.match(new RegExp(`<button[^>]*aria-label="${label}"[^>]*>`))?.[0];
    expect(button).toBeDefined();
    expect(button).not.toContain('disabled=""');
    expect(button).not.toContain('aria-disabled="true"');
  }
  const join = markup.match(/<button[^>]*aria-label="Join voice"[^>]*>(.*?)<\/button>/)?.[1];
  expect(join).toContain("Join voice");
  expect(join).not.toContain("<svg");
  expect(markup).not.toContain('aria-label="Leave voice"');
});

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
      initialHistory={emptyHistory(direct)}
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

test("messages from Caper admins carry the ADMIN badge, and nobody else's do", () => {
  const message = (id: string, admin?: boolean) => ({
    id,
    channelId: "channel",
    seq: id === "admin" ? "1" : "2",
    clientMessageId: `client-${id}`,
    author: { id, name: "Jose", isGuest: false, ...(admin === undefined ? {} : { admin }) },
    content: { version: 1 as const, type: "text" as const, text: `From ${id}` },
    createdAt: "2026-10-11T00:00:00Z",
  });
  const markup = renderToStaticMarkup(
    <Call
      initialHistory={{
        space: { id: "space", name: "Space" },
        channel: { id: "channel", name: "general" },
        cursor: "2",
        hasMore: false,
        messages: [message("admin", true), message("lookalike")],
      }}
    />,
  );
  const badges = [...markup.matchAll(/<span class="chat-admin-badge"[^>]*>Admin<\/span>/g)];
  expect(badges).toHaveLength(1);
  expect(markup.indexOf("chat-admin-badge")).toBeLessThan(markup.indexOf("From admin"));
});

test.each([
  ["the person who sent feedback", "team"],
  ["a team member reading someone's feedback", "other"],
])("an empty feedback conversation explains who reads it, for %s", (_viewer, directPeerId) => {
  const markup = renderToStaticMarkup(
    <Call
      initialAccount={{ id: "me", username: "me", displayName: "Me" }}
      channel={{
        id: "conversation",
        name: "Same name",
        spaceName: "Space",
        direct: true,
        directPeerId,
        feedback: true,
      }}
      initialHistory={emptyHistory(true)}
    />,
  );
  expect(markup).toContain("No messages yet.");
  expect(markup).toContain(
    "The Caper team reads and replies here. Mention @feedback anywhere to send a message or its thread here.",
  );
  expect(markup).not.toContain("Only you and");
});

test.each([
  ["a space channel", { direct: false }, ["everyone", "here", "feedback"]],
  ["a DM", { direct: true, directPeerId: "other" }, ["feedback"]],
  ["personal notes", { direct: true, directPeerId: "me" }, ["feedback"]],
  ["a feedback conversation", { direct: true, directPeerId: "team", feedback: true }, []],
])("the composer and thread composer in %s offer the same special mentions", (_place, channel, specials) => {
  offeredSpecials.clear();
  renderToStaticMarkup(
    <Call
      initialAccount={{ id: "me", username: "me", displayName: "Me" }}
      channel={{ id: "conversation", name: "Same name", spaceName: "Space", ...channel }}
      initialHistory={emptyHistory(channel.direct)}
    />,
  );
  expect(offeredSpecials.get("chat")).toEqual(specials);
  expect(offeredSpecials.get("chat-thread")).toEqual(specials);
});
