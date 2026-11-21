// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test, vi } from "vitest";
import JumpToLatest from "./JumpToLatest.tsx";

test("appears half a screen above the live end, counts other people's new messages, and leaves at the bottom", async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  // A 400px viewport over 2,000px of messages.
  const scroller = document.createElement("div");
  let top = 1600;
  let height = 2000;
  Object.defineProperties(scroller, {
    scrollHeight: { get: () => height },
    clientHeight: { get: () => 400 },
    scrollTop: { get: () => top },
  });
  const scrollTo = async (value: number) => {
    top = value;
    await act(async () => scroller.dispatchEvent(new Event("scroll")));
  };
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const jumped = vi.fn();
  const message = (id: number, author: string) => ({ clientMessageId: `m${id}`, author: { id: author } });
  let messages = [message(1, "other"), message(2, "viewer")];
  const render = () =>
    act(async () =>
      root.render(
        <JumpToLatest
          scroller={scroller}
          messages={messages}
          viewerId="viewer"
          blockedIds={new Set(["blocked"])}
          onJump={jumped}
        />,
      ),
    );
  const button = () => container.querySelector<HTMLButtonElement>(".chat-jump-latest");

  await render();
  expect(button()).toBeNull();
  // 200px away is exactly half a screen: still part of reading the newest messages.
  await scrollTo(1400);
  expect(button()).toBeNull();
  await scrollTo(1399);
  expect(button()?.getAttribute("aria-label")).toBe("Jump to latest");
  expect(button()?.textContent).toBe("");

  // Older history loading above, the viewer's own messages and blocked people are not new.
  messages = [message(0, "other"), ...messages, message(3, "viewer"), message(4, "blocked")];
  await render();
  expect(button()?.getAttribute("aria-label")).toBe("Jump to latest");
  messages = [...messages, message(5, "other")];
  await render();
  expect(button()?.textContent).toBe("1 new message");
  messages = [...messages, message(6, "other")];
  await render();
  expect(button()?.getAttribute("aria-label")).toBe("Jump to latest, 2 new messages");
  expect(button()?.hasAttribute("data-new")).toBe(true);

  // Three screens or fewer away scrolls smoothly; farther jumps.
  await act(async () => button()!.click());
  expect(jumped).toHaveBeenLastCalledWith(true);
  await scrollTo(0);
  await act(async () => button()!.click());
  expect(jumped).toHaveBeenLastCalledWith(false);

  // Back at the live end the button and its count go; leaving again starts from zero.
  await scrollTo(1600);
  expect(button()).toBeNull();
  await scrollTo(1000);
  expect(button()?.textContent).toBe("");

  // Messages that arrive under a reader who is not scrolling can push the live end away too.
  await scrollTo(1500);
  expect(button()).toBeNull();
  height = 2400;
  messages = [...messages, message(7, "other"), message(8, "other")];
  await render();
  await act(async () => void (await new Promise((resolve) => requestAnimationFrame(resolve))));
  expect(button()).not.toBeNull();

  act(() => root.unmount());
  container.remove();
});
