/** Versioned vector artwork; IDs remain persisted on the account. */
export function avatarUrl(avatarId?: number | null): string | undefined {
  if (typeof avatarId !== "number" || !Number.isInteger(avatarId) || avatarId < 0 || avatarId > 799) return;
  return `/images/avatars/v2/${avatarId}.svg`;
}
