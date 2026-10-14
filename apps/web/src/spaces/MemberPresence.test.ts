import { describe, expect, it } from "vitest";
import { memberGroups } from "./MemberPresence";
import type { Member } from "./client";

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
});
