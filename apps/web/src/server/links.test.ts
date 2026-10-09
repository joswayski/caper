import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { linkSegments } from "../chat/links.ts";

// Every client runs these same cases; see shared/messages/link-cases.json.
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/link-cases.json", import.meta.url), "utf8"),
) as { cases: { note: string; text: string; segments: { text: string; href?: string }[] }[] };

test("shared link cases match", () => {
  assert.ok(shared.cases.length > 30);
  for (const { note, text, segments } of shared.cases) {
    const actual = linkSegments(text).map((segment) =>
      segment.href ? { text: segment.text, href: segment.href } : { text: segment.text },
    );
    assert.deepEqual(actual, segments, note);
  }
});

test("segments always rebuild the original text and only link http(s)", () => {
  for (const text of ["a https://x.io b", "javascript:alert(1) www.ok.com", "(https://a.b/c))", "", "www.a.b."]) {
    const segments = linkSegments(text);
    assert.equal(segments.map((segment) => segment.text).join(""), text);
    for (const segment of segments) if (segment.href) assert.match(segment.href, /^https?:\/\//);
  }
});
