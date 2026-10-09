export interface EmojiChoice {
  id: string;
  emoji: string;
  name: string;
  keywords: string;
}

export interface EmojiToken {
  start: number;
  end: number;
  query: string;
}
const queryCharacter = /^[a-z0-9_+-]$/i;
const normalize = (value: string) => value.toLowerCase().replace(/[_-]/g, " ");

export function emojiToken(text: string, start: number, end = start): EmojiToken | undefined {
  if (start !== end || start < 0 || start > text.length) return;
  if (text[start] === ":" || queryCharacter.test(text[start] ?? "")) return;
  let colon = start - 1;
  while (colon >= 0 && queryCharacter.test(text[colon])) colon--;
  if (text[colon] !== ":" || (colon > 0 && !/[\s([{]/u.test(text[colon - 1]))) return;
  const query = text.slice(colon + 1, start);
  // One character is an emoticon (":D", ":P", ":3"), not a search; Enter sends it as typed.
  if (query.length === 1) return;
  return { start: colon, end: start, query };
}

export function emojiSuggestions(catalog: EmojiChoice[], query: string): EmojiChoice[] {
  if (!query)
    return ["1f44d", "1f600", "2764", "1f389", "1f680", "1f440"].flatMap(
      (id) => catalog.find((entry) => entry.id === id) ?? [],
    );
  const needle = normalize(query);
  return (
    catalog
      .map((entry) => {
        const name = normalize(entry.name),
          keywords = normalize(entry.keywords);
        const rank =
          name === needle
            ? 0
            : name.startsWith(needle)
              ? 1
              : keywords.startsWith(needle) || keywords.includes(` ${needle}`)
                ? 2
                : name.includes(needle) || keywords.includes(needle)
                  ? 3
                  : 4;
        return { entry, rank };
      })
      .filter(({ rank }) => rank < 4)
      // Within a rank the shorter name is the closer match: ":fi" offers 🔥 fire before 🎞️ film-frames.
      .sort((a, b) => a.rank - b.rank || a.entry.name.length - b.entry.name.length)
      .slice(0, 6)
      .map(({ entry }) => entry)
  );
}

export function insertEmoji(text: string, token: EmojiToken, emoji: string) {
  const value = text.slice(0, token.start) + emoji + text.slice(token.end);
  if (Array.from(value).length > 4_000) return;
  return { value, caret: token.start + emoji.length };
}

let catalog: Promise<EmojiChoice[]> | undefined;
export function loadEmojiChoices() {
  return (catalog ??= fetch("/emoji/twemoji-15/autocomplete-v3.json")
    .then(async (response) => {
      if (!response.ok) throw new Error("Emoji suggestions unavailable.");
      return (await response.json()) as EmojiChoice[];
    })
    .catch((error) => {
      catalog = undefined;
      throw error;
    }));
}
