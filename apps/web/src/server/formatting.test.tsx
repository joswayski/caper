import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { renderToStaticMarkup } from "react-dom/server";
import { Window } from "happy-dom";
import { test } from "vitest";
import { formattedRuns, toggleFormat } from "../chat/formatting.ts";
import LinkedText from "../chat/LinkedText.tsx";
import { MessageText } from "../chat/MessageRow.tsx";

const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/formatting-cases.json", import.meta.url), "utf8"),
);

test("bold and italic follow the shared message contract", () => {
  for (const { text, runs } of shared.cases) assert.deepEqual(formattedRuns(text), runs, text);
});

test("selection formatting toggles without losing adjacent text or Unicode", () => {
  assert.deepEqual(toggleFormat("a 🙂word z", 2, 8, "**"), { text: "a **🙂word** z", start: 4, end: 10 });
  assert.deepEqual(toggleFormat("a **🙂word** z", 4, 10, "**"), { text: "a 🙂word z", start: 2, end: 8 });
  assert.deepEqual(toggleFormat("a _word_ z", 2, 8, "_"), { text: "a word z", start: 2, end: 6 });
  assert.deepEqual(toggleFormat("a z", 2, 2, "_"), { text: "a __z", start: 3, end: 3 });
  assert.deepEqual(toggleFormat("a **word** z", 4, 8, "_"), { text: "a **_word_** z", start: 5, end: 9 });
  assert.deepEqual(toggleFormat(" word \n", 0, 7, "_"), { text: " _word_ \n", start: 2, end: 6 });
  assert.deepEqual(toggleFormat(" \t", 0, 2, "**").text, " \t");
  assert.deepEqual(toggleFormat("**bold** plus normal", 0, 20, "**"), {
    text: "**bold plus normal**",
    start: 2,
    end: 18,
  });
  assert.deepEqual(toggleFormat("**one** and **two**", 0, 19, "**").text, "**one and two**");
  assert.deepEqual(toggleFormat("**bold _both_**", 0, 15, "_").text, "_**bold both**_");
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
