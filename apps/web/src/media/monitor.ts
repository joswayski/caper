import { localDescription, preferOpus, waitFor } from "./rtc.ts";
import type { JoinResponse, SessionDescriptionResponse } from "./types";

type Request = <T = void>(operation: string, body: object, token: string) => Promise<T>;

/** Owns two private SFU sessions. Borrows capture; never plays or stops its track. */
export class ReceivedMonitor {
  private readonly tokens = new Set<string>();
  private readonly peers: RTCPeerConnection[] = [];
  private sender?: RTCRtpSender;
  private stream?: MediaStream;
  private heartbeat?: number;
  private stopped = false;
  private readonly request: Request;
  private readonly parent: string;
  private readonly failed: () => void;

  constructor(request: Request, parent: string, failed: () => void) {
    this.request = request;
    this.parent = parent;
    this.failed = failed;
  }

  private check() { if (this.stopped) throw new Error("Mic test cancelled."); }

  private async join(monitor: "sender" | "receiver") {
    const joined = await this.request<JoinResponse>("join", { name: "Microphone test", monitor }, this.parent);
    if (this.stopped) {
      void this.request("leave", {}, joined.token).catch(() => undefined);
      this.check();
    }
    this.tokens.add(joined.token);
    if (this.heartbeat === undefined) {
      this.heartbeat = window.setInterval(() => {
        void Promise.all([...this.tokens].map((token) => this.request("snapshot", {}, token)))
          .catch(() => { if (!this.stopped) this.failed(); });
      }, 10_000);
    }
    return joined;
  }

  private peer(iceServers: RTCIceServer[]) {
    const pc = new RTCPeerConnection({ iceServers, bundlePolicy: "max-bundle" });
    this.peers.push(pc);
    pc.onconnectionstatechange = () => {
      if (!this.stopped && ["failed", "disconnected"].includes(pc.connectionState)) this.failed();
    };
    return pc;
  }

  async start(track: MediaStreamTrack): Promise<MediaStream> {
    try {
      // Register each successful capability immediately so partial failure/late joins clean up.
      const joined = await Promise.all([this.join("sender"), this.join("receiver")]);
      this.check();
      const tx = this.peer(joined[0].iceServers);
      const transceiver = tx.addTransceiver(track, { direction: "sendonly", streams: [new MediaStream([track])] });
      preferOpus(transceiver);
      this.sender = transceiver.sender;
      await tx.setLocalDescription(await tx.createOffer());
      this.check();
      if (!transceiver.mid) throw new Error("Mic test did not assign a media identifier.");
      const publication = await this.request<SessionDescriptionResponse & { trackId: string }>("publish", {
        kind: "microphone", mid: transceiver.mid, sessionDescription: await localDescription(tx),
      }, joined[0].token);
      this.check();
      if (!publication.sessionDescription || !publication.trackId) throw new Error("Mic test publication failed.");
      await tx.setRemoteDescription(publication.sessionDescription);
      this.check();
      const rx = this.peer(joined[1].iceServers);
      rx.ontrack = ({ track: received }) => {
        if (this.stopped) { received.stop(); return; }
        this.stream = new MediaStream([received]);
        received.onended = () => { if (!this.stopped) this.failed(); };
      };
      const subscription = await this.request<SessionDescriptionResponse>("subscribe", { trackId: publication.trackId }, joined[1].token);
      this.check();
      if (subscription.sessionDescription?.type !== "offer") throw new Error("Mic test receive negotiation failed.");
      await rx.setRemoteDescription(subscription.sessionDescription);
      await rx.setLocalDescription(await rx.createAnswer());
      this.check();
      await this.request("negotiate", { sessionDescription: await localDescription(rx) }, joined[1].token);
      await Promise.all(this.peers.map((pc) => waitFor(pc, "connectionstatechange", 12_000, () => this.stopped || pc.connectionState === "connected")));
      this.check();
      if (!this.stream) throw new Error("No received microphone audio.");
      return this.stream;
    } catch (error) {
      this.stop();
      throw error;
    }
  }

  async replaceTrack(track: MediaStreamTrack) {
    this.check();
    if (!this.sender) throw new Error("Mic test is still connecting.");
    await this.sender.replaceTrack(track);
    this.check();
  }

  stop() {
    if (this.stopped) return;
    this.stopped = true;
    window.clearInterval(this.heartbeat);
    for (const pc of this.peers) pc.close();
    this.stream?.getTracks().forEach((track) => track.stop());
    for (const token of this.tokens) void this.request("leave", {}, token).catch(() => undefined);
    this.tokens.clear();
  }
}
