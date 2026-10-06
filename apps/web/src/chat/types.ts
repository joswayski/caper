export interface ChatAuthor {
  id: string;
  avatarId?: number | null;
  name: string;
  isGuest: boolean;
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

/** Server-resolved `@mentions`; clients ignore unknown types. */
export type ChatMention =
  | { type: "user"; id: string; username: string }
  | { type: "everyone" }
  | { type: "here" }
  | { type: string; id?: string; username?: string };

export interface ChatMessage {
  id: string;
  channelId: string;
  seq: string;
  author: ChatAuthor;
  content: { version: 1; type: "text"; text: string; mentions?: ChatMention[] };
  createdAt: string;
  clientMessageId: string;
  reactions?: ChatReaction[];
  reactionSeq?: string;
}

export interface ChatHistory {
  messages: ChatMessage[];
  cursor: string;
  hasMore: boolean;
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
    && (message.reactions === undefined || isChatReactions(message.reactions))
    && (message.reactionSeq === undefined || (typeof message.reactionSeq === "string" && /^(0|[1-9]\d*)$/.test(message.reactionSeq)))
    && isChatAuthor(message.author) && !!message.content
    && message.content.version === 1 && message.content.type === "text" && typeof message.content.text === "string";
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
