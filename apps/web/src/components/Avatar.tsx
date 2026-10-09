import { avatarUrl } from "../account/avatar";

/** First character of a name, whole even when it is an emoji or other surrogate pair. */
export function initial(name: string) {
  return (Array.from(name.trim())[0] ?? "").toUpperCase();
}

/** Decorative; the adjacent account/member/author name supplies the label. */
export default function Avatar({ avatarId, name }: { avatarId?: number | null; name: string }) {
  const url = avatarUrl(avatarId);
  return (
    <span
      aria-hidden="true"
      data-avatar-id={url ? avatarId : undefined}
      style={{
        display: "grid",
        placeItems: "center",
        width: "100%",
        height: "100%",
        borderRadius: "inherit",
        backgroundColor: url ? undefined : "var(--surface-composer)",
        backgroundImage: url ? `url('${url}')` : undefined,
        backgroundSize: "100% 100%",
        backgroundRepeat: "no-repeat",
      }}
    >
      {url ? null : initial(name)}
    </span>
  );
}
