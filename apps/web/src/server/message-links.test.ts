import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { messageLink, messageLinkUrl } from "../chat/message-links.ts";

// Every client runs these same cases; see shared/messages/message-link-cases.json.
const shared = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/message-link-cases.json", import.meta.url), "utf8"),
) as { cases: { input: string; link: { channel: string; message: string } | null }[] };

test("shared message link cases match", () => {
  assert.ok(shared.cases.length > 10);
  for (const { input, link } of shared.cases) assert.deepEqual(messageLink(input) ?? null, link, input);
  const url = messageLinkUrl("Chan12345678", "Msg123456789012");
  assert.equal(url, "https://caper.chat/m/Chan12345678/Msg123456789012");
  assert.deepEqual(messageLink(url), { channel: "Chan12345678", message: "Msg123456789012" });
});
