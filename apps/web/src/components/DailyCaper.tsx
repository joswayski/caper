import { useContext } from "react";
import { DailyIconContext } from "./RotatingFavicon";
import { dailyIconUrl } from "./daily-icon";

/** Today's Caper character as a decorative illustration, matching the browser tab. */
export default function DailyCaper({ className }: { className?: string }) {
  const index = useContext(DailyIconContext);
  return (
    <img
      className={className}
      src={dailyIconUrl(index)}
      alt=""
      aria-hidden="true"
      width="256"
      height="256"
      draggable={false}
    />
  );
}
