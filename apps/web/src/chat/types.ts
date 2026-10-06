export interface ChatAuthor {
  id: string;
  avatarId?: number | null;
  name: string;
  isGuest: boolean;
}

export interface ChatThreadSummary {
  replyCount: number;
  participants: ChatAuthor[];
  seq: string;
}

export interface ChatReaction {
  emoji: string;
  authorIds: string[];
}

export interface ChatReactionEvent {
  type: "message.reactions";
  schemaVersion: 1;
  channelId: string;
  seq: string;
  messageId: string;
  reactions: ChatReaction[];
}

export interface ChatPinEvent {
  type: "message.pin";
  schemaVersion: 1;
  channelId: string;
  seq: string;
  message: ChatMessage;
}

export interface ChatEditEvent {
  type: "message.edited";
  schemaVersion: 1;
  channelId: string;
  seq: string;
  message: ChatMessage;
}

export interface MessageVersion {
  revision: number;
  content: ChatMessage["content"];
  createdAt: string;
}

export interface ChatMessage {
  id: string;
  channelId: string;
  seq: string;
  author: ChatAuthor;
  content: { version: 1; type: "text"; text: string };
  createdAt: string;
  clientMessageId: string;
  reactions?: ChatReaction[];
  reactionSeq?: string;
  pin?: { author: ChatAuthor; createdAt: string } | null;
  pinSeq?: string;
  threadRootId?: string;
  broadcast?: boolean;
  thread?: ChatThreadSummary;
  revision?: number;
  editedAt?: string;
  editSeq?: string;
}

export interface ChatHistory {
  messages: ChatMessage[];
  cursor: string;
  hasMore: boolean;
  pinnedMessages?: ChatMessage[];
}

export interface ChatThreadHistory extends ChatHistory {
  root: ChatMessage;
}

export interface GeneralChatHistory extends ChatHistory {
  space: { id: string; name: string };
  channel: { id: string; name: string; direct?: boolean };
}

export interface ChatSession {
  token: string;
  author: ChatAuthor;
}

export interface ChatTypingEvent {
  type: "typing.updated";
  channelId: string;
  author: ChatAuthor;
  typing: boolean;
  revision: string;
}

export type ChatEvent =
  | ChatTypingEvent
  | ChatReactionEvent
  | ChatPinEvent
  | ChatEditEvent
  | { type: "message.created"; channelId: string; seq: string; message: ChatMessage }
  | { type: "ready"; cursor: string }
  | { type: "migrating" }
  | { type: "resync_required" };

export function sequence(value: string): bigint {
  if (!/^(0|[1-9]\d*)$/.test(value)) throw new Error("Invalid chat sequence.");
  return BigInt(value);
}

export function isChatAuthor(value: unknown): value is ChatAuthor {
  if (!value || typeof value !== "object") return false;
  const author = value as Partial<ChatAuthor>;
  return typeof author.id === "string" && typeof author.name === "string" && typeof author.isGuest === "boolean";
}

export function isChatMessage(value: unknown): value is ChatMessage {
  if (!value || typeof value !== "object") return false;
  const message = value as Partial<ChatMessage>;
  return typeof message.id === "string" && typeof message.channelId === "string"
    && typeof message.seq === "string" && /^(0|[1-9]\d*)$/.test(message.seq)
    && typeof message.createdAt === "string" && typeof message.clientMessageId === "string"
    && (message.revision === undefined || (Number.isSafeInteger(message.revision) && message.revision >= 1))
    && (message.editedAt === undefined || typeof message.editedAt === "string")
    && (message.editSeq === undefined || (typeof message.editSeq === "string" && /^(0|[1-9]\d*)$/.test(message.editSeq)))
    && ((message.revision ?? 1) === 1 || (message.editedAt !== undefined && message.editSeq !== undefined))
    && (message.threadRootId === undefined || (typeof message.threadRootId === "string" && !!message.threadRootId))
    && (message.broadcast === undefined || (typeof message.broadcast === "boolean" && (!message.broadcast || !!message.threadRootId)))
    && (message.thread === undefined || isChatThreadSummary(message.thread))
    && (message.reactions === undefined || isChatReactions(message.reactions))
    && (message.reactionSeq === undefined || (typeof message.reactionSeq === "string" && /^(0|[1-9]\d*)$/.test(message.reactionSeq)))
    && (message.pinSeq === undefined || (typeof message.pinSeq === "string" && /^(0|[1-9]\d*)$/.test(message.pinSeq)))
    && (message.pin === undefined || message.pin === null || (typeof message.pin === "object" && isChatAuthor(message.pin.author) && typeof message.pin.createdAt === "string"))
    && isChatAuthor(message.author) && !!message.content
    && message.content.version === 1 && message.content.type === "text" && typeof message.content.text === "string";
}

export function isChatEditEvent(value: unknown): value is ChatEditEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatEditEvent>;
  return event.type === "message.edited" && event.schemaVersion === 1 && typeof event.channelId === "string"
    && typeof event.seq === "string" && /^(0|[1-9]\d*)$/.test(event.seq) && isChatMessage(event.message)
    && event.message.channelId === event.channelId && event.message.editSeq === event.seq && (event.message.revision ?? 1) > 1;
}

export function isChatPinEvent(value: unknown): value is ChatPinEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatPinEvent>;
  return event.type === "message.pin" && event.schemaVersion === 1 && typeof event.channelId === "string"
    && typeof event.seq === "string" && /^(0|[1-9]\d*)$/.test(event.seq) && isChatMessage(event.message)
    && event.message.channelId === event.channelId && event.message.pinSeq === event.seq;
}

export function isChatThreadSummary(value: unknown): value is ChatThreadSummary {
  if (!value || typeof value !== "object") return false;
  const summary = value as Partial<ChatThreadSummary>;
  return Number.isSafeInteger(summary.replyCount) && summary.replyCount! > 0
    && typeof summary.seq === "string" && /^(0|[1-9]\d*)$/.test(summary.seq)
    && Array.isArray(summary.participants) && summary.participants.length <= 5
    && summary.participants.every(isChatAuthor)
    && new Set(summary.participants.map((author) => author.id)).size === summary.participants.length;
}

export function isChannelMessage(message: ChatMessage): boolean {
  return !message.threadRootId || message.broadcast === true;
}

export function isChatReactions(value: unknown): value is ChatReaction[] {
  return Array.isArray(value) && value.every((reaction: unknown) => {
    if (!reaction || typeof reaction !== "object") return false;
    const item = reaction as Partial<ChatReaction>;
    return typeof item.emoji === "string" && item.emoji.length > 0 && Array.isArray(item.authorIds)
      && item.authorIds.length > 0 && item.authorIds.every((id) => typeof id === "string")
      && new Set(item.authorIds).size === item.authorIds.length;
  });
}

export function isChatReactionEvent(value: unknown): value is ChatReactionEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatReactionEvent>;
  return event.type === "message.reactions" && event.schemaVersion === 1
    && typeof event.channelId === "string" && typeof event.messageId === "string"
    && typeof event.seq === "string" && /^(0|[1-9]\d*)$/.test(event.seq) && isChatReactions(event.reactions);
}
