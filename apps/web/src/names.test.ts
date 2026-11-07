import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { displayNameError } from "./account/client.ts";
import { hasHiddenNameCharacters } from "./names.ts";
import { channelNameError, normalizeChannelName, spaceNameError } from "./spaces/client.ts";

// The same cases as apps/api/src/names.rs.
const allowed = [
  "Café Society",
  "José 🌱",
  "مرحبا بالعالم",
  "👨‍👩‍👧‍👦 family",
  "👩🏽‍💻",
  "❤️‍🔥",
  "🏳️‍🌈",
  "🏳️‍⚧️",
  "1️⃣",
  "☺︎ text style",
  "🏴\u{E0067}\u{E0062}\u{E0065}\u{E006E}\u{E0067}\u{E007F} England",
  "می‌خواهم",
  "क्‍ष",
  "葛\u{E0100}城",
  "Screenshot at 10.00.00 AM",
];
const rejected = [
  "a\u0000b",
  "admin‮nimda",
  "‪x‬",
  "⁦x⁩",
  "a​b",
  "‎left",
  "‏right",
  "؜x",
  "a⁠b",
  "﻿bom",
  "soft­hyphen",
  "a͏b",
  "ᅟ",
  "ㅤ",
  "ﾠ",
  "⠀",
  "a᠎b",
  "hidden\u{E0041}\u{E0042}",
  "️",
  "a ️",
  "a️️",
  "‍a",
  "a‍",
  "a‍‍b",
  "a ‍b",
  "a‍ b",
  "🏴\u{E0067}\u{E0062}",
  "🏴\u{E007F}",
  "x\u{E0067}\u{E007F}",
];

test("names refuse invisible and text-direction characters but keep emoji and joined scripts", () => {
  for (const name of allowed) assert.equal(hasHiddenNameCharacters(name), false, JSON.stringify(name));
  for (const name of rejected) assert.equal(hasHiddenNameCharacters(name), true, JSON.stringify(name));
});

test("space and display name errors explain the invisible-character rule", () => {
  assert.match(spaceNameError("Team‮maeT")!, /invisible or text-direction/);
  assert.equal(spaceNameError("Family 👨‍👩‍👧‍👦"), undefined);
  assert.match(displayNameError("Jo​se")!, /invisible or text-direction/);
  assert.equal(displayNameError(" José 🌱 "), undefined);
  assert.match(displayNameError("  ")!, /Enter a display name/);
  // A name saved before the rule stays savable while it is unchanged.
  assert.equal(displayNameError(" Legacy​Name ", "Legacy​Name"), undefined);
  assert.match(displayNameError("Other​Name", "Legacy​Name")!, /invisible/);
});

// Generated from these functions; every native client replays them through
// apps/native/client/src/names.rs.
test("space and channel names follow the shared cases", () => {
  const shared = JSON.parse(
    readFileSync(new URL("../../../shared/names/channel-cases.json", import.meta.url), "utf8"),
  ) as {
    normalize: { input: string; output: string }[];
    channelErrors: { name: string; error: string | null }[];
    spaceNames: { name: string; trimmed: string; error: string | null }[];
  };
  for (const { input, output } of shared.normalize) assert.equal(normalizeChannelName(input), output, input);
  for (const { name, error } of shared.channelErrors) assert.equal(channelNameError(name) ?? null, error, name);
  for (const { name, trimmed, error } of shared.spaceNames) {
    assert.equal(name.trim(), trimmed, name);
    assert.equal(spaceNameError(name) ?? null, error, name);
  }
});
