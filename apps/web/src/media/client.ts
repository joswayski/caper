import { captureMicrophone, type AudioSetup, type Microphone, type NoiseSuppression } from "./microphone.ts";
import { appGateway, GatewayError } from "../gateway/client.ts";
import { NoiseAssets } from "./noise-assets.ts";
import { DpdfnetPreparation } from "./dpdfnet-preparation.ts";
import { EventConnection } from "./event-connection.ts";
import { localDescription, preferOpus, waitFor, withOpusDtx } from "./rtc.ts";
import { TurnRenewal } from "./turn-renewal.ts";
import { clampVoiceProcessingStrength, DEFAULT_VOICE_PROCESSING_STRENGTH } from "./voice-processing.ts";
export { waitFor } from "./rtc.ts";
import type {
  BatchSubscribeResponse,
  CallSnapshot,
  ConnectionDiagnostics,
  CallViewState,
  JoinResponse,
  MediaKind,
  Participant,
  RemoteMedia,
  SessionDescriptionResponse,
} from "./types";

interface PreparedPublication {
  pc: RTCPeerConnection;
  transceiver: RTCRtpTransceiver;
  offer: RTCSessionDescriptionInit;
  mid: string;
}

const CONNECT_TIMEOUT_MS = 12_000;
const MAX_REJOINS = 3;
const FETCH_TIMEOUT_MS = 25_000;
const SNAPSHOT_TIMEOUT_MS = 5_000;
const CONTROL_RECOVERY_MS = 30_000;
const DISCONNECT_GRACE_MS = 10_000;
/**
 * Re-checks for a listed source that cannot be pulled yet. A newcomer's track is
 * listed at publication but pullable only once its transport sends media, which
 * usually takes 0.3–3 s, so check densely early and back off to the same ~16 s
 * window. A refused pull allocates nothing and needs no renegotiation.
 */
const PULL_RETRY_DELAYS_MS = [250, 250, 500, 500, 1_000, 2_000, 4_000, 8_000];

class CallApiError extends Error {
  readonly status: number;
  readonly code?: string;
  constructor(message: string, status: number, code?: string) { super(message); this.status = status; this.code = code; }
}

function message(error: unknown) {
  if (error instanceof DOMException && error.name === "NotAllowedError") {
    return "Microphone permission was denied. Allow access and try again.";
  }
  if (error instanceof DOMException && (error.name === "NotFoundError" || error.name === "OverconstrainedError")) {
    return "No microphone is available. Connect a microphone or choose another input and try again.";
  }
  if (error instanceof DOMException && error.name === "NotReadableError") {
    return "Your microphone could not be opened. Check whether another app is using it and try again.";
  }
  return error instanceof Error ? error.message : "The call could not continue.";
}

function transientControlError(error: unknown) {
  return error instanceof CallApiError
    ? error.status === 408 || error.status === 429 || error.status >= 500
    : error instanceof TypeError || (error instanceof DOMException && error.name === "AbortError");
}

const MEDIA_OPERATIONS = new Set([
  "join", "prepare", "state", "leave", "snapshot", "publish", "subscribe", "negotiate", "close",
  "turn", "restart-ice", "restart-ice-ack", "status",
]);

const PREPARE_INTERVAL_MS = 4_000;
const prepared = new Map<string, number>();

/**
 * Asks the API to create a signed-in member's provider session and TURN
 * credentials moments before they join a channel, so Join skips both provider
 * calls. Call it on intent (pointer over or focus on Join). Cloudflare drops an
 * unused session within 10-15 s, so the API keeps it for 8 s; this reissues at
 * most every 4 s per channel. Best effort: failures only mean an ordinary join.
 */
export function prepareVoiceJoin(apiRoot: string) {
  const channelId = channelFromRoot(apiRoot);
  if (!channelId || typeof window === "undefined") return; // The public demo creates on join.
  const now = Date.now();
  if (now - (prepared.get(apiRoot) ?? -Infinity) < PREPARE_INTERVAL_MS) return;
  prepared.set(apiRoot, now);
  void appGateway().command({ method: "media.prepare", channelId, body: {}, timeoutMs: 5_000 }).catch(() => undefined);
}

function channelFromRoot(apiRoot: string) {
  const match = /^\/api\/channels\/([^/]+)\/media$/.exec(apiRoot);
  return match ? decodeURIComponent(match[1]) : undefined;
}

export class PublicCallClient {
  private pc?: RTCPeerConnection;
  private token?: string;
  private name = "Guest";
  private selfId?: string;
  private phase: CallViewState["phase"] = "idle";
  private participants: Participant[] = [];
  private remoteMedia = new Map<string, RemoteMedia>();
  private localMedia?: MediaStream;
  private senders = new Map<MediaKind, { sender: RTCRtpSender; mid: string; track: MediaStreamTrack }>();
  private subscriptions = new Map<string, string>();
  private queue: Promise<unknown> = Promise.resolve();
  private mediaQueue: Promise<unknown> = Promise.resolve();
  private pollTimer?: number;
  private reconnectTimer?: number;
  private disconnectTimer?: number;
  private turnRenewal?: TurnRenewal;
  private pollRetryTimer?: number;
  private unavailableRetryTimer?: number;
  private unavailableRetries = 0;
  private batchUnsupported = false;
  private eventRetryTimer?: number;
  private eventRetryAttempts = 0;
  private eventDrainRetries = 0;
  private controlFailedSince?: number;
  private stateDirty = false;
  private stateRevision = 0;
  private statePromise?: Promise<void>;
  private stateRetryTimer?: number;
  private snapshotPromise?: Promise<boolean>;
  private generation = 0;
  private reconnects = 0;
  private muted = false;
  private deafened = false;
  private mutedBeforeDeafen?: boolean;
  private inputVolume = 100;
  private monitoring = false;
  private monitorStream?: MediaStream;
  private localTestTrack?: MediaStreamTrack;
  private localTestGeneration = 0;
  private localTestController?: AbortController;
  private stateBeforeMonitoring?: { muted: boolean; deafened: boolean };
  private pollPromise?: Promise<void>;
  private pollAgain = false;
  private pendingSnapshot?: CallSnapshot & { revision?: number };
  private pushedSnapshotVersion = 0;
  private snapshotInvalidation = 0;
  private latestRevision?: number;
  private events?: EventConnection;
  private microphoneDeviceId?: string;
  private statsTimer?: number;
  private diagnostics?: ConnectionDiagnostics;
  private previousStats?: { received: number; sent: number; sampledAt: number };
  private mediaDetail?: object;
  private noiseSuppression: NoiseSuppression = "dpdfnet8";
  private audioSetup: AudioSetup = "headphones";
  private voiceProcessingStrength = DEFAULT_VOICE_PROCESSING_STRENGTH;
  private microphoneStatus?: string;
  private captureAttempt: "not-started" | "opening" | "opened" | "failed" = "not-started";
  private captureError?: string;
  private captures = new Map<MediaStreamTrack, Microphone>();
  private captureController = new AbortController();
  private joinTiming?: Omit<ConnectionDiagnostics, "receivedBytes" | "sentBytes" | "receiveBitrate" | "sendBitrate" | "packetsLost" | "maxJitterMs" | "roundTripMs" | "route">;
  private readonly noiseAssets = new NoiseAssets();
  private readonly dpdfnet = new DpdfnetPreparation();
  private readonly apiRoot: string;
  private readonly fetchTransport?: typeof fetch;

  private readonly changed: (state: CallViewState) => void;
  constructor(changed: (state: CallViewState) => void, apiRoot = "/api/media", fetchTransport?: typeof fetch) {
    this.changed = changed;
    this.apiRoot = apiRoot;
    this.fetchTransport = fetchTransport;
  }

  /** Carry user intent to a new channel without sharing session capabilities or media. */
  copyAudioPreferencesFrom(previous: PublicCallClient) {
    this.muted = previous.muted;
    this.deafened = previous.deafened;
    this.mutedBeforeDeafen = previous.mutedBeforeDeafen;
    this.inputVolume = previous.inputVolume;
    this.voiceProcessingStrength = previous.voiceProcessingStrength;
    this.noiseSuppression = previous.noiseSuppression;
    this.audioSetup = previous.audioSetup;
    this.emit();
  }

  prepareMicrophone() {
    // Download/compile only: no permission prompt, hardware capture or AudioContext.
    if (this.phase !== "idle") return; // Do not hold a spare model alongside an active capture.
    if (this.noiseSuppression === "dpdfnet8") {
      void this.dpdfnet.prepare().catch(() => undefined);
      return;
    }
    this.dpdfnet.stop();
    if (this.noiseSuppression === "rnnoise") void this.noiseAssets.load("rnnoise").catch(() => undefined);
    else if (this.noiseSuppression.startsWith("deepfilter")) void this.noiseAssets.load("deepfilter").catch(() => undefined);
  }

  private emit(error?: string) {
    this.changed({
      phase: this.phase,
      muted: this.muted,
      deafened: this.deafened,
      stateSyncPending: this.phase === "connected" && this.stateDirty,
      liveUpdatesPending: this.phase === "connected" && !this.events?.connected,
      inputVolume: this.inputVolume,
      monitoring: this.monitoring,
      monitorStream: this.monitorStream,
      selfId: this.selfId,
      localMedia: this.localMedia,
      participants: this.participants,
      // Subscriptions are negotiated while joining, but nobody may hear the room
      // until the join has actually completed and the microphone is live.
      remoteMedia: this.phase === "connected" ? [...this.remoteMedia.values()] : [],
      diagnostics: this.diagnostics,
      noiseSuppression: this.noiseSuppression,
      audioSetup: this.audioSetup,
      voiceProcessingStrength: this.voiceProcessingStrength,
      noiseSuppressionStatus: this.captures.get(this.senders.get("microphone")?.track!)?.status ?? this.microphoneStatus,
      error,
    });
  }

  private serialize<T>(operation: () => Promise<T>, generation = this.generation): Promise<T> {
    const guarded = async () => {
      if (generation !== this.generation) throw new Error("Call session changed.");
      const value = await operation();
      if (generation !== this.generation) throw new Error("Call session changed.");
      return value;
    };
    const result = this.queue.then(guarded, guarded);
    this.queue = result.catch(() => undefined);
    return result;
  }

  private serializeMedia<T>(operation: () => Promise<T>, generation = this.generation): Promise<T> {
    const guarded = async () => {
      if (generation !== this.generation) throw new Error("Call session changed.");
      const value = await operation();
      if (generation !== this.generation) throw new Error("Call session changed.");
      return value;
    };
    const result = this.mediaQueue.then(guarded, guarded);
    this.mediaQueue = result.catch(() => undefined);
    return result;
  }

  private async api<T = void>(operation: string, body: object = {}, token = this.token, timeout = FETCH_TIMEOUT_MS, ownerSignal?: AbortSignal): Promise<T> {
    if (!MEDIA_OPERATIONS.has(operation)) throw new Error(`Unsupported media operation: ${operation}.`);
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), timeout);
    const owned = operation === "join" || operation === "leave" ? [controller.signal] : [controller.signal, this.captureController.signal];
    if (ownerSignal) owned.push(ownerSignal);
    const signal = owned.length === 1 ? owned[0] : AbortSignal.any(owned);
    try {
      if (!this.fetchTransport) {
        try {
          return await appGateway().command({
            method: `media.${operation}`, channelId: channelFromRoot(this.apiRoot), token,
            body, timeoutMs: timeout, signal,
          }) as T;
        } catch (error) {
          if (error instanceof GatewayError) throw new CallApiError(error.message, error.status, error.code);
          throw error;
        }
      }
      for (let attempt = 0; ; attempt++) {
        signal.throwIfAborted();
        const response = await this.fetchTransport(`${this.apiRoot}/${operation}`, {
          method: "POST",
          headers: {
            "content-type": "application/json",
            ...(token ? { "x-caper-media-token": token } : {}),
          },
          body: JSON.stringify(body),
          keepalive: operation === "leave",
          signal,
        });
        if (!response.ok) {
          const detail = await response.json().catch(() => ({})) as { error?: string; code?: string };
          // Only this admission rejection guarantees the operation never started.
          // Never replay an ambiguous provider mutation after a generic 5xx/timeout.
          if (response.status === 503 && detail.code === "api_draining" && attempt < 3) {
            let retryTimer: number | undefined;
            let abort!: () => void;
            await new Promise<void>((resolve, reject) => {
              abort = () => reject(signal.reason);
              retryTimer = window.setTimeout(resolve, 250 * 2 ** attempt);
              signal.addEventListener("abort", abort, { once: true });
              if (signal.aborted) abort();
            }).finally(() => {
              window.clearTimeout(retryTimer);
              signal.removeEventListener("abort", abort);
            });
            continue;
          }
          throw new CallApiError(detail.error || `Call service returned ${response.status}.`, response.status, detail.code);
        }
        if (response.status === 204 || response.headers.get("content-length") === "0") return undefined as T;
        const text = await response.text();
        return (text ? JSON.parse(text) : undefined) as T;
      }
    } finally { window.clearTimeout(timer); }
  }

  private async prepareJoin(started: number) {
    let microphoneMs = 0;
    const capture = this.openMicrophone(this.microphoneDeviceId).then((track) => {
      microphoneMs = performance.now() - started;
      return track;
    });
    const issuedAt = performance.now();
    // An offer needs a transceiver, not audio. Create it while the microphone
    // opens so session creation and publication share one request. TURN servers
    // arrive with that response and are configured before the offer is applied.
    const pc = this.makePeerConnection([]);
    const offering = (async () => {
      const transceiver = pc.addTransceiver("audio", { direction: "sendonly", streams: [new MediaStream([])] });
      preferOpus(transceiver);
      const offer = await pc.createOffer();
      const mid = /^a=mid:(\S+)/m.exec(offer.sdp ?? "")?.[1];
      if (!mid) throw new Error("The browser did not assign a media identifier.");
      const joined = await this.joinWithOffer(offer, mid);
      return { joined, publication: { pc, transceiver, offer, mid }, sessionMs: performance.now() - started };
    })();
    const [captureResult, joinResult] = await Promise.allSettled([capture, offering]);
    if (captureResult.status === "rejected" || joinResult.status === "rejected") {
      pc.close();
      if (captureResult.status === "fulfilled") this.stopMicrophone(captureResult.value);
      if (joinResult.status === "fulfilled") void this.api("leave", {}, joinResult.value.joined.token).catch(() => undefined);
      throw captureResult.status === "rejected" ? captureResult.reason : joinResult.status === "rejected" ? joinResult.reason : new Error("Join preparation failed.");
    }
    return { microphone: captureResult.value, ...joinResult.value, microphoneMs, issuedAt };
  }

  private async joinWithOffer(offer: RTCSessionDescriptionInit, mid: string) {
    const body = { name: this.name, muted: this.muted, deafened: this.deafened };
    try {
      return await this.api<JoinResponse>("join", { ...body, publish: { mid, sessionDescription: { type: "offer", sdp: offer.sdp } } }, undefined);
    } catch (error) {
      // An API without combined publication rejects the unknown field during
      // deserialization, before any mutation. Join the original way instead.
      if (!(error instanceof CallApiError && error.status === 422)) throw error;
      return await this.api<JoinResponse>("join", body, undefined);
    }
  }

  async join(name = "Guest", microphoneDeviceId?: string) {
    if (this.phase !== "idle" && this.phase !== "failed") return;
    this.stopLocalMicTest();
    const started = performance.now();
    this.name = name.trim();
    if (microphoneDeviceId !== undefined) this.microphoneDeviceId = microphoneDeviceId || undefined;
    this.phase = "joining";
    this.emit();
    const generation = ++this.generation;
    let capturedMicrophone: MediaStreamTrack | undefined;
    try {
      const prepared = await this.prepareJoin(started);
      capturedMicrophone = prepared.microphone;
      if (generation !== this.generation || this.phase !== "joining") {
        prepared.publication.pc.close();
        this.stopMicrophone(prepared.microphone);
        void this.api("leave", {}, prepared.joined.token).catch(() => undefined);
        return;
      }
      await this.connectPrepared(prepared, generation, started, "Joined");
      this.reconnects = 0;
    } catch (error) {
      if (capturedMicrophone && !this.senders.has("microphone")) this.stopMicrophone(capturedMicrophone);
      if (generation !== this.generation) return;
      // Local teardown is synchronous; provider cleanup must not delay the
      // failed state or a subsequent explicit Join.
      void this.teardown(false);
      if (generation !== this.generation) return;
      this.phase = "failed";
      this.emit(message(error));
    }
  }

  private async connectPrepared(
    { microphone, joined, publication, issuedAt, microphoneMs, sessionMs }: Awaited<ReturnType<PublicCallClient["prepareJoin"]>>,
    generation: number, started: number, label: string,
  ) {
    const prepared = performance.now();
    const signal = this.captureController.signal;
    this.token = joined.token;
    this.selfId = joined.id;
    const pc = this.pc = publication.pc;
    let iceConnected: number | undefined;
    const iceChanged = () => {
      if (iceConnected === undefined && (pc.iceConnectionState === "connected" || pc.iceConnectionState === "completed")) iceConnected = performance.now();
    };
    pc.addEventListener("iceconnectionstatechange", iceChanged);
    pc.setConfiguration({ ...pc.getConfiguration(), iceServers: joined.iceServers });
    // Negotiate with a disabled track while the independent event stream opens.
    // All startup promises have rejection handlers before any can fail.
    const [events] = await Promise.all([
      this.openEvents(generation),
      this.publishMicrophone(microphone, publication, joined.publish, generation),
      // Reconcile any intent changed during capture/join, without waiting for SDP.
      this.setState(),
    ]);
    signal.throwIfAborted();
    const signaled = performance.now();
    // The roster request renews the lease consumed by signaling. It needs no
    // transport, so overlap it with ICE. Pulls must wait: Cloudflare holds
    // tracks/new for a listener whose PeerConnection is not connected, then
    // answers 425 "Session is not ready yet" (live-verified, about 11 s).
    const leased = this.renewLease();
    leased.catch(() => undefined);
    await waitFor(pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => pc.connectionState === "connected", signal);
    const connected = performance.now();
    pc.removeEventListener("iceconnectionstatechange", iceChanged);
    if (await leased) await this.poll();
    if (this.stateDirty) await this.setState();
    if (this.pollAgain) await this.poll();
    signal.throwIfAborted();
    if (generation !== this.generation || !events.connected || microphone.readyState !== "live") {
      throw new Error("Voice setup changed before it was ready. Please join again.");
    }
    // Applying a subscription offer can legitimately move an established
    // transport back through `connecting`. Keep audio closed until that
    // negotiation restores connectivity instead of treating the transition as
    // a changed session.
    await waitFor(pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => pc.connectionState === "connected", signal);
    signal.throwIfAborted();
    if (generation !== this.generation || !events.connected || pc.connectionState !== "connected" || microphone.readyState !== "live") {
      throw new Error("Voice setup changed before it was ready. Please join again.");
    }
    this.phase = "connected";
    // Monitoring has already detached the public sender; only its private return uses audio.
    microphone.enabled = this.monitoring || !this.muted;
    this.joinTiming = {
      join: `${label} in ${(performance.now() - started).toFixed(0)} ms`,
      microphoneMs,
      sessionMs,
      signalingMs: signaled - prepared,
      transportMs: connected - signaled,
      iceMs: iceConnected === undefined ? undefined : Math.max(0, iceConnected - signaled),
      rosterMs: performance.now() - connected,
    };
    this.emit();
    this.startPolling();
    if (joined.turn) {
      this.turnRenewal = new TurnRenewal(pc, joined.token, joined.turn,
        (operation, body, token, owner) => this.api(operation, body, token, FETCH_TIMEOUT_MS, owner),
        (operation) => this.serialize(operation, generation),
        () => generation === this.generation && pc === this.pc,
        () => { if (generation === this.generation) this.scheduleReconnect(); }, issuedAt);
    }
  }

  private async openEvents(generation: number) {
    this.events?.stop();
    const events = this.events = new EventConnection(() => {
      if (generation !== this.generation) return;
      ++this.snapshotInvalidation;
      if (this.phase === "connected") void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
      else this.pollAgain = true;
    }, (error, draining) => {
      if (generation !== this.generation) return;
      if (this.phase === "connected") this.scheduleEventRecovery(generation, draining);
      else this.captureController.abort(error); // Startup still fails closed.
    }, (snapshot) => {
      if (generation !== this.generation) return;
      this.queueSnapshot(snapshot);
      if (this.phase === "connected") void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
    }, () => {
      if (generation === this.generation && this.phase === "connected") this.scheduleEventRecovery(generation, true);
    }, this.apiRoot, () => {
      if (generation === this.generation && this.phase === "connected") this.emit();
    });
    await events.open(this.token!, this.captureController.signal);
    if (generation === this.generation && events === this.events && events.connected) {
      this.eventRetryAttempts = 0;
      this.eventDrainRetries = 0;
      if (this.phase === "connected") this.emit();
    }
    return events;
  }

  private scheduleEventRecovery(generation: number, draining = false) {
    // Reconnect control updates with the same capability, not a new voice session.
    // The authenticated heartbeat owns session validity. Losing gateway updates alone must
    // not tear down healthy audio while those renewals still succeed.
    this.emit();
    window.clearTimeout(this.eventRetryTimer);
    // Keep explicit routing rejections fast, but do not hammer an unavailable
    // deployment indefinitely. Ordinary failures still use exponential backoff.
    const delay = draining && this.eventDrainRetries++ < 10 ? 50 : Math.min(250 * 2 ** this.eventRetryAttempts++, 3_000);
    this.eventRetryTimer = window.setTimeout(() => {
      if (generation !== this.generation || this.phase !== "connected") return;
      void this.openEvents(generation).then(() => {
        if (generation === this.generation) return this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
      }).catch(() => { /* Stream failures schedule their own retry; heartbeat validates the session. */ });
    }, delay);
  }

  private queueSnapshot(snapshot: CallSnapshot & { revision?: number }) {
    if (snapshot.revision !== undefined && this.latestRevision !== undefined && snapshot.revision < this.latestRevision) return;
    if (snapshot.revision !== undefined) this.latestRevision = snapshot.revision;
    this.pendingSnapshot = snapshot;
    this.pushedSnapshotVersion++;
    this.pollAgain = true;
    // Roster presentation is independent of slow SDP work and our own writes.
    this.participants = snapshot.participants;
    if (this.phase === "connected") {
      const self = snapshot.participants.find((participant) => participant.id === this.selfId);
      // A timed-out write can still commit later. Reassert current intent if a
      // newer server snapshot disagrees; never change the local microphone to it.
      if (self && (self.muted !== this.muted || self.deafened !== this.deafened)) {
        void this.setState().catch((error) => this.emit(message(error)));
      }
      this.emit();
    }
  }

  private get readyToTalk() {
    // Initial setup requires gateway readiness; an established call can tolerate its
    // bounded recovery window without silencing a healthy media transport.
    return this.phase === "connected" && this.pc?.connectionState === "connected";
  }

  private makePeerConnection(iceServers: RTCIceServer[]) {
    const pc = new RTCPeerConnection({ iceServers, bundlePolicy: "max-bundle" });
    pc.ontrack = (event) => {
      if (pc !== this.pc) { event.track.stop(); return; }
      const transceiver = event.transceiver;
      const found = [...this.subscriptions].find(([, mid]) => mid === transceiver.mid);
      if (!found) return;
      const [trackId] = found;
      const participant = this.participants.find((item) => item.tracks.some((track) => track.id === trackId));
      const kind = participant?.tracks.find((track) => track.id === trackId)?.kind;
      if (!participant || !kind) return;
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      this.remoteMedia.set(trackId, { trackId, participantId: participant.id, kind, stream });
      event.track.onended = () => { if (pc === this.pc) { this.remoteMedia.delete(trackId); this.emit(); } };
      this.emit();
    };
    pc.onconnectionstatechange = () => {
      if (pc !== this.pc) return;
      if (this.phase !== "connected") return;
      if (pc.connectionState === "connected") {
        window.clearTimeout(this.disconnectTimer);
        this.disconnectTimer = undefined;
        const microphone = this.senders.get("microphone");
        if (microphone) microphone.track.enabled = this.monitoring || !this.muted;
      } else if (pc.connectionState === "failed") this.scheduleReconnect();
      else if (this.disconnectTimer === undefined) {
        // Intermediate states must not cancel or extend an existing deadline.
        this.disconnectTimer = window.setTimeout(() => {
          if (pc === this.pc && pc.connectionState !== "connected") this.scheduleReconnect();
        }, DISCONNECT_GRACE_MS);
      }
    };
    return pc;
  }

  private async publishMicrophone(
    track: MediaStreamTrack,
    { pc, transceiver, offer, mid }: PreparedPublication,
    published: SessionDescriptionResponse | undefined,
    generation = this.generation,
  ) {
    const kind: MediaKind = "microphone";
    return this.serialize(async () => {
      if (pc !== this.pc) throw new Error("Call session changed.");
      const token = this.token;
      // Attach before the answer so the transport never connects without the
      // (still disabled) microphone, exactly as when the track was added first.
      track.enabled = this.readyToTalk && !this.muted;
      await transceiver.sender.replaceTrack(this.muted || this.monitoring ? null : track);
      await pc.setLocalDescription(offer);
      if (generation !== this.generation) throw new Error("Call session changed.");
      this.senders.set(kind, { sender: transceiver.sender, mid, track });
      const response = published ?? await this.api<SessionDescriptionResponse>("publish", {
        kind, mid, sessionDescription: await localDescription(pc, this.captureController.signal),
      }, token);
      if (generation !== this.generation || pc !== this.pc) throw new Error("Call session changed.");
      if (!response.sessionDescription) throw new Error("The media service did not answer publication.");
      await pc.setRemoteDescription(withOpusDtx(response.sessionDescription));
      if (generation !== this.generation) throw new Error("Call session changed.");
      this.localMedia = new MediaStream([track]);
      track.addEventListener("ended", () => {
        if (generation === this.generation && this.senders.get(kind)?.track === track) {
          void this.unpublish(kind).catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
        }
      }, { once: true });
      if (this.muted || this.monitoring) track.enabled = this.readyToTalk && this.monitoring;
    }, generation).catch((error) => {
      track.stop();
      if (generation === this.generation && this.phase === "connected") { this.pc?.close(); this.scheduleReconnect(); }
      throw error;
    });
  }

  async setMuted(muted: boolean) {
    if (this.monitoring) return;
    const generation = this.generation;
    if (!muted) {
      this.deafened = false;
      this.mutedBeforeDeafen = undefined;
    }
    this.muted = muted;
    const microphone = this.senders.get("microphone");
    if (microphone) microphone.track.enabled = this.readyToTalk && !muted;
    this.emit();
    await this.serializeMedia(async () => {
      const current = this.senders.get("microphone");
      if (!current) return;
      current.track.enabled = this.readyToTalk && !this.muted;
      await current.sender.replaceTrack(this.muted || !this.readyToTalk ? null : current.track);
    }).catch((error) => { if (generation === this.generation) throw error; });
    if (generation === this.generation && this.token) await this.setState();
  }

  async setDeafened(deafened: boolean) {
    if (this.monitoring) return;
    const generation = this.generation;
    if (deafened && !this.deafened) this.mutedBeforeDeafen = this.muted;
    if (!deafened && this.deafened) {
      this.muted = this.mutedBeforeDeafen ?? false;
      this.mutedBeforeDeafen = undefined;
    }
    if (deafened) this.muted = true;
    this.deafened = deafened;
    const microphone = this.senders.get("microphone");
    if (microphone) microphone.track.enabled = this.readyToTalk && !this.muted;
    this.emit();
    await this.serializeMedia(async () => {
      const current = this.senders.get("microphone");
      if (!current) return;
      current.track.enabled = this.readyToTalk && !this.muted;
      await current.sender.replaceTrack(this.muted || !this.readyToTalk ? null : current.track);
    }).catch((error) => { if (generation === this.generation) throw error; });
    if (generation === this.generation && this.token) await this.setState();
  }

  async setMonitoring(monitoring: boolean) {
    if (monitoring === this.monitoring) return;
    if (monitoring && (this.phase !== "connected" || !this.token)) throw new Error("Join voice before testing your microphone.");
    if (monitoring) {
      this.stateBeforeMonitoring = { muted: this.muted, deafened: this.deafened };
      this.monitoring = true;
      this.muted = true;
      this.deafened = true;
      const microphone = this.senders.get("microphone");
      if (!microphone) throw new Error("No microphone is available for testing.");
      // The test records the locally captured, noise-suppressed audio. It does
      // not need a round trip through the SFU and never publishes this stream.
      this.monitorStream = new MediaStream([this.captures.get(microphone.track)?.naturalTrack ?? microphone.track]);
    } else {
      this.monitorStream = undefined;
      this.monitoring = false;
      this.muted = this.stateBeforeMonitoring?.muted ?? false;
      this.deafened = this.stateBeforeMonitoring?.deafened ?? false;
      this.stateBeforeMonitoring = undefined;
    }

    const microphone = this.senders.get("microphone");
    // Silence immediately, before asynchronous sender detachment.
    if (microphone && monitoring) microphone.track.enabled = false;
    this.emit();
    await this.serializeMedia(async () => {
      const current = this.senders.get("microphone");
      if (!current) return;
      await current.sender.replaceTrack(this.muted || this.monitoring ? null : current.track);
      current.track.enabled = this.monitoring || !this.muted;
    });
    // State synchronization is best-effort: local testing must work even when
    // the channel control plane is temporarily unavailable.
    if (this.token) void this.setState().catch(() => undefined);
  }

  private setState(): Promise<void> {
    this.stateDirty = true;
    ++this.stateRevision;
    if (this.phase === "connected") this.emit();
    if (this.statePromise) return this.statePromise;
    const generation = this.generation;
    const token = this.token;
    // State is idempotent and does not negotiate SDP. Coalesce rapid changes
    // independently of the signaling queue, retrying current intent only.
    const sync = async () => {
      while (this.stateDirty && token && generation === this.generation) {
        const revision = this.stateRevision;
        try {
          await this.api("state", { muted: this.muted, deafened: this.deafened, sequence: revision }, token, SNAPSHOT_TIMEOUT_MS);
        } catch (error) {
          if (generation !== this.generation) return;
          if (this.phase !== "connected" || !transientControlError(error)) throw error;
          window.clearTimeout(this.stateRetryTimer);
          this.stateRetryTimer = window.setTimeout(() => {
            if (generation === this.generation) void this.setState().catch((error) => this.emit(message(error)));
          }, 250);
          return;
        }
        if (generation !== this.generation) return;
        this.stateDirty = revision !== this.stateRevision;
        window.clearTimeout(this.stateRetryTimer);
      }
    };
    const pending = sync().finally(() => {
      if (this.statePromise === pending) this.statePromise = undefined;
      if (generation === this.generation && this.phase === "connected") this.emit();
    });
    this.statePromise = pending;
    return pending;
  }

  private async openMicrophone(deviceId?: string, signal = this.captureController.signal) {
    this.captureAttempt = "opening";
    this.captureError = undefined;
    let microphone: Microphone;
    try {
      microphone = await captureMicrophone(deviceId, this.noiseSuppression, signal, () => {
        this.microphoneStatus = microphone.status;
        this.emit();
      }, this.audioSetup, this.noiseAssets, this.dpdfnet, this.inputVolume, this.voiceProcessingStrength);
      this.captureAttempt = "opened";
    } catch (error) {
      this.captureAttempt = "failed";
      this.captureError = error instanceof Error ? error.name : "UnknownError";
      throw error;
    }
    this.microphoneStatus = microphone.status;
    this.captures.set(microphone.track, microphone);
    return microphone.track;
  }

  getAudioDiagnostics() {
    return {
      captureAttempt: this.captureAttempt, captureError: this.captureError,
      captures: [...this.captures.values()].map((capture) => capture.diagnostics()),
      connection: this.diagnostics,
      voice: this.voiceDiagnostics(),
    };
  }

  private voiceDiagnostics() {
    const sender = this.senders.get("microphone");
    const others = this.participants.filter((participant) => participant.id !== this.selfId);
    return {
      phase: this.phase, muted: this.muted, deafened: this.deafened, monitoring: this.monitoring,
      liveUpdates: !!this.events?.connected,
      peer: this.pc && {
        connection: this.pc.connectionState, ice: this.pc.iceConnectionState, signaling: this.pc.signalingState,
      },
      microphoneSender: sender && {
        attached: !!sender.sender.track, enabled: sender.track.enabled,
        readyState: sender.track.readyState, muted: sender.track.muted,
      },
      participants: this.participants.length,
      otherTracks: others.reduce((count, participant) => count + participant.tracks.length, 0),
      subscriptions: this.subscriptions.size,
      remoteStreams: [...this.remoteMedia.values()].map(({ stream }) => stream.getAudioTracks().map((track) => ({
        readyState: track.readyState, muted: track.muted, enabled: track.enabled,
      }))).flat(),
      media: this.mediaDetail,
    };
  }

  private stopMicrophone(track: MediaStreamTrack) {
    const capture = this.captures.get(track);
    if (capture) { capture.stop(); this.captures.delete(track); }
    else track.stop();
  }

  setInputVolume(volume: number) {
    this.inputVolume = Math.max(0, Math.min(volume, 200));
    this.captures.get(this.localTestTrack ?? this.senders.get("microphone")?.track!)?.setInputVolume(this.inputVolume);
    this.emit();
  }

  setVoiceProcessingStrength(strength: number) {
    this.voiceProcessingStrength = clampVoiceProcessingStrength(strength);
    this.captures.get(this.localTestTrack ?? this.senders.get("microphone")?.track!)?.setVoiceProcessingStrength(this.voiceProcessingStrength);
    this.emit();
  }

  async startLocalMicTest(deviceId?: string) {
    if ((this.phase !== "idle" && this.phase !== "failed") || this.localTestTrack) return;
    this.stopLocalMicTest();
    if (deviceId !== undefined) this.microphoneDeviceId = deviceId || undefined;
    const generation = ++this.localTestGeneration;
    const controller = new AbortController();
    this.localTestController = controller;
    const signal = AbortSignal.any([controller.signal, this.captureController.signal]);
    const timer = window.setTimeout(() => controller.abort(new Error("Microphone setup timed out. Check your browser’s microphone permission and selected input, then try again.")), 30_000);
    let abort = () => {};
    try {
      const cancelled = new Promise<never>((_, reject) => {
        abort = () => reject(signal.reason);
        signal.addEventListener("abort", abort, { once: true });
        if (signal.aborted) abort();
      });
      const opening = this.openMicrophone(this.microphoneDeviceId, signal).then((track) => {
        if (signal.aborted || generation !== this.localTestGeneration) {
          this.stopMicrophone(track);
          throw signal.reason ?? new DOMException("Microphone test cancelled.", "AbortError");
        }
        return track;
      });
      const track = await Promise.race([opening, cancelled]);
      this.localTestTrack = track;
      this.monitorStream = new MediaStream([this.captures.get(track)?.naturalTrack ?? track]);
      this.emit();
    } catch (error) {
      if (generation === this.localTestGeneration) throw new Error(message(error));
    } finally {
      window.clearTimeout(timer);
      signal.removeEventListener("abort", abort);
    }
  }

  stopLocalMicTest() {
    ++this.localTestGeneration;
    this.localTestController?.abort();
    this.localTestController = undefined;
    if (!this.localTestTrack) return;
    this.stopMicrophone(this.localTestTrack);
    this.localTestTrack = undefined;
    this.monitorStream = undefined;
    this.emit();
  }

  async setNoiseSuppression(mode: NoiseSuppression) {
    const previous = this.noiseSuppression;
    this.noiseSuppression = mode;
    try {
      if (this.phase === "connected") await this.changeMicrophone(this.microphoneDeviceId ?? "");
    } catch (error) {
      this.noiseSuppression = previous;
      throw error;
    } finally { this.emit(); }
  }

  async setAudioSetup(setup: AudioSetup) {
    const previous = this.audioSetup;
    this.audioSetup = setup;
    try {
      if (this.phase === "connected") await this.changeMicrophone(this.microphoneDeviceId ?? "");
    } catch (error) {
      this.audioSetup = previous;
      throw error;
    } finally { this.emit(); }
  }

  async changeMicrophone(deviceId: string) {
    if ((this.phase === "idle" || this.phase === "failed") && !this.senders.has("microphone")) {
      this.microphoneDeviceId = deviceId || undefined;
      return;
    }
    const generation = this.generation;
    await this.serializeMedia(async () => {
      const microphone = this.senders.get("microphone");
      if (!microphone) return;
      const old = microphone.track;
      const oldCapture = this.captures.get(old);
      let track: MediaStreamTrack | undefined;
      let senderReplaced = false;
      // Enhanced filters are CPU-heavy. Do not run two pipelines while preparing
      // a replacement; the brief overlap can starve the active worklet and end
      // its track, which escalates an ordinary device change into a full rejoin.
      await oldCapture?.pause();
      try {
        track = await this.openMicrophone(deviceId || undefined);
        if (generation !== this.generation || this.senders.get("microphone") !== microphone) throw new Error("Call session changed.");
        track.enabled = !this.muted;
        await microphone.sender.replaceTrack(this.muted || this.monitoring ? null : track);
        senderReplaced = true;
        if (generation !== this.generation) throw new Error("Call session changed.");
        track.enabled = this.monitoring || !this.muted;
        if (this.monitoring) this.monitorStream = new MediaStream([this.captures.get(track)?.naturalTrack ?? track]);
        if (generation !== this.generation) throw new Error("Call session changed.");
        microphone.track = track;
        this.localMedia = new MediaStream([track]);
        this.microphoneDeviceId = deviceId || undefined;
        this.stopMicrophone(old);
        this.emit();
      } catch (error) {
        if (generation !== this.generation) {
          if (track) this.stopMicrophone(track);
          throw error;
        }
        let rollbackFailed = false;
        try { await oldCapture?.resume(); } catch { rollbackFailed = true; }
        if (senderReplaced && generation === this.generation && this.senders.get("microphone") === microphone) {
          try {
            await microphone.sender.replaceTrack(this.muted || this.monitoring ? null : old);
            if (this.monitoring) this.monitorStream = new MediaStream([oldCapture?.naturalTrack ?? old]);
          } catch { rollbackFailed = true; }
        }
        if (track) this.stopMicrophone(track);
        if (rollbackFailed && generation === this.generation) this.scheduleReconnect();
        throw error;
      }
    }, generation);
  }

  /** Resolves false when the source exists but its track cannot be pulled yet. */
  private subscribe(trackId: string) {
    const generation = this.generation;
    return this.serialize(async () => {
      if (this.subscriptions.has(trackId)) return true;
      const pc = this.requirePc();
      const token = this.token;
      let response: SessionDescriptionResponse;
      try { response = await this.api<SessionDescriptionResponse>("subscribe", { trackId }, token); }
      catch (error) {
        // Ordinary departure between roster delivery and subscription. The API
        // authenticated us and rejected before touching the provider/SDP.
        if (error instanceof CallApiError && error.status === 404 && error.code === "track_gone") return false;
        throw error;
      }
      if (generation !== this.generation || pc !== this.pc) return true;
      const mid = response.tracks?.[0]?.mid;
      if (!mid) throw new Error("The media service did not return a subscription identifier.");
      this.subscriptions.set(trackId, mid);
      if (response.sessionDescription) {
        await pc.setRemoteDescription(withOpusDtx(response.sessionDescription));
        await pc.setLocalDescription(await pc.createAnswer());
        await this.api("negotiate", { sessionDescription: await localDescription(pc, this.captureController.signal) }, token);
      } else if (response.requiresImmediateRenegotiation) {
        throw new Error("The media service requested negotiation without an offer.");
      }
      return true;
    }, generation);
  }

  /** Resolves false when some sources cannot be pulled yet; "unsupported" for an API without batches. */
  private subscribeMany(trackIds: string[]) {
    const generation = this.generation;
    return this.serialize(async (): Promise<boolean | "unsupported"> => {
      const wanted = trackIds.filter((id) => !this.subscriptions.has(id));
      if (!wanted.length) return true;
      const pc = this.requirePc();
      const token = this.token;
      let response: BatchSubscribeResponse;
      try { response = await this.api<BatchSubscribeResponse>("subscribe", { trackIds: wanted }, token); }
      catch (error) {
        if (error instanceof CallApiError && error.status === 404 && error.code === "track_gone") return false;
        // An API without batches rejects the field during deserialization, before any mutation.
        if (error instanceof CallApiError && error.status === 422) {
          this.batchUnsupported = true;
          return "unsupported";
        }
        throw error;
      }
      if (generation !== this.generation || pc !== this.pc) return true;
      // Map every allocated MID before applying the offer; ontrack uses it.
      for (const { trackId, mid } of response.tracks ?? []) this.subscriptions.set(trackId, mid);
      if (response.sessionDescription) {
        await pc.setRemoteDescription(withOpusDtx(response.sessionDescription));
        await pc.setLocalDescription(await pc.createAnswer());
        await this.api("negotiate", { sessionDescription: await localDescription(pc, this.captureController.signal) }, token);
      } else if (response.requiresImmediateRenegotiation) {
        throw new Error("The media service requested negotiation without an offer.");
      }
      return !response.gone?.length;
    }, generation);
  }

  private unsubscribe(trackIds: string[]) {
    const generation = this.generation;
    return this.serialize(async () => {
      const departed = trackIds.flatMap((trackId) => {
        const mid = this.subscriptions.get(trackId);
        return mid ? [{ trackId, mid }] : [];
      });
      if (!departed.length) return;
      // A departed source is no longer playable, regardless of cleanup latency.
      for (const { trackId } of departed) {
        this.remoteMedia.get(trackId)?.stream.getTracks().forEach((track) => track.stop());
        this.remoteMedia.delete(trackId);
      }
      this.emit();
      // Force-close negotiates no SDP and each close is one atomic server update,
      // so several departures cost one round trip rather than one each.
      const results = await Promise.allSettled(departed.map(({ mid }) => this.closeMid(mid)));
      if (generation !== this.generation) return;
      let failure: unknown;
      results.forEach((result, index) => {
        if (result.status === "fulfilled") this.subscriptions.delete(departed[index].trackId);
        // Keep healthy media and retry a transiently failed MID on the next
        // roster reconciliation/lease heartbeat, not a new join.
        else if (!transientControlError(result.reason)) failure ??= result.reason;
      });
      if (failure) throw failure;
    }, generation);
  }

  private async unpublish(kind: MediaKind) {
    const generation = this.generation;
    await this.serialize(async () => {
      const publication = this.senders.get(kind);
      if (!publication) return;
      const token = this.token;
      publication.track.stop();
      await publication.sender.replaceTrack(null);
      await this.closeMid(publication.mid, token);
      if (generation !== this.generation || this.senders.get(kind) !== publication) return;
      publication.track.stop();
      this.senders.delete(kind);
      if (kind === "microphone") this.localMedia = undefined;
    }, generation);
    this.emit();
  }

  private async closeMid(mid: string, token = this.token) {
    try { await this.api("close", { mid }, token, SNAPSHOT_TIMEOUT_MS); } catch (error) {
      if (error instanceof CallApiError && error.status === 404) return;
      throw error;
    }
  }

  private startPolling() {
    window.clearInterval(this.pollTimer);
    window.clearInterval(this.statsTimer);
    // Gateway snapshots handle discovery; command snapshots still renew the lease and repair missed state.
    const generation = this.generation;
    if (this.pollAgain) void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
    this.pollTimer = window.setInterval(() => void this.poll(true).catch(() => { if (generation === this.generation) this.scheduleReconnect(); }), 15_000);
    void this.readStats();
    this.statsTimer = window.setInterval(() => void this.readStats(), 1_000);
  }

  private async readStats() {
    const pc = this.pc;
    if (!pc || this.phase !== "connected") return;
    try {
      const report = await pc.getStats();
      if (pc !== this.pc) return;
      let received = 0, sent = 0, lost = 0, jitter = 0, rtt = 0, relay = false;
      // Per-stream counters separate "nothing arrives" from "arrives but is silent".
      const outbound: object[] = [], inbound: object[] = [], sources: object[] = [];
      const level = (value: unknown) => typeof value === "number" ? Number(value.toFixed(4)) : undefined;
      report.forEach((stat) => {
        if (stat.type === "outbound-rtp") {
          sent += stat.bytesSent ?? 0;
          outbound.push({ mid: stat.mid, packetsSent: stat.packetsSent, bytesSent: stat.bytesSent, active: stat.active });
        }
        if (stat.type === "media-source" && stat.kind === "audio") {
          sources.push({ audioLevel: level(stat.audioLevel), totalAudioEnergy: level(stat.totalAudioEnergy) });
        }
        if (stat.type === "inbound-rtp") {
          received += stat.bytesReceived ?? 0;
          lost += stat.packetsLost ?? 0;
          jitter = Math.max(jitter, stat.jitter ?? 0);
          inbound.push({
            mid: stat.mid, packetsReceived: stat.packetsReceived, bytesReceived: stat.bytesReceived,
            audioLevel: level(stat.audioLevel), totalSamplesReceived: stat.totalSamplesReceived,
            concealedSamples: stat.concealedSamples,
          });
        }
        if (stat.type === "candidate-pair" && stat.state === "succeeded" && stat.nominated) {
          rtt = Math.max(rtt, stat.currentRoundTripTime ?? 0);
          // Unanswered checks before connecting show retransmission delay in setup.
          if (this.joinTiming && this.joinTiming.checks === undefined && typeof stat.requestsSent === "number") {
            this.joinTiming.checks = `${stat.requestsSent} sent · ${stat.responsesReceived ?? 0} answered`;
          }
          relay ||= report.get(stat.localCandidateId)?.candidateType === "relay";
        }
      });
      const sampledAt = performance.now();
      const elapsed = sampledAt - (this.previousStats?.sampledAt ?? sampledAt);
      const receiveBitrate = elapsed > 0 ? Math.max(0, (received - (this.previousStats?.received ?? received)) * 8_000 / elapsed) : 0;
      const sendBitrate = elapsed > 0 ? Math.max(0, (sent - (this.previousStats?.sent ?? sent)) * 8_000 / elapsed) : 0;
      this.previousStats = { received, sent, sampledAt };
      this.mediaDetail = { outbound, sources, inbound };
      if (!this.joinTiming) return;
      this.diagnostics = {
        ...this.joinTiming,
        receivedBytes: received,
        sentBytes: sent,
        receiveBitrate,
        sendBitrate,
        packetsLost: lost,
        maxJitterMs: jitter * 1_000,
        roundTripMs: rtt * 1_000,
        route: relay ? "relay" : rtt > 0 ? "direct" : "unknown",
      };
      this.emit();
    } catch { /* Stats support varies; diagnostics must never interrupt media. */ }
  }

  private renewLease(): Promise<boolean> {
    if (this.snapshotPromise) return this.snapshotPromise;
    const generation = this.generation;
    const started = performance.now();
    const invalidation = this.snapshotInvalidation;
    const pushedVersion = this.pushedSnapshotVersion;
    const pending = this.api<CallSnapshot & { revision?: number }>("snapshot", {}, this.token, SNAPSHOT_TIMEOUT_MS).then((snapshot) => {
      if (generation !== this.generation) return false;
      window.clearTimeout(this.pollRetryTimer);
      this.controlFailedSince = undefined;
      if (invalidation !== this.snapshotInvalidation) this.pollAgain = true;
      else if (pushedVersion === this.pushedSnapshotVersion || snapshot.revision !== undefined) this.queueSnapshot(snapshot);
      return true;
    }).catch((error) => {
      if (generation !== this.generation) return false;
      if (this.phase !== "connected") throw error;
      this.controlFailedSince ??= started;
      if (!transientControlError(error) || performance.now() - this.controlFailedSince >= CONTROL_RECOVERY_MS) {
        this.scheduleReconnect();
      } else {
        window.clearTimeout(this.pollRetryTimer);
        this.pollRetryTimer = window.setTimeout(() => {
          if (generation === this.generation) void this.poll(true).catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
        }, 3_000);
      }
      return false;
    }).finally(() => {
      if (this.snapshotPromise === pending) this.snapshotPromise = undefined;
    });
    this.snapshotPromise = pending;
    return pending;
  }

  private poll(heartbeat = false): Promise<void> {
    if (this.phase !== "connected" && this.phase !== "joining") return Promise.resolve();
    // Renew even while reconciliation is waiting on a slow media operation.
    if (heartbeat) {
      const generation = this.generation;
      const renewal = this.snapshotPromise
        ? this.snapshotPromise.then((renewed) => renewed && generation === this.generation ? this.renewLease() : false)
        : this.renewLease();
      return renewal.then((renewed) => renewed && generation === this.generation ? this.poll() : undefined);
    }
    this.pollAgain = true;
    if (this.pollPromise) return this.pollPromise;
    const generation = this.generation;
    const refresh = async () => {
      while (this.pollAgain && generation === this.generation) {
        this.pollAgain = false;
        const snapshot = this.pendingSnapshot;
        this.pendingSnapshot = undefined;
        if (!snapshot) {
          if (!await this.renewLease()) return;
          continue;
        }
        const pushedVersion = this.pushedSnapshotVersion;
        if (generation !== this.generation) return;
        if (snapshot.revision !== undefined && this.latestRevision !== undefined && snapshot.revision < this.latestRevision) continue;
        if (snapshot.revision !== undefined) this.latestRevision = snapshot.revision;
        if (pushedVersion !== this.pushedSnapshotVersion) continue;
        // ontrack uses the roster to associate arriving media with its owner.
        this.participants = snapshot.participants;
        const available = new Set(snapshot.participants.flatMap((participant) => participant.tracks.map((track) => track.id)));
        const departed = [...this.subscriptions.keys()].filter((id) => !available.has(id));
        if (departed.length) await this.unsubscribe(departed);
        if (generation !== this.generation) return;
        let unavailable = false;
        const missing = snapshot.participants
          .filter((participant) => participant.id !== this.selfId)
          .flatMap((participant) => participant.tracks.map((track) => track.id))
          .filter((id) => !this.subscriptions.has(id));
        // Several sources share one provider request and one SDP exchange.
        const batched = missing.length > 1 && !this.batchUnsupported ? await this.subscribeMany(missing) : "unsupported";
        if (batched === "unsupported") {
          for (const participant of snapshot.participants) {
            if (participant.id === this.selfId) continue;
            for (const track of participant.tracks) {
              if (generation !== this.generation) return;
              if (pushedVersion !== this.pushedSnapshotVersion) break;
              if (!this.subscriptions.has(track.id) && !await this.subscribe(track.id)) unavailable = true;
            }
          }
        } else if (!batched) unavailable = true;
        if (pushedVersion !== this.pushedSnapshotVersion) continue;
        this.retryUnavailable(unavailable, generation);
        if (generation === this.generation) {
          this.emit();
        }
      }
    };
    const pending = refresh().finally(() => {
      if (this.pollPromise === pending) this.pollPromise = undefined;
    });
    this.pollPromise = pending;
    return pending;
  }

  private retryUnavailable(unavailable: boolean, generation: number) {
    // A newly published source can be listed before it sends media, and the
    // provider refuses pulls until then. Re-check soon rather than waiting for
    // the next roster change or lease heartbeat; a departed source drops out.
    window.clearTimeout(this.unavailableRetryTimer);
    if (!unavailable || this.unavailableRetries >= PULL_RETRY_DELAYS_MS.length) {
      if (!unavailable) this.unavailableRetries = 0;
      return;
    }
    const delay = PULL_RETRY_DELAYS_MS[this.unavailableRetries++];
    this.unavailableRetryTimer = window.setTimeout(() => {
      if (generation !== this.generation) return;
      void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
    }, delay);
  }

  private scheduleReconnect() {
    if (this.phase !== "connected" && this.phase !== "joining") return;
    this.phase = "reconnecting";
    ++this.generation;
    this.captureController.abort(new Error("Voice connection interrupted."));
    this.events?.stop();
    this.emit();
    window.clearTimeout(this.reconnectTimer);
    this.reconnectTimer = window.setTimeout(() => void this.rejoin(), Math.min(1_000 * 2 ** this.reconnects, 5_000));
  }

  private async rejoin() {
    if (++this.reconnects > MAX_REJOINS) {
      void this.teardown(true);
      this.resetMonitoring();
      this.phase = "failed";
      this.emit("Connection lost. Please join again.");
      return;
    }
    const generation = ++this.generation;
    // Start old-capability cleanup without putting the replacement session
    // behind provider latency. teardown has already stopped local media here.
    void this.teardown(false);
    if (generation !== this.generation) return;
    this.phase = "joining";
    this.emit();
    const started = performance.now();
    let captured: MediaStreamTrack | undefined;
    try {
      const prepared = await this.prepareJoin(started);
      captured = prepared.microphone;
      if (generation !== this.generation) {
        prepared.publication.pc.close();
        this.stopMicrophone(prepared.microphone);
        void this.api("leave", {}, prepared.joined.token).catch(() => undefined);
        return;
      }
      await this.connectPrepared(prepared, generation, started, "Rejoined");
      this.reconnects = 0;
      if (this.monitoring) {
        const microphone = this.senders.get("microphone");
        if (microphone) this.monitorStream = new MediaStream([this.captures.get(microphone.track)?.naturalTrack ?? microphone.track]);
        this.emit();
      }
    } catch {
      if (captured) this.stopMicrophone(captured);
      if (generation !== this.generation) return;
      // Stop the failed attempt before scheduling another one; its leave can
      // finish independently of the next generation.
      void this.teardown(false);
      this.scheduleReconnect();
    }
  }

  async leave() {
    if (this.phase === "idle") return;
    this.leaveImmediately();
    this.emit();
    this.prepareMicrophone();
  }

  leaveImmediately() {
    ++this.generation;
    this.phase = "idle";
    this.resetMonitoring();
    this.participants = [];
    this.selfId = undefined;
    // Teardown stops local media synchronously. Provider cleanup owns only the
    // captured old token and must not hold up Leave or the next Join.
    void this.teardown(false);
    this.dpdfnet.stop();
  }

  private teardown(skipLeave: boolean, preserve?: MediaStreamTrack) {
    window.clearInterval(this.pollTimer);
    window.clearTimeout(this.reconnectTimer);
    const token = this.token;
    for (const publication of this.senders.values()) if (publication.track !== preserve) publication.track.stop();
    this.stopEverything(preserve);
    return !skipLeave && token ? this.api("leave", {}, token).catch(() => undefined) : Promise.resolve();
  }

  private stopEverything(preserve?: MediaStreamTrack) {
    this.turnRenewal?.stop();
    this.turnRenewal = undefined;
    window.clearTimeout(this.disconnectTimer);
    this.disconnectTimer = undefined;
    window.clearTimeout(this.pollRetryTimer);
    window.clearTimeout(this.unavailableRetryTimer);
    this.unavailableRetries = 0;
    window.clearTimeout(this.eventRetryTimer);
    window.clearTimeout(this.stateRetryTimer);
    this.statePromise = undefined;
    this.snapshotPromise = undefined;
    this.queue = Promise.resolve();
    this.mediaQueue = Promise.resolve();
    this.eventRetryAttempts = 0;
    this.eventDrainRetries = 0;
    this.controlFailedSince = undefined;
    this.stateDirty = false;
    this.captureController.abort();
    this.events?.stop();
    this.events = undefined;
    this.captureController = new AbortController();
    for (const capture of this.captures.values()) capture.stop();
    this.captures.clear();
    window.clearInterval(this.statsTimer);
    this.diagnostics = undefined;
    this.previousStats = undefined;
    this.mediaDetail = undefined;
    this.joinTiming = undefined;
    this.microphoneStatus = undefined;
    this.monitorStream = undefined;
    this.localTestTrack = undefined;
    this.pc?.getReceivers().forEach((receiver) => receiver.track.stop());
    this.pc?.getSenders().forEach((sender) => { if (sender.track !== preserve) sender.track?.stop(); });
    for (const publication of this.senders.values()) if (publication.track !== preserve) publication.track.stop();
    this.pc?.close();
    this.pc = undefined; this.token = undefined;
    this.senders.clear(); this.subscriptions.clear(); this.remoteMedia.clear(); this.localMedia = undefined; this.pollPromise = undefined; this.pollAgain = false;
    this.pendingSnapshot = undefined; this.pushedSnapshotVersion = 0; this.latestRevision = undefined;
  }

  private resetMonitoring() {
    if (!this.monitoring) {
      this.stopLocalMicTest();
      return;
    }
    this.monitoring = false;
    this.muted = this.stateBeforeMonitoring?.muted ?? false;
    this.deafened = this.stateBeforeMonitoring?.deafened ?? false;
    this.stateBeforeMonitoring = undefined;
    this.monitorStream = undefined;
    this.localTestTrack = undefined;
  }

  private requirePc() {
    if (!this.pc) throw new Error("There is no active media session.");
    return this.pc;
  }
}
