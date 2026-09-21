import assert from "node:assert/strict";
import test from "node:test";
import { hasVoiceActivity } from "../media/voice-activity.ts";

function signal(rms: number) {
  return Float32Array.from({ length: 256 }, (_, index) => index % 2 ? rms : -rms);
}

test("quiet audible speech activates the speaking indicator", () => {
  assert.equal(hasVoiceActivity(signal(0.006)), true);
});

test("suppressed background noise does not activate the speaking indicator", () => {
  assert.equal(hasVoiceActivity(signal(0.002)), false);
});
