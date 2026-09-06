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
const ICE_TIMEOUT_MS = 5_000;
const CONNECT_TIMEOUT_MS = 12_000;
const MAX_REJOINS = 3;
const FETCH_TIMEOUT_MS = 25_000;

class CallApiError extends Error {
  readonly status: number;
  constructor(message: string, status: number) { super(message); this.status = status; }
}

function message(error: unknown) {
  return error instanceof Error ? error.message : "The call could not continue.";
}

export function waitFor(target: EventTarget, event: string, timeout: number, ready: () => boolean) {
  if (ready()) return Promise.resolve();
  return new Promise<void>((resolve, reject) => {
    const timer = window.setTimeout(() => finish(new Error(`Timed out waiting for ${event}`)), timeout);
    const listener = () => { if (ready()) finish(); };
    const finish = (error?: Error) => {
      window.clearTimeout(timer);
      target.removeEventListener(event, listener);
      error ? reject(error) : resolve();
    };
    target.addEventListener(event, listener);
  });
}

async function localDescription(pc: RTCPeerConnection) {
  await waitFor(pc, "icegatheringstatechange", ICE_TIMEOUT_MS, () => pc.iceGatheringState === "complete").catch(() => undefined);
  if (!pc.localDescription) throw new Error("WebRTC did not produce a session description.");
  return pc.localDescription.toJSON();
}

export class PublicCallClient {
  private pc?: RTCPeerConnection;
  private token?: string;
  private selfId?: string;
  private name = "Guest";
  private phase: CallViewState["phase"] = "idle";
  private participants: Participant[] = [];
  private remoteMedia = new Map<string, RemoteMedia>();
  private senders = new Map<MediaKind, { sender: RTCRtpSender; mid: string; track: MediaStreamTrack }>();
  private subscriptions = new Map<string, string>();
  private queue: Promise<unknown> = Promise.resolve();
  private pollTimer?: number;
  private reconnectTimer?: number;
  private generation = 0;
  private reconnects = 0;
  private muted = false;
  private deafened = false;
  private polling = false;
  private microphoneDeviceId?: string;
  private statsTimer?: number;
  private speaking: string[] = [];
  private diagnostics = "";

  private readonly changed: (state: CallViewState) => void;
  constructor(changed: (state: CallViewState) => void) { this.changed = changed; }

  private emit(error?: string) {
    this.changed({
      phase: this.phase,
      selfId: this.selfId,
      participants: this.participants,
      remoteMedia: [...this.remoteMedia.values()],
      speaking: this.speaking,
      diagnostics: this.diagnostics,
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

  private async api<T = void>(operation: string, body: object = {}, token = this.token): Promise<T> {
    const controller = new AbortController();
    const timer = window.setTimeout(() => controller.abort(), FETCH_TIMEOUT_MS);
    const response = await fetch(`${API_ROOT}/${operation}`, {
      method: "POST",
      headers: {
        "content-type": "application/json",
        ...(token ? { authorization: `Bearer ${token}` } : {}),
      },
      body: JSON.stringify(body),
      signal: controller.signal,
    }).finally(() => window.clearTimeout(timer));
    if (!response.ok) {
      const detail = await response.json().catch(() => ({})) as { error?: string };
      throw new CallApiError(detail.error || `Call service returned ${response.status}.`, response.status);
    }
    if (response.status === 204 || response.headers.get("content-length") === "0") return undefined as T;
    const text = await response.text();
    return (text ? JSON.parse(text) : undefined) as T;
  }

  async join(name: string, microphoneDeviceId?: string) {
    if (this.phase !== "idle" && this.phase !== "failed") return;
    this.name = name.trim().slice(0, 40) || `Guest ${Math.floor(100 + Math.random() * 900)}`;
    this.microphoneDeviceId = microphoneDeviceId || undefined;
    this.phase = "joining";
    this.emit();
    const generation = ++this.generation;
    let capturedMicrophone: MediaStreamTrack | undefined;
    try {
      const stream = await navigator.mediaDevices.getUserMedia({
        audio: {
          deviceId: microphoneDeviceId ? { exact: microphoneDeviceId } : undefined,
          echoCancellation: true,
          noiseSuppression: true,
          autoGainControl: true,
        },
      });
      const microphone = stream.getAudioTracks()[0];
      if (!microphone) throw new Error("No microphone track was available.");
      capturedMicrophone = microphone;
      if (generation !== this.generation || this.phase !== "joining") {
        stream.getTracks().forEach((track) => track.stop());
        return;
      }
      const joined = await this.api<JoinResponse>("join", { name: this.name }, undefined);
      if (generation !== this.generation) {
        stream.getTracks().forEach((track) => track.stop());
        void this.api("leave", {}, joined.token).catch(() => undefined);
        return;
      }
      this.token = joined.token;
      this.selfId = joined.id;
      this.pc = this.makePeerConnection(joined.iceServers);
      await this.publishTrack("microphone", microphone, generation);
      if (generation !== this.generation) return;
      await waitFor(this.pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => this.pc?.connectionState === "connected");
      if (generation !== this.generation) return;
      this.phase = "connected";
      this.reconnects = 0;
      await this.setState(this.muted, this.deafened);
      this.startPolling();
      this.emit();
    } catch (error) {
      if (capturedMicrophone && !this.senders.has("microphone")) capturedMicrophone.stop();
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
      if ((pc.connectionState === "failed" || pc.connectionState === "disconnected") && this.phase === "connected") {
        this.scheduleReconnect();
      }
    };
    return pc;
  }

  private async publishTrack(kind: MediaKind, track: MediaStreamTrack, generation = this.generation) {
    return this.serialize(async () => {
      const pc = this.requirePc();
      const token = this.token;
      if (kind === "microphone") track.enabled = !this.muted;
      const transceiver = pc.addTransceiver(track, { direction: "sendonly", streams: [new MediaStream([track])] });
      if (track.kind === "audio" && typeof RTCRtpSender !== "undefined" && transceiver.setCodecPreferences) {
        const codecs = RTCRtpSender.getCapabilities("audio")?.codecs ?? [];
        transceiver.setCodecPreferences([...codecs.filter((c) => c.mimeType.toLowerCase() === "audio/opus"), ...codecs.filter((c) => c.mimeType.toLowerCase() !== "audio/opus")]);
      }
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
      track.addEventListener("ended", () => {
        if (generation === this.generation && this.senders.get(kind)?.track === track) {
          void this.unpublish(kind).catch(() => this.scheduleReconnect());
        }
      }, { once: true });
      if (kind === "microphone" && this.muted) await transceiver.sender.replaceTrack(null);
    }, generation).catch((error) => {
      track.stop();
      if (generation === this.generation && this.phase === "connected") { this.pc?.close(); this.scheduleReconnect(); }
      throw error;
    });
  }

  async setMuted(muted: boolean) {
    this.muted = muted;
    await this.serialize(async () => {
      const token = this.token;
      const microphone = this.senders.get("microphone");
      if (microphone) {
        microphone.track.enabled = !muted;
        await microphone.sender.replaceTrack(muted ? null : microphone.track);
      }
      if (token) await this.api("state", { muted, deafened: this.deafened }, token);
    });
    this.emit();
  }

  async setDeafened(deafened: boolean) {
    this.deafened = deafened;
    await this.setState(this.muted, deafened);
    this.emit();
  }

  private setState(muted: boolean, deafened: boolean) {
    return this.serialize(() => this.api("state", { muted, deafened }));
  }

  async changeMicrophone(deviceId: string) {
    this.microphoneDeviceId = deviceId || undefined;
    const generation = this.generation;
    const stream = await navigator.mediaDevices.getUserMedia({ audio: { deviceId: deviceId ? { exact: deviceId } : undefined, echoCancellation: true, noiseSuppression: true } });
    const track = stream.getAudioTracks()[0]!;
    if (generation !== this.generation || !this.senders.has("microphone")) { track.stop(); return; }
    await this.serialize(async () => {
      const microphone = this.senders.get("microphone");
      if (!microphone) { track.stop(); return; }
      const old = microphone.track;
      track.enabled = !this.muted;
      await microphone.sender.replaceTrack(this.muted ? null : track);
      microphone.track = track;
      old.stop();
    }, generation).catch((error) => { track.stop(); throw error; });
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
      const speaking = new Set<string>();
      let received = 0, sent = 0, lost = 0, jitter = 0, rtt = 0, relay = false;
      report.forEach((stat) => {
        if (stat.type === "outbound-rtp") sent += stat.bytesSent ?? 0;
        if (stat.type === "inbound-rtp") {
          received += stat.bytesReceived ?? 0;
          lost += stat.packetsLost ?? 0;
          jitter = Math.max(jitter, stat.jitter ?? 0);
          if (stat.kind === "audio" && stat.audioLevel > 0.02) {
            for (const media of this.remoteMedia.values()) {
              if (media.kind === "microphone" && media.stream.getTracks().some((track) => track.id === stat.trackIdentifier)) speaking.add(media.participantId);
            }
          }
        }
        if (stat.type === "media-source" && stat.kind === "audio" && stat.audioLevel > 0.02 && !this.muted && this.selfId) speaking.add(this.selfId);
        if (stat.type === "candidate-pair" && stat.state === "succeeded" && stat.nominated) {
          rtt = Math.max(rtt, stat.currentRoundTripTime ?? 0);
          relay ||= report.get(stat.localCandidateId)?.candidateType === "relay";
        }
      });
      this.speaking = [...speaking];
      this.diagnostics = `This connection: ${(received / 1e6).toFixed(2)} MB received · ${(sent / 1e6).toFixed(2)} MB sent · ${lost} packets lost · ${(jitter * 1000).toFixed(0)} ms max jitter · ${(rtt * 1000).toFixed(0)} ms RTT · ${relay ? "TURN relay" : "direct / relay not observed"}`;
      this.emit();
    } catch { /* Stats support varies; diagnostics must never interrupt media. */ }
  }

  private async poll() {
    if (this.phase !== "connected" || this.polling) return;
    this.polling = true;
    const generation = this.generation;
    try {
      const snapshot = await this.api<CallSnapshot>("snapshot");
      if (generation !== this.generation || this.phase !== "connected") return;
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
      this.phase = "failed";
      this.emit("Connection lost. Please join again.");
      return;
    }
    const generation = ++this.generation;
    await this.teardown(false);
    if (generation !== this.generation) return;
    this.phase = "joining";
    this.emit();
    let captured: MediaStreamTrack | undefined;
    try {
      const track = (await navigator.mediaDevices.getUserMedia({ audio: { deviceId: this.microphoneDeviceId ? { exact: this.microphoneDeviceId } : undefined, echoCancellation: true, noiseSuppression: true } })).getAudioTracks()[0]!;
      captured = track;
      if (generation !== this.generation) { track.stop(); return; }
      const joined = await this.api<JoinResponse>("join", { name: this.name }, undefined);
      if (generation !== this.generation) { track.stop(); void this.api("leave", {}, joined.token).catch(() => undefined); return; }
      this.token = joined.token; this.selfId = joined.id; this.pc = this.makePeerConnection(joined.iceServers);
      await this.publishTrack("microphone", track, generation);
      await waitFor(this.pc, "connectionstatechange", CONNECT_TIMEOUT_MS, () => this.pc?.connectionState === "connected");
      if (generation !== this.generation) return;
      this.phase = "connected";
      await this.setState(this.muted, this.deafened);
      this.startPolling();
      this.emit();
    } catch {
      captured?.stop();
      if (generation !== this.generation) return;
      this.scheduleReconnect();
    }
  }

  async leave() {
    if (this.phase === "idle" || this.phase === "leaving") return;
    this.phase = "leaving";
    ++this.generation;
    this.emit();
    await this.teardown(false);
    this.phase = "idle";
    this.participants = [];
    this.selfId = undefined;
    this.emit();
  }

  leaveImmediately() {
    if (this.token) void fetch(`${API_ROOT}/leave`, { method: "POST", keepalive: true, headers: { "content-type": "application/json", authorization: `Bearer ${this.token}` }, body: "{}" }).catch(() => undefined);
    ++this.generation;
    this.phase = "idle";
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
    window.clearInterval(this.statsTimer);
    this.speaking = [];
    this.diagnostics = "";
    this.pc?.getReceivers().forEach((receiver) => receiver.track.stop());
    this.pc?.getSenders().forEach((sender) => { if (sender.track !== preserve) sender.track?.stop(); });
    for (const publication of this.senders.values()) if (publication.track !== preserve) publication.track.stop();
    this.pc?.close();
    this.pc = undefined; this.token = undefined;
    this.senders.clear(); this.subscriptions.clear(); this.remoteMedia.clear(); this.polling = false;
  }

  private requirePc() {
    if (!this.pc) throw new Error("There is no active media session.");
    return this.pc;
  }
}
