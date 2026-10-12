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
import { clampProgress, movesOf, settlesForward, SLOP, Velocity } from "../components/swipe";
import { BLOCKING, browseSliding, OWN_GESTURES, releaseTiming, SETTLE } from "./browseTransition";

// On phones the member list is a drawer over the conversation. Like Browse, it
// slides in and out with the same timing and follows a finger: left on the
// conversation opens it, and right on the panel or backdrop closes it. A touch
// on the moving drawer stops it under the finger: dragging carries on from
// there, and a tap lets it finish without activating anything.

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
  /** Where the running slide is going (true: shown), while one runs. */
  const heading = useRef<boolean>(undefined);
  /** Where the next slide starts (0 hidden, 1 shown) and its speed toward its end, in progress per millisecond. */
  const start = useRef({ from: 0, velocity: 0, interactive: false });
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
  /** Holds the panel `progress` of the way in, as a finger moves it `length` pixels. */
  const pose = (progress: number, length: number) => {
    const { panel, backdrop } = parts();
    if (!panel) return;
    panel.style.transform = `translateX(${(1 - progress) * length}px)`;
    if (backdrop) backdrop.style.opacity = String(progress);
  };
  const reset = () => {
    for (const animation of animations.current) animation.cancel();
    animations.current = [];
    heading.current = undefined;
    const { panel, backdrop } = parts();
    panel?.style.removeProperty("transform");
    backdrop?.style.removeProperty("opacity");
  };

  /** Slides to shown or hidden from `start`; resolves false when a newer slide or drag takes over. */
  const slide = (show: boolean) => {
    const { from, velocity, interactive } = start.current;
    start.current = { from: 0, velocity: 0, interactive: false };
    reset();
    const { room, panel, backdrop } = parts();
    if (!room || !panel) return Promise.resolve(true);
    // A closing panel keeps its place but no longer takes focus. Its backdrop
    // still takes touches, so one meant for the moving drawer catches it
    // rather than landing on the conversation beneath.
    panel.inert = !show;
    if (backdrop) backdrop.inert = false;
    const to = show ? 1 : 0;
    const distance = Math.abs(to - from);
    if (still() || distance === 0) return Promise.resolve(true);
    // Browse's timing: a release carries on at the finger's speed.
    const timing = interactive
      ? releaseTiming(distance, velocity)
      : { ...SETTLE, duration: Math.max(160, SETTLE.duration * distance) };
    const length = travel(room, panel);
    const place = (progress: number) => ({ transform: `translateX(${(1 - progress) * length}px)` });
    const current = [panel.animate([place(from), place(to)], { ...timing, fill: "both" })];
    if (backdrop) current.push(backdrop.animate([{ opacity: from }, { opacity: to }], { ...timing, fill: "both" }));
    animations.current = current;
    heading.current = show;
    return Promise.all(current.map((animation) => animation.finished)).then(
      () => animations.current === current,
      () => false,
    );
  };

  const settle = (show: boolean) =>
    void slide(show).then((done) => {
      if (!done) return;
      heading.current = undefined;
      if (show) reset();
      // Unmounting removes the panel with its final pose.
      else setClosing(false);
    });

  /** Stops a running slide where it is, holding the panel there for a finger. */
  const grab = () => {
    const { room, panel } = parts();
    const resume = heading.current;
    if (resume === undefined || !room || !panel) return undefined;
    const progress = shown();
    const length = travel(room, panel);
    for (const animation of animations.current) animation.cancel();
    animations.current = [];
    heading.current = undefined;
    pose(progress, length);
    return { progress, resume, length };
  };

  /** Settles on shown or hidden from `from`, carrying on at `velocity` (progress per millisecond toward that end). */
  const finish = (show: boolean, from: number, velocity: number) => {
    dragging.current = false;
    start.current = { from, velocity, interactive: true };
    const { open, setOpen } = latest.current;
    if (show === open) settle(show);
    else {
      setClosing(!show && !still());
      setOpen(show);
    }
  };

  const change = (next: boolean) => {
    const { enabled, open, setOpen } = latest.current;
    if (next === open) return;
    const { panel } = parts();
    const slides = enabled && !still();
    start.current = { from: slides && panel ? shown() : 0, velocity: 0, interactive: false };
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
        // Browse's slide owns touches while it moves.
        if (!enabled || event.pointerType !== "touch" || !event.isPrimary || browseSliding()) return;
        const { panel, backdrop } = parts();
        const caught = target === backdrop || panel?.contains(target) ? grab() : undefined;
        if (
          !caught &&
          (document.querySelector(BLOCKING) ||
            window.getSelection()?.type === "Range" ||
            target.closest(OWN_GESTURES) ||
            // The open drawer owns the whole room, including its backdrop.
            // Closed, only the conversation opens it.
            !(open ? room : room.querySelector(":scope > .stage"))?.contains(target))
        )
          return;
        if (caught) dragging.current = true;
        const id = event.pointerId,
          startX = event.clientX,
          startY = event.clientY,
          base = caught?.progress ?? (open ? 1 : 0);
        let length = caught?.length ?? 1;
        /** Where the finger was, toward shown, when the drag began; the panel follows from there. */
        let origin: number | undefined;
        const speed = new Velocity();
        // Moving left shows more of the panel.
        const along = (point: { clientX: number }) => startX - point.clientX;
        const progressAt = (point: { clientX: number }) => clampProgress(base + (along(point) - origin!) / length);

        const end = (event?: PointerEvent) => {
          window.removeEventListener("pointermove", move);
          window.removeEventListener("pointerup", lift);
          window.removeEventListener("pointercancel", lift);
          document.removeEventListener("visibilitychange", cancel);
          if (stop.current === cancel) stop.current = undefined;
          if (origin === undefined) {
            // A tap or scroll on the caught drawer lets it carry on; the tap is not a click.
            if (caught) {
              suppressClick.current = true;
              finish(caught.resume, caught.progress, 0);
            }
            return;
          }
          const released = event?.type === "pointerup";
          const travelled = event ? along(event) : 0;
          if (event) speed.add(event.timeStamp, travelled);
          const velocity = event ? speed.speed(event.timeStamp) : 0;
          const show = released
            ? settlesForward(progressAt(event), velocity, travelled)
            : // Interrupted: back where it started, or on to where a caught drawer was going.
              (caught?.resume ?? open);
          finish(show, shown(), (show ? velocity : -velocity) / length);
        };
        const cancel = () => end();
        const lift = (event: PointerEvent) => {
          if (event.pointerId === id) end(event);
        };
        const move = (event: PointerEvent) => {
          if (event.pointerId !== id) return;
          const dx = along(event),
            dy = event.clientY - startY;
          if (origin === undefined) {
            if (Math.abs(dy) > Math.max(12, Math.abs(dx))) return end();
            if (Math.abs(dx) < SLOP) return;
            // A caught drawer can go either way; otherwise only toward the other state.
            if ((!caught && (open ? dx > 0 : dx < 0)) || Math.abs(dx) < Math.abs(dy) * 1.5) return end();
            if (!caught && (document.querySelector(BLOCKING) || window.getSelection()?.type === "Range")) return end();
            origin = dx;
            dragging.current = true;
            suppressClick.current = true;
            if (!caught) {
              // Opening mounts the panel first, then holds it off screen.
              if (!open) {
                setClosing(false);
                flushSync(() => latest.current.setOpen(true));
              }
              for (const animation of animations.current) animation.cancel();
              animations.current = [];
              heading.current = undefined;
              const { room, panel } = parts();
              if (!room || !panel) return end();
              length = travel(room, panel);
            }
            const { panel, backdrop } = parts();
            if (panel) panel.inert = false;
            if (backdrop) backdrop.inert = false;
          }
          for (const point of movesOf(event)) speed.add(point.timeStamp, along(point));
          pose(progressAt(event), length);
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
        // A touch drag may end without a click. Keyboard and assistive
        // activations (no pointer, so detail 0) are never a drag's release.
        if (event.detail === 0) return;
        event.preventDefault();
        event.stopPropagation();
      },
    },
  };
}
