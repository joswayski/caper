import { useEffect, useRef, useState, type RefObject } from "react";
import { MoveHorizontal, MoveVertical } from "lucide-react";

const STORAGE_KEY = "caper:room-size";

/** Outer sizing is independent of the sidebar and conversation dividers. */
export default function RoomResize({ room }: { room: RefObject<HTMLElement | null> }) {
  const drag = useRef<{ x: number; y: number; width: number; height: number } | undefined>(undefined);
  const [size, setSize] = useState<{ width?: number; height?: number }>({});
  const [bounds, setBounds] = useState({ width: 1400, height: 800 });

  useEffect(() => {
    const restore = () => {
      try {
        const saved = JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}");
        setSize({
          width: Number.isFinite(saved?.width) ? saved.width : undefined,
          height: Number.isFinite(saved?.height) ? saved.height : undefined,
        });
      } catch {
        /* Optional preference. */
      }
    };
    const storageChanged = (event: StorageEvent) => {
      if (event.storageArea === localStorage && (event.key === STORAGE_KEY || event.key === null)) restore();
    };
    restore();
    window.addEventListener("storage", storageChanged);
    const measure = () =>
      setBounds({ width: document.documentElement.clientWidth - 24, height: window.innerHeight - 104 });
    measure();
    window.addEventListener("resize", measure);
    const observer = new ResizeObserver(measure);
    observer.observe(document.documentElement);
    return () => {
      window.removeEventListener("storage", storageChanged);
      window.removeEventListener("resize", measure);
      observer.disconnect();
    };
  }, []);

  useEffect(() => {
    const element = room.current!;
    for (const axis of ["width", "height"] as const) {
      if (size[axis] === undefined) element.style.removeProperty(`--room-${axis}`);
      else element.style.setProperty(`--room-${axis}`, `${size[axis]}px`);
    }
  }, [room, size]);

  const save = () => {
    try {
      localStorage.setItem(STORAGE_KEY, JSON.stringify(size));
    } catch {
      /* Optional preference. */
    }
    drag.current = undefined;
  };

  return ["left", "right", "bottom"].map((edge) => {
    const axis = edge === "bottom" ? "height" : "width";
    const minimum = Math.min(axis === "width" ? 760 : 420, bounds[axis]);
    const value = Math.max(
      minimum,
      Math.min(bounds[axis], size[axis] ?? (axis === "width" ? Math.min(1400, bounds.width - 32) : bounds.height)),
    );
    const resize = (next: number) =>
      setSize((current) => ({
        ...current,
        [axis]: Math.round(Math.max(minimum, Math.min(bounds[axis], next))),
      }));
    return (
      <div
        key={edge}
        className={`room-resize room-resize-${edge}`}
        role="separator"
        aria-label={`Room ${axis}${axis === "width" ? ` (${edge} edge)` : ""}`}
        aria-orientation={axis === "width" ? "vertical" : "horizontal"}
        aria-valuemin={minimum}
        aria-valuemax={bounds[axis]}
        aria-valuenow={value}
        tabIndex={0}
        title="Drag to resize. Arrow keys to adjust. Double-click to reset."
        onPointerDown={(event) => {
          if (event.button !== 0) return;
          event.preventDefault();
          const rect = room.current!.getBoundingClientRect();
          drag.current = { x: event.clientX, y: event.clientY, width: rect.width, height: rect.height };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          const start = drag.current;
          if (!start) return;
          // Auto margins move both sides: double the pointer delta so the grabbed edge follows it.
          resize(
            start[axis] +
              (axis === "height"
                ? event.clientY - start.y
                : 2 * (event.clientX - start.x) * (edge === "left" ? -1 : 1)),
          );
        }}
        onPointerUp={(event) => {
          save();
          event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onLostPointerCapture={save}
        onBlur={save}
        onDoubleClick={() => {
          setSize({});
          try {
            localStorage.removeItem(STORAGE_KEY);
          } catch {
            /* Optional preference. */
          }
        }}
        onKeyDown={(event) => {
          const negative = axis === "width" ? "ArrowLeft" : "ArrowUp";
          const positive = axis === "width" ? "ArrowRight" : "ArrowDown";
          if (![negative, positive, "Home", "End"].includes(event.key)) return;
          event.preventDefault();
          resize(
            event.key === "Home"
              ? minimum
              : event.key === "End"
                ? bounds[axis]
                : value + (event.key === positive ? 10 : -10) * (edge === "left" ? -1 : 1),
          );
        }}
        onKeyUp={save}
      >
        <span className="room-resize-icon" aria-hidden="true">
          {axis === "width" ? <MoveHorizontal /> : <MoveVertical />}
        </span>
      </div>
    );
  });
}
