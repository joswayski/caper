import { useRef, type TouchEventHandler } from "react";

/** Only a vertical timeline drag dismisses its composer; automatic scrolling and taps keep focus. */
export function useTimelineKeyboardDismiss(composerId: string) {
  const start = useRef<{ x: number; y: number } | undefined>(undefined);
  const reset = () => {
    start.current = undefined;
  };
  const onTouchStart: TouchEventHandler = (event) => {
    const touch = event.touches[0];
    start.current =
      event.touches.length === 1 && document.activeElement?.id === composerId
        ? { x: touch.clientX, y: touch.clientY }
        : undefined;
  };
  const onTouchMove: TouchEventHandler = (event) => {
    if (event.touches.length !== 1) return reset();
    if (!start.current) return;
    const dx = Math.abs(event.touches[0].clientX - start.current.x);
    const dy = Math.abs(event.touches[0].clientY - start.current.y);
    if (Math.max(dx, dy) <= 10) return;
    reset();
    // Decide once, after touch slop: horizontal Browse/Members swipes own their gesture.
    const composer = document.getElementById(composerId);
    if (dy > dx && composer === document.activeElement) composer?.blur();
  };
  return { onTouchStart, onTouchMove, onTouchEnd: reset, onTouchCancel: reset };
}
