import assert from "node:assert/strict";
import test from "node:test";
import { applyBeat, cast, initialPreview, nextSpeakers, previewBeats } from "../components/livePreview.ts";

const ids = (people: { id: string }[]) => people.map((person) => person.id).sort();

test("one pass of the preview script returns voice and typing to the opening state", () => {
  const start = initialPreview();
  let state = start;
  for (let i = 0; i < previewBeats.length; i++) state = applyBeat(state);
  assert.equal(state.step, 0, "the script loops");
  assert.deepEqual(ids(state.voice), ids(start.voice), "voice membership is stable across loops");
  assert.deepEqual(state.typing, [], "every typing indicator ends in a message");
  assert.ok(state.messages.length <= 16, "the message list is bounded");
  const keys = state.messages.map((message) => message.key);
  assert.equal(new Set(keys).size, keys.length, "message keys stay unique");
});

test("a message clears its author's typing indicator and advances the clock", () => {
  const typing = applyBeat(initialPreview(), { wait: 0, type: "typing", who: cast.maya, on: true });
  assert.deepEqual(ids(typing.typing), [cast.maya.id]);
  const sent = applyBeat(typing, { wait: 0, type: "message", who: cast.maya, text: "Hi" });
  assert.deepEqual(sent.typing, []);
  assert.equal(sent.messages.at(-1)?.text, "Hi");
  assert.notEqual(sent.messages.at(-1)?.time, typing.messages.at(-1)?.time);
});

test("the preview depicts only shipped features", () => {
  assert.ok(previewBeats.every((beat) => ["typing", "message", "voice"].includes(beat.type)));
});

test("speakers are chosen only from people in voice", () => {
  const voice = [cast.alex, cast.sam];
  let previous: ReadonlySet<string> = new Set();
  for (let i = 0; i < 200; i++) {
    const turn = nextSpeakers(voice, previous);
    for (const id of turn.speaking) assert.ok(voice.some((person) => person.id === id));
    assert.ok(turn.duration > 0);
    previous = turn.speaking;
  }
  assert.equal(nextSpeakers([], new Set()).speaking.size, 0);
});
