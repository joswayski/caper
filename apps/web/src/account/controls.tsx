/** Form controls shared by sign-in and the profile form, so the two never drift apart. */

/** Chrome and Safari paint autofilled fields light blue/yellow; an inset shadow keeps them on the dark surface. */
export const fieldClass =
  "w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4 read-only:opacity-55 disabled:cursor-not-allowed disabled:opacity-55 autofill:shadow-[inset_0_0_0_100px_var(--surface)] autofill:[-webkit-text-fill-color:var(--text)]";

/**
 * Right-aligned. Give each use a min-width that fits its longest label, so the
 * pending label never resizes it, and set aria-busy while its request runs.
 */
export const primaryButtonClass =
  "ml-auto flex w-fit items-center justify-center gap-4 rounded-control border border-terracotta bg-terracotta px-5 py-4 font-bold text-content transition-colors duration-150 enabled:hover:border-terracotta-bright enabled:hover:bg-terracotta-bright disabled:opacity-60 aria-busy:cursor-wait focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4";

export function Spinner() {
  return (
    <span
      className="size-[1em] animate-spin rounded-full border-2 border-current border-r-transparent"
      aria-hidden="true"
    />
  );
}
