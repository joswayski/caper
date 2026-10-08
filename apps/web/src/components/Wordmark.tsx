import { useContext } from "react";
import { DailyIconContext } from "./RotatingFavicon";
import { dailyIconUrl } from "./daily-icon";

export default function Wordmark({ rotating = true }: { rotating?: boolean }) {
  const index = useContext(DailyIconContext);
  // The day's character is chosen after hydration; fade it in rather than
  // flashing the default face and swapping it a moment later.
  const pending = rotating && index === null;
  return (
    <a className="wordmark" href="/" aria-label="Caper home">
      <img src="/caper-wordmark-letters.svg" alt="" width="1042" height="276" />
      <img
        className="wordmark-character"
        data-pending={pending || undefined}
        src={dailyIconUrl(rotating ? index : null)}
        alt=""
        width="256"
        height="256"
      />
    </a>
  );
}
