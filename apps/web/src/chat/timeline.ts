import { sequence, type ChatMessage } from "./types.ts";

const MAX_PENDING_EVENTS = 256;

export class ChatTimeline {
  private cursorValue = 0n;
  private readonly byId = new Map<string, ChatMessage>();
  private readonly eventBuffer = new Map<bigint, ChatMessage>();

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
    for (const message of messages) this.merge(message);
  }

  prepend(messages: ChatMessage[]) {
    for (const message of messages) this.merge(message);
  }

  mergeSent(message: ChatMessage) {
    this.merge(message);
  }

  applyEvent(message: ChatMessage): "applied" | "buffered" | "duplicate" | "overflow" {
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
    return "applied";
  }

  private applyContiguous(next: bigint, message: ChatMessage) {
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

  private merge(message: ChatMessage) {
    const existing = this.byId.get(message.id);
    if (!existing) this.byId.set(message.id, message);
  }
}
