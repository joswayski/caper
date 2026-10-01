/** Immutable v1 tile layout: 32×25 row-major, IDs persisted on the account. */
export function avatarPosition(avatarId?: number | null): string | undefined {
  if (typeof avatarId !== "number" || !Number.isInteger(avatarId) || avatarId < 0 || avatarId > 799) return;
  return `${(avatarId % 32) * 100 / 31}% ${Math.floor(avatarId / 32) * 100 / 24}%`;
}
