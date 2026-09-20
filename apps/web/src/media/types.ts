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
  countryCode?: string;
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

export interface ConnectionDiagnostics {
  join: string;
  microphoneSessionMs: number;
  signalingMs: number;
  transportMs: number;
  rosterMs: number;
  receivedBytes: number;
  sentBytes: number;
  receiveBitrate: number;
  sendBitrate: number;
  packetsLost: number;
  maxJitterMs: number;
  roundTripMs: number;
  route: "relay" | "direct" | "unknown";
}

export interface CallViewState {
  phase: CallPhase;
  muted: boolean;
  deafened: boolean;
  inputVolume: number;
  monitoring: boolean;
  monitorStream?: MediaStream;
  monitorConnecting?: boolean;
  monitorStatus?: string;
  selfId?: string;
  localMedia?: MediaStream;
  participants: Participant[];
  remoteMedia: RemoteMedia[];
  error?: string;
  diagnostics?: ConnectionDiagnostics;
  noiseSuppression?: NoiseSuppression;
  audioSetup?: AudioSetup;
  noiseSuppressionStatus?: string;
}
