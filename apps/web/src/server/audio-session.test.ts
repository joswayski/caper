import assert from "node:assert/strict";
import { test, type TestContext } from "node:test";
import { beginCapture, mixWithOtherAudio } from "../audio/session.ts";

function installSession(t: TestContext, session?: { type: string }) {
  const descriptor = Object.getOwnPropertyDescriptor(globalThis, "navigator");
  Object.defineProperty(globalThis, "navigator", { value: session ? { audioSession: session } : {}, configurable: true });
  t.after(() => { if (descriptor) Object.defineProperty(globalThis, "navigator", descriptor); else Reflect.deleteProperty(globalThis, "navigator"); });
}

test("UI sounds mix with other apps' audio, but never while the microphone is captured", (t) => {
  const session = { type: "auto" };
  installSession(t, session);
  mixWithOtherAudio();
  assert.equal(session.type, "ambient", "idle: mix instead of pausing other apps' music");
  const first = beginCapture();
  assert.equal(session.type, "auto", "capture restores the default session before getUserMedia");
  mixWithOtherAudio();
  assert.equal(session.type, "auto", "a sound during a call never switches the session");
  const second = beginCapture();
  first.end();
  first.end();
  mixWithOtherAudio();
  assert.equal(session.type, "auto", "still capturing (mic test beside a call)");
  second.end();
  mixWithOtherAudio();
  assert.equal(session.type, "ambient");
  // A capture whose track ended without being released no longer blocks mixing.
  const leaked = beginCapture();
  const track = { readyState: "live" as MediaStreamTrackState };
  leaked.track(track);
  mixWithOtherAudio();
  assert.equal(session.type, "auto");
  track.readyState = "ended";
  mixWithOtherAudio();
  assert.equal(session.type, "ambient");
});

test("browsers without the Audio Session API are left alone", (t) => {
  installSession(t);
  assert.doesNotThrow(() => { mixWithOtherAudio(); beginCapture().end(); });
});
