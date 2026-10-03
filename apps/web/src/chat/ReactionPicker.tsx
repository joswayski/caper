import EmojiPicker, { Categories, EmojiStyle, Theme } from "emoji-picker-react";
import { emojiAsset } from "./emoji.ts";

export default function ReactionPicker({ onSelect }: { onSelect: (emoji: string) => void }) {
  return <EmojiPicker theme={Theme.DARK} emojiStyle={EmojiStyle.TWITTER}
    emojiVersion="15.0" getEmojiUrl={emojiAsset} width="100%" height="100%"
    searchPlaceholder="Search emoji" autoFocusSearch lazyLoadEmojis skinTonesDisabled
    previewConfig={{ showPreview: false }}
    categories={[
      { category: Categories.SMILEYS_PEOPLE, name: "Smileys & people" },
      { category: Categories.ANIMALS_NATURE, name: "Animals & nature" },
      { category: Categories.FOOD_DRINK, name: "Food & drink" },
      { category: Categories.TRAVEL_PLACES, name: "Travel & places" },
      { category: Categories.ACTIVITIES, name: "Activities" },
      { category: Categories.OBJECTS, name: "Objects" },
      { category: Categories.SYMBOLS, name: "Symbols" },
      { category: Categories.FLAGS, name: "Flags" },
    ]}
    onEmojiClick={(data) => onSelect(data.emoji)} />;
}
