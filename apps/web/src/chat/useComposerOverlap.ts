import { useLayoutEffect, type RefObject } from "react";

/**
 * The composer floats over the end of its conversation on a frosted backdrop
 * (see .chat-composer in chat.css). Publish its height to the panel as
 * --composer-height, which the list reserves at its end, and keep the newest
 * message in view when the composer grows over it, as a shrinking list did.
 */
export function useComposerOverlap(
  composer: HTMLElement | null,
  scroller: HTMLElement | null,
  follow: RefObject<boolean>,
) {
  useLayoutEffect(() => {
    const panel = composer?.parentElement;
    if (!composer || !panel) return;
    const observer = new ResizeObserver(([entry]) => {
      // Before the list grows: the reader's position against the old height.
      const atBottom = !!scroller && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 80;
      // Layout size, unaffected by the homepage window's transform scale.
      panel.style.setProperty("--composer-height", `${entry.borderBoxSize[0].blockSize}px`);
      if (atBottom && follow.current) scroller.scrollTop = scroller.scrollHeight;
    });
    observer.observe(composer);
    return () => observer.disconnect();
  }, [composer, scroller, follow]);
}
