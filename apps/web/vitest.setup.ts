import { beforeEach, vi } from "vitest";

beforeEach(({ onTestFinished }) => {
  // Registered first, so this runs after each test's own resource cleanup.
  // Keep real setImmediate available for flushing promises under fake timers.
  onTestFinished(() => {
    vi.restoreAllMocks();
    vi.unstubAllGlobals();
    vi.useRealTimers();
  });
});
