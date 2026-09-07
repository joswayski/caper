import { fakerEN as faker } from "@faker-js/faker";
import { captureMicrophone, type AudioSetup, type Microphone, type NoiseSuppression } from "./microphone.ts";
import { ReceivedMonitor } from "./monitor.ts";
import { NoiseAssets } from "./noise-assets.ts";
import { DpdfnetPreparation } from "./dpdfnet-preparation.ts";
import { CallEvents } from "./events.ts";
import { localDescription, preferOpus, waitFor } from "./rtc.ts";
export { waitFor } from "./rtc.ts";
import type {
  CallSnapshot,
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
  constructor(message: string, status: number) { super(message); this.status = status; }
}

function message(error: unknown) {
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
  private selfId?: string;
  private name = "Guest";
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
  private pollRetryTimer?: number;
  private eventRetryTimer?: number;
  private eventRecoveryTimer?: number;
  private controlFailedSince?: number;
  private stateDirty = false;
  private generation = 0;
  private reconnects = 0;
  private muted = false;
  private deafened = false;
  private monitoring = false;
  private monitorStream?: MediaStream;
  private receivedMonitor?: ReceivedMonitor;
  private monitorConnecting = false;
  private monitorStatus?: string;
  private stateBeforeMonitoring?: { muted: boolean; deafened: boolean };
  private pollPromise?: Promise<void>;
  private pollAgain = false;
  private events?: CallEvents;
  private microphoneDeviceId?: string;
  private statsTimer?: number;
  private diagnostics = "";
  private noiseSuppression: NoiseSuppression = "dpdfnet8";
  private audioSetup: AudioSetup = "headphones";
  private captures = new Map<MediaStreamTrack, Microphone>();
  private captureController = new AbortController();
  private joinTiming = "";
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
      monitoring: this.monitoring,
      monitorStream: this.monitorStream,
      monitorConnecting: this.monitorConnecting,
      monitorStatus: this.monitorStatus,
      selfId: this.selfId,
      localMedia: this.localMedia,
      participants: this.participants,
      remoteMedia: [...this.remoteMedia.values()],
      diagnostics: this.diagnostics,
      noiseSuppression: this.noiseSuppression,
      audioSetup: this.audioSetup,
      noiseSuppressionStatus: this.captures.get(this.senders.get("microphone")?.track!)?.status,
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

  private async api<T = void>(operation: string, body: object = {}, token = this.token, timeout = FETCH_TIMEOUT_MS): Promise<T> {
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), timeout);
    try {
      const response = await fetch(`${API_ROOT}/${operation}`, {
        method: "POST",
        headers: {
          "content-type": "application/json",
          ...(token ? { authorization: `Bearer ${token}` } : {}),
        },
        body: JSON.stringify(body),
        keepalive: operation === "leave",
        signal: operation === "join" || operation === "leave" ? controller.signal : AbortSignal.any([controller.signal, this.captureController.signal]),
      });
      if (!response.ok) {
        const detail = await response.json().catch(() => ({})) as { error?: string };
        throw new CallApiError(detail.error || `Call service returned ${response.status}.`, response.status);
      }
      if (response.status === 204 || response.headers.get("content-length") === "0") return undefined as T;
      const text = await response.text();
      return (text ? JSON.parse(text) : undefined) as T;
    } finally { window.clearTimeout(timer); }
  }

  private async prepareJoin() {
    const capture = this.openMicrophone(this.microphoneDeviceId);
    const joining = this.api<JoinResponse>("join", { name: this.name }, undefined);
    const [captureResult, joinResult] = await Promise.allSettled([capture, joining]);
    if (captureResult.status === "rejected" || joinResult.status === "rejected") {
      if (captureResult.status === "fulfilled") this.stopMicrophone(captureResult.value);
      if (joinResult.status === "fulfilled") void this.api("leave", {}, joinResult.value.token).catch(() => undefined);
      throw captureResult.status === "rejected" ? captureResult.reason : joinResult.status === "rejected" ? joinResult.reason : new Error("Join preparation failed.");
    }
    return { microphone: captureResult.value, joined: joinResult.value };
  }

  async join(name = "", microphoneDeviceId?: string) {
    if (this.phase !== "idle" && this.phase !== "failed") return;
    const started = performance.now();
    // Generate once per explicit join, not per roster render or automatic rejoin.
    this.name = name.trim().slice(0, 40) || `${faker.word.adjective()} ${faker.animal.type()}`.slice(0, 40);
    this.microphoneDeviceId = microphoneDeviceId || undefined;
    this.phase = "joining";
    this.emit();
    const generation = ++this.generation;
    let capturedMicrophone: MediaStreamTrack | undefined;
    try {
      const { microphone, joined } = await this.prepareJoin();
      capturedMicrophone = microphone;
      if (generation !== this.generation || this.phase !== "joining") {
        this.stopMicrophone(microphone);
        void this.api("leave", {}, joined.token).catch(() => undefined);
        return;
      }
      await this.connectPrepared(microphone, joined, generation, started, "Joined");
      this.reconnects = 0;
    } catch (error) {
      if (capturedMicrophone && !this.senders.has("microphone")) this.stopMicrophone(capturedMicrophone);
      if (generation !== this.generation) return;
      await this.teardown(false);
      if (generation !== this.generation) return;
      this.phase = "failed";
      this.emit(message(error));
    }
  }

  private async connectPrepared(microphone: MediaStreamTrack, joined: JoinResponse, generation: number, started: number, label: string) {
    const prepared = performance.now();
    const signal = this.captureController.signal;
    this.token = joined.token;
    this.selfId = joined.id;
    const pc = this.pc = this.makePeerConnection(joined.iceServers);
    const events = await this.openEvents(generation);
    signal.throwIfAborted();
    const liveUpdates = performance.now();
    await this.publishTrack("microphone", microphone, generation);
    const published = performance.now();
    await waitFor(pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => pc.connectionState === "connected", signal);
    const connected = performance.now();
    // Keep sending silence through negotiation and initial roster/state synchronization.
    await this.poll();
    await this.setState(this.muted, this.deafened);
    if (this.pollAgain) await this.poll();
    signal.throwIfAborted();
    if (generation !== this.generation || !events.connected || pc.connectionState !== "connected" || microphone.readyState !== "live") {
      throw new Error("Voice setup changed before it was ready. Please join again.");
    }
    this.phase = "connected";
    // Monitoring has already detached the public sender; only its private return uses audio.
    microphone.enabled = this.monitoring || !this.muted;
    this.joinTiming = `${label} in ${(performance.now() - started).toFixed(0)} ms · microphone + session ${(prepared - started).toFixed(0)} ms · live updates ${(liveUpdates - prepared).toFixed(0)} ms · ICE + signaling ${(published - liveUpdates).toFixed(0)} ms · transport ${(connected - published).toFixed(0)} ms · roster + state ${(performance.now() - connected).toFixed(0)} ms`;
    this.emit();
    this.startPolling();
  }

  private async openEvents(generation: number) {
    this.events?.stop();
    const events = this.events = new CallEvents(() => {
      if (generation !== this.generation) return;
      if (this.phase === "connected") void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
      else this.pollAgain = true;
    }, (error) => {
      if (generation !== this.generation) return;
      if (this.phase === "connected") this.scheduleEventRecovery(generation);
      else this.captureController.abort(error); // Startup still fails closed.
    });
    await events.open(this.token!, this.captureController.signal);
    if (generation === this.generation && events === this.events && events.connected) {
      window.clearTimeout(this.eventRecoveryTimer);
      this.eventRecoveryTimer = undefined;
    }
    return events;
  }

  private scheduleEventRecovery(generation: number) {
    // Reconnect control updates with the same capability, not a new voice session.
    this.eventRecoveryTimer ??= window.setTimeout(() => {
      if (generation === this.generation) this.scheduleReconnect();
    }, CONTROL_RECOVERY_MS);
    window.clearTimeout(this.eventRetryTimer);
    this.eventRetryTimer = window.setTimeout(() => {
      if (generation !== this.generation || this.phase !== "connected") return;
      void this.openEvents(generation).then(() => {
        if (generation === this.generation) return this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
      }).catch(() => { /* Stream failures schedule their own retry; heartbeat validates the session. */ });
    }, 3_000);
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
      await pc.setRemoteDescription(response.sessionDescription);
      if (generation !== this.generation) throw new Error("Call session changed.");
      this.senders.set(kind, { sender: transceiver.sender, mid, track });
      if (kind === "microphone") this.localMedia = new MediaStream([track]);
      track.addEventListener("ended", () => {
        if (generation === this.generation && this.senders.get(kind)?.track === track) {
          void this.unpublish(kind).catch(() => this.scheduleReconnect());
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
    this.muted = muted;
    const microphone = this.senders.get("microphone");
    if (microphone) microphone.track.enabled = this.readyToTalk && !muted;
    this.emit();
    await this.serializeMedia(async () => {
      const current = this.senders.get("microphone");
      if (!current) return;
      current.track.enabled = this.readyToTalk && !this.muted;
      await current.sender.replaceTrack(this.muted || !this.readyToTalk ? null : current.track);
    });
    if (this.token) await this.setState(muted, this.deafened);
  }

  async setDeafened(deafened: boolean) {
    if (this.monitoring) return;
    this.deafened = deafened;
    this.emit();
    await this.setState(this.muted, deafened);
  }

  async setMonitoring(monitoring: boolean) {
    if (monitoring === this.monitoring) return;
    if (monitoring && (this.phase !== "connected" || !this.token)) throw new Error("Join voice before testing your microphone.");
    if (monitoring) {
      this.stateBeforeMonitoring = { muted: this.muted, deafened: this.deafened };
      this.monitoring = true;
      this.monitorConnecting = true;
      this.monitorStatus = "Preparing private microphone test…";
      this.muted = true;
      this.deafened = true;
    } else {
      this.stopReceivedMonitor();
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
    if (this.token) await this.setState(this.muted, this.deafened);
    if (monitoring && this.monitoring) await this.startReceivedMonitor();
  }

  private async startReceivedMonitor() {
    const microphone = this.senders.get("microphone");
    if (!this.monitoring || !microphone || !this.token || this.receivedMonitor || this.phase !== "connected") return;
    const monitor = new ReceivedMonitor(this.api.bind(this), this.token, () => {
      if (this.receivedMonitor !== monitor) return;
      this.stopReceivedMonitor();
      this.monitorStatus = "Mic test connection lost. Stop the test and try again; the channel remains muted.";
      this.emit();
    });
    this.receivedMonitor = monitor;
    this.monitorConnecting = true;
    this.monitorStatus = "Connecting private microphone return through Cloudflare…";
    this.emit();
    try {
      const stream = await monitor.start(microphone.track);
      if (this.receivedMonitor !== monitor || !this.monitoring) return;
      this.monitorStream = stream;
      this.monitorConnecting = false;
      this.monitorStatus = "Private Cloudflare return connected · Record to check the received audio.";
      this.emit();
    } catch (error) {
      if (this.receivedMonitor !== monitor) return;
      this.stopReceivedMonitor();
      this.monitorStatus = "Mic test failed. Stop the test and try again; the channel remains muted.";
      this.emit();
      throw error;
    }
  }

  private stopReceivedMonitor() {
    this.receivedMonitor?.stop();
    this.receivedMonitor = undefined;
    this.monitorStream = undefined;
    this.monitorConnecting = false;
    this.monitorStatus = undefined;
  }

  private setState(muted: boolean, deafened: boolean) {
    this.stateDirty = true;
    const generation = this.generation;
    return this.serialize(async () => {
      await this.api("state", { muted, deafened });
      if (generation === this.generation) this.stateDirty = this.muted !== muted || this.deafened !== deafened;
    }, generation);
  }

  private async openMicrophone(deviceId?: string) {
    const microphone = await captureMicrophone(deviceId, this.noiseSuppression, this.captureController.signal, () => {
      if (this.phase === "connected" && this.senders.get("microphone")?.track.readyState === "ended") this.scheduleReconnect();
      this.emit();
    }, this.audioSetup, this.noiseAssets, this.dpdfnet);
    this.captures.set(microphone.track, microphone);
    return microphone.track;
  }

  private stopMicrophone(track: MediaStreamTrack) {
    const capture = this.captures.get(track);
    if (capture) { capture.stop(); this.captures.delete(track); }
    else track.stop();
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
        if (this.monitoring && this.receivedMonitor) await this.receivedMonitor.replaceTrack(track);
        if (generation !== this.generation) throw new Error("Call session changed.");
        microphone.track = track;
        this.localMedia = new MediaStream([track]);
        this.microphoneDeviceId = deviceId || undefined;
        this.stopMicrophone(old);
        this.emit();
      } catch (error) {
        let rollbackFailed = false;
        try { await oldCapture?.resume(); } catch { rollbackFailed = true; }
        if (senderReplaced && generation === this.generation && this.senders.get("microphone") === microphone) {
          try {
            await microphone.sender.replaceTrack(this.muted || this.monitoring ? null : old);
            if (this.monitoring && this.receivedMonitor) await this.receivedMonitor.replaceTrack(old);
          } catch { rollbackFailed = true; }
        }
        if (track) this.stopMicrophone(track);
        if (rollbackFailed) this.scheduleReconnect();
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
      const response = await this.api<SessionDescriptionResponse>("subscribe", { trackId }, token);
      if (generation !== this.generation || pc !== this.pc) return;
      const mid = response.tracks?.[0]?.mid;
      if (!mid) throw new Error("The media service did not return a subscription identifier.");
      this.subscriptions.set(trackId, mid);
      if (response.sessionDescription) {
        await pc.setRemoteDescription(response.sessionDescription);
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
      await this.closeMid(mid);
      if (generation !== this.generation) return;
      this.subscriptions.delete(trackId);
      this.remoteMedia.get(trackId)?.stream.getTracks().forEach((track) => track.stop());
      this.remoteMedia.delete(trackId);
      this.emit();
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
    try { await this.api("close", { mid }, token); } catch (error) {
      if (error instanceof CallApiError && error.status === 404) return;
      if (token === this.token) this.scheduleReconnect();
      throw error;
    }
  }

  private startPolling() {
    window.clearInterval(this.pollTimer);
    window.clearInterval(this.statsTimer);
    // SSE handles discovery; snapshots still renew the lease and repair missed state.
    const generation = this.generation;
    if (this.pollAgain) void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
    this.pollTimer = window.setInterval(() => void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); }), 15_000);
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
      this.diagnostics = `${this.joinTiming} · This connection: ${(received / 1e6).toFixed(2)} MB received · ${(sent / 1e6).toFixed(2)} MB sent · ${lost} packets lost · ${(jitter * 1000).toFixed(0)} ms max jitter · ${(rtt * 1000).toFixed(0)} ms RTT · ${relay ? "TURN relay" : "direct / relay not observed"}`;
      this.emit();
    } catch { /* Stats support varies; diagnostics must never interrupt media. */ }
  }

  private poll(): Promise<void> {
    if (this.phase !== "connected" && this.phase !== "joining") return Promise.resolve();
    this.pollAgain = true;
    if (this.pollPromise) return this.pollPromise;
    const generation = this.generation;
    const refresh = async () => {
      while (this.pollAgain && generation === this.generation) {
        this.pollAgain = false;
        const started = performance.now();
        let snapshot: CallSnapshot;
        try {
          snapshot = await this.api<CallSnapshot>("snapshot", {}, this.token, SNAPSHOT_TIMEOUT_MS);
        } catch (error) {
          if (generation !== this.generation) return;
          if (this.phase !== "connected") throw error;
          this.controlFailedSince ??= started;
          if (!transientControlError(error) || performance.now() - this.controlFailedSince >= CONTROL_RECOVERY_MS) {
            this.scheduleReconnect();
          } else {
            window.clearTimeout(this.pollRetryTimer);
            this.pollRetryTimer = window.setTimeout(() => {
              if (generation === this.generation) void this.poll().catch(() => { if (generation === this.generation) this.scheduleReconnect(); });
            }, 3_000);
          }
          return;
        }
        if (generation !== this.generation) return;
        window.clearTimeout(this.pollRetryTimer);
        this.controlFailedSince = undefined;
        if (this.stateDirty && this.phase === "connected") {
          try { await this.setState(this.muted, this.deafened); }
          catch (error) { if (!transientControlError(error)) throw error; }
          if (generation !== this.generation) return;
        }
        this.participants = snapshot.participants;
        const available = new Set(snapshot.participants.flatMap((participant) => participant.tracks.map((track) => track.id)));
        for (const id of this.subscriptions.keys()) {
          if (generation !== this.generation) return;
          if (!available.has(id)) await this.unsubscribe(id);
        }
        for (const participant of snapshot.participants) {
          if (participant.id === this.selfId) continue;
          for (const track of participant.tracks) {
            if (generation !== this.generation) return;
            if (!this.subscriptions.has(track.id)) await this.subscribe(track.id);
          }
        }
        if (generation === this.generation) this.emit();
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
      await this.teardown(true);
      this.resetMonitoring();
      this.phase = "failed";
      this.emit("Connection lost. Please join again.");
      return;
    }
    const generation = ++this.generation;
    await this.teardown(false);
    if (generation !== this.generation) return;
    this.phase = "joining";
    this.emit();
    const started = performance.now();
    let captured: MediaStreamTrack | undefined;
    try {
      const { microphone: track, joined } = await this.prepareJoin();
      captured = track;
      if (generation !== this.generation) { this.stopMicrophone(track); void this.api("leave", {}, joined.token).catch(() => undefined); return; }
      await this.connectPrepared(track, joined, generation, started, "Rejoined");
      if (this.monitoring) await this.startReceivedMonitor().catch(() => undefined);
    } catch {
      if (captured) this.stopMicrophone(captured);
      if (generation !== this.generation) return;
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
    window.clearTimeout(this.disconnectTimer);
    this.disconnectTimer = undefined;
    window.clearTimeout(this.pollRetryTimer);
    window.clearTimeout(this.eventRetryTimer);
    window.clearTimeout(this.eventRecoveryTimer);
    this.eventRecoveryTimer = undefined;
    this.controlFailedSince = undefined;
    this.stateDirty = false;
    this.captureController.abort();
    this.events?.stop();
    this.events = undefined;
    this.captureController = new AbortController();
    for (const capture of this.captures.values()) capture.stop();
    this.captures.clear();
    window.clearInterval(this.statsTimer);
    this.diagnostics = "";
    this.joinTiming = "";
    this.stopReceivedMonitor();
    this.pc?.getReceivers().forEach((receiver) => receiver.track.stop());
    this.pc?.getSenders().forEach((sender) => { if (sender.track !== preserve) sender.track?.stop(); });
    for (const publication of this.senders.values()) if (publication.track !== preserve) publication.track.stop();
    this.pc?.close();
    this.pc = undefined; this.token = undefined;
    this.senders.clear(); this.subscriptions.clear(); this.remoteMedia.clear(); this.localMedia = undefined; this.pollPromise = undefined; this.pollAgain = false;
  }

  private resetMonitoring() {
    if (!this.monitoring) return;
    this.monitoring = false;
    this.muted = this.stateBeforeMonitoring?.muted ?? false;
    this.deafened = this.stateBeforeMonitoring?.deafened ?? false;
    this.stateBeforeMonitoring = undefined;
    this.stopReceivedMonitor();
  }

  private requirePc() {
    if (!this.pc) throw new Error("There is no active media session.");
    return this.pc;
  }
}
