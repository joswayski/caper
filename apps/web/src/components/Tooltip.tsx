import {
  cloneElement,
  useEffect,
  useId,
  useRef,
  type FocusEvent,
  type KeyboardEvent,
  type MouseEvent,
  type ReactElement,
  type ReactNode,
} from "react";
import "./tooltip.css";

/** Hover intent before the first tooltip; moving between triggers then shows the next at once. */
const OPEN_DELAY = 350;
const WARM_WINDOW = 400;
/** One tooltip at a time: showing one hides the previous immediately. */
let visible: { hide: () => void } | undefined;
let lastHidden = 0;

type TriggerProps = {
  "aria-describedby"?: string;
  onClick?: (event: MouseEvent<HTMLElement>) => void;
  onBlur?: (event: FocusEvent<HTMLElement>) => void;
  onFocus?: (event: FocusEvent<HTMLElement>) => void;
  onKeyDown?: (event: KeyboardEvent<HTMLElement>) => void;
  onMouseEnter?: (event: MouseEvent<HTMLElement>) => void;
  onMouseLeave?: (event: MouseEvent<HTMLElement>) => void;
};

export default function Tooltip({
  children,
  content,
  id,
  placement = "top",
  touch = false,
}: {
  children: ReactElement<TriggerProps>;
  content?: ReactNode;
  id?: string;
  placement?: "top" | "bottom";
  touch?: boolean;
}) {
  const generatedId = useId();
  const tooltipId = id ?? `tooltip-${generatedId.replace(/:/g, "")}`;
  const tooltipRef = useRef<HTMLSpanElement>(null);
  const anchorRef = useRef<HTMLElement | null>(null);
  const focusedRef = useRef(false);
  const hoveredRef = useRef(false);
  const touchPointerRef = useRef(false);
  const hideTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const showTimer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const self = useRef<{ hide: () => void }>({ hide: () => undefined });

  const hide = () => {
    clearTimeout(hideTimer.current);
    clearTimeout(showTimer.current);
    const tooltip = tooltipRef.current;
    if (tooltip?.matches(":popover-open")) {
      tooltip.hidePopover();
      lastHidden = performance.now();
    }
    if (visible === self.current) visible = undefined;
  };
  self.current.hide = hide;
  const position = () => {
    const tooltip = tooltipRef.current;
    const anchor = anchorRef.current;
    if (!tooltip || !anchor) return;
    const gap = 8;
    const edge = 8;
    const viewport = window.visualViewport;
    const x = viewport?.offsetLeft ?? 0;
    const y = viewport?.offsetTop ?? 0;
    const width = viewport?.width ?? window.innerWidth;
    const height = viewport?.height ?? window.innerHeight;
    tooltip.style.maxWidth = `${Math.min(280, width - edge * 2)}px`;
    tooltip.style.maxHeight = `${height - edge * 2}px`;
    const anchorBounds = anchor.getBoundingClientRect();
    const tooltipBounds = tooltip.getBoundingClientRect();
    const left = Math.max(
      x + edge,
      Math.min(
        anchorBounds.left + (anchorBounds.width - tooltipBounds.width) / 2,
        x + width - tooltipBounds.width - edge,
      ),
    );
    const fitsAbove = anchorBounds.top - y >= tooltipBounds.height + gap + edge;
    const top =
      placement === "bottom" || !fitsAbove
        ? Math.min(anchorBounds.bottom + gap, y + height - tooltipBounds.height - edge)
        : anchorBounds.top - tooltipBounds.height - gap;
    tooltip.style.left = `${left}px`;
    tooltip.style.top = `${Math.max(y + edge, top)}px`;
  };
  const show = (anchor: HTMLElement) => {
    clearTimeout(hideTimer.current);
    clearTimeout(showTimer.current);
    if (content == null) return;
    anchorRef.current = anchor;
    const tooltip = tooltipRef.current;
    if (!tooltip) return;
    if (visible && visible !== self.current) visible.hide();
    visible = self.current;
    if (!tooltip.matches(":popover-open")) tooltip.showPopover();
    position();
  };
  /** Pointer hover waits briefly, unless a tooltip was just showing (warm). */
  const showAfterHover = (anchor: HTMLElement) => {
    clearTimeout(hideTimer.current);
    clearTimeout(showTimer.current);
    const warm = visible !== undefined || performance.now() - lastHidden < WARM_WINDOW;
    if (warm) return show(anchor);
    showTimer.current = setTimeout(() => {
      if (hoveredRef.current) show(anchor);
    }, OPEN_DELAY);
  };

  useEffect(() => {
    const onResize = () => tooltipRef.current?.matches(":popover-open") && position();
    const onScroll = () => hide();
    const onPointerDown = (event: PointerEvent) => {
      touchPointerRef.current = event.pointerType === "touch";
      if (!anchorRef.current?.contains(event.target as Node) && !tooltipRef.current?.contains(event.target as Node))
        hide();
    };
    const onKeyDown = (event: globalThis.KeyboardEvent) => {
      touchPointerRef.current = false;
      if (event.key === "Escape" && tooltipRef.current?.matches(":popover-open")) {
        event.preventDefault();
        event.stopPropagation();
        hide();
      }
    };
    window.addEventListener("resize", onResize);
    window.visualViewport?.addEventListener("resize", onResize);
    window.visualViewport?.addEventListener("scroll", onScroll);
    document.addEventListener("pointerdown", onPointerDown, true);
    window.addEventListener("scroll", onScroll, true);
    document.addEventListener("keydown", onKeyDown);
    return () => {
      clearTimeout(hideTimer.current);
      clearTimeout(showTimer.current);
      if (visible === self.current) visible = undefined;
      window.removeEventListener("resize", onResize);
      window.visualViewport?.removeEventListener("resize", onResize);
      window.visualViewport?.removeEventListener("scroll", onScroll);
      document.removeEventListener("pointerdown", onPointerDown, true);
      window.removeEventListener("scroll", onScroll, true);
      document.removeEventListener("keydown", onKeyDown);
    };
  }, []);
  useEffect(() => hide, [content]);

  const trigger = cloneElement(children, {
    "aria-describedby": content == null ? children.props["aria-describedby"] : tooltipId,
    onClick: (event) => {
      children.props.onClick?.(event);
      if (!touch) {
        hide();
        return;
      }
      if (touchPointerRef.current && tooltipRef.current?.matches(":popover-open")) hide();
      else show(event.currentTarget);
    },
    onMouseEnter: (event) => {
      children.props.onMouseEnter?.(event);
      if (matchMedia("(hover: none)").matches) return;
      hoveredRef.current = true;
      showAfterHover(event.currentTarget);
    },
    onMouseLeave: (event) => {
      children.props.onMouseLeave?.(event);
      hoveredRef.current = false;
      clearTimeout(showTimer.current);
      if (!focusedRef.current)
        hideTimer.current = setTimeout(() => {
          if (!hoveredRef.current && !focusedRef.current) hide();
        }, 100);
    },
    onFocus: (event) => {
      children.props.onFocus?.(event);
      // Keyboard focus only: focus returned to a trigger after a mouse action
      // (closing a menu or dialog) must not pop a tooltip under the pointer.
      if (touchPointerRef.current || !event.currentTarget.matches(":focus-visible")) return;
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

  return (
    <>
      {trigger}
      {content != null && (
        <span
          ref={tooltipRef}
          className="top-layer-tooltip"
          id={tooltipId}
          role="tooltip"
          popover="manual"
          onMouseEnter={() => {
            clearTimeout(hideTimer.current);
            hoveredRef.current = true;
          }}
          onMouseLeave={() => {
            hoveredRef.current = false;
            if (!focusedRef.current) hide();
          }}
        >
          {content}
        </span>
      )}
    </>
  );
}
