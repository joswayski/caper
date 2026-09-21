import { localDescription, withOpusDtx } from "./rtc.ts";
import type { TurnGeneration, TurnResponse } from "./types.ts";

type Request = <T>(operation: string, body: object, token: string, signal: AbortSignal) => Promise<T>;
type Serial = <T>(operation: () => Promise<T>) => Promise<T>;

const BACKOFF = [1_000, 2_000, 4_000, 8_000, 15_000, 30_000];

function wait(ms: number, signal: AbortSignal) {
  return new Promise<void>((resolve, reject) => {
    const timer = window.setTimeout(() => { signal.removeEventListener("abort", abort); resolve(); }, ms);
    const abort = () => { window.clearTimeout(timer); reject(signal.reason); };
    signal.addEventListener("abort", abort, { once: true });
    if (signal.aborted) abort();
  });
}

function status(error: unknown) {
  return typeof error === "object" && error !== null && "status" in error ? Number(error.status) : undefined;
}

function code(error: unknown) {
  return typeof error === "object" && error !== null && "code" in error ? error.code : undefined;
}

/** Renews one capability without replacing its peer, tracks, or surrounding call. */
export class TurnRenewal {
  private readonly controller = new AbortController();
  private readonly pc: RTCPeerConnection;
  private readonly token: string;
  private readonly request: Request;
  private readonly serialize: Serial;
  private readonly current: () => boolean;
  private readonly invalid: () => void;
  private timer?: number;
  private sequence = 1;
  private generation: string;

  constructor(
    pc: RTCPeerConnection,
    token: string,
    initial: TurnGeneration,
    request: Request,
    serialize: Serial,
    current: () => boolean,
    invalid: () => void,
    issuedAt: number,
  ) {
    this.pc = pc;
    this.token = token;
    this.request = request;
    this.serialize = serialize;
    this.current = current;
    this.invalid = invalid;
    this.generation = initial.generation;
    this.schedule(initial.refreshAfterMs - (performance.now() - issuedAt));
  }

  stop() {
    window.clearTimeout(this.timer);
    this.controller.abort(new DOMException("TURN renewal stopped", "AbortError"));
  }

  private live() {
    this.controller.signal.throwIfAborted();
    if (!this.current()) throw new DOMException("TURN renewal superseded", "AbortError");
  }

  private schedule(delay: number) {
    this.live();
    window.clearTimeout(this.timer);
    // Invalid/expired metadata must not form a zero-delay loop.
    this.timer = window.setTimeout(() => void this.renew(), Math.max(1_000, Number.isFinite(delay) ? delay : 30_000));
  }

  private async retry<T>(operation: () => Promise<T>): Promise<T> {
    for (let attempt = 0; ; attempt++) {
      this.live();
      try {
        const value = await operation();
        this.live();
        return value;
      } catch (error) {
        this.live();
        const http = status(error);
        if (http === 401 || http === 403 || code(error) === "ice_restart_invalid") throw error;
        // restart-ice is uniquely replay-safe. ACK is idempotent. A pending 409
        // means admission is busy and must not disturb the healthy transport.
        if (!((http === 409 && code(error) === "ice_restart_pending") || http === 408 || http === 429 || (http !== undefined && http >= 500) || http === undefined)) throw error;
        await wait(BACKOFF[Math.min(attempt, BACKOFF.length - 1)], this.controller.signal);
        this.live();
      }
    }
  }

  private async renew() {
    const started = performance.now();
    let signaling = false;
    try {
      // Credential minting intentionally stays outside the SDP serialization queue.
      const credentials = await this.retry(() => this.request<TurnResponse>(
        "turn", { generation: this.generation }, this.token, this.controller.signal,
      ));
      this.live();
      if (credentials.turn.generation === this.generation) {
        this.schedule(credentials.turn.refreshAfterMs - (performance.now() - started));
        return;
      }
      await this.serialize(async () => {
        this.live();
        signaling = true;
        this.pc.setConfiguration({ ...this.pc.getConfiguration(), iceServers: credentials.iceServers });
        const created = await this.pc.createOffer({ iceRestart: true });
        this.live();
        await this.pc.setLocalDescription(created);
        this.live();
        const offer = await localDescription(this.pc, this.controller.signal);
        this.live();
        const body = { generation: credentials.turn.generation, sequence: this.sequence, sessionDescription: offer };
        const answer = await this.retry(() => this.request<{ sessionDescription: RTCSessionDescriptionInit }>(
          "restart-ice", body, this.token, this.controller.signal,
        ));
        this.live();
        if (!answer.sessionDescription) throw new Error("The media service did not answer ICE restart.");
        await this.pc.setRemoteDescription(withOpusDtx(answer.sessionDescription));
        this.live();
        await this.retry(() => this.request<void>("restart-ice-ack", {
          generation: credentials.turn.generation, sequence: this.sequence,
        }, this.token, this.controller.signal));
        this.live();
        this.sequence++;
        this.generation = credentials.turn.generation;
      });
      this.live();
      this.schedule(credentials.turn.refreshAfterMs - (performance.now() - started));
    } catch (error) {
      if (this.controller.signal.aborted || !this.current()) return;
      const http = status(error);
      if (signaling || http === 401 || http === 403) {
        // A permanent signaling failure must use the owner's bounded recovery,
        // never create a different offer behind an unacknowledged server guard.
        this.stop();
        this.invalid();
        return;
      }
      // Credential acquisition has not changed the transport yet.
      this.schedule(BACKOFF[BACKOFF.length - 1]);
    }
  }
}
