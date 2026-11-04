import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "vitest";
import { carriesFiles, dropPlan, dropPose, dropRandom, dropText, type DropPose } from "../chat/drop.ts";

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

const motion = JSON.parse(
  readFileSync(new URL("../../../../shared/messages/drop-motion-cases.json", import.meta.url), "utf8"),
) as {
  random: { seed: number; values: number[] };
  seed: number;
  cases: ({ elapsed: number; still: boolean } & DropPose)[];
};

test("shared drop motion matches", () => {
  const random = dropRandom(motion.random.seed);
  for (const value of motion.random.values) assert.ok(Math.abs(random() - value) < 1e-6);
  const plan = dropPlan(motion.seed);
  const close = (actual: unknown, expected: unknown, path: string): void => {
    if (typeof expected === "number") assert.ok(Math.abs((actual as number) - expected) < 1e-4, path);
    else
      for (const [key, value] of Object.entries(expected as object))
        close((actual as never)[key], value, `${path}.${key}`);
  };
  for (const { elapsed, still, ...pose } of motion.cases) close(dropPose(plan, elapsed, still), pose, `${elapsed}`);
});

test("drop motion is smooth and differs per open", () => {
  // At 120 frames a second, nothing jumps: the drift and hops move in small steps.
  const plan = dropPlan(7);
  let previous = dropPose(plan, 0);
  for (let frame = 1; frame <= 120 * 30; frame++) {
    const pose = dropPose(plan, frame / 120);
    pose.cards.forEach((card, index) => {
      assert.ok(Math.abs(card.x - previous.cards[index].x) < 1.5 && Math.abs(card.y - previous.cards[index].y) < 1.5);
    });
    assert.ok(Math.abs(pose.character.y - previous.character.y) < 2);
    assert.ok(Math.abs(pose.character.scaleY - previous.character.scaleY) < 0.05);
    previous = pose;
  }
  // Hops vary in height and timing; another seed rests and hops differently.
  const gaps = plan.hops.slice(1).map((hop, index) => hop.start - plan.hops[index].start);
  assert.ok(new Set(plan.hops.map((hop) => hop.height.toFixed(2))).size > 20);
  assert.ok(Math.max(...gaps) - Math.min(...gaps) > 1);
  assert.notDeepEqual(dropPose(dropPlan(8), 0, true), dropPose(plan, 0, true));
});
