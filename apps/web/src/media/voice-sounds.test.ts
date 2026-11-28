import { expect, test } from "vitest";
import type { CallPhase, Participant } from "./types.ts";
import { voiceSoundFor } from "./voice-sounds.ts";

const person = (id: string): Participant => ({ id, name: id, muted: false, deafened: false, tracks: [] });
const call = (phase: CallPhase, ids: string[] = ["me"], deafened = false, selfId = "me") => ({
  phase,
  selfId,
  deafened,
  participants: ids.map(person),
});

test("chimes for your own join and leave, but not a cancelled join or a reconnect", () => {
  expect(voiceSoundFor(call("joining", []), call("connected"))).toBe("join");
  expect(voiceSoundFor(call("connected"), call("leaving"))).toBe("leave");
  expect(voiceSoundFor(call("reconnecting"), call("failed"))).toBe("leave");
  expect(voiceSoundFor(call("joining", []), call("idle", []))).toBeUndefined();
  expect(voiceSoundFor(call("connected"), call("reconnecting"))).toBeUndefined();
  expect(voiceSoundFor(call("reconnecting", ["me"]), call("connected", ["me2"], false, "me2"))).toBeUndefined();
});

test("chimes as others come and go, unless you are deafened", () => {
  expect(voiceSoundFor(call("connected", ["me"]), call("connected", ["me", "ana"]))).toBe("join");
  expect(voiceSoundFor(call("connected", ["me", "ana"]), call("connected", ["me"]))).toBe("leave");
  expect(voiceSoundFor(call("connected", ["me", "ana"]), call("connected", ["me", "ana"]))).toBeUndefined();
  expect(voiceSoundFor(call("connected", ["me"]), call("connected", ["me", "ana"], true))).toBeUndefined();
});
