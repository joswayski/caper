import { useContext } from "react";
import { DailyIconContext } from "./DailyIconProvider";
import { conversationCaperIndex, dailyIconUrl } from "./daily-icon";

/** Today's Caper character, or a conversation-specific one when seeded. */
export default function DailyCaper({ className, seed }: { className?: string; seed?: string }) {
  const daily = useContext(DailyIconContext);
  const index = seed ? conversationCaperIndex(seed) : daily;
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
