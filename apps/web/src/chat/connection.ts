import { appGateway, type AppGateway, type GatewaySubscription } from "../gateway/client.ts";
import { isChatAuthor, isChatMessage, sequence, type ChatEvent, type ChatMessage, type ChatTypingEvent } from "./types.ts";

export interface ChatConnectionCallbacks {
  message: (message: ChatMessage) => "applied" | "buffered" | "duplicate" | "overflow";
  cursor: () => string;
  status: (online: boolean) => void;
  resync: () => void;
  typing?: (event: ChatTypingEvent) => void;
}

function parseEvent(value: unknown): ChatEvent {
  if (!value || typeof value !== "object") throw new Error("Invalid chat event.");
  const event = value as Partial<ChatEvent>;
  if (event.type === "resync_required") return { type: event.type };
  if (event.type === "ready" && typeof event.cursor === "string") {
    sequence(event.cursor);
    return { type: "ready", cursor: event.cursor };
  }
  if (event.type === "typing.updated" && typeof event.channelId === "string" && isChatAuthor(event.author)
    && typeof event.typing === "boolean" && typeof event.revision === "string") {
    sequence(event.revision);
    return event as ChatTypingEvent;
  }
  if (event.type === "message.created" && typeof event.channelId === "string" && typeof event.seq === "string" && isChatMessage(event.message)) {
    sequence(event.seq);
    if (event.message.seq !== event.seq || event.message.channelId !== event.channelId) throw new Error("Invalid chat event.");
    return event as ChatEvent;
  }
  throw new Error("Invalid chat event.");
}

/** A logical chat subscription on the shared application gateway. */
export class ChatConnection {
  private subscription?: GatewaySubscription;
  private stopped = false;
  private readonly channelId: string;
  private readonly callbacks: ChatConnectionCallbacks;
  private readonly gateway: AppGateway;

  constructor(
    channelId: string,
    callbacks: ChatConnectionCallbacks,
    gateway: AppGateway = appGateway(),
  ) {
    this.channelId = channelId;
    this.callbacks = callbacks;
    this.gateway = gateway;
  }

  start() {
    if (this.subscription || this.stopped) return;
    this.subscription = this.gateway.subscribe({
      kind: "chat", channelId: this.channelId, after: this.callbacks.cursor(),
    }, {
      cursor: this.callbacks.cursor,
      status: this.callbacks.status,
      error: () => this.callbacks.resync(),
      event: (value) => {
        let event: ChatEvent;
        try { event = parseEvent(value); }
        catch { this.callbacks.resync(); return; }
        if (event.type === "resync_required") { this.callbacks.resync(); return; }
        if (event.type === "ready") return;
        if (event.type === "typing.updated") {
          if (event.channelId === this.channelId) this.callbacks.typing?.(event);
          return;
        }
        if (event.type !== "message.created" || event.channelId !== this.channelId) {
          this.callbacks.resync();
          return;
        }
        if (this.callbacks.message(event.message) === "overflow") this.callbacks.resync();
      },
    });
    void this.subscription.ready.catch(() => undefined);
  }

  stop() {
    this.stopped = true;
    this.subscription?.unsubscribe();
    this.subscription = undefined;
  }
}
