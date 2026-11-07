import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { test } from "vitest";
import { cleanLastNewline, parseDiffFromFile } from "@pierre/diffs";

// MessageDiff's word highlights (lineDiffType "word"), using @pierre/diffs' own jsdiff,
// so a library upgrade that changes them fails here before the native clients diverge.
const { diffWordsWithSpace } = createRequire(import.meta.resolve("@pierre/diffs"))("diff") as {
  diffWordsWithSpace: (a: string, b: string) => { value: string; added: boolean; removed: boolean }[];
};
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/history-diff-cases.json", import.meta.url), "utf8"),
);

function highlights(before: string, after: string) {
  // One flag per UTF-16 unit, as the highlighter's spans count.
  const old = Array.from({ length: before.length }, () => false);
  const next = Array.from({ length: after.length }, () => false);
  if (before !== after) {
    const diff = parseDiffFromFile({ name: "message.txt", contents: before }, { name: "message.txt", contents: after });
    const starts = (lines: string[]) => lines.map((_, index) => lines.slice(0, index).join("").length);
    const [deleted, added] = [starts(diff.deletionLines), starts(diff.additionLines)];
    const mark = (flags: boolean[], at: number, length: number) => flags.fill(true, at, at + length);
    for (const hunk of diff.hunks)
      for (const block of hunk.hunkContent) {
        if (block.type !== "change") continue;
        for (let line = 0; line < Math.max(block.deletions, block.additions); line++) {
          const d = line < block.deletions ? block.deletionLineIndex + line : undefined;
          const a = line < block.additions ? block.additionLineIndex + line : undefined;
          if (d === undefined) mark(next, added[a!], diff.additionLines[a!].length);
          else if (a === undefined) mark(old, deleted[d], diff.deletionLines[d].length);
          else {
            let [o, n] = [deleted[d], added[a]];
            for (const item of diffWordsWithSpace(
              cleanLastNewline(diff.deletionLines[d]),
              cleanLastNewline(diff.additionLines[a]),
            )) {
              if (!item.added) {
                if (item.removed) mark(old, o, item.value.length);
                o += item.value.length;
              }
              if (!item.removed) {
                if (item.added) mark(next, n, item.value.length);
                n += item.value.length;
              }
            }
          }
        }
      }
  }
  const runs = (text: string, flags: boolean[]) => {
    const out: [string, boolean][] = [];
    for (let start = 0, end = 0; start < text.length; start = end) {
      while (end < text.length && flags[end] === flags[start]) end++;
      out.push([text.slice(start, end), flags[start]]);
    }
    return out;
  };
  return { old: runs(before, old), new: runs(after, next) };
}

test("edit history highlights follow the shared contract", () => {
  for (const { note, before, after, ...expected } of shared.cases) {
    assert.deepEqual(highlights(before, after), expected, note);
  }
});
