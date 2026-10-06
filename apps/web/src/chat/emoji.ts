// Twemoji keeps presentation selectors inside ZWJ sequences only. Its file
// names have unpadded hexadecimal code points (e.g. 1️⃣ -> 31-20e3.svg).
export function emojiAsset(unified: string) {
  const points = unified.toLowerCase().split("-").map((point) => Number.parseInt(point, 16).toString(16));
  const filename = (points.includes("200d") ? points : points.filter((point) => point !== "fe0f")).join("-");
  return `/emoji/twemoji-15/${filename}.svg`;
}

export function emojiCode(emoji: string) {
  return Array.from(emoji, (point) => point.codePointAt(0)!.toString(16)).join("-");
}

// The picker displays the last alias. Prefer dashes, but accept all three
// separator spellings in search across web and the generated native catalog.
export function emojiNames(names: string[]) {
  const label = names[names.length - 1];
  // CLDR's "flag: Country" is a display label, not a shortcode. Use the
  // country alone, normalized for the composer's ASCII shortcode input.
  const preferred = (label.startsWith("flag: ")
    ? label.slice(6).normalize("NFD").replace(/\p{M}/gu, "").toLowerCase()
      .replaceAll("&", "and").replace(/[.'’()]/g, "")
    : label).replace(/[\s_]+/g, "-");
  const aliases = [...names, preferred].flatMap((name) => {
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
  const preload = fetch(`/emoji/twemoji-15/${manifest}.json`).then(async (response) => {
    if (!response.ok) throw new Error("Emoji preload unavailable.");
    const unified: string[] = await response.json();
    await Promise.all(unified.map((code) => {
      const image = new Image();
      image.src = emojiAsset(code);
      return image.decode();
    }));
  }).catch(() => { imagePreloads.delete(category); });
  imagePreloads.set(category, preload);
  return preload;
}
