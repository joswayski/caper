import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { Window } from "happy-dom";
import { test } from "vitest";
import { formatStyles, formattedRuns, type FormattedRun } from "../chat/formatting.ts";
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
import { detectLanguage, highlightCode, languageFromTag, parseMessageBlocks } from "../chat/code.ts";
import { linkSegments } from "../chat/links.ts";
import LinkedText from "../chat/LinkedText.tsx";
import { MessageText } from "../chat/MessageRow.tsx";

const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/formatting-cases.json", import.meta.url), "utf8"),
);
const sharedCode = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/code-cases.json", import.meta.url), "utf8"),
);

// Fixture runs list the styles they have; the others are false.
const fixtureRuns = (runs: ({ text: string } & Partial<FormattedRun>)[]) =>
  runs.map((run) => ({ ...Object.fromEntries(formatStyles.map((style) => [style, false])), ...run }) as FormattedRun);

test("formatting follows the shared message contract", () => {
  for (const { text, runs: listed } of shared.cases) {
    const runs = fixtureRuns(listed);
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
  assert.equal(composerSource(state.doc), "a*word* **safe**", "mid-word italics switch cleanly to * delimiters");
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
    ["support them all", 4, 16, "italic", "supp*ort them all*"],
    ["support them all", 4, 16, "underline", "supp__ort them all__"],
    ["run echo hi", 4, 11, "code", "run `echo hi`"],
    ["run `echo hi`", 4, 11, "code", "run echo hi"],
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

test("code block toggle wraps and unwraps selections cleanly", () => {
  let state = EditorState.create({ doc: composerDocument("let x = 1;") });
  state = state.apply(state.tr.setSelection(TextSelection.create(state.doc, 0, 10)));
  assert.equal(
    formatComposer("codeBlock")(state, (tr) => {
      state = state.apply(tr);
    }),
    true,
  );
  assert.equal(composerSource(state.doc), "```\nlet x = 1;\n```");
  assert.equal(
    formatComposer("codeBlock")(state, (tr) => {
      state = state.apply(tr);
    }),
    true,
  );
  assert.equal(composerSource(state.doc), "let x = 1;");
});

test("code block parsing, language detection, and syntax highlighting match shared cases", () => {
  for (const c of sharedCode.blocks) {
    const parsed = parseMessageBlocks(c.message);
    assert.equal(parsed.length, c.blocks.length, c.message);
    parsed.forEach((actual, index) => {
      const expected = c.blocks[index];
      assert.equal(actual.kind, expected.kind, c.message);
      if (actual.kind === "prose") {
        assert.equal(actual.text, expected.text, c.message);
      } else {
        assert.equal(actual.code, expected.text, c.message);
        if (expected.language !== undefined) assert.equal(actual.language, expected.language, c.message);
        if (expected.detected !== undefined) assert.equal(actual.detected, expected.detected, c.message);
      }
    });
  }
  for (const c of sharedCode.detect) {
    assert.equal(detectLanguage(c.code), c.language, c.code);
  }
  for (const c of sharedCode.highlight) {
    const lang = languageFromTag(c.language) ?? "plain";
    const spans = highlightCode(c.code, lang);
    let pos = 0;
    for (const span of spans) {
      assert.equal(span.start, pos);
      assert.ok(span.end > span.start);
      pos = span.end;
    }
    assert.equal(pos, c.code.length);
    for (const expected of c.must_include) {
      assert.ok(
        spans.some((s) => c.code.slice(s.start, s.end) === expected.text && s.kind === expected.kind),
        `missing ${JSON.stringify(expected)}`,
      );
    }
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

test("inline code and fenced code blocks suppress links and mentions and highlight syntax", () => {
  const html = renderToStaticMarkup(
    <MessageText
      message={{
        id: "m-code",
        channelId: "c",
        seq: "2",
        createdAt: "",
        clientMessageId: "y",
        author: { id: "author", name: "Author", isGuest: false },
        content: {
          version: 1,
          type: "text",
          text: "Use `https://example.com @alice`:\n```rust\npub fn hi() {}\n```",
          mentions: [{ type: "user", id: "u", username: "alice" }],
        },
      }}
      mentionNames={new Map()}
      actions={{ openMention: () => {} }}
    />,
  );
  const document = new Window().document;
  document.body.innerHTML = html;
  assert.equal(document.querySelector("code.chat-inline-code")?.textContent, "https://example.com @alice");
  assert.equal(document.querySelector("a"), null);
  assert.equal(document.querySelector("button.chat-mention"), null);
  assert.equal(document.querySelector(".chat-code-lang")?.textContent, "Rust");
  assert.equal(document.querySelector(".chat-code-block .tok-keyword")?.textContent, "pub");
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
