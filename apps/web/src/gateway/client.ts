const GATEWAY_PATH = "/api/chat/events";
const HEARTBEAT_INTERVAL_MS = 10_000;
const HEARTBEAT_TIMEOUT_MS = 30_000;
const OPEN_TIMEOUT_MS = 10_000;
const IDLE_CLOSE_MS = 1_000;
const ACTIVITY_THROTTLE_MS = 1_000;
const MAX_COMMAND_TIMEOUT_MS = 35_000;
const MAX_SUBSCRIPTION_RETRIES = 8;

export type PresenceStatus = "online" | "idle" | "offline";
export interface PresenceMember {
  userId: string;
  status: PresenceStatus;
}
export type SubscriptionKind = "chat" | "media" | "presence";

export interface SubscriptionRequest {
  kind: SubscriptionKind;
  channelId?: string;
  after?: string;
  token?: string;
  spaceId?: string;
  userIds?: string[];
}

export interface SubscriptionCallbacks {
  event: (event: unknown) => void;
  status?: (online: boolean) => void;
  error?: (error: GatewayError) => void;
  cursor?: () => string;
}

export interface GatewaySubscription {
  readonly id: string;
  readonly ready: Promise<void>;
  unsubscribe(): void;
}

export interface CommandRequest {
  method: string;
  body?: object;
  channelId?: string;
  token?: string;
  chatToken?: string;
  timeoutMs?: number;
  signal?: AbortSignal;
}

interface SocketLike {
  send(data: string): void;
  close(code?: number, reason?: string): void;
  addEventListener(type: "open" | "message" | "close" | "error", listener: (event: Event | MessageEvent) => void): void;
}

type SocketFactory = (url: string) => SocketLike;

interface StreamSubscription {
  subscribed: boolean;
  position?: bigint;
  snapshotRevision?: number;
  pendingEvent?: unknown;
}

interface Stream {
  socket: SocketLike;
  hello: boolean;
  serverOffsetMs: number;
  closed: boolean;
  subscriptions: Map<string, StreamSubscription>;
  openTimer?: ReturnType<typeof setTimeout>;
  heartbeatTimer?: ReturnType<typeof setInterval>;
  watchdog?: ReturnType<typeof setTimeout>;
}

interface LogicalSubscription {
  id: string;
  request: SubscriptionRequest;
  callbacks: SubscriptionCallbacks;
  revision?: number;
  readySettled: boolean;
  retries: number;
  retryTimer?: ReturnType<typeof setTimeout>;
  resolve: () => void;
  reject: (error: Error) => void;
}

interface PendingCommand {
  frame: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
  timer: ReturnType<typeof setTimeout>;
  deadline: number;
  retries: number;
  retryTimer?: ReturnType<typeof setTimeout>;
  signal?: AbortSignal;
  abort?: () => void;
}

export class GatewayError extends Error {
  readonly status: number;
  readonly code?: string;

  constructor(status: number, message: string, code?: string) {
    super(message);
    this.status = status;
    this.code = code;
  }
}

let lastActivityAt = Date.now();
let singleton: AppGateway | undefined;

function socketUrl() {
  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  return new URL(GATEWAY_PATH, `${protocol}//${window.location.host}`).toString();
}

function sequence(value: string) {
  if (!/^(0|[1-9]\d*)$/.test(value)) throw new Error("Invalid chat sequence.");
  return BigInt(value);
}

function frameData(data: unknown): Record<string, unknown> {
  if (typeof data !== "string" || data.length > 256 * 1024) throw new Error("Invalid gateway frame.");
  const value: unknown = JSON.parse(data);
  if (!value || typeof value !== "object") throw new Error("Invalid gateway frame.");
  return value as Record<string, unknown>;
}

function errorBody(body: unknown, fallback: string) {
  if (!body || typeof body !== "object") return { message: fallback, code: undefined };
  const detail = body as { error?: unknown; code?: unknown };
  return {
    message: typeof detail.error === "string" ? detail.error : fallback,
    code: typeof detail.code === "string" ? detail.code : undefined,
  };
}

export class AppGateway {
  private active?: Stream;
  private candidate?: Stream;
  private reconnectTimer?: ReturnType<typeof setTimeout>;
  private idleTimer?: ReturnType<typeof setTimeout>;
  private activityTimer?: ReturnType<typeof setTimeout>;
  private reconnects = 0;
  private idleTimeoutMs = 600_000;
  private readonly subscriptions = new Map<string, LogicalSubscription>();
  private readonly commands = new Map<string, PendingCommand>();
  private readonly socketFactory: SocketFactory;
  private readonly random: () => number;

  constructor(
    socketFactory: SocketFactory = (url) => new WebSocket(url),
    random: () => number = Math.random,
  ) {
    this.socketFactory = socketFactory;
    this.random = random;
    if (typeof window !== "undefined" && typeof window.addEventListener === "function") {
      window.addEventListener("pointerdown", this.activity, { passive: true });
      window.addEventListener("pointermove", this.activity, { passive: true });
      window.addEventListener("keydown", this.activity);
      window.addEventListener("touchstart", this.activity, { passive: true });
    }
  }

  subscribe(request: SubscriptionRequest, callbacks: SubscriptionCallbacks): GatewaySubscription {
    if (request.kind === "presence" && (request.userIds?.length ?? 0) > 100) {
      throw new Error("Presence subscriptions support at most 100 users.");
    }
    const id = crypto.randomUUID();
    let resolve!: () => void;
    let reject!: (error: Error) => void;
    const ready = new Promise<void>((yes, no) => { resolve = yes; reject = no; });
    const subscription: LogicalSubscription = {
      id, request, callbacks, readySettled: false, retries: 0, resolve, reject,
    };
    this.subscriptions.set(id, subscription);
    callbacks.status?.(false);
    clearTimeout(this.idleTimer);
    this.ensureConnected();
    if (this.active?.hello) this.sendSubscription(this.active, subscription);
    if (this.candidate?.hello) this.sendSubscription(this.candidate, subscription);
    return {
      id,
      ready,
      unsubscribe: () => this.unsubscribe(id),
    };
  }

  command(request: CommandRequest): Promise<unknown> {
    if (request.signal?.aborted) return Promise.reject(request.signal.reason);
    if (request.method === "typing" && !this.active?.hello) {
      return Promise.reject(new GatewayError(503, "Typing is unavailable while reconnecting."));
    }
    const id = crypto.randomUUID();
    const createdAt = Date.now();
    const timeoutMs = Math.min(request.timeoutMs ?? 25_000, MAX_COMMAND_TIMEOUT_MS);
    const frame = {
      type: "command", id, method: request.method,
      ...(request.channelId ? { channelId: request.channelId } : {}),
      ...(request.token ? { token: request.token } : {}),
      ...(request.chatToken ? { chatToken: request.chatToken } : {}),
      body: request.body ?? {},
    };
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.finishCommand(id, undefined, new DOMException("The request timed out.", "AbortError"));
      }, timeoutMs);
      const pending: PendingCommand = {
        frame, resolve, reject, timer, signal: request.signal,
        deadline: createdAt + timeoutMs, retries: 0,
      };
      if (request.signal) {
        pending.abort = () => this.finishCommand(id, undefined, request.signal!.reason);
        request.signal.addEventListener("abort", pending.abort, { once: true });
      }
      this.commands.set(id, pending);
      clearTimeout(this.idleTimer);
      this.ensureConnected();
      if (this.active?.hello) this.sendCommand(this.active, pending);
    });
  }

  /** This tab's activity, not cross-device account presence. No new subscription. */
  localPresence(connected: boolean): PresenceStatus {
    if (!connected) return "offline";
    return Date.now() - lastActivityAt >= this.idleTimeoutMs ? "idle" : "online";
  }

  reportActivity() {
    lastActivityAt = Date.now();
    if (this.activityTimer) return;
    this.activityTimer = setTimeout(() => {
      this.activityTimer = undefined;
      if (this.active?.hello) this.sendActivity(this.active);
    }, ACTIVITY_THROTTLE_MS);
  }

  destroy() {
    clearTimeout(this.reconnectTimer);
    clearTimeout(this.idleTimer);
    clearTimeout(this.activityTimer);
    this.reconnectTimer = undefined;
    this.idleTimer = undefined;
    this.activityTimer = undefined;
    for (const subscription of this.subscriptions.values()) {
      clearTimeout(subscription.retryTimer);
      if (!subscription.readySettled) subscription.reject(new DOMException("Gateway closed.", "AbortError"));
    }
    this.subscriptions.clear();
    for (const id of [...this.commands.keys()]) this.finishCommand(id, undefined, new DOMException("Gateway closed.", "AbortError"));
    clearTimeout(this.idleTimer);
    this.idleTimer = undefined;
    this.close(this.active, "gateway closed");
    this.close(this.candidate, "gateway closed");
    this.active = undefined;
    this.candidate = undefined;
    if (typeof window !== "undefined" && typeof window.removeEventListener === "function") {
      window.removeEventListener("pointerdown", this.activity);
      window.removeEventListener("pointermove", this.activity);
      window.removeEventListener("keydown", this.activity);
      window.removeEventListener("touchstart", this.activity);
    }
  }

  private activity = () => this.reportActivity();

  private unsubscribe(id: string) {
    const subscription = this.subscriptions.get(id);
    if (!subscription) return;
    this.subscriptions.delete(id);
    clearTimeout(subscription.retryTimer);
    if (!subscription.readySettled) subscription.reject(new DOMException("Subscription cancelled.", "AbortError"));
    if (this.active?.hello) this.send(this.active, { type: "unsubscribe", id });
    if (this.candidate?.hello) this.send(this.candidate, { type: "unsubscribe", id });
    this.scheduleIdleClose();
  }

  private ensureConnected() {
    if (typeof window === "undefined" || this.active || this.reconnectTimer) return;
    this.open(false);
  }

  private open(replacement: boolean) {
    if (replacement ? this.candidate : this.active) return;
    let socket: SocketLike;
    try { socket = this.socketFactory(socketUrl()); }
    catch { this.scheduleReconnect(replacement); return; }
    const stream: Stream = { socket, hello: false, serverOffsetMs: 0, closed: false, subscriptions: new Map() };
    if (replacement) this.candidate = stream;
    else this.active = stream;
    stream.openTimer = setTimeout(() => this.failed(stream), OPEN_TIMEOUT_MS);
    socket.addEventListener("message", (event) => {
      if (stream.closed) return;
      try { this.receive(stream, frameData((event as MessageEvent).data)); }
      catch { this.failed(stream); }
    });
    socket.addEventListener("close", () => this.failed(stream));
    socket.addEventListener("error", () => undefined);
  }

  private receive(stream: Stream, frame: Record<string, unknown>) {
    if (frame.type === "hello") {
      if (stream.hello || typeof frame.idleTimeoutSeconds !== "number"
        || typeof frame.serverTime !== "number" || !Number.isFinite(frame.serverTime)) {
        throw new Error("Invalid gateway hello.");
      }
      stream.hello = true;
      this.idleTimeoutMs = frame.idleTimeoutSeconds * 1_000;
      stream.serverOffsetMs = frame.serverTime - Date.now();
      clearTimeout(stream.openTimer);
      this.armWatchdog(stream);
      stream.heartbeatTimer = setInterval(() => this.sendHeartbeat(stream), HEARTBEAT_INTERVAL_MS);
      this.sendHeartbeat(stream);
      for (const subscription of this.subscriptions.values()) this.sendSubscription(stream, subscription);
      if (stream === this.active) this.sendPendingCommands(stream);
      this.maybePromote(stream);
      return;
    }
    if (!stream.hello) throw new Error("Gateway frame received before hello.");
    if (frame.type === "heartbeat") { this.armWatchdog(stream); return; }
    if (frame.type === "migrating") {
      if (stream === this.active && !this.candidate) this.open(true);
      else if (stream === this.candidate) this.failed(stream);
      return;
    }
    if (frame.type === "subscribed" && typeof frame.id === "string") {
      const subscription = this.subscriptions.get(frame.id);
      if (!subscription) return;
      const state = stream.subscriptions.get(frame.id);
      if (!state || state.subscribed) return;
      state.subscribed = true;
      clearTimeout(subscription.retryTimer);
      subscription.retryTimer = undefined;
      subscription.retries = 0;
      if (stream === this.active) {
        subscription.callbacks.status?.(true);
        if (!subscription.readySettled) { subscription.readySettled = true; subscription.resolve(); }
      }
      this.maybePromote(stream);
      return;
    }
    if (frame.type === "event" && typeof frame.id === "string" && "event" in frame) {
      this.receiveEvent(stream, frame.id, frame.event);
      return;
    }
    if (frame.type === "error" && typeof frame.id === "string" && typeof frame.status === "number" && typeof frame.error === "string") {
      const error = new GatewayError(frame.status, frame.error);
      const subscription = this.subscriptions.get(frame.id);
      if (!subscription) return;
      if (stream === this.candidate) { this.failed(stream); return; }
      if (stream !== this.active) return;
      subscription.callbacks.status?.(false);
      if (frame.status === 503 && subscription.retries < MAX_SUBSCRIPTION_RETRIES) {
        this.retrySubscription(subscription);
        return;
      }
      this.terminateSubscription(subscription, error);
      return;
    }
    if (frame.type === "result" && typeof frame.id === "string" && typeof frame.status === "number") {
      const pending = this.commands.get(frame.id);
      if (!pending) return;
      if (frame.status >= 200 && frame.status < 300) this.finishCommand(frame.id, frame.body);
      else {
        const detail = errorBody(frame.body, `Call service returned ${frame.status}.`);
        if (pending.frame.method !== "typing" && ((frame.status === 409 && detail.code === "command_pending")
          || (frame.status === 503 && detail.code === "gateway_draining"))) {
          if (detail.code === "gateway_draining" && !this.candidate) this.open(true);
          this.retryCommand(frame.id, detail.code === "command_pending" ? 100 : 250);
        } else {
          this.finishCommand(frame.id, undefined, new GatewayError(frame.status, detail.message, detail.code));
        }
      }
      return;
    }
    throw new Error("Invalid gateway frame.");
  }

  private receiveEvent(stream: Stream, id: string, event: unknown) {
    const subscription = this.subscriptions.get(id);
    const state = stream.subscriptions.get(id);
    if (!subscription || !state) return;
    if (!event || typeof event !== "object") throw new Error("Invalid gateway event.");
    const value = event as Record<string, unknown>;
    if (subscription.request.kind === "chat") {
      if (value.type === "message.created") {
        if (typeof value.seq !== "string") throw new Error("Invalid chat event.");
        const next = sequence(value.seq);
        if (state.position !== undefined && next > state.position + 1n) throw new Error("Chat event gap.");
        if (state.position === undefined || next > state.position) state.position = next;
      } else if (value.type === "ready") {
        if (typeof value.cursor !== "string") throw new Error("Invalid chat checkpoint.");
        const checkpoint = sequence(value.cursor);
        if (state.position !== undefined && checkpoint !== state.position) throw new Error("Invalid chat checkpoint.");
        state.position = checkpoint;
      }
    } else if (subscription.request.kind === "presence" && stream === this.candidate) {
      state.pendingEvent = event;
      this.maybePromote(stream);
      return;
    } else if (value.type === "snapshot" && typeof value.revision === "number") {
      if (!Number.isSafeInteger(value.revision) || value.revision < 0) throw new Error("Invalid snapshot revision.");
      state.snapshotRevision = value.revision;
      if (subscription.revision !== undefined && value.revision <= subscription.revision) {
        this.maybePromote(stream);
        return;
      }
      subscription.revision = value.revision;
    }
    subscription.callbacks.event(event);
    this.maybePromote(stream);
  }

  private sendSubscription(stream: Stream, subscription: LogicalSubscription) {
    const after = subscription.request.kind === "chat"
      ? subscription.callbacks.cursor?.() ?? subscription.request.after
      : subscription.request.after;
    const position = after === undefined ? undefined : sequence(after);
    stream.subscriptions.set(subscription.id, { subscribed: false, position });
    this.send(stream, {
      type: "subscribe", id: subscription.id, ...subscription.request,
      ...(after === undefined ? {} : { after }),
    });
  }

  private retrySubscription(subscription: LogicalSubscription) {
    if (subscription.retryTimer) return;
    const delay = Math.round(Math.min(250 * 2 ** subscription.retries++, 4_000) * (.75 + this.random() * .5));
    subscription.retryTimer = setTimeout(() => {
      subscription.retryTimer = undefined;
      if (!this.subscriptions.has(subscription.id)) return;
      if (this.active?.hello) this.sendSubscription(this.active, subscription);
    }, delay);
  }

  private terminateSubscription(subscription: LogicalSubscription, error: GatewayError) {
    if (!this.subscriptions.delete(subscription.id)) return;
    clearTimeout(subscription.retryTimer);
    subscription.retryTimer = undefined;
    if (this.active?.hello) this.send(this.active, { type: "unsubscribe", id: subscription.id });
    if (this.candidate?.hello) this.send(this.candidate, { type: "unsubscribe", id: subscription.id });
    subscription.callbacks.error?.(error);
    if (!subscription.readySettled) {
      subscription.readySettled = true;
      subscription.reject(error);
    }
    this.scheduleIdleClose();
  }

  private sendHeartbeat(stream: Stream) {
    this.send(stream, { type: "heartbeat", activityAgeMs: Math.max(0, Date.now() - lastActivityAt) });
  }

  private sendActivity(stream: Stream) {
    this.send(stream, { type: "activity", activityAgeMs: Math.max(0, Date.now() - lastActivityAt) });
  }

  private send(stream: Stream, value: unknown) {
    if (!stream.closed) stream.socket.send(JSON.stringify(value));
  }

  private armWatchdog(stream: Stream) {
    clearTimeout(stream.watchdog);
    stream.watchdog = setTimeout(() => this.failed(stream), HEARTBEAT_TIMEOUT_MS);
  }

  private maybePromote(stream: Stream) {
    if (stream !== this.candidate || !stream.hello) return;
    for (const subscription of this.subscriptions.values()) {
      const candidateState = stream.subscriptions.get(subscription.id);
      if (!candidateState?.subscribed) return;
      if (subscription.request.kind === "chat") {
        const activePosition = this.active?.subscriptions.get(subscription.id)?.position;
        const applied = sequence(subscription.callbacks.cursor?.() ?? subscription.request.after ?? "0");
        if ((candidateState.position ?? -1n) < applied || (activePosition !== undefined && (candidateState.position ?? -1n) < activePosition)) return;
      } else if (subscription.request.kind === "media") {
        if (candidateState.snapshotRevision === undefined
          || (subscription.revision !== undefined && candidateState.snapshotRevision < subscription.revision)) return;
      } else if (candidateState.pendingEvent === undefined) {
        return;
      }
    }
    for (const subscription of this.subscriptions.values()) {
      if (subscription.request.kind !== "presence") continue;
      const state = stream.subscriptions.get(subscription.id)!;
      subscription.callbacks.event(state.pendingEvent);
    }
    const previous = this.active;
    this.active = stream;
    this.candidate = undefined;
    this.reconnects = 0;
    for (const subscription of this.subscriptions.values()) {
      subscription.callbacks.status?.(true);
      if (!subscription.readySettled) {
        subscription.readySettled = true;
        subscription.resolve();
      }
    }
    this.sendPendingCommands(stream);
    this.close(previous, "gateway migrated");
  }

  private failed(stream: Stream) {
    if (stream.closed) return;
    const active = stream === this.active;
    const candidate = stream === this.candidate;
    this.close(stream, "gateway disconnected");
    if (candidate) {
      this.candidate = undefined;
      if (this.active && !this.active.closed) this.scheduleReconnect(true);
      else this.scheduleReconnect(false);
      return;
    }
    if (!active) return;
    this.active = undefined;
    for (const subscription of this.subscriptions.values()) subscription.callbacks.status?.(false);
    if (this.candidate) {
      this.maybePromote(this.candidate);
      if (this.active) return;
    }
    this.scheduleReconnect(false);
  }

  private scheduleReconnect(replacement: boolean) {
    if (this.reconnectTimer || (!this.subscriptions.size && !this.commands.size)) return;
    const delay = Math.round(Math.min(250 * 2 ** Math.min(this.reconnects++, 5), 5_000) * (.75 + this.random() * .5));
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = undefined;
      this.open(replacement && !!this.active);
    }, delay);
  }

  private finishCommand(id: string, value?: unknown, error?: unknown) {
    const pending = this.commands.get(id);
    if (!pending) return;
    this.commands.delete(id);
    clearTimeout(pending.timer);
    clearTimeout(pending.retryTimer);
    if (pending.abort) pending.signal?.removeEventListener("abort", pending.abort);
    if (error !== undefined) pending.reject(error);
    else pending.resolve(value);
    this.scheduleIdleClose();
  }

  private retryCommand(id: string, baseDelay: number) {
    const pending = this.commands.get(id);
    if (!pending || pending.retryTimer) return;
    const delay = Math.min(baseDelay * 2 ** Math.min(pending.retries++, 3), 1_000);
    if (Date.now() + delay >= pending.deadline) return;
    pending.retryTimer = setTimeout(() => {
      pending.retryTimer = undefined;
      if (!this.candidate && this.active?.hello && Date.now() < pending.deadline) this.sendCommand(this.active, pending);
    }, delay);
  }

  private sendPendingCommands(stream: Stream) {
    const now = Date.now();
    for (const [id, command] of this.commands) {
      if (command.frame.method === "typing") {
        this.finishCommand(id, undefined, new GatewayError(503, "Typing update discarded during reconnect."));
      } else if (now < command.deadline) this.sendCommand(stream, command);
    }
  }

  private sendCommand(stream: Stream, command: PendingCommand) {
    command.frame.issuedAt ??= Math.round(Date.now() + stream.serverOffsetMs);
    this.send(stream, command.frame);
  }

  private scheduleIdleClose() {
    if (this.subscriptions.size || this.commands.size || this.idleTimer) return;
    this.idleTimer = setTimeout(() => {
      this.idleTimer = undefined;
      if (this.subscriptions.size || this.commands.size) return;
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = undefined;
      this.close(this.active, "gateway idle");
      this.close(this.candidate, "gateway idle");
      this.active = undefined;
      this.candidate = undefined;
    }, IDLE_CLOSE_MS);
  }

  private close(stream: Stream | undefined, reason: string) {
    if (!stream || stream.closed) return;
    stream.closed = true;
    clearTimeout(stream.openTimer);
    clearTimeout(stream.watchdog);
    clearInterval(stream.heartbeatTimer);
    stream.socket.close(1000, reason);
  }
}

export function appGateway() {
  if (typeof window === "undefined") throw new Error("The application gateway is only available in the browser.");
  return singleton ??= new AppGateway();
}

export function reportActivity() {
  lastActivityAt = Date.now();
  singleton?.reportActivity();
}

export function watchPresence(
  spaceId: string,
  userIds: string[],
  onMembers: (members: PresenceMember[]) => void,
  onLive?: (online: boolean) => void,
) {
  if (userIds.length > 100) throw new Error("Presence subscriptions support at most 100 users.");
  const subscription = appGateway().subscribe({ kind: "presence", spaceId, userIds: [...userIds] }, {
    status: onLive,
    event: (value) => {
      if (!value || typeof value !== "object") throw new Error("Invalid presence snapshot.");
      const event = value as { type?: unknown; members?: unknown };
      if (event.type !== "snapshot" || !Array.isArray(event.members)) throw new Error("Invalid presence snapshot.");
      const members = event.members.filter((member): member is PresenceMember => {
        if (!member || typeof member !== "object") return false;
        const item = member as Partial<PresenceMember>;
        return typeof item.userId === "string" && ["online", "idle", "offline"].includes(item.status ?? "");
      });
      if (members.length !== event.members.length) throw new Error("Invalid presence snapshot.");
      onMembers(members);
    },
  });
  void subscription.ready.catch(() => undefined);
  return () => subscription.unsubscribe();
}

/** Test-only singleton replacement. */
export function setAppGatewayForTests(value: AppGateway | undefined) {
  singleton = value;
}
