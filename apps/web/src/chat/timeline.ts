import { sequence, type ChatMessage, type ChatReactionEvent } from "./types.ts";

const MAX_PENDING_EVENTS = 256;

export class ChatTimeline {
  private cursorValue = 0n;
  private readonly byId = new Map<string, ChatMessage>();
  private readonly eventBuffer = new Map<bigint, ChatMessage | ChatReactionEvent>();
  private readonly unseenReactions = new Map<string, ChatReactionEvent>();

  get cursor() { return this.cursorValue.toString(); }

  get messages() {
    return [...this.byId.values()].sort((left, right) => {
      const order = sequence(left.seq) - sequence(right.seq);
      return order < 0n ? -1 : order > 0n ? 1 : left.id.localeCompare(right.id);
    });
  }

  reset(messages: ChatMessage[], cursor: string) {
    this.cursorValue = sequence(cursor);
    this.byId.clear();
    this.eventBuffer.clear();
    this.unseenReactions.clear();
    for (const message of messages) this.merge(message);
  }

  prepend(messages: ChatMessage[]) {
    for (const message of messages) this.merge(message);
  }

  mergeSent(message: ChatMessage) {
    this.merge(message);
  }

  applyEvent(message: ChatMessage | ChatReactionEvent): "applied" | "buffered" | "duplicate" | "overflow" {
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
    if (this.unseenReactions.size > MAX_PENDING_EVENTS) return "overflow";
    return "applied";
  }

  private applyContiguous(next: bigint, message: ChatMessage | ChatReactionEvent) {
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
      }
    } else {
      const previous = this.unseenReactions.get(event.messageId);
      if (!previous || sequence(event.seq) > sequence(previous.seq)) this.unseenReactions.set(event.messageId, event);
    }
  }

  private merge(message: ChatMessage | ChatReactionEvent) {
    if ("type" in message) { this.mergeReactions(message); return; }
    const existing = this.byId.get(message.id);
    if (!existing || sequence(message.reactionSeq ?? "0") > sequence(existing.reactionSeq ?? "0")) this.byId.set(message.id, message);
    const unseen = this.unseenReactions.get(message.id);
    if (unseen) { this.mergeReactions(unseen); this.unseenReactions.delete(message.id); }
  }
}
