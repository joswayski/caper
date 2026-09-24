/**
 * Geometry and motion for the homepage's 3D window.
 *
 * The scene's layout box is always its *active* size, so the real app inside
 * lays out once for the size people use it at. At rest a transform shrinks and
 * tilts that box into the stage; opening it animates the transform to identity.
 * Wide screens open in place over the hero ("expand"); narrower screens open a
 * full-screen sheet. The controller writes CSS custom properties; React never
 * re-renders for motion.
 */

export type LiveMode = "expand" | "sheet";

interface MotionOptions {
  /** Element the expanded window fills on wide screens (the hero). */
  bounds: () => HTMLElement | null;
  activate: () => void;
  deactivate: () => void;
  interacted: () => void;
}

/** Resting pose in degrees. The window turns toward the hero copy on its left. */
const REST = { x: 11, y: 16, z: -1.6 };
const NARROW_REST = { x: 8, y: 5, z: -0.8 };
const LIMIT = 16;
const HOVER = 2.4;
const IDLE = { x: 0.9, y: 1.4, z: 0.5, lift: 7 };
export const EXPAND_MIN_WIDTH = 1024;

const clamp = (value: number, limit = LIMIT) => Math.max(-limit, Math.min(limit, value));

export function attachLiveMotion(stage: HTMLElement, scene: HTMLElement, options: MotionOptions) {
  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let frame = 0;
  let visible = true;
  let previous = 0;
  const start = performance.now();
  let mode: LiveMode = "expand";
  let active = false;
  /** 0 at rest, 1 fully open. */
  let open = 0;
  const geometry = { width: 0, height: 0, left: 0, top: 0, scale: 1, dx: 0, dy: 0 };
  const tilt = { x: 0, y: 0 };
  const target = { x: 0, y: 0 };
  const hover = { x: 0, y: 0, tx: 0, ty: 0 };
  let press: { id: number; x: number; y: number; tiltX: number; tiltY: number; touch: boolean; dragging: boolean } | null = null;

  const layout = () => {
    const stageBox = stage.getBoundingClientRect();
    if (!stageBox.width || !stageBox.height) return;
    const viewportWidth = document.documentElement.clientWidth;
    // innerWidth matches CSS media queries, which include the scrollbar.
    mode = window.innerWidth >= EXPAND_MIN_WIDTH ? "expand" : "sheet";
    scene.dataset.mode = mode;
    if (mode === "expand") {
      const bounds = options.bounds()?.getBoundingClientRect() ?? stageBox;
      const width = Math.round(Math.min(bounds.width, 1240));
      const height = Math.round(Math.max(560, Math.min(bounds.height - 24, 780)));
      const left = bounds.left - stageBox.left + (bounds.width - width) / 2;
      const top = bounds.top - stageBox.top + (bounds.height - height) / 2;
      Object.assign(geometry, {
        width, height, left, top,
        scale: Math.min(stageBox.width * 0.92 / width, stageBox.height * 0.9 / height),
        dx: stageBox.width / 2 - (left + width / 2),
        dy: stageBox.height / 2 - (top + height / 2),
      });
    } else {
      // The resting card keeps the viewport's width so the app inside already
      // uses the same layout it will have full screen.
      const width = viewportWidth;
      const height = Math.round(width * stageBox.height / stageBox.width);
      Object.assign(geometry, {
        width, height, left: (stageBox.width - width) / 2, top: (stageBox.height - height) / 2,
        scale: Math.min(stageBox.width * 0.93 / width, stageBox.height * 0.93 / height), dx: 0, dy: 0,
      });
    }
    scene.style.setProperty("--scene-width", `${geometry.width}px`);
    scene.style.setProperty("--scene-height", `${geometry.height}px`);
    scene.style.setProperty("--scene-left", `${geometry.left}px`);
    scene.style.setProperty("--scene-top", `${geometry.top}px`);
    // Lets resting-state labels stay a readable size on the shrunken window.
    scene.style.setProperty("--counter-scale", (1 / geometry.scale).toFixed(4));
    scene.dataset.size = geometry.width >= 700 ? "wide" : "narrow";
    apply(performance.now());
  };

  const apply = (time: number) => {
    const reduced = motion.matches;
    const step = Math.min(time - previous, 64);
    previous = time;
    const blend = reduced ? 1 : 1 - Math.exp(-step / 90);
    tilt.x += (target.x - tilt.x) * blend;
    tilt.y += (target.y - tilt.y) * blend;
    hover.x += (hover.tx - hover.x) * blend * 0.6;
    hover.y += (hover.ty - hover.y) * blend * 0.6;
    const goal = active && mode === "expand" ? 1 : 0;
    // Opening uses a slightly slower, softer curve than tilting.
    open = reduced ? goal : open + (goal - open) * (1 - Math.exp(-step / 120));
    if (Math.abs(goal - open) < 0.0015) open = goal;
    const rest = mode === "expand" ? REST : NARROW_REST;
    const closed = 1 - open;
    const elapsed = reduced ? 0 : time - start;
    const idle = reduced ? 0 : closed;
    const x = (rest.x + tilt.x + hover.x) * closed + Math.sin(elapsed * 0.00027) * IDLE.x * idle;
    const y = (rest.y + tilt.y + hover.y) * closed + Math.sin(elapsed * 0.00035) * IDLE.y * idle;
    const z = rest.z * closed + Math.sin(elapsed * 0.00022) * IDLE.z * idle;
    const lift = Math.sin(elapsed * 0.0007) * IDLE.lift * idle;
    const scale = geometry.scale + (1 - geometry.scale) * open;
    scene.style.setProperty("--depth", closed.toFixed(4));
    if (open === 1 || (active && mode === "sheet")) {
      // Fully open: plain layout, so text is crisp and the app behaves normally.
      scene.style.transform = "none";
      scene.dataset.settled = "";
    } else {
      delete scene.dataset.settled;
      scene.style.transform = `translate3d(${(geometry.dx * closed).toFixed(2)}px, ${(geometry.dy * closed + lift).toFixed(2)}px, 0) `
        + `scale3d(${scale.toFixed(4)}, ${scale.toFixed(4)}, ${scale.toFixed(4)}) `
        + `rotateX(${x.toFixed(3)}deg) rotateY(${y.toFixed(3)}deg) rotateZ(${z.toFixed(3)}deg)`;
    }
  };

  const settled = () => Math.abs(target.x - tilt.x) < 0.01 && Math.abs(target.y - tilt.y) < 0.01
    && Math.abs(hover.tx - hover.x) < 0.01 && Math.abs(hover.ty - hover.y) < 0.01
    && open === (active && mode === "expand" ? 1 : 0);
  const loop = (time: number) => {
    apply(time);
    frame = 0;
    if (!visible || document.hidden) return;
    // The idle float needs frames only at rest; an open window stops animating.
    if ((!motion.matches && !active) || !settled()) frame = requestAnimationFrame(loop);
  };
  const wake = () => {
    if (frame || document.hidden || (!visible && !active)) return;
    previous = performance.now();
    frame = requestAnimationFrame(loop);
  };

  const interactive = (target: EventTarget | null) => (target as Element | null)?.closest?.("[data-live-control]");
  const pointerDown = (event: PointerEvent) => {
    if (active || press || !event.isPrimary || (event.pointerType === "mouse" && event.button !== 0)) return;
    if (interactive(event.target)) return;
    options.interacted();
    press = { id: event.pointerId, x: event.clientX, y: event.clientY, tiltX: tilt.x, tiltY: tilt.y, touch: event.pointerType !== "mouse", dragging: false };
    try { stage.setPointerCapture(event.pointerId); } catch { /* Synthetic pointers cannot be captured. */ }
  };
  const pointerMove = (event: PointerEvent) => {
    if (active) return;
    const { left, top, width, height } = stage.getBoundingClientRect();
    if (press?.id === event.pointerId) {
      const dx = event.clientX - press.x;
      const dy = event.clientY - press.y;
      if (!press.dragging && Math.hypot(dx, dy) > 6) { press.dragging = true; stage.dataset.dragging = ""; }
      if (!press.dragging) return;
      target.y = clamp(press.tiltY + dx / width * 70);
      // One-finger vertical movement belongs to page scrolling on touch screens.
      if (!press.touch) target.x = clamp(press.tiltX - dy / height * 50);
      wake();
    } else if (event.pointerType === "mouse" && !motion.matches) {
      hover.ty = ((event.clientX - left) / width - 0.5) * 2 * HOVER;
      hover.tx = -((event.clientY - top) / height - 0.5) * 2 * HOVER;
      wake();
    }
  };
  const release = (event?: PointerEvent) => {
    if (!press || (event && press.id !== event.pointerId)) return;
    const clicked = event?.type === "pointerup" && !press.dragging;
    if (stage.hasPointerCapture(press.id)) stage.releasePointerCapture(press.id);
    press = null;
    delete stage.dataset.dragging;
    target.x = target.y = 0;
    wake();
    if (clicked) options.activate();
  };
  const pointerLeave = () => { hover.tx = hover.ty = 0; wake(); };
  const cancel = () => release();
  const keyDown = (event: KeyboardEvent) => {
    if (active || !(event.target as Element | null)?.matches?.("[data-live-activator]")) return;
    const step = 3;
    if (event.key === "ArrowUp") target.x = clamp(target.x + step);
    else if (event.key === "ArrowDown") target.x = clamp(target.x - step);
    else if (event.key === "ArrowLeft") target.y = clamp(target.y - step);
    else if (event.key === "ArrowRight") target.y = clamp(target.y + step);
    else return;
    event.preventDefault();
    options.interacted();
    wake();
  };
  // Close on Escape unless an in-app menu or dialog handles it first, or on a
  // press outside the expanded window.
  const documentKey = (event: KeyboardEvent) => {
    if (!active || event.key !== "Escape" || event.defaultPrevented) return;
    const from = event.target as Element | null;
    if (from?.closest?.("dialog, details[open], [popover]") || scene.querySelector("details[open], dialog[open]")) return;
    options.deactivate();
  };
  const documentPress = (event: PointerEvent) => {
    if (!active || mode !== "expand" || scene.contains(event.target as Node)) return;
    if ((event.target as Element | null)?.closest?.("[data-live-open]")) return;
    options.deactivate();
  };

  stage.addEventListener("pointerdown", pointerDown);
  stage.addEventListener("pointermove", pointerMove);
  stage.addEventListener("pointerup", release);
  stage.addEventListener("pointercancel", release);
  stage.addEventListener("lostpointercapture", release);
  stage.addEventListener("pointerleave", pointerLeave);
  stage.addEventListener("keydown", keyDown);
  document.addEventListener("keydown", documentKey);
  document.addEventListener("pointerdown", documentPress);
  window.addEventListener("blur", cancel);
  window.addEventListener("resize", layout);
  const observer = new ResizeObserver(layout);
  observer.observe(stage);
  const boundsElement = options.bounds();
  if (boundsElement) observer.observe(boundsElement);
  const visibility = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; wake(); });
  visibility.observe(stage);
  motion.addEventListener("change", wake);
  document.addEventListener("visibilitychange", wake);
  layout();
  wake();

  return {
    get mode() { return mode; },
    setActive(next: boolean) {
      if (active === next) return;
      active = next;
      hover.tx = hover.ty = 0;
      target.x = target.y = 0;
      layout();
      wake();
    },
    detach() {
      cancelAnimationFrame(frame);
      observer.disconnect();
      visibility.disconnect();
      motion.removeEventListener("change", wake);
      document.removeEventListener("visibilitychange", wake);
      stage.removeEventListener("pointerdown", pointerDown);
      stage.removeEventListener("pointermove", pointerMove);
      stage.removeEventListener("pointerup", release);
      stage.removeEventListener("pointercancel", release);
      stage.removeEventListener("lostpointercapture", release);
      stage.removeEventListener("pointerleave", pointerLeave);
      stage.removeEventListener("keydown", keyDown);
      document.removeEventListener("keydown", documentKey);
      document.removeEventListener("pointerdown", documentPress);
      window.removeEventListener("blur", cancel);
      window.removeEventListener("resize", layout);
    },
  };
}
