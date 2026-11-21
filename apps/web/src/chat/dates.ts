// Compare calendar days, not elapsed 24-hour periods (DST days vary in length).
export function dateDivider(value: string, previous?: string, formatter?: Intl.DateTimeFormat): string | undefined {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return undefined;
  if (previous && date.toDateString() === new Date(previous).toDateString()) return undefined;
  return (formatter ?? dayFormatter()).format(date);
}

/** A timeline shares one across its rows: constructing a formatter costs far more than using it. */
export const dayFormatter = () => new Intl.DateTimeFormat(undefined, { dateStyle: "full" });
