// Edit profile lives with the account controls; a profile card elsewhere on
// the page (your own name in chat, the member list) asks that host to open it.

const hosts = new Set<() => void>();

/** Registers the page's Edit profile dialog; returns the unregister function. */
export function hostProfileEditor(open: () => void) {
  hosts.add(open);
  return () => void hosts.delete(open);
}

/** Whether a mounted host can open Edit profile. */
export function canEditProfile() {
  return hosts.size > 0;
}

/** Opens Edit profile on the most recently mounted host. */
export function editProfile() {
  [...hosts].at(-1)?.();
}
