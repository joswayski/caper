export interface ChatAuthor {
  id: string;
  avatarId?: number | null;
  name: string;
  isGuest: boolean;
}

export type ChatAttachmentKind = "image" | "video" | "audio" | "file";

/** Server-side processing state. Absent in payloads from before processing
 * moved to the media worker, which means ready. */
export type ChatAttachmentStatus = "processing" | "ready" | "failed";

/** A file on a message. URLs are signed per response and expire in 1–2 days. */
export interface ChatAttachment {
  id: string;
  kind: ChatAttachmentKind;
  contentType: string;
  name: string;
  size: number;
  width?: number;
  height?: number;
  durationMs?: number;
  preview?: Record<string, never>;
  status?: ChatAttachmentStatus;
  /** A GIF or animated image stored as a silent looping video. */
  animated?: boolean;
  /** Present only when ready. */
  url?: string;
  previewUrl?: string;
  /** The file was deleted; show a placeholder. */
  unavailable?: boolean;
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

/** The media worker replaced a message's files: a preview appeared, or
 * processing finished or failed. Sequenced like reactions. Entries are
 * filtered with `attachmentsOf` when rendered, like message content. */
export interface ChatAttachmentsEvent {
  type: "message.attachments";
  schemaVersion: 1;
  channelId: string;
  seq: string;
  messageId: string;
  attachments: ChatAttachment[];
}

/** Ephemeral video-encoding progress; unsequenced, like typing. */
export interface ChatAttachmentProgressEvent {
  type: "attachment.progress";
  channelId: string;
  messageId: string;
  attachmentId: string;
  percent: number;
}

export interface ChatMessage {
  id: string;
  channelId: string;
  seq: string;
  author: ChatAuthor;
  content: { version: 1; type: "text"; text: string; attachments?: ChatAttachment[] };
  createdAt: string;
  clientMessageId: string;
  reactions?: ChatReaction[];
  reactionSeq?: string;
  /** Sequence of the latest `message.attachments` reflected in `content.attachments`. */
  attachmentsSeq?: string;
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
  | ChatAttachmentsEvent
  | ChatAttachmentProgressEvent
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
    && (message.attachmentsSeq === undefined || (typeof message.attachmentsSeq === "string" && /^(0|[1-9]\d*)$/.test(message.attachmentsSeq)))
    && isChatAuthor(message.author) && !!message.content
    && message.content.version === 1 && message.content.type === "text" && typeof message.content.text === "string"
    && (message.content.attachments === undefined || Array.isArray(message.content.attachments));
}

/** Attachments safe to render. Malformed entries are skipped rather than
 * rejecting the message, so one bad file never breaks a history page. */
export function attachmentsOf(message: Pick<ChatMessage, "content">): ChatAttachment[] {
  return (message.content.attachments ?? []).filter(isChatAttachment);
}

export function isChatAttachment(value: unknown): value is ChatAttachment {
  if (!value || typeof value !== "object") return false;
  const item = value as Partial<ChatAttachment>;
  const optionalNumber = (v: unknown) => v === undefined || (typeof v === "number" && Number.isFinite(v) && v >= 0);
  const optionalUrl = (v: unknown) => v === undefined || (typeof v === "string" && /^https?:\/\//.test(v));
  return typeof item.id === "string" && typeof item.name === "string" && typeof item.contentType === "string"
    && typeof item.size === "number" && ["image", "video", "audio", "file"].includes(item.kind as string)
    && optionalNumber(item.width) && optionalNumber(item.height) && optionalNumber(item.durationMs)
    && optionalUrl(item.url) && optionalUrl(item.previewUrl)
    && (item.status === undefined || ["processing", "ready", "failed"].includes(item.status))
    && (item.animated === undefined || typeof item.animated === "boolean");
}

export type AttachmentView = "unavailable" | "processing" | "failed" | "image" | "video" | "animated" | "audio" | "file";

/** How a message attachment renders. Only ready files carry a URL; a ready
 * file without one falls back to a plain file card. */
export function attachmentView(attachment: ChatAttachment): AttachmentView {
  if (attachment.unavailable) return "unavailable";
  if (attachment.status === "processing") return "processing";
  if (attachment.status === "failed") return "failed";
  if (!attachment.url) return "file";
  if (attachment.kind === "video") return attachment.animated ? "animated" : "video";
  return attachment.kind;
}

export function isChatAttachmentsEvent(value: unknown): value is ChatAttachmentsEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatAttachmentsEvent>;
  return event.type === "message.attachments" && event.schemaVersion === 1
    && typeof event.channelId === "string" && typeof event.messageId === "string"
    && typeof event.seq === "string" && /^(0|[1-9]\d*)$/.test(event.seq) && Array.isArray(event.attachments);
}

export function isChatAttachmentProgressEvent(value: unknown): value is ChatAttachmentProgressEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatAttachmentProgressEvent>;
  return event.type === "attachment.progress" && typeof event.channelId === "string"
    && typeof event.messageId === "string" && typeof event.attachmentId === "string"
    && typeof event.percent === "number" && Number.isFinite(event.percent) && event.percent >= 0 && event.percent <= 100;
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
