// The API's name-character rule (apps/api/src/names.rs): names (display and
// space names) keep visible text but not control characters, Unicode
// Default_Ignorable_Code_Point characters (bidi controls, zero-width
// characters, soft hyphen, Hangul fillers, tags…) or the braille blank. Emoji
// keep their parts: a lone ZWJ/ZWNJ between visible characters, one variation
// selector after a visible character, and tags inside a subdivision flag.
// Message text is not checked by this rule.

const FLAG = 0x1f3f4;
const CANCEL_TAG = 0xe007f;

const joiner = (c: number) => c === 0x200c || c === 0x200d;
const selector = (c: number) =>
  (c >= 0x180b && c <= 0x180d) || c === 0x180f || (c >= 0xfe00 && c <= 0xfe0f) || (c >= 0xe0100 && c <= 0xe01ef);
const tag = (c: number) => c >= 0xe0020 && c <= 0xe007e;
const HIDDEN: readonly (readonly [number, number])[] = [
  [0x0000, 0x001f],
  [0x007f, 0x009f],
  [0x00ad, 0x00ad],
  [0x034f, 0x034f],
  [0x061c, 0x061c],
  [0x115f, 0x1160],
  [0x17b4, 0x17b5],
  [0x180b, 0x180f],
  [0x200b, 0x200f],
  [0x202a, 0x202e],
  [0x2060, 0x206f],
  [0x2800, 0x2800],
  [0x3164, 0x3164],
  [0xfe00, 0xfe0f],
  [0xfeff, 0xfeff],
  [0xffa0, 0xffa0],
  [0xfff0, 0xfff8],
  [0x1bca0, 0x1bca3],
  [0x1d173, 0x1d17a],
  [0xe0000, 0xe0fff],
];
const hidden = (c: number) => HIDDEN.some(([start, end]) => c >= start && c <= end);
// Rust's char::is_whitespace (Unicode White_Space), minus the controls above.
const space = (character: string) => /^\p{White_Space}$/u.test(character);

/** Whether `value` holds a character a name may not (see above). */
export function hasHiddenNameCharacters(value: string) {
  let last: "start" | "space" | "visible" | "flag" | "selector" = "start";
  let joined = false;
  let tags = 0;
  for (const character of value) {
    const c = character.codePointAt(0)!;
    if (tags > 0 && !tag(c) && c !== CANCEL_TAG) return true;
    if (joined && (joiner(c) || selector(c) || tag(c) || c === CANCEL_TAG || hidden(c) || space(character)))
      return true;
    joined = false;
    if (tag(c)) {
      if (last !== "flag" && tags === 0) return true;
      tags += 1;
    } else if (c === CANCEL_TAG) {
      if (tags === 0) return true;
      tags = 0;
      last = "visible";
    } else if (joiner(c)) {
      if (last !== "visible" && last !== "flag" && last !== "selector") return true;
      joined = true;
    } else if (selector(c)) {
      if (last !== "visible" && last !== "flag") return true;
      last = "selector";
    } else if (hidden(c)) {
      return true;
    } else if (space(character)) {
      last = "space";
    } else {
      last = c === FLAG ? "flag" : "visible";
    }
  }
  return joined || tags > 0;
}
