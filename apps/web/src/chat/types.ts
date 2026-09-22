export interface ChatAuthor {
  id: string;
  name: string;
  isGuest: boolean;
}

export interface ChatMessage {
  id: string;
  channelId: string;
  seq: string;
  author: ChatAuthor;
  content: { version: 1; type: "text"; text: string };
  createdAt: string;
  clientMessageId: string;
}

export interface ChatHistory {
  messages: ChatMessage[];
  cursor: string;
  hasMore: boolean;
}

export interface GeneralChatHistory extends ChatHistory {
  space: { id: string; name: string };
  channel: { id: string; name: string };
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

export interface ChatPresenceEvent {
  type: "presence.updated";
  author: ChatAuthor;
  revision: string;
}

export type ChatEvent =
  | ChatTypingEvent
  | ChatPresenceEvent
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
    && isChatAuthor(message.author) && !!message.content
    && message.content.version === 1 && message.content.type === "text" && typeof message.content.text === "string";
}
