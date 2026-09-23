import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

const DEFAULT_WIDTH = 280;
const MIN_WIDTH = 220;
const MAX_WIDTH = 440;
const STORAGE_KEY = "caper:channel-sidebar-width";

export default function ChannelSidebar({ children }: { children: ReactNode }) {
  const panel = useRef<HTMLElement>(null);
  const drag = useRef<{ x: number; width: number } | undefined>(undefined);
  const [width, setWidth] = useState(DEFAULT_WIDTH);
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
      setWidth((current) => Math.min(current, max));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(room);
    return () => observer.disconnect();
  }, []);

  const resize = (value: number) => {
    const next = Math.round(Math.max(MIN_WIDTH, Math.min(maximum, value)));
    setWidth(next);
    try {
      localStorage.setItem(STORAGE_KEY, String(next));
    } catch {
      /* Optional preference. */
    }
  };

  return (
    <aside ref={panel} className="people-panel" style={{ width }}>
      {children}
      <div
        className="channel-sidebar-resize"
        role="separator"
        aria-label="Channel sidebar width"
        aria-orientation="vertical"
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={maximum}
        aria-valuenow={width}
        aria-valuetext={`${width} pixels`}
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
            ArrowLeft: width - 10,
            ArrowRight: width + 10,
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
