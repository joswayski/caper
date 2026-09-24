let sharedContext: AudioContext | undefined;
let contextUsers = 0;
const blockedOutputs = new Set<object>();

export function acquireAudioContext() {
  sharedContext ??= new AudioContext({ latencyHint: "interactive" });
  contextUsers++;
  return sharedContext;
}

export function releaseAudioContext(context: AudioContext) {
  if (context !== sharedContext || --contextUsers > 0) return;
  sharedContext = undefined;
  void context.close().catch(() => undefined);
}

/** Records whether a remote output is waiting for a user gesture before it can play. */
export function setPlaybackBlocked(output: object, blocked: boolean) {
  if (blocked) blockedOutputs.add(output);
  else blockedOutputs.delete(output);
}

export function playbackDiagnostics() {
  return {
    context: sharedContext?.state,
    contextSampleRate: sharedContext?.sampleRate,
    contextUsers,
    blockedOutputs: blockedOutputs.size,
  };
}
