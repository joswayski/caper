import { useSyncExternalStore, type ReactNode } from "react";

/**
 * Who is audibly speaking, kept outside Call's state. Speaking flips several
 * times a second in a lively call; as state it re-rendered the whole call view,
 * conversation included. Here a flip re-renders only the faces and rows showing it.
 */
export class SpeakingStore {
  private readonly ids = new Set<string>();
  private readonly listeners = new Set<() => void>();

  set(id: string, active: boolean) {
    if (this.ids.has(id) === active) return;
    if (active) this.ids.add(id);
    else this.ids.delete(id);
    for (const listener of this.listeners) listener();
  }

  has(id: string) {
    return this.ids.has(id);
  }

  subscribe = (listener: () => void) => {
    this.listeners.add(listener);
    return () => void this.listeners.delete(listener);
  };
}

/** Renders `children` with whether any of `people` is speaking and unmuted. */
export function Speaking({
  store,
  people,
  children,
}: {
  store: SpeakingStore;
  people: { id: string; muted: boolean }[];
  children: (speaking: boolean) => ReactNode;
}) {
  const speaking = useSyncExternalStore(
    store.subscribe,
    () => people.some((person) => !person.muted && store.has(person.id)),
    () => false,
  );
  return children(speaking);
}
