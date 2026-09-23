import assert from "node:assert/strict";
import { test } from "node:test";
import { playSliderTick, playSound } from "../audio/effects.ts";

test("effects bound overlap, tolerate playback rejection, and change slider pitch and loudness", async (t) => {
  const instances: FakeAudio[] = [];
  class FakeAudio extends EventTarget {
    volume = 1;
    playbackRate = 1;
    preservesPitch = true;
    paused = false;
    preload = "";
    readonly src: string;
    constructor(src: string) { super(); this.src = src; instances.push(this); }
    play() { return Promise.resolve(); }
    pause() { this.paused = true; }
  }
  const original = Object.getOwnPropertyDescriptor(globalThis, "Audio");
  Object.defineProperty(globalThis, "Audio", { configurable: true, value: FakeAudio });
  t.after(() => {
    instances.forEach((audio) => audio.dispatchEvent(new Event("ended")));
    if (original) Object.defineProperty(globalThis, "Audio", original);
    else Reflect.deleteProperty(globalThis, "Audio");
  });
  let now = 1000;
  t.mock.method(performance, "now", () => now);
  playSliderTick(0);
  assert.equal(instances[0].src, "/audio/effects/slider-tick.wav");
  assert.equal(instances[0].preservesPitch, false, "playbackRate must actually change pitch");
  assert.equal(instances[0].playbackRate, 0.75);
  assert.equal(instances[0].volume, 0.1);
  now += 39;
  playSliderTick(1);
  assert.equal(instances.length, 1);
  now++;
  playSliderTick(1);
  assert.equal(instances[1].playbackRate, 1.35);
  assert.equal(instances[1].volume, 0.32);
  for (let i = 0; i < 4; i++) playSound("warning");
  assert.equal(instances.filter((audio) => !audio.paused).length, 4);
  instances.forEach((audio) => audio.dispatchEvent(new Event("ended")));
  t.mock.method(FakeAudio.prototype, "play", () => Promise.reject(new Error("Autoplay denied")));
  playSound("channel-join");
  await Promise.resolve();
  assert.equal(instances.at(-1)?.src, "/audio/effects/channel-join.wav");
});
