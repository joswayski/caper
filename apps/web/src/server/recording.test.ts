import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { runInNewContext } from "node:vm";
import { test } from "node:test";
import { encodePcm16Wav, recordReceivedAudio } from "../media/recording.ts";

test("WAV encoding is mono PCM16 and bounded to the requested frame count", async () => {
  const wav = new DataView(await encodePcm16Wav([new Float32Array([-2, -1, 0, 0.5, 1, 2])], 8_000, 4).arrayBuffer());
  assert.equal(wav.getUint32(24, true), 8_000);
  assert.equal(wav.getUint16(22, true), 1);
  assert.equal(wav.getUint16(34, true), 16);
  assert.equal(wav.getUint32(40, true), 8);
  assert.deepEqual(Array.from({ length: 4 }, (_, i) => wav.getInt16(44 + i * 2, true)), [-32768, -32768, 0, 16383]);
});

test("recording worklet emits at most its bound, zeros its sink, and stops", async () => {
  const code = await readFile(new URL("../../public/audio/recording-v1/worklet.js", import.meta.url), "utf8");
  const messages: Array<{ type: string; samples?: Float32Array }> = [];
  let Processor: any;
  runInNewContext(code, {
    sampleRate: 48_000,
    AudioWorkletProcessor: class { port = { onmessage: undefined, postMessage: (message: { type: string; samples?: Float32Array }) => messages.push(message) }; },
    registerProcessor: (_name: string, value: unknown) => { Processor = value; },
  });
  const processor = new Processor({ processorOptions: { maximumFrames: 5 } });
  const output = new Float32Array(8).fill(1);
  assert.equal(processor.process([[new Float32Array(8).fill(0.25)]], [[output]]), false);
  assert.ok(output.every((sample) => sample === 0));
  assert.equal(messages[0]?.samples?.length, 5);
  assert.equal(messages[1]?.type, "done");
  assert.equal(processor.process([], []), false);
});

test("cancelling pending recorder startup closes its context without stopping borrowed audio", async (t) => {
  let loaded!: () => void;
  let closed = 0;
  let created = 0;
  let stopped = 0;
  let receiverPaused = false;
  const descriptors = ["AudioContext", "AudioWorkletNode", "Audio"].map((key) => [key, Object.getOwnPropertyDescriptor(globalThis, key)] as const);
  t.after(() => descriptors.forEach(([key, descriptor]) => {
    if (descriptor) Object.defineProperty(globalThis, key, descriptor);
    else Reflect.deleteProperty(globalThis, key);
  }));
  Object.defineProperty(globalThis, "AudioContext", { configurable: true, value: class {
    state = "running";
    audioWorklet = { addModule: () => new Promise<void>((resolve) => { loaded = resolve; }) };
    async resume() {}
    async close() { this.state = "closed"; closed++; }
    createMediaStreamSource() { created++; }
  } });
  Object.defineProperty(globalThis, "AudioWorkletNode", { configurable: true, value: class {} });
  Object.defineProperty(globalThis, "Audio", { configurable: true, value: class {
    async play() {}
    pause() { receiverPaused = true; }
  } });
  const stream = { getAudioTracks: () => [{ readyState: "live", stop: () => stopped++ }] } as unknown as MediaStream;
  const session = recordReceivedAudio(stream);
  const rejection = assert.rejects(session.result, /cancelled/);
  session.stop(); session.stop(); loaded();
  await rejection;
  await new Promise((resolve) => setImmediate(resolve));
  assert.equal(closed, 1);
  assert.equal(created, 0);
  assert.equal(stopped, 0);
  assert.equal(receiverPaused, true);
});
