import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { EditorState, TextSelection } from "prosemirror-state";
import { composerDocument, composerSource, normalizeComposer } from "../chat/rich-text.ts";

// The native composers type these same steps through shared/messages/composer.rs.
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/composer-cases.json", import.meta.url), "utf8"),
);

function place(source: string, caret: number) {
  const state = EditorState.create({ doc: composerDocument(source) });
  return state.apply(state.tr.setSelection(TextSelection.create(state.doc, caret)));
}

test("composer typing follows the shared cases", () => {
  for (const { note, text, caret, steps } of shared.cases) {
    let state = place(text, caret);
    for (const step of steps) {
      if (step.placed) state = place(composerSource(state.doc), state.selection.head);
      state = state.apply(normalizeComposer(state.tr.insertText(step.key)));
      const pending = state.storedMarks
        ? state.storedMarks.reduce((bits, mark) => bits | (mark.type.name === "bold" ? 1 : 2), 0)
        : -1;
      assert.deepEqual(
        { text: composerSource(state.doc), caret: state.selection.head, pending },
        { text: step.text, caret: step.caret, pending: step.pending },
        `${note}: ${JSON.stringify(step.key)}`,
      );
    }
  }
});

test("replacing a styled draft preserves newly typed literal markers", () => {
  for (const literal of ["*", "**", "***", "**x", "_", "🙂*", "e\u0301 👩‍💻 *"]) {
    let state = place("**bold** a", 0);
    state = state.apply(
      normalizeComposer(
        state.tr.setSelection(TextSelection.create(state.doc, 0, state.doc.content.size)).insertText(literal),
      ),
    );
    // The replacement may inherit bold, but none of its literal text is lost.
    assert.equal(state.doc.textContent, literal);
    assert.equal(composerDocument(composerSource(state.doc)).textContent, literal);
    assert.equal(state.selection.head, literal.length);
  }
});
