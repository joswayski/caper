import { useEffect } from "react";
import { avatarUrl } from "../account/avatar";
import { dailyIcon, type DailyIcon } from "./daily-icon";

/** Browser tabs only. Installed shortcuts use the original mascot in the manifest. */
export default function RotatingFavicon() {
  useEffect(() => {
    if (window.matchMedia("(display-mode: standalone)").matches || (navigator as Navigator & { standalone?: boolean }).standalone) return;
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
      const url = avatarUrl(saved.index)!;
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
  return null;
}
