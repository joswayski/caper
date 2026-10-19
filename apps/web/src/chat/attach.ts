import { MAX_ATTACHMENTS } from "./uploads.ts";

/** The attach drawer's capacity line, worded as on every client. */
export function attachCapacity(attached: number, max = MAX_ATTACHMENTS) {
  const count = Math.max(0, Math.min(attached, max));
  const full = count >= max;
  return {
    count,
    full,
    label: count === 0 ? `Up to ${max} files` : `${count} of ${max} attached`,
    /** Shown once full: how to make room. */
    hint: full ? "Remove a file to add another." : undefined,
  };
}
