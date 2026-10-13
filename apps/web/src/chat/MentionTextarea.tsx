import {
  useEffect,
  useId,
  useImperativeHandle,
  useLayoutEffect,
  useRef,
  useState,
  type Ref,
  type TextareaHTMLAttributes,
  type KeyboardEvent,
  type FocusEvent,
  type PointerEvent,
} from "react";
import {
  autoUpdate,
  flip,
  FloatingPortal,
  offset,
  shift,
  useDismiss,
  useFloating,
  useInteractions,
  useMergeRefs,
} from "@floating-ui/react";
import { emojiSegments } from "./emoji.ts";
import { emojiName } from "./reactors.ts";
import { draftMentionSegments, type MentionCandidate } from "./mentions.ts";

/** A background-only mirror keeps native editing and supplies emoji hit targets. */
export default function MentionTextarea({
  ref,
  people,
  specialMentions = false,
  ...props
}: TextareaHTMLAttributes<HTMLTextAreaElement> & {
  ref?: Ref<HTMLTextAreaElement>;
  people?: MentionCandidate[];
  specialMentions?: boolean;
}) {
  const input = useRef<HTMLTextAreaElement>(null);
  const mirror = useRef<HTMLDivElement>(null);
  useImperativeHandle(ref, () => input.current!, []);
  const text = String(props.value ?? "");
  const segments = draftMentionSegments(text, people, specialMentions);
  const [name, setName] = useState<string>();
  const tooltipId = useId();
  const timer = useRef<ReturnType<typeof setTimeout>>(undefined);
  const target = useRef<HTMLElement | undefined>(undefined);
  const press = useRef<{ x: number; y: number } | undefined>(undefined);
  const generation = useRef(0);
  const close = () => {
    clearTimeout(timer.current);
    target.current = undefined;
    generation.current++;
    setName(undefined);
  };
  const { refs, floatingStyles, context } = useFloating({
    open: !!name,
    onOpenChange: (open) => {
      if (!open) close();
    },
    placement: "top",
    strategy: "fixed",
    middleware: [offset(6), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps } = useInteractions([useDismiss(context, { ancestorScroll: true })]);
  const fieldRef = useMergeRefs([input, refs.setReference]);
  useEffect(() => {
    close();
    return () => {
      clearTimeout(timer.current);
      generation.current++;
    };
  }, [text]);
  const atPoint = (x: number, y: number) => {
    if (!input.current || props.disabled) return;
    const field = input.current.getBoundingClientRect();
    if (x < field.left || x > field.right || y < field.top || y > field.bottom) return;
    return Array.from(mirror.current?.querySelectorAll<HTMLElement>("[data-emoji]") ?? []).find((span) =>
      Array.from(span.getClientRects()).some(
        (rect) => x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom,
      ),
    );
  };
  const inspect = (span: HTMLElement | undefined, delay: number) => {
    if (span === target.current) return;
    close();
    if (!span) return;
    target.current = span;
    const request = generation.current;
    timer.current = setTimeout(() => {
      void emojiName(span.textContent ?? "").then((label) => {
        if (generation.current !== request) return;
        refs.setPositionReference({
          getBoundingClientRect: () => span.getBoundingClientRect(),
          contextElement: input.current!,
        });
        setName(label);
      });
    }, delay);
  };
  const renderText = (value: string) =>
    emojiSegments(value).map((part, index) =>
      part.emoji ? (
        <span key={index} data-emoji>
          {part.text}
        </span>
      ) : (
        part.text
      ),
    );
  const sync = () => {
    const field = input.current;
    const backdrop = mirror.current;
    if (!field || !backdrop) return;
    const style = getComputedStyle(field);
    for (const property of [
      "font-family",
      "font-size",
      "font-weight",
      "font-style",
      "line-height",
      "letter-spacing",
      "text-align",
      "text-indent",
      "text-transform",
      "padding-top",
      "padding-right",
      "padding-bottom",
      "padding-left",
      "border-top-width",
      "border-right-width",
      "border-bottom-width",
      "border-left-width",
      "tab-size",
    ])
      backdrop.style.setProperty(property, style.getPropertyValue(property));
    backdrop.style.width = `${field.clientWidth + parseFloat(style.borderLeftWidth) + parseFloat(style.borderRightWidth)}px`;
    backdrop.scrollTop = field.scrollTop;
    backdrop.scrollLeft = field.scrollLeft;
  };
  useLayoutEffect(sync);
  useLayoutEffect(() => {
    const field = input.current;
    if (!field) return;
    const observer = new ResizeObserver(sync);
    observer.observe(field);
    // Loaded fonts can change wrapping without changing the field's dimensions.
    void document.fonts?.ready.then(sync);
    return () => observer.disconnect();
  }, []);
  return (
    <div className="chat-mention-input" data-disabled={props.disabled || undefined}>
      <div ref={mirror} className="chat-mention-input-mirror" aria-hidden="true">
        {segments.map((segment, index) => (
          <span key={index}>
            {segment.mention ? <mark>{renderText(segment.text)}</mark> : renderText(segment.text)}
          </span>
        ))}
        {"\n"}
      </div>
      <textarea
        {...props}
        ref={fieldRef}
        aria-describedby={
          name ? [props["aria-describedby"], tooltipId].filter(Boolean).join(" ") : props["aria-describedby"]
        }
        {...getReferenceProps({
          onKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => {
            close();
            props.onKeyDown?.(event);
          },
          onBlur: (event: FocusEvent<HTMLTextAreaElement>) => {
            close();
            props.onBlur?.(event);
          },
          onPointerDown: (event: PointerEvent<HTMLTextAreaElement>) => {
            close();
            press.current = undefined;
            if (event.pointerType !== "mouse" && event.isPrimary) {
              press.current = { x: event.clientX, y: event.clientY };
              inspect(atPoint(event.clientX, event.clientY), 500);
            }
            props.onPointerDown?.(event);
          },
          onPointerMove: (event: PointerEvent<HTMLTextAreaElement>) => {
            if (event.pointerType === "mouse" && event.buttons === 0)
              inspect(atPoint(event.clientX, event.clientY), 400);
            else if (press.current && Math.hypot(event.clientX - press.current.x, event.clientY - press.current.y) > 10)
              close();
            props.onPointerMove?.(event);
          },
          onPointerUp: (event: PointerEvent<HTMLTextAreaElement>) => {
            if (press.current) {
              clearTimeout(timer.current);
              // Keep a completed hold readable briefly after the finger lifts.
              if (name) timer.current = setTimeout(close, 1500);
              else close();
              press.current = undefined;
            }
            props.onPointerUp?.(event);
          },
          onPointerCancel: (event: PointerEvent<HTMLTextAreaElement>) => {
            close();
            press.current = undefined;
            props.onPointerCancel?.(event);
          },
          onPointerLeave: (event: PointerEvent<HTMLTextAreaElement>) => {
            if (event.pointerType === "mouse") close();
            props.onPointerLeave?.(event);
          },
        })}
        onScroll={(event) => {
          close();
          sync();
          props.onScroll?.(event);
        }}
      />
      {name && (
        <FloatingPortal root={input.current?.closest<HTMLElement>(".live-scene") ?? undefined}>
          <div
            ref={refs.setFloating}
            style={floatingStyles}
            className="chat-emoji-tooltip"
            role="tooltip"
            id={tooltipId}
          >
            :{name}:
          </div>
        </FloatingPortal>
      )}
    </div>
  );
}
