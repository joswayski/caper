// Unsent composer text per conversation, kept while the page is open so
// switching channels or DMs and coming back restores it. Signing out reloads
// the page, which discards every draft.
const drafts = new Map<string, string>();

export const readDraft = (key: string) => drafts.get(key) ?? "";

export function saveDraft(key: string, text: string) {
  if (text) drafts.set(key, text);
  else drafts.delete(key);
}
