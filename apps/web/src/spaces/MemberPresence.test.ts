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
      await act(async () => update([{ userId: "ana", status: "online" }]));
      expect(rows("Online")).toEqual(["ANA", "BO", "DEE"]);
      expect(host.querySelector('section[aria-label="Offline"]')).toBeNull();
    } finally {
      await act(async () => root.unmount());
    }
  });
});
