import { describe, expect, it } from "vitest";
import { ARRIVAL_MS, Arrivals } from "./arrivals";

function tracker() {
  let time = 0;
  const arrivals = new Arrivals(() => time);
  return { arrivals, advance: (ms: number) => (time += ms) };
}

describe("Arrivals", () => {
  it("does not animate a channel's first render or a channel switch", () => {
    const { arrivals } = tracker();
    arrivals.observe("channel", "a", ["1", "2"], true);
    expect(arrivals.has("2")).toBe(false);
    arrivals.observe("channel", "b", ["3", "4"], true);
    expect(arrivals.has("4")).toBe(false);
  });

  it("animates rows appended at the live end", () => {
    const { arrivals } = tracker();
    arrivals.observe("channel", "a", ["1", "2"], true);
    arrivals.observe("channel", "a", ["1", "2", "3"], true);
    expect(arrivals.has("3")).toBe(true);
    expect(arrivals.has("2")).toBe(false);
  });

  it("does not animate an empty list's first messages", () => {
    const { arrivals } = tracker();
    arrivals.observe("channel", "a", [], true);
    arrivals.observe("channel", "a", ["1", "2"], true);
    expect(arrivals.has("1")).toBe(false);
  });

  it("ignores older pages and newer pages loaded towards the live end", () => {
    const { arrivals } = tracker();
    arrivals.observe("channel", "a", ["5", "6"], false);
    arrivals.observe("channel", "a", ["3", "4", "5", "6"], false);
    arrivals.observe("channel", "a", ["3", "4", "5", "6", "7", "8"], true);
    expect(["3", "4", "7", "8"].some((id) => arrivals.has(id))).toBe(false);
    arrivals.observe("channel", "a", ["1", "2", "3", "4", "5", "6", "7", "8"], true);
    expect(arrivals.has("1")).toBe(false);
  });

  it("animates a confirmed send once, keeping its client message ID", () => {
    const { arrivals, advance } = tracker();
    arrivals.observe("channel", "a", ["1"], true);
    arrivals.observe("channel", "a", ["1", "sent"], true);
    expect(arrivals.has("sent")).toBe(true);
    advance(ARRIVAL_MS + 1);
    arrivals.observe("channel", "a", ["1", "sent"], true);
    expect(arrivals.has("sent")).toBe(false);
  });

  it("keeps channel and thread lists apart", () => {
    const { arrivals } = tracker();
    arrivals.observe("channel", "a", ["1"], true);
    arrivals.observe("thread", "root", ["r1"], true);
    arrivals.observe("thread", "root", ["r1", "r2"], true);
    arrivals.observe("channel", "a", ["1"], true);
    expect(arrivals.has("r2")).toBe(true);
  });
});
