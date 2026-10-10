import {
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { flushSync } from "react-dom";
import { BLOCKING, OWN_GESTURES, RELEASE_EASING, SETTLE } from "./browseTransition";

// On phones the member list is a drawer over the conversation. Like Browse, it
// slides in and out with the same timing and follows a finger: left on the
// conversation opens it, and right on the panel closes it.

const PANEL = ":scope > .space-member-presence";
const BACKDROP = ":scope > .member-list-backdrop";
/** Extra travel past the screen edge, so the panel's shadow leaves with it. */
const SHADOW = 48;

function still() {
  return matchMedia("(prefers-reduced-motion: reduce)").matches;
}

export interface MembersDrawer {
  /** True while the panel slides out, so it stays mounted until it is gone. */
  closing: boolean;
  /** Opens or closes the member list, sliding it on phones. */
  change(open: boolean): void;
  /** Touch swipes on the room. */
  swipe: {
    onPointerDownCapture(event: ReactPointerEvent<HTMLElement>): void;
    onClickCapture(event: ReactMouseEvent): void;
  };
}

export function useMembersDrawer(
  roomRef: RefObject<HTMLElement | null>,
  enabled: boolean,
  open: boolean,
  setOpen: (open: boolean) => void,
): MembersDrawer {
  const [closing, setClosingState] = useState(false);
  const closingRef = useRef(false);
  const setClosing = (value: boolean) => {
    closingRef.current = value;
    setClosingState(value);
  };
  const latest = useRef({ enabled, open, setOpen });
  latest.current = { enabled, open, setOpen };
  const animations = useRef<Animation[]>([]);
  /** Where the next slide starts (0 hidden, 1 shown) and its speed toward its end, in progress per millisecond. */
  const start = useRef({ from: 0, velocity: 0 });
  /** Set while a finger poses the panel, so state changes do not also animate it. */
  const dragging = useRef(false);
  const stop = useRef<() => void>(undefined);
  const suppressClick = useRef(false);

  const parts = () => {
    const room = roomRef.current;
    return {
      room,
      panel: room?.querySelector<HTMLElement>(PANEL) ?? undefined,
      backdrop: room?.querySelector<HTMLElement>(BACKDROP) ?? undefined,
    };
  };
  /** How far the panel moves between shown and hidden. */
  const travel = (room: HTMLElement, panel: HTMLElement) => Math.max(1, room.clientWidth - panel.offsetLeft + SHADOW);
  /** How much of the panel is in view, including partway through a slide. */
  const shown = () => {
    const { room, panel } = parts();
    if (!room || !panel) return 0;
    const offset = new DOMMatrixReadOnly(getComputedStyle(panel).transform).m41;
    return Math.min(1, Math.max(0, 1 - offset / travel(room, panel)));
  };
  /** Holds the panel `progress` of the way in, as a finger moves it. */
  const pose = (progress: number) => {
    const { room, panel, backdrop } = parts();
    if (!room || !panel) return;
    panel.style.transform = `translateX(${(1 - progress) * travel(room, panel)}px)`;
    if (backdrop) backdrop.style.opacity = String(progress);
  };
  const reset = () => {
    for (const animation of animations.current) animation.cancel();
    animations.current = [];
    const { panel, backdrop } = parts();
    panel?.style.removeProperty("transform");
    backdrop?.style.removeProperty("opacity");
  };

  /** Slides to shown or hidden from `start`; resolves false when a newer slide or drag takes over. */
  const slide = (show: boolean) => {
    const { from, velocity } = start.current;
    start.current = { from: 0, velocity: 0 };
    reset();
    const { room, panel, backdrop } = parts();
    if (!room || !panel) return Promise.resolve(true);
    // A closing panel keeps its place but no longer takes focus or touches.
    panel.inert = !show;
    if (backdrop) backdrop.inert = !show;
    const to = show ? 1 : 0;
    const distance = Math.abs(to - from);
    if (still() || distance === 0) return Promise.resolve(true);
    // Browse's timing: a release carries on at the finger's speed.
    const timing =
      velocity > 0
        ? { duration: Math.min(SETTLE.duration, Math.max(160, (3 * distance) / velocity)), easing: RELEASE_EASING }
        : { duration: Math.max(160, SETTLE.duration * distance), easing: SETTLE.easing };
    const length = travel(room, panel);
    const place = (progress: number) => ({ transform: `translateX(${(1 - progress) * length}px)` });
    const current = [panel.animate([place(from), place(to)], { ...timing, fill: "both" })];
    if (backdrop) current.push(backdrop.animate([{ opacity: from }, { opacity: to }], { ...timing, fill: "both" }));
    animations.current = current;
    return Promise.all(current.map((animation) => animation.finished)).then(
      () => animations.current === current,
      () => false,
    );
  };

  const settle = (show: boolean) =>
    void slide(show).then((done) => {
      if (!done) return;
      if (show) reset();
      // Unmounting removes the panel with its final pose.
      else setClosing(false);
    });

  const change = (next: boolean) => {
    const { enabled, open, setOpen } = latest.current;
    if (next === open) return;
    const { panel } = parts();
    const slides = enabled && !still();
    start.current = { from: slides && panel ? shown() : 0, velocity: 0 };
    setClosing(!next && slides && !!panel);
    setOpen(next);
  };

  // The header button, Close and the backdrop change state; the panel follows.
  const shownBefore = useRef(open);
  useLayoutEffect(() => {
    if (shownBefore.current === open) return;
    shownBefore.current = open;
    if (!enabled || dragging.current) return;
    settle(open);
  }, [open]);

  useEffect(() => {
    if (enabled) return;
    stop.current?.();
    reset();
    if (closingRef.current) setClosing(false);
  }, [enabled]);
  useEffect(() => () => stop.current?.(), []);

  return {
    closing,
    change,
    swipe: {
      onPointerDownCapture(event: ReactPointerEvent<HTMLElement>) {
        suppressClick.current = false;
        stop.current?.();
        const { enabled, open } = latest.current;
        const target = event.target as HTMLElement;
        const room = event.currentTarget;
        const { panel } = parts();
        if (
          !enabled ||
          event.pointerType !== "touch" ||
          !event.isPrimary ||
          document.querySelector(BLOCKING) ||
          window.getSelection()?.type === "Range" ||
          target.closest(OWN_GESTURES) ||
          // Open, the panel itself closes; closed, the conversation opens it.
          !(open ? panel : room.querySelector(":scope > .stage"))?.contains(target)
        )
          return;
        const id = event.pointerId,
          startX = event.clientX,
          startY = event.clientY,
          // Closing moves the panel right; opening, left.
          direction = open ? 1 : -1;
        let length = 1;
        let moving = false;
        let samples: Array<{ time: number; distance: number }> = [];
        const progressAt = (distance: number) =>
          Math.min(1, Math.max(0, open ? 1 - distance / length : distance / length));

        const end = (event?: PointerEvent) => {
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", lift);
          window.removeEventListener("pointercancel", lift);
          document.removeEventListener("visibilitychange", cancel);
          if (stop.current === cancel) stop.current = undefined;
          if (!moving) return;
          dragging.current = false;
          const released = event?.type === "pointerup";
          const distance = event ? Math.max(0, (event.clientX - startX) * direction) : 0;
          // Speed over the last 100ms, so a pause before lifting is not a fling.
          const recent = event
            ? [...samples, { time: event.timeStamp, distance }].filter((sample) => event.timeStamp - sample.time <= 100)
            : [];
          const first = recent[0],
            last = recent[recent.length - 1];
          const velocity =
            first && last.time - first.time >= 8 ? (last.distance - first.distance) / (last.time - first.time) : 0;
          const commit = released && velocity > -0.3 && (distance >= length / 2 || (velocity >= 0.3 && distance >= 40));
          const show = open !== commit;
          start.current = {
            from: shown(),
            velocity: (commit ? velocity : -velocity) / length,
          };
          if (show === latest.current.open) settle(show);
          else {
            setClosing(!show && !still());
            latest.current.setOpen(show);
          }
        };
        const cancel = () => end();
        const lift = (event: PointerEvent) => {
          if (event.pointerId === id) end(event);
        };
        const move = (event: PointerEvent) => {
          if (event.pointerId !== id) return;
          const dx = (event.clientX - startX) * direction,
            dy = event.clientY - startY;
          if (!moving) {
            if (Math.abs(dy) > Math.max(12, Math.abs(dx))) return end();
            if (Math.abs(dx) < 10) return;
            if (dx < 0 || Math.abs(dx) < Math.abs(dy) * 1.5) return end();
            if (document.querySelector(BLOCKING) || window.getSelection()?.type === "Range") return end();
            moving = true;
            dragging.current = true;
            suppressClick.current = true;
            // Opening mounts the panel first, then holds it off screen.
            if (!open) {
              setClosing(false);
              flushSync(() => latest.current.setOpen(true));
            }
            for (const animation of animations.current) animation.cancel();
            animations.current = [];
            const { room, panel } = parts();
            if (!room || !panel) return end();
            panel.inert = false;
            length = travel(room, panel);
          }
          const distance = Math.max(0, dx);
          const moves = event.getCoalescedEvents?.() ?? [];
          samples = [
            ...samples.filter((sample) => event.timeStamp - sample.time <= 100),
            ...(moves.length ? moves : [event]).map((move) => ({
              time: move.timeStamp,
              distance: Math.max(0, (move.clientX - startX) * direction),
            })),
          ];
          pose(progressAt(distance));
        };
        window.addEventListener("pointermove", move);
        window.addEventListener("pointerup", lift);
        window.addEventListener("pointercancel", lift);
        document.addEventListener("visibilitychange", cancel);
        stop.current = cancel;
      },
      onClickCapture(event: ReactMouseEvent) {
        if (!suppressClick.current) return;
        suppressClick.current = false;
        event.preventDefault();
        event.stopPropagation();
      },
    },
  };
}
