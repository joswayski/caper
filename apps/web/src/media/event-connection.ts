import { appGateway, type GatewaySubscription } from "../gateway/client.ts";
import { callSnapshot } from "./events.ts";
import type { CallSnapshot } from "./types.ts";

function channelFromRoot(apiRoot: string) {
  const match = /^\/api\/channels\/([^/]+)\/media$/.exec(apiRoot);
  return match ? decodeURIComponent(match[1]) : undefined;
}

/** One logical media subscription on the shared application gateway. */
export class EventConnection {
  private subscription?: GatewaySubscription;
  private owner?: AbortSignal;
  private stopped = false;
  private wasConnected = false;
  private readonly changed: () => void;
  private readonly lost: (error: Error, draining: boolean) => void;
  private readonly snapshot: (value: CallSnapshot & { revision?: number }) => void;
  private readonly apiRoot: string;
  private readonly live?: (online: boolean) => void;
  connected = false;

  constructor(
    changed: () => void,
    lost: (error: Error, draining: boolean) => void,
    snapshot: (value: CallSnapshot & { revision?: number }) => void,
    _draining: () => void,
    apiRoot = "/api/media",
    live?: (online: boolean) => void,
  ) {
    this.changed = changed;
    this.lost = lost;
    this.snapshot = snapshot;
    this.apiRoot = apiRoot;
    this.live = live;
  }

  open(token: string, signal: AbortSignal) { return this.start(token, signal); }
  openPresence(signal: AbortSignal) { return this.start(undefined, signal); }

  private start(token: string | undefined, signal: AbortSignal) {
    if (signal.aborted) return Promise.reject(signal.reason);
    this.owner = signal;
    signal.addEventListener("abort", this.stop, { once: true });
    this.subscription = appGateway().subscribe({
      kind: "media", channelId: channelFromRoot(this.apiRoot), token,
    }, {
      status: (online) => {
        if (this.stopped) return;
        const changed = this.connected !== online;
        this.connected = online;
        if (changed) this.live?.(online);
        if (online) this.wasConnected = true;
        if (changed && this.wasConnected) this.changed();
      },
      error: (error) => { if (!this.stopped) this.lost(error, false); },
      event: (value) => {
        if (this.stopped) return;
        this.snapshot(callSnapshot(value, token === undefined));
      },
    });
    return this.subscription.ready.then(() => {
      if (signal.aborted) throw signal.reason;
    });
  }

  stop = () => {
    if (this.stopped) return;
    this.stopped = true;
    this.connected = false;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
    this.owner?.removeEventListener("abort", this.stop);
  };
}
