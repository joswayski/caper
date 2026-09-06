import { fakerEN as faker } from "@faker-js/faker";
import { captureMicrophone, type AudioSetup, type Microphone, type NoiseSuppression } from "./microphone.ts";
import { ReceivedMonitor } from "./monitor.ts";
import { NoiseAssets } from "./noise-assets.ts";
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
  private polling = false;
  private microphoneDeviceId?: string;
  private statsTimer?: number;
  private diagnostics = "";
  private noiseSuppression: NoiseSuppression = "dpdfnet2";
  private audioSetup: AudioSetup = "headphones";
  private captures = new Map<MediaStreamTrack, Microphone>();
  private captureController = new AbortController();
  private joinTiming = "";
  private readonly noiseAssets = new NoiseAssets();

  private readonly changed: (state: CallViewState) => void;
  constructor(changed: (state: CallViewState) => void) { this.changed = changed; }

  prepareMicrophone() {
    // Download/compile only: no permission prompt, hardware capture or AudioContext.
    if (this.noiseSuppression === "rnnoise") void this.noiseAssets.load("rnnoise").catch(() => undefined);
    else if (this.noiseSuppression.startsWith("deepfilter")) void this.noiseAssets.load("deepfilter").catch(() => undefined);
    // DPDFNet owns its ONNX runtime in a worker; it does not use these WASM assets.
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
        signal: controller.signal,
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
      const prepared = performance.now();
      capturedMicrophone = microphone;
      if (generation !== this.generation || this.phase !== "joining") {
        this.stopMicrophone(microphone);
        void this.api("leave", {}, joined.token).catch(() => undefined);
        return;
      }
      this.token = joined.token;
      this.selfId = joined.id;
      this.pc = this.makePeerConnection(joined.iceServers);
      await this.publishTrack("microphone", microphone, generation);
      const published = performance.now();
      if (generation !== this.generation) return;
      await waitFor(this.pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => this.pc?.connectionState === "connected");
      if (generation !== this.generation) return;
      this.phase = "connected";
      this.reconnects = 0;
      this.joinTiming = `Joined in ${(performance.now() - started).toFixed(0)} ms · microphone + session ${(prepared - started).toFixed(0)} ms · ICE + signaling ${(published - prepared).toFixed(0)} ms · transport ${(performance.now() - published).toFixed(0)} ms`;
      this.emit();
      const stateSync = this.setState(this.muted, this.deafened);
      this.startPolling();
      await stateSync;
    } catch (error) {
      if (capturedMicrophone && !this.senders.has("microphone")) this.stopMicrophone(capturedMicrophone);
      if (generation !== this.generation) return;
      await this.teardown(false);
      if (generation !== this.generation) return;
      this.phase = "failed";
      this.emit(message(error));
    }
  }

  private makePeerConnection(iceServers: RTCIceServer[]) {
    const pc = new RTCPeerConnection({ iceServers, bundlePolicy: "max-bundle" });
    pc.ontrack = (event) => {
      const transceiver = event.transceiver;
      const found = [...this.subscriptions].find(([, mid]) => mid === transceiver.mid);
      if (!found) return;
      const [trackId] = found;
      const participant = this.participants.find((item) => item.tracks.some((track) => track.id === trackId));
      const kind = participant?.tracks.find((track) => track.id === trackId)?.kind;
      if (!participant || !kind) return;
      const stream = event.streams[0] ?? new MediaStream([event.track]);
      this.remoteMedia.set(trackId, { trackId, participantId: participant.id, kind, stream });
      event.track.onended = () => { this.remoteMedia.delete(trackId); this.emit(); };
      this.emit();
    };
    pc.onconnectionstatechange = () => {
      if (pc !== this.pc) return;
      window.clearTimeout(this.disconnectTimer);
      if (this.phase !== "connected") return;
      if (pc.connectionState === "failed") this.scheduleReconnect();
      else if (pc.connectionState === "disconnected") {
        this.disconnectTimer = window.setTimeout(() => {
          if (pc === this.pc && pc.connectionState === "disconnected") this.scheduleReconnect();
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
        track.enabled = !this.muted;
      }
      const transceiver = pc.addTransceiver(track, { direction: "sendonly", streams: [new MediaStream([track])] });
      if (track.kind === "audio") preferOpus(transceiver);
      await pc.setLocalDescription(await pc.createOffer());
      const mid = transceiver.mid;
      if (!mid) throw new Error("The browser did not assign a media identifier.");
      if (generation !== this.generation) throw new Error("Call session changed.");
      this.senders.set(kind, { sender: transceiver.sender, mid, track });
      const response = await this.api<SessionDescriptionResponse>("publish", {
        kind, mid, sessionDescription: await localDescription(pc),
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
        track.enabled = this.monitoring;
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
    if (microphone) microphone.track.enabled = !muted;
    this.emit();
    await this.serializeMedia(async () => {
      const current = this.senders.get("microphone");
      if (!current) return;
      current.track.enabled = !this.muted;
      await current.sender.replaceTrack(this.muted ? null : current.track);
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
      this.monitorStatus = "Received through Cloudflare · Opus audio. Use headphones; the return is delayed.";
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
    const microphone = await captureMicrophone(deviceId, this.noiseSuppression, this.captureController.signal, () => this.emit(), this.audioSetup, this.noiseAssets);
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
    const track = await this.openMicrophone(deviceId || undefined);
    if (generation !== this.generation || !this.senders.has("microphone")) { this.stopMicrophone(track); return; }
    await this.serializeMedia(async () => {
      const microphone = this.senders.get("microphone");
      if (!microphone) { this.stopMicrophone(track); return; }
      const old = microphone.track;
      track.enabled = !this.muted;
      await microphone.sender.replaceTrack(this.muted || this.monitoring ? null : track);
      track.enabled = this.monitoring || !this.muted;
      if (this.monitoring && this.receivedMonitor) await this.receivedMonitor.replaceTrack(track);
      microphone.track = track;
      this.localMedia = new MediaStream([track]);
      this.microphoneDeviceId = deviceId || undefined;
      this.stopMicrophone(old);
      this.emit();
    }, generation).catch((error) => { this.stopMicrophone(track); throw error; });
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
        await this.api("negotiate", { sessionDescription: await localDescription(pc) }, token);
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
    void this.poll();
    this.pollTimer = window.setInterval(() => void this.poll(), 3_000);
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

  private async poll() {
    if (this.phase !== "connected" || this.polling) return;
    this.polling = true;
    const generation = this.generation;
    const started = performance.now();
    try {
      let snapshot: CallSnapshot;
      try {
        snapshot = await this.api<CallSnapshot>("snapshot", {}, this.token, SNAPSHOT_TIMEOUT_MS);
      } catch (error) {
        if (generation !== this.generation || this.phase !== "connected") return;
        this.controlFailedSince ??= started;
        // Retry only the heartbeat. Never replay ambiguous SFU mutations, and
        // never discard healthy audio for a brief control-plane outage.
        if (!transientControlError(error) || performance.now() - this.controlFailedSince >= CONTROL_RECOVERY_MS) {
          this.scheduleReconnect();
        }
        return;
      }
      if (generation !== this.generation || this.phase !== "connected") return;
      this.controlFailedSince = undefined;
      if (this.stateDirty) {
        try { await this.setState(this.muted, this.deafened); }
        catch (error) { if (!transientControlError(error)) throw error; }
        if (generation !== this.generation || this.phase !== "connected") return;
      }
      this.participants = snapshot.participants;
      const available = new Set(snapshot.participants.flatMap((participant) => participant.tracks.map((track) => track.id)));
      for (const id of this.subscriptions.keys()) if (!available.has(id)) await this.unsubscribe(id);
      for (const participant of snapshot.participants) {
        if (participant.id === this.selfId) continue;
        for (const track of participant.tracks) {
          if (!this.subscriptions.has(track.id)) await this.subscribe(track.id);
        }
      }
      this.emit();
    } catch { if (generation === this.generation) this.scheduleReconnect(); }
    finally { this.polling = false; }
  }

  private scheduleReconnect() {
    if (this.phase !== "connected" && this.phase !== "joining") return;
    this.phase = "reconnecting";
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
      const prepared = performance.now();
      captured = track;
      if (generation !== this.generation) { this.stopMicrophone(track); void this.api("leave", {}, joined.token).catch(() => undefined); return; }
      this.token = joined.token; this.selfId = joined.id; this.pc = this.makePeerConnection(joined.iceServers);
      await this.publishTrack("microphone", track, generation);
      const published = performance.now();
      await waitFor(this.pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => this.pc?.connectionState === "connected");
      if (generation !== this.generation) return;
      this.phase = "connected";
      this.joinTiming = `Rejoined in ${(performance.now() - started).toFixed(0)} ms · microphone + session ${(prepared - started).toFixed(0)} ms · ICE + signaling ${(published - prepared).toFixed(0)} ms · transport ${(performance.now() - published).toFixed(0)} ms`;
      this.emit();
      const stateSync = this.setState(this.muted, this.deafened);
      this.startPolling();
      await stateSync;
      if (this.monitoring) await this.startReceivedMonitor().catch(() => undefined);
    } catch {
      if (captured) this.stopMicrophone(captured);
      if (generation !== this.generation) return;
      this.scheduleReconnect();
    }
  }

  async leave() {
    if (this.phase === "idle" || this.phase === "leaving") return;
    ++this.generation;
    this.resetMonitoring();
    // The view renders this as the join screen while cleanup prevents another
    // session from starting until the prior capability has been released.
    this.phase = "leaving";
    this.participants = [];
    this.selfId = undefined;
    this.emit();
    await this.teardown(false);
    this.phase = "idle";
    this.emit();
  }

  leaveImmediately() {
    if (this.token) void fetch(`${API_ROOT}/leave`, { method: "POST", keepalive: true, headers: { "content-type": "application/json", authorization: `Bearer ${this.token}` }, body: "{}" }).catch(() => undefined);
    ++this.generation;
    this.phase = "idle";
    this.resetMonitoring();
    window.clearInterval(this.pollTimer);
    window.clearTimeout(this.reconnectTimer);
    for (const publication of this.senders.values()) publication.track.stop();
    this.stopEverything();
  }

  private async teardown(skipLeave: boolean, preserve?: MediaStreamTrack) {
    window.clearInterval(this.pollTimer);
    window.clearTimeout(this.reconnectTimer);
    const token = this.token;
    for (const publication of this.senders.values()) if (publication.track !== preserve) publication.track.stop();
    this.stopEverything(preserve);
    if (!skipLeave && token) await this.api("leave", {}, token).catch(() => undefined);
  }

  private stopEverything(preserve?: MediaStreamTrack) {
    window.clearTimeout(this.disconnectTimer);
    this.controlFailedSince = undefined;
    this.stateDirty = false;
    this.captureController.abort();
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
    this.senders.clear(); this.subscriptions.clear(); this.remoteMedia.clear(); this.localMedia = undefined; this.polling = false;
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
