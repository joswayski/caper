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
