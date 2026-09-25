/** Other people who joined or left between two voice rosters of the same session. */
export function rosterChanges(previous: ReadonlySet<string> | undefined, next: ReadonlySet<string>, selfId: string) {
  if (!previous) return { joined: false, left: false };
  const others = (ids: ReadonlySet<string>) => [...ids].filter((id) => id !== selfId);
  return {
    joined: others(next).some((id) => !previous.has(id)),
    left: others(previous).some((id) => !next.has(id)),
  };
}
