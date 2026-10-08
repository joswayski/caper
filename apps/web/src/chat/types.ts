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
/** Server-resolved `@mentions`; clients ignore unknown types. */
export type ChatMention =
  | { type: "user"; id: string; username: string }
  | { type: "everyone" }
  | { type: "here" }
  | { type: string; id?: string; username?: string };

export interface ChatPinEvent {
  type: "message.pin";
  schemaVersion: 1;
  channelId: string;
  seq: string;
  message: ChatMessage;
}

export interface ChatForwardEvent {
  type: "message.forward";
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
  content: { version: 1; type: "text"; text: string; attachments?: ChatAttachment[]; mentions?: ChatMention[] };
  createdAt: string;
  clientMessageId: string;
  reactions?: ChatReaction[];
  reactionSeq?: string;
  /** Sequence of the latest `message.attachments` reflected in `content.attachments`. */
  attachmentsSeq?: string;
  pin?: { author: ChatAuthor; createdAt: string } | null;
  pinSeq?: string;
  threadRootId?: string;
  broadcast?: boolean;
  thread?: ChatThreadSummary;
  forward?: { message: ChatMessage | null; seq: string };
  forwardSeq?: string;
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
  | ChatAttachmentsEvent
  | ChatAttachmentProgressEvent
  | ChatPinEvent
  | ChatForwardEvent
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
  return (
    typeof message.id === "string" &&
    typeof message.channelId === "string" &&
    typeof message.seq === "string" &&
    /^(0|[1-9]\d*)$/.test(message.seq) &&
    typeof message.createdAt === "string" &&
    typeof message.clientMessageId === "string" &&
    (message.revision === undefined || (Number.isSafeInteger(message.revision) && message.revision >= 1)) &&
    (message.editedAt === undefined || typeof message.editedAt === "string") &&
    (message.editSeq === undefined ||
      (typeof message.editSeq === "string" && /^(0|[1-9]\d*)$/.test(message.editSeq))) &&
    ((message.revision ?? 1) === 1 || (message.editedAt !== undefined && message.editSeq !== undefined)) &&
    (message.threadRootId === undefined || (typeof message.threadRootId === "string" && !!message.threadRootId)) &&
    (message.broadcast === undefined ||
      (typeof message.broadcast === "boolean" && (!message.broadcast || !!message.threadRootId))) &&
    (message.thread === undefined || isChatThreadSummary(message.thread)) &&
    (message.reactions === undefined || isChatReactions(message.reactions)) &&
    (message.reactionSeq === undefined ||
      (typeof message.reactionSeq === "string" && /^(0|[1-9]\d*)$/.test(message.reactionSeq))) &&
    (message.attachmentsSeq === undefined ||
      (typeof message.attachmentsSeq === "string" && /^(0|[1-9]\d*)$/.test(message.attachmentsSeq))) &&
    (message.pinSeq === undefined || (typeof message.pinSeq === "string" && /^(0|[1-9]\d*)$/.test(message.pinSeq))) &&
    (message.pin === undefined ||
      message.pin === null ||
      (typeof message.pin === "object" &&
        isChatAuthor(message.pin.author) &&
        typeof message.pin.createdAt === "string")) &&
    (message.forwardSeq === undefined ||
      (typeof message.forwardSeq === "string" && /^(0|[1-9]\d*)$/.test(message.forwardSeq))) &&
    (message.forward === undefined ||
      (!!message.forward &&
        typeof message.forward.seq === "string" &&
        /^(0|[1-9]\d*)$/.test(message.forward.seq) &&
        (message.forward.message === null ||
          (!!message.forward.message &&
            message.forward.message.forward === undefined &&
            isChatMessage(message.forward.message))))) &&
    isChatAuthor(message.author) &&
    !!message.content &&
    message.content.version === 1 &&
    message.content.type === "text" &&
    typeof message.content.text === "string" &&
    (message.content.attachments === undefined || Array.isArray(message.content.attachments))
  );
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
  return (
    typeof item.id === "string" &&
    typeof item.name === "string" &&
    typeof item.contentType === "string" &&
    typeof item.size === "number" &&
    ["image", "video", "audio", "file"].includes(item.kind as string) &&
    optionalNumber(item.width) &&
    optionalNumber(item.height) &&
    optionalNumber(item.durationMs) &&
    optionalUrl(item.url) &&
    optionalUrl(item.previewUrl) &&
    (item.status === undefined || ["processing", "ready", "failed"].includes(item.status)) &&
    (item.animated === undefined || typeof item.animated === "boolean")
  );
}

export type AttachmentView =
  | "unavailable"
  | "processing"
  | "failed"
  | "image"
  | "video"
  | "animated"
  | "audio"
  | "file";

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
  return (
    event.type === "message.attachments" &&
    event.schemaVersion === 1 &&
    typeof event.channelId === "string" &&
    typeof event.messageId === "string" &&
    typeof event.seq === "string" &&
    /^(0|[1-9]\d*)$/.test(event.seq) &&
    Array.isArray(event.attachments)
  );
}

export function isChatAttachmentProgressEvent(value: unknown): value is ChatAttachmentProgressEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatAttachmentProgressEvent>;
  return (
    event.type === "attachment.progress" &&
    typeof event.channelId === "string" &&
    typeof event.messageId === "string" &&
    typeof event.attachmentId === "string" &&
    typeof event.percent === "number" &&
    Number.isFinite(event.percent) &&
    event.percent >= 0 &&
    event.percent <= 100
  );
}

export function isChatEditEvent(value: unknown): value is ChatEditEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatEditEvent>;
  return (
    event.type === "message.edited" &&
    event.schemaVersion === 1 &&
    typeof event.channelId === "string" &&
    typeof event.seq === "string" &&
    /^(0|[1-9]\d*)$/.test(event.seq) &&
    isChatMessage(event.message) &&
    event.message.channelId === event.channelId &&
    event.message.editSeq === event.seq &&
    (event.message.revision ?? 1) > 1
  );
}

export function isChatPinEvent(value: unknown): value is ChatPinEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatPinEvent>;
  return (
    event.type === "message.pin" &&
    event.schemaVersion === 1 &&
    typeof event.channelId === "string" &&
    typeof event.seq === "string" &&
    /^(0|[1-9]\d*)$/.test(event.seq) &&
    isChatMessage(event.message) &&
    event.message.channelId === event.channelId &&
    event.message.pinSeq === event.seq
  );
}

export function isChatThreadSummary(value: unknown): value is ChatThreadSummary {
  if (!value || typeof value !== "object") return false;
  const summary = value as Partial<ChatThreadSummary>;
  return (
    Number.isSafeInteger(summary.replyCount) &&
    summary.replyCount! > 0 &&
    typeof summary.seq === "string" &&
    /^(0|[1-9]\d*)$/.test(summary.seq) &&
    Array.isArray(summary.participants) &&
    summary.participants.length <= 5 &&
    summary.participants.every(isChatAuthor) &&
    new Set(summary.participants.map((author) => author.id)).size === summary.participants.length
  );
}

export function isChannelMessage(message: ChatMessage): boolean {
  return !message.threadRootId || message.broadcast === true;
}

export function isChatForwardEvent(value: unknown): value is ChatForwardEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatForwardEvent>;
  return (
    event.type === "message.forward" &&
    event.schemaVersion === 1 &&
    typeof event.channelId === "string" &&
    typeof event.seq === "string" &&
    /^(0|[1-9]\d*)$/.test(event.seq) &&
    isChatMessage(event.message) &&
    !!event.message.forward &&
    event.message.channelId === event.channelId &&
    event.message.forwardSeq === event.seq
  );
}

export function isChatReactions(value: unknown): value is ChatReaction[] {
  return (
    Array.isArray(value) &&
    value.every((reaction: unknown) => {
      if (!reaction || typeof reaction !== "object") return false;
      const item = reaction as Partial<ChatReaction>;
      return (
        typeof item.emoji === "string" &&
        item.emoji.length > 0 &&
        Array.isArray(item.authorIds) &&
        item.authorIds.length > 0 &&
        item.authorIds.every((id) => typeof id === "string") &&
        new Set(item.authorIds).size === item.authorIds.length
      );
    })
  );
}

export function isChatReactionEvent(value: unknown): value is ChatReactionEvent {
  if (!value || typeof value !== "object") return false;
  const event = value as Partial<ChatReactionEvent>;
  return (
    event.type === "message.reactions" &&
    event.schemaVersion === 1 &&
    typeof event.channelId === "string" &&
    typeof event.messageId === "string" &&
    typeof event.seq === "string" &&
    /^(0|[1-9]\d*)$/.test(event.seq) &&
    isChatReactions(event.reactions)
  );
}
