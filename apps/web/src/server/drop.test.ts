import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { carriesFiles, dropText } from "../chat/drop.ts";

// Every client runs these same cases; see shared/messages/drop-cases.json.
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/drop-cases.json", import.meta.url), "utf8"),
) as { cases: { note: string; channel: string; direct: boolean; attached: number; title: string; detail: string }[] };

test("shared drop overlay text matches", () => {
  assert.ok(shared.cases.length >= 5);
  for (const { note, channel, direct, attached, title, detail } of shared.cases) {
    assert.deepEqual(dropText(channel, direct, attached), { title, detail }, note);
  }
});

test("only file drags open the overlay", () => {
  const transfer = (types: string[]) => ({ types }) as unknown as DataTransfer;
  assert.equal(carriesFiles(transfer(["Files"])), true);
  assert.equal(carriesFiles(transfer(["text/plain", "Files"])), true);
  assert.equal(carriesFiles(transfer(["text/plain", "text/uri-list"])), false);
  assert.equal(carriesFiles(null), false);
});
