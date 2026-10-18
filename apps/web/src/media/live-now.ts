// Live channels (voice cards in the channel list): shared by the call and the homepage simulation.
/** Faces your call card shows before folding the rest into "+N": two rows of five. */
export const CALL_CARD_FACES = 10;
/** Faces the phone strip shows, in the card's order, before folding the rest into "+N". */
export const STRIP_FACES = 5;

/** "Maya", "Maya and Alex", "Maya, Alex and Sam", "Maya, Alex and 6 more". */
export function liveNames(names: string[]) {
  if (names.length <= 1) return names[0] ?? "";
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  if (names.length === 3) return `${names[0]}, ${names[1]} and ${names[2]}`;
  return `${names[0]}, ${names[1]} and ${names.length - 2} more`;
}

/** The faces a call card shows and how many fold into its "+N" tile. */
export function callCardFaces<T>(people: T[], capacity = CALL_CARD_FACES) {
  if (people.length <= capacity) return { shown: people, hidden: [] as T[] };
  return { shown: people.slice(0, capacity - 1), hidden: people.slice(capacity - 1) };
}
