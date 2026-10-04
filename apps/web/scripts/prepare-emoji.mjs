import { cp, copyFile, mkdir, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import { basename, dirname, join } from "node:path";
import { emojiAsset } from "../src/chat/emoji.ts";

// Serve licensed artwork ourselves: no third-party CDN requests from chat.
const require = createRequire(import.meta.url);
const source = dirname(require.resolve("@twemoji/svg/package.json"));
const target = new URL("../public/emoji/twemoji-15/", import.meta.url);
await mkdir(target, { recursive: true });
await cp(source, target, { recursive: true, filter: (path) => path === source || path.endsWith(".svg") || path === join(source, "license") });
// Some artwork files use an unqualified sequence while the picker supplies
// presentation selectors. Alias those filenames and verify every offered asset.
const data = require("emoji-picker-react/dist/data/emojis-en").default;
for (const emoji of Object.values(data.emojis).flat().filter((emoji) => Number(emoji.a) <= 15)) {
  const filename = basename(emojiAsset(emoji.u));
  if (existsSync(new URL(filename, target))) continue;
  await copyFile(join(source, filename.replaceAll("-fe0f", "")), new URL(filename, target));
}
// Cover the opening grid and its overscan, including the wider mobile drawer,
// without sending the entire catalog when a user merely hovers the trigger.
await writeFile(new URL("preload.json", target), JSON.stringify(
  data.emojis.smileys_people.filter((emoji) => Number(emoji.a) <= 15).slice(0, 128).map((emoji) => emoji.u),
));
