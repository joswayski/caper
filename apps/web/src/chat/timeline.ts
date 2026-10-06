import { sequence, type ChatForwardEvent, type ChatMessage, type ChatPinEvent, type ChatReactionEvent, type ChatThreadSummary } from "./types.ts";

const MAX_PENDING_EVENTS = 256;
type DurableEvent = ChatMessage | ChatReactionEvent | ChatPinEvent | ChatForwardEvent;

export class ChatTimeline {
  private cursorValue = 0n;
  private readonly byId = new Map<string, ChatMessage>();
  private readonly eventBuffer = new Map<bigint, DurableEvent>();
  private readonly unseenReactions = new Map<string, ChatReactionEvent>();
  private readonly forwardUpdates = new Map<string, ChatMessage>();
  private readonly pinUpdates = new Map<string, ChatMessage>();
  private pinSnapshotCursor = 0n;
  private pinnedById = new Map<string, ChatMessage>();
  private readonly threadSummaries = new Map<string, ChatThreadSummary>();
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
    this.threadSummaries.clear();
    if (this.cursorValue === 0n) this.forwardUpdates.clear();
    this.sortedMessages = undefined;
    this.pinSnapshotCursor = 0n;
    this.pinnedById = new Map();
    for (const message of messages) this.merge(message);
    for (const message of pinnedMessages) { this.mergePinMessage(message); this.mergeForwardMessage(message); }
    for (const message of this.pinUpdates.values()) this.mergePinMessage(message);
    this.pinSnapshotCursor = this.cursorValue;
  }

  prepend(messages: ChatMessage[]) {
    for (const message of messages) this.merge(message);
  }

  mergeSent(message: ChatMessage) {
    this.merge(message);
  }

  applyEvent(message: DurableEvent): "applied" | "buffered" | "duplicate" | "overflow" {
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

  private applyContiguous(next: bigint, message: DurableEvent) {
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

  private mergeForwardMessage(message: ChatMessage) {
    if (!message.forward) return;
    const previous = this.forwardUpdates.get(message.id);
    // Source snapshots may be fresher than the asynchronous projection. Its
    // source cursor and the destination replay revision are independent.
    const snapshot = previous && previous.forward && message.forward.message !== null
      && (previous.forward.message === null || sequence(previous.forward.seq) > sequence(message.forward.seq)) ? previous : message;
    const forwardSeq = sequence(previous?.forwardSeq ?? "0") > sequence(message.forwardSeq ?? "0") ? previous?.forwardSeq : message.forwardSeq;
    const updated = { ...snapshot, forwardSeq };
    this.forwardUpdates.delete(message.id);
    this.forwardUpdates.set(message.id, updated);
    // Updates to unloaded forwards must not insert old messages into the list.
    if (this.forwardUpdates.size > MAX_PENDING_EVENTS) this.forwardUpdates.delete(this.forwardUpdates.keys().next().value!);
    const visible = this.byId.get(message.id);
    if (visible && (visible.forward !== updated.forward || visible.forwardSeq !== forwardSeq)) {
      this.byId.set(message.id, { ...visible, forward: updated.forward, forwardSeq });
      this.sortedMessages = undefined;
    }
    const pinned = this.pinnedById.get(message.id);
    if (pinned) this.pinnedById.set(message.id, { ...pinned, forward: updated.forward, forwardSeq });
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
    if (snapshot.pin) {
      const forward = this.forwardUpdates.get(snapshot.id);
      this.pinnedById.set(snapshot.id, forward ? { ...snapshot, forward: forward.forward, forwardSeq: forward.forwardSeq } : snapshot);
    }
    else this.pinnedById.delete(snapshot.id);
    const visible = this.byId.get(snapshot.id);
    if (visible && sequence(snapshot.pinSeq ?? "0") > sequence(visible.pinSeq ?? "0")) {
      this.byId.set(visible.id, { ...visible, pin: snapshot.pin, pinSeq: snapshot.pinSeq });
      this.sortedMessages = undefined;
    }
  }

  private merge(message: DurableEvent) {
    if ("type" in message) {
      if (message.type === "message.pin") this.mergePin(message);
      else if (message.type === "message.forward") this.mergeForwardMessage(message.message);
      else this.mergeReactions(message);
      return;
    }
    const rootId = message.threadRootId ?? message.id;
    const previous = this.threadSummaries.get(rootId);
    if (message.thread && (!previous || sequence(message.thread.seq) > sequence(previous.seq))) {
      this.threadSummaries.set(rootId, message.thread);
      const root = this.byId.get(rootId);
      if (root) this.byId.set(rootId, { ...root, thread: message.thread });
      this.sortedMessages = undefined;
    }
    const summary = this.threadSummaries.get(rootId);
    if (summary) message = { ...message, thread: summary };
    const existing = this.byId.get(message.id);
    if (!existing) {
      this.byId.set(message.id, message);
      this.sortedMessages = undefined;
    } else {
      // reset puts fresh rows first; retain their author metadata even when a
      // cached row carries a more recent HTTP reaction snapshot.
      const newerReactions = sequence(message.reactionSeq ?? "0") > sequence(existing.reactionSeq ?? "0");
      if (newerReactions || (summary && summary !== existing.thread)) {
        this.byId.set(message.id, { ...existing,
          ...(newerReactions ? { reactions: message.reactions, reactionSeq: message.reactionSeq } : {}),
          ...(summary ? { thread: summary } : {}) });
        this.sortedMessages = undefined;
      }
    }
    const unseen = this.unseenReactions.get(message.id);
    if (unseen) { this.mergeReactions(unseen); this.unseenReactions.delete(message.id); }
    this.mergePinMessage(message);
    this.mergeForwardMessage(message);
  }
}
