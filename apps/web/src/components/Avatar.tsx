import { avatarPosition } from "../account/avatar";

/** Decorative; the adjacent account/member/author name supplies the label. */
export default function Avatar({ avatarId, name }: { avatarId?: number | null; name: string }) {
  const position = avatarPosition(avatarId);
  return <span aria-hidden="true" data-avatar-id={position ? avatarId : undefined} style={{
    display: "grid", placeItems: "center", width: "100%", height: "100%", borderRadius: "inherit",
    backgroundColor: position ? undefined : "var(--surface-composer)",
    backgroundImage: position ? "url('/images/avatars/capers-v1.webp')" : undefined,
    backgroundSize: "3200% 2500%", backgroundPosition: position,
  }}>{position ? null : name.slice(0, 1).toUpperCase()}</span>;
}
