import { useContext } from "react";
import { avatarUrl } from "../account/avatar";
import { DailyIconContext } from "./RotatingFavicon";

export default function Wordmark() {
  const index = useContext(DailyIconContext);
  return <a className="wordmark" href="/" aria-label="Caper home">
    <img src="/caper-wordmark-letters.svg" alt="" width="1042" height="276" />
    <img className="wordmark-character" src={avatarUrl(index) ?? "/caper-face.svg?v=3"} alt="" width="256" height="256" />
  </a>;
}
