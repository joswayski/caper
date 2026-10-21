import { MAX_ATTACHMENTS } from "./uploads.ts";

/** Only full drafts need a hint explaining how to make room. */
export function attachCapacity(attached: number, max = MAX_ATTACHMENTS) {
  const full = attached >= max;
  return {
    full,
    /** How to make room. */
    hint: full ? "Remove a file to add another." : undefined,
  };
}
