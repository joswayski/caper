import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

const STORAGE_KEY = "caper:sidebar-channels-share";
const DEFAULT_SHARE = 0.5;

/** Separate scrollports keep long channel lists from pushing DMs out of reach. */
export default function SidebarSections({
  children,
  directs,
  resizable,
}: {
  children: ReactNode;
  directs: ReactNode;
  resizable: boolean;
}) {
  const panel = useRef<HTMLDivElement>(null);
  const handle = useRef<HTMLDivElement>(null);
  const drag = useRef<{ y: number; share: number; last: number; cursor: string; userSelect: string } | undefined>(
    undefined,
  );
  const [preference, setPreference] = useState(DEFAULT_SHARE);
  const [available, setAvailable] = useState(0);
  const minimum = available > 0 ? Math.min(96 / available, 1 / 3) : 1 / 3;
  const clamp = (value: number) => Math.max(minimum, Math.min(1 - minimum, value));
  const share = clamp(preference);

  useLayoutEffect(() => {
    if (!resizable) return;
    try {
      const saved = Number(localStorage.getItem(STORAGE_KEY));
      if (saved > 0 && saved < 1) setPreference(saved);
    } catch {
      /* Optional preference. */
    }
    const measure = () => setAvailable(Math.max(0, panel.current!.clientHeight - handle.current!.offsetHeight));
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(panel.current!);
    observer.observe(handle.current!);
    return () => {
      observer.disconnect();
      if (drag.current) {
        document.documentElement.style.cursor = drag.current.cursor;
        document.documentElement.style.userSelect = drag.current.userSelect;
        drag.current = undefined;
      }
    };
  }, [resizable]);

  const resize = (value: number, persist = true) => {
    const next = clamp(value);
    setPreference(next);
    if (persist) {
      try {
        localStorage.setItem(STORAGE_KEY, String(next));
      } catch {
        /* Optional preference. */
      }
    }
    return next;
  };
  const endDrag = () => {
    if (!drag.current) return;
    resize(drag.current.last);
    document.documentElement.style.cursor = drag.current.cursor;
    document.documentElement.style.userSelect = drag.current.userSelect;
    drag.current = undefined;
  };

  if (!resizable)
    return (
      <>
        {children}
        {directs}
      </>
    );

  return (
    <div
      ref={panel}
      className="sidebar-sections"
      style={{ gridTemplateRows: `minmax(0, ${share}fr) auto minmax(0, ${1 - share}fr)` }}
      onScrollCapture={(event) => {
        if (!(event.target instanceof HTMLElement) || !event.target.classList.contains("sidebar-section-scroll"))
          return;
        event.target.querySelectorAll<HTMLDetailsElement>("details[open]").forEach((menu) => {
          menu.open = false;
        });
      }}
    >
      <div id="sidebar-channels" className="sidebar-section-scroll" role="region" aria-label="Channels" tabIndex={0}>
        {children}
      </div>
      <div
        ref={handle}
        className="sidebar-sections-resize"
        role="separator"
        aria-label="Channels and direct messages"
        aria-controls="sidebar-channels sidebar-directs"
        aria-orientation="horizontal"
        aria-valuemin={Math.round(minimum * 100)}
        aria-valuemax={Math.round((1 - minimum) * 100)}
        aria-valuenow={Math.round(share * 100)}
        aria-valuetext={`${Math.round(share * 100)}% channels, ${Math.round((1 - share) * 100)}% direct messages`}
        tabIndex={0}
        title="Drag to resize. Arrow keys to adjust. Double-click to reset."
        onPointerDown={(event) => {
          if (event.button !== 0 || !available) return;
          event.preventDefault();
          drag.current = {
            y: event.clientY,
            share,
            last: share,
            cursor: document.documentElement.style.cursor,
            userSelect: document.documentElement.style.userSelect,
          };
          document.documentElement.style.cursor = "ns-resize";
          document.documentElement.style.userSelect = "none";
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          if (drag.current && available)
            drag.current.last = resize(drag.current.share + (event.clientY - drag.current.y) / available, false);
        }}
        onPointerUp={(event) => {
          endDrag();
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={endDrag}
        onLostPointerCapture={endDrag}
        onDoubleClick={() => resize(DEFAULT_SHARE)}
        onKeyDown={(event) => {
          const step = available > 0 ? 10 / available : 0.02;
          const sizes: Record<string, number> = {
            ArrowUp: share - step,
            ArrowDown: share + step,
            Home: minimum,
            End: 1 - minimum,
          };
          const next = sizes[event.key];
          if (next === undefined) return;
          event.preventDefault();
          resize(next);
        }}
      />
      <div
        id="sidebar-directs"
        className="sidebar-section-scroll"
        role="region"
        aria-label="Conversations"
        tabIndex={0}
      >
        {directs}
      </div>
    </div>
  );
}
