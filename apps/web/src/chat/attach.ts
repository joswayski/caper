import { MAX_ATTACHMENTS } from "./uploads.ts";

/** The attach drawer's limit line, worded as on every client. Drafts below
 * the limit show nothing, so only a full one has a label. */
export function attachCapacity(attached: number, max = MAX_ATTACHMENTS) {
  const full = attached >= max;
  return {
    full,
    label: full ? `${max} of ${max} attached` : undefined,
    /** How to make room. */
    hint: full ? "Remove a file to add another." : undefined,
  };
}
