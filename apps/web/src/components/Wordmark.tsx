import { useContext } from "react";
import { DailyIconContext } from "./RotatingFavicon";
import { dailyIconUrl } from "./daily-icon";

export default function Wordmark() {
  const index = useContext(DailyIconContext);
  return <a className="wordmark" href="/" aria-label="Caper home">
    <img src="/caper-wordmark-letters.svg" alt="" width="1042" height="276" />
    <img className="wordmark-character" src={dailyIconUrl(index)} alt="" width="256" height="256" />
  </a>;
}
