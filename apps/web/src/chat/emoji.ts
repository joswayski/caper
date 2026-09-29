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
