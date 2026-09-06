import assert from "node:assert/strict";
import { test } from "node:test";
import { recordReceivedAudio } from "../media/recording.ts";

test("received recording keeps browser timestamps and finishes without stopping borrowed audio", async (t) => {
  let instance!: FakeMediaRecorder;
  let trackStops = 0;
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "MediaRecorder");
  t.after(() => descriptor
    ? Object.defineProperty(globalThis, "MediaRecorder", descriptor)
    : Reflect.deleteProperty(globalThis, "MediaRecorder"));

  class FakeMediaRecorder {
    static isTypeSupported(type: string) { return type === "audio/webm;codecs=opus"; }
    state: RecordingState = "inactive";
    mimeType: string;
    ondataavailable: ((event: { data: Blob }) => void) | null = null;
    onerror: ((event: { error: DOMException }) => void) | null = null;
    onstop: (() => void) | null = null;
    constructor(_stream: MediaStream, options?: MediaRecorderOptions) {
      instance = this;
      this.mimeType = options?.mimeType ?? "";
    }
    start(timeslice?: number) {
      assert.equal(timeslice, 250);
      this.state = "recording";
    }
    stop() {
      this.state = "inactive";
      queueMicrotask(() => {
        this.ondataavailable?.({ data: new Blob(["timestamped opus"], { type: this.mimeType }) });
        this.onstop?.();
      });
    }
  }
  Object.defineProperty(globalThis, "MediaRecorder", { configurable: true, value: FakeMediaRecorder });
  const stream = { getAudioTracks: () => [{ readyState: "live", stop: () => trackStops++ }] } as unknown as MediaStream;

  const recording = recordReceivedAudio(stream);
  assert.equal(instance.mimeType, "audio/webm;codecs=opus");
  recording.finish();
  const blob = await recording.result;

  assert.equal(blob.type, "audio/webm;codecs=opus");
  assert.equal(await blob.text(), "timestamped opus");
  assert.equal(trackStops, 0);
});

test("cancelling a recording rejects without stopping borrowed audio", async (t) => {
  let trackStops = 0;
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "MediaRecorder");
  t.after(() => descriptor
    ? Object.defineProperty(globalThis, "MediaRecorder", descriptor)
    : Reflect.deleteProperty(globalThis, "MediaRecorder"));
  class FakeMediaRecorder {
    static isTypeSupported() { return false; }
    state: RecordingState = "inactive";
    mimeType = "audio/webm";
    ondataavailable = null;
    onerror = null;
    onstop: (() => void) | null = null;
    constructor(_stream: MediaStream) {}
    start() { this.state = "recording"; }
    stop() { this.state = "inactive"; queueMicrotask(() => this.onstop?.()); }
  }
  Object.defineProperty(globalThis, "MediaRecorder", { configurable: true, value: FakeMediaRecorder });
  const stream = { getAudioTracks: () => [{ readyState: "live", stop: () => trackStops++ }] } as unknown as MediaStream;

  const recording = recordReceivedAudio(stream);
  const rejection = assert.rejects(recording.result, /cancelled/);
  recording.cancel();
  await rejection;
  assert.equal(trackStops, 0);
});
