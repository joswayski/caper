import { sequence, type ChatAttachmentsEvent, type ChatMessage, type ChatReactionEvent } from "./types.ts";

const MAX_PENDING_EVENTS = 256;

export type ChatTimelineEvent = ChatMessage | ChatReactionEvent | ChatAttachmentsEvent;

export class ChatTimeline {
  private cursorValue = 0n;
  private readonly byId = new Map<string, ChatMessage>();
  private readonly eventBuffer = new Map<bigint, ChatTimelineEvent>();
  private readonly unseenReactions = new Map<string, ChatReactionEvent>();
  private readonly unseenAttachments = new Map<string, ChatAttachmentsEvent>();
  private sortedMessages?: ChatMessage[];

  get cursor() { return this.cursorValue.toString(); }

  get messages() {
    return this.sortedMessages ??= [...this.byId.values()].sort((left, right) => {
      const order = sequence(left.seq) - sequence(right.seq);
      return order < 0n ? -1 : order > 0n ? 1 : left.id.localeCompare(right.id);
    });
  }

  reset(messages: ChatMessage[], cursor: string) {
    this.cursorValue = sequence(cursor);
    this.byId.clear();
    this.eventBuffer.clear();
    this.unseenReactions.clear();
    this.unseenAttachments.clear();
    this.sortedMessages = undefined;
    for (const message of messages) this.merge(message);
  }

  prepend(messages: ChatMessage[]) {
    for (const message of messages) this.merge(message);
  }

  mergeSent(message: ChatMessage) {
    this.merge(message);
  }

  applyEvent(message: ChatTimelineEvent): "applied" | "buffered" | "duplicate" | "overflow" {
    const next = sequence(message.seq);
    if (next <= this.cursorValue) {
      this.merge(message);
      return "duplicate";
    }
    if (next > this.cursorValue + 1n) {
      if (this.eventBuffer.size >= MAX_PENDING_EVENTS) return "overflow";
      this.eventBuffer.set(next, message);
      return "buffered";
    }
    this.applyContiguous(next, message);
    if (this.unseenReactions.size > MAX_PENDING_EVENTS || this.unseenAttachments.size > MAX_PENDING_EVENTS) return "overflow";
    return "applied";
  }

  private applyContiguous(next: bigint, message: ChatTimelineEvent) {
    this.merge(message);
    this.cursorValue = next;
    while (true) {
      const sequenceNumber = this.cursorValue + 1n;
      const buffered = this.eventBuffer.get(sequenceNumber);
      if (!buffered) break;
      this.eventBuffer.delete(sequenceNumber);
      this.merge(buffered);
      this.cursorValue = sequenceNumber;
    }
  }

  // HTTP acknowledgements update the snapshot, never the replay cursor.
  mergeReactions(event: ChatReactionEvent) {
    const existing = this.byId.get(event.messageId);
    if (existing) {
      if (sequence(event.seq) > sequence(existing.reactionSeq ?? "0")) {
        this.byId.set(existing.id, { ...existing, reactions: event.reactions, reactionSeq: event.seq });
        this.sortedMessages = undefined;
      }
    } else {
      const previous = this.unseenReactions.get(event.messageId);
      if (!previous || sequence(event.seq) > sequence(previous.seq)) this.unseenReactions.set(event.messageId, event);
    }
  }

  /** Same revision rule as reactions: a replayed older event (for example a
   * stale "processing") never overwrites a newer snapshot. */
  private mergeAttachments(event: ChatAttachmentsEvent) {
    const existing = this.byId.get(event.messageId);
    if (existing) {
      if (sequence(event.seq) > sequence(existing.attachmentsSeq ?? "0")) {
        this.byId.set(existing.id, { ...existing, content: { ...existing.content, attachments: event.attachments }, attachmentsSeq: event.seq });
        this.sortedMessages = undefined;
      }
    } else {
      const previous = this.unseenAttachments.get(event.messageId);
      if (!previous || sequence(event.seq) > sequence(previous.seq)) this.unseenAttachments.set(event.messageId, event);
    }
  }

  private merge(message: ChatTimelineEvent) {
    if ("type" in message) {
      if (message.type === "message.reactions") this.mergeReactions(message);
      else this.mergeAttachments(message);
      return;
    }
    const existing = this.byId.get(message.id);
    if (!existing) {
      this.byId.set(message.id, message);
      this.sortedMessages = undefined;
    } else {
      // reset puts fresh rows first; retain their author metadata even when a
      // cached row carries a more recent reaction or attachment snapshot.
      let next = existing;
      if (sequence(message.reactionSeq ?? "0") > sequence(existing.reactionSeq ?? "0")) {
        next = { ...next, reactions: message.reactions, reactionSeq: message.reactionSeq };
      }
      if (sequence(message.attachmentsSeq ?? "0") > sequence(existing.attachmentsSeq ?? "0")) {
        next = { ...next, content: { ...next.content, attachments: message.content.attachments }, attachmentsSeq: message.attachmentsSeq };
      }
      if (next !== existing) {
        this.byId.set(message.id, next);
        this.sortedMessages = undefined;
      }
    }
    const unseen = this.unseenReactions.get(message.id);
    if (unseen) { this.mergeReactions(unseen); this.unseenReactions.delete(message.id); }
    const unseenAttachments = this.unseenAttachments.get(message.id);
    if (unseenAttachments) { this.mergeAttachments(unseenAttachments); this.unseenAttachments.delete(message.id); }
  }
}
