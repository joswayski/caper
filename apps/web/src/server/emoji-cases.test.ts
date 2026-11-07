import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { emojiSuggestions, emojiToken, type EmojiChoice } from "../chat/emoji-autocomplete.ts";
import { formattedMessage } from "../chat/formatting.ts";

// The native composers replay these same cases through shared/messages/emoji.rs.
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/emoji-cases.json", import.meta.url), "utf8"),
);
const catalog: EmojiChoice[] = JSON.parse(
  readFileSync(new URL("../../../../shared/emoji/catalog.json", import.meta.url), "utf8"),
).filter((entry: { selectable: boolean }) => entry.selectable);

test("emoji autocomplete follows the shared cases", () => {
  assert.ok(shared.tokens.length > 400);
  for (const { text, caret, token } of shared.tokens) {
    // As ComposerSuggestions maps the source caret through the visible text and back.
    const projection = formattedMessage(text);
    const visible = projection.runs.map((run) => run.text).join("");
    const at = projection.sourceIndices.filter((index) => index < caret).length;
    const found = emojiToken(visible, at, at);
    const source = found && {
      start: projection.sourceIndices[found.start],
      end: projection.sourceIndices[found.end - 1] + 1,
      query: found.query,
    };
    assert.deepEqual(source ?? null, token, `${JSON.stringify(text)} at ${caret}`);
  }
  for (const { query, ids } of shared.suggestions) {
    assert.deepEqual(
      emojiSuggestions(catalog, query).map((entry) => entry.id),
      ids,
      query,
    );
  }
});
