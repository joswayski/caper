import { useEffect, useRef, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { flushSync } from "react-dom";

// Phones show Browse (spaces and channels) or the conversation, one at a time.
// Moving between them slides the conversation over Browse like a native
// navigation stack, following a finger when swiped. Both views share the
// room's DOM, so a view transition snapshots the outgoing view while the
// incoming one renders live; the pseudo-elements are posed by hand so a swipe
// can scrub them and then settle forward or back.

const NARROW = "(max-width: 760px)";
const NAME = "browse-room";
/** How far Browse sits under the conversation, as a share of its width. */
const PARALLAX = 0.3;
const SETTLE = { duration: 320, easing: "cubic-bezier(0.32, 0.72, 0, 1)" };
/** An ease-out whose initial speed is three times its average, to carry on from a finger. */
const RELEASE_EASING = "cubic-bezier(0.2, 0.6, 0.35, 1)";

export interface BrowseDrag {
  /** Share of the way to the other view, from 0 to 1. */
  move(progress: number): void;
  /** Settles on the other view (`commit`) or back. Velocity is progress per millisecond toward the other view. */
  release(commit: boolean, velocity?: number): void;
}

function animatable() {
  return (
    typeof document !== "undefined" &&
    "startViewTransition" in document &&
    matchMedia(NARROW).matches &&
    !matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** How much of Browse is uncovered, from 0 (conversation) to 1 (Browse). */
function pose(conversation: boolean, uncovered: number): Keyframe {
  return conversation
    ? { transform: `translateX(${uncovered * 100}%)` }
    : { transform: `translateX(${(uncovered - 1) * PARALLAX * 100}%)`, opacity: 0.5 + 0.5 * uncovered };
}

let current: Slide | undefined;

/** Starts a slide; a newer one owns Browse's state from then on. */
function start(opening: boolean, update: (open: boolean) => void, interactive: boolean) {
  if (current) current.superseded = true;
  return (current = new Slide(opening, update, interactive));
}

class Slide implements BrowseDrag {
  private readonly transition: ViewTransition;
  private progress = 0;
  private scrub: Animation[] = [];
  /** Every animation on the transition's pseudo-elements, cancelled when it ends. */
  private readonly animations: Animation[] = [];
  private ready = false;
  private released?: { commit: boolean; velocity: number };
  private ended = false;
  superseded = false;

  constructor(
    private readonly opening: boolean,
    private readonly update: (open: boolean) => void,
    interactive: boolean,
  ) {
    const root = document.documentElement;
    root.dataset.browseTransition = opening ? "open" : "close";
    this.transition = document.startViewTransition(() => flushSync(() => update(opening)));
    if (!interactive) this.released = { commit: true, velocity: 0 };
    this.transition.ready.then(
      () => {
        if (this.ended) return;
        this.ready = true;
        if (this.released) return this.settle();
        try {
          this.scrub = this.animate(0, 1, { duration: 1000, fill: "both" });
          for (const animation of this.scrub) {
            animation.pause();
            animation.currentTime = this.progress * 1000;
          }
        } catch {
          this.end(this.progress >= 0.5);
        }
      },
      () => {
        // Skipped, by a newer slide or a failed capture. The update still runs;
        // undo it for a cancelled swipe unless a newer slide owns the state.
        this.ended = true;
        void this.transition.updateCallbackDone.then(() => {
          if (!this.superseded && this.released && !this.released.commit) this.update(!this.opening);
        });
      },
    );
    void this.transition.finished.finally(() => {
      this.ended = true;
      // Filled poses would otherwise outlive the transition and pose the next one.
      for (const animation of this.animations) animation.cancel();
      if (current !== this) return;
      current = undefined;
      delete root.dataset.browseTransition;
    });
  }

  move(progress: number) {
    if (this.ended) return;
    this.progress = Math.min(1, Math.max(0, progress));
    for (const animation of this.scrub) animation.currentTime = this.progress * 1000;
  }

  release(commit: boolean, velocity = 0) {
    if (this.released) return;
    this.released = { commit, velocity };
    if (this.ended) {
      if (!commit && !this.superseded) void this.transition.updateCallbackDone.then(() => this.update(!this.opening));
    } else if (this.ready) this.settle();
  }

  /** Poses the outgoing (old) and incoming (new) snapshots between two progress values. */
  private animate(from: number, to: number, timing: KeyframeAnimationOptions) {
    const root = document.documentElement;
    const uncovered = (progress: number) => (this.opening ? progress : 1 - progress);
    const animations = (["old", "new"] as const).map((part) => {
      const conversation = (part === "old") === this.opening;
      return root.animate([pose(conversation, uncovered(from)), pose(conversation, uncovered(to))], {
        ...timing,
        pseudoElement: `::view-transition-${part}(${NAME})`,
      });
    });
    this.animations.push(...animations);
    return animations;
  }

  private settle() {
    const { commit, velocity } = this.released!;
    const distance = Math.abs((commit ? 1 : 0) - this.progress),
      speed = commit ? velocity : -velocity;
    const timing =
      speed > 0
        ? { duration: Math.min(SETTLE.duration, Math.max(160, (3 * distance) / speed)), easing: RELEASE_EASING }
        : { duration: Math.max(160, SETTLE.duration * distance), easing: SETTLE.easing };
    try {
      // Settled poses would end the transition before a cancelled swipe
      // restores its view; this keeps it alive until that update is in.
      this.animations.push(
        document.documentElement.animate([{ opacity: 1 }, { opacity: 1 }], {
          duration: timing.duration + 1000,
          pseudoElement: "::view-transition",
        }),
      );
      const settling = this.animate(this.progress, commit ? 1 : 0, { ...timing, fill: "both" });
      for (const animation of this.scrub) animation.cancel();
      this.scrub = [];
      const end = () => this.end(commit);
      Promise.all(settling.map((animation) => animation.finished)).then(end, end);
    } catch {
      this.end(commit);
    }
  }

  /** Lands on the other view (`commit`) or back, and ends the transition. */
  private end(commit: boolean) {
    if (this.ended) return;
    this.ended = true;
    this.released ??= { commit, velocity: 0 };
    if (!commit && !this.superseded) flushSync(() => this.update(!this.opening));
    this.transition.skipTransition();
  }
}

/**
 * Shows or hides Browse. On phones the conversation slides over or off it;
 * elsewhere, or with reduced motion, `update` runs at once.
 */
export function transitionBrowse(open: boolean, update: () => void) {
  if (!animatable()) return update();
  start(open, update, false);
}

/** Starts a swipe between Browse and the conversation, or undefined where it cannot animate. */
export function dragBrowse(opening: boolean, update: (open: boolean) => void): BrowseDrag | undefined {
  if (!animatable()) return undefined;
  return start(opening, update, true);
}

const BLOCKING = "dialog[open], details[open], [role=dialog], [popover]:popover-open";
const OWN_GESTURES =
  "input, textarea, select, [contenteditable=true], [role=slider], a, summary, button:not(.channel-select):not(.direct-select)";

/**
 * Touch swipes on the room: right from the conversation reveals Browse, left
 * from Browse returns. Text entry, links, controls, open menus and selections
 * keep their own gestures. `change` sets Browse at once; the slide animates it.
 */
export function useBrowseSwipe(enabled: boolean, open: boolean, change: ((open: boolean) => void) | undefined) {
  const latest = useRef({ enabled, open, change });
  latest.current = { enabled, open, change };
  const stop = useRef<() => void>(undefined);
  const suppressClick = useRef(false);
  useEffect(() => {
    if (!enabled) stop.current?.();
  }, [enabled]);
  useEffect(() => () => stop.current?.(), []);

  return {
    onPointerDownCapture(event: ReactPointerEvent<HTMLElement>) {
      suppressClick.current = false;
      // A second finger ends the swipe where it is.
      stop.current?.();
      const { enabled, open, change } = latest.current;
      if (
        !enabled ||
        !change ||
        event.pointerType !== "touch" ||
        !event.isPrimary ||
        document.querySelector(BLOCKING) ||
        window.getSelection()?.type === "Range" ||
        (event.target as HTMLElement).closest(OWN_GESTURES)
      )
        return;
      const id = event.pointerId,
        startX = event.clientX,
        startY = event.clientY,
        startTime = event.timeStamp,
        width = event.currentTarget.getBoundingClientRect().width || 1,
        opening = !open,
        direction = opening ? 1 : -1;
      let drag: BrowseDrag | undefined;
      let dragging = false;
      let samples: Array<{ time: number; distance: number }> = [];

      const end = (event?: PointerEvent) => {
        window.removeEventListener("pointermove", move);
        window.removeEventListener("pointerup", lift);
        window.removeEventListener("pointercancel", lift);
        document.removeEventListener("visibilitychange", cancel);
        if (stop.current === cancel) stop.current = undefined;
        if (!dragging) return;
        const released = event?.type === "pointerup";
        const dx = event ? (event.clientX - startX) * direction : 0,
          dy = event ? event.clientY - startY : 0,
          distance = Math.max(0, dx);
        // Speed over the last 100ms, so a pause before lifting is not a fling.
        const recent = event
          ? [...samples, { time: event.timeStamp, distance }].filter((sample) => event.timeStamp - sample.time <= 100)
          : [];
        const first = recent[0],
          last = recent[recent.length - 1];
        const velocity =
          first && last.time - first.time >= 8 ? (last.distance - first.distance) / (last.time - first.time) : 0;
        if (drag)
          drag.release(
            released && velocity > -0.3 && (distance >= width / 2 || (velocity >= 0.3 && distance >= 40)),
            velocity / width,
          );
        // Without a slide, keep the original quick-swipe threshold.
        else if (
          released &&
          distance >= 64 &&
          Math.abs(dx) >= Math.abs(dy) * 2 &&
          event!.timeStamp - startTime <= 600 &&
          !document.querySelector(BLOCKING)
        )
          change(opening);
      };
      const cancel = () => end();
      const lift = (event: PointerEvent) => {
        if (event.pointerId === id) end(event);
      };
      const move = (event: PointerEvent) => {
        if (event.pointerId !== id) return;
        const dx = (event.clientX - startX) * direction,
          dy = event.clientY - startY;
        if (!dragging) {
          if (Math.abs(dy) > Math.max(12, Math.abs(dx))) return end();
          if (Math.abs(dx) < 10) return;
          if (dx < 0 || Math.abs(dx) < Math.abs(dy) * 1.5) return end();
          if (document.querySelector(BLOCKING) || window.getSelection()?.type === "Range") return end();
          dragging = true;
          suppressClick.current = true;
          drag = dragBrowse(opening, change);
        }
        const distance = Math.max(0, dx);
        // Coalesced moves keep their own times when the page is busy.
        const moves = event.getCoalescedEvents?.() ?? [];
        samples = [
          ...samples.filter((sample) => event.timeStamp - sample.time <= 100),
          ...(moves.length ? moves : [event]).map((move) => ({
            time: move.timeStamp,
            distance: Math.max(0, (move.clientX - startX) * direction),
          })),
        ];
        drag?.move(distance / width);
      };
      window.addEventListener("pointermove", move);
      window.addEventListener("pointerup", lift);
      window.addEventListener("pointercancel", lift);
      document.addEventListener("visibilitychange", cancel);
      stop.current = cancel;
    },
    onKeyDownCapture() {
      suppressClick.current = false;
    },
    onClickCapture(event: ReactMouseEvent) {
      if (!suppressClick.current) return;
      suppressClick.current = false;
      event.preventDefault();
      event.stopPropagation();
    },
  };
}
