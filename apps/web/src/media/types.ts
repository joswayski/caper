import type { AudioSetup, NoiseSuppression } from "./microphone";

export type CallPhase = "idle" | "joining" | "connected" | "reconnecting" | "leaving" | "failed";
export type MediaKind = "microphone";

export interface CallTrack {
  id: string;
  kind: MediaKind;
}

export interface Participant {
  id: string;
  name: string;
  muted: boolean;
  deafened: boolean;
  tracks: CallTrack[];
}

export interface CallSnapshot {
  participants: Participant[];
}

export interface JoinResponse {
  token: string;
  id: string;
  iceServers: RTCIceServer[];
}

export interface SessionDescriptionResponse {
  sessionDescription?: RTCSessionDescriptionInit;
  tracks?: Array<{ mid: string }>;
  requiresImmediateRenegotiation?: boolean;
}

export interface RemoteMedia {
  trackId: string;
  participantId: string;
  kind: MediaKind;
  stream: MediaStream;
}

export interface CallViewState {
  phase: CallPhase;
  muted: boolean;
  deafened: boolean;
  monitoring: boolean;
  monitorStream?: MediaStream;
  monitorConnecting?: boolean;
  monitorStatus?: string;
  selfId?: string;
  participants: Participant[];
  remoteMedia: RemoteMedia[];
  speaking?: string[];
  error?: string;
  diagnostics?: string;
  noiseSuppression?: NoiseSuppression;
  audioSetup?: AudioSetup;
  noiseSuppressionStatus?: string;
}
