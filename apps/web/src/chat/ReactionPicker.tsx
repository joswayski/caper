import { useState } from "react";
import { Search, X } from "lucide-react";
import { VirtuosoGrid } from "react-virtuoso";
import english from "emoji-picker-react/dist/data/emojis-en";
import { emojiAsset, emojiNames, preloadEmojiImages } from "./emoji.ts";
import emojiLabels from "./emoji-labels.json";

const categories = [
  { category: "smileys_people", name: "Smileys & people", icon: "1f600" },
  { category: "animals_nature", name: "Animals & nature", icon: "1f431" },
  { category: "food_drink", name: "Food & drink", icon: "1f354" },
  { category: "travel_places", name: "Travel & places", icon: "1f68c" },
  { category: "activities", name: "Activities", icon: "26bd" },
  { category: "objects", name: "Objects", icon: "1f455" },
  { category: "symbols", name: "Symbols", icon: "1f3b5" },
  { category: "flags", name: "Flags", icon: "1f3c1" },
];
const choices = Object.entries(english.emojis).flatMap(([category, entries]) =>
  entries
    .filter((entry) => Number(entry.a) <= 15)
    .map((entry) => {
      const names = emojiNames(entry.n, (emojiLabels as Record<string, string>)[entry.u]);
      return {
        category,
        code: entry.u,
        emoji: String.fromCodePoint(...entry.u.split("-").map((point) => Number.parseInt(point, 16))),
        name: names[names.length - 1],
        keywords: names.join(" ").toLowerCase(),
      };
    }),
);

export default function ReactionPicker({ onSelect }: { onSelect: (emoji: string) => void }) {
  const [category, setCategory] = useState(categories[0]);
  const [query, setQuery] = useState("");
  const term = query.trim().toLowerCase();
  const shown = choices.filter((entry) =>
    term
      ? entry.keywords.includes(term) || entry.category.replaceAll("_", " ").includes(term)
      : entry.category === category.category,
  );
  // A single grid owns visibility. Category jumps must not depend on a second
  // IntersectionObserver deciding whether an otherwise visible section exists.
  return (
    <div className="chat-emoji-picker">
      <div className="chat-emoji-categories" role="group" aria-label="Emoji categories">
        {categories.map((item) => (
          <button
            key={item.category}
            type="button"
            data-category={item.category}
            aria-label={item.name}
            aria-pressed={!term && category === item}
            title={item.name}
            onPointerEnter={() => void preloadEmojiImages(item.category)}
            onFocus={() => void preloadEmojiImages(item.category)}
            onClick={() => {
              setCategory(item);
              setQuery("");
              void preloadEmojiImages(item.category);
            }}
          >
            <img src={emojiAsset(item.icon)} alt="" width={22} height={22} loading="eager" />
          </button>
        ))}
      </div>
      <div className="chat-emoji-search">
        <Search size={18} aria-hidden="true" />
        <input
          aria-label="Search emoji"
          placeholder="Search emoji"
          autoFocus
          value={query}
          onChange={(event) => setQuery(event.target.value)}
        />
        {query && (
          <button type="button" aria-label="Clear search" onClick={() => setQuery("")}>
            <X size={16} />
          </button>
        )}
      </div>
      <strong className="chat-emoji-category-label">{term ? "Search results" : category.name}</strong>
      <span className={shown.length ? "sr-only" : "chat-emoji-empty"} role="status">
        {shown.length ? `${shown.length} emoji` : "No results. Try another search."}
      </span>
      {!!shown.length && (
        <VirtuosoGrid
          key={`${category.category}:${term}`}
          className="chat-emoji-grid"
          data={shown}
          increaseViewportBy={80}
          listClassName="chat-emoji-grid-list"
          itemClassName="chat-emoji-grid-item"
          itemContent={(_, entry) => (
            <button
              type="button"
              data-unified={entry.code}
              aria-label={entry.name}
              title={entry.name}
              onClick={() => onSelect(entry.emoji)}
            >
              <img src={emojiAsset(entry.code)} alt={entry.name} width={26} height={26} loading="eager" />
            </button>
          )}
        />
      )}
    </div>
  );
}
