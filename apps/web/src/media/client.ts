import { captureMicrophone, type AudioSetup, type Microphone, type NoiseSuppression } from "./microphone.ts";
import { NoiseAssets } from "./noise-assets.ts";
import { DpdfnetPreparation } from "./dpdfnet-preparation.ts";
import { EventConnection } from "./event-connection.ts";
import { localDescription, preferOpus, waitFor, withOpusDtx } from "./rtc.ts";
import { TurnRenewal } from "./turn-renewal.ts";
import { clampVoiceProcessingStrength, DEFAULT_VOICE_PROCESSING_STRENGTH } from "./voice-processing.ts";
export { waitFor } from "./rtc.ts";
import type {
  CallSnapshot,
  ConnectionDiagnostics,
  CallViewState,
  JoinResponse,
  MediaKind,
  Participant,
  RemoteMedia,
  SessionDescriptionResponse,
} from "./types";

const API_ROOT = "/api/media";
const CONNECT_TIMEOUT_MS = 12_000;
const MAX_REJOINS = 3;
const FETCH_TIMEOUT_MS = 25_000;
const SNAPSHOT_TIMEOUT_MS = 5_000;
const CONTROL_RECOVERY_MS = 30_000;
const DISCONNECT_GRACE_MS = 10_000;

class CallApiError extends Error {
  readonly status: number;
  readonly code?: string;
  constructor(message: string, status: number, code?: string) { super(message); this.status = status; this.code = code; }
}

function message(error: unknown) {
  if (error instanceof DOMException && error.name === "NotAllowedError") {
    return "Microphone permission was denied. Allow access and try again.";
  }
  return error instanceof Error ? error.message : "The call could not continue.";
}

function transientControlError(error: unknown) {
  return error instanceof CallApiError
    ? error.status === 408 || error.status === 429 || error.status >= 500
    : error instanceof TypeError || (error instanceof DOMException && error.name === "AbortError");
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
  private inputVolume = 100;
  private monitoring = false;
  private monitorStream?: MediaStream;
  private localTestTrack?: MediaStreamTrack;
  private localTestGeneration = 0;
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
  private noiseSuppression: NoiseSuppression = "dpdfnet8";
  private audioSetup: AudioSetup = "headphones";
  private voiceProcessingStrength = DEFAULT_VOICE_PROCESSING_STRENGTH;
  private microphoneStatus?: string;
  private captures = new Map<MediaStreamTrack, Microphone>();
  private captureController = new AbortController();
  private joinTiming?: Omit<ConnectionDiagnostics, "receivedBytes" | "sentBytes" | "receiveBitrate" | "sendBitrate" | "packetsLost" | "maxJitterMs" | "roundTripMs" | "route">;
  private readonly noiseAssets = new NoiseAssets();
  private readonly dpdfnet = new DpdfnetPreparation();

  private readonly changed: (state: CallViewState) => void;
  constructor(changed: (state: CallViewState) => void) { this.changed = changed; }

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
      remoteMedia: [...this.remoteMedia.values()],
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
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), timeout);
    const owned = operation === "join" || operation === "leave" ? [controller.signal] : [controller.signal, this.captureController.signal];
    if (ownerSignal) owned.push(ownerSignal);
    const signal = owned.length === 1 ? owned[0] : AbortSignal.any(owned);
    try {
      for (let attempt = 0; ; attempt++) {
        signal.throwIfAborted();
        const response = await fetch(`${API_ROOT}/${operation}`, {
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

  private async prepareJoin() {
    const capture = this.openMicrophone(this.microphoneDeviceId);
    const issuedAt = performance.now();
    const joining = this.api<JoinResponse>("join", { name: this.name, muted: this.muted, deafened: this.deafened }, undefined);
    const [captureResult, joinResult] = await Promise.allSettled([capture, joining]);
    if (captureResult.status === "rejected" || joinResult.status === "rejected") {
      if (captureResult.status === "fulfilled") this.stopMicrophone(captureResult.value);
      if (joinResult.status === "fulfilled") void this.api("leave", {}, joinResult.value.token).catch(() => undefined);
      throw captureResult.status === "rejected" ? captureResult.reason : joinResult.status === "rejected" ? joinResult.reason : new Error("Join preparation failed.");
    }
    return { microphone: captureResult.value, joined: joinResult.value, issuedAt };
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
      const { microphone, joined, issuedAt } = await this.prepareJoin();
      capturedMicrophone = microphone;
      if (generation !== this.generation || this.phase !== "joining") {
        this.stopMicrophone(microphone);
        void this.api("leave", {}, joined.token).catch(() => undefined);
        return;
      }
      await this.connectPrepared(microphone, joined, generation, started, "Joined", issuedAt);
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

  private async connectPrepared(microphone: MediaStreamTrack, joined: JoinResponse, generation: number, started: number, label: string, issuedAt: number) {
    const prepared = performance.now();
    const signal = this.captureController.signal;
    this.token = joined.token;
    this.selfId = joined.id;
    const pc = this.pc = this.makePeerConnection(joined.iceServers);
    // Negotiate with a disabled track while the independent event stream opens.
    // All startup promises have rejection handlers before any can fail.
    const [events] = await Promise.all([
      this.openEvents(generation),
      this.publishTrack("microphone", microphone, generation),
      // Reconcile any intent changed during capture/join, without waiting for SDP.
      this.setState(),
    ]);
    signal.throwIfAborted();
    const signaled = performance.now();
    await waitFor(pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => pc.connectionState === "connected", signal);
    const connected = performance.now();
    // Subscription negotiation still waits for transport. Never open audio early.
    // A pushed roster does not renew the lease consumed by signaling/transport setup.
    await this.poll(true);
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
      microphoneSessionMs: prepared - started,
      signalingMs: signaled - prepared,
      transportMs: connected - signaled,
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
    // The authenticated heartbeat owns session validity. Losing SSE alone must
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
    // Initial setup requires SSE readiness; an established call can tolerate its
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

  private async publishTrack(kind: MediaKind, track: MediaStreamTrack, generation = this.generation) {
    return this.serialize(async () => {
      const pc = this.requirePc();
      const token = this.token;
      if (kind === "microphone") {
        track.enabled = this.readyToTalk && !this.muted;
      }
      const transceiver = pc.addTransceiver(track, { direction: "sendonly", streams: [new MediaStream([track])] });
      if (track.kind === "audio") preferOpus(transceiver);
      await pc.setLocalDescription(await pc.createOffer());
      const mid = transceiver.mid;
      if (!mid) throw new Error("The browser did not assign a media identifier.");
      if (generation !== this.generation) throw new Error("Call session changed.");
      this.senders.set(kind, { sender: transceiver.sender, mid, track });
      const response = await this.api<SessionDescriptionResponse>("publish", {
        kind, mid, sessionDescription: await localDescription(pc, this.captureController.signal),
      }, token);
      if (generation !== this.generation || pc !== this.pc) throw new Error("Call session changed.");
      if (!response.sessionDescription) throw new Error("The media service did not answer publication.");
      await pc.setRemoteDescription(withOpusDtx(response.sessionDescription));
      if (generation !== this.generation) throw new Error("Call session changed.");
      this.senders.set(kind, { sender: transceiver.sender, mid, track });
      if (kind === "microphone") this.localMedia = new MediaStream([track]);
      track.addEventListener("ended", () => {
        if (generation === this.generation && this.senders.get(kind)?.track === track) {
          void this.unpublish(kind).catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
        }
      }, { once: true });
      if (kind === "microphone" && (this.muted || this.monitoring)) {
        await transceiver.sender.replaceTrack(null);
        track.enabled = this.readyToTalk && this.monitoring;
      }
    }, generation).catch((error) => {
      track.stop();
      if (generation === this.generation && this.phase === "connected") { this.pc?.close(); this.scheduleReconnect(); }
      throw error;
    });
  }

  async setMuted(muted: boolean) {
    if (this.monitoring) return;
    const generation = this.generation;
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
    if (deafened || this.deafened) this.muted = deafened;
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

  private async openMicrophone(deviceId?: string) {
    let microphone: Microphone;
    microphone = await captureMicrophone(deviceId, this.noiseSuppression, this.captureController.signal, () => {
      this.microphoneStatus = microphone.status;
      this.emit();
    }, this.audioSetup, this.noiseAssets, this.dpdfnet, this.inputVolume, this.voiceProcessingStrength);
    this.microphoneStatus = microphone.status;
    this.captures.set(microphone.track, microphone);
    return microphone.track;
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
    if (deviceId !== undefined) this.microphoneDeviceId = deviceId || undefined;
    const generation = ++this.localTestGeneration;
    const track = await this.openMicrophone(this.microphoneDeviceId);
    if (generation !== this.localTestGeneration || (this.phase !== "idle" && this.phase !== "failed")) {
      this.stopMicrophone(track);
      return;
    }
    this.localTestTrack = track;
    this.monitorStream = new MediaStream([this.captures.get(track)?.naturalTrack ?? track]);
    this.emit();
  }

  stopLocalMicTest() {
    ++this.localTestGeneration;
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

  private subscribe(trackId: string) {
    const generation = this.generation;
    return this.serialize(async () => {
      if (this.subscriptions.has(trackId)) return;
      const pc = this.requirePc();
      const token = this.token;
      let response: SessionDescriptionResponse;
      try { response = await this.api<SessionDescriptionResponse>("subscribe", { trackId }, token); }
      catch (error) {
        // Ordinary departure between roster delivery and subscription. The API
        // authenticated us and rejected before touching the provider/SDP.
        if (error instanceof CallApiError && error.status === 404 && error.code === "track_gone") return;
        throw error;
      }
      if (generation !== this.generation || pc !== this.pc) return;
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
    }, generation);
  }

  private unsubscribe(trackId: string) {
    const generation = this.generation;
    return this.serialize(async () => {
      const mid = this.subscriptions.get(trackId);
      if (!mid) return;
      // A departed source is no longer playable, regardless of cleanup latency.
      this.remoteMedia.get(trackId)?.stream.getTracks().forEach((track) => track.stop());
      this.remoteMedia.delete(trackId);
      this.emit();
      try { await this.closeMid(mid); } catch (error) {
        if (generation !== this.generation) return;
        // Force-close does not negotiate SDP. Keep healthy media and retry this
        // MID on the next roster reconciliation/lease heartbeat, not a new join.
        if (transientControlError(error)) return;
        throw error;
      }
      if (generation === this.generation) this.subscriptions.delete(trackId);
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
    // SSE handles discovery; snapshots still renew the lease and repair missed state.
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
      report.forEach((stat) => {
        if (stat.type === "outbound-rtp") sent += stat.bytesSent ?? 0;
        if (stat.type === "inbound-rtp") {
          received += stat.bytesReceived ?? 0;
          lost += stat.packetsLost ?? 0;
          jitter = Math.max(jitter, stat.jitter ?? 0);
        }
        if (stat.type === "candidate-pair" && stat.state === "succeeded" && stat.nominated) {
          rtt = Math.max(rtt, stat.currentRoundTripTime ?? 0);
          relay ||= report.get(stat.localCandidateId)?.candidateType === "relay";
        }
      });
      const sampledAt = performance.now();
      const elapsed = sampledAt - (this.previousStats?.sampledAt ?? sampledAt);
      const receiveBitrate = elapsed > 0 ? Math.max(0, (received - (this.previousStats?.received ?? received)) * 8_000 / elapsed) : 0;
      const sendBitrate = elapsed > 0 ? Math.max(0, (sent - (this.previousStats?.sent ?? sent)) * 8_000 / elapsed) : 0;
      this.previousStats = { received, sent, sampledAt };
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
        for (const id of this.subscriptions.keys()) {
          if (generation !== this.generation) return;
          if (pushedVersion !== this.pushedSnapshotVersion) break;
          if (!available.has(id)) await this.unsubscribe(id);
        }
        for (const participant of snapshot.participants) {
          if (participant.id === this.selfId) continue;
          for (const track of participant.tracks) {
            if (generation !== this.generation) return;
            if (pushedVersion !== this.pushedSnapshotVersion) break;
            if (!this.subscriptions.has(track.id)) await this.subscribe(track.id);
          }
        }
        if (pushedVersion !== this.pushedSnapshotVersion) continue;
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
      const { microphone: track, joined, issuedAt } = await this.prepareJoin();
      captured = track;
      if (generation !== this.generation) { this.stopMicrophone(track); void this.api("leave", {}, joined.token).catch(() => undefined); return; }
      await this.connectPrepared(track, joined, generation, started, "Rejoined", issuedAt);
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
