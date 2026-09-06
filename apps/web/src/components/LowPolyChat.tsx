import { useEffect, useRef } from "react";
import {
  AmbientLight,
  BoxGeometry,
  DirectionalLight,
  EdgesGeometry,
  Group,
  IcosahedronGeometry,
  LineBasicMaterial,
  LineSegments,
  Mesh,
  MeshStandardMaterial,
  PerspectiveCamera,
  Scene,
  SRGBColorSpace,
  WebGLRenderer,
} from "three";
import type { BufferGeometry, Material } from "three";

export default function LowPolyChat() {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;

    const scene = new Scene();
    const camera = new PerspectiveCamera(34, 1, 0.1, 100);
    camera.position.set(0, 0, 10.5);

    const renderer = new WebGLRenderer({ alpha: true, antialias: true });
    renderer.setClearColor(0x000000, 0);
    renderer.setPixelRatio(Math.min(window.devicePixelRatio, 2));
    renderer.outputColorSpace = SRGBColorSpace;
    renderer.domElement.setAttribute("aria-hidden", "true");
    host.appendChild(renderer.domElement);

    const geometries: BufferGeometry[] = [];
    const materials: Material[] = [];
    const chat = new Group();
    chat.rotation.set(-0.12, -0.28, -0.035);
    scene.add(chat);

    const material = (color: number, roughness = 0.72) => {
      const value = new MeshStandardMaterial({ color, roughness, metalness: 0.04 });
      materials.push(value);
      return value;
    };

    const box = (
      width: number,
      height: number,
      depth: number,
      color: number,
      x: number,
      y: number,
      z: number,
    ) => {
      const geometry = new BoxGeometry(width, height, depth);
      geometries.push(geometry);
      const mesh = new Mesh(geometry, material(color));
      mesh.position.set(x, y, z);
      chat.add(mesh);
      return mesh;
    };

    box(6.4, 4.35, 0.2, 0x151719, 0, 0, 0);
    box(1.35, 4.05, 0.12, 0x25292b, -2.35, 0, 0.17);
    box(4.72, 0.48, 0.1, 0x1c1f21, 0.7, 1.72, 0.18);
    box(0.72, 0.1, 0.08, 0xb64d32, -2.35, 1.44, 0.3);

    [-0.98, -0.45, 0.08, 0.61].forEach((y, index) => {
      box(index === 1 ? 0.68 : 0.82, 0.11, 0.07, index === 1 ? 0x637a43 : 0x4b5153, -2.25, y, 0.3);
    });

    const avatarColors = [0xb64d32, 0x637a43, 0xe4e5df, 0x596166];
    const lineWidths = [2.7, 2.15, 2.9, 2.35];
    [1.02, 0.3, -0.42, -1.14].forEach((y, index) => {
      const avatarGeometry = new IcosahedronGeometry(0.22, 1);
      geometries.push(avatarGeometry);
      const avatar = new Mesh(avatarGeometry, material(avatarColors[index]));
      avatar.position.set(-1.25, y, 0.32);
      avatar.rotation.set(index * 0.4, index * 0.25, 0);
      chat.add(avatar);

      box(0.72, 0.12, 0.07, 0x717779, -0.65, y + 0.11, 0.28);
      box(lineWidths[index], 0.12, 0.07, 0x3b4042, 0.45, y - 0.12, 0.27);
      box(lineWidths[index] * 0.67, 0.1, 0.07, 0x303537, -0.05, y - 0.34, 0.27);
    });

    [1.7, 2.18, 2.66].forEach((x, index) => {
      const controlGeometry = new IcosahedronGeometry(0.14, 1);
      geometries.push(controlGeometry);
      const control = new Mesh(
        controlGeometry,
        material(index === 2 ? 0xb64d32 : 0x4b5153),
      );
      control.position.set(x, 1.72, 0.33);
      chat.add(control);
    });

    const edgeSourceGeometry = new BoxGeometry(6.4, 4.35, 0.2);
    const edgeGeometry = new EdgesGeometry(edgeSourceGeometry);
    geometries.push(edgeSourceGeometry, edgeGeometry);
    const edgeMaterial = new LineBasicMaterial({ color: 0x4b5153, transparent: true, opacity: 0.75 });
    materials.push(edgeMaterial);
    chat.add(new LineSegments(edgeGeometry, edgeMaterial));

    scene.add(new AmbientLight(0xffffff, 1.9));
    const keyLight = new DirectionalLight(0xf3f4f5, 3.2);
    keyLight.position.set(-4, 6, 8);
    scene.add(keyLight);
    const fillLight = new DirectionalLight(0x637a43, 2.2);
    fillLight.position.set(5, -2, 5);
    scene.add(fillLight);

    const reducedMotion = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    let animationFrame = 0;

    const render = (time = 0) => {
      if (!reducedMotion) {
        chat.position.y = Math.sin(time * 0.0007) * 0.1;
        chat.rotation.y = -0.28 + Math.sin(time * 0.00035) * 0.035;
        animationFrame = requestAnimationFrame(render);
      }
      renderer.render(scene, camera);
    };

    const resize = () => {
      const { width, height } = host.getBoundingClientRect();
      renderer.setSize(width, height, false);
      camera.position.z = width < 500 ? 12.5 : 10.5;
      camera.aspect = width / height;
      camera.updateProjectionMatrix();
      renderer.render(scene, camera);
    };

    const observer = new ResizeObserver(resize);
    observer.observe(host);
    resize();
    render();

    return () => {
      cancelAnimationFrame(animationFrame);
      observer.disconnect();
      renderer.dispose();
      geometries.forEach((geometry) => geometry.dispose());
      materials.forEach((value) => value.dispose());
      host.removeChild(renderer.domElement);
    };
  }, []);

  return <div className="low-poly-chat" ref={hostRef} aria-hidden="true" />;
}
