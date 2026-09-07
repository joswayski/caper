import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { recordReceivedAudio } from "../media/recording.ts";

function install(t: TestContext, key: string, value: unknown) {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, key);
  Object.defineProperty(globalThis, key, { configurable: true, value });
  t.after(() => descriptor
    ? Object.defineProperty(globalThis, key, descriptor)
    : Reflect.deleteProperty(globalThis, key));
}

test("received recording keeps browser timestamps and finishes without stopping borrowed audio", async (t) => {
  let instance!: FakeMediaRecorder;
  let receiver!: FakeAudio;
  let trackStops = 0;

  class FakeAudio {
    muted = false;
    srcObject: MediaStream | null = null;
    playing = false;
    constructor() { receiver = this; }
    async play() { this.playing = true; }
    pause() { this.playing = false; }
  }

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
        if (receiver.playing) this.ondataavailable?.({ data: new Blob(["timestamped opus"], { type: this.mimeType }) });
        this.onstop?.();
      });
    }
  }
  install(t, "MediaRecorder", FakeMediaRecorder);
  install(t, "Audio", FakeAudio);
  const stream = { getAudioTracks: () => [{ readyState: "live", stop: () => trackStops++ }] } as unknown as MediaStream;

  const recording = recordReceivedAudio(stream);
  assert.equal(instance.mimeType, "audio/webm;codecs=opus");
  assert.equal(receiver.muted, true);
  assert.equal(receiver.srcObject, stream);
  assert.equal(receiver.playing, true);
  recording.finish();
  const blob = await recording.result;

  assert.equal(blob.type, "audio/webm;codecs=opus");
  assert.equal(await blob.text(), "timestamped opus");
  assert.equal(receiver.playing, false);
  assert.equal(receiver.srcObject, null);
  assert.equal(trackStops, 0);
});

test("cancelling a recording rejects without stopping borrowed audio", async (t) => {
  let trackStops = 0;
  class FakeAudio {
    muted = false;
    srcObject: MediaStream | null = null;
    async play() {}
    pause() {}
  }
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
  install(t, "MediaRecorder", FakeMediaRecorder);
  install(t, "Audio", FakeAudio);
  const stream = { getAudioTracks: () => [{ readyState: "live", stop: () => trackStops++ }] } as unknown as MediaStream;

  const recording = recordReceivedAudio(stream);
  const rejection = assert.rejects(recording.result, /cancelled/);
  recording.cancel();
  await rejection;
  assert.equal(trackStops, 0);
});
