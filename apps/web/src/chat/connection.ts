import { isChatMessage, sequence, type ChatEvent, type ChatMessage } from "./types.ts";

const OPEN_TIMEOUT_MS = 10_000;
const MAX_HANDOFF_RETRIES = 8;

interface SocketLike {
  close(code?: number, reason?: string): void;
  addEventListener(type: "open" | "message" | "close" | "error", listener: (event: Event | MessageEvent) => void): void;
}

interface Stream {
  socket: SocketLike;
  position: bigint;
  ready: boolean;
  closed: boolean;
  watchdog?: ReturnType<typeof setTimeout>;
}

export interface ChatConnectionCallbacks {
  message: (message: ChatMessage) => "applied" | "buffered" | "duplicate" | "overflow";
  cursor: () => string;
  status: (online: boolean) => void;
  resync: () => void;
}

type SocketFactory = (url: string) => SocketLike;

function websocketUrl(channelId: string, cursor: string) {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const url = new URL("/api/chat/events", `${protocol}//${window.location.host}`);
  url.searchParams.set("channelId", channelId);
  url.searchParams.set("after", cursor);
  return url.toString();
}

function parseFrame(data: unknown): ChatEvent {
  if (typeof data !== "string" || data.length > 64 * 1024) throw new Error("Invalid chat event.");
  const value = JSON.parse(data) as Partial<ChatEvent>;
  if (value.type === "migrating" || value.type === "resync_required") return { type: value.type };
  if (value.type === "ready" && typeof value.cursor === "string") {
    sequence(value.cursor);
    return { type: "ready", cursor: value.cursor };
  }
  if (value.type === "message.created" && typeof value.channelId === "string" && typeof value.seq === "string" && isChatMessage(value.message)) {
    sequence(value.seq);
    if (value.message.seq !== value.seq || value.message.channelId !== value.channelId) throw new Error("Invalid chat event.");
    return value as ChatEvent;
  }
  throw new Error("Invalid chat event.");
}

export class ChatConnection {
  private active?: Stream;
  private candidate?: Stream;
  private stopped = false;
  private retries = 0;
  private reconnects = 0;
  private retryTimer?: ReturnType<typeof setTimeout>;
  private readonly channelId: string;
  private readonly callbacks: ChatConnectionCallbacks;
  private readonly socketFactory: SocketFactory;
  private readonly random: () => number;

  constructor(
    channelId: string,
    callbacks: ChatConnectionCallbacks,
    socketFactory: SocketFactory = (url) => new WebSocket(url),
    random: () => number = Math.random,
  ) {
    this.channelId = channelId;
    this.callbacks = callbacks;
    this.socketFactory = socketFactory;
    this.random = random;
  }

  start() {
    if (!this.active && !this.stopped) this.open(false);
  }

  stop() {
    this.stopped = true;
    clearTimeout(this.retryTimer);
    this.close(this.active);
    this.close(this.candidate);
    this.active = undefined;
    this.candidate = undefined;
  }

  private open(replacement: boolean) {
    if (this.stopped || (replacement ? this.candidate : this.active)) return;
    const start = sequence(this.callbacks.cursor());
    let socket: SocketLike;
    try { socket = this.socketFactory(websocketUrl(this.channelId, start.toString())); }
    catch { this.schedule(replacement); return; }
    const stream: Stream = { socket, position: start, ready: false, closed: false };
    if (replacement) this.candidate = stream;
    else this.active = stream;
    stream.watchdog = setTimeout(() => this.failed(stream), OPEN_TIMEOUT_MS);
    socket.addEventListener("message", (event) => {
      if (stream.closed || this.stopped) return;
      try { this.receive(stream, parseFrame((event as MessageEvent).data)); }
      catch { this.callbacks.resync(); }
    });
    socket.addEventListener("close", () => this.failed(stream));
    socket.addEventListener("error", () => undefined);
  }

  private receive(stream: Stream, event: ChatEvent) {
    if (event.type === "resync_required") { this.callbacks.resync(); return; }
    if (event.type === "migrating") {
      if (stream === this.candidate) this.failed(stream);
      else if (stream === this.active && !this.candidate) this.open(true);
      return;
    }
    if (event.type === "message.created") {
      if (event.channelId !== this.channelId) throw new Error("Invalid chat channel.");
      const next = sequence(event.seq);
      if (next > stream.position + 1n) throw new Error("Chat event gap.");
      if (next === stream.position + 1n) stream.position = next;
      if (this.callbacks.message(event.message) === "overflow") { this.callbacks.resync(); return; }
      this.maybePromote(stream);
      return;
    }
    const checkpoint = sequence(event.cursor);
    if (checkpoint < stream.position || checkpoint > stream.position) {
      // Ordered replay must deliver every sequence before its checkpoint.
      this.callbacks.resync();
      return;
    }
    stream.ready = true;
    if (stream === this.active) {
      clearTimeout(stream.watchdog);
      stream.watchdog = undefined;
      this.reconnects = 0;
      this.callbacks.status(true);
    }
    this.maybePromote(stream);
  }

  private maybePromote(stream: Stream) {
    if (stream !== this.candidate || !stream.ready || stream.position < sequence(this.callbacks.cursor())) return;
    const previous = this.active;
    this.active = stream;
    this.candidate = undefined;
    clearTimeout(stream.watchdog);
    stream.watchdog = undefined;
    this.retries = 0;
    this.reconnects = 0;
    this.callbacks.status(true);
    this.close(previous);
  }

  private failed(stream: Stream) {
    if (stream.closed || this.stopped) return;
    const wasCandidate = stream === this.candidate;
    const wasActive = stream === this.active;
    this.close(stream);
    if (wasCandidate) {
      this.candidate = undefined;
      if (this.active && !this.active.closed) this.schedule(true);
      else if (!this.active) { this.callbacks.status(false); this.schedule(false); }
      return;
    }
    if (!wasActive) return;
    this.active = undefined;
    if (this.candidate) {
      this.maybePromote(this.candidate);
      if (this.active) return;
      return;
    }
    this.callbacks.status(false);
    this.schedule(false);
  }

  private schedule(replacement: boolean) {
    if (this.stopped || this.retryTimer) return;
    if (replacement && this.retries >= MAX_HANDOFF_RETRIES) return;
    const attempt = replacement ? this.retries++ : this.reconnects++;
    const base = Math.min(250 * 2 ** Math.min(attempt, 4), 4_000);
    const delay = Math.round(base * (.75 + this.random() * .5));
    this.retryTimer = setTimeout(() => {
      this.retryTimer = undefined;
      this.open(replacement);
    }, delay);
  }

  private close(stream?: Stream) {
    if (!stream || stream.closed) return;
    stream.closed = true;
    clearTimeout(stream.watchdog);
    stream.socket.close(1000, "chat stream replaced");
  }
}
