import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { Window } from "happy-dom";
import { test } from "vitest";
import { formattedRuns } from "../chat/formatting.ts";
import { EditorState, TextSelection, type Transaction } from "prosemirror-state";
import { history, undo, redo } from "prosemirror-history";
import {
  composerDocument,
  composerSource,
  displayedOffset,
  formatComposer,
  normalizeComposer,
  sourceOffset,
} from "../chat/rich-text.ts";
import { linkSegments } from "../chat/links.ts";
import LinkedText from "../chat/LinkedText.tsx";
import { MessageText } from "../chat/MessageRow.tsx";

const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/formatting-cases.json", import.meta.url), "utf8"),
);

test("bold and italic follow the shared message contract", () => {
  for (const { text, runs } of shared.cases) {
    assert.deepEqual(formattedRuns(text), runs, text);
    const doc = composerDocument(text);
    assert.equal(doc.textContent, runs.map((run: { text: string }) => run.text).join(""));
    assert.deepEqual(formattedRuns(composerSource(doc)), runs, text);
  }
});

test("formatting never splits a URL, including a collapsed caret and surrounding selections", () => {
  const url = "https://youtu.be/NQXAC9IVRw?x=a_b&y=2";
  const text = `See ${url} now`;
  const end = 4 + url.length;
  for (const style of ["bold", "italic"] as const) {
    let state = EditorState.create({ doc: composerDocument(text) });
    const apply = (transaction: Transaction) => {
      state = state.apply(transaction);
    };
    for (const [start, stop] of [
      [8, 12],
      [0, 12],
      [8, text.length],
      [12, 12],
    ]) {
      state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, start, stop)));
      assert.equal(formatComposer(style)(state, apply), false);
      assert.equal(composerSource(state.doc), text);
      assert.deepEqual([state.selection.from, state.selection.to], [start, stop]);
    }
    state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 4, end)));
    assert.equal(formatComposer(style)(state, apply), true);
    const whole = composerSource(state.doc);
    assert.deepEqual(
      linkSegments(whole)
        .filter((part) => part.href)
        .map((part) => part.href),
      [url],
    );
    assert.equal(
      formattedRuns(whole)
        .map((run) => run.text)
        .join(""),
      text,
    );
  }
});

test("visual editing keeps formatted text, selection, undo, redo and wire text in agreement", () => {
  let state = EditorState.create({ doc: composerDocument("a 🙂word z"), plugins: [history()] });
  const apply = (transaction: Transaction) => {
    state = state.apply(normalizeComposer(transaction));
  };
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 2, 8)));
  assert.equal(formatComposer("bold")(state, apply), true);
  assert.equal(state.doc.textContent, "a 🙂word z");
  assert.equal(composerSource(state.doc), "a **🙂word** z");
  assert.deepEqual([state.selection.from, state.selection.to], [2, 8]);
  assert.equal(sourceOffset(composerSource(state.doc), 2), 4);
  assert.equal(displayedOffset(composerSource(state.doc), 10), 8);
  assert.equal(undo(state, apply), true);
  assert.equal(composerSource(state.doc), "a 🙂word z");
  assert.equal(redo(state, apply), true);
  assert.equal(composerSource(state.doc), "a **🙂word** z");
  apply(state.tr.insertText("new"));
  assert.equal(composerSource(state.doc), "a **new** z");
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 2, 5)));
  apply(state.tr.deleteSelection());
  assert.equal(composerSource(state.doc), "a  z", "deleting a styled selection leaves no orphan delimiters");
});

test("visual formatting protects URLs even when typing extends a styled link", () => {
  const url = "https://example.com/path";
  let state = EditorState.create({ doc: composerDocument(url) });
  const apply = (transaction: Transaction) => {
    state = state.apply(normalizeComposer(transaction));
  };
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 10, url.length)));
  assert.equal(formatComposer("bold")(state, apply), false);
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 0, url.length)));
  assert.equal(formatComposer("bold")(state, apply), true);
  assert.equal(composerSource(state.doc), `**${url}**`);
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, url.length)).setStoredMarks([]));
  apply(state.tr.insertText("?a=1"));
  assert.equal(state.doc.textContent, `${url}?a=1`);
  assert.deepEqual(
    linkSegments(composerSource(state.doc))
      .filter((part) => part.href)
      .map((part) => part.href),
    [`${url}?a=1`],
  );
});

test("Markdown shortcuts and trailing whitespace retain the visible caret", () => {
  let state = EditorState.create({ doc: composerDocument("before **bold*") });
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, state.doc.content.size)));
  state = state.apply(normalizeComposer(state.tr.insertText("*")));
  assert.equal(state.doc.textContent, "before bold");
  assert.equal(state.selection.from, 11);
  state = state.apply(normalizeComposer(state.tr.insertText(" ")));
  assert.equal(state.doc.textContent, "before bold ");
  assert.equal(state.selection.from, 12);
  assert.equal(composerSource(state.doc), "before **bold** ");
  state = state.apply(normalizeComposer(state.tr.insertText("again")));
  assert.equal(composerSource(state.doc), "before **bold** again", "a typed closing marker ends the style");
});

test("editing a style boundary never exposes generated Markdown as user text", () => {
  let state = EditorState.create({ doc: composerDocument("a _word_ **safe**") });
  state = state.apply(normalizeComposer(state.tr.delete(1, 2)));
  assert.equal(state.doc.textContent, "aword safe");
  assert.equal(composerSource(state.doc), "aword **safe**", "only unrepresentable emphasis is dropped");
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 1)));
  state = state.apply(normalizeComposer(state.tr.insertText("🙂")));
  assert.equal(state.doc.textContent, "a🙂word safe");
});

test("selection formatting toggles without losing adjacent text or Unicode", () => {
  for (const [source, start, end, style, expected] of [
    ["a 🙂word z", 2, 8, "bold", "a **🙂word** z"],
    ["a **🙂word** z", 2, 8, "bold", "a 🙂word z"],
    ["a _word_ z", 2, 6, "italic", "a word z"],
    ["a z", 2, 2, "italic", "a z"],
    ["a **word** z", 2, 6, "italic", "a **_word_** z"],
    [" word \n", 0, 7, "italic", " _word_ \n"],
    [" \t", 0, 2, "bold", " \t"],
  ] as const) {
    let state = EditorState.create({ doc: composerDocument(source) });
    state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, start, end)));
    formatComposer(style)(state, (transaction) => {
      state = state.apply(transaction);
    });
    assert.equal(composerSource(state.doc), expected);
    assert.deepEqual([state.selection.from, state.selection.to], [start, end]);
  }
});

test("sent emphasis composes with safe links and never interprets HTML", () => {
  const html = renderToStaticMarkup(
    <LinkedText text={"**bold _both_** _italics_ <img src=x onerror=alert(1)> **https://example.com/a_b_c**"} />,
  );
  const document = new Window().document;
  document.body.innerHTML = html;
  assert.equal(document.body.textContent, "bold both italics <img src=x onerror=alert(1)> https://example.com/a_b_c");
  assert.equal(document.querySelector("strong em")?.textContent, "both");
  assert.equal(document.querySelector("body > em")?.textContent, "italics");
  assert.equal(document.querySelector("img"), null);
  assert.equal(document.querySelector("strong a")?.getAttribute("href"), "https://example.com/a_b_c");
});

test("emphasis inside a mention keeps one profile control and cannot create a mention boundary", () => {
  const html = renderToStaticMarkup(
    <MessageText
      message={{
        id: "m",
        channelId: "c",
        seq: "1",
        createdAt: "",
        clientMessageId: "x",
        author: { id: "author", name: "Author", isGuest: false },
        content: {
          version: 1,
          type: "text",
          text: "@**alice** name**@alice**",
          mentions: [{ type: "user", id: "u", username: "alice" }],
        },
      }}
      mentionNames={new Map()}
      actions={{ openMention: () => {} }}
    />,
  );
  const document = new Window().document;
  document.body.innerHTML = html;
  assert.equal(document.body.textContent, "@alice name@alice");
  assert.equal(document.querySelectorAll("button.chat-mention").length, 1);
  assert.equal(document.querySelector("button.chat-mention")?.textContent, "@alice");
  assert.equal(document.querySelector("button.chat-mention strong")?.textContent, "alice");
});
