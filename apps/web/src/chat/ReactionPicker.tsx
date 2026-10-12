import { useState } from "react";
import { Bus, Cat, Flag, Music, Search, Shirt, Smile, Utensils, Volleyball, X } from "lucide-react";
import { VirtuosoGrid } from "react-virtuoso";
import english from "emoji-picker-react/dist/data/emojis-en";
import { emojiAsset, emojiNames, preloadEmojiImages } from "./emoji.ts";
import emojiLabels from "./emoji-labels.json";

const categories = [
  { category: "smileys_people", name: "Smileys & people", icon: Smile },
  { category: "animals_nature", name: "Animals & nature", icon: Cat },
  { category: "food_drink", name: "Food & drink", icon: Utensils },
  { category: "travel_places", name: "Travel & places", icon: Bus },
  { category: "activities", name: "Activities", icon: Volleyball },
  { category: "objects", name: "Objects", icon: Shirt },
  { category: "symbols", name: "Symbols", icon: Music },
  { category: "flags", name: "Flags", icon: Flag },
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
            <item.icon size={22} aria-hidden="true" />
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
