/**
 * The page's audio session on browsers with the Audio Session API (Safari/iOS
 * WebKit 17+): UI sounds mix with other apps' audio instead of pausing their
 * music, as a native app's would. Only while nothing captures the microphone:
 * capture and calls keep the browser's default session. The type is never
 * switched while a capture is running.
 */
type Session = { type: string };
type Capture = { readyState: MediaStreamTrackState } | undefined;
/** Live captures; a track that ended without release no longer counts. */
const captures = new Set<{ track: Capture }>();

function session(): Session | undefined {
  return (globalThis.navigator as (Navigator & { audioSession?: Session }) | undefined)?.audioSession;
}

function capturing() {
  for (const capture of captures) if (capture.track?.readyState === "ended") captures.delete(capture);
  return captures.size > 0;
}

/** Before decorative playback. "ambient" mixes with other audio and follows the silent switch. */
export function mixWithOtherAudio() {
  const current = session();
  if (!current || current.type === "ambient" || capturing()) return;
  try { current.type = "ambient"; } catch { /* Unsupported value: keep the default. */ }
}

/**
 * Before getUserMedia. Call `track()` with the captured track once it exists;
 * call `end()` when the capture stops.
 */
export function beginCapture() {
  const capture: { track: Capture } = { track: undefined };
  captures.add(capture);
  const current = session();
  if (current && current.type !== "auto") {
    try { current.type = "auto"; } catch { /* Keep whatever the browser chose. */ }
  }
  return {
    track(track: Capture) { capture.track = track; },
    end() { captures.delete(capture); },
  };
}

/** Forgets captures other tests left running. */
export function resetCapturesForTests() {
  captures.clear();
}
