/** Form controls shared by sign-in and the profile form, so the two never drift apart. */

/** Chrome and Safari paint autofilled fields light blue/yellow; an inset shadow keeps them on the dark surface. */
export const fieldClass =
  "w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content transition-[border-color,outline-color] duration-150 outline-2 outline-transparent focus-visible:border-terracotta focus-visible:outline-terracotta/22 read-only:opacity-55 disabled:cursor-not-allowed disabled:opacity-55 autofill:shadow-[inset_0_0_0_100px_var(--surface)] autofill:[-webkit-text-fill-color:var(--text)]";

/**
 * Right-aligned. Give each use a min-width that fits its longest label, so the
 * pending label never resizes it, and set aria-busy while its request runs.
 */
export const primaryButtonClass =
  "ml-auto flex w-fit items-center justify-center gap-4 rounded-control border border-action bg-action px-5 py-4 font-bold text-on-action transition-colors duration-150 enabled:hover:border-action-hover enabled:hover:bg-action-hover enabled:active:border-action-press enabled:active:bg-action-press disabled:border-border disabled:bg-surface-raised disabled:text-content-muted aria-busy:cursor-wait focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4";

export function Spinner() {
  return (
    <span
      className="size-[1em] animate-spin rounded-full border-2 border-current border-r-transparent"
      aria-hidden="true"
    />
  );
}
