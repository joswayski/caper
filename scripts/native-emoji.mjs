// Regenerate the native artwork from the same locked packages as the web picker.
// Requires npm ci and ImageMagick 7. Native builds consume the committed output.
import { execFileSync } from "node:child_process";
import { cp, mkdir, readdir, writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { fileURLToPath } from "node:url";
import { basename, join } from "node:path";
import { emojiAsset, emojiNames } from "../apps/web/src/chat/emoji.ts";
import "../apps/web/scripts/prepare-emoji.mjs";

const require = createRequire(import.meta.url);
const source = fileURLToPath(new URL("../apps/web/public/emoji/twemoji-15/", import.meta.url));
const target = fileURLToPath(new URL("../shared/emoji/", import.meta.url));
await mkdir(target, { recursive: true });
const data = require("emoji-picker-react/dist/data/emojis-en").default;
const offered = new Map();
for (const [category, entries] of Object.entries(data.emojis)) {
  for (const entry of entries.filter((item) => Number(item.a) <= 15)) {
    const id = basename(emojiAsset(entry.u), ".svg");
    const names = emojiNames(entry.n);
    offered.set(id, { name: names.at(-1), keywords: names.join(" "), category });
  }
}
const files = (await readdir(source)).filter((name) => name.endsWith(".svg")).sort();
const entries = files.map((file, index) => {
  const id = basename(file, ".svg");
  return {
    id, emoji: String.fromCodePoint(...id.split("-").map((point) => parseInt(point, 16))),
    name: offered.get(id)?.name ?? id, keywords: offered.get(id)?.keywords ?? "",
    category: offered.get(id)?.category ?? "", selectable: offered.has(id),
    sheet: Math.floor(index / 256), x: (index % 16) * 64, y: Math.floor((index % 256) / 16) * 64,
  };
});
for (let index = 0; index < files.length; index += 256) {
  const output = join(target, `sheet-${index / 256}.png`);
  execFileSync("magick", ["montage", "-background", "none", ...files.slice(index, index + 256).map((file) => join(source, file)),
    "-geometry", "64x64+0+0", "-tile", "16x16", `PNG32:${output}`], { stdio: "inherit" });
  execFileSync("magick", [output, "-background", "none", "-gravity", "NorthWest", "-extent", "1024x1024", "-strip", `PNG32:${output}`]);
}
await writeFile(join(target, "catalog.json"), JSON.stringify(entries) + "\n");
// SwiftPM bundles resources inside its target directory. Keep that generated
// copy (including attribution/licenses) identical to Android/desktop's source.
await cp(target, fileURLToPath(new URL("../apps/native/apple/Sources/CaperCore/EmojiAssets/", import.meta.url)), { recursive: true });
console.log(`Generated ${entries.length} image entries, ${offered.size} picker choices, ${Math.ceil(files.length / 256)} sheets.`);
