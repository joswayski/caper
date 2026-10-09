// Regenerate apps/web/src/chat/emoji-labels.json: CLDR short names for picker
// entries whose package names would otherwise pick a slang or category alias
// (🔥 "litaf", 👀 "looking"). Then run scripts/native-emoji.mjs --catalog-only.
import { writeFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { emojiNames } from "../apps/web/src/chat/emoji.ts";

// Twemoji 15 and the picker's `a <= 15` filter both target Emoji 15.0.
const source = "https://unicode.org/Public/emoji/15.0/emoji-test.txt";
const require = createRequire(import.meta.url);
const data = require("emoji-picker-react/dist/data/emojis-en").default;

const response = await fetch(source, { signal: AbortSignal.timeout(30_000) });
if (!response.ok) throw new Error(`${source} returned ${response.status}`);
const cldr = new Map();
for (const line of (await response.text()).split("\n")) {
  const match = line.match(/^([0-9A-F ]+?)\s*;\s*(?:fully-qualified|component)\s*#\s*\S+\s+E[\d.]+\s+(.+)$/u);
  if (match) cldr.set(match[1].toLowerCase().split(" ").join("-"), match[2]);
}

// Hyphenation and case differences are already the same shortcode.
const key = (name) => name.toLowerCase().replace(/[\s_-]+/g, "-");
const labels = {};
for (const entry of Object.values(data.emojis)
  .flat()
  .filter((item) => Number(item.a) <= 15)) {
  const label = cldr.get(entry.u) ?? cldr.get(entry.u.replaceAll("-fe0f", ""));
  if (!label) throw new Error(`No CLDR name for ${entry.u}`);
  // Flags already use their country; CLDR's "Congo - Kinshasa" spacing is not typeable.
  if (label.startsWith("flag: ")) continue;
  if (key(emojiNames(entry.n).at(-1)) !== key(label)) labels[entry.u] = label;
}
const sorted = Object.fromEntries(Object.entries(labels).sort(([a], [b]) => a.localeCompare(b)));
await writeFile(
  new URL("../apps/web/src/chat/emoji-labels.json", import.meta.url),
  JSON.stringify(sorted, null, 2) + "\n",
);
console.log(`Wrote ${Object.keys(sorted).length} CLDR labels from ${source}.`);
