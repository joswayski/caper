// 3D architecture diagram for the README.
// node scripts/generate-architecture-diagram.mjs [--check]
// Edit the layout below (grid units: x runs right, y runs toward the viewer), then regenerate.
// The script warns when a label would cover a model or another label.
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { logos } from "./architecture-diagram-logos.mjs";

const root = new URL("../", import.meta.url);
const read = (path, encoding) => readFileSync(new URL(path, root), encoding);
const output = fileURLToPath(new URL("docs/architecture.svg", root));

// Camera: turned 15° from the x axis and looking down about 38°, so the flow reads left to right.
const UNIT = 30;
const AZIMUTH = (15 * Math.PI) / 180;
const [SA, CA] = [Math.sin(AZIMUTH), Math.cos(AZIMUTH)];
const [SE, CE] = [0.62, Math.sqrt(1 - 0.62 ** 2)];
const FLOOR = 0.3;
const project = (x, y, z = 0) => [(x * CA - y * SA) * UNIT, (x * SA + y * CA) * SE * UNIT - z * CE * UNIT];
const fixed = (n) => n.toFixed(2).replace(/\.?0+$/, "");
const point = (x, y, z) => project(x, y, z).map(fixed).join(",");
// Local drawing planes, in grid units: the floor (u along x, v along y) and an upright face toward the viewer (u along x, v down).
const floorPlane = (x, y, z) =>
  `matrix(${fixed(CA * UNIT)},${fixed(SA * SE * UNIT)},${fixed(-SA * UNIT)},${fixed(CA * SE * UNIT)},${point(x, y, z)})`;
const frontPlane = (x, y, z) =>
  `matrix(${fixed(CA * UNIT)},${fixed(SA * SE * UNIT)},0,${fixed(CE * UNIT)},${point(x, y, z)})`;
// Screen angle of the x axis, so text can run along the floor's edges.
const EDGE_ANGLE = (Math.atan2(SA * SE, CA) * 180) / Math.PI;
const escape = (text) => text.replaceAll("&", "&amp;").replaceAll("<", "&lt;");
const shade = (hex, amount) => {
  const n = parseInt(hex.slice(1), 16);
  const channel = (shift) => Math.round(((n >> shift) & 255) * amount);
  return `#${((channel(16) << 16) | (channel(8) << 8) | channel(0)).toString(16).padStart(6, "0")}`;
};

const palettes = {
  client: { top: "#F1EDE4", left: "#D3CDC1", right: "#A8A295" },
  graphite: { top: "#4A4F53", left: "#2E3134", right: "#222427" },
  cloudflare: { top: "#C9603F", left: "#B64D32", right: "#853823" },
  aws: { top: "#7C9657", left: "#637A43", right: "#475831" },
  neutral: { top: "#4A4F53", left: "#34383B", right: "#26292C" },
  ses: { top: "#EA5B6E", left: "#DD344C", right: "#A8263A" },
};

// Screen-space bounds, collected per model so labels can be placed under it and checked for overlaps.
let bounds;
const grow = ([x, y]) => {
  bounds[0] = Math.min(bounds[0], x);
  bounds[1] = Math.min(bounds[1], y);
  bounds[2] = Math.max(bounds[2], x);
  bounds[3] = Math.max(bounds[3], y);
};
const polygon = (fill, points) => {
  points.forEach((p) => grow(project(...p)));
  return `<polygon fill="${fill}" points="${points.map((p) => point(...p)).join(" ")}"/>`;
};
// A box from back corner (x, y) with width w along x, depth d along y and height h; only the faces toward the camera.
const prism = (x, y, w, d, h, z, colors) =>
  polygon(colors.left, [
    [x, y + d, z],
    [x + w, y + d, z],
    [x + w, y + d, z + h],
    [x, y + d, z + h],
  ]) +
  polygon(colors.right, [
    [x + w, y, z],
    [x + w, y + d, z],
    [x + w, y + d, z + h],
    [x + w, y, z + h],
  ]) +
  polygon(colors.top, [
    [x, y, z + h],
    [x + w, y, z + h],
    [x + w, y + d, z + h],
    [x, y + d, z + h],
  ]);

const logoWidth = (name, height) => (logos[name].box[0] / logos[name].box[1]) * height;
// Each logo outline is defined once and reused, in its own colours or as one solid fill for extrusion sides.
const defined = new Map();
const logo = (name, height, fill) => {
  const {
    box: [w, h],
    paths,
  } = logos[name];
  const id = `logo-${name}${fill ? `-${fill.slice(1)}` : ""}`;
  if (!defined.has(id)) {
    defined.set(
      id,
      `<g id="${id}">${paths.map(([color, d]) => `<path fill="${fill ?? color}" d="${d}"/>`).join("")}</g>`,
    );
  }
  return `<use href="#${id}" transform="scale(${(height / h).toFixed(5)}) translate(${fixed(-w / 2)},${fixed(-h)})"/>`;
};
// A logo standing upright at (x, y), extruded backward so it reads as a solid piece.
const standingLogo = (name, x, y, z, height, depth = 0.26) => {
  const half = logoWidth(name, height) / 2;
  for (const [dx, dz] of [
    [-half, 0],
    [half, 0],
    [-half, height],
    [half, height],
  ]) {
    grow(project(x + dx, y, z + dz));
    grow(project(x + dx, y - depth, z + dz));
  }
  const side = shade(logos[name].paths[0][0], 0.42);
  const layers = [];
  const steps = Math.max(8, Math.round(depth * 30));
  for (let step = steps; step >= 1; step--) {
    layers.push(`<g transform="${frontPlane(x, y - (depth * step) / steps, z)}">${logo(name, height, side)}</g>`);
  }
  layers.push(`<g transform="${frontPlane(x, y, z)}">${logo(name, height)}</g>`);
  return layers.join("");
};
// Models. Each draws itself centred on grid point (x, y) and returns its footprint for edge planning.
const models = {
  // One or more logos standing on the floor, side by side.
  logos(node) {
    const names = [node.logo].flat();
    const height = node.logoHeight ?? 1.7;
    const gap = 0.35;
    const widths = names.map((name) => logoWidth(name, height));
    let left = node.x - (widths.reduce((sum, w) => sum + w, 0) + gap * (names.length - 1)) / 2;
    return names
      .map((name, index) => {
        const svg = standingLogo(name, left + widths[index] / 2, node.y, FLOOR, height, node.depth);
        left += widths[index] + gap;
        return svg;
      })
      .join("");
  },
  monitor({ x, y }) {
    let svg = prism(x - 0.7, y - 0.6, 1.4, 0.9, 0.07, FLOOR, palettes.client);
    svg += prism(x - 0.15, y - 0.25, 0.3, 0.2, 0.62, FLOOR + 0.07, palettes.client);
    svg += prism(x - 1.45, y - 0.05, 2.9, 0.14, 1.75, FLOOR + 0.5, palettes.client);
    const bars = [
      [0.78, 0.52, 1.1, "#34383B"],
      [0.78, 0.72, 0.8, "#34383B"],
      [0.78, 0.92, 1.25, "#B64D32"],
      [0.78, 1.12, 0.95, "#34383B"],
      [0.78, 1.32, 0.6, "#34383B"],
    ];
    svg += `<g transform="${frontPlane(x - 1.45, y + 0.09, FLOOR + 2.25)}">
<rect x="0.08" y="0.08" width="2.74" height="1.59" rx="0.05" fill="#0C0D0F"/>
<rect x="0.08" y="0.08" width="2.74" height="0.24" fill="#26292C"/>
${["#B64D32", "#F1EDE4", "#637A43"].map((fill, i) => `<circle cx="${0.22 + i * 0.13}" cy="0.2" r="0.045" fill="${fill}"/>`).join("")}
<rect x="0.75" y="0.14" width="1.4" height="0.12" rx="0.06" fill="#151719"/>
<rect x="0.08" y="0.32" width="0.55" height="1.35" fill="#151719"/>
${[0.45, 0.65, 0.85].map((v) => `<rect x="0.16" y="${v}" width="0.38" height="0.08" rx="0.04" fill="#34383B"/>`).join("")}
${bars.map(([u, v, w, fill]) => `<rect x="${u}" y="${v}" width="${w}" height="0.1" rx="0.05" fill="${fill}"/>`).join("")}
</g>`;
    return svg;
  },
  phones({ x, y }) {
    let svg = "";
    for (const [dx, name] of [
      [-0.8, "apple"],
      [0.8, "android"],
    ]) {
      svg += prism(x + dx - 0.55, y - 0.07, 1.1, 0.14, 2.15, FLOOR, palettes.graphite);
      svg += `<g transform="${frontPlane(x + dx - 0.55, y + 0.07, FLOOR + 2.15)}">
<rect x="0.07" y="0.08" width="0.96" height="1.99" rx="0.12" fill="#0C0D0F"/>
<rect x="0.42" y="0.15" width="0.26" height="0.07" rx="0.035" fill="#26292C"/>
<g transform="translate(0.55,1.12)">${logo(name, 0.5)}</g>
<rect x="0.2" y="1.36" width="0.7" height="0.08" rx="0.04" fill="#34383B"/>
<rect x="0.2" y="1.52" width="0.46" height="0.08" rx="0.04" fill="#B64D32"/>
</g>`;
    }
    return svg;
  },
  laptop({ x, y }) {
    const face = read("apps/web/public/caper-face.svg", "utf8").match(/<path[^>]+\/>/)[0];
    let svg = prism(x - 1.3, y - 0.85, 2.6, 0.1, 1.55, FLOOR + 0.09, palettes.client);
    svg += `<g transform="${frontPlane(x - 1.3, y - 0.75, FLOOR + 1.64)}">
<rect x="0.08" y="0.08" width="2.44" height="1.39" rx="0.05" fill="#0C0D0F"/>
<g transform="translate(1.3,0.78) scale(0.0062) translate(-994,-170)">${face}</g>
</g>`;
    svg += prism(x - 1.3, y - 0.75, 2.6, 1.55, 0.09, FLOOR, palettes.client);
    const keys = [];
    for (let row = 0; row < 3; row++) {
      for (let col = 0; col < 11; col++) {
        keys.push(
          `<rect x="${fixed(0.22 + col * 0.2)}" y="${fixed(0.2 + row * 0.2)}" width="0.15" height="0.15" rx="0.03"/>`,
        );
      }
    }
    svg += `<g transform="${floorPlane(x - 1.3, y - 0.75, FLOOR + 0.09)}" fill="#C6C0B3">${keys.join("")}<rect x="0.95" y="0.9" width="0.7" height="0.45" rx="0.05" fill="#DCD7CD"/></g>`;
    return svg;
  },
  bucket({ x, y }) {
    const [bx, by] = project(x, y, FLOOR);
    const [tx, ty] = project(x, y, FLOOR + 1.3);
    const [bottom, top] = [0.78 * UNIT, 1.02 * UNIT];
    grow([bx - top, ty - top * SE]);
    grow([bx + top, by + bottom * SE]);
    return `<path fill="url(#bucket)" d="M${fixed(bx - bottom)},${fixed(by)}A${fixed(bottom)},${fixed(bottom * SE)} 0 0 0 ${fixed(bx + bottom)},${fixed(by)}L${fixed(tx + top)},${fixed(ty)}A${fixed(top)},${fixed(top * SE)} 0 0 1 ${fixed(tx - top)},${fixed(ty)}Z"/>
<ellipse cx="${fixed(tx)}" cy="${fixed(ty)}" rx="${fixed(top)}" ry="${fixed(top * SE)}" fill="#D9734F"/>
<ellipse cx="${fixed(tx)}" cy="${fixed(ty + 1.5)}" rx="${fixed(top - 4)}" ry="${fixed(top * SE - 3)}" fill="#6E2E1C"/>
<g transform="${frontPlane(x, y + 0.92, FLOOR + 0.35)}">${logo("cloudflare", 0.62, "#F1EDE4")}</g>`;
  },
  cube({ x, y }) {
    const s = 1.5;
    let svg = prism(x - s / 2, y - s / 2, s, s, s, FLOOR, palettes.ses);
    svg += `<g transform="${frontPlane(x, y + s / 2, FLOOR + 0.18)}">${logo("ses", s * 0.95)}</g>`;
    return svg;
  },
};

const R0 = -6.2;
const [R1, R2, R3] = [2.2, 9.4, 16.6];
// Columns: clients, the two Cloudflare columns, AWS, Valkey, PlanetScale.
const [C, A, B, K, V, P] = [1.6, 10.6, 16.4, 24.6, 30.1, 36.5];
// Edge labels between clients and Cloudflare sit in the gap between the two zones.
const GAP = 6.2;
const zones = [
  { name: "CLIENTS", x: -1.2, y: 0, w: 5.8, d: 21.4, accent: "#F1EDE4" },
  { name: "CLOUDFLARE", x: 7.8, y: 0, w: 11.6, d: 21.4, accent: "#D9734F" },
  { name: "AWS · K3S", x: 21, y: 0, w: 11.8, d: 21.4, accent: "#8FAA66" },
  { name: "PLANETSCALE", x: 34.2, y: R2 - 3.4, w: 4.6, d: 6.8, accent: "#9AA0A4" },
  { name: "THIRD-PARTY SERVICES", x: -1.2, y: -9.4, w: 34.8, d: 6.4, accent: "#9AA0A4", above: true },
];

// A label is [title, detail], or [detail] alone when the logo already names the service.
const nodes = [
  { model: "monitor", x: C, y: R1, label: ["Web", "browser app"] },
  { model: "phones", x: C, y: R2, label: ["iOS · Android", "native apps"] },
  { model: "laptop", x: C, y: R3, label: ["Desktop", "macOS · Windows · Linux"] },
  { model: "logos", x: A, y: R1, logo: "cloudflare", logoHeight: 1.9, label: ["Realtime SFU / TURN", "voice calls"] },
  {
    model: "logos",
    x: A,
    y: R2,
    logo: "tunnel",
    logoHeight: 1.45,
    depth: 0.85,
    label: ["Cloudflare Tunnel", "caper.chat ingress"],
  },
  { model: "logos", x: A, y: R3, logo: "workers", label: ["CDN Worker", "cdn.caper.chat"] },
  { model: "logos", x: B, y: R1, logo: "workers", label: ["Embed Worker", "link previews"] },
  { model: "bucket", x: B, y: R3, label: ["R2", "uploads"] },
  { model: "logos", x: K, y: R1, logo: "rust", label: ["Rust API", "accounts · spaces · messages"] },
  { model: "logos", x: K, y: R2, logo: "rust", label: ["Rust gateway", "live WebSocket"] },
  { model: "logos", x: K, y: R3, logo: "tanstack", label: ["Web server", "TanStack Start"] },
  { model: "logos", x: V, y: R3, logo: "valkey", label: ["Valkey", "fanout · presence · calls"] },
  { model: "logos", x: P, y: R2, logo: "planetscale", label: ["PlanetScale", "Postgres"] },
  { model: "logos", x: C, y: R0, logo: "klipy", logoHeight: 0.95, label: ["GIFs · stickers"] },
  {
    model: "logos",
    x: B,
    y: R0,
    logo: ["x", "youtube", "tiktok", "instagram"],
    logoHeight: 1.05,
    label: ["Linked sites", "fetched for previews"],
  },
  { model: "cube", x: 24.4, y: R0, label: ["Amazon SES", "sign-in email"] },
  { model: "logos", x: 28.6, y: R0, logo: ["apple", "firebase"], logoHeight: 1.25, label: ["APNs / FCM", "push"] },
  { model: "logos", x: 32.6, y: R0, logo: "openrouter", logoHeight: 1.35, label: ["OpenRouter", "transcripts"] },
];

// Edges run on the floor through grid waypoints. `both` puts an arrow at each end; `arrow: false` at neither.
const edges = [
  {
    path: [
      [3.2, R1],
      [A - 1.1, R1],
    ],
    both: true,
    label: [GAP, R1, "voice audio"],
  },
  {
    path: [
      [3.1, R2],
      [A - 1, R2],
    ],
    label: [GAP, R2, "HTTPS · WebSocket"],
  },
  {
    path: [
      [3.1, R3],
      [A - 1, R3],
    ],
    label: [GAP + 0.5, R3, "signed download"],
  },
  {
    path: [
      [4.9, R3],
      [4.9, R3 + 3.9],
      [B, R3 + 3.9],
      [B, R3 + 1.05],
    ],
    label: [GAP, R3 + 3.9, "direct upload"],
  },
  {
    path: [
      [A + 1, R3],
      [B - 1.1, R3],
    ],
  },
  {
    path: [
      [C, R1 - 0.6],
      [C, R0 + 0.4],
    ],
    label: [C, -2.3, "GIF search"],
  },
  {
    path: [
      [B, R1 - 0.6],
      [B, R0 + 0.4],
    ],
  },
  {
    path: [
      [A + 1, R2],
      [K - 1, R2],
    ],
  },
  {
    path: [
      [22, R1 - 0.1],
      [22, R3],
    ],
    arrow: false,
  },
  {
    path: [
      [22, R1 - 0.1],
      [K - 1, R1 - 0.1],
    ],
  },
  {
    path: [
      [22, R3],
      [K - 1, R3],
    ],
  },
  {
    path: [
      [K - 1, R1 - 0.4],
      [B + 1, R1 - 0.4],
    ],
    label: [20.5, R1 - 0.4, "unfurl"],
  },
  {
    path: [
      [K - 0.4, R1 - 0.6],
      [K - 0.4, -1.3],
      [A, -1.3],
      [A, R1 - 0.6],
    ],
    label: [13.6, -1.3, "call control"],
  },
  {
    path: [
      [K + 1, R1 - 0.1],
      [V + 0.8, R1 - 0.1],
      [V + 0.8, R3 - 0.6],
    ],
    both: true,
  },
  {
    path: [
      [K + 1, R2 - 0.1],
      [V - 0.8, R2 - 0.1],
      [V - 0.8, R3 - 0.6],
    ],
    both: true,
  },
  {
    path: [
      [K + 1, R1 - 0.45],
      [P, R1 - 0.45],
      [P, R2 - 0.6],
    ],
    label: [33.5, R1 - 0.45, "SQL · outbox"],
  },
  {
    path: [
      [K + 1, R2 - 0.45],
      [P - 1, R2 - 0.45],
    ],
  },
  {
    path: [
      [K + 0.6, R1 - 0.6],
      [K + 0.6, -2.3],
    ],
    arrow: false,
  },
  {
    path: [
      [24.4, -2.3],
      [32.6, -2.3],
    ],
    arrow: false,
  },
  ...[24.4, 28.6, 32.6].map((x) => ({
    path: [
      [x, -2.3],
      [x, R0 + (x === 24.4 ? 0.8 : 0.4)],
    ],
  })),
];

const layers = { zones: [], edges: [], models: [], labels: [] };
const pills = [];
const all = [Infinity, Infinity, -Infinity, -Infinity];
const include = (box) => {
  all[0] = Math.min(all[0], box[0]);
  all[1] = Math.min(all[1], box[1]);
  all[2] = Math.max(all[2], box[2]);
  all[3] = Math.max(all[3], box[3]);
};

for (const zone of zones) {
  const { x, y, w, d } = zone;
  bounds = [Infinity, Infinity, -Infinity, -Infinity];
  const grid = [];
  for (let gx = Math.ceil(x); gx < x + w; gx++) grid.push(`M${point(gx, y, FLOOR)}L${point(gx, y + d, FLOOR)}`);
  for (let gy = Math.ceil(y); gy < y + d; gy++) grid.push(`M${point(x, gy, FLOOR)}L${point(x + w, gy, FLOOR)}`);
  const [lx, ly] = zone.above ? project(x + 0.3, y, FLOOR) : project(x + 0.3, y + d, 0);
  layers.zones.push(
    prism(x, y, w, d, FLOOR, 0, { top: "#17191C", left: "#111315", right: "#0E0F11" }),
    `<path d="${grid.join("")}" stroke="#202327" stroke-width="1" fill="none"/>`,
    `<polygon points="${[
      [x, y],
      [x + w, y],
      [x + w, y + d],
      [x, y + d],
    ]
      .map((p) => point(...p, FLOOR))
      .join(" ")}" fill="none" stroke="${zone.accent}" stroke-opacity="0.45" stroke-width="1.2"/>`,
    `<text class="halo" transform="translate(${fixed(lx)},${fixed(ly)}) rotate(${fixed(EDGE_ANGLE)})" y="${zone.above ? -9 : 20}" font-size="12.5" font-weight="700" letter-spacing="0.12em" fill="${zone.accent}">${escape(zone.name)}</text>`,
  );
  include(bounds);
}

for (const edge of edges) {
  const d = `M${edge.path.map(([x, y]) => point(x, y, FLOOR)).join("L")}`;
  const markers = `${edge.arrow === false ? "" : ' marker-end="url(#arrow)"'}${edge.both ? ' marker-start="url(#arrow)"' : ""}`;
  layers.edges.push(`<path d="${d}" class="wire"/><path d="${d}" class="flow"${markers}/>`);
  if (edge.label) {
    const [lx, ly, text] = edge.label;
    const [px, py] = project(lx, ly, FLOOR);
    const width = text.length * 6.3 + 18;
    pills.push({ name: text, box: [px - width / 2, py - 10, px + width / 2, py + 10] });
    layers.labels.push(
      `<rect x="${fixed(px - width / 2)}" y="${fixed(py - 10)}" width="${fixed(width)}" height="20" rx="10" fill="#0C0D0F" stroke="#34383B"/>`,
      `<text x="${fixed(px)}" y="${fixed(py + 4)}" text-anchor="middle" font-size="11.5" font-weight="500" fill="#C9C4BA">${escape(text)}</text>`,
    );
  }
}

const placed = [];
for (const node of nodes) {
  bounds = [Infinity, Infinity, -Infinity, -Infinity];
  const svg = models[node.model](node);
  const box = [...bounds];
  include(box);
  const [, , , bottom] = box;
  const [cx] = project(node.x, node.y, FLOOR);
  const [title, detail] = node.label.length === 2 ? node.label : [null, node.label[0]];
  const half = Math.max((title ?? "").length * 4.1, detail.length * 3.3) + 4;
  const top = title ? 0 : -16;
  const labelBox = [cx - half, bottom + 6, cx + half, bottom + 42 + top];
  include(labelBox);
  if (title) {
    layers.labels.push(
      `<text class="halo" x="${fixed(cx)}" y="${fixed(bottom + 22)}" text-anchor="middle" font-size="14.5" font-weight="700" fill="#F3F4F5">${escape(title)}</text>`,
    );
  }
  layers.labels.push(
    `<text class="halo" x="${fixed(cx)}" y="${fixed(bottom + 38 + top)}" text-anchor="middle" font-size="12" font-weight="500" fill="#9AA0A4">${escape(detail)}</text>`,
  );
  placed.push({ name: title ?? detail, depth: node.x * SA + node.y * CA, svg, box, labelBox });
}
layers.models = placed.sort((a, b) => a.depth - b.depth).map((model) => model.svg);

const overlaps = (a, b) => a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3];
for (const a of placed) {
  for (const b of placed) {
    if (a !== b && overlaps(a.labelBox, b.box)) console.warn(`Label "${a.name}" covers the "${b.name}" model`);
    if (a.name < b.name && overlaps(a.labelBox, b.labelBox)) console.warn(`Labels "${a.name}" and "${b.name}" overlap`);
  }
  for (const pill of pills) {
    if (overlaps(pill.box, a.box)) console.warn(`Edge label "${pill.name}" covers the "${a.name}" model`);
    if (overlaps(pill.box, a.labelBox)) console.warn(`Edge label "${pill.name}" covers the "${a.name}" label`);
  }
}

const font = (weight) =>
  `@font-face{font-family:Satoshi;font-weight:${weight};src:url(data:font/woff2;base64,${read(`apps/web/public/fonts/satoshi-${weight}.woff2`).toString("base64")}) format("woff2")}`;
const lettering = read("apps/web/public/caper-wordmark-letters.svg", "utf8").match(/<path[^>]+\/>/)[0];

const pad = 40;
const header = 84;
const [minX, minY, maxX, maxY] = all;
const width = Math.ceil(maxX - minX + pad * 2);
const height = Math.ceil(maxY - minY + pad * 2 + header);
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="Caper architecture: web, iOS, Android and desktop clients reach the Rust API, Rust gateway and web server on AWS through Cloudflare Tunnel; voice audio flows through Cloudflare's Realtime SFU, files through R2 and the CDN Worker, link previews through the Embed Worker; data lives in PlanetScale Postgres with Valkey for fanout; the API uses Amazon SES, APNs/FCM and OpenRouter, and clients search GIFs on KLIPY.">
<style>${font(500)}${font(700)}
text{font-family:Satoshi,-apple-system,"Segoe UI",Helvetica,Arial,sans-serif}
.halo{paint-order:stroke;stroke:#0C0D0F;stroke-width:4px;stroke-linejoin:round}
.wire{fill:none;stroke:#2E3236;stroke-width:4;stroke-linecap:round;stroke-linejoin:round}
.flow{fill:none;stroke:#F1EDE4;stroke-opacity:.75;stroke-width:1.6;stroke-dasharray:5 9;stroke-linecap:round;animation:flow 1.4s linear infinite}
@keyframes flow{to{stroke-dashoffset:-14}}
@media (prefers-reduced-motion:reduce){.flow{animation:none}}
</style>
<defs>
${[...defined.values()].join("\n")}
<linearGradient id="bucket" x1="0" x2="1"><stop offset="0" stop-color="#C9603F"/><stop offset="1" stop-color="#853823"/></linearGradient>
<marker id="arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" markerUnits="userSpaceOnUse" orient="auto-start-reverse"><path d="M0,1L9,5L0,9Z" fill="#F1EDE4"/></marker>
</defs>
<rect width="${width}" height="${height}" rx="16" fill="#0C0D0F"/>
<g transform="translate(${pad},${pad - 6}) scale(0.1)"><g transform="translate(-20,-17)">${lettering}</g></g>
<text x="${pad + 122}" y="${pad + 13}" font-size="22" font-weight="700" fill="#F3F4F5">Architecture</text>
<text x="${pad}" y="${pad + 46}" font-size="13" font-weight="500" fill="#9AA0A4">Control traffic runs on AWS. Voice audio, files and link previews stay on Cloudflare.</text>
<g transform="translate(${fixed(pad - minX)},${fixed(pad + header - minY)})">
${layers.zones.join("\n")}
${layers.edges.join("\n")}
${layers.models.join("\n")}
${layers.labels.join("\n")}
</g>
</svg>
`;

if (process.argv.includes("--check"))
  assert.equal(read("docs/architecture.svg", "utf8"), svg, "Architecture diagram drift");
else writeFileSync(output, svg);
console.log(
  `${process.argv.includes("--check") ? "Verified" : "Exported"} docs/architecture.svg (${width}×${height}).`,
);
