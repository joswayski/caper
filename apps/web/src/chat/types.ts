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

export type ChatEvent =
  | { type: "message.created"; channelId: string; seq: string; message: ChatMessage }
  | { type: "ready"; cursor: string }
  | { type: "migrating" }
  | { type: "resync_required" };

export function sequence(value: string): bigint {
  if (!/^(0|[1-9]\d*)$/.test(value)) throw new Error("Invalid chat sequence.");
  return BigInt(value);
}

export function isChatMessage(value: unknown): value is ChatMessage {
  if (!value || typeof value !== "object") return false;
  const message = value as Partial<ChatMessage>;
  return typeof message.id === "string" && typeof message.channelId === "string"
    && typeof message.seq === "string" && /^(0|[1-9]\d*)$/.test(message.seq)
    && typeof message.createdAt === "string" && typeof message.clientMessageId === "string"
    && !!message.author && typeof message.author.id === "string" && typeof message.author.name === "string"
    && typeof message.author.isGuest === "boolean" && !!message.content
    && message.content.version === 1 && message.content.type === "text" && typeof message.content.text === "string";
}
