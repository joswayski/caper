export function waitFor(target: EventTarget, event: string, timeout: number, ready: () => boolean, signal?: AbortSignal) {
  if (signal?.aborted) return Promise.reject(signal.reason);
  if (ready()) return Promise.resolve();
  return new Promise<void>((resolve, reject) => {
    const timer = window.setTimeout(() => finish(new Error(`Timed out waiting for ${event}`)), timeout);
    const listener = () => { if (ready()) finish(); };
    const abort = () => finish(signal?.reason ?? new Error("Connection setup cancelled."));
    const finish = (error?: Error) => {
      window.clearTimeout(timer);
      target.removeEventListener(event, listener);
      signal?.removeEventListener("abort", abort);
      error ? reject(error) : resolve();
    };
    target.addEventListener(event, listener);
    signal?.addEventListener("abort", abort, { once: true });
  });
}

export async function localDescription(pc: RTCPeerConnection, signal?: AbortSignal) {
  await waitFor(pc, "icegatheringstatechange", 5_000, () => pc.iceGatheringState === "complete", signal).catch(() => signal?.throwIfAborted());
  if (!pc.localDescription) throw new Error("WebRTC did not produce a session description.");
  return pc.localDescription.toJSON();
}

export function preferOpus(transceiver: RTCRtpTransceiver) {
  if (typeof RTCRtpSender !== "undefined" && transceiver.setCodecPreferences) {
    const codecs = RTCRtpSender.getCapabilities("audio")?.codecs ?? [];
    transceiver.setCodecPreferences([...codecs.filter((c) => c.mimeType.toLowerCase() === "audio/opus"), ...codecs.filter((c) => c.mimeType.toLowerCase() !== "audio/opus")]);
  }
}
