import {
  useEffect,
  useLayoutEffect,
  useRef,
  type MouseEvent as ReactMouseEvent,
  type PointerEvent as ReactPointerEvent,
  type RefObject,
} from "react";
import { flushSync } from "react-dom";
import { clampProgress, movesOf, settlesForward, SLOP, Velocity } from "../components/swipe";

// Phones show the conversation or Browse (spaces and channels), never both.
// Moving between them slides the conversation fully over or off the still
// Browse, following a finger when swiped, while Browse's account bar rises over
// it from the bottom edge. Both views share the room's DOM, so a view
// transition snapshots the outgoing view while the incoming one renders live;
// the conversation's and bar's snapshots are posed by hand so a swipe can scrub
// them and then settle forward or back.

const NARROW = "(max-width: 760px)";
const ROOM = ".call-room.spaces-room";
const NAME = "browse-room";
const BAR_NAME = "browse-bar";
/** Browse's account and voice bar, which stays uncovered at the bottom. */
const BAR = ":scope > .people-panel > .voice-panel";
export const SETTLE = { duration: 320, easing: "cubic-bezier(0.32, 0.72, 0, 1)" };
/** A tapped toggle's slide, which has no finger speed to carry on. */
const TAP = { duration: 220 };

/** Carries on from a finger without accelerating a held drag or slamming a short remainder into the edge. */
export function releaseTiming(distance: number, velocity: number) {
  const duration = velocity > 0 ? Math.min(SETTLE.duration, Math.max(160, (3 * distance) / velocity)) : SETTLE.duration;
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
    "startViewTransition" in document &&
    matchMedia(NARROW).matches &&
    !matchMedia("(prefers-reduced-motion: reduce)").matches
  );
}

/** How far the conversation moves to reveal Browse: past the room, so its edge shadow leaves too. */
export function browseTravel(room: Element) {
  return room.getBoundingClientRect().width + 48;
}

/**
 * Keeps the hidden conversation laid out where it was on phones (the offset it
 * has below its channel row), and measures Browse's account bar, which rises
 * over the sliding conversation by that height.
 */
export function useBrowseLayout(roomRef: RefObject<HTMLElement | null>, open: boolean) {
  useLayoutEffect(() => {
    const room = roomRef.current;
    if (!room) return;
    const media = matchMedia(NARROW);
    let observer: ResizeObserver | undefined;
    const watch = () => {
      observer?.disconnect();
      if (!media.matches) return;
      const stage = room.querySelector<HTMLElement>(":scope > .stage");
      const bar = room.querySelector<HTMLElement>(BAR);
      const measure = () => {
        // The bar's 12px margin above it is part of the uncovered area.
        if (open && bar)
          room.style.setProperty(
            "--browse-bar",
            `${room.getBoundingClientRect().bottom - bar.getBoundingClientRect().top + 12}px`,
          );
        if (!open && stage) room.style.setProperty("--browse-stage-top", `${stage.offsetTop}px`);
      };
      measure();
      observer = new ResizeObserver(measure);
      for (const element of [room, open ? bar : stage]) if (element) observer.observe(element);
    };
    watch();
    media.addEventListener("change", watch);
    return () => {
      media.removeEventListener("change", watch);
      observer?.disconnect();
    };
  }, [roomRef, open]);
}

let current: Slide | undefined;

/** Starts a slide; a newer one owns Browse's state from then on. */
function start(opening: boolean, update: (open: boolean) => void, interactive: boolean) {
  if (current) current.superseded = true;
  return (current = new Slide(opening, update, interactive));
}

class Slide implements BrowseDrag {
  private readonly transition: ViewTransition;
  private readonly room: HTMLElement | null;
  private readonly travel: number;
  private progress = 0;
  private scrub: Animation[] = [];
  /** The release's poses while it settles, and the progress it runs between. */
  private settling?: { animations: Animation[]; from: number; to: number };
  private keepAlive?: Animation;
  /** Counts settles, so a stopped one does not end the slide when it is cancelled. */
  private generation = 0;
  /** Every animation on the transition's pseudo-elements, cancelled when it ends. */
  private readonly animations: Animation[] = [];
  private ready = false;
  private released?: { commit: boolean; velocity: number };
  private ended = false;
  superseded = false;

  constructor(
    readonly opening: boolean,
    private readonly update: (open: boolean) => void,
    private interactive: boolean,
  ) {
    const root = document.documentElement;
    const room = (this.room = document.querySelector<HTMLElement>(ROOM));
    this.travel = room ? browseTravel(room) : 1;
    root.dataset.browseTransition = opening ? "open" : "close";
    this.transition = document.startViewTransition(() => flushSync(() => update(opening)));
    if (!interactive) this.released = { commit: true, velocity: 0 };
    this.transition.ready.then(
      () => {
        if (this.ended) return;
        this.ready = true;
        if (this.released) return this.settle();
        try {
          this.scrub = this.pose(this.progress);
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

  /** Still moving or held, and not replaced by a newer slide. */
  get active() {
    return !this.ended && !this.superseded;
  }

  /** A finger is posing it. */
  get held() {
    return this.active && !this.released;
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

  /**
   * Stops a settling slide where it is, so a new finger carries on from there
   * instead of the slide finishing (or restarting) under it. Returns that
   * progress and which way it was going.
   */
  grab(): { progress: number; heading: boolean } | undefined {
    if (!this.ready || !this.active || !this.released) return undefined;
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
    if (!this.ready || !this.active || !this.released) return false;
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
    const eased = settling.animations[0]?.effect?.getComputedTiming().progress;
    return eased == null ? settling.to : settling.from + (settling.to - settling.from) * eased;
  }

  /** Paused poses at `progress`, which `move` scrubs. */
  private pose(progress: number) {
    const scrub = this.animate(0, 1, { duration: 1000, fill: "both" });
    for (const animation of scrub) {
      animation.pause();
      animation.currentTime = progress * 1000;
    }
    return scrub;
  }

  /** Stops settling without ending the slide. */
  private halt() {
    this.generation++;
    for (const animation of this.settling?.animations ?? []) animation.cancel();
    this.keepAlive?.cancel();
    this.settling = this.keepAlive = undefined;
  }

  /**
   * Moves the conversation's snapshot (outgoing when opening Browse) between two
   * progress values, and raises Browse's account bar over it from the bottom edge.
   */
  private animate(from: number, to: number, timing: KeyframeAnimationOptions) {
    const bar = parseFloat(this.room?.style.getPropertyValue("--browse-bar") ?? "") || 0;
    const uncovered = (progress: number) => (this.opening ? progress : 1 - progress);
    const conversation = (progress: number) => ({ transform: `translateX(${uncovered(progress) * this.travel}px)` });
    const account = (progress: number) => ({ transform: `translateY(${(1 - uncovered(progress)) * bar}px)` });
    const animations = [
      document.documentElement.animate([conversation(from), conversation(to)], {
        ...timing,
        pseudoElement: `::view-transition-${this.opening ? "old" : "new"}(${NAME})`,
      }),
      document.documentElement.animate([account(from), account(to)], {
        ...timing,
        pseudoElement: `::view-transition-${this.opening ? "new" : "old"}(${BAR_NAME})`,
      }),
    ];
    this.animations.push(...animations);
    return animations;
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
      : { duration: Math.max(160, TAP.duration * distance), easing: SETTLE.easing };
    const generation = ++this.generation;
    try {
      // Settled poses would end the transition before a cancelled swipe
      // restores its view; this keeps it alive until that update is in.
      this.keepAlive = document.documentElement.animate([{ opacity: 1 }, { opacity: 1 }], {
        duration: timing.duration + 1000,
        pseudoElement: "::view-transition",
      });
      this.animations.push(this.keepAlive);
      const settling = this.animate(from, to, { ...timing, fill: "both" });
      for (const animation of this.scrub) animation.cancel();
      this.scrub = [];
      this.settling = { animations: settling, from, to };
      const end = () => {
        if (generation === this.generation) this.end(commit);
      };
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
    // Committing re-applies the view too, so the caller's record of it is
    // right even if something asked for the other view while a finger held it.
    if (!this.superseded) flushSync(() => this.update(commit ? this.opening : !this.opening));
    this.transition.skipTransition();
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

  // While a slide runs, its snapshots cover the page and touches land on the
  // root element rather than the room; catch those here.
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
