import {
  BoxGeometry, BufferGeometry, CanvasTexture, CircleGeometry, CylinderGeometry, Group, IcosahedronGeometry,
  Line, LineBasicMaterial, Mesh, MeshBasicMaterial, MeshStandardMaterial, PlaneGeometry,
  SRGBColorSpace, TextureLoader, Vector3,
} from "three";
import type { Material, Texture } from "three";

/** A miniature app built in pixels (100px = one world unit), not a screenshot. */
export function createChatModel(invalidate: () => void) {
  const group = new Group();
  // Keep actual depth, but avoid exaggerated parallax between labels and controls.
  group.scale.z = 0.45;
  const geometries: BufferGeometry[] = [];
  const materials: Material[] = [];
  const textures: Texture[] = [];
  const ink = "#f1f3ef";
  const muted = "#929da5";
  const orange = 0xb85336;
  const palette = new Map<number, MeshStandardMaterial>();
  const material = (color: number) => {
    if (!palette.has(color)) {
      const value = new MeshStandardMaterial({ color, roughness: 0.75, metalness: 0.12, flatShading: true });
      materials.push(value);
      palette.set(color, value);
    }
    return palette.get(color)!;
  };
  const mesh = (geometry: BufferGeometry, surface: Material, x: number, y: number, z: number) => {
    geometries.push(geometry);
    const object = new Mesh(geometry, surface);
    object.position.set((x - 600) / 100, (400 - y) / 100, z / 100);
    object.castShadow = surface instanceof MeshStandardMaterial;
    object.receiveShadow = surface instanceof MeshStandardMaterial;
    group.add(object);
    return object;
  };
  const panel = (x: number, y: number, w: number, h: number, color: number, z = 14, depth = 6) =>
    mesh(new BoxGeometry(w / 100, h / 100, depth / 100), material(color), x + w / 2, y + h / 2, z);
  const text = (value: string, x: number, y: number, size = 15, color = ink, bold = false, z = 24) => {
    const canvas = document.createElement("canvas");
    const ctx = canvas.getContext("2d")!;
    const font = `${bold ? 700 : 400} ${size * 3}px "Satoshi", "Avenir Next", "Segoe UI", sans-serif`;
    ctx.font = font;
    canvas.width = Math.ceil(ctx.measureText(value).width + 9);
    canvas.height = size * 4.5;
    ctx.font = font;
    ctx.fillStyle = color;
    ctx.textBaseline = "top";
    ctx.fillText(value, 0, size * 0.3);
    const texture = new CanvasTexture(canvas);
    texture.colorSpace = SRGBColorSpace;
    textures.push(texture);
    const surface = new MeshBasicMaterial({ map: texture, transparent: true, depthWrite: false });
    materials.push(surface);
    mesh(new PlaneGeometry(canvas.width / 300, canvas.height / 300), surface,
      x + canvas.width / 6, y + canvas.height / 6, z);
    return canvas.width / 3;
  };
  const dot = (x: number, y: number, radius: number, color: number, z = 25) =>
    mesh(new IcosahedronGeometry(radius / 100, 1), material(color), x, y, z);
  const iconPaths = {
    inbox: [[[2, 5], [22, 5], [22, 20], [2, 20], [2, 5], [12, 13], [22, 5]]],
    chat: [[[3, 3], [21, 3], [21, 17], [10, 17], [3, 22], [3, 3]]],
    saved: [[[5, 22], [5, 2], [19, 2], [19, 22], [12, 17], [5, 22]]],
    attach: [[[8, 14], [16, 6], [19, 6], [20, 9], [8, 21], [4, 21], [2, 18], [2, 14], [15, 1], [20, 1], [23, 4], [23, 9], [12, 20]]],
    call: [[[3, 2], [8, 2], [10, 8], [7, 10], [14, 17], [17, 14], [23, 17], [23, 22], [18, 23], [10, 19], [4, 12], [1, 6], [3, 2]]],
  };
  const iconSurface = new LineBasicMaterial({ color: 0xc0cbcf });
  materials.push(iconSurface);
  const icon = (name: keyof typeof iconPaths, x: number, y: number, size = 18, z = 38) => {
    iconPaths[name].forEach((path) => {
      const geometry = new BufferGeometry().setFromPoints(path.map(([px, py]) =>
        new Vector3((x + px * size / 24 - 600) / 100, (400 - y - py * size / 24) / 100, z / 100)));
      geometries.push(geometry);
      group.add(new Line(geometry, iconSurface));
    });
  };
  let settle: () => void = () => {};
  const whenReady = new Promise<void>((resolve) => {
    settle = resolve;
  });
  const atlas = new TextureLoader().load("/images/caper-avatars.webp", () => {
    invalidate();
    settle();
  }, undefined, settle);
  atlas.colorSpace = SRGBColorSpace;
  textures.push(atlas);
  const portraitMaterial = new MeshBasicMaterial({ map: atlas });
  materials.push(portraitMaterial);
  const avatar = (person: number, x: number, y: number, size = 42, online = false) => {
    const rim = mesh(new CylinderGeometry((size / 2 + 2) / 100, (size / 2 + 2) / 100, 0.06, 16), material(0x414a48), x, y, 38);
    rim.rotation.x = Math.PI / 2;
    const geometry = new CircleGeometry(size / 200, 16);
    const uv = geometry.attributes.uv;
    for (let i = 0; i < uv.count; i++) {
      uv.setXY(i, (uv.getX(i) + person % 2) / 2, (uv.getY(i) + (person < 2 ? 1 : 0)) / 2);
    }
    mesh(geometry, portraitMaterial, x, y, 41.5);
    if (online) dot(x + size * 0.35, y + size * 0.35, 4, 0x86b56d, 43);
  };
  const pill = (label: string, x: number, y: number, w: number, color = 0x252c2e, labelColor = ink) => {
    panel(x, y, w, 29, color, 42, 9);
    text(label, x + 10, y + 5, 13, labelColor, false, 49);
  };
  const message = (person: number, name: string, time: string, y: number, body: string) => {
    avatar(person, 259, y + 18);
    const nameWidth = text(name, 292, y, 17, ink, true);
    text(time, 292 + nameWidth + 10, y + 4, 12, muted);
    text(body, 292, y + 27, 15);
  };

  // Beveled-looking stacked chassis, with exposed side walls and inset surfaces.
  panel(-5, -5, 1210, 810, 0x252e30, -5, 26);
  panel(0, 0, 1200, 800, 0x111719, 8, 8);
  panel(2, 2, 1196, 42, 0x171e20, 15);
  [0xe47258, 0xe8bd68, 0x89a967].forEach((color, i) => dot(20 + i * 20, 23, 5, color));
  text("caper", 94, 7, 26, ink, true);
  panel(395, 9, 412, 27, 0x252d30, 20);
  text("⌕   Search messages, files, and people", 410, 13, 13, muted);
  text("?", 1116, 12, 17, muted);
  avatar(1, 1170, 24, 27, true);

  // Workspace navigation.
  panel(3, 46, 210, 750, 0x151c1e);
  panel(213, 45, 1, 752, 0x3b4548, 20);
  panel(20, 63, 38, 38, 0x303b3d, 22);
  text("S", 31, 67, 24, ink, true, 29);
  text("Studio", 72, 73, 20, ink, true);
  text("⌄", 185, 75, 18, muted);
  (["inbox", "chat", "saved"] as const).forEach((name, i) => {
    icon(name, 24, 132 + i * 38);
    text(["Inbox", "Threads", "Saved"][i], 56, 130 + i * 38, 17);
  });
  dot(186, 140, 10, orange);
  text("3", 182, 132, 13, ink, false, 38);
  panel(20, 250, 174, 1, 0x384246, 20);
  text("Channels", 22, 267, 14, muted);
  text("+", 179, 260, 24, muted);
  panel(12, 296, 190, 36, 0x4b2d26, 24);
  ["general", "design", "building", "random"].forEach((label, i) => {
    text("#", 25, 302 + i * 39, 21, i ? muted : ink, false, 32);
    text(label, 56, 306 + i * 39, 16, ink, false, 32);
  });
  dot(178, 353, 4, orange);
  panel(20, 471, 174, 1, 0x384246, 20);
  text("Direct messages", 22, 489, 14, muted);
  text("+", 179, 482, 24, muted);
  ["Maya", "Alex", "Sam"].forEach((name, i) => {
    avatar([0, 2, 3][i], 36, 537 + i * 45, 30, i !== 1);
    text(name, 62, 528 + i * 45, 16);
  });
  panel(4, 729, 207, 1, 0x384246, 20);
  avatar(1, 36, 763, 35, true);
  text("Jose", 64, 743, 15, ink, true);
  text("Online", 64, 765, 12, muted);
  text("♬  ⚙", 148, 754, 19, muted);

  // Channel header and the main conversation.
  text("#", 236, 61, 28, muted);
  text("general", 270, 60, 23, ink, true);
  text("A little work. A little everything.", 270, 90, 12, muted);
  [0, 1, 2].forEach((person, i) => avatar(person, 635 + i * 23, 80, 29));
  pill("    Start call", 711, 65, 108, orange);
  icon("call", 720, 72, 15, 50);
  panel(215, 119, 623, 1, 0x384246, 20);
  message(0, "Maya", "10:24 AM", 139, "This was the moment the whole plan fell apart.");

  // A real little low-poly diorama sits inside the video attachment.
  const videoStart = group.children.length;
  panel(292, 196, 386, 193, 0x303e4d, 25, 13);
  panel(292, 355, 386, 34, 0x222b2e, 41, 10);
  text("▷   the-plan.mp4", 306, 363, 14, ink, false, 49);
  text("148 MB", 585, 365, 12, muted, false, 49);
  for (let i = 0; i < 8; i++) {
    const rock = dot(317 + i * 47, 287 + Math.sin(i * 2) * 25, 36, i % 2 ? 0x536474 : 0x465665, 30);
    rock.scale.set(0.8, 1.6, 0.7);
  }
  for (let i = 0; i < 12; i++) {
    const plank = panel(303 + i * 29, 298 - i * 1.5, 27, 17, i % 2 ? 0x947354 : 0x795b45, 77, 12);
    plank.rotation.z = -0.08;
  }
  // Caper bud actors: faceted body, feet, leaf, eyes and waving hands.
  const caper = (x: number, y: number, color: number, lean: number) => {
    const body = dot(x, y, 20, color, 99);
    body.scale.set(0.88, 1.1, 0.8);
    body.rotation.z = lean;
    dot(x - 10, y + 21, 8, color, 96).scale.y = 0.5;
    dot(x + 10, y + 21, 8, color, 96).scale.y = 0.5;
    const leaf = dot(x + 3, y - 20, 10, color, 103);
    leaf.scale.set(1.3, 0.4, 0.6);
    leaf.rotation.z = 0.6;
    dot(x - 7, y - 2, 2.4, 0x10191a, 115);
    dot(x + 7, y - 2, 2.4, 0x10191a, 115);
    dot(x - 23, y - 7, 7, color, 98);
    dot(x + 22, y + 4, 7, color, 98);
  };
  caper(357, 265, 0x84954c, 0.18);
  caper(563, 268, 0xb5b9ad, -0.3);
  caper(621, 260, 0xc87845, 0.12);
  dot(486, 274, 23, 0x1c282d, 125);
  text("▶", 479, 262, 23, ink, false, 151);
  pill("0:18", 625, 321, 45);
  const video = new Group();
  video.add(...group.children.slice(videoStart));
  group.add(video);
  pill("♥  12", 292, 405, 63, 0x352923, "#ee9a83");
  pill("☺  6", 364, 405, 56);
  pill("☺ +", 429, 405, 43);
  [0, 2, 3].forEach((person, i) => avatar(person, 304 + i * 19, 457, 24));
  text("4 replies", 365, 450, 13, muted);

  message(1, "Jose", "10:26 AM", 484, "The new upload flow is ready to go.");
  panel(292, 541, 521, 68, 0x26302f, 27, 13);
  text("◉   captur.es", 306, 549, 17, ink, true, 37);
  text("✓  All checks passed", 321, 577, 13, "#a8cb8c", false, 37);
  pill("Deploy", 630, 560, 72, orange);
  pill("View changes", 711, 560, 94);
  message(2, "Alex", "10:28 AM", 629, "@caper  find the clip Maya posted yesterday.");
  avatar(0, 259, 703, 30);
  text("caper", 292, 687, 14, ink, true);
  text("APP", 340, 689, 10, muted);
  text("Found it in #random. One more round?", 292, 708, 13, "#a8bc8c");
  panel(231, 735, 586, 48, 0x283133, 25, 12);
  text("+", 245, 742, 27, muted, false, 34);
  text("Message #general", 281, 751, 14, muted, false, 34);
  icon("attach", 658, 749);
  text("@   ☺", 694, 747, 20, muted, false, 34);
  pill("➤", 770, 744, 36, orange);

  // The thread is a separate raised pane, like an open drawer.
  panel(840, 46, 357, 750, 0x192123, 23, 16);
  panel(840, 46, 2, 750, 0x465053, 34);
  text("Thread", 860, 71, 20, ink, true, 36);
  text("×", 1160, 64, 28, muted, false, 36);
  panel(858, 119, 321, 1, 0x384246, 34);
  avatar(0, 878, 158, 35);
  text("Maya", 906, 142, 16, ink, true, 37);
  text("10:24 AM", 961, 145, 11, muted, false, 37);
  text("This was the moment the whole", 906, 170, 13, ink, false, 37);
  text("plan fell apart.", 906, 192, 13, ink, false, 37);
  const threadVideo = video.clone();
  const thumbnailScale = 265 / 386;
  threadVideo.scale.setScalar(thumbnailScale);
  threadVideo.position.set(3.06 + 3.08 * thumbnailScale, 1.75 - 2.04 * thumbnailScale, 0.2);
  group.add(threadVideo);
  text("4 replies", 860, 384, 12, muted, false, 36);
  panel(924, 394, 247, 1, 0x384246, 34);
  const replies = [
    { person: 2, name: "Alex", time: "10:25 AM", body: "A perfectly executed disaster.", reaction: "♥  4" },
    { person: 1, name: "Jose", time: "10:27 AM", body: "We are absolutely trying that again.", reaction: "☺  3" },
    { person: 3, name: "Sam", time: "10:31 AM", body: "I saved the clip. Obviously.", reaction: "♥  2" },
    { person: 0, name: "Maya", time: "10:34 AM", body: "Same time tomorrow?", reaction: "☺  3" },
  ];
  replies.forEach((reply, i) => {
    const y = 420 + i * 77;
    avatar(reply.person, 878, y + 15, 34);
    const nameWidth = text(reply.name, 906, y, 16, ink, true, 37);
    text(reply.time, 906 + nameWidth + 10, y + 4, 11, muted, false, 37);
    text(reply.body, 906, y + 26, 12, ink, false, 37);
    pill(reply.reaction, 906, y + 48, 49);
  });
  panel(856, 735, 325, 48, 0x2b3436, 36, 10);
  text("Reply in thread...", 869, 744, 13, muted, false, 44);
  icon("attach", 868, 764, 13, 44);
  text("@   ☺", 894, 764, 14, muted, false, 44);
  text("➤", 1152, 752, 19, "#82908f", false, 44);

  return {
    group,
    whenReady,
    dispose() {
      geometries.forEach((geometry) => geometry.dispose());
      materials.forEach((surface) => surface.dispose());
      textures.forEach((texture) => texture.dispose());
    },
  };
}
