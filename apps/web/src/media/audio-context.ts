let sharedContext: AudioContext | undefined;
let contextUsers = 0;

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
