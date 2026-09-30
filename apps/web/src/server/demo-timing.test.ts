import assert from "node:assert/strict";
import test from "node:test";
import { createDemoTiming } from "../components/demoTiming.ts";

test("midpoint fixture preserves speech windows and message order with delayed reactions", () => {
  const timing = createDemoTiming([[[1.5, 5.5], [8, 10.75]], [[2.75, 6.25]]], [0, 1.25, 3, 4.75], () => .5);
  assert.deepEqual(timing.speech, [[[1.5, 5.5], [8, 10.75]], [[2.75, 6.25]]]);
  assert.deepEqual(timing.messages.map((message) => message.at), [0, 1.25, 3, 4.75]);
  assert.deepEqual(timing.messages[3], { at: 4.75, reactions: [7, 9, 11, 13], typing: 1.25 });
});

test("extreme jitter preserves short speech, reply bursts, and positive reader delays", () => {
  for (const draws of [[0], [1], [1, 0]]) {
    let cursor = 0;
    const timing = createDemoTiming([[[0, 4.25], [15.75, 16.5]]], [0, 1.25, 3, 11.25, 11.75, 12.5], () => draws[cursor++ % draws.length]);
    for (const [start, end] of timing.speech[0]) {
      assert.ok(start >= 0);
      assert.ok(end - start >= .6 - 1e-12, "even the shortest speaking window remains visible");
    }
    assert.deepEqual(timing.messages.slice(0, 3).map((message) => message.at), [0, 1.25, 3], "no hydration flicker in introductory messages");
    for (const [index, message] of timing.messages.entries()) {
      if (index > 0) assert.ok(message.at > timing.messages[index - 1].at, "replies cannot overtake their preceding message");
      const firstDelay = message.reactions[0] - message.at;
      assert.ok(firstDelay >= 1.5 - 1e-12 && firstDelay <= 3 + 1e-12);
      assert.equal(message.reactions.filter((at) => at <= message.at).length, 0);
      assert.ok(message.typing < message.at);
      for (let reaction = 1; reaction < message.reactions.length; reaction++) {
        assert.ok(message.reactions[reaction] > message.reactions[reaction - 1], "later readers react separately");
      }
    }
    assert.ok(timing.messages[4].at - timing.messages[3].at < 1, "the burst survives random offsets");
  }
});

test("speech and reader timing use independent draws instead of one shared offset", () => {
  const sample = (draws: number[]) => {
    let cursor = 0;
    return createDemoTiming([[[2, 4]]], [0], () => draws[cursor++]);
  };
  const first = sample([.2, .8, .1, .9, .3, .7, .4]);
  const otherSpeech = sample([.9, .1, .1, .9, .3, .7, .4]);
  const otherReaders = sample([.2, .8, .8, .1, .9, .2, .6]);
  assert.notDeepEqual(first.speech, otherSpeech.speech);
  assert.deepEqual(first.messages, otherSpeech.messages);
  assert.deepEqual(first.speech, otherReaders.speech);
  assert.notDeepEqual(first.messages, otherReaders.messages);
  const gaps = first.messages[0].reactions.map((at, index, all) => at - (index === 0 ? 0 : all[index - 1]));
  assert.equal(new Set(gaps).size, 4, "reactions do not grow on a regular beat");
});
