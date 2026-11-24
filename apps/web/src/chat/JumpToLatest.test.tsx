// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test, vi } from "vitest";
import JumpToLatest from "./JumpToLatest.tsx";

test("appears ten messages above the live end, counts other people's new messages, and leaves at the bottom", async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // A 400px viewport over 100px rows, 2,000px of them, rendered like a virtual list: only rows near the viewport.
  const scroller = document.createElement("div");
  let top = 1600;
  let height = 2000;
  Object.defineProperties(scroller, {
    scrollHeight: { get: () => height },
    clientHeight: { get: () => 400 },
    scrollTop: { get: () => top },
  });
  scroller.getBoundingClientRect = () => new DOMRect(0, 0, 300, 400);
  const renderRows = () => {
    scroller.replaceChildren();
    for (let index = Math.max(0, Math.floor((top - 100) / 100)); index * 100 < Math.min(height, top + 500); index++) {
      const row = document.createElement("div");
      row.dataset.itemIndex = String(index);
      row.getBoundingClientRect = () => new DOMRect(0, index * 100 - top, 300, 100);
      scroller.append(row);
    }
  };
  const scrollTo = async (value: number) => {
    top = value;
    renderRows();
    await act(async () => scroller.dispatchEvent(new Event("scroll")));
  };
  renderRows();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const jumped = vi.fn();
  const message = (id: number, author: string) => ({ clientMessageId: `m${id}`, author: { id: author } });
  let messages = [message(1, "other"), message(2, "viewer")];
  let detached = false;
  const render = () =>
    act(async () =>
      root.render(
        <JumpToLatest
          scroller={scroller}
          messages={messages}
          viewerId="viewer"
          blockedIds={new Set(["blocked"])}
          detached={detached}
          onJump={jumped}
        />,
      ),
    );
  const button = () => container.querySelector<HTMLButtonElement>(".chat-jump-latest button");
  const toast = () => container.querySelector(".chat-jump-latest > span")?.textContent;

  await render();
  expect(button()).toBeNull();
  // Nine rows below the viewport (900px, over two screens) is still reading the newest messages.
  await scrollTo(700);
  expect(button()).toBeNull();
  // The tenth, counted past the rendered rows from their average height.
  await scrollTo(600);
  expect(button()?.getAttribute("aria-label")).toBe("Jump to latest");
  expect(toast()).toBe("You’re viewing older messages");
  // Coming most of the way back keeps it until the bottom.
  await scrollTo(1500);
  expect(button()).not.toBeNull();
  await scrollTo(1000);

  // Older history loading above, the viewer's own messages and blocked people are not new.
  messages = [message(0, "other"), ...messages, message(3, "viewer"), message(4, "blocked")];
  await render();
  expect(button()?.getAttribute("aria-label")).toBe("Jump to latest");
  messages = [...messages, message(5, "other")];
  await render();
  expect(toast()).toBe("1 new message");
  messages = [...messages, message(6, "other")];
  await render();
  expect(button()?.getAttribute("aria-label")).toBe("Jump to latest, 2 new messages");
  expect(container.querySelector(".chat-jump-latest")?.hasAttribute("data-new")).toBe(true);

  // Three screens or fewer away scrolls smoothly; farther jumps.
  await act(async () => button()!.click());
  expect(jumped).toHaveBeenLastCalledWith(true);
  await scrollTo(0);
  await act(async () => button()!.click());
  expect(jumped).toHaveBeenLastCalledWith(false);

  // Back at the live end the button and its count go; leaving again starts from zero.
  await scrollTo(1600);
  expect(button()).toBeNull();
  await scrollTo(500);
  expect(toast()).toBe("You’re viewing older messages");

  // Messages that arrive under a reader who is not scrolling can push the live end away too.
  await scrollTo(1600);
  await scrollTo(1500);
  expect(button()).toBeNull();
  height = 3000;
  renderRows();
  messages = [...messages, message(7, "other"), message(8, "other")];
  await render();
  await act(async () => void (await new Promise((resolve) => requestAnimationFrame(resolve))));
  expect(button()).not.toBeNull();

  // A jump's window short of the live end shows the toast even at its bottom, and jumps without a glide.
  await scrollTo(2600);
  expect(button()).toBeNull();
  detached = true;
  await render();
  expect(toast()).toBe("You’re viewing older messages");
  await act(async () => button()!.click());
  expect(jumped).toHaveBeenLastCalledWith(false);

  act(() => root.unmount());
  container.remove();
});
