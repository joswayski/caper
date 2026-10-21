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

test("history shows one source link before each release's highlights, including same-day releases", async () => {
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
    const releases = [...container.querySelectorAll(".whats-new-entry")];
    expect(releases).toHaveLength(2);
    for (const [index, revision] of ["b", "a"].entries()) {
      const release = releases[index];
      const links = release.querySelectorAll("a");
      expect(links).toHaveLength(1);
      expect(links[0].href).toBe(`https://github.com/joswayski/caper/commit/${revision.repeat(40)}`);
      expect(links[0].textContent).toBe(`Web build ${revision.repeat(7)}`);
      const highlights = [...release.querySelectorAll("h3")];
      expect(highlights.map((heading) => heading.textContent)).toEqual(
        index === 0 ? ["First change", "Second change"] : ["Earlier update"],
      );
      expect(links[0].compareDocumentPosition(highlights[0]) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    }
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
