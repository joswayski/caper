import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { emojiToken, emojiSuggestions, insertEmoji, type EmojiChoice } from "../chat/emoji-autocomplete.ts";

const catalog: EmojiChoice[] = JSON.parse(
  readFileSync(new URL("../../../../shared/emoji/catalog.json", import.meta.url), "utf8"),
).filter((entry: { selectable: boolean }) => entry.selectable);

test("colon autocomplete respects boundaries, selections and complete tokens", () => {
  for (const text of [":", ":tom", "hello (:tom", "line\n:+1", ":thumbs_up", ":thumbs-up"]) {
    assert.ok(emojiToken(text, text.length), text);
  }
  for (const text of ["word:tom", "12:30", "https://tom", ":tom:", ":two words"]) {
    assert.equal(emojiToken(text, text.length), undefined, text);
  }
  assert.equal(emojiToken(":tomato", 4), undefined);
  assert.equal(emojiToken(":tom:", 4), undefined);
  assert.equal(emojiToken(":tom", 0, 4), undefined);
  assert.deepEqual(emojiToken("ok :tom suffix", 7), { start: 3, end: 7, query: "tom" });
  for (const text of [":D", "ok :P", "hi (:3", ":o"]) {
    assert.equal(emojiToken(text, text.length), undefined, text);
  }
  assert.deepEqual(emojiToken(":sm", 3), { start: 0, end: 3, query: "sm" });
});

test("catalog search matches names, aliases and stable ranked results", () => {
  assert.ok(emojiSuggestions(catalog, "tom").some((entry) => entry.emoji === "🍅"));
  assert.equal(emojiSuggestions(catalog, "tomato")[0].emoji, "🍅");
  assert.equal(emojiSuggestions(catalog, "thumbs_up")[0].emoji, "👍");
  assert.equal(emojiSuggestions(catalog, "+1")[0].emoji, "👍");
  assert.equal(emojiSuggestions(catalog, "WOMAN-TECHNOLOGIST")[0].emoji, "👩‍💻");
  assert.equal(
    emojiSuggestions(catalog, "red_heart")[0].emoji,
    "❤️",
    "Insertion must retain emoji presentation selectors",
  );
  assert.deepEqual(
    emojiSuggestions(catalog, "").map((entry) => entry.id),
    ["1f44d", "1f600", "2764", "1f389", "1f680", "1f440"],
  );
  assert.equal(emojiSuggestions(catalog, "face").length, 6);
  assert.deepEqual(emojiSuggestions(catalog, "notanemojiname"), []);
  const entries = [
    { id: "substring", emoji: "a", name: "pocket rock et", keywords: "" },
    { id: "keyword", emoji: "b", name: "stone", keywords: "space rock et" },
    { id: "prefix", emoji: "c", name: "rock et fuel", keywords: "" },
    { id: "exact", emoji: "d", name: "rock et", keywords: "" },
  ];
  assert.deepEqual(
    emojiSuggestions(entries, "rock_et").map((entry) => entry.id),
    ["exact", "prefix", "keyword", "substring"],
  );
});

test("country flags use clean names in suggestions and insert the original Unicode", () => {
  for (const [query, emoji, name] of [
    ["ISRAEL", "🇮🇱", "israel"],
    ["IL", "🇮🇱", "israel"],
    ["united-states", "🇺🇸", "united-states"],
    ["united_states", "🇺🇸", "united-states"],
    ["bosnia-and-herzegovina", "🇧🇦", "bosnia-and-herzegovina"],
    ["cote-divoire", "🇨🇮", "cote-divoire"],
    ["turkiye", "🇹🇷", "turkiye"],
  ]) {
    const choice = emojiSuggestions(catalog, query).find((entry) => entry.emoji === emoji)!;
    assert.ok(choice, query);
    assert.equal(choice.name, name);
    const text = `flag :${query}`;
    assert.deepEqual(insertEmoji(text, emojiToken(text, text.length)!, choice.emoji), {
      value: `flag ${emoji}`,
      caret: `flag ${emoji}`.length,
    });
  }
});

test("insertion preserves Unicode prefix/suffix and enforces scalar boundary", () => {
  const text = "👩‍💻 hi :roc suffix 🚀";
  const caret = "👩‍💻 hi :roc".length;
  assert.deepEqual(insertEmoji(text, emojiToken(text, caret)!, "🚀"), {
    value: "👩‍💻 hi 🚀 suffix 🚀",
    caret: "👩‍💻 hi 🚀".length,
  });
  const nearLimit = "😀".repeat(3998) + " :xy";
  assert.ok(insertEmoji(nearLimit, emojiToken(nearLimit, nearLimit.length)!, "🚀"));
  assert.equal(insertEmoji(nearLimit, emojiToken(nearLimit, nearLimit.length)!, "👩‍💻"), undefined);
});
