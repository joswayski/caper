import { afterEach, expect, test, vi } from "vitest";
import { unreadChanged } from "../account/app-badge.ts";
import { listUnreadSpaces, startingUnreadSpaces } from "../spaces/client.ts";

afterEach(() => vi.restoreAllMocks());

test("the page's first unread snapshot reuses one just started, but never one from before a read", async () => {
  const fetched = vi
    .spyOn(globalThis, "fetch")
    .mockImplementation(async () => Response.json({ spaces: [], badgeCount: fetched.mock.calls.length }));
  // A channel visit starts a snapshot; the page asks for its first moments later.
  const visit = listUnreadSpaces();
  expect(await startingUnreadSpaces()).toBe(await visit);
  expect(fetched).toHaveBeenCalledTimes(1);

  // A read acknowledged since then changed the counts.
  unreadChanged();
  expect((await startingUnreadSpaces()).badgeCount).toBe(2);

  // A failed snapshot is not handed on.
  fetched.mockImplementationOnce(async () => new Response("{}", { status: 503 }));
  await expect(listUnreadSpaces()).rejects.toThrow();
  expect((await startingUnreadSpaces()).badgeCount).toBe(4);

  // Neither is an old one.
  vi.useFakeTimers();
  try {
    vi.advanceTimersByTime(5_000);
    expect((await startingUnreadSpaces()).badgeCount).toBe(5);
  } finally {
    vi.useRealTimers();
  }
});
