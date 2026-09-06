import assert from "node:assert/strict";
import { test } from "node:test";
import { localDescription } from "../media/rtc.ts";

for (const type of ["offer", "answer"] as const) {
  test(`${type} is available while ICE gathering is still pending, without a timer or candidate wait`, async (t) => {
    t.mock.timers.enable({ apis: ["setTimeout"] });
    const description = { type, sdp: "v=0\r\n" };
    const pc = Object.assign(new EventTarget(), {
      iceGatheringState: "gathering",
      localDescription: { toJSON: () => description },
    });
    const listeners = t.mock.method(pc, "addEventListener");
    // No timer is advanced and no candidates/completion events are emitted.
    const result = await localDescription(pc as unknown as RTCPeerConnection);
    assert.deepEqual(result, description);
    assert.equal(listeners.mock.callCount(), 0);
    assert.equal(pc.iceGatheringState, "gathering");
  });
}

test("local description still rejects missing SDP and an already cancelled call", async () => {
  const pc = { localDescription: null } as RTCPeerConnection;
  await assert.rejects(localDescription(pc), /did not produce a session description/);
  const controller = new AbortController();
  controller.abort();
  await assert.rejects(localDescription(pc, controller.signal), { name: "AbortError" });
});
