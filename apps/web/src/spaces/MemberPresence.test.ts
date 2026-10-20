// @vitest-environment happy-dom
import { act, createElement } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it, vi } from "vitest";
import MemberPresence, { memberGroups } from "./MemberPresence";
import { watchPresence } from "../gateway/client";
import type { Member } from "./client";

vi.mock("../gateway/client", () => ({ watchPresence: vi.fn(() => () => {}) }));

const member = (id: string, owner = false): Member =>
  ({ id, avatarId: 0, username: id, displayName: id.toUpperCase(), owner }) as Member;

describe("memberGroups", () => {
  const members = [member("ana", true), member("bo"), member("cy"), member("dee")];

  it("leads a private channel's list with its readers, keeping the space order in each group", () => {
    const { inChannel, others } = memberGroups(members, ["dee", "ana"]);
    expect(inChannel?.map((m) => m.id)).toEqual(["ana", "dee"]);
    expect(others.map((m) => m.id)).toEqual(["bo", "cy"]);
  });

  it("keeps one ungrouped list for public channels", () => {
    const { inChannel, others } = memberGroups(members);
    expect(inChannel).toBeUndefined();
    expect(others).toBe(members);
  });

  it("ignores reader ids that are no longer space members", () => {
    const { inChannel, others } = memberGroups(members, ["gone", "bo"]);
    expect(inChannel?.map((m) => m.id)).toEqual(["bo"]);
    expect(others).toHaveLength(3);
  });

  it("groups live members including idle, keeps unknown separate, and moves rows on presence changes", async () => {
    const host = document.createElement("div");
    const root = createRoot(host);
    try {
      await act(async () =>
        root.render(
          createElement(MemberPresence, {
            spaceId: "space",
            members,
            onMessagePerson: async () => {},
            onBlockAuthor: () => {},
            profileSpaces: () => [],
            onClose: () => {},
          }),
        ),
      );
      const rows = (label: string) =>
        [...host.querySelectorAll(`section[aria-label="${label}"] strong`)].map((el) => el.textContent);
      expect(rows("Status unavailable")).toEqual(["ANA", "BO", "CY", "DEE"]);
      const update = vi.mocked(watchPresence).mock.calls.at(-1)![2];
      await act(async () =>
        update([
          { userId: "ana", status: "offline" },
          { userId: "bo", status: "idle" },
          { userId: "dee", status: "online" },
        ]),
      );
      expect(rows("Online")).toEqual(["BO", "DEE"]);
      expect(rows("Offline")).toEqual(["ANA"]);
      expect(rows("Status unavailable")).toEqual(["CY"]);
      expect(host.querySelector('section[aria-label="Online"] .section-count')?.textContent).toBe("2");
      await act(async () =>
        update([
          { userId: "ana", status: "online" },
          { userId: "bo", status: "idle" },
          { userId: "dee", status: "online" },
        ]),
      );
      expect(rows("Online")).toEqual(["ANA", "BO", "DEE"]);
      expect(host.querySelector('section[aria-label="Offline"]')).toBeNull();
      await act(async () => vi.mocked(watchPresence).mock.calls.at(-1)![3]!(false));
      expect(rows("Status unavailable")).toEqual(["ANA", "BO", "CY", "DEE"]);
    } finally {
      await act(async () => root.unmount());
    }
  });

  it("filters the whole roster by name or username and clears back to the same live groups", async () => {
    const roster = Array.from({ length: 151 }, (_, index) => member(`person_${index}`));
    roster[150] = { ...roster[150], displayName: "Beyond the old page" };
    const host = document.createElement("div");
    document.body.append(host);
    const root = createRoot(host);
    const click = async (label: string) =>
      act(async () => host.querySelector<HTMLButtonElement>(`[aria-label="${label}"]`)!.click());
    const filter = async (value: string) =>
      act(async () => {
        const input = host.querySelector("input")!;
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")!.set!.call(input, value);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    try {
      await act(async () =>
        root.render(
          createElement(MemberPresence, {
            spaceId: "space",
            members: roster,
            channel: { name: "private", memberIds: [roster[150].id] },
            onMessagePerson: async () => {},
            onBlockAuthor: () => {},
            profileSpaces: () => [],
            onClose: () => {},
          }),
        ),
      );
      const update = vi.mocked(watchPresence).mock.calls.at(-1)![2];
      await act(async () =>
        update(roster.map((person, index) => ({ userId: person.id, status: index === 150 ? "online" : "offline" }))),
      );
      expect(host.querySelectorAll("li")).toHaveLength(151);
      expect(host.textContent).not.toContain("Previous");
      await click("Search members");
      expect(document.activeElement).toBe(host.querySelector("input"));
      await filter("  PERSON_150  ");
      expect(host.querySelectorAll("li")).toHaveLength(1);
      expect(host.querySelector('section[aria-label="In #private"] strong')?.textContent).toBe("Beyond the old page");
      expect(host.querySelector('section[aria-label="Online"] .section-count')?.textContent).toBe("1");
      expect(host.querySelector(".member-presence-heading .section-count")?.textContent).toBe("151");
      await filter("Beyond THE");
      expect(host.querySelectorAll("li")).toHaveLength(1);
      await filter("nobody matches");
      expect(host.querySelector('[role="status"]')?.textContent).toBe("No members found.");
      expect(host.querySelectorAll("li")).toHaveLength(0);
      await click("Clear and close member search");
      expect(host.querySelector("input")).toBeNull();
      expect(host.querySelectorAll("li")).toHaveLength(151);
      expect(host.querySelector('section[aria-label="Online"] strong')?.textContent).toBe("Beyond the old page");
      await click("Search members");
      await filter("person_1");
      await act(async () =>
        host.querySelector("input")!.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })),
      );
      expect(host.querySelector("input")).toBeNull();
      expect(host.querySelectorAll("li")).toHaveLength(151);
    } finally {
      await act(async () => root.unmount());
      host.remove();
    }
  });
});
