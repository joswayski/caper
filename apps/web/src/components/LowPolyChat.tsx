import { useEffect, useRef } from "react";
import { AmbientLight, Box3, DirectionalLight, PCFShadowMap, PerspectiveCamera, Scene, SRGBColorSpace, Vector3, WebGLRenderer } from "three";
import { createChatModel } from "./chatModel";

export default function LowPolyChat() {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const scene = new Scene();
    const camera = new PerspectiveCamera(34, 1, 0.1, 100);
    const renderer = new WebGLRenderer({ alpha: true, antialias: true });
    renderer.setClearColor(0x000000, 0);
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = SRGBColorSpace;
    renderer.shadowMap.enabled = true;
    renderer.shadowMap.type = PCFShadowMap;
    renderer.domElement.setAttribute("aria-hidden", "true");
    host.appendChild(renderer.domElement);

    let disposed = false;
    const draw = () => { if (!disposed) renderer.render(scene, camera); };
    const model = createChatModel(draw);
    const chat = model.group;
    chat.rotation.set(0.10, -0.20, -0.035);
    scene.add(chat);
    scene.add(new AmbientLight(0xffffff, 1.3));
    const key = new DirectionalLight(0xfff1e4, 2.2);
    key.position.set(-5, 8, 12);
    key.castShadow = true;
    key.shadow.mapSize.set(2048, 2048);
    Object.assign(key.shadow.camera, { left: -8, right: 8, top: 7, bottom: -7, near: 0.5, far: 30 });
    key.shadow.bias = -0.0002;
    key.shadow.normalBias = 0.015;
    scene.add(key);
    const fill = new DirectionalLight(0xa9c5c0, 1.1);
    fill.position.set(8, -3, 5);
    scene.add(fill);

    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    let animationFrame = 0;
    let visible = true;
    const render = (time = 0) => {
      chat.position.y = motion.matches ? 0 : Math.sin(time * 0.0007) * 0.10;
      chat.rotation.y = -0.20 + (motion.matches ? 0 : Math.sin(time * 0.00035) * 0.045);
      draw();
      if (!motion.matches && visible && !document.hidden) animationFrame = requestAnimationFrame(render);
    };
    const resume = () => {
      cancelAnimationFrame(animationFrame);
      render(performance.now());
    };
    const resize = () => {
      const { width, height } = host.getBoundingClientRect();
      if (!width || !height) return;
      renderer.setSize(width, height, false);
      camera.aspect = width / height;
      const verticalFov = camera.fov * Math.PI / 180;
      const horizontalFov = 2 * Math.atan(Math.tan(verticalFov / 2) * camera.aspect);
      chat.updateWorldMatrix(true, true);
      const frame = new Box3().setFromObject(chat).getSize(new Vector3());
      camera.position.z = Math.max(
        (frame.y + 0.3) / (2 * Math.tan(verticalFov / 2)),
        (frame.x + 0.3) / (2 * Math.tan(horizontalFov / 2)),
      ) + frame.z / 2 + 0.5;
      camera.updateProjectionMatrix();
      draw();
    };
    const observer = new ResizeObserver(resize);
    observer.observe(host);
    const visibility = new IntersectionObserver(([entry]) => {
      visible = entry.isIntersecting;
      resume();
    });
    visibility.observe(host);
    motion.addEventListener("change", resume);
    document.addEventListener("visibilitychange", resume);
    resize();
    resume();

    return () => {
      disposed = true;
      cancelAnimationFrame(animationFrame);
      observer.disconnect();
      visibility.disconnect();
      motion.removeEventListener("change", resume);
      document.removeEventListener("visibilitychange", resume);
      model.dispose();
      key.shadow.dispose();
      renderer.dispose();
      host.removeChild(renderer.domElement);
    };
  }, []);

  return <div className="low-poly-chat" ref={hostRef} role="img" aria-label="A floating 3D Caper chat preview: Studio channels, a shared game clip, and a conversation between friends with colorful caper avatars." />;
}
