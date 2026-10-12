import assert from "node:assert/strict";
import { test } from "vitest";
import { FLING, clampProgress, settlesForward, Velocity } from "../components/swipe.ts";

test("release speed covers only the last 100ms of movement", () => {
  const speed = new Velocity();
  speed.add(0, 0);
  speed.add(50, 10);
  speed.add(100, 60);
  speed.add(150, 160);
  // 50ms → 150ms: 150px in 100ms.
  assert.equal(speed.speed(150), 1.5);
  // A finger that stops before lifting is not a fling.
  assert.equal(speed.speed(400), 0);
  // Nor is a single sample, or two too close together to measure.
  const once = new Velocity();
  once.add(10, 50);
  assert.equal(once.speed(10), 0);
  once.add(14, 80);
  assert.equal(once.speed(14), 0);
});

test("release speed is signed, so a drag thrown back reads as negative", () => {
  const speed = new Velocity();
  speed.add(0, 200);
  speed.add(20, 190);
  speed.add(40, 160);
  assert.ok(speed.speed(40) < -FLING);
});

test("a fling goes the way it is thrown; otherwise the nearer end wins", () => {
  // Held and lifted: nearer end.
  assert.equal(settlesForward(0.4, 0, 150), false);
  assert.equal(settlesForward(0.6, 0, 230), true);
  // Thrown forward from early on.
  assert.equal(settlesForward(0.2, 0.8, 80), true);
  // Thrown back after passing halfway.
  assert.equal(settlesForward(0.7, -0.8, 270), false);
  // A twitch while lifting is not a fling.
  assert.equal(settlesForward(0.05, 1.2, 20), false);
  assert.equal(settlesForward(0.95, -1.2, -20), true);
  // A caught slide moving back toward its start (negative travel) still flings.
  assert.equal(settlesForward(0.5, -0.6, -60), false);
});

test("progress stays between the ends", () => {
  assert.equal(clampProgress(-0.2), 0);
  assert.equal(clampProgress(1.4), 1);
  assert.equal(clampProgress(0.25), 0.25);
});
