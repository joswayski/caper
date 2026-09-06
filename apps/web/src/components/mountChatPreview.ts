import { AmbientLight, Box3, DirectionalLight, PCFShadowMap, PerspectiveCamera, Scene, SRGBColorSpace, Vector3, WebGLRenderer } from "three";
import { createChatModel } from "./chatModel";

export function mountChatPreview(host: HTMLElement, onReady: () => void) {
  let renderer: WebGLRenderer;
  try {
    renderer = new WebGLRenderer({ alpha: true, antialias: true });
  } catch {
    return () => {};
  }

  const scene = new Scene();
  const camera = new PerspectiveCamera(34, 1, 0.1, 100);
  renderer.setClearColor(0x000000, 0);
  renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
  renderer.outputColorSpace = SRGBColorSpace;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = PCFShadowMap;
  renderer.domElement.setAttribute("aria-hidden", "true");
  host.appendChild(renderer.domElement);

  let disposed = false;
  let ready = false;
  const draw = () => { if (!disposed) renderer.render(scene, camera); };
  const model = createChatModel(draw);
  const chat = model.group;
  chat.rotation.set(0.10, -0.20, -0.035);
  scene.add(chat);
  scene.add(new AmbientLight(0xffffff, 1.7));
  const key = new DirectionalLight(0xfff1e4, 1.6);
  key.position.set(-5, 8, 12);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  Object.assign(key.shadow.camera, { left: -8, right: 8, top: 7, bottom: -7, near: 0.5, far: 30 });
  key.shadow.camera.updateProjectionMatrix();
  key.shadow.bias = -0.0002;
  key.shadow.normalBias = 0.015;
  key.shadow.intensity = 0.35;
  scene.add(key);
  const fill = new DirectionalLight(0xa9c5c0, 1.1);
  fill.position.set(8, -3, 5);
  scene.add(fill);

  const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
  let animationFrame = 0;
  let visible = true;
  let pointer: { id: number; x: number; y: number; tiltX: number; tiltY: number } | null = null;
  const target = { x: 0, y: 0 };
  const tilt = { x: 0, y: 0 };
  const limit = 0.14; // Eight degrees in each direction, on top of the resting pose.
  const clamp = (value: number) => Math.max(-limit, Math.min(limit, value));
  let previousTime = 0;
  let startTime = 0;
  const restPose = () => {
    chat.position.y = 0;
    chat.rotation.set(0.10, -0.20, -0.035);
    tilt.x = tilt.y = target.x = target.y = 0;
  };
  const render = (time = 0) => {
    const blend = motion.matches ? 1 : 1 - Math.exp(-Math.min(time - previousTime, 64) / 85);
    previousTime = time;
    const elapsed = time - startTime;
    tilt.x += (target.x - tilt.x) * blend;
    tilt.y += (target.y - tilt.y) * blend;
    chat.position.y = motion.matches ? 0 : Math.sin(elapsed * 0.0007) * 0.10;
    chat.rotation.x = 0.10 + tilt.x + (motion.matches ? 0 : Math.sin(elapsed * 0.00027) * 0.018);
    chat.rotation.y = -0.20 + tilt.y + (motion.matches ? 0 : Math.sin(elapsed * 0.00035) * 0.025);
    chat.rotation.z = -0.035 + (motion.matches ? 0 : Math.sin(elapsed * 0.00022) * 0.009);
    draw();
    if (!motion.matches && visible && !document.hidden) animationFrame = requestAnimationFrame(render);
  };
  const resume = () => {
    if (!ready) return;
    cancelAnimationFrame(animationFrame);
    render(performance.now());
  };
  const reveal = () => {
    if (disposed || ready) return;
    ready = true;
    restPose();
    draw();
    onReady();
    startTime = previousTime = performance.now();
    resume();
  };
  const reset = () => {
    const activePointer = pointer;
    pointer = null;
    target.x = target.y = 0;
    host.classList.remove("is-dragging");
    if (activePointer && host.hasPointerCapture(activePointer.id)) host.releasePointerCapture(activePointer.id);
    if (ready) resume();
  };
  const pointerDown = (event: PointerEvent) => {
    if (event.button !== 0 || !event.isPrimary || pointer) return;
    pointer = { id: event.pointerId, x: event.clientX, y: event.clientY, tiltX: tilt.x, tiltY: tilt.y };
    host.setPointerCapture(event.pointerId);
    host.classList.add("is-dragging");
    host.focus({ preventScroll: true });
  };
  const pointerMove = (event: PointerEvent) => {
    if (!pointer || pointer.id !== event.pointerId) return;
    const { width, height } = host.getBoundingClientRect();
    target.x = clamp(pointer.tiltX + (event.clientY - pointer.y) / height * 0.6);
    target.y = clamp(pointer.tiltY + (event.clientX - pointer.x) / width * 0.6);
    if (ready) resume();
  };
  const pointerEnd = (event: PointerEvent) => {
    if (pointer?.id === event.pointerId) reset();
  };
  const keyDown = (event: KeyboardEvent) => {
    if (event.key === "Escape" || event.key === "Home") {
      event.preventDefault();
      reset();
    } else if (["ArrowUp", "ArrowDown", "ArrowLeft", "ArrowRight"].includes(event.key)) {
      event.preventDefault();
      target.x = clamp(target.x + (event.key === "ArrowDown" ? 0.035 : event.key === "ArrowUp" ? -0.035 : 0));
      target.y = clamp(target.y + (event.key === "ArrowRight" ? 0.035 : event.key === "ArrowLeft" ? -0.035 : 0));
      if (ready) resume();
    }
  };
  host.addEventListener("pointerdown", pointerDown);
  host.addEventListener("pointermove", pointerMove);
  host.addEventListener("pointerup", pointerEnd);
  host.addEventListener("pointercancel", pointerEnd);
  host.addEventListener("lostpointercapture", pointerEnd);
  host.addEventListener("keydown", keyDown);
  host.addEventListener("blur", reset);
  window.addEventListener("blur", reset);
  const resize = () => {
    const { width, height } = host.getBoundingClientRect();
    if (!width || !height) return;
    renderer.setSize(width, height, false);
    camera.aspect = width / height;
    const verticalFov = camera.fov * Math.PI / 180;
    const horizontalFov = 2 * Math.atan(Math.tan(verticalFov / 2) * camera.aspect);
    chat.updateWorldMatrix(true, true);
    const bounds = new Box3().setFromObject(chat);
    const frame = bounds.getSize(new Vector3());
    const center = bounds.getCenter(new Vector3());
    camera.position.z = Math.max(
      (frame.y + 0.8) / (2 * Math.tan(verticalFov / 2)),
      (frame.x + 0.8) / (2 * Math.tan(horizontalFov / 2)),
    ) + frame.z / 2 + 0.5;
    camera.position.y = center.y;
    const stacked = window.matchMedia("(max-width: 1000px)").matches;
    if (stacked) {
      camera.position.x = center.x;
    } else {
      const visibleWidth = 2 * Math.tan(horizontalFov / 2) * (camera.position.z - center.z);
      camera.position.x = center.x - frame.x / 2 + visibleWidth / 2 + 0.42;
    }
    camera.updateProjectionMatrix();
    if (ready) draw();
  };
  const observer = new ResizeObserver(resize);
  observer.observe(host);
  const visibility = new IntersectionObserver(([entry]) => {
    visible = entry.isIntersecting;
    if (ready) resume();
  });
  visibility.observe(host);
  motion.addEventListener("change", resume);
  document.addEventListener("visibilitychange", resume);
  resize();
  void model.whenReady.then(reveal);

  return () => {
    disposed = true;
    cancelAnimationFrame(animationFrame);
    observer.disconnect();
    visibility.disconnect();
    motion.removeEventListener("change", resume);
    document.removeEventListener("visibilitychange", resume);
    host.removeEventListener("pointerdown", pointerDown);
    host.removeEventListener("pointermove", pointerMove);
    host.removeEventListener("pointerup", pointerEnd);
    host.removeEventListener("pointercancel", pointerEnd);
    host.removeEventListener("lostpointercapture", pointerEnd);
    host.removeEventListener("keydown", keyDown);
    host.removeEventListener("blur", reset);
    window.removeEventListener("blur", reset);
    host.classList.remove("is-dragging");
    model.dispose();
    key.shadow.dispose();
    renderer.dispose();
    if (renderer.domElement.parentNode === host) host.removeChild(renderer.domElement);
  };
}
