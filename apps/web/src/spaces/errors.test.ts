import assert from "node:assert/strict";
import { test } from "vitest";
import { SpacesApiError } from "./client.ts";
import { friendlyError } from "./errors.ts";

test("known server errors read as sentences", () => {
  assert.equal(
    friendlyError(new SpacesApiError(409, "channel name already exists")),
    "A channel with that name already exists.",
  );
  assert.equal(
    friendlyError(new SpacesApiError(409, "user must join the space first")),
    "This person needs to join the space before you can add them to a channel.",
  );
  assert.equal(friendlyError(new SpacesApiError(409, "user already in space")), "This person is already in the space.");
});

test("network failures and timeouts don't show the browser's text", () => {
  assert.equal(friendlyError(new TypeError("Failed to fetch")), "Couldn’t reach Caper. Check your connection.");
  assert.equal(friendlyError(new DOMException("signal timed out", "TimeoutError")), "That took too long. Try again.");
  assert.equal(friendlyError(new DOMException("aborted", "AbortError")), "That took too long. Try again.");
});

test("unknown text is capitalized and punctuated; readable text is kept", () => {
  assert.equal(
    friendlyError(new SpacesApiError(429, "too many things; try again later")),
    "Too many things. Try again later.",
  );
  assert.equal(
    friendlyError(new Error("This channel is no longer accessible.")),
    "This channel is no longer accessible.",
  );
  assert.equal(friendlyError("nope"), "That didn’t work. Try again.");
});
