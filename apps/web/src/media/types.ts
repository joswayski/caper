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
  turn?: TurnGeneration;
  /** Present when the API published the join's microphone offer in the same request. */
  publish?: SessionDescriptionResponse;
  /** Everyone already publishing, pulled into a receive-only session during join. */
  receive?: BatchSubscribeResponse;
  /** The join adopted the warm sessions whose ticket it sent. */
  warm?: boolean;
}

export interface TurnGeneration {
  generation: string;
  refreshAfterMs: number;
  expiresInMs: number;
}

export interface TurnResponse {
  iceServers: RTCIceServer[];
  turn: TurnGeneration;
}

export interface SessionDescriptionResponse {
  sessionDescription?: RTCSessionDescriptionInit;
  tracks?: Array<{ mid: string }>;
  requiresImmediateRenegotiation?: boolean;
}

/** A batched pull: allocated receiving MIDs plus sources that cannot be pulled yet. */
export interface BatchSubscribeResponse {
  sessionDescription?: RTCSessionDescriptionInit;
  tracks?: Array<{ trackId: string; mid: string }>;
  gone?: string[];
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
  /** Microphone capture and the join request run concurrently; both are measured from Join. */
  microphoneMs: number;
  /** Hardware open vs. audio processing, e.g. "device 150 ms · processing 30 ms". */
  microphoneDetail?: string;
  sessionMs: number;
  signalingMs: number;
  transportMs: number;
  iceMs?: number;
  rosterMs: number;
  /** From Join until audio from the people already present can play; set shortly after Joined. */
  hearingMs?: number;
  /** Connectivity checks on the selected candidate pair, first sampled after joining. */
  checks?: string;
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
  stateSyncPending?: boolean;
  liveUpdatesPending?: boolean;
  inputVolume: number;
  monitoring: boolean;
  monitorStream?: MediaStream;
  selfId?: string;
  localMedia?: MediaStream;
  participants: Participant[];
  remoteMedia: RemoteMedia[];
  error?: string;
  diagnostics?: ConnectionDiagnostics;
  noiseSuppression?: NoiseSuppression;
  audioSetup?: AudioSetup;
  voiceProcessingStrength?: number;
  noiseSuppressionStatus?: string;
}
