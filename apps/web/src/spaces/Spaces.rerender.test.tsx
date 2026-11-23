// @vitest-environment happy-dom
import { act } from "react";
import { createRoot, type Root } from "react-dom/client";
import { afterEach, beforeEach, expect, test, vi } from "vitest";
import type { Account } from "../account/client";
import { resetChatSessionsForTests, sharedAccountSession } from "../chat/client";
import { klipyConfig, resetKlipyConfigForTests } from "../chat/klipy";
import { loadUploadSettings, resetUploadSettingsForTests } from "../chat/uploads";
import type { GeneralChatHistory } from "../chat/types";
import type { DirectConversation, Space, SpaceDetail } from "./client";

const fixture = vi.hoisted(() => ({
  spaceIconRenders: new Map<string, number>(),
  avatarRenders: new Map<string, number>(),
  presenceCallbacks: new Map<string, (members: { userId: string; status: "online" | "idle" | "offline" }[]) => void>(),
}));

vi.mock("../components/SpaceIcon", () => ({
  default: ({ space }: { space: { id?: string; name: string } }) => {
    const key = space.id ?? space.name;
    fixture.spaceIconRenders.set(key, (fixture.spaceIconRenders.get(key) ?? 0) + 1);
    return <span>{space.name.slice(0, 1).toUpperCase()}</span>;
  },
}));

vi.mock("../components/Avatar", () => ({
  default: ({ name }: { name: string }) => {
    fixture.avatarRenders.set(name, (fixture.avatarRenders.get(name) ?? 0) + 1);
    return <span>{name.slice(0, 1).toUpperCase()}</span>;
  },
  initial: (name: string) => (Array.from(name.trim())[0] ?? "").toUpperCase(),
}));

vi.mock("../gateway/client", () => ({
  appGateway: () => ({
    localPresence: () => "online" as const,
    subscribe: () => ({ ready: Promise.resolve(), unsubscribe: () => {} }),
    command: async () => ({}),
  }),
  watchPresence: (
    spaceId: string,
    _users: string[] | "all",
    onMembers: (members: { userId: string; status: "online" | "idle" | "offline" }[]) => void,
    onLive?: (live: boolean) => void,
  ) => {
    fixture.presenceCallbacks.set(spaceId, onMembers);
    queueMicrotask(() => {
      onLive?.(true);
      onMembers([{ userId: "acct00000001", status: "online" }]);
    });
    return () => {
      if (fixture.presenceCallbacks.get(spaceId) === onMembers) fixture.presenceCallbacks.delete(spaceId);
    };
  },
}));

vi.mock("../media/presence", () => ({
  watchPresence: () => () => {},
}));

vi.mock("../media/warm", () => ({
  hasWarmVoice: () => false,
  keepVoiceWarm: () => {},
  stopVoiceWarm: () => {},
}));

vi.mock("../account/browser-notifications", () => ({
  watchBrowserNotifications: () => () => {},
}));

vi.mock("../account/live-updates", () => ({
  watchLiveUpdates: () => () => {},
}));

const { default: Spaces } = await import("./Spaces");

const account: Account = {
  id: "acct00000001",
  username: "maya",
  displayName: "Maya",
};

const spaces: Space[] = [
  { id: "space0000001", name: "Alpha", ownerId: "acct00000001" },
  { id: "space0000002", name: "Beta", ownerId: "acct00000001" },
  { id: "space0000003", name: "Gamma", ownerId: "acct00000001" },
];

const details: Record<string, SpaceDetail> = {
  space0000001: {
    space: spaces[0],
    channels: [{ id: "chan00000001", spaceId: "space0000001", name: "general", private: false, joined: true }],
    members: [{ id: "acct00000001", username: "maya", displayName: "Maya", owner: true }],
  },
  space0000002: {
    space: spaces[1],
    channels: [{ id: "chan00000002", spaceId: "space0000002", name: "design", private: false, joined: true }],
    members: [{ id: "acct00000001", username: "maya", displayName: "Maya", owner: true }],
  },
  space0000003: {
    space: spaces[2],
    channels: [{ id: "chan00000003", spaceId: "space0000003", name: "music", private: false, joined: true }],
    members: [{ id: "acct00000001", username: "maya", displayName: "Maya", owner: true }],
  },
};

const histories: Record<string, GeneralChatHistory> = {
  chan00000001: {
    space: { id: "space0000001", name: "Alpha" },
    channel: { id: "chan00000001", name: "general" },
    cursor: "1",
    hasMore: false,
    messages: [
      {
        id: "msg000000001",
        channelId: "chan00000001",
        seq: "1",
        clientMessageId: "client-a1",
        author: { id: "acct00000002", name: "Alex", isGuest: false },
        content: { version: 1, type: "text", text: "Hello Alpha" },
        createdAt: "2026-11-20T00:00:00Z",
        reactions: [{ emoji: "👍", authorIds: ["acct00000002"] }],
      },
    ],
  },
  chan00000002: {
    space: { id: "space0000002", name: "Beta" },
    channel: { id: "chan00000002", name: "design" },
    cursor: "1",
    hasMore: false,
    messages: [
      {
        id: "msg000000002",
        channelId: "chan00000002",
        seq: "1",
        clientMessageId: "client-b1",
        author: { id: "acct00000002", name: "Alex", isGuest: false },
        content: { version: 1, type: "text", text: "Hello Beta" },
        createdAt: "2026-11-20T00:01:00Z",
        reactions: [{ emoji: "🔥", authorIds: ["acct00000002"] }],
      },
    ],
  },
  chan00000003: {
    space: { id: "space0000003", name: "Gamma" },
    channel: { id: "chan00000003", name: "music" },
    cursor: "1",
    hasMore: false,
    messages: [],
  },
};

const directs: DirectConversation[] = [
  {
    id: "dm0000000001",
    peer: { id: "acct00000009", username: "sam", displayName: "Sam" },
    status: "accepted",
    readSeq: "1",
    lastSeq: "1",
  },
];

let container: HTMLElement;
let root: Root;

beforeEach(() => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  fixture.spaceIconRenders.clear();
  fixture.avatarRenders.clear();
  fixture.presenceCallbacks.clear();
  resetChatSessionsForTests();
  resetKlipyConfigForTests();
  resetUploadSettingsForTests();
  vi.spyOn(document, "visibilityState", "get").mockReturnValue("visible");
  window.history.replaceState({}, "", "/spaces?space=space0000001&channel=chan00000001");
  if (!window.matchMedia) {
    Object.defineProperty(window, "matchMedia", {
      writable: true,
      value: () => ({
        matches: false,
        media: "",
        onchange: null,
        addEventListener: () => {},
        removeEventListener: () => {},
        dispatchEvent: () => false,
      }),
    });
  }
  container = document.createElement("div");
  document.body.append(container);
  root = createRoot(container);
});

afterEach(async () => {
  await act(async () => root.unmount());
  container.remove();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

function installFetch(options: { holdSpace?: string; holdMediaStatus?: boolean } = {}) {
  const heldSpaces: { spaceId: string; finish: () => void }[] = [];
  const heldMedia: { url: string; finish: () => void }[] = [];
  vi.spyOn(globalThis, "fetch").mockImplementation((input) => {
    const url = new URL(String(input), "http://localhost");
    const path = url.pathname;
    if (path === "/api/klipy")
      return Promise.resolve(Response.json({ appKey: "klipy-key", customerId: "acct00000001" }));
    if (path === "/api/assets/usage")
      return Promise.resolve(
        Response.json({
          used: 0,
          limit: 1000,
          compression: { imageFormat: "webp", imageQuality: 85, imageMaxEdge: 1600 },
        }),
      );
    if (path === "/api/chat/session")
      return Promise.resolve(
        Response.json({
          token: "chat-token",
          author: { id: "acct00000001", name: "Maya", isGuest: false },
        }),
      );
    if (path === "/api/spaces/unread") return Promise.resolve(Response.json({ spaces: [], badgeCount: 0 }));
    if (path === "/api/spaces/voice-activity") return Promise.resolve(Response.json({ spaceIds: [] }));
    if (path === "/api/people") return Promise.resolve(Response.json({ people: [] }));
    if (path === "/api/blocks") return Promise.resolve(Response.json({ blocks: [] }));
    if (path === "/api/notifications/settings")
      return Promise.resolve(Response.json({ defaultLevel: "mentions", voiceStarted: true, overrides: [] }));
    if (path.endsWith("/media/status")) {
      if (options.holdMediaStatus && path.includes("chan00000002")) {
        return new Promise((resolve) => {
          heldMedia.push({ url: path, finish: () => resolve(Response.json({ enabled: true })) });
        });
      }
      return Promise.resolve(Response.json({ enabled: true }));
    }
    const spaceMatch = path.match(/^\/api\/spaces\/([^/]+)$/);
    if (spaceMatch) {
      const spaceId = spaceMatch[1];
      if (options.holdSpace === spaceId) {
        return new Promise((resolve) => {
          heldSpaces.push({ spaceId, finish: () => resolve(Response.json(details[spaceId])) });
        });
      }
      return Promise.resolve(Response.json(details[spaceId]));
    }
    const historyMatch = path.match(/^\/api\/chat\/channels\/([^/]+)\/messages$/);
    if (historyMatch) return Promise.resolve(Response.json(histories[historyMatch[1]]));
    if (path.endsWith("/threads/unread")) return Promise.resolve(Response.json({ threads: [] }));
    return Promise.resolve(Response.json({}));
  });
  return { heldSpaces, heldMedia };
}

test("switching spaces keeps composer, reaction, voice, presence, and unrelated sidebar buttons steady without flashes", async () => {
  const { heldMedia } = installFetch({ holdMediaStatus: true });

  expect(await klipyConfig()).not.toBeNull();
  expect(await loadUploadSettings(account.id)).not.toBeNull();
  expect(await sharedAccountSession(account.id, account.displayName!)).toBeTruthy();

  await act(async () => {
    root.render(
      <Spaces
        initialAccount={account}
        initialSpaceList={{
          spaces,
          invitations: [],
          limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
        }}
        initialDirects={directs}
        initialSelection={{ spaceId: "space0000001", channelId: "chan00000001" }}
        initialView={{
          detail: details.space0000001,
          channelId: "chan00000001",
          history: histories.chan00000001,
        }}
      />,
    );
  });
  // Let initial presence and Alpha's media/status settle.
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });

  const attach = () => container.querySelector<HTMLButtonElement>('button[aria-label="Attach files"]');
  const gif = () => container.querySelector<HTMLButtonElement>('button[aria-label="Send a GIF or sticker"]');
  const presenceDot = () => container.querySelector<HTMLElement>(".call-account .presence-dot");
  const railButton = (name: string) =>
    Array.from(container.querySelectorAll<HTMLButtonElement>(".space-rail-item button")).find((button) =>
      button.getAttribute("aria-label")?.startsWith(name),
    )!;

  expect(attach()?.title).toBe("Attach files");
  expect(attach()?.disabled).toBe(false);
  expect(gif()).not.toBeNull();
  expect(presenceDot()?.dataset.status).toBe("online");

  const gammaBefore = fixture.spaceIconRenders.get("space0000003") ?? 0;
  const samDmBefore = fixture.avatarRenders.get("Sam") ?? 0;

  // Switch to unvisited Beta while Beta's /media/status is still pending.
  await act(async () => {
    railButton("Beta").click();
  });

  // Immediately after Beta opens (before Beta's /media/status finishes):
  // - Composer Attach & GIF buttons never flashed disabled or disappeared.
  // - Account presence dot stayed online (never flashed unknown/grey).
  // - Beta's channel voice join button stayed enabled via confirmed account voice availability.
  // - Reaction chips stayed interactive immediately.
  // - Unrelated Gamma rail tile and Sam DM row did not re-render.
  expect(container.querySelector(".chat-title-text")?.textContent).toContain("design");
  expect(attach()?.disabled).toBe(false);
  expect(gif()).not.toBeNull();
  expect(presenceDot()?.dataset.status).toBe("online");
  const betaVoiceJoin = container.querySelector<HTMLButtonElement>('button[aria-label="Join voice in #design"]');
  expect(betaVoiceJoin?.getAttribute("aria-disabled")).toBe("false");
  const reactionChip = container.querySelector<HTMLButtonElement>(".chat-reaction");
  expect(reactionChip?.getAttribute("aria-disabled")).toBe("false");
  expect(railButton("Beta").getAttribute("aria-busy")).toBe("false");
  expect(fixture.spaceIconRenders.get("space0000003")).toBe(gammaBefore);
  expect(fixture.avatarRenders.get("Sam")).toBe(samDmBefore);

  // Switch back to cached Alpha: switches immediately without setting aria-busy,
  // and keeps unrelated Gamma tile and Sam DM untouched.
  const gammaBeforeReturn = fixture.spaceIconRenders.get("space0000003") ?? 0;
  const samBeforeReturn = fixture.avatarRenders.get("Sam") ?? 0;
  await act(async () => {
    railButton("Alpha").click();
  });
  expect(container.querySelector(".chat-title-text")?.textContent).toContain("general");
  expect(railButton("Alpha").getAttribute("aria-busy")).toBe("false");
  expect(attach()?.disabled).toBe(false);
  expect(gif()).not.toBeNull();
  expect(presenceDot()?.dataset.status).toBe("online");
  expect(fixture.spaceIconRenders.get("space0000003")).toBe(gammaBeforeReturn);
  expect(fixture.avatarRenders.get("Sam")).toBe(samBeforeReturn);

  // Finish held media request cleanly.
  await act(async () => {
    for (const item of heldMedia) item.finish();
  });
});

test("uncached space switch avoids aria-busy flash on fast loads and shows busy only when slow", async () => {
  vi.useFakeTimers();
  const { heldSpaces } = installFetch({ holdSpace: "space0000002" });

  await act(async () => {
    root.render(
      <Spaces
        initialAccount={account}
        initialSpaceList={{
          spaces,
          invitations: [],
          limits: { ownedSpaces: 20, totalSpaces: 100, channelsPerSpace: 100 },
        }}
        initialDirects={directs}
        initialSelection={{ spaceId: "space0000001", channelId: "chan00000001" }}
        initialView={{
          detail: details.space0000001,
          channelId: "chan00000001",
          history: histories.chan00000001,
        }}
      />,
    );
  });

  const railButton = (name: string) =>
    Array.from(container.querySelectorAll<HTMLButtonElement>(".space-rail-item button")).find((button) =>
      button.getAttribute("aria-label")?.startsWith(name),
    )!;

  await act(async () => {
    railButton("Beta").click();
  });

  // Immediately after click (before 150ms): no busy dim flash yet.
  expect(railButton("Beta").getAttribute("aria-busy")).toBe("false");
  expect(heldSpaces).toHaveLength(1);

  // After 150ms on a slow network: busy indicator activates.
  await act(async () => {
    vi.advanceTimersByTime(160);
  });
  expect(railButton("Beta").getAttribute("aria-busy")).toBe("true");

  // Once the space finishes loading, busy clears and Beta opens.
  await act(async () => {
    heldSpaces[0].finish();
    await Promise.resolve();
  });
  expect(railButton("Beta").getAttribute("aria-busy")).toBe("false");
  expect(container.querySelector(".chat-title-text")?.textContent).toContain("design");
});
