import { ChatConnection } from "./connection.ts";
import { ChatTimeline } from "./timeline.ts";
import { isChatMessage, sequence, type ChatAuthor, type ChatHistory, type ChatMessage, type ChatSession, type GeneralChatHistory } from "./types.ts";

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
  hasMore: boolean;
  loadingOlder: boolean;
  author?: ChatAuthor;
  sessionError?: string;
  sendError?: string;
  sendRejected?: boolean;
  pendingSend?: PendingChatMessage;
  error?: string;
}

const initialState: ChatViewState = {
  phase: "loading", online: false, spaceName: "Caper", channelName: "General",
  messages: [], hasMore: false, loadingOlder: false,
};

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
  private readonly changed: (state: ChatViewState) => void;

  constructor(changed: (state: ChatViewState) => void) { this.changed = changed; }

  start() {
    void this.loadInitial();
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
    this.connection?.stop();
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
    if (!this.state.channelId || !this.state.hasMore || this.state.loadingOlder || !this.state.messages.length) return;
    this.update({ loadingOlder: true, error: undefined });
    try {
      const before = this.state.messages[0].seq;
      const response = await fetch(`/api/chat/channels/${encodeURIComponent(this.state.channelId)}/messages?before=${encodeURIComponent(before)}`, {
        cache: "no-store", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
      });
      if (!response.ok) throw await apiError(response, "Older messages could not be loaded.");
      const history: unknown = await response.json();
      if (!validHistory(history, false)) throw new Error("The chat service returned invalid history.");
      this.timeline.prepend(history.messages);
      this.update({ messages: this.timeline.messages, hasMore: history.hasMore, loadingOlder: false });
    } catch (error) {
      if (!this.controller.signal.aborted) this.update({ loadingOlder: false, error: error instanceof Error ? error.message : "Older messages could not be loaded." });
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

  private async loadInitial() {
    const generation = ++this.generation;
    this.connection?.stop();
    this.connection = undefined;
    this.update({ phase: "loading", online: false, error: undefined });
    try {
      const response = await fetch("/api/chat/general", { cache: "no-store", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]) });
      if (!response.ok) throw await apiError(response, "Messages are unavailable.");
      const history: unknown = await response.json();
      if (!validHistory(history, true)) throw new Error("The chat service returned invalid history.");
      if (generation !== this.generation) return;
      this.timeline.reset(history.messages, history.cursor);
      this.update({
        phase: "ready", spaceName: history.space.name, channelId: history.channel.id,
        channelName: history.channel.name, messages: this.timeline.messages, hasMore: history.hasMore,
      });
      this.connection = new ChatConnection(history.channel.id, {
        cursor: () => this.timeline.cursor,
        message: (message) => {
          const result = this.timeline.applyEvent(message);
          if (result !== "buffered" && result !== "overflow") this.update({ messages: this.timeline.messages });
          return result;
        },
        status: (online) => this.update({ online }),
        resync: () => { if (generation === this.generation) void this.loadInitial(); },
      });
      this.connection.start();
    } catch (error) {
      if (!this.controller.signal.aborted && generation === this.generation) this.update({
        phase: "error", online: false, error: error instanceof Error ? error.message : "Messages are unavailable.",
      });
    }
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
    } catch (error) {
      if (!this.controller.signal.aborted) this.update({ sessionError: error instanceof Error ? error.message : "Guest messaging is unavailable." });
    }
  }

  private update(change: Partial<ChatViewState>) {
    this.state = { ...this.state, ...change };
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
