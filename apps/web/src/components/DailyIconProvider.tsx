import { createContext, useEffect, useState, type ReactNode } from "react";
import { dailyIcon, type DailyIcon } from "./daily-icon";

export const DailyIconContext = createContext<number | null>(null);

/** Daily illustrations rotate; browser tabs and installed icons keep the fixed Caper identity. */
export default function DailyIconProvider({ children }: { children: ReactNode }) {
  const [index, setIndex] = useState<number | null>(null);
  useEffect(() => {
    let saved: DailyIcon | null = null;

    const refresh = () => {
      try {
        const stored = localStorage.getItem("caper.daily-icon.v1");
        if (stored) saved = JSON.parse(stored);
      } catch {
        /* Storage may be unavailable; retain this tab's choice. */
      }
      saved = dailyIcon(saved, Date.now());
      try {
        localStorage.setItem("caper.daily-icon.v1", JSON.stringify(saved));
      } catch {
        /* Rotation still works in this tab. */
      }
      setIndex(saved.index);
    };
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    window.addEventListener("focus", refresh);
    window.addEventListener("storage", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      window.removeEventListener("storage", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, []);
  return <DailyIconContext.Provider value={index}>{children}</DailyIconContext.Provider>;
}
