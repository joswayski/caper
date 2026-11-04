// Isometric architecture diagram for the README.
// node scripts/generate-architecture-diagram.mjs [--check]
// Edit the layout below (grid units: x runs down-right, y runs down-left), then regenerate.
import assert from "node:assert/strict";
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const read = (path, encoding) => readFileSync(new URL(path, root), encoding);
const output = fileURLToPath(new URL("docs/architecture.svg", root));

const UNIT = 34;
const COS = Math.cos(Math.PI / 6);
const FLOOR = 0.35;
const project = (x, y, z = 0) => [(x - y) * COS * UNIT, (x + y) * 0.5 * UNIT - z * UNIT];
const point = (x, y, z) =>
  project(x, y, z)
    .map((n) => n.toFixed(1))
    .join(",");
const polygon = (fill, points) => `<polygon fill="${fill}" points="${points.map((p) => point(...p)).join(" ")}"/>`;
const escape = (text) => text.replaceAll("&", "&amp;").replaceAll("<", "&lt;");
// Maps local text coordinates onto the isometric floor at (x, y, z), reading down-right along x.
const floorText = (x, y, z, size, fill, text, anchor = "start", weight = 700) => {
  const [tx, ty] = project(x, y, z);
  return `<text transform="matrix(${COS.toFixed(4)},0.5,${(-COS).toFixed(4)},0.5,${tx.toFixed(1)},${ty.toFixed(1)})" font-size="${size}" font-weight="${weight}" fill="${fill}" text-anchor="${anchor}" dominant-baseline="central" letter-spacing="0.06em">${escape(text)}</text>`;
};

const palettes = {
  client: { top: "#F1EDE4", left: "#CFC9BC", right: "#A39D90", ink: "#0C0D0F" },
  cloudflare: { top: "#C9603F", left: "#B64D32", right: "#853823", ink: "#F1EDE4" },
  aws: { top: "#7C9657", left: "#637A43", right: "#475831", ink: "#F1EDE4" },
  neutral: { top: "#565B60", left: "#3F4347", right: "#2B2E31", ink: "#F3F4F5" },
};

const zones = [
  { name: "CLIENTS", x: 0, y: 0, w: 4.6, d: 13.4, accent: "#F1EDE4" },
  { name: "CLOUDFLARE", x: 6, y: 0, w: 8.4, d: 13.4, accent: "#D9734F" },
  { name: "AWS · K3S", x: 15.6, y: 0, w: 11.4, d: 13.4, accent: "#8FAA66" },
  { name: "PLANETSCALE", x: 27.7, y: 2.6, w: 4.2, d: 5, accent: "#9AA0A4" },
  { name: "THIRD-PARTY APIS", x: 15.6, y: -6.6, w: 10.4, d: 5.4, accent: "#9AA0A4" },
];

// Boxes: [x, y] is the back corner, w along x, d along y. `top` is the short label printed on the lid.
const boxes = [
  ...[
    ["WEB", 0.8],
    ["iOS · macOS", 3.9],
    ["ANDROID", 7],
    ["DESKTOP", 10.1],
  ].map(([top, y]) => ({ x: 0.8, y, w: 2, d: 1.8, h: 0.55, palette: "client", top, size: top.length > 8 ? 8 : 10 })),
  { x: 0.8, y: -4.6, w: 2, d: 1.8, h: 0.6, palette: "neutral", top: "GIF", label: ["KLIPY", "GIFs · stickers"] },
  {
    x: 7,
    y: 0.8,
    w: 2,
    d: 2,
    h: 0.8,
    palette: "cloudflare",
    top: "SFU",
    label: ["Realtime SFU / TURN", "voice calls"],
  },
  { x: 11, y: 0.8, w: 2, d: 2, h: 0.8, palette: "cloudflare", top: "EMBED", label: ["Embed Worker", "link previews"] },
  {
    x: 11,
    y: -4.6,
    w: 2,
    d: 1.8,
    h: 0.6,
    palette: "neutral",
    top: "WWW",
    label: ["Linked sites", "X · YouTube · TikTok…"],
  },
  {
    x: 7,
    y: 5.3,
    w: 2,
    d: 2,
    h: 0.8,
    palette: "cloudflare",
    top: "TUNNEL",
    label: ["Cloudflare Tunnel", "caper.chat ingress"],
  },
  { x: 7, y: 9.8, w: 2, d: 2, h: 0.8, palette: "cloudflare", top: "CDN", label: ["CDN Worker", "cdn.caper.chat"] },
  {
    x: 17.4,
    y: 1.5,
    w: 2.2,
    d: 2,
    h: 1.1,
    palette: "aws",
    top: "API",
    label: ["Rust API", "accounts · spaces · messages"],
  },
  { x: 17.4, y: 7.4, w: 2.2, d: 2, h: 1.1, palette: "aws", top: "GATEWAY", label: ["Rust gateway", "live WebSocket"] },
  { x: 17.4, y: 11, w: 2.2, d: 1.8, h: 0.8, palette: "aws", top: "WEB", label: ["Web server", "TanStack Start"] },
  { x: 16.6, y: -5.6, w: 2, d: 1.8, h: 0.6, palette: "neutral", top: "SES", label: ["Amazon SES", "sign-in email"] },
  { x: 19.9, y: -5.6, w: 2, d: 1.8, h: 0.6, palette: "neutral", top: "PUSH", label: ["APNs / FCM", "notifications"] },
  { x: 23.2, y: -5.6, w: 2, d: 1.8, h: 0.6, palette: "neutral", top: "AI", label: ["OpenRouter", "transcripts"] },
];

const cylinders = [
  { x: 12, y: 10.8, r: 1, h: 1.1, palette: "cloudflare", top: "R2", label: ["R2", "uploads"] },
  { x: 23.6, y: 5.4, r: 0.95, h: 1, palette: "aws", top: "KV", label: ["Valkey", "fanout · presence · calls"] },
  { x: 30.1, y: 5.6, r: 1.1, h: 1.2, palette: "neutral", top: "SQL", label: ["Postgres", "durable data"] },
];

// Edges run on the floor through grid waypoints. `both` draws arrows at each end.
const edges = [
  {
    path: [
      [3.6, 1.7],
      [3.6, 12.6],
    ],
    arrow: false,
  },
  ...[1.7, 4.8, 7.9, 11].map((y) => ({
    path: [
      [2.8, y],
      [3.6, y],
    ],
    arrow: false,
  })),
  {
    path: [
      [3.6, 1.8],
      [7, 1.8],
    ],
    both: true,
    label: [5.3, 1.8, "voice audio"],
  },
  {
    path: [
      [3.6, 6.3],
      [7, 6.3],
    ],
    label: [5.3, 6.3, "HTTPS · WebSocket"],
  },
  {
    path: [
      [3.6, 10.8],
      [7, 10.8],
    ],
    label: [5.3, 10.8, "signed download"],
  },
  {
    path: [
      [3.6, 12.6],
      [12, 12.6],
      [12, 11.8],
    ],
    label: [5.3, 12.6, "direct upload"],
  },
  {
    path: [
      [9, 10.8],
      [11, 10.8],
    ],
  },
  {
    path: [
      [2.4, 0.8],
      [2.4, -2.8],
    ],
    label: [2.4, -0.3, "GIF search"],
  },
  {
    path: [
      [12, 0.8],
      [12, -2.8],
    ],
  },
  {
    path: [
      [9, 6.3],
      [16.4, 6.3],
    ],
    arrow: false,
  },
  {
    path: [
      [16.4, 2.5],
      [16.4, 11.9],
    ],
    arrow: false,
  },
  ...[2.5, 8.4, 11.9].map((y) => ({
    path: [
      [16.4, y],
      [17.4, y],
    ],
  })),
  {
    path: [
      [17.9, 1.5],
      [17.9, 1.2],
      [13, 1.2],
    ],
    label: [15.2, 1.2, "unfurl"],
  },
  {
    path: [
      [18.9, 1.5],
      [18.9, 0.4],
      [8, 0.4],
      [8, 0.8],
    ],
    label: [10, 0.4, "call control"],
  },
  {
    path: [
      [19.6, 3],
      [23.6, 3],
      [23.6, 4.45],
    ],
    both: true,
  },
  {
    path: [
      [19.6, 7.9],
      [23.6, 7.9],
      [23.6, 6.35],
    ],
    both: true,
  },
  {
    path: [
      [19.6, 2.2],
      [26, 2.2],
      [26, 5.6],
    ],
    arrow: false,
  },
  {
    path: [
      [19.6, 8.9],
      [26, 8.9],
      [26, 5.6],
    ],
    arrow: false,
  },
  {
    path: [
      [26, 5.6],
      [29, 5.6],
    ],
    label: [27.35, 5.6, "SQL · outbox"],
  },
  {
    path: [
      [19.3, 1.5],
      [19.3, -2.4],
    ],
    arrow: false,
  },
  {
    path: [
      [17.6, -2.4],
      [24.2, -2.4],
    ],
    arrow: false,
  },
  ...[17.6, 20.9, 24.2].map((x) => ({
    path: [
      [x, -2.4],
      [x, -3.8],
    ],
  })),
];

const parts = { zones: [], edges: [], solids: [], labels: [] };
const bounds = [Infinity, Infinity, -Infinity, -Infinity];
const grow = ([x, y]) => {
  bounds[0] = Math.min(bounds[0], x);
  bounds[1] = Math.min(bounds[1], y);
  bounds[2] = Math.max(bounds[2], x);
  bounds[3] = Math.max(bounds[3], y);
};

const prism = (x, y, w, d, h, z, colors) => {
  for (const corner of [
    [x, y],
    [x + w, y],
    [x, y + d],
    [x + w, y + d],
  ])
    for (const level of [z, z + h + 0.8]) grow(project(...corner, level));
  return [
    polygon(colors.left, [
      [x, y + d, z],
      [x + w, y + d, z],
      [x + w, y + d, z + h],
      [x, y + d, z + h],
    ]),
    polygon(colors.right, [
      [x + w, y, z],
      [x + w, y + d, z],
      [x + w, y + d, z + h],
      [x + w, y, z + h],
    ]),
    polygon(colors.top, [
      [x, y, z + h],
      [x + w, y, z + h],
      [x + w, y + d, z + h],
      [x, y + d, z + h],
    ]),
  ].join("");
};

const label = ([px, py], [title, detail]) => {
  grow([px - 80, py + 34]);
  grow([px + 80, py]);
  parts.labels.push(
    `<text class="halo" x="${px.toFixed(1)}" y="${(py + 18).toFixed(1)}" text-anchor="middle" font-size="12.5" font-weight="700" fill="#F3F4F5">${escape(title)}</text>`,
    `<text class="halo" x="${px.toFixed(1)}" y="${(py + 33).toFixed(1)}" text-anchor="middle" font-size="10.5" font-weight="500" fill="#9AA0A4">${escape(detail)}</text>`,
  );
};

for (const zone of zones) {
  const { x, y, w, d } = zone;
  const grid = [];
  for (let gx = Math.ceil(x); gx < x + w; gx++) grid.push(`M${point(gx, y, FLOOR)}L${point(gx, y + d, FLOOR)}`);
  for (let gy = Math.ceil(y); gy < y + d; gy++) grid.push(`M${point(x, gy, FLOOR)}L${point(x + w, gy, FLOOR)}`);
  parts.zones.push(
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
    `<text x="${project(x, y)[0].toFixed(1)}" y="${(project(x, y)[1] - 14).toFixed(1)}" class="halo" text-anchor="middle" font-size="11" font-weight="700" letter-spacing="0.12em" fill="${zone.accent}">${escape(zone.name)}</text>`,
  );
}

for (const edge of edges) {
  const d = `M${edge.path.map(([x, y]) => point(x, y, FLOOR)).join("L")}`;
  const markers = `${edge.arrow === false ? "" : ' marker-end="url(#arrow)"'}${edge.both ? ' marker-start="url(#arrow)"' : ""}`;
  parts.edges.push(`<path d="${d}" class="wire"/><path d="${d}" class="flow"${markers}/>`);
  if (edge.label) {
    const [lx, ly, text] = edge.label;
    const [px, py] = project(lx, ly, FLOOR);
    const width = text.length * 5.6 + 14;
    parts.labels.push(
      `<rect x="${(px - width / 2).toFixed(1)}" y="${(py - 9).toFixed(1)}" width="${width.toFixed(1)}" height="18" rx="9" fill="#0C0D0F" stroke="#34383B"/>`,
      `<text x="${px.toFixed(1)}" y="${(py + 0.5).toFixed(1)}" text-anchor="middle" dominant-baseline="central" font-size="10" font-weight="500" fill="#C9C4BA">${escape(text)}</text>`,
    );
  }
}

const solids = [];
for (const box of boxes) {
  const colors = palettes[box.palette];
  const z = FLOOR;
  solids.push({
    depth: box.x + box.y + box.w + box.d,
    svg:
      prism(box.x, box.y, box.w, box.d, box.h, z, colors) +
      floorText(box.x + box.w / 2, box.y + box.d / 2, z + box.h, box.size ?? 10, colors.ink, box.top, "middle"),
  });
  if (box.label) label(project(box.x + box.w, box.y + box.d, z), box.label);
}
for (const cylinder of cylinders) {
  const { x, y, r, h } = cylinder;
  const colors = palettes[cylinder.palette];
  const [bx, by] = project(x, y, FLOOR);
  const [, ty] = project(x, y, FLOOR + h);
  const rx = Math.SQRT2 * r * COS * UNIT;
  const ry = Math.SQRT2 * r * 0.5 * UNIT;
  grow([bx - rx, ty - ry]);
  grow([bx + rx, by + ry]);
  const id = `body-${cylinder.palette}`;
  solids.push({
    depth: x + y + r * 2,
    svg:
      `<path fill="url(#${id})" d="M${(bx - rx).toFixed(1)},${ty.toFixed(1)}V${by.toFixed(1)}A${rx.toFixed(1)},${ry.toFixed(1)} 0 0 0 ${(bx + rx).toFixed(1)},${by.toFixed(1)}V${ty.toFixed(1)}Z"/>` +
      `<ellipse cx="${bx.toFixed(1)}" cy="${ty.toFixed(1)}" rx="${rx.toFixed(1)}" ry="${ry.toFixed(1)}" fill="${colors.top}"/>` +
      floorText(x, y, FLOOR + h, 10, colors.ink, cylinder.top, "middle"),
  });
  label([bx, by + ry], cylinder.label);
}
parts.solids = solids.sort((a, b) => a.depth - b.depth).map((solid) => solid.svg);

const gradients = Object.entries(palettes)
  .map(
    ([name, colors]) =>
      `<linearGradient id="body-${name}" x1="0" x2="1"><stop offset="0" stop-color="${colors.left}"/><stop offset="1" stop-color="${colors.right}"/></linearGradient>`,
  )
  .join("");

const font = (weight) =>
  `@font-face{font-family:Satoshi;font-weight:${weight};src:url(data:font/woff2;base64,${read(`apps/web/public/fonts/satoshi-${weight}.woff2`).toString("base64")}) format("woff2")}`;
const lettering = read("apps/web/public/caper-wordmark-letters.svg", "utf8").match(/<path[^>]+\/>/)[0];

const pad = 44;
const header = 92;
const [minX, minY, maxX, maxY] = bounds;
const width = Math.ceil(maxX - minX + pad * 2);
const height = Math.ceil(maxY - minY + pad * 2 + header);
const svg = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${width} ${height}" width="${width}" height="${height}" role="img" aria-label="Caper architecture: clients reach a Rust API, WebSocket gateway and web server on AWS through Cloudflare Tunnel; voice audio flows through Cloudflare's Realtime SFU, files through R2 and the CDN Worker, and data lives in PlanetScale Postgres with Valkey for fanout.">
<style>${font(500)}${font(700)}
text{font-family:Satoshi,-apple-system,"Segoe UI",Helvetica,Arial,sans-serif}
.halo{paint-order:stroke;stroke:#0C0D0F;stroke-width:4px;stroke-linejoin:round}
.wire{fill:none;stroke:#2E3236;stroke-width:4;stroke-linecap:round;stroke-linejoin:round}
.flow{fill:none;stroke:#F1EDE4;stroke-opacity:.75;stroke-width:1.6;stroke-dasharray:5 9;stroke-linecap:round;animation:flow 1.4s linear infinite}
@keyframes flow{to{stroke-dashoffset:-14}}
@media (prefers-reduced-motion:reduce){.flow{animation:none}}
</style>
<defs>${gradients}<marker id="arrow" viewBox="0 0 10 10" refX="8" refY="5" markerWidth="7" markerHeight="7" markerUnits="userSpaceOnUse" orient="auto-start-reverse"><path d="M0,1L9,5L0,9Z" fill="#F1EDE4"/></marker></defs>
<rect width="${width}" height="${height}" rx="16" fill="#0C0D0F"/>
<g transform="translate(${pad},${pad - 6}) scale(0.1)" opacity="0.98"><g transform="translate(-20,-17)">${lettering}</g></g>
<text x="${pad + 122}" y="${pad + 13}" font-size="22" font-weight="700" fill="#F3F4F5">Architecture</text>
<text x="${pad}" y="${pad + 46}" font-size="13" font-weight="500" fill="#9AA0A4">Control traffic runs on AWS. Voice audio, files and link previews stay on Cloudflare.</text>
<g transform="translate(${(pad - minX).toFixed(1)},${(pad + header - minY).toFixed(1)})">
${parts.zones.join("\n")}
${parts.edges.join("\n")}
${parts.solids.join("\n")}
${parts.labels.join("\n")}
</g>
</svg>
`;

if (process.argv.includes("--check"))
  assert.equal(read("docs/architecture.svg", "utf8"), svg, "Architecture diagram drift");
else writeFileSync(output, svg);
console.log(
  `${process.argv.includes("--check") ? "Verified" : "Exported"} docs/architecture.svg (${width}×${height}).`,
);
