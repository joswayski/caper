/**
 * Pose and interaction for the homepage's 3D window. Writes CSS custom
 * properties on the scene each frame; React never re-renders for motion.
 */

export interface SceneSize { width: number; height: number }

interface MotionOptions {
  /** Logical scene size in CSS pixels for the current stage width. */
  size: (stageWidth: number, stageHeight: number) => SceneSize;
  resized: (size: SceneSize) => void;
  interacted: () => void;
}

/** Resting pose, in degrees. The window turns toward the hero copy on its left. */
const REST = { x: 10, y: 16, z: -1.6 };
const COMPACT_REST = { x: 7, y: 6, z: -0.8 };
/** Drag limit on top of the resting pose. */
const LIMIT = 16;
const HOVER = 2.4;
const IDLE = { x: 0.9, y: 1.4, z: 0.5, lift: 7 };

const interactive = "a, button, input, textarea, select, label, [data-interactive]";
const clamp = (value: number, limit = LIMIT) => Math.max(-limit, Math.min(limit, value));

export function attachLiveMotion(stage: HTMLElement, scene: HTMLElement, options: MotionOptions) {
  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let frame = 0;
  let visible = true;
  let previous = 0;
  const start = performance.now();
  let compact = false;
  let size: SceneSize = { width: 0, height: 0 };
  const tilt = { x: 0, y: 0 };
  const target = { x: 0, y: 0 };
  const hover = { x: 0, y: 0, tx: 0, ty: 0 };
  let focus = 0;
  let focusTarget = 0;
  let drag: { id: number; x: number; y: number; tiltX: number; tiltY: number; touch: boolean } | null = null;

  const apply = (time: number) => {
    const reduced = motion.matches;
    const blend = reduced ? 1 : 1 - Math.exp(-Math.min(time - previous, 64) / 90);
    previous = time;
    tilt.x += (target.x - tilt.x) * blend;
    tilt.y += (target.y - tilt.y) * blend;
    hover.x += (hover.tx - hover.x) * blend * 0.6;
    hover.y += (hover.ty - hover.y) * blend * 0.6;
    focus += (focusTarget - focus) * blend;
    const elapsed = reduced ? 0 : time - start;
    const rest = compact ? COMPACT_REST : REST;
    // Typing turns the window toward the visitor so the composer is easy to read.
    const settle = 1 - focus * 0.8;
    const idle = reduced ? 0 : settle;
    const x = (rest.x + tilt.x + hover.x) * settle + Math.sin(elapsed * 0.00027) * IDLE.x * idle;
    const y = (rest.y + tilt.y + hover.y) * settle + Math.sin(elapsed * 0.00035) * IDLE.y * idle;
    const z = rest.z * settle + Math.sin(elapsed * 0.00022) * IDLE.z * idle;
    scene.style.setProperty("--rx", `${x.toFixed(3)}deg`);
    scene.style.setProperty("--ry", `${y.toFixed(3)}deg`);
    scene.style.setProperty("--rz", `${z.toFixed(3)}deg`);
    scene.style.setProperty("--lift", `${(Math.sin(elapsed * 0.0007) * IDLE.lift * idle).toFixed(2)}px`);
    scene.style.setProperty("--zoom", `${(1 + focus * 0.05).toFixed(4)}`);
  };

  const settled = () => Math.abs(target.x - tilt.x) < 0.01 && Math.abs(target.y - tilt.y) < 0.01
    && Math.abs(hover.tx - hover.x) < 0.01 && Math.abs(hover.ty - hover.y) < 0.01 && Math.abs(focusTarget - focus) < 0.001;
  const loop = (time: number) => {
    apply(time);
    frame = 0;
    if (!visible || document.hidden) return;
    // Reduced motion keeps drag and focus but drops the idle float.
    if (!motion.matches || !settled()) frame = requestAnimationFrame(loop);
  };
  const wake = () => {
    if (frame || !visible || document.hidden) return;
    previous = performance.now();
    frame = requestAnimationFrame(loop);
  };

  const resize = () => {
    const { width, height } = stage.getBoundingClientRect();
    if (!width || !height) return;
    compact = height > width;
    const next = options.size(width, height);
    // Leave room for the tilted corners and the idle float.
    const room = compact ? 0.94 : 0.92;
    const scale = Math.min(width * room / next.width, height * room / next.height);
    scene.style.setProperty("--scale", scale.toFixed(4));
    if (next.width !== size.width || next.height !== size.height) {
      size = next;
      options.resized(next);
    }
    apply(performance.now());
  };

  const pointerDown = (event: PointerEvent) => {
    if (drag || !event.isPrimary || (event.pointerType === "mouse" && event.button !== 0)) return;
    if ((event.target as Element | null)?.closest(interactive)) return;
    options.interacted();
    drag = { id: event.pointerId, x: event.clientX, y: event.clientY, tiltX: tilt.x, tiltY: tilt.y, touch: event.pointerType !== "mouse" };
    try { stage.setPointerCapture(event.pointerId); } catch { /* Synthetic pointers cannot be captured. */ }
    stage.dataset.dragging = "";
    wake();
  };
  const pointerMove = (event: PointerEvent) => {
    const { left, top, width, height } = stage.getBoundingClientRect();
    if (drag?.id === event.pointerId) {
      target.y = clamp(drag.tiltY + (event.clientX - drag.x) / width * 70);
      // One-finger vertical movement belongs to page scrolling on touch screens.
      if (!drag.touch) target.x = clamp(drag.tiltX - (event.clientY - drag.y) / height * 50);
      wake();
    } else if (event.pointerType === "mouse" && !motion.matches) {
      hover.ty = ((event.clientX - left) / width - 0.5) * 2 * HOVER;
      hover.tx = -((event.clientY - top) / height - 0.5) * 2 * HOVER;
      wake();
    }
  };
  const release = () => {
    if (drag && stage.hasPointerCapture(drag.id)) stage.releasePointerCapture(drag.id);
    drag = null;
    delete stage.dataset.dragging;
    target.x = target.y = 0;
    wake();
  };
  const pointerEnd = (event: PointerEvent) => { if (drag?.id === event.pointerId) release(); };
  const pointerLeave = () => { hover.tx = hover.ty = 0; wake(); };
  const keyDown = (event: KeyboardEvent) => {
    if (event.target !== stage) return;
    const step = 3;
    if (event.key === "Escape" || event.key === "Home") { target.x = target.y = 0; }
    else if (event.key === "ArrowUp") target.x = clamp(target.x + step);
    else if (event.key === "ArrowDown") target.x = clamp(target.x - step);
    else if (event.key === "ArrowLeft") target.y = clamp(target.y - step);
    else if (event.key === "ArrowRight") target.y = clamp(target.y + step);
    else return;
    event.preventDefault();
    options.interacted();
    wake();
  };
  const focusIn = (event: FocusEvent) => {
    if ((event.target as Element | null)?.matches("input, textarea")) { focusTarget = 1; wake(); }
  };
  const focusOut = (event: FocusEvent) => {
    if ((event.target as Element | null)?.matches("input, textarea")) { focusTarget = 0; wake(); }
  };

  stage.addEventListener("pointerdown", pointerDown);
  stage.addEventListener("pointermove", pointerMove);
  stage.addEventListener("pointerup", pointerEnd);
  stage.addEventListener("pointercancel", pointerEnd);
  stage.addEventListener("lostpointercapture", pointerEnd);
  stage.addEventListener("pointerleave", pointerLeave);
  stage.addEventListener("keydown", keyDown);
  stage.addEventListener("focusin", focusIn);
  stage.addEventListener("focusout", focusOut);
  window.addEventListener("blur", release);
  const observer = new ResizeObserver(resize);
  observer.observe(stage);
  const visibility = new IntersectionObserver(([entry]) => { visible = entry.isIntersecting; wake(); });
  visibility.observe(stage);
  motion.addEventListener("change", wake);
  document.addEventListener("visibilitychange", wake);
  resize();
  wake();

  return () => {
    cancelAnimationFrame(frame);
    observer.disconnect();
    visibility.disconnect();
    motion.removeEventListener("change", wake);
    document.removeEventListener("visibilitychange", wake);
    stage.removeEventListener("pointerdown", pointerDown);
    stage.removeEventListener("pointermove", pointerMove);
    stage.removeEventListener("pointerup", pointerEnd);
    stage.removeEventListener("pointercancel", pointerEnd);
    stage.removeEventListener("lostpointercapture", pointerEnd);
    stage.removeEventListener("pointerleave", pointerLeave);
    stage.removeEventListener("keydown", keyDown);
    stage.removeEventListener("focusin", focusIn);
    stage.removeEventListener("focusout", focusOut);
    window.removeEventListener("blur", release);
  };
}
