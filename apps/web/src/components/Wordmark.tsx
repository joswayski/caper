import { useContext } from "react";
import { DailyIconContext } from "./RotatingFavicon";
import { dailyIconUrl } from "./daily-icon";

export default function Wordmark({ rotating = true }: { rotating?: boolean }) {
  const index = useContext(DailyIconContext);
  return (
    <a className="wordmark" href="/" aria-label="Caper home">
      <img src="/caper-wordmark-letters.svg" alt="" width="1042" height="276" />
      {/* Until the client picks today's character, keep the slot empty rather than
          flashing the default face and swapping it after hydration. */}
      <img
        className="wordmark-character"
        src={dailyIconUrl(rotating ? index : null)}
        alt=""
        width="256"
        height="256"
        data-pending={rotating && index === null ? "" : undefined}
      />
    </a>
  );
}
