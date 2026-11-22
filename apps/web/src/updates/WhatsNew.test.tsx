// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test, vi } from "vitest";
import WhatsNew from "./WhatsNew";
import { loadReleaseHighlights } from "./whats-new";

vi.mock("./whats-new", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./whats-new")>()),
  sourceRevisionValid: () => true,
  loadReleaseHighlights: vi.fn(),
}));

test("history lists every retained highlight newest first under its day, without a build per release", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(loadReleaseHighlights).mockResolvedValue({
    schema: 1,
    platform: "web",
    revision: "b".repeat(40),
    sequence: 2,
    releases: [
      {
        sequence: 1,
        revision: "a".repeat(40),
        publishedAt: "2026-10-21T09:00:00Z",
        highlights: [{ title: "Earlier update", body: "A different release on the same day." }],
      },
      {
        sequence: 2,
        revision: "b".repeat(40),
        publishedAt: "2026-10-21T12:00:00Z",
        highlights: [
          { title: "First change", body: "One change in the newer release." },
          { title: "Second change", body: "Another change in the same release." },
        ],
      },
    ],
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => {
      root.render(<WhatsNew ready blocked={false} manualOpen onManualClose={() => {}} />);
    });
    expect(container.querySelectorAll("time")).toHaveLength(1);
    expect([...container.querySelectorAll("h3")].map((heading) => heading.textContent)).toEqual([
      "First change",
      "Second change",
      "Earlier update",
    ]);
    // Only the header names a build: the one running now.
    const links = [...container.querySelectorAll("a")];
    expect(links).toHaveLength(1);
    expect(links[0].closest("header")).not.toBeNull();
    const buttons = [...container.querySelectorAll("footer button")].map((button) => button.textContent);
    expect(buttons).toEqual(["Done"]);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
