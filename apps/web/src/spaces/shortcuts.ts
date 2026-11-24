// Keyboard navigation shared by the quick switcher and the sidebar shortcuts.
// Every client implements the same matching and stepping, checked against
// shared/shortcuts/switcher-cases.json.

const WORD = /[\p{L}\p{N}]/u;

/** How well `query` matches `text`: higher is better, null for no match, 0 for an empty query. */
export function switcherScore(query: string, text: string): number | null {
  const q = Array.from(query.trim().replace(/^[#@]/, "").toLowerCase());
  if (!q.length) return 0;
  const lower = text.toLowerCase();
  const joined = q.join("");
  if (lower === joined) return 1000;
  if (lower.startsWith(joined)) return 900;
  const t = Array.from(lower);
  const wordStart = (index: number) => index === 0 || !WORD.test(t[index - 1]);
  for (let index = t.indexOf(q[0]); index >= 0; index = t.indexOf(q[0], index + 1))
    if (wordStart(index) && t.slice(index, index + q.length).join("") === joined) return 800;
  if (lower.includes(joined)) return 700;
  let score = 100;
  let last = -2;
  let from = 0;
  for (const character of q) {
    const index = t.indexOf(character, from);
    if (index < 0) return null;
    if (index === last + 1) score += 10;
    if (wordStart(index)) score += 5;
    score -= index - from;
    last = index;
    from = index + 1;
  }
  return Math.min(699, Math.max(1, score));
}

export interface Rankable {
  name: string;
  alternates?: string[];
  unread: boolean;
}

/** The matching items, best first; unread ones lead among equals. */
export function rankSwitcher<T extends Rankable>(items: T[], query: string): T[] {
  const scored: { item: T; key: number; index: number }[] = [];
  items.forEach((item, index) => {
    let best: number | null = null;
    for (const text of [item.name, ...(item.alternates ?? [])]) {
      const score = switcherScore(query, text);
      if (score !== null && (best === null || score > best)) best = score;
    }
    if (best !== null) scored.push({ item, key: best + (item.unread ? 50 : 0), index });
  });
  return scored.sort((a, b) => b.key - a.key || a.index - b.index).map((entry) => entry.item);
}

/** The index Alt+Up/Down (or with Shift, unread only) moves to, wrapping; null when there is none. */
export function adjacentConversation(
  unread: boolean[],
  current: number | null,
  forward: boolean,
  unreadOnly: boolean,
): number | null {
  const count = unread.length;
  for (let step = 0; step < count; step++) {
    const index =
      current === null
        ? forward
          ? step
          : count - 1 - step
        : (((current + (forward ? step + 1 : -step - 1)) % count) + count) % count;
    if (index === current) return null;
    if (!unreadOnly || unread[index]) return index;
  }
  return null;
}

/** Apple keyboards use Command where others use Control. */
export const appleKeyboard =
  typeof navigator !== "undefined" && /Mac|iPhone|iPad|iPod/.test(navigator.platform || navigator.userAgent);

/** Command on Apple keyboards, Control elsewhere, with no other primary modifier. */
export function primaryModifier(event: KeyboardEvent) {
  return appleKeyboard ? event.metaKey && !event.ctrlKey : event.ctrlKey && !event.metaKey;
}

/** Key names as this platform writes them. */
export function keyLabel(keys: string[]) {
  const names: Record<string, [apple: string, other: string]> = {
    mod: ["⌘", "Ctrl"],
    alt: ["⌥", "Alt"],
    shift: ["⇧", "Shift"],
    up: ["↑", "↑"],
    down: ["↓", "↓"],
    enter: ["Return", "Enter"],
    esc: ["Esc", "Esc"],
  };
  return keys.map((key) => names[key]?.[appleKeyboard ? 0 : 1] ?? key);
}

/** The keyboard shortcuts sheet: what each key does on the web. */
export const SHORTCUT_GROUPS: { title: string; shortcuts: { label: string; keys: string[][] }[] }[] = [
  {
    title: "Navigation",
    shortcuts: [
      { label: "Quick switcher", keys: [["mod", "K"]] },
      {
        label: "Previous or next conversation",
        keys: [
          ["alt", "up"],
          ["alt", "down"],
        ],
      },
      {
        label: "Previous or next unread conversation",
        keys: [
          ["alt", "shift", "up"],
          ["alt", "shift", "down"],
        ],
      },
      { label: "Keyboard shortcuts", keys: [["mod", "/"]] },
    ],
  },
  {
    title: "Conversation",
    shortcuts: [
      { label: "Mark as read", keys: [["esc"]] },
      { label: "Edit your last message (in an empty message box)", keys: [["up"]] },
      { label: "Jump to latest (from the messages)", keys: [["End"]] },
      { label: "New line", keys: [["shift", "enter"]] },
      {
        label: "Bold, italic, underline, code",
        keys: [
          ["mod", "B"],
          ["mod", "I"],
          ["mod", "U"],
          ["mod", "E"],
        ],
      },
    ],
  },
  {
    title: "Voice",
    shortcuts: [
      { label: "Mute or unmute", keys: [["mod", "shift", "M"]] },
      { label: "Deafen or undeafen", keys: [["mod", "shift", "D"]] },
    ],
  },
];
