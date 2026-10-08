// Export frozen vector masters to all clients. No tracing/image tool at build/runtime.
// node scripts/generate-avatar-vectors.mjs [--check]
import assert from "node:assert/strict";
import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const check = process.argv.includes("--check");
const write = (path, value) => {
  path = resolve(root, path);
  if (check) assert.equal(readFileSync(path, "utf8"), value, `Generated avatar drift: ${path}`);
  else {
    mkdirSync(dirname(path), { recursive: true });
    writeFileSync(path, value);
  }
};
const json = (value) => `${JSON.stringify(value, null, 2)}\n`;
const info = { author: "xcode", version: 1 };
const apple = "apps/native/apple/Sources/CaperCore/CaperAvatars.xcassets";
const android = "apps/native/android/app/src/main/res/drawable";
// The same HSL hue shift as v1's ImageMagick -modulate 100,100,(100 + color * 25).
function hue(hex, turns) {
  if (!turns) return hex;
  const rgb = [1, 3, 5].map((at) => parseInt(hex.slice(at, at + 2), 16) / 255);
  const high = Math.max(...rgb),
    low = Math.min(...rgb),
    delta = high - low;
  if (!delta) return hex;
  let h =
    (high === rgb[0]
      ? (rgb[1] - rgb[2]) / delta
      : high === rgb[1]
        ? 2 + (rgb[2] - rgb[0]) / delta
        : 4 + (rgb[0] - rgb[1]) / delta) / 6;
  h = (((h + turns) % 1) + 1) % 1;
  const light = (high + low) / 2;
  const saturation = delta / (1 - Math.abs(2 * light - 1));
  const chroma = (1 - Math.abs(2 * light - 1)) * saturation;
  const x = chroma * (1 - Math.abs(((h * 6) % 2) - 1));
  const channels = [
    [chroma, x, 0],
    [x, chroma, 0],
    [0, chroma, x],
    [0, x, chroma],
    [x, 0, chroma],
    [chroma, 0, x],
  ][Math.floor(h * 6)];
  return (
    "#" +
    channels
      .map((c) =>
        Math.round((c + light - chroma / 2) * 255)
          .toString(16)
          .padStart(2, "0"),
      )
      .join("")
      .toUpperCase()
  );
}
assert.equal(hue("#FF0000", 1 / 8), "#FFBF00"); // +45°, not saturation/lightness.
assert.equal(hue("#00FF00", 7 / 8), "#BFFF00"); // Wrap around 360°.
assert.equal(hue("#424242", 3 / 8), "#424242"); // Neutral ink stays neutral.
const circle = "M128 0A128 128 0 1 1 128 256A128 128 0 1 1 128 0Z";
const backgrounds = [
  "M0 0 C84.48 0 168.96 0 256 0 C256 84.48 256 168.96 256 256 C171.52 256 87.04 256 0 256 C0 171.52 0 87.04 0 0 Z",
  "M0 0H256V256H0Z",
];
const header = '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256" width="256" height="256">\n';
const kotlin = [];
const kotlinBranding = [];
const rust = [];
const rustBranding = [];
write(`${apple}/Contents.json`, json({ info }));
for (let id = 0; id < 800; id++) {
  // Only repaired characters have overrides; the other 40 retain v2's exact paths.
  const repaired = resolve(root, `assets/avatars/vector-v3/${id % 100}.svg`);
  const master = readFileSync(
    existsSync(repaired) ? repaired : resolve(root, `assets/avatars/vector-v2/${id % 100}.svg`),
    "utf8",
  );
  const paths = [
    ...master.matchAll(
      /<path d="([^"]+)" fill="(#[A-Fa-f0-9]{6})"(?: transform="translate\(([-\d.]+),([-\d.]+)\)")?\/>/g,
    ),
  ];
  assert.ok(paths.length > 0, `No paths for avatar ${id}`);
  assert.equal(paths.length, (master.match(/<path\b/g) ?? []).length, `Unsupported path in avatar ${id}`);
  const colored = paths.map(([, d, fill, x = "0", y = "0"]) => ({
    d,
    fill: hue(fill, Math.floor(id / 100) / 8),
    x,
    y,
  }));
  assert.ok(
    backgrounds.includes(colored[0].d) && Number(colored[0].x) === 0 && Number(colored[0].y) === 0,
    `Expected full-canvas background first for avatar ${id}`,
  );
  const elements = colored.map(
    ({ d, fill, x, y }) => `<path d="${d}" fill="${fill}" transform="translate(${x},${y})"/>`,
  );
  const svg =
    header +
    `<defs><clipPath id="crop"><path d="${circle}"/></clipPath></defs><g clip-path="url(#crop)">\n` +
    elements.join("\n") +
    "\n</g></svg>\n";
  write(`apps/web/public/images/avatars/v3/${id}.svg`, svg);
  // In-app branding keeps the character paths, without the avatar tile or circular crop.
  const branding = header + elements.slice(1).join("\n") + "\n</svg>\n";
  write(`apps/web/public/images/branding/v1/${id}.svg`, branding);
  for (const [name, artwork] of [
    ["caper-avatar", svg],
    ["caper-branding", branding],
  ]) {
    write(`${apple}/${name}-${id}.imageset/avatar.svg`, artwork);
    write(
      `${apple}/${name}-${id}.imageset/Contents.json`,
      json({
        images: [{ filename: "avatar.svg", idiom: "universal" }],
        info,
        properties: { "preserves-vector-representation": true, "template-rendering-intent": "original" },
      }),
    );
  }
  const vectorElements = colored.map(
    ({ d, fill, x, y }) =>
      `<group android:translateX="${x}" android:translateY="${y}"><path android:pathData="${d}" android:fillColor="${fill}"/></group>`,
  );
  write(
    `${android}/caper_avatar_${id}.xml`,
    '<?xml version="1.0" encoding="utf-8"?>\n' +
      '<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="256dp" android:height="256dp" android:viewportWidth="256" android:viewportHeight="256"><group>\n' +
      `<clip-path android:pathData="${circle}"/>\n` +
      vectorElements.join("\n") +
      "\n</group></vector>\n",
  );
  write(
    `${android}/caper_branding_${id}.xml`,
    '<?xml version="1.0" encoding="utf-8"?>\n' +
      '<vector xmlns:android="http://schemas.android.com/apk/res/android" android:width="256dp" android:height="256dp" android:viewportWidth="256" android:viewportHeight="256"><group>\n' +
      vectorElements.slice(1).join("\n") +
      "\n</group></vector>\n",
  );
  kotlin.push(`    R.drawable.caper_avatar_${id},`);
  kotlinBranding.push(`    R.drawable.caper_branding_${id},`);
  rust.push(`    include_bytes!("../../../web/public/images/avatars/v3/${id}.svg"),`);
  rustBranding.push(`    include_bytes!("../../../web/public/images/branding/v1/${id}.svg"),`);
}
write(
  "apps/native/android/app/src/main/java/chat/caper/android/AvatarResources.kt",
  "// Generated by scripts/generate-avatar-vectors.mjs; do not reorder persisted IDs.\npackage chat.caper.android\n\ninternal val caperAvatarResources = intArrayOf(\n" +
    kotlin.join("\n") +
    "\n)\n\ninternal val caperBrandingResources = intArrayOf(\n" +
    kotlinBranding.join("\n") +
    "\n)\n",
);
write(
  "apps/native/desktop/src/avatar_images.rs",
  "// Generated by scripts/generate-avatar-vectors.mjs; do not reorder persisted IDs.\npub const SVG: [&[u8]; 800] = [\n" +
    rust.join("\n") +
    "\n];\n\npub const BRANDING: [&[u8]; 800] = [\n" +
    rustBranding.join("\n") +
    "\n];\n",
);
console.log(
  `${check ? "Verified" : "Exported"} 800 path-only avatars and transparent branding variants for web, Rust desktop, Apple and Android; persisted IDs unchanged.`,
);
