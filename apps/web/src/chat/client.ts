import { ChatConnection } from "./connection.ts";
import { ChatTimeline } from "./timeline.ts";
import { isChatAuthor, isChatMessage, sequence, type ChatAuthor, type ChatHistory, type ChatMessage, type ChatSession, type ChatTypingEvent, type GeneralChatHistory, type ChatPresenceEvent } from "./types.ts";

const SESSION_KEY = "caper.chat.session";

export interface PendingChatMessage {
  clientMessageId: string;
  text: string;
  author?: ChatAuthor;
  createdAt: string;
}

export interface ChatViewState {
  phase: "loading" | "ready" | "error";
  online: boolean;
  spaceName: string;
  channelId?: string;
  channelName: string;
  messages: ChatMessage[];
  typingAuthors: ChatAuthor[];
  activeAuthorIds: string[];
  hasMore: boolean;
  loadingOlder: boolean;
  olderError?: string;
  author?: ChatAuthor;
  sessionError?: string;
  sendError?: string;
  sendRejected?: boolean;
  pendingSend?: PendingChatMessage;
  error?: string;
}

const initialState: ChatViewState = {
  phase: "loading", online: false, spaceName: "Caper", channelName: "general",
  messages: [], typingAuthors: [], activeAuthorIds: [], hasMore: false, loadingOlder: false,
};

export function initialChatView(history?: GeneralChatHistory, error?: string): ChatViewState {
  if (error) return { ...initialState, phase: "error", error };
  return history ? {
    ...initialState, phase: "ready", spaceName: history.space.name,
    channelId: history.channel.id, channelName: history.channel.name,
    messages: history.messages, hasMore: history.hasMore,
  } : initialState;
}

function apiError(response: Response, fallback: string) {
  return response.json().catch(() => undefined).then((body: { error?: unknown } | undefined) =>
    new Error(typeof body?.error === "string" ? body.error : fallback));
}

function validHistory(value: unknown, general: true): value is GeneralChatHistory;
function validHistory(value: unknown, general: false): value is ChatHistory;
function validHistory(value: unknown, general: boolean): value is ChatHistory | GeneralChatHistory {
  if (!value || typeof value !== "object") return false;
  const history = value as Partial<GeneralChatHistory>;
  try { if (typeof history.cursor !== "string") return false; sequence(history.cursor); } catch { return false; }
  return Array.isArray(history.messages) && history.messages.every(isChatMessage) && typeof history.hasMore === "boolean"
    && (!general || (!!history.space && typeof history.space.id === "string" && typeof history.space.name === "string"
      && !!history.channel && typeof history.channel.id === "string" && typeof history.channel.name === "string"));
}

export class ChatHistoryError extends Error {
  readonly status: number;
  constructor(status: number, message: string) { super(message); this.status = status; }
}

export async function loadChatHistory(channelId?: string, signal?: AbortSignal): Promise<GeneralChatHistory> {
  const path = channelId ? `/api/chat/channels/${encodeURIComponent(channelId)}/messages` : "/api/chat/general";
  const response = await fetch(path, { cache: "no-store", signal: AbortSignal.any([...(signal ? [signal] : []), AbortSignal.timeout(10_000)]) });
  if (!response.ok) throw new ChatHistoryError(response.status, (await apiError(response, "Messages are unavailable.")).message);
  const history: unknown = await response.json();
  if (!validHistory(history, true)) throw new Error("The chat service returned invalid history.");
  if (channelId && history.channel.id !== channelId) throw new Error("The chat service returned the wrong channel.");
  if (history.messages.some((message) => message.channelId !== history.channel.id)) throw new Error("The chat service returned messages from another channel.");
  return history;
}

function isPresenceEvent(value: unknown): value is ChatPresenceEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatPresenceEvent>;
  try { sequence(event.revision ?? ""); } catch { return false; }
  return event.type === "presence.updated" && isChatAuthor(event.author);
}

function storedSession(): ChatSession | undefined {
  try {
    const value = JSON.parse(localStorage.getItem(SESSION_KEY) ?? "null") as Partial<ChatSession> | null;
    if (value && typeof value.token === "string" && value.token && value.author && typeof value.author.id === "string"
      && typeof value.author.name === "string" && typeof value.author.isGuest === "boolean") return value as ChatSession;
  } catch { /* Storage is optional. */ }
  return undefined;
}

export class ChatClient {
  private state = initialState;
  private readonly timeline = new ChatTimeline();
  private connection?: ChatConnection;
  private readonly controller = new AbortController();
  private generation = 0;
  private name = "Guest";
  private session?: ChatSession;
  private sending = false;
  private confirmSend?: (message: ChatMessage) => void;
  private typingActive = false;
  private typingSent = false;
  private typingSentAt = 0;
  private typingRequest?: Promise<void>;
  private typingIdleTimer?: ReturnType<typeof setTimeout>;
  private typingExpiryTimer?: ReturnType<typeof setTimeout>;
  private presenceTimer?: ReturnType<typeof setTimeout>;
  private presenceExpiryTimer?: ReturnType<typeof setTimeout>;
  private readonly typers = new Map<string, { author: ChatAuthor; typing: boolean; revision: bigint; expires: number }>();
  private readonly activeAuthors = new Map<string, { revision: bigint; expires: number }>();
  private readonly changed: (state: ChatViewState) => void;
  private readonly channelId?: string;
  private spaceId?: string;

  constructor(changed: (state: ChatViewState) => void, channelId?: string) {
    this.changed = changed;
    this.channelId = channelId;
  }

  start(history?: GeneralChatHistory, error?: string) {
    if (typeof document !== "undefined") {
      document.addEventListener("visibilitychange", this.refreshPresence);
      window.addEventListener("focus", this.refreshPresence);
    }
    if (error) this.update({ phase: "error", error });
    else void this.loadInitial(history);
  }

  snapshotHistory(): GeneralChatHistory | undefined {
    if (this.state.phase !== "ready" || !this.spaceId || !this.state.channelId) return;
    return {
      space: { id: this.spaceId, name: this.state.spaceName },
      channel: { id: this.state.channelId, name: this.state.channelName },
      messages: this.timeline.messages, cursor: this.timeline.cursor, hasMore: this.state.hasMore,
    };
  }

  identify(name: string, signedIn = false) {
    this.name = name;
    const saved = storedSession();
    // Guest identity may persist. Account names are not unique: mint a fresh
    // capability from the current account cookie rather than matching by name.
    if (saved?.author.isGuest && !signedIn) {
      this.session = saved; this.update({ author: saved.author });
    }
    else void this.createSession();
  }

  stop() {
    this.generation++;
    this.controller.abort();
    clearTimeout(this.typingIdleTimer);
    clearTimeout(this.typingExpiryTimer);
    clearTimeout(this.presenceTimer);
    clearTimeout(this.presenceExpiryTimer);
    if (typeof document !== "undefined") {
      document.removeEventListener("visibilitychange", this.refreshPresence);
      window.removeEventListener("focus", this.refreshPresence);
    }
    this.connection?.stop();
  }

  setTyping(active: boolean) {
    if (this.controller.signal.aborted) return;
    clearTimeout(this.typingIdleTimer);
    this.typingActive = active;
    if (active) this.typingIdleTimer = setTimeout(() => this.setTyping(false), 500);
    this.flushTyping();
  }

  private flushTyping() {
    const channel = this.state.channelId;
    const session = this.session;
    if (this.typingRequest || !channel || !session || this.controller.signal.aborted) return;
    const active = this.typingActive;
    if (!active && !this.typingSent) return;
    if (active && this.typingSent && Date.now() - this.typingSentAt < 500) return;
    this.typingSent = active;
    this.typingSentAt = Date.now();
    // Serialize start/stop so a delayed start request cannot overtake its stop.
    // Presence is best-effort: failure must never block or fail a real message.
    this.typingRequest = fetch(`/api/chat/channels/${encodeURIComponent(channel)}/typing`, {
      method: "POST", headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
      body: JSON.stringify({ typing: active }),
      signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(2_000)]),
    }).then(() => undefined, () => undefined).finally(() => {
      this.typingRequest = undefined;
      if (this.typingActive !== active) this.flushTyping();
    });
  }

  private receiveTyping(event: ChatTypingEvent) {
    if (event.author.id === this.state.author?.id) return;
    const previous = this.typers.get(event.author.id);
    const revision = sequence(event.revision);
    if (previous && revision <= previous.revision) return;
    if (!previous && this.typers.size >= 64) return;
    // Retain stop tombstones briefly to reject delayed/duplicate handoff frames.
    this.typers.set(event.author.id, { author: event.author, typing: event.typing, revision, expires: Date.now() + 6_000 });
    this.refreshTypers();
  }

  private refreshTypers() {
    clearTimeout(this.typingExpiryTimer);
    const now = Date.now();
    for (const [id, entry] of this.typers) if (entry.expires <= now) this.typers.delete(id);
    const entries = [...this.typers.values()];
    this.update({ typingAuthors: entries.filter((entry) => entry.typing && entry.author.id !== this.state.author?.id).map((entry) => entry.author) });
    if (entries.length) this.typingExpiryTimer = setTimeout(() => this.refreshTypers(), Math.min(...entries.map((entry) => entry.expires)) - now);
  }

  private refreshPresence = () => {
    clearTimeout(this.presenceTimer);
    if (typeof document === "undefined" || document.visibilityState === "hidden" || !document.hasFocus()) return;
    const session = this.session;
    if (!session || this.controller.signal.aborted) return;
    void fetch("/api/chat/presence", {
      method: "POST", headers: { "content-type": "application/json", "x-caper-chat-token": session.token }, body: "{}",
      signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(2_000)]),
    }).catch(() => undefined);
    this.presenceTimer = setTimeout(this.refreshPresence, 30_000);
  };

  private receivePresence(event: ChatPresenceEvent) {
    const revision = sequence(event.revision);
    const previous = this.activeAuthors.get(event.author.id);
    if (previous && revision <= previous.revision) return;
    const expiresAt = (event as ChatPresenceEvent & { expiresAt?: unknown }).expiresAt;
    const expires = typeof expiresAt === "number" && Number.isFinite(expiresAt) ? expiresAt : Date.now() + 120_000;
    this.activeAuthors.set(event.author.id, { revision, expires });
    this.refreshActiveAuthors();
  }

  private refreshActiveAuthors() {
    clearTimeout(this.presenceExpiryTimer);
    const now = Date.now();
    for (const [id, entry] of this.activeAuthors) if (entry.expires <= now) this.activeAuthors.delete(id);
    this.update({ activeAuthorIds: [...this.activeAuthors.keys()] });
    const expires = [...this.activeAuthors.values()].map((entry) => entry.expires);
    if (expires.length) this.presenceExpiryTimer = setTimeout(() => this.refreshActiveAuthors(), Math.min(...expires) - now);
  }

  retryLoad() { void this.loadInitial(); }

  retrySession() { void this.createSession(); }

  discardRejected(): string | undefined {
    if (this.sending || !this.state.sendRejected) return;
    const text = this.state.pendingSend?.text;
    this.update({ pendingSend: undefined, sendError: undefined, sendRejected: undefined });
    return text;
  }

  async loadOlder() {
    if (this.state.phase !== "ready" || !this.state.channelId || !this.state.hasMore || this.state.loadingOlder || !this.state.messages.length || this.controller.signal.aborted) return;
    const generation = this.generation;
    this.update({ loadingOlder: true, olderError: undefined });
    try {
      const before = this.state.messages[0].seq;
      const response = await fetch(`/api/chat/channels/${encodeURIComponent(this.state.channelId)}/messages?before=${encodeURIComponent(before)}`, {
        cache: "no-store", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
      });
      if (!response.ok) throw await apiError(response, "Older messages could not be loaded.");
      const history: unknown = await response.json();
      if (!validHistory(history, false)) throw new Error("The chat service returned invalid history.");
      if (generation !== this.generation) return;
      this.timeline.prepend(history.messages);
      this.update({ messages: this.timeline.messages, hasMore: history.hasMore, loadingOlder: false });
    } catch (error) {
      if (!this.controller.signal.aborted && generation === this.generation) this.update({ loadingOlder: false, olderError: error instanceof Error ? error.message : "Older messages could not be loaded." });
    }
  }

  async send(text: string): Promise<boolean> {
    if (this.sending || this.state.sendRejected) return false;
    // A timeout is an unknown outcome. Enter/Send must retry the same command,
    // just like the explicit retry button, before allowing a new command.
    const pending = {
      ...(this.state.pendingSend ?? { clientMessageId: crypto.randomUUID(), text, createdAt: new Date().toISOString() }),
      author: this.session?.author,
    };
    const count = Array.from(pending.text).length;
    if (!pending.text.trim() || count > 4_000) throw new Error(count > 4_000 ? "Messages can be at most 4,000 characters." : "Write a message first.");
    if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(pending.text)) throw new Error("Messages cannot contain control characters.");
    const channelId = this.state.channelId;
    if (!channelId) throw new Error("Chat is not ready yet.");
    const session = this.session;
    if (!session) {
      this.update({ pendingSend: pending, sendError: "Your guest session is unavailable. Retry the session, then send again." });
      return false;
    }
    this.setTyping(false);
    this.sending = true;
    const confirmation = new Promise<ChatMessage>((resolve) => { this.confirmSend = resolve; });
    this.update({ pendingSend: pending, sendError: undefined, sendRejected: undefined });
    let rejected = false;
    try {
      // Either transport can prove acceptance. A late HTTP failure must not
      // undo an ordered WebSocket/history confirmation or block the next send.
      const request = (async () => {
        const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages`, {
          method: "POST",
          headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
          body: JSON.stringify({ clientMessageId: pending.clientMessageId, text: pending.text }),
          signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
        });
        if (!response.ok) {
          rejected = [400, 404, 409, 413, 422].includes(response.status);
          if ((response.status === 401 || response.status === 403) && this.state.pendingSend?.clientMessageId === pending.clientMessageId) {
            try { localStorage.removeItem(SESSION_KEY); } catch { /* Storage is optional. */ }
            this.session = undefined;
            void this.createSession();
          }
          throw await apiError(response, "Message could not be sent.");
        }
        const message: unknown = await response.json();
        if (!isChatMessage(message) || message.channelId !== channelId
          || message.clientMessageId !== pending.clientMessageId || message.author.id !== session.author.id) throw new Error("The chat service returned an invalid message.");
        return message;
      })();
      const message = await Promise.race([request, confirmation]);
      this.timeline.mergeSent(message);
      this.update({ messages: this.timeline.messages, pendingSend: undefined, sendError: undefined, sendRejected: undefined });
      return true;
    } catch (error) {
      // Confirmation can clear the command while an HTTP rejection is already
      // propagating through Promise.race, before this continuation runs.
      if (!this.state.pendingSend) return true;
      if (!this.controller.signal.aborted) this.update({ pendingSend: pending, sendRejected: rejected, sendError: error instanceof Error ? error.message : "Message could not be sent." });
      return false;
    } finally {
      this.sending = false;
      this.confirmSend = undefined;
    }
  }

  private async loadInitial(prepared?: GeneralChatHistory) {
    const generation = ++this.generation;
    const previous = this.state.phase === "ready" ? this.snapshotHistory() : undefined;
    this.connection?.stop();
    this.connection = undefined;
    this.typers.clear();
    if (!prepared) {
      this.refreshTypers();
      this.update({ phase: previous ? "ready" : "loading", online: false, error: undefined, loadingOlder: false, olderError: undefined });
    }
    try {
      const history = prepared ?? await loadChatHistory(this.channelId, this.controller.signal);
      if (!validHistory(history, true)) throw new Error("The chat service returned invalid history.");
      if (this.channelId && history.channel.id !== this.channelId) throw new Error("The chat service returned the wrong channel.");
      if (generation !== this.generation) return;
      this.spaceId = history.space.id;
      // Retain older pages only when the fresh page joins the saved range.
      // A resync beyond the replay window must not leave an unpageable gap.
      const contiguous = previous && history.messages[0]
        && sequence(history.messages[0].seq) <= sequence(previous.cursor) + 1n;
      const retainedOlder = previous?.messages[0] && history.messages[0]
        && contiguous
        && sequence(previous.messages[0].seq) < sequence(history.messages[0].seq);
      this.timeline.reset([...(contiguous ? previous.messages : []), ...history.messages], history.cursor);
      this.update({
        phase: "ready", spaceName: history.space.name, channelId: history.channel.id,
        channelName: history.channel.name, messages: this.timeline.messages, hasMore: retainedOlder ? previous.hasMore : history.hasMore,
      });
      void this.loadPresence(generation);
      this.connection = new ChatConnection(history.channel.id, {
        cursor: () => this.timeline.cursor,
        message: (message) => {
          const result = this.timeline.applyEvent(message);
          const typer = this.typers.get(message.author.id);
          if (typer && result !== "duplicate") { typer.typing = false; this.refreshTypers(); }
          if (result !== "buffered" && result !== "overflow") this.update({ messages: this.timeline.messages });
          return result;
        },
        status: (online) => {
          if (!online) { this.typers.clear(); this.refreshTypers(); }
          this.update({ online });
        },
        typing: (event) => this.receiveTyping(event),
        presence: (event) => this.receivePresence(event),
        resync: () => { if (generation === this.generation) void this.loadInitial(); },
      });
      this.connection.start();
    } catch (error) {
      if (!this.controller.signal.aborted && generation === this.generation) {
        const denied = error instanceof ChatHistoryError && [401, 403, 404].includes(error.status);
        if (denied) { this.timeline.reset([], "0"); this.spaceId = undefined; }
        this.update({
          phase: previous && !denied ? "ready" : "error", online: false,
          ...(denied ? { messages: [], channelId: undefined } : {}),
          error: error instanceof Error ? error.message : "Messages are unavailable.",
        });
      }
    }
  }

  private async loadPresence(generation: number) {
    try {
      const response = await fetch("/api/chat/presence", {
        cache: "no-store", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(5_000)]),
      });
      const value: unknown = response.ok ? await response.json() : undefined;
      if (generation !== this.generation || !value || typeof value !== "object" || !Array.isArray((value as { presence?: unknown }).presence)) return;
      for (const event of (value as { presence: unknown[] }).presence) if (isPresenceEvent(event)) this.receivePresence(event);
    } catch { /* Presence must not block chat history. */ }
  }

  private async createSession() {
    this.update({ sessionError: undefined });
    try {
      const response = await fetch("/api/chat/session", {
        method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name: this.name }), signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
      });
      if (!response.ok) throw await apiError(response, "Guest messaging is unavailable.");
      const session = await response.json() as Partial<ChatSession>;
      if (typeof session.token !== "string" || !session.token || !session.author || typeof session.author.id !== "string"
        || typeof session.author.name !== "string" || typeof session.author.isGuest !== "boolean") throw new Error("The chat service returned an invalid session.");
      this.session = session as ChatSession;
      try { localStorage.setItem(SESSION_KEY, JSON.stringify(session)); } catch { /* The in-memory response still permits this page to render. */ }
      this.update({ author: session.author, sessionError: undefined });
      this.refreshPresence();
    } catch (error) {
      if (!this.controller.signal.aborted) this.update({ sessionError: error instanceof Error ? error.message : "Guest messaging is unavailable." });
    }
  }

  private update(change: Partial<ChatViewState>) {
    this.state = { ...this.state, ...change };
    if (change.author) {
      this.typers.delete(change.author.id);
      this.state = { ...this.state, typingAuthors: this.state.typingAuthors.filter((author) => author.id !== change.author!.id) };
    }
    const pending = this.state.pendingSend;
    if (pending && change.messages) {
      const accepted = change.messages.find((message) => message.channelId === this.state.channelId
        && message.clientMessageId === pending.clientMessageId && message.author.id === pending.author?.id);
      if (accepted) {
        this.confirmSend?.(accepted);
        this.state = { ...this.state, pendingSend: undefined, sendError: undefined, sendRejected: undefined };
      }
    }
    this.changed(this.state);
  }
}
