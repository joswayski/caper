import { useCallback, useLayoutEffect, useRef, type RefObject } from "react";
import { ARRIVAL_MS } from "./arrivals.ts";

/** Keep live-end layout corrections instant, but carry the painted rows from
 * their previous positions. This also covers short, bottom-aligned histories,
 * where scrollTop never changes. User scrolling and history loads stay instant.
 * Link preview cards that load into a row count as arrivals too. Returns
 * `carry`: call it as a change outside the list resizes it, before layout (the
 * composer's preview card), so the rows glide instead of jumping. */
export function useTimelineMotion(
  ref: RefObject<HTMLElement | null>,
  identity: string | undefined,
  live: boolean,
  mounted?: HTMLElement | null,
) {
  const carryRef = useRef<() => void>(undefined);
  useLayoutEffect(() => {
    const scroller = ref.current;
    if (!scroller || !live) return;
    const reduced = matchMedia("(prefers-reduced-motion: reduce)");
    const tokens = getComputedStyle(scroller);
    const duration = parseFloat(tokens.getPropertyValue("--motion-slow")) || 260;
    const easing = tokens.getPropertyValue("--ease-out").trim() || "cubic-bezier(0.16, 1, 0.3, 1)";
    let positions = new Map<string, number>();
    const seen = new Set<string>();
    const cards = new WeakSet<Element>();
    const animations = new Map<HTMLElement, Animation>();
    let until = 0;
    let watchUntil = 0;
    let pinUntil = 0;
    let frame = 0;
    let near = true;
    // More than a screen above the live end nothing can glide, so skip the
    // per-row layout reads that would otherwise run on every scrolled frame.
    let far = false;
    const rows = () => [...scroller.querySelectorAll<HTMLElement>("[data-motion-key]")];
    const shift = (row: HTMLElement) => {
      const transform = getComputedStyle(row).transform;
      return transform === "none" ? 0 : new DOMMatrixReadOnly(transform).m42;
    };
    const stop = () => {
      until = 0;
      watchUntil = 0;
      cancelAnimationFrame(frame);
      frame = 0;
      for (const animation of animations.values()) animation.cancel();
      animations.clear();
    };
    const measure = () => {
      if (far) return;
      const now = performance.now();
      // Hold the live end before reading rows, so a carried resize glides from
      // where the rows were painted rather than first jumping to the end.
      if (now < pinUntil && near) scroller.scrollTop = scroller.scrollHeight;
      const current = rows();
      const bounds = scroller.getBoundingClientRect();
      const scale = bounds.height / scroller.offsetHeight || 1;
      const next = new Map<string, number>();
      let delta: number | undefined;
      let carried = 0;
      let arriving = false;
      for (const row of current) {
        const key = row.dataset.motionKey!;
        const offset = shift(row);
        const top = (row.getBoundingClientRect().top - bounds.top) / scale - offset;
        next.set(key, top);
        if (!seen.has(key) && row.querySelector(".chat-message-arrive")) arriving = true;
        seen.add(key);
        // Use a retained row, never the incoming row's independent entrance.
        if (delta === undefined && positions.has(key)) {
          delta = positions.get(key)! - top;
          carried = offset;
        }
      }
      for (const card of scroller.querySelectorAll(".chat-embed[data-arrive]"))
        if (!cards.has(card)) {
          cards.add(card);
          arriving = true;
        }
      if (arriving && near && !reduced.matches) {
        until = now + duration;
        // Confirmation can remeasure a pending row after its entrance ends.
        // Keep observing it without running animation frames while at rest.
        watchUntil = now + (current.some((row) => row.querySelector(".chat-message-pending")) ? ARRIVAL_MS : duration);
      }
      if (now < watchUntil && delta !== undefined && Math.abs(delta) > 0.25) {
        const from = carried + delta;
        for (const row of current) {
          animations.get(row)?.cancel();
          animations.set(
            row,
            row.animate([{ transform: `translateY(${from}px)` }, { transform: "translateY(0)" }], { duration, easing }),
          );
        }
        until = now + duration;
      }
      positions = next;
      near = scroller.scrollHeight - scroller.clientHeight - scroller.scrollTop <= 80;
      if (now < until) {
        if (!frame)
          frame = requestAnimationFrame(() => {
            frame = 0;
            measure();
          });
      } else {
        for (const animation of animations.values()) animation.cancel();
        animations.clear();
      }
    };
    // Observe only DOM/layout changes: idle timelines do no per-frame work.
    const resize = new ResizeObserver(measure);
    const changed = () => {
      if (far) return;
      resize.disconnect();
      resize.observe(scroller);
      for (const row of rows()) resize.observe(row);
      measure();
    };
    const mutations = new MutationObserver(changed);
    changed();
    carryRef.current = () => {
      if (!near || reduced.matches) return;
      const now = performance.now();
      watchUntil = Math.max(watchUntil, now + duration);
      // Only the resize this call announces, which is measured within a frame.
      pinUntil = now + 50;
    };
    mutations.observe(scroller, { childList: true, subtree: true });
    const interrupt = () => {
      stop();
      measure();
    };
    // A click must keep its painted target; cancelling on pointerdown would
    // move that target away before pointerup. Scrollbars are hidden here.
    const inputs = ["wheel", "touchmove", "keydown"] as const;
    for (const input of inputs) scroller.addEventListener(input, interrupt, { passive: true });
    const scrolled = () => {
      const wasFar = far;
      far = scroller.scrollHeight - scroller.scrollTop > 2 * scroller.clientHeight;
      if (far && !wasFar) {
        // Coming back measures afresh, so the scroll itself never glides.
        stop();
        resize.disconnect();
        resize.observe(scroller);
        positions = new Map();
        near = false;
      } else if (!far) {
        if (wasFar) changed();
        else measure();
      }
    };
    scroller.addEventListener("scroll", scrolled, { passive: true });
    reduced.addEventListener("change", interrupt);
    return () => {
      carryRef.current = undefined;
      stop();
      resize.disconnect();
      mutations.disconnect();
      for (const input of inputs) scroller.removeEventListener(input, interrupt);
      scroller.removeEventListener("scroll", scrolled);
      reduced.removeEventListener("change", interrupt);
    };
  }, [ref, identity, live, mounted]);
  return useCallback(() => carryRef.current?.(), []);
}
