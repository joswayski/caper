import { afterEach, beforeEach, expect, test, vi } from "vitest";
import { setAccountFeedLive, shellPoll } from "../account/shell-poll.ts";

beforeEach(() => vi.useFakeTimers());
afterEach(() => {
  setAccountFeedLive(false);
  vi.useRealTimers();
});

test("the shell poll runs every 15 seconds without the live feed and every minute with it", () => {
  const run = vi.fn();
  const stop = shellPoll(run);
  vi.advanceTimersByTime(45_000);
  expect(run).toHaveBeenCalledTimes(3);

  setAccountFeedLive(true);
  vi.advanceTimersByTime(59_000);
  expect(run).toHaveBeenCalledTimes(3);
  vi.advanceTimersByTime(1_000);
  expect(run).toHaveBeenCalledTimes(4);
  vi.advanceTimersByTime(120_000);
  expect(run).toHaveBeenCalledTimes(6);

  // Losing the feed makes the poll the only source again, at its next tick.
  setAccountFeedLive(false);
  vi.advanceTimersByTime(15_000);
  expect(run).toHaveBeenCalledTimes(7);

  stop();
  vi.advanceTimersByTime(60_000);
  expect(run).toHaveBeenCalledTimes(7);
});
