import { createContext, useEffect, useState, type ReactNode } from "react";
import { dailyIcon, dailyIconUrl, type DailyIcon } from "./daily-icon";

export const DailyIconContext = createContext<number | null>(null);

/** One daily choice for in-app branding and browser tabs. Installed icons stay fixed. */
export default function RotatingFavicon({ children }: { children: ReactNode }) {
  const [index, setIndex] = useState<number | null>(null);
  useEffect(() => {
    const standalone = window.matchMedia("(display-mode: standalone)").matches || (navigator as Navigator & { standalone?: boolean }).standalone;
    let alive = true;
    let saved: DailyIcon | null = null;
    let renderedIndex: number | undefined;

    const refresh = () => {
      try {
        const stored = localStorage.getItem("caper.daily-icon.v1");
        if (stored) saved = JSON.parse(stored);
      } catch { /* Storage may be unavailable; retain this tab's choice. */ }
      saved = dailyIcon(saved, Date.now());
      try { localStorage.setItem("caper.daily-icon.v1", JSON.stringify(saved)); } catch { /* Rotation still works in this tab. */ }
      setIndex(saved.index);
      if (standalone) return;
      const url = dailyIconUrl(saved.index);
      const svg = document.querySelector<HTMLLinkElement>("#caper-favicon-svg");
      if (!svg || (svg.getAttribute("href") === url && renderedIndex === saved.index)) return;
      svg.href = url;
      const index = saved.index;
      const image = new Image();
      image.onload = () => {
        if (!alive || saved?.index !== index) return;
        for (const size of [32, 192]) {
          const canvas = document.createElement("canvas"); canvas.width = canvas.height = size;
          canvas.getContext("2d")!.drawImage(image, 0, 0, size, size);
          const link = document.querySelector<HTMLLinkElement>(`#caper-favicon-${size}`);
          if (link) link.href = canvas.toDataURL("image/png");
        }
        renderedIndex = index;
      };
      image.src = url;
    };
    refresh();
    const timer = window.setInterval(refresh, 60_000);
    window.addEventListener("focus", refresh);
    window.addEventListener("storage", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      alive = false;
      window.clearInterval(timer);
      window.removeEventListener("focus", refresh);
      window.removeEventListener("storage", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, []);
  return <DailyIconContext.Provider value={index}>{children}</DailyIconContext.Provider>;
}
