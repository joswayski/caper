// @vitest-environment happy-dom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { expect, test } from "vitest";
import MessageGif from "./MessageGif.tsx";

test("GIFs and stickers open their original in the media viewer; failed MP4 falls back to animation", async () => {
  (globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  try {
    for (const kind of ["gif", "sticker"] as const) {
      const image = `https://static.klipy.com/test.${kind === "gif" ? "gif" : "webp"}`;
      await act(async () =>
        root.render(
          <MessageGif
            key={kind}
            content={{
              gif: {
                provider: "klipy",
                kind,
                slug: "test",
                title: "Test animation",
                width: 320,
                height: 180,
                ...(kind === "gif" ? { gif: image, mp4: "https://static.klipy.com/test.mp4" } : { webp: image }),
              },
            }}
          />,
        ),
      );
      if (kind === "gif") {
        const video = container.querySelector("video")!;
        expect(video.autoplay && video.loop && video.muted).toBe(true);
        await act(async () => container.querySelector("button")!.click());
        expect(document.querySelector<HTMLVideoElement>(".chat-viewer video")?.src).toBe(video.src);
        expect(video.autoplay).toBe(false);
        await act(async () => document.querySelector<HTMLElement>('[aria-label="Close viewer"]')!.click());
        expect(video.autoplay).toBe(true);
        await act(async () => video.dispatchEvent(new Event("error")));
        expect(container.querySelector("img")?.src).toBe(image);
      }
      const button = container.querySelector("button")!;
      expect(button.getAttribute("aria-label")).toBe("Open Test animation in viewer");
      await act(async () => button.click());
      const viewer = document.querySelector(".chat-viewer")!;
      expect(viewer).not.toBeNull();
      expect(viewer.querySelector("img")?.src).toBe(image);
      await act(async () => viewer.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })));
      expect(document.querySelector(".chat-viewer")).toBeNull();
    }
  } finally {
    await act(async () => root.unmount());
    container.remove();
  }
});
