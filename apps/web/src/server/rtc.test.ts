import assert from "node:assert/strict";
import { test } from "node:test";
import { localDescription, withOpusDtx } from "../media/rtc.ts";

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

for (const newline of ["\r\n", "\n"]) {
  for (const type of ["offer", "answer"] as const) {
    test(`DTX in remote ${type} preserves codecs and transport (${JSON.stringify(newline)})`, () => {
      const lines = [
        "v=0", "a=group:BUNDLE mic speaker video", "a=ice-ufrag:fixture",
        "m=audio 9 UDP/TLS/RTP/SAVPF 109 0", "a=mid:mic", "a=recvonly",
        "a=rtpmap:109 opus/48000/2", "a=fmtp:109 minptime=10;useinbandfec=1;usedtx=0;maxaveragebitrate=64000",
        "a=rtpmap:0 PCMU/8000", "a=fmtp:0 usedtx=0", "a=ptime:20",
        "m=audio 9 UDP/TLS/RTP/SAVPF 111", "a=mid:speaker", "a=sendonly",
        "a=rtpmap:111 OPUS/48000/2", // No fmtp; this payload differs from the first section.
        "m=video 9 UDP/TLS/RTP/SAVPF 109", "a=mid:video",
        "a=rtpmap:109 VP8/90000", "a=fmtp:109 x-google-start-bitrate=1000", "",
      ];
      const original = { type, sdp: lines.join(newline) };
      const expected = [...lines];
      expected[7] = "a=fmtp:109 minptime=10;useinbandfec=1;maxaveragebitrate=64000;usedtx=1";
      expected.splice(15, 0, "a=fmtp:111 usedtx=1");
      const result = withOpusDtx(original);
      assert.deepEqual(result, { type, sdp: expected.join(newline) });
      assert.equal(original.sdp, lines.join(newline), "do not mutate the provider response");
      assert.deepEqual(withOpusDtx(result), result, "renegotiation must not duplicate usedtx");
    });
  }
}

test("DTX replaces existing preferences, including duplicate/whitespace variants", () => {
  const sdp = "m=audio 9 UDP/TLS/RTP/SAVPF 112\r\na=rtpmap:112 opus/48000/2\r\na=fmtp:112 useinbandfec=1; usedtx = 0;USEDTX=1;stereo=0\r\n";
  assert.equal(withOpusDtx({ type: "answer", sdp }).sdp,
    "m=audio 9 UDP/TLS/RTP/SAVPF 112\r\na=rtpmap:112 opus/48000/2\r\na=fmtp:112 useinbandfec=1;stereo=0;usedtx=1\r\n");
});

test("non-Opus descriptions and rollback stay unchanged", () => {
  for (const description of [
    { type: "rollback" as const },
    { type: "answer" as const, sdp: "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 0\r\na=rtpmap:0 PCMU/8000\r\n" },
    { type: "answer" as const, sdp: "v=0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=sctp-port:5000\r\n" },
  ]) assert.deepEqual(withOpusDtx(description), description);
});
