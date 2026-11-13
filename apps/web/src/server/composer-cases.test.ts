import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { EditorState, TextSelection } from "prosemirror-state";
import { composerDocument, composerSource, formatComposer, markBits, normalizeComposer } from "../chat/rich-text.ts";

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
      const pending = state.storedMarks ? markBits(state.storedMarks) : -1;
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

test("toolbar toggles follow the shared cases", () => {
  const toggles = JSON.parse(
    readFileSync(new URL("../../../../shared/messages/toggle-cases.json", import.meta.url), "utf8"),
  );
  assert.ok(toggles.cases.length > 300);
  for (const { text, from, to, style, result } of toggles.cases) {
    let state = EditorState.create({ doc: composerDocument(text) });
    state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, from, to)));
    const applied = formatComposer(style)(state, (transaction) => {
      state = state.apply(transaction);
    });
    const label = JSON.stringify({ text, from, to, style });
    assert.equal(applied ? composerSource(state.doc) : null, result, label);
    assert.deepEqual([state.selection.from, state.selection.to], [from, to], label);
  }
});
