import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { gifLabel, gifOf, klipyMediaUrl, parseKlipy, type KlipyKind } from "./klipy.ts";

// Every client runs these same cases; see shared/klipy/cases.json.
const shared = JSON.parse(readFileSync(new URL("../../../../shared/klipy/cases.json", import.meta.url), "utf8")) as {
  parse: { kind: KlipyKind; response: unknown; items: unknown[]; hasNext: boolean };
  parseSticker: { kind: KlipyKind; response: unknown; items: unknown[]; hasNext: boolean };
  messages: { valid: unknown[]; invalid: unknown[] };
  urls: { url: string; valid: boolean }[];
};

// JSON has no undefined; the shared cases spell an absent ad size as null.
const plain = (value: unknown) => JSON.parse(JSON.stringify(value)) as unknown;

test("shared KLIPY responses parse in order", () => {
  for (const { kind, response, items, hasNext } of [shared.parse, shared.parseSticker]) {
    const page = parseKlipy(response, kind);
    assert.deepEqual(plain(page.items), items);
    assert.equal(page.hasNext, hasNext);
  }
  assert.deepEqual(parseKlipy(null, "gif"), { items: [], hasNext: false });
});

test("shared GIF message rules", () => {
  for (const gif of shared.messages.valid) assert.deepEqual(gifOf({ gif }), gif);
  for (const gif of shared.messages.invalid) assert.equal(gifOf({ gif }), undefined, JSON.stringify(gif));
  assert.equal(gifOf({}), undefined);
});

test("shared KLIPY media URL rules", () => {
  for (const { url, valid } of shared.urls) assert.equal(klipyMediaUrl(url), valid, url);
});

test("labels fall back to the kind", () => {
  assert.equal(gifLabel({ kind: "gif", title: "" }), "GIF");
  assert.equal(gifLabel({ kind: "sticker", title: "" }), "Sticker");
  assert.equal(gifLabel({ kind: "gif", title: "Hello" }), "Hello");
});
