export interface DailyIcon {
  day: number;
  index: number;
}

/** Transparent branding artwork; profile avatars keep their circular backgrounds. */
export function dailyIconUrl(index: number | null): string {
  return index === null ? "/caper-face.svg?v=3" : `/images/branding/v1/${index}.svg`;
}

/** Installation-local choice; saved profile avatars are unrelated. */
export function dailyIcon(previous: Partial<DailyIcon> | null, now: number, random = Math.random): DailyIcon {
  const day = Math.floor(now / 86_400_000);
  const index = typeof previous?.index === "number" && Number.isInteger(previous.index) && previous.index >= 0 && previous.index < 800
    ? previous.index : undefined;
  if (previous?.day === day && index !== undefined) return { day, index };
  const candidate = Math.floor(random() * (index === undefined ? 800 : 799));
  return { day, index: index === undefined || candidate < index ? candidate : candidate + 1 };
}
