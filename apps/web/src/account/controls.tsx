/** Form controls shared by sign-in and the profile form, so the two never drift apart. */

/** Chrome and Safari paint autofilled fields light blue/yellow; an inset shadow keeps them on the dark surface. */
export const fieldClass =
  "w-full rounded-control border border-border bg-surface px-3.5 py-[13px] text-content transition-[border-color,outline-color] duration-150 outline-2 outline-transparent focus-visible:border-terracotta focus-visible:outline-terracotta/22 read-only:opacity-55 disabled:cursor-not-allowed disabled:opacity-55 autofill:shadow-[inset_0_0_0_100px_var(--surface)] autofill:[-webkit-text-fill-color:var(--text)]";

/**
 * Right-aligned. Give each use a min-width that fits its longest label, so the
 * pending label never resizes it, and set aria-busy while its request runs.
 */
export const primaryButtonClass =
  "ml-auto flex w-fit items-center justify-center gap-4 rounded-full border border-white/55 bg-gradient-to-b from-[#faf9f6] to-[#eae8e1] px-6 py-3.5 font-bold text-on-action shadow-[inset_0_1px_0_rgba(255,255,255,0.9),0_2px_6px_rgba(0,0,0,0.38),0_6px_16px_rgba(0,0,0,0.24)] transition-[filter,transform,background-color,border-color] duration-150 enabled:hover:brightness-[1.04] enabled:active:scale-[0.97] motion-reduce:enabled:active:scale-100 disabled:border-border disabled:bg-surface-raised disabled:bg-none disabled:text-content-muted disabled:shadow-none aria-busy:cursor-wait focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-4";

export function Spinner() {
  return (
    <span
      className="size-[1em] animate-spin rounded-full border-2 border-current border-r-transparent"
      aria-hidden="true"
    />
  );
}
