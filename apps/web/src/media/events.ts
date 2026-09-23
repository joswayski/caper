import type { CallSnapshot } from "./types.ts";

/** Validate a media snapshot before it reaches call state or SDP reconciliation. */
export function callSnapshot(value: unknown, spectator: boolean): CallSnapshot & { revision: number } {
  if (!value || typeof value !== "object") throw new Error("Invalid live update snapshot.");
  const event = value as { type?: unknown; participants?: unknown; revision?: unknown };
  if (event.type !== "snapshot" || !Array.isArray(event.participants)
    || !event.participants.every((participant) => {
      if (!participant || typeof participant !== "object") return false;
      const item = participant as Record<string, unknown>;
      return typeof item.id === "string" && typeof item.name === "string"
        && typeof item.muted === "boolean" && typeof item.deafened === "boolean"
        && (spectator ? !("tracks" in item) : Array.isArray(item.tracks)
          && item.tracks.every((track) => track && typeof track === "object"
            && typeof (track as { id?: unknown }).id === "string"
            && (track as { kind?: unknown }).kind === "microphone"));
    })
    || !Number.isSafeInteger(event.revision) || (event.revision as number) < 0) {
    throw new Error("Invalid live update snapshot.");
  }
  const snapshot = event as unknown as CallSnapshot & { revision: number };
  return spectator
    ? { ...snapshot, participants: snapshot.participants.map((participant) => ({ ...participant, tracks: [] })) }
    : snapshot;
}
