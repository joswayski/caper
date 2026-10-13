// Twemoji keeps presentation selectors inside ZWJ sequences only. Its file
// names have unpadded hexadecimal code points (e.g. 1️⃣ -> 31-20e3.svg).
export function emojiAsset(unified: string) {
  const points = unified
    .toLowerCase()
    .split("-")
    .map((point) => Number.parseInt(point, 16).toString(16));
  const filename = (points.includes("200d") ? points : points.filter((point) => point !== "fe0f")).join("-");
  return `/emoji/twemoji-15/${filename}.svg`;
}

export function emojiCode(emoji: string) {
  return Array.from(emoji, (point) => point.codePointAt(0)!.toString(16)).join("-");
}

/** Keep joined emoji, flags, keycaps and skin tones together when inspecting a draft. */
export function emojiSegments(text: string) {
  const segments: { text: string; emoji?: true }[] = [];
  // Unicode property escapes work at our browser floor; Intl.Segmenter does not.
  const pattern =
    /\p{Regional_Indicator}{2}|[0-9#*]\ufe0f?\u20e3|\p{Extended_Pictographic}[\ufe0e\ufe0f\p{Emoji_Modifier}]*(?:\u200d\p{Extended_Pictographic}[\ufe0e\ufe0f\p{Emoji_Modifier}]*)*(?:[\u{e0020}-\u{e007e}]+\u{e007f})?/gu;
  let cursor = 0;
  for (const match of text.matchAll(pattern)) {
    if (match.index > cursor) segments.push({ text: text.slice(cursor, match.index) });
    segments.push({ text: match[0], emoji: true });
    cursor = match.index + match[0].length;
  }
  if (cursor < text.length) segments.push({ text: text.slice(cursor) });
  return segments;
}

// The picker displays the last alias. Prefer dashes, but accept all three
// separator spellings in search across web and the generated native catalog.
// The package orders names by length, so its last name can be slang or a
// category ("litaf" for 🔥); pass the CLDR name from emoji-labels.json then.
export function emojiNames(names: string[], cldrLabel?: string) {
  const label = cldrLabel ?? names[names.length - 1];
  // CLDR's "flag: Country" is a display label, not a shortcode. Use the
  // country alone, normalized for the composer's ASCII shortcode input.
  const preferred = (
    label.startsWith("flag: ")
      ? label
          .slice(6)
          .normalize("NFD")
          .replace(/\p{M}/gu, "")
          .toLowerCase()
          .replaceAll("&", "and")
          .replace(/[.'’()]/g, "")
      : label
  ).replace(/[\s_]+/g, "-");
  const aliases = [...names, label, preferred].flatMap((name) => {
    const dashed = name.replace(/[\s_]+/g, "-");
    return [name, dashed.replaceAll("-", " "), dashed.replaceAll("-", "_"), dashed];
  });
  return [...new Set(aliases.filter((name) => name !== preferred)), preferred];
}

const imagePreloads = new Map<string, Promise<void>>();
export function preloadEmojiImages(category = "smileys_people") {
  const cached = imagePreloads.get(category);
  if (cached) return cached;
  // Keep the existing immutable opening-grid manifest unchanged.
  const manifest = category === "smileys_people" ? "preload" : `preload-${category}`;
  const preload = fetch(`/emoji/twemoji-15/${manifest}.json`)
    .then(async (response) => {
      if (!response.ok) throw new Error("Emoji preload unavailable.");
      const unified: string[] = await response.json();
      await Promise.all(
        unified.map((code) => {
          const image = new Image();
          image.src = emojiAsset(code);
          return image.decode();
        }),
      );
    })
    .catch(() => {
      imagePreloads.delete(category);
    });
  imagePreloads.set(category, preload);
  return preload;
}
