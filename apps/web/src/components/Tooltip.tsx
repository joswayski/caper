import { cloneElement, useEffect, useId, useRef, type FocusEvent, type KeyboardEvent, type MouseEvent, type ReactElement, type ReactNode } from "react";
import "./tooltip.css";

type TriggerProps = {
  "aria-describedby"?: string;
  onBlur?: (event: FocusEvent<HTMLElement>) => void;
  onFocus?: (event: FocusEvent<HTMLElement>) => void;
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  onMouseEnter?: (event: MouseEvent<HTMLElement>) => void;
  onMouseLeave?: (event: MouseEvent<HTMLElement>) => void;
};

export default function Tooltip({ children, content, id, placement = "top" }: {
  children: ReactElement<TriggerProps>;
  content?: ReactNode;
  id?: string;
  placement?: "top" | "bottom";
}) {
  const generatedId = useId();
  const tooltipId = id ?? `tooltip-${generatedId.replace(/:/g, "")}`;
  const tooltipRef = useRef<HTMLSpanElement>(null);
  const anchorRef = useRef<HTMLElement | null>(null);
  const focusedRef = useRef(false);
  const hoveredRef = useRef(false);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  const hide = () => {
    clearTimeout(hideTimer.current);
    const tooltip = tooltipRef.current;
    if (tooltip?.matches(":popover-open")) tooltip.hidePopover();
  };
  const position = () => {
    const tooltip = tooltipRef.current;
    const anchor = anchorRef.current;
    if (!tooltip || !anchor) return;
    const gap = 8;
    const edge = 8;
    const anchorBounds = anchor.getBoundingClientRect();
    const tooltipBounds = tooltip.getBoundingClientRect();
    const left = Math.max(edge, Math.min(anchorBounds.left + (anchorBounds.width - tooltipBounds.width) / 2, window.innerWidth - tooltipBounds.width - edge));
    const fitsAbove = anchorBounds.top >= tooltipBounds.height + gap + edge;
    const top = placement === "bottom" || !fitsAbove
      ? Math.min(anchorBounds.bottom + gap, window.innerHeight - tooltipBounds.height - edge)
      : anchorBounds.top - tooltipBounds.height - gap;
    tooltip.style.left = `${left}px`;
    tooltip.style.top = `${Math.max(edge, top)}px`;
  };
  const show = (anchor: HTMLElement) => {
    clearTimeout(hideTimer.current);
    if (content == null) return;
    anchorRef.current = anchor;
    const tooltip = tooltipRef.current;
    if (!tooltip) return;
    if (!tooltip.matches(":popover-open")) tooltip.showPopover();
    position();
  };

  useEffect(() => {
    const onResize = () => tooltipRef.current?.matches(":popover-open") && position();
    const onScroll = () => hide();
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape" && tooltipRef.current?.matches(":popover-open")) {
        event.preventDefault();
        event.stopPropagation();
        hide();
      }
    };
    window.addEventListener("resize", onResize);
    window.addEventListener("scroll", onScroll, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      clearTimeout(hideTimer.current);
      window.removeEventListener("resize", onResize);
      window.removeEventListener("scroll", onScroll, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, []);
  useEffect(() => hide, [content]);

  const trigger = cloneElement(children, {
    "aria-describedby": content == null ? children.props["aria-describedby"] : tooltipId,
    onMouseEnter: (event) => {
      children.props.onMouseEnter?.(event);
      hoveredRef.current = true;
      show(event.currentTarget);
    },
    onMouseLeave: (event) => {
      children.props.onMouseLeave?.(event);
      hoveredRef.current = false;
      if (!focusedRef.current) hideTimer.current = setTimeout(() => { if (!hoveredRef.current && !focusedRef.current) hide(); }, 100);
    },
    onFocus: (event) => {
      children.props.onFocus?.(event);
      focusedRef.current = true;
      show(event.currentTarget);
    },
    onBlur: (event) => {
      children.props.onBlur?.(event);
      focusedRef.current = false;
      if (!hoveredRef.current) hide();
    },
    onKeyDown: (event) => {
      children.props.onKeyDown?.(event);
      if (event.key === "Escape" && tooltipRef.current?.matches(":popover-open")) {
        event.preventDefault();
        event.stopPropagation();
        hide();
      }
    },
  });

  return <>{trigger}{content != null && <span ref={tooltipRef} className="top-layer-tooltip" id={tooltipId} role="tooltip" popover="manual" onMouseEnter={() => { clearTimeout(hideTimer.current); hoveredRef.current = true; }} onMouseLeave={() => { hoveredRef.current = false; if (!focusedRef.current) hide(); }}>{content}</span>}</>;
}
