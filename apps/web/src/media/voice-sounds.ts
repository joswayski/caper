import joinSound from "../../../../shared/sounds/voice-join.wav";
import leaveSound from "../../../../shared/sounds/voice-leave.wav";
import type { CallPhase, Participant } from "./types.ts";

export type VoiceSound = "join" | "leave";

const sounds: Record<VoiceSound, string> = { join: joinSound, leave: leaveSound };

/** A short voice chime on the chosen speaker ("" is the system default), at the call's output volume (percent). */
export function playVoiceSound(sound: VoiceSound, output = "", volume = 100) {
  if (typeof Audio === "undefined" || volume <= 0) return;
  const audio = new Audio(sounds[sound]);
  audio.volume = Math.min(1, volume / 100);
  const routed = output && "setSinkId" in audio ? audio.setSinkId(output).catch(() => undefined) : Promise.resolve();
  void routed.then(() => audio.play()).catch(() => undefined);
}

export interface VoiceSoundState {
  phase: CallPhase;
  selfId?: string;
  deafened: boolean;
  participants: Participant[];
}

/**
 * The chime one call update plays: yours when you get in or leave (a cancelled
 * join stays silent), and others' as they join or leave while you are in and
 * not deafened. A reconnect keeps quiet, so only real arrivals and departures sound.
 */
export function voiceSoundFor(before: VoiceSoundState, after: VoiceSoundState): VoiceSound | undefined {
  const inCall = (phase: CallPhase) => phase === "connected" || phase === "reconnecting";
  if (before.phase === "joining" && after.phase === "connected") return "join";
  if (inCall(before.phase) && !inCall(after.phase)) return "leave";
  if (before.phase !== "connected" || after.phase !== "connected" || after.deafened) return;
  if (!before.selfId || before.selfId !== after.selfId) return;
  const others = (state: VoiceSoundState) =>
    new Set(state.participants.filter((person) => person.id !== state.selfId).map((person) => person.id));
  const was = others(before);
  const now = others(after);
  if ([...now].some((id) => !was.has(id))) return "join";
  if ([...was].some((id) => !now.has(id))) return "leave";
}
