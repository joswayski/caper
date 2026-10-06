import { sequence, type ChatMessage, type ChatPinEvent, type ChatReactionEvent } from "./types.ts";

const MAX_PENDING_EVENTS = 256;

export class ChatTimeline {
  private cursorValue = 0n;
  private readonly byId = new Map<string, ChatMessage>();
  private readonly eventBuffer = new Map<bigint, ChatMessage | ChatReactionEvent | ChatPinEvent>();
  private readonly unseenReactions = new Map<string, ChatReactionEvent>();
  private readonly pinUpdates = new Map<string, ChatMessage>();
  private pinSnapshotCursor = 0n;
  private pinnedById = new Map<string, ChatMessage>();
  private sortedMessages?: ChatMessage[];

  get cursor() { return this.cursorValue.toString(); }

  get messages() {
    return this.sortedMessages ??= [...this.byId.values()].sort((left, right) => {
      const order = sequence(left.seq) - sequence(right.seq);
      return order < 0n ? -1 : order > 0n ? 1 : left.id.localeCompare(right.id);
    });
  }

  get pinnedMessages() { return [...this.pinnedById.values()].sort((a, b) => sequence(b.pinSeq ?? "0") > sequence(a.pinSeq ?? "0") ? 1 : -1); }

  reset(messages: ChatMessage[], cursor: string, pinnedMessages: ChatMessage[] = []) {
    this.cursorValue = sequence(cursor);
    // A complete history snapshot supersedes every pin update through its
    // cursor, including pins removed while offline. Only newer HTTP acks survive.
    for (const [id, message] of this.pinUpdates) {
      if (this.cursorValue === 0n || sequence(message.pinSeq ?? "0") < this.cursorValue) this.pinUpdates.delete(id);
    }
    this.byId.clear();
    this.eventBuffer.clear();
    this.unseenReactions.clear();
    this.sortedMessages = undefined;
    this.pinSnapshotCursor = 0n;
    this.pinnedById = new Map();
    for (const message of messages) this.merge(message);
    for (const message of pinnedMessages) this.mergePinMessage(message);
    for (const message of this.pinUpdates.values()) this.mergePinMessage(message);
    this.pinSnapshotCursor = this.cursorValue;
  }

  prepend(messages: ChatMessage[]) {
    for (const message of messages) this.merge(message);
  }

  mergeSent(message: ChatMessage) {
    this.merge(message);
  }

  applyEvent(message: ChatMessage | ChatReactionEvent | ChatPinEvent): "applied" | "buffered" | "duplicate" | "overflow" {
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

  private applyContiguous(next: bigint, message: ChatMessage | ChatReactionEvent | ChatPinEvent) {
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

  mergePin(event: ChatPinEvent) {
    this.mergePinMessage(event.message);
  }

  private mergePinMessage(message: ChatMessage) {
    const previous = this.pinUpdates.get(message.id);
    if (this.pinSnapshotCursor > 0n && sequence(message.pinSeq ?? "0") <= this.pinSnapshotCursor) {
      // The complete collection also governs messages outside loaded history.
      // Overlay old pages, but ignore acknowledgements already covered by it.
      const visible = this.byId.get(message.id);
      const pin = previous?.pin ?? null;
      const pinSeq = previous?.pinSeq ?? this.pinSnapshotCursor.toString();
      if (visible && (previous || visible.pin) && (visible.pin !== pin || visible.pinSeq !== pinSeq)) {
        this.byId.set(visible.id, { ...visible, pin, pinSeq });
        this.sortedMessages = undefined;
      }
      return;
    }
    const snapshot = previous && sequence(previous.pinSeq ?? "0") > sequence(message.pinSeq ?? "0") ? previous : message;
    if (snapshot.pinSeq !== undefined) this.pinUpdates.set(snapshot.id, snapshot);
    if (snapshot.pin) this.pinnedById.set(snapshot.id, snapshot);
    else this.pinnedById.delete(snapshot.id);
    const visible = this.byId.get(snapshot.id);
    if (visible && sequence(snapshot.pinSeq ?? "0") > sequence(visible.pinSeq ?? "0")) {
      this.byId.set(visible.id, { ...visible, pin: snapshot.pin, pinSeq: snapshot.pinSeq });
      this.sortedMessages = undefined;
    }
  }

  private merge(message: ChatMessage | ChatReactionEvent | ChatPinEvent) {
    if ("type" in message) { message.type === "message.pin" ? this.mergePin(message) : this.mergeReactions(message); return; }
    const existing = this.byId.get(message.id);
    if (!existing) {
      this.byId.set(message.id, message);
      this.sortedMessages = undefined;
    } else if (sequence(message.reactionSeq ?? "0") > sequence(existing.reactionSeq ?? "0")) {
      // reset puts fresh rows first; retain their author metadata even when a
      // cached row carries a more recent HTTP reaction snapshot.
      this.byId.set(message.id, { ...existing, reactions: message.reactions, reactionSeq: message.reactionSeq });
      this.sortedMessages = undefined;
    }
    const unseen = this.unseenReactions.get(message.id);
    if (unseen) { this.mergeReactions(unseen); this.unseenReactions.delete(message.id); }
    this.mergePinMessage(message);
  }
}
