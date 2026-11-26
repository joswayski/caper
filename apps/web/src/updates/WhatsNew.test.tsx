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

test("history lists every retained highlight newest first in one list, naming only older builds", async () => {
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
    expect(container.querySelectorAll("time")).toHaveLength(0);
    expect([...container.querySelectorAll("h3")].map((heading) => heading.textContent)).toEqual([
      "First change",
      "Second change",
      "Earlier update",
    ]);
    // The header names the running build; the list names only the older one.
    const links = [...container.querySelectorAll("a")];
    expect(links).toHaveLength(2);
    expect(links[0].closest("header")).not.toBeNull();
    expect(links[1].textContent).toBe("aaaaaaa");
    expect(links[1].nextElementSibling?.textContent).toContain("Earlier update");
    const buttons = [...container.querySelectorAll("footer button")].map((button) => button.textContent);
    expect(buttons).toEqual(["Done"]);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

test("a version without highlights links its changes on GitHub", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(loadReleaseHighlights).mockResolvedValue({
    schema: 1,
    platform: "web",
    revision: "b".repeat(40),
    sequence: 1,
    releases: [{ sequence: 1, revision: "b".repeat(40), publishedAt: "2026-10-21T12:00:00Z", highlights: [] }],
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    await act(async () => {
      root.render(<WhatsNew ready blocked={false} manualOpen onManualClose={() => {}} />);
    });
    expect(container.querySelector(".whats-new-empty")?.textContent).toBe(
      "No release highlights for this version yet.",
    );
    const link = container.querySelector<HTMLAnchorElement>(".whats-new-changes");
    expect(link?.textContent).toBe("See what changed on GitHub");
    expect(link?.href).toMatch(/^https:\/\/github\.com\/joswayski\/caper\/commits\//);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});

test("a click on the backdrop closes it, but a selection dragged out of the text does not", async () => {
  vi.stubGlobal("IS_REACT_ACT_ENVIRONMENT", true);
  vi.mocked(loadReleaseHighlights).mockResolvedValue({
    schema: 1,
    platform: "web",
    revision: "b".repeat(40),
    sequence: 1,
    releases: [
      {
        sequence: 1,
        revision: "b".repeat(40),
        publishedAt: "2026-10-21T12:00:00Z",
        highlights: [{ title: "A change", body: "Text someone might copy." }],
      },
    ],
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const onManualClose = vi.fn();
  try {
    await act(async () => {
      root.render(<WhatsNew ready blocked={false} manualOpen onManualClose={onManualClose} />);
    });
    const dialog = container.querySelector("dialog")!;
    const press = (down: Element, up: Element) =>
      act(() => {
        down.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
        up.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });
    await press(container.querySelector("article p")!, dialog);
    await press(container.querySelector("article p")!, container.querySelector("article p")!);
    expect(onManualClose).not.toHaveBeenCalled();
    await press(dialog, dialog);
    expect(onManualClose).toHaveBeenCalledTimes(1);
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
