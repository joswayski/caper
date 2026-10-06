import { cp, copyFile, mkdir, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, join } from "node:path";
import { emojiAsset } from "../src/chat/emoji.ts";
import catalog from "../../../shared/emoji/catalog.json" with { type: "json" };

// Serve licensed artwork ourselves: no third-party CDN requests from chat.
const require = createRequire(import.meta.url);
const source = dirname(require.resolve("@twemoji/svg/package.json"));
const target = new URL("../public/emoji/twemoji-15/", import.meta.url);
await mkdir(target, { recursive: true });
await cp(source, target, { recursive: true, filter: (path) => path === source || path.endsWith(".svg") || path === join(source, "license") });
// All composers use the native catalog's names/order, without its sprite metadata.
// Version metadata when names change: this directory is cached immutably.
await writeFile(new URL("autocomplete-v2.json", target), JSON.stringify(catalog.filter((entry) => entry.selectable)
  .map(({ id, emoji, name, keywords }) => ({ id, emoji, name, keywords }))));
// Some artwork files use an unqualified sequence while the picker supplies
// presentation selectors. Alias those filenames and verify every offered asset.
const data = require("emoji-picker-react/dist/data/emojis-en").default;
for (const emoji of Object.values(data.emojis).flat().filter((emoji) => Number(emoji.a) <= 15)) {
  const filename = basename(emojiAsset(emoji.u));
  if (existsSync(new URL(filename, target))) continue;
  await copyFile(join(source, filename.replaceAll("-fe0f", "")), new URL(filename, target));
}
// Cover each category's opening grid and overscan, including the wider mobile
// drawer, without sending the whole catalog on trigger or category intent.
for (const [category, emojis] of Object.entries(data.emojis)) {
  const manifest = category === "smileys_people" ? "preload" : `preload-${category}`;
  await writeFile(new URL(`${manifest}.json`, target), JSON.stringify(
    emojis.filter((emoji) => Number(emoji.a) <= 15).slice(0, 128).map((emoji) => emoji.u),
  ));
}
