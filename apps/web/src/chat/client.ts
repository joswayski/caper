import { ChatConnection } from "./connection.ts";
import { playSound } from "../audio/effects.ts";
import { appGateway } from "../gateway/client.ts";
import { ChatTimeline, type ChatTimelineEvent } from "./timeline.ts";
import { attachmentsOf, isChannelMessage, isChatMessage, isChatPinEvent, isChatReactionEvent, sequence, type ChatAttachment, type ChatAttachmentProgressEvent, type ChatAuthor, type ChatHistory, type ChatMessage, type ChatSession, type ChatThreadHistory, type ChatTypingEvent, type GeneralChatHistory, type MessageVersion } from "./types.ts";

const SESSION_KEY = "caper.chat.session";

export interface PendingChatMessage {
  clientMessageId: string;
  text: string;
  /** Uploaded files; URLs may be local object URLs until the server confirms. */
  attachments?: ChatAttachment[];
  author?: ChatAuthor;
  createdAt: string;
  threadRootId?: string;
  broadcast?: boolean;
}

export interface ThreadViewState {
  rootId: string;
  loading: boolean;
  loadingOlder: boolean;
  hasMore: boolean;
  before?: string;
  error?: string;
}

export interface ChatViewState {
  phase: "loading" | "ready" | "error";
  online: boolean;
  spaceName: string;
  channelId?: string;
  channelName: string;
  messages: ChatMessage[];
  pinnedMessages: ChatMessage[];
  channelMessages?: ChatMessage[];
  typingAuthors: ChatAuthor[];
  hasMore: boolean;
  loadingOlder: boolean;
  olderError?: string;
  author?: ChatAuthor;
  sessionError?: string;
  sendError?: string;
  sendRejected?: boolean;
  pendingSend?: PendingChatMessage;
  /** Latest `attachment.progress` percent per still-processing attachment id. */
  attachmentProgress: Record<string, number>;
  error?: string;
  thread?: ThreadViewState;
}

const initialState: ChatViewState = {
  phase: "loading", online: false, spaceName: "Caper", channelName: "general",
  messages: [], pinnedMessages: [], typingAuthors: [], hasMore: false, loadingOlder: false, attachmentProgress: {},
};

/** Ids of attachments the server is still processing. */
function processingIds(messages: ChatMessage[]) {
  const ids = new Set<string>();
  for (const message of messages) for (const attachment of attachmentsOf(message)) if (attachment.status === "processing") ids.add(attachment.id);
  return ids;
}

export function initialChatView(history?: GeneralChatHistory, error?: string): ChatViewState {
  if (error) return { ...initialState, phase: "error", error };
  return history ? {
    ...initialState, phase: "ready", spaceName: history.space.name,
    channelId: history.channel.id, channelName: history.channel.name,
    messages: history.messages, pinnedMessages: history.pinnedMessages ?? [], hasMore: history.hasMore,
  } : initialState;
}

export function apiError(response: Response, fallback: string) {
  return response.json().catch(() => undefined).then((body: { error?: unknown } | undefined) =>
    new Error(typeof body?.error === "string" ? body.error : fallback));
}

function validHistory(value: unknown, general: true): value is GeneralChatHistory;
function validHistory(value: unknown, general: false): value is ChatHistory;
function validHistory(value: unknown, general: boolean): value is ChatHistory | GeneralChatHistory {
  if (!value || typeof value !== "object") return false;
  const history = value as Partial<GeneralChatHistory>;
  try { if (typeof history.cursor !== "string") return false; sequence(history.cursor); } catch { return false; }
  return Array.isArray(history.messages) && history.messages.every(isChatMessage)
    && (history.pinnedMessages === undefined || (Array.isArray(history.pinnedMessages) && history.pinnedMessages.length <= 100 && history.pinnedMessages.every(isChatMessage)))
    && typeof history.hasMore === "boolean"
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
  if (history.pinnedMessages?.some((message) => message.channelId !== history.channel.id)) throw new Error("The chat service returned pins from another channel.");
  return history;
}

export async function loadMessageVersions(channelId: string, messageId: string, before?: number, signal?: AbortSignal): Promise<{ versions: MessageVersion[]; hasMore: boolean }> {
  const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages/${encodeURIComponent(messageId)}/versions${before ? `?before=${before}` : ""}`, {
    cache: "no-store", signal: AbortSignal.any([...(signal ? [signal] : []), AbortSignal.timeout(10_000)]),
  });
  if (!response.ok) throw await apiError(response, "Message history could not be loaded.");
  const page = await response.json() as { messageId?: string; versions?: MessageVersion[]; hasMore?: boolean };
  if (page.messageId !== messageId || typeof page.hasMore !== "boolean" || !Array.isArray(page.versions) || page.versions.length > 50
    || !page.versions.every((version, index, all) => version && Number.isSafeInteger(version.revision) && version.revision >= 1
      && (!before || version.revision < before) && (!index || version.revision < all[index - 1].revision)
      && typeof version.createdAt === "string" && version.content?.version === 1 && version.content.type === "text" && typeof version.content.text === "string")) throw new Error("The chat service returned invalid message history.");
  return { versions: page.versions, hasMore: page.hasMore };
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
  private sessionGeneration = 0;
  private loadingHistory = false;
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
  private readonly typers = new Map<string, { author: ChatAuthor; typing: boolean; revision: bigint; expires: number }>();
  private readonly changed: (state: ChatViewState) => void;
  private readonly channelId?: string;
  private sounds: boolean;
  private spaceId?: string;
  private channel?: GeneralChatHistory["channel"];
  private readonly reactionIntents = new Map<string, Map<string, { active: boolean; authorId: string; generation: number }>>();
  private readonly reactionRequests = new Map<string, Promise<void>>();
  private threadRequest?: AbortController;
  // Fetching a thread's older rows must not insert them into the channel page
  // or move the channel's exclusive pagination boundary past a history gap.
  private readonly threadOnlyRows = new Set<string>();

  constructor(changed: (state: ChatViewState) => void, channelId?: string, options: { sounds?: boolean } = {}) {
    this.changed = changed;
    this.channelId = channelId;
    this.sounds = options.sounds ?? true;
  }

  start(history?: GeneralChatHistory, error?: string) {
    if (error) this.update({ phase: "error", error });
    else void this.loadInitial(history);
  }

  /** New-message sounds can be enabled once a visitor engages with an embedded chat. */
  setSounds(enabled: boolean) { this.sounds = enabled; }

  snapshotHistory(): GeneralChatHistory | undefined {
    if (this.state.phase !== "ready" || this.spaceId === undefined || !this.channel) return;
    const pinnedMessages = this.timeline.pinnedMessages;
    return {
      space: { id: this.spaceId, name: this.state.spaceName },
      channel: this.channel,
      messages: this.timeline.messages.filter((message) => isChannelMessage(message) && !this.threadOnlyRows.has(message.id)), cursor: this.timeline.cursor, hasMore: this.state.hasMore,
      ...(pinnedMessages.length ? { pinnedMessages } : {}),
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
    this.typingRequest = appGateway().command({
      method: "typing", channelId: channel, chatToken: session.token,
      body: { typing: active }, timeoutMs: 2_000, signal: this.controller.signal,
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

  retryLoad() { void this.loadInitial(); }

  retrySession() { void this.createSession(); }

  closeThread() {
    this.threadRequest?.abort();
    this.update({ thread: undefined });
  }

  async openThread(rootId: string) {
    this.threadRequest?.abort();
    this.update({ thread: { rootId, loading: true, loadingOlder: false, hasMore: false } });
    await this.loadThreadPage();
  }

  async loadOlderThread() {
    if (this.state.thread?.hasMore && !this.state.thread.loading && !this.state.thread.loadingOlder) await this.loadThreadPage(true);
  }

  private async loadThreadPage(older = false) {
    const thread = this.state.thread;
    const channelId = this.state.channelId;
    if (!thread || !channelId || this.controller.signal.aborted) return;
    const controller = new AbortController();
    this.threadRequest = controller;
    const generation = this.generation;
    const current = () => !controller.signal.aborted && !this.controller.signal.aborted
      && generation === this.generation && this.state.thread?.rootId === thread.rootId;
    this.update({ thread: { ...thread, error: undefined, loading: !older, loadingOlder: older } });
    try {
      const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages/${encodeURIComponent(thread.rootId)}/thread${older && thread.before ? `?before=${thread.before}` : ""}`, {
        cache: "no-store", signal: AbortSignal.any([controller.signal, this.controller.signal, AbortSignal.timeout(10_000)]),
      });
      if (!response.ok) {
        const error = await apiError(response, "Thread could not be loaded.");
        if (current() && [401, 403, 404].includes(response.status)) {
          this.closeThread();
          this.retryLoad();
          return;
        }
        throw error;
      }
      const page: unknown = await response.json();
      if (!current()) return;
      const history = page as ChatThreadHistory;
      if (!validHistory(page, false) || !isChatMessage(history.root) || history.root.id !== thread.rootId
        || history.root.channelId !== channelId || history.root.threadRootId
        || history.messages.some((message) => message.channelId !== channelId || message.threadRootId !== thread.rootId)) throw new Error("The chat service returned an invalid thread.");
      // Merge instead of replacing: live replies/reactions may arrive during GET.
      const loaded = new Set(this.timeline.messages.map((message) => message.id));
      const rows = [history.root, ...history.messages];
      for (const message of rows) if (isChannelMessage(message) && !loaded.has(message.id)) this.threadOnlyRows.add(message.id);
      this.timeline.prepend(rows);
      this.update({ messages: this.timeline.messages, thread: { rootId: thread.rootId, loading: false, loadingOlder: false,
        hasMore: history.hasMore, before: history.messages[0]?.seq ?? thread.before } });
    } catch (error) {
      if (current()) this.update({ thread: { ...this.state.thread!, loading: false, loadingOlder: false,
        error: error instanceof Error ? error.message : "Thread could not be loaded." } });
    }
  }

  discardRejected(): string | undefined {
    if (this.sending || !this.state.sendRejected) return;
    const text = this.state.pendingSend?.text;
    this.update({ pendingSend: undefined, sendError: undefined, sendRejected: undefined });
    return text;
  }

  async loadOlder() {
    if (this.loadingHistory || this.state.phase !== "ready" || !this.state.channelId || !this.state.hasMore || this.state.loadingOlder || !this.state.messages.length || this.controller.signal.aborted) return;
    const generation = this.generation;
    const channelId = this.state.channelId;
    this.update({ loadingOlder: true, olderError: undefined });
    try {
      const before = this.state.channelMessages?.[0]?.seq;
      if (!before) { this.update({ loadingOlder: false }); return; }
      const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages?before=${encodeURIComponent(before)}`, {
        cache: "no-store", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
      });
      if (!response.ok) throw await apiError(response, "Older messages could not be loaded.");
      const history: unknown = await response.json();
      if (!validHistory(history, false)) throw new Error("The chat service returned invalid history.");
      if (generation !== this.generation) return;
      if (history.messages.some((message) => message.channelId !== channelId)) throw new Error("The chat service returned messages from another channel.");
      for (const message of history.messages) this.threadOnlyRows.delete(message.id);
      this.timeline.prepend(history.messages);
      this.update({ messages: this.timeline.messages, hasMore: history.hasMore, loadingOlder: false });
    } catch (error) {
      if (!this.controller.signal.aborted && generation === this.generation) this.update({ loadingOlder: false, olderError: error instanceof Error ? error.message : "Older messages could not be loaded." });
    }
  }

  async send(text: string, options: { threadRootId?: string; broadcast?: boolean; attachments?: ChatAttachment[] } = {}): Promise<boolean> {
    if (this.controller.signal.aborted || this.sending || this.state.sendRejected) return false;
    if (this.state.pendingSend && this.state.pendingSend.threadRootId !== options.threadRootId) return false;
    const { attachments = [], ...thread } = options;
    // A timeout is an unknown outcome. Enter/Send must retry the same command,
    // just like the explicit retry button, before allowing a new command.
    const pending = {
      ...(this.state.pendingSend ?? { clientMessageId: crypto.randomUUID(), text, attachments: attachments.length ? attachments : undefined, createdAt: new Date().toISOString(), ...thread }),
      author: this.session?.author,
    };
    const count = Array.from(pending.text).length;
    const attachmentIds = (pending.attachments ?? []).map((attachment) => attachment.id);
    if ((!pending.text.trim() && !attachmentIds.length) || count > 4_000) throw new Error(count > 4_000 ? "Messages can be at most 4,000 characters." : "Write a message first.");
    if (/[\u0000-\u0008\u000b-\u001f\u007f-\u009f]/u.test(pending.text)) throw new Error("Messages cannot contain control characters.");
    const channelId = this.state.channelId;
    if (!channelId) throw new Error("Chat is not ready yet.");
    const session = this.session;
    if (!session) {
      this.update({ pendingSend: pending, sendError: "Your guest session is unavailable. Retry the session, then send again." });
      return false;
    }
    // A refresh may revoke access while HTTP is in flight. Successful refreshes
    // of the same channel still allow the acknowledgement to confirm the send.
    const current = () => !this.controller.signal.aborted && this.state.channelId === channelId;
    this.setTyping(false);
    this.sending = true;
    const controller = new AbortController();
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
          body: JSON.stringify({ clientMessageId: pending.clientMessageId, text: pending.text, ...(attachmentIds.length ? { attachmentIds } : {}),
            ...(pending.threadRootId ? { threadRootId: pending.threadRootId, broadcast: pending.broadcast ?? false } : {}) }),
          signal: AbortSignal.any([this.controller.signal, controller.signal, AbortSignal.timeout(10_000)]),
        });
        if (!response.ok) {
          rejected = [400, 404, 409, 413, 422].includes(response.status);
          if (current() && (response.status === 401 || response.status === 403) && this.state.pendingSend?.clientMessageId === pending.clientMessageId) {
            try { localStorage.removeItem(SESSION_KEY); } catch { /* Storage is optional. */ }
            this.session = undefined;
            void this.createSession();
          }
          throw await apiError(response, "Message could not be sent.");
        }
        const message: unknown = await response.json();
        if (!isChatMessage(message) || message.channelId !== channelId
          || message.threadRootId !== pending.threadRootId || (message.broadcast ?? false) !== (pending.broadcast ?? false)
          || message.clientMessageId !== pending.clientMessageId || message.author.id !== session.author.id) throw new Error("The chat service returned an invalid message.");
        return message;
      })();
      const message = await Promise.race([request, confirmation]);
      if (!current()) return false;
      this.timeline.mergeSent(message);
      this.update({ messages: this.timeline.messages, pendingSend: undefined, sendError: undefined, sendRejected: undefined });
      return true;
    } catch (error) {
      if (!current()) return false;
      // Confirmation can clear the command while an HTTP rejection is already
      // propagating through Promise.race, before this continuation runs.
      if (!this.state.pendingSend) return true;
      if (!this.controller.signal.aborted) this.update({ pendingSend: pending, sendRejected: rejected, sendError: error instanceof Error ? error.message : "Message could not be sent." });
      return false;
    } finally {
      // Ordered replay/history may confirm delivery before HTTP responds.
      // Release that request without aborting the client or its next command.
      controller.abort();
      this.sending = false;
      this.confirmSend = undefined;
    }
  }

  async setReaction(messageId: string, emoji: string, active: boolean): Promise<void> {
    const generation = this.generation;
    const channelId = this.state.channelId;
    const session = this.session;
    if (this.controller.signal.aborted || !channelId || !session) throw new Error("Your chat session is unavailable. Retry the session, then react again.");
    const intents = this.reactionIntents.get(messageId) ?? new Map();
    const intent = { active, authorId: session.author.id, generation };
    intents.set(emoji, intent);
    this.reactionIntents.set(messageId, intents);
    this.update({ messages: this.timeline.messages });
    const current = () => !this.controller.signal.aborted && generation === this.generation && session === this.session;
    const save = async () => {
      try {
        if (!current() || intents.get(emoji) !== intent) return;
        const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages/${encodeURIComponent(messageId)}/reactions`, {
          method: "PUT", headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
          body: JSON.stringify({ emoji, active }),
          signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
        });
        if (!current()) return;
        if (!response.ok) {
          const error = await apiError(response, "Reaction could not be saved. Try again.");
          if (response.status === 401 || response.status === 403) {
            this.session = undefined;
            void this.createSession();
          }
          throw error;
        }
        const event: unknown = await response.json();
        if (!isChatReactionEvent(event) || event.channelId !== channelId || event.messageId !== messageId) throw new Error("The chat service returned an invalid reaction.");
        if (!current()) return;
        this.timeline.mergeReactions(event);
      } catch (error) {
        // An older failed toggle must not report failure for a newer intent.
        if (!this.controller.signal.aborted && generation === this.generation && intents.get(emoji) === intent) throw error;
      } finally {
        if (intents.get(emoji) === intent) {
          intents.delete(emoji);
          if (!intents.size) this.reactionIntents.delete(messageId);
        }
        if (!this.controller.signal.aborted && generation === this.generation) this.update({ messages: this.timeline.messages });
      }
    };
    // Keep full-message acknowledgements ordered; taps still project immediately.
    const previous = this.reactionRequests.get(messageId);
    const request = previous ? previous.catch(() => {}).then(save) : save();
    this.reactionRequests.set(messageId, request);
    try { await request; }
    finally { if (this.reactionRequests.get(messageId) === request) this.reactionRequests.delete(messageId); }
  }

  /** Ephemeral, like typing: shown on the processing placeholder, dropped when
   * the attachment leaves processing. Unknown attachments are ignored. */
  private receiveProgress(event: ChatAttachmentProgressEvent) {
    const message = this.timeline.messages.find((item) => item.id === event.messageId)
      ?? this.timeline.pinnedMessages.find((item) => item.id === event.messageId);
    const attachment = message && attachmentsOf(message).find((item) => item.id === event.attachmentId);
    if (attachment?.status !== "processing") return;
    const percent = Math.round(event.percent);
    if (this.state.attachmentProgress[attachment.id] === percent) return;
    this.update({ attachmentProgress: { ...this.state.attachmentProgress, [attachment.id]: percent } });
  }

  async setPin(messageId: string, active: boolean): Promise<void> {
    const generation = this.generation;
    const channelId = this.state.channelId;
    const session = this.session;
    if (this.controller.signal.aborted || !channelId || !session) throw new Error("Your chat session is unavailable. Retry the session, then try again.");
    const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages/${encodeURIComponent(messageId)}/pin`, {
      method: "PUT", headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
      body: JSON.stringify({ active }), signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
    });
    if (!response.ok) throw await apiError(response, active ? "Message could not be pinned." : "Message could not be unpinned.");
    const event: unknown = await response.json();
    if (!isChatPinEvent(event) || event.channelId !== channelId || event.message.id !== messageId) throw new Error("The chat service returned an invalid pin.");
    if (generation !== this.generation || this.controller.signal.aborted) return;
    this.timeline.mergePin(event);
    this.update({ messages: this.timeline.messages, pinnedMessages: this.timeline.pinnedMessages });
  }

  async forward(destination: string, messageId: string, clientMessageId: string, text: string): Promise<ChatMessage> {
    const channelId = this.state.channelId;
    const session = this.session;
    if (this.controller.signal.aborted || !channelId || !session || session.author.isGuest) throw new Error("Your chat session is unavailable. Retry the session, then try again.");
    const response = await fetch(`/api/chat/channels/${encodeURIComponent(destination)}/forwards`, {
      method: "POST", headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
      body: JSON.stringify({ sourceChannelId: channelId, sourceMessageId: messageId, clientMessageId, text }),
      signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
    });
    if (!response.ok) throw new ChatHistoryError(response.status, (await apiError(response, "Forward could not be confirmed. Retry to check the same forward.")).message);
    const message: unknown = await response.json();
    if (!isChatMessage(message) || !message.forward || message.channelId !== destination || message.clientMessageId !== clientMessageId || message.author.id !== session.author.id) throw new Error("The chat service returned an invalid forward.");
    if (!this.controller.signal.aborted && this.state.channelId === destination) {
      this.timeline.mergeSent(message);
      this.update({ messages: this.timeline.messages });
    }
    return message;
  }

  async editMessage(messageId: string, text: string, expectedRevision: number): Promise<void> {
    const generation = this.generation, channelId = this.state.channelId, session = this.session;
    if (this.controller.signal.aborted || !channelId || !session) throw new Error("Your chat session is unavailable. Retry the session, then edit again.");
    const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages/${encodeURIComponent(messageId)}`, {
      method: "PUT", headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
      body: JSON.stringify({ text, expectedRevision }), signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
    });
    if (!response.ok) throw await apiError(response, "Edit could not be saved. Your draft is kept.");
    const message: unknown = await response.json();
    if (!isChatMessage(message) || message.channelId !== channelId || message.id !== messageId || message.author.id !== session.author.id) throw new Error("The chat service returned an invalid edit.");
    if (generation !== this.generation || this.controller.signal.aborted || session !== this.session) throw new Error("The conversation changed. Reopen the message to edit it.");
    this.timeline.mergeEdit(message);
    this.update({ messages: this.timeline.messages, pinnedMessages: this.timeline.pinnedMessages });
  }

  async reloadMessage(messageId: string): Promise<ChatMessage> {
    const generation = this.generation, channelId = this.state.channelId;
    if (!channelId || this.controller.signal.aborted) throw new Error("The conversation is unavailable.");
    const response = await fetch(`/api/chat/channels/${encodeURIComponent(channelId)}/messages/${encodeURIComponent(messageId)}`, {
      cache: "no-store", signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
    });
    if (!response.ok) throw await apiError(response, "The latest message could not be loaded.");
    const message: unknown = await response.json();
    if (!isChatMessage(message) || message.channelId !== channelId || message.id !== messageId) throw new Error("The chat service returned an invalid message.");
    if (generation !== this.generation || this.controller.signal.aborted) throw new Error("The conversation changed. Reopen the message to edit it.");
    this.timeline.mergeEdit(message);
    this.update({ messages: this.timeline.messages, pinnedMessages: this.timeline.pinnedMessages });
    return message;
  }

  private receiveEvent(event: ChatTimelineEvent) {
    const visible = new Set(this.timeline.messages.map((item) => item.id));
    const result = this.timeline.applyEvent(event);
    if (!("type" in event)) {
      const typer = this.typers.get(event.author.id);
      if (typer && result !== "duplicate") { typer.typing = false; this.refreshTypers(); }
    }
    if (result !== "buffered" && result !== "overflow") {
      const messages = this.timeline.messages;
      this.update({ messages, pinnedMessages: this.timeline.pinnedMessages });
      const ownAuthorId = this.session?.author.id ?? this.state.author?.id;
      if (this.sounds && (!("type" in event) || event.type !== "message.edited") && result === "applied" && messages.some((item) => !visible.has(item.id) && item.author.id !== ownAuthorId)) playSound("new-message");
    }
    return result;
  }

  private async loadInitial(prepared?: GeneralChatHistory) {
    if (this.controller.signal.aborted) return;
    const generation = ++this.generation;
    this.loadingHistory = true;
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
      if (history.messages.some((message) => message.channelId !== history.channel.id)) throw new Error("The chat service returned messages from another channel.");
      if (history.pinnedMessages?.some((message) => message.channelId !== history.channel.id)) throw new Error("The chat service returned pins from another channel.");
      if (generation !== this.generation) return;
      this.spaceId = history.space.id;
      this.channel = history.channel;
      // Retain older pages only when every missing event is a fresh message.
      // An unaccounted sequence may be a reaction on an older cached row.
      const applied = sequence(previous?.cursor ?? "0");
      let accounted = applied;
      const contiguous = previous && history.messages.length > 0
        && history.messages.every((message) => {
          const next = sequence(message.seq);
          if (next <= applied) return true;
          if (next !== accounted + 1n) return false;
          accounted = next;
          return true;
        }) && accounted === sequence(history.cursor);
      const retainedOlder = previous?.messages[0] && history.messages[0]
        && contiguous
        && sequence(previous.messages[0].seq) < sequence(history.messages[0].seq);
      // reset deduplicates by first occurrence: fresh author metadata wins.
      // Even across a gap, preserve newer HTTP reaction revisions on fresh rows.
      const freshIds = new Set(history.messages.map((message) => message.id));
      const retained = this.timeline.messages.filter((message) => (contiguous && isChannelMessage(message) && !this.threadOnlyRows.has(message.id)) || freshIds.has(message.id));
      this.threadOnlyRows.clear();
      this.timeline.reset([...history.messages, ...retained], history.cursor, history.pinnedMessages ?? []);
      this.update({
        phase: "ready", spaceName: history.space.name, channelId: history.channel.id,
        channelName: history.channel.name, messages: this.timeline.messages, pinnedMessages: this.timeline.pinnedMessages, hasMore: retainedOlder ? previous.hasMore : history.hasMore,
      });
      this.connection = new ChatConnection(history.channel.id, {
        cursor: () => this.timeline.cursor,
        message: (message) => this.receiveEvent(message),
        reactions: (event) => this.receiveEvent(event),
        attachments: (event) => this.receiveEvent(event),
        progress: (event) => this.receiveProgress(event),
        pin: (event) => this.receiveEvent(event),
        forward: (event) => this.receiveEvent(event),
        edit: (event) => this.receiveEvent(event),
        status: (online) => {
          if (!online) { this.typers.clear(); this.refreshTypers(); }
          this.update({ online });
        },
        typing: (event) => this.receiveTyping(event),
        resync: () => { if (generation === this.generation) void this.loadInitial(); },
      });
      this.connection.start();
      if (this.state.thread) void this.openThread(this.state.thread.rootId);
    } catch (error) {
      if (!this.controller.signal.aborted && generation === this.generation) {
        const denied = error instanceof ChatHistoryError && [401, 403, 404].includes(error.status);
        if (denied) { this.timeline.reset([], "0"); this.spaceId = undefined; }
        this.update({
          phase: previous && !denied ? "ready" : "error", online: false,
          ...(denied ? { messages: [], pinnedMessages: [], channelId: undefined, thread: undefined } : {}),
          error: error instanceof Error ? error.message : "Messages are unavailable.",
        });
      }
    } finally {
      if (generation === this.generation) this.loadingHistory = false;
    }
  }

  private async createSession() {
    if (this.controller.signal.aborted) return;
    const generation = ++this.sessionGeneration;
    this.update({ sessionError: undefined });
    try {
      const response = await fetch("/api/chat/session", {
        method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ name: this.name }), signal: AbortSignal.any([this.controller.signal, AbortSignal.timeout(10_000)]),
      });
      if (!response.ok) throw await apiError(response, "Guest messaging is unavailable.");
      const session = await response.json() as Partial<ChatSession>;
      if (typeof session.token !== "string" || !session.token || !session.author || typeof session.author.id !== "string"
        || typeof session.author.name !== "string" || typeof session.author.isGuest !== "boolean") throw new Error("The chat service returned an invalid session.");
      if (this.controller.signal.aborted || generation !== this.sessionGeneration) return;
      this.session = session as ChatSession;
      try { localStorage.setItem(SESSION_KEY, JSON.stringify(session)); } catch { /* The in-memory response still permits this page to render. */ }
      this.update({ author: session.author, sessionError: undefined });
    } catch (error) {
      if (!this.controller.signal.aborted && generation === this.sessionGeneration) this.update({ sessionError: error instanceof Error ? error.message : "Guest messaging is unavailable." });
    }
  }

  private update(change: Partial<ChatViewState>) {
    if (change.messages && this.reactionIntents.size) {
      change = { ...change, messages: change.messages.map((message) => {
        const intents = this.reactionIntents.get(message.id);
        if (!intents) return message;
        const reactions = new Map((message.reactions ?? []).map((reaction) => [reaction.emoji, reaction]));
        for (const [emoji, intent] of intents) {
          if (intent.generation !== this.generation || intent.authorId !== this.session?.author.id) continue;
          const authorIds = reactions.get(emoji)?.authorIds.filter((id) => id !== intent.authorId) ?? [];
          if (intent.active) authorIds.push(intent.authorId);
          if (authorIds.length) reactions.set(emoji, { emoji, authorIds });
          else reactions.delete(emoji);
        }
        return { ...message, reactions: [...reactions.values()] };
      }) };
    }
    this.state = { ...this.state, ...change };
    if (change.messages) this.state.channelMessages = change.messages.filter((message) => isChannelMessage(message) && !this.threadOnlyRows.has(message.id));
    if ((change.messages || change.pinnedMessages) && Object.keys(this.state.attachmentProgress).length) {
      const processing = processingIds([...this.state.messages, ...this.state.pinnedMessages]);
      const progress = Object.fromEntries(Object.entries(this.state.attachmentProgress).filter(([id]) => processing.has(id)));
      if (Object.keys(progress).length !== Object.keys(this.state.attachmentProgress).length) this.state = { ...this.state, attachmentProgress: progress };
    }
    if (change.author) {
      this.typers.delete(change.author.id);
      this.state = { ...this.state, typingAuthors: this.state.typingAuthors.filter((author) => author.id !== change.author!.id) };
    }
    const pending = this.state.pendingSend;
    if (pending && change.messages) {
      const accepted = change.messages.find((message) => message.channelId === this.state.channelId
        && message.threadRootId === pending.threadRootId && (message.broadcast ?? false) === (pending.broadcast ?? false)
        && message.clientMessageId === pending.clientMessageId && message.author.id === pending.author?.id);
      if (accepted) {
        this.confirmSend?.(accepted);
        this.state = { ...this.state, pendingSend: undefined, sendError: undefined, sendRejected: undefined };
      }
    }
    this.changed(this.state);
  }
}
