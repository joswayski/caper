import { useEffect, useRef, type MouseEvent as ReactMouseEvent, type PointerEvent as ReactPointerEvent } from "react";
import { flushSync } from "react-dom";
import { clampProgress, movesOf, settlesForward, SLOP, Velocity } from "../components/swipe";

// Phones show the conversation or Browse (spaces, channels and the account
// bar), never both. Moving between them slides the conversation, a layer above
// the whole of Browse, fully over or off it, following a finger when swiped.
// Browse stays still beneath it. Both are live: the conversation moves as one
// composited layer, so nothing is snapshotted, reflowed or clipped mid-slide.

const NARROW = "(max-width: 760px)";
const ROOM = ".call-room.spaces-room";
const STAGE = ":scope > .stage";
/** Extra travel past the screen edge, so the conversation's shadow leaves with it. */
const SHADOW = 48;
/** The longest a released slide takes, in milliseconds. */
const SETTLE = 320;
/**
 * A tapped toggle's slide, which has no finger speed to carry on: it leaves at
 * once and slows steadily, rather than leaping most of the way in two frames
 * and then crawling into place.
 */
export const TAP = { duration: 300, easing: "cubic-bezier(0.33, 1, 0.68, 1)" };

/** Carries on from a finger without accelerating a held drag or slamming a short remainder into the edge. */
export function releaseTiming(distance: number, velocity: number) {
  const duration = velocity > 0 ? Math.min(SETTLE, Math.max(160, (3 * distance) / velocity)) : SETTLE;
  // Match the initial speed to the release, capped at three times the average.
  // A held or reversing finger starts at rest; every curve lands at rest too.
  const slope = distance > 0 ? Math.min(3, Math.max(0, (velocity * duration) / distance)) : 0;
  return { duration, easing: `cubic-bezier(0.2, ${0.2 * slope}, 0.35, 1)` };
}

export interface BrowseDrag {
  /** Share of the way to the other view, from 0 to 1. */
  move(progress: number): void;
  /** Settles on the other view (`commit`) or back. Velocity is progress per millisecond toward the other view. */
  release(commit: boolean, velocity?: number): void;
}

function animatable() {
  return (
    typeof document !== "undefined" &&
    "animate" in Element.prototype &&
    matchMedia(NARROW).matches &&
    !matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** How far the conversation moves to reveal Browse: off the screen's edge, so its shadow leaves too. */
export function browseTravel(room: Element) {
  return Math.max(1, document.documentElement.clientWidth - room.getBoundingClientRect().left + SHADOW);
}

let current: Slide | undefined;

/** Starts a slide; a newer one owns Browse's state from then on. */
function start(opening: boolean, update: (open: boolean) => void, interactive: boolean) {
  current?.supersede();
  return (current = new Slide(opening, update, interactive));
}

class Slide implements BrowseDrag {
  private readonly room: HTMLElement | null;
  private readonly stage: HTMLElement | null;
  private readonly travel: number;
  private progress = 0;
  /** The paused pose a finger scrubs. */
  private scrub?: Animation;
  /** The release's pose while it settles, and the progress it runs between. */
  private settling?: { animation: Animation; from: number; to: number };
  private released?: { commit: boolean; velocity: number };
  private ended = false;
  private superseded = false;

  constructor(
    readonly opening: boolean,
    private readonly update: (open: boolean) => void,
    private interactive: boolean,
  ) {
    const room = (this.room = document.querySelector<HTMLElement>(ROOM));
    this.stage = room?.querySelector<HTMLElement>(STAGE) ?? null;
    this.travel = room ? browseTravel(room) : 1;
    // A newer slide takes over the pose where the older one left it.
    for (const animation of this.stage?.getAnimations() ?? []) animation.cancel();
    if (!room || !this.stage) {
      this.ended = true;
      update(opening);
      return;
    }
    // Both views show for the slide's length, the conversation posed over Browse.
    room.dataset.browseSlide = "";
    try {
      this.scrub = this.pose(0);
    } catch {
      this.end(true);
      return;
    }
    update(opening);
    if (!interactive) {
      this.released = { commit: true, velocity: 0 };
      this.settle();
    }
  }

  /** Hands the conversation's pose, and Browse's state, to a newer slide. */
  supersede() {
    this.superseded = this.ended = true;
  }

  /** Still moving or held, and not replaced by a newer slide. */
  get active() {
    return !this.ended && !this.superseded;
  }

  /** A finger is posing it. */
  get held() {
    return this.active && !this.released;
  }

  move(progress: number) {
    if (this.ended || this.released) return;
    this.progress = Math.min(1, Math.max(0, progress));
    if (this.scrub) this.scrub.currentTime = this.progress * 1000;
  }

  release(commit: boolean, velocity = 0) {
    if (this.released) return;
    this.released = { commit, velocity };
    if (this.ended) {
      if (!commit && !this.superseded) this.update(!this.opening);
    } else this.settle();
  }

  /**
   * Stops a settling slide where it is, so a new finger carries on from there
   * instead of the slide finishing (or restarting) under it. Returns that
   * progress and which way it was going.
   */
  grab(): { progress: number; heading: boolean } | undefined {
    if (!this.active || !this.released) return undefined;
    const heading = this.released.commit;
    const progress = this.now();
    try {
      const scrub = this.pose(progress);
      this.halt();
      this.scrub = scrub;
    } catch {
      return undefined;
    }
    this.progress = progress;
    this.released = undefined;
    this.interactive = true;
    return { progress, heading };
  }

  /** Turns a settling slide around (or keeps it going) from wherever it is. */
  redirect(commit: boolean) {
    if (!this.active || !this.released) return false;
    if (this.released.commit === commit) return true;
    const progress = this.now();
    this.halt();
    this.progress = progress;
    this.released = { commit, velocity: 0 };
    this.interactive = false;
    this.settle();
    return true;
  }

  /** How far the conversation is now, including partway through settling. */
  private now() {
    const settling = this.settling;
    if (!settling) return this.progress;
    // The eased share of the settle, so this matches what is on screen.
    const eased = settling.animation.effect?.getComputedTiming().progress;
    return eased == null ? settling.to : settling.from + (settling.to - settling.from) * eased;
  }

  /** A paused pose at `progress`, which `move` scrubs. */
  private pose(progress: number) {
    const scrub = this.animate(0, 1, { duration: 1000, fill: "both" });
    scrub.pause();
    scrub.currentTime = progress * 1000;
    return scrub;
  }

  /** Stops settling without ending the slide. */
  private halt() {
    this.settling?.animation.cancel();
    this.settling = undefined;
  }

  /** Moves the conversation between two progress values toward the other view. */
  private animate(from: number, to: number, timing: KeyframeAnimationOptions) {
    const uncovered = (progress: number) => (this.opening ? progress : 1 - progress);
    const place = (progress: number) => ({ transform: `translate3d(${uncovered(progress) * this.travel}px, 0, 0)` });
    return this.stage!.animate([place(from), place(to)], timing);
  }

  private settle() {
    const { commit, velocity } = this.released!;
    const to = commit ? 1 : 0;
    const from = this.progress;
    const distance = Math.abs(to - from),
      speed = commit ? velocity : -velocity;
    // Taps and programmatic changes take the tap timing for the distance left.
    const timing = this.interactive
      ? releaseTiming(distance, speed)
      : { ...TAP, duration: Math.max(160, TAP.duration * distance) };
    try {
      const animation = this.animate(from, to, { ...timing, fill: "both" });
      this.scrub?.cancel();
      this.scrub = undefined;
      const settling = (this.settling = { animation, from, to });
      animation.finished.then(
        () => {
          if (this.settling === settling) this.end(commit);
        },
        () => {},
      );
    } catch {
      this.end(commit);
    }
  }

  /** Lands on the other view (`commit`) or back, and lets the layout hold it. */
  private end(commit: boolean) {
    if (this.ended) return;
    this.ended = true;
    this.released ??= { commit, velocity: 0 };
    // Committing re-applies the view too, so the caller's record of it is
    // right even if something asked for the other view while a finger held it.
    if (!this.superseded) {
      flushSync(() => this.update(commit ? this.opening : !this.opening));
      // The settled layout now shows what the last pose did.
      delete this.room?.dataset.browseSlide;
      for (const animation of this.stage?.getAnimations() ?? []) animation.cancel();
    }
    if (current === this) current = undefined;
  }
}

/**
 * Shows or hides Browse. On phones the conversation slides over or off it;
 * elsewhere, or with reduced motion, `update` runs at once. A slide already
 * moving turns around from where it is rather than restarting; one a finger
 * holds keeps the finger's view, and its release decides.
 */
export function transitionBrowse(open: boolean, update: (open: boolean) => void) {
  if (!animatable()) return update(open);
  const slide = current;
  if (slide?.held) return update(slide.opening);
  if (slide?.redirect(open === slide.opening)) return;
  start(open, update, false);
}

/** Starts a swipe between Browse and the conversation, or undefined where it cannot animate. */
export function dragBrowse(opening: boolean, update: (open: boolean) => void): BrowseDrag | undefined {
  if (!animatable()) return undefined;
  return start(opening, update, true);
}

/** A slide is moving or held; the member drawer leaves those touches to it. */
export function browseSliding() {
  return !!current?.active;
}

/**
 * Takes hold of a settling slide for a new finger: where it is (as progress
 * toward `opening`'s view) and where it was going, or undefined if none moves.
 */
function catchBrowse() {
  const slide = current;
  const grabbed = slide?.grab();
  return slide && grabbed && { drag: slide as BrowseDrag, opening: slide.opening, ...grabbed };
}

export const BLOCKING = "dialog[open], details[open], [role=dialog], [popover]:popover-open";
// Tappable rows, links and buttons can start a swipe; their release click is
// suppressed only after a horizontal drag wins. Composer/audio controls own drags.
export const OWN_GESTURES =
  ".chat-composer, .voice-panel, input, textarea, select, [contenteditable=true], [role=slider], audio, video";

/**
 * Touch swipes on the room: right from the conversation reveals Browse, left
 * from Browse returns. Text entry, sliders, open menus and selections
 * keep their own gestures. `change` sets Browse at once; the slide animates it.
 * A touch while a slide settles stops it under the finger: dragging carries on
 * from there, and a tap lets it finish without activating anything.
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

  const begin = (event: PointerEvent, room: HTMLElement) => {
    suppressClick.current = false;
    // A second finger ends the swipe where it is.
    stop.current?.();
    const { enabled, open, change } = latest.current;
    if (!enabled || !change || event.pointerType !== "touch" || !event.isPrimary) return;
    const caught = catchBrowse();
    if (
      !caught &&
      (document.querySelector(BLOCKING) ||
        window.getSelection()?.type === "Range" ||
        !(event.target instanceof Element) ||
        event.target.closest(OWN_GESTURES))
    )
      return;
    const id = event.pointerId,
      startX = event.clientX,
      startY = event.clientY,
      startTime = event.timeStamp,
      travel = browseTravel(room),
      opening = caught ? caught.opening : !open,
      direction = opening ? 1 : -1,
      base = caught?.progress ?? 0;
    let drag = caught?.drag;
    /** Where the finger was, toward `opening`'s view, when the drag began; the slide follows from there. */
    let origin: number | undefined;
    const speed = new Velocity();
    const along = (point: { clientX: number }) => (point.clientX - startX) * direction;

    const end = (event?: PointerEvent) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", lift);
      window.removeEventListener("pointercancel", lift);
      document.removeEventListener("visibilitychange", cancel);
      if (stop.current === cancel) stop.current = undefined;
      if (origin === undefined) {
        // A tap or scroll on a caught slide lets it carry on; the tap is not a click.
        if (caught) {
          caught.drag.release(caught.heading);
          suppressClick.current = true;
        }
        return;
      }
      const released = event?.type === "pointerup";
      const dx = event ? along(event) : 0;
      if (event) speed.add(event.timeStamp, dx);
      const velocity = event ? speed.speed(event.timeStamp) : 0;
      if (drag)
        drag.release(
          released
            ? settlesForward(clampProgress(base + (dx - origin) / travel), velocity, dx)
            : // Interrupted: back where it started, or on to where a caught slide was going.
              (caught?.heading ?? false),
          velocity / travel,
        );
      // Without a slide, keep the original quick-swipe threshold.
      else if (
        released &&
        dx >= 64 &&
        Math.abs(dx) >= Math.abs(event!.clientY - startY) * 2 &&
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
      const dx = along(event),
        dy = event.clientY - startY;
      if (origin === undefined) {
        if (Math.abs(dy) > Math.max(12, Math.abs(dx))) return end();
        if (Math.abs(dx) < SLOP) return;
        // A caught slide can go either way; otherwise only toward the other view.
        if ((!caught && dx < 0) || Math.abs(dx) < Math.abs(dy) * 1.5) return end();
        if (!caught && (document.querySelector(BLOCKING) || window.getSelection()?.type === "Range")) return end();
        origin = dx;
        suppressClick.current = true;
        drag ??= dragBrowse(opening, change);
      }
      for (const point of movesOf(event)) speed.add(point.timeStamp, along(point));
      drag?.move(base + (dx - origin) / travel);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", lift);
    window.addEventListener("pointercancel", lift);
    document.addEventListener("visibilitychange", cancel);
    stop.current = cancel;
  };

  // A touch outside the room while a slide runs still catches it.
  useEffect(() => {
    if (!enabled) return;
    const press = (event: PointerEvent) => {
      const room = document.querySelector<HTMLElement>(ROOM);
      if (!room || room.contains(event.target as Node)) return;
      if (browseSliding() || stop.current) begin(event, room);
    };
    window.addEventListener("pointerdown", press, true);
    return () => window.removeEventListener("pointerdown", press, true);
  }, [enabled]);

  return {
    onPointerDownCapture(event: ReactPointerEvent<HTMLElement>) {
      begin(event.nativeEvent, event.currentTarget);
    },
    onKeyDownCapture() {
      suppressClick.current = false;
    },
    onClickCapture(event: ReactMouseEvent) {
      if (!suppressClick.current) return;
      suppressClick.current = false;
      // As in the member drawer: keyboard and assistive activations pass.
      if (event.detail === 0) return;
      event.preventDefault();
      event.stopPropagation();
    },
  };
}
