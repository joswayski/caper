import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

const DEFAULT_WIDTH = 280;
const MIN_WIDTH = 220;
const MAX_WIDTH = 440;
const STORAGE_KEY = "caper:channel-sidebar-width";

// Runs in the document head so server-rendered loading and hydrated content
// use the same saved width, without waiting for React to mount.
export const sidebarWidthScript = `try{const w=Number(localStorage.getItem('${STORAGE_KEY}'));if(w>=${MIN_WIDTH}&&w<=${MAX_WIDTH})document.documentElement.style.setProperty('--channel-sidebar-width',w+'px')}catch{}`;

export default function ChannelSidebar({ children }: { children: ReactNode }) {
  const panel = useRef<HTMLElement>(null);
  const drag = useRef<{ x: number; width: number } | undefined>(undefined);
  const [width, setWidth] = useState<number>();
  const [maximum, setMaximum] = useState(MAX_WIDTH);

  useLayoutEffect(() => {
    try {
      const saved = Number(localStorage.getItem(STORAGE_KEY));
      if (saved >= MIN_WIDTH && saved <= MAX_WIDTH) setWidth(saved);
    } catch {
      /* Storage may be unavailable in private browsing. */
    }
    const room = panel.current!.parentElement!;
    const measure = () => {
      if (window.innerWidth <= 760) return;
      const rail =
        room.querySelector(".space-rail")?.getBoundingClientRect().width ?? 0;
      const max = Math.max(
        MIN_WIDTH,
        Math.min(MAX_WIDTH, room.clientWidth - rail - 320),
      );
      setMaximum(max);
      setWidth((current) => Math.min(current ?? DEFAULT_WIDTH, max));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(room);
    return () => observer.disconnect();
  }, []);

  const resize = (value: number) => {
    const next = Math.round(Math.max(MIN_WIDTH, Math.min(maximum, value)));
    setWidth(next);
    document.documentElement.style.setProperty("--channel-sidebar-width", `${next}px`);
    try {
      localStorage.setItem(STORAGE_KEY, String(next));
    } catch {
      /* Optional preference. */
    }
  };

  return (
    <aside ref={panel} className="people-panel" style={{ width: width ?? `var(--channel-sidebar-width, ${DEFAULT_WIDTH}px)` }}>
      {children}
      <div
        className="channel-sidebar-resize"
        role="separator"
        aria-label="Channel sidebar width"
        aria-orientation="vertical"
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={maximum}
        aria-valuenow={width ?? DEFAULT_WIDTH}
        aria-valuetext={`${width ?? DEFAULT_WIDTH} pixels`}
        tabIndex={0}
        title="Drag to resize. Arrow keys to adjust. Double-click to reset."
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          event.preventDefault();
          drag.current = {
            x: event.clientX,
            width: panel.current!.getBoundingClientRect().width,
          };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          if (drag.current)
            resize(drag.current.width + event.clientX - drag.current.x);
        }}
        onPointerUp={(event) => {
          drag.current = undefined;
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onLostPointerCapture={() => {
          drag.current = undefined;
        }}
        onDoubleClick={() => resize(DEFAULT_WIDTH)}
        onKeyDown={(event) => {
          const sizes: Record<string, number> = {
            ArrowLeft: (width ?? DEFAULT_WIDTH) - 10,
            ArrowRight: (width ?? DEFAULT_WIDTH) + 10,
            Home: MIN_WIDTH,
            End: maximum,
          };
          const next = sizes[event.key];
          if (next === undefined) return;
          event.preventDefault();
          resize(next);
        }}
      />
    </aside>
  );
}
