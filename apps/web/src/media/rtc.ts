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
  // Follow Cloudflare's browser flow: exchange SDP without waiting for every
  // STUN/TURN probe to finish. Gathering continues; callers still gate audio on
  // the actual transport connection, not on this description being available.
  signal?.throwIfAborted();
  if (!pc.localDescription) throw new Error("WebRTC did not produce a session description.");
  return pc.localDescription.toJSON();
}

/** Request codec-managed silence suppression, never a microphone volume gate. */
export function withOpusDtx(description: RTCSessionDescriptionInit): RTCSessionDescriptionInit {
  if (!description.sdp) return description;
  // RFC 7587: usedtx is a receive preference. It must be in the REMOTE
  // description to affect our sender, including subsequent remote offers.
  // Keep the codec's own speech detection; do not change bitrate, FEC or ptime.
  const sdp = description.sdp.split(/(?=^m=)/m).map((section) => {
    if (!section.startsWith("m=audio ")) return section;
    const newline = section.includes("\r\n") ? "\r\n" : "\n";
    const lines = section.split(/\r?\n/);
    const opus = new Set(lines.flatMap((line) => {
      const match = /^a=rtpmap:(\d+) opus\/48000\/2$/i.exec(line);
      return match ? [match[1]] : [];
    }));
    for (const payload of opus) {
      const fmtp = new RegExp(`^a=fmtp:${payload}(?:[ \\t]|$)`);
      const index = lines.findIndex((line) => fmtp.test(line));
      if (index === -1) {
        const mapping = lines.findIndex((line) => line.startsWith(`a=rtpmap:${payload} `));
        lines.splice(mapping + 1, 0, `a=fmtp:${payload} usedtx=1`);
      } else {
        const parameters = lines[index].replace(fmtp, "").trim().split(";")
          .filter((parameter) => parameter.trim() && !/^usedtx\s*=/i.test(parameter.trim()));
        lines[index] = `a=fmtp:${payload} ${[...parameters, "usedtx=1"].join(";")}`;
      }
    }
    return lines.join(newline);
  }).join("");
  return { ...description, sdp };
}

export function preferOpus(transceiver: RTCRtpTransceiver) {
  if (typeof RTCRtpSender !== "undefined" && transceiver.setCodecPreferences) {
    const codecs = RTCRtpSender.getCapabilities("audio")?.codecs ?? [];
    transceiver.setCodecPreferences([...codecs.filter((c) => c.mimeType.toLowerCase() === "audio/opus"), ...codecs.filter((c) => c.mimeType.toLowerCase() !== "audio/opus")]);
  }
}
