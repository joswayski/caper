// Compare calendar days, not elapsed 24-hour periods (DST days vary in length).
export function dateDivider(value: string, previous?: string): string | undefined {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return undefined;
  if (previous && date.toDateString() === new Date(previous).toDateString()) return undefined;
  return new Intl.DateTimeFormat(undefined, { dateStyle: "full" }).format(date);
}
