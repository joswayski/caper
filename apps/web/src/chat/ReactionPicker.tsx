import type { SyntheticEvent } from "react";
import EmojiPicker, { Categories, EmojiStyle, Theme } from "emoji-picker-react";
import { emojiAsset, preloadEmojiImages } from "./emoji.ts";

const categories = [
  { category: Categories.SMILEYS_PEOPLE, name: "Smileys & people" },
  { category: Categories.ANIMALS_NATURE, name: "Animals & nature" },
  { category: Categories.FOOD_DRINK, name: "Food & drink" },
  { category: Categories.TRAVEL_PLACES, name: "Travel & places" },
  { category: Categories.ACTIVITIES, name: "Activities" },
  { category: Categories.OBJECTS, name: "Objects" },
  { category: Categories.SYMBOLS, name: "Symbols" },
  { category: Categories.FLAGS, name: "Flags" },
];

function preloadCategory(event: SyntheticEvent<HTMLDivElement>) {
  // The library exposes no category-intent callback. Capture events from its
  // built-in tabs, including their nested SVG icons, without selecting them.
  const tab = event.target instanceof Element ? event.target.closest(".epr-cat-btn") : null;
  const category = categories.find(({ category }) => tab?.classList.contains(`epr-icn-${category}`))?.category;
  if (category) void preloadEmojiImages(category);
}

export default function ReactionPicker({ onSelect }: { onSelect: (emoji: string) => void }) {
  // The picker already virtualizes the grid. Load its mounted window eagerly
  // so category jumps do not add a second, browser-controlled loading delay.
  return <div style={{ height: "100%" }} onPointerOverCapture={preloadCategory} onFocusCapture={preloadCategory}>
    <EmojiPicker theme={Theme.DARK} emojiStyle={EmojiStyle.TWITTER}
      emojiVersion="15.0" getEmojiUrl={emojiAsset} width="100%" height="100%"
      searchPlaceholder="Search emoji" autoFocusSearch lazyLoadEmojis={false} skinTonesDisabled
      previewConfig={{ showPreview: false }}
      categories={categories}
      onEmojiClick={(data) => onSelect(data.emoji)} />
  </div>;
}
