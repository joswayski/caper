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
import { linkSegments } from "./links.ts";
import { toggleFormat } from "./formatting.ts";
import { Bold, Italic } from "lucide-react";

/** An aligned text mirror styles URLs/mentions while the textarea keeps native editing. */
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
  const [selection, setSelection] = useState<{ start: number; end: number }>();
  const composing = useRef(false);
  const formatting = useFloating({
    open: !!selection && !props.disabled && !props.readOnly,
    placement: "top",
    strategy: "fixed",
    middleware: [offset(8), flip(), shift({ padding: 12 })],
    whileElementsMounted: autoUpdate,
  });
  const trackSelection = () => {
    const field = input.current;
    if (
      !field ||
      field.disabled ||
      field.readOnly ||
      composing.current ||
      field.selectionStart === field.selectionEnd
    ) {
      setSelection(undefined);
      return;
    }
    setSelection({ start: field.selectionStart, end: field.selectionEnd });
    formatting.refs.setPositionReference({
      contextElement: field,
      getBoundingClientRect: () => {
        // The existing mirror has the textarea's exact font, wrapping and scroll.
        const range = document.createRange();
        const walker = document.createTreeWalker(mirror.current!, NodeFilter.SHOW_TEXT);
        let offset = 0;
        let started = false;
        while (walker.nextNode()) {
          const node = walker.currentNode;
          const end = offset + (node.textContent?.length ?? 0);
          if (!started && field.selectionStart <= end) {
            range.setStart(node, field.selectionStart - offset);
            started = true;
          }
          if (field.selectionEnd <= end) {
            range.setEnd(node, field.selectionEnd - offset);
            const rect = range.getBoundingClientRect();
            const bounds = field.getBoundingClientRect();
            return new DOMRect(
              Math.max(bounds.left, Math.min(rect.left, bounds.right)),
              Math.max(bounds.top, Math.min(rect.top, bounds.bottom)),
              Math.min(rect.width, bounds.width),
              Math.min(rect.height, bounds.height),
            );
          }
          offset = end;
        }
        return field.getBoundingClientRect();
      },
    });
  };
  const format = (marker: "**" | "_") => {
    const field = input.current;
    if (!field || field.disabled || field.readOnly || composing.current) return;
    const result = toggleFormat(field.value, field.selectionStart, field.selectionEnd, marker);
    if (result.text === field.value || Array.from(result.text).length > 4_000) return;
    // Replace only the changed range. insertText keeps this one native undo step;
    // setRangeText is the fallback for browsers without that editing command.
    let start = 0;
    while (start < field.value.length && field.value[start] === result.text[start]) start++;
    let oldEnd = field.value.length;
    let newEnd = result.text.length;
    while (oldEnd > start && newEnd > start && field.value[oldEnd - 1] === result.text[newEnd - 1]) {
      oldEnd--;
      newEnd--;
    }
    field.focus();
    field.setSelectionRange(start, oldEnd);
    const inserted = result.text.slice(start, newEnd);
    if (!document.execCommand("insertText", false, inserted)) {
      field.setRangeText(inserted, start, oldEnd, "end");
      field.dispatchEvent(new Event("input", { bubbles: true }));
    }
    requestAnimationFrame(() => {
      field.setSelectionRange(result.start, result.end);
      trackSelection();
    });
  };
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
            {segment.mention ? (
              <mark>{renderText(segment.text)}</mark>
            ) : (
              linkSegments(segment.text).map((part, index) =>
                part.href ? (
                  <mark key={index} className="chat-draft-link">
                    {renderText(part.text)}
                  </mark>
                ) : (
                  <span key={index}>{renderText(part.text)}</span>
                ),
              )
            )}
          </span>
        ))}
        {"\n"}
      </div>
      <textarea
        {...props}
        ref={fieldRef}
        aria-keyshortcuts="Control+B Meta+B Control+I Meta+I"
        aria-description={
          props["aria-description"] ??
          "Select text to format it. Control or Command B for bold, I for italic. Markdown markers stay visible while editing."
        }
        aria-describedby={
          name ? [props["aria-describedby"], tooltipId].filter(Boolean).join(" ") : props["aria-describedby"]
        }
        {...getReferenceProps({
          onKeyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => {
            close();
            if (!event.nativeEvent.isComposing && !event.altKey && (event.metaKey || event.ctrlKey)) {
              const key = event.key.toLowerCase();
              if (key === "b" || key === "i") {
                event.preventDefault();
                format(key === "b" ? "**" : "_");
                return;
              }
            }
            if (selection && event.key === "Escape") {
              event.preventDefault();
              setSelection(undefined);
              return;
            }
            props.onKeyDown?.(event);
          },
          onBlur: (event: FocusEvent<HTMLTextAreaElement>) => {
            close();
            if (!formatting.refs.floating.current?.contains(event.relatedTarget)) setSelection(undefined);
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
        onSelect={(event) => {
          if (document.activeElement === input.current) trackSelection();
          props.onSelect?.(event);
        }}
        onCompositionStart={(event) => {
          composing.current = true;
          setSelection(undefined);
          props.onCompositionStart?.(event);
        }}
        onCompositionEnd={(event) => {
          composing.current = false;
          props.onCompositionEnd?.(event);
        }}
        onScroll={(event) => {
          close();
          sync();
          formatting.update();
          props.onScroll?.(event);
        }}
      />
      {selection && !props.disabled && !props.readOnly && (
        <FloatingPortal root={input.current?.closest<HTMLElement>("dialog, [role='dialog'], .live-scene") ?? undefined}>
          <div
            ref={formatting.refs.setFloating}
            style={formatting.floatingStyles}
            className="chat-format-toolbar"
            role="group"
            aria-label="Message formatting"
            onPointerDown={(event) => event.preventDefault()}
            onBlur={(event) => {
              if (!event.currentTarget.contains(event.relatedTarget) && event.relatedTarget !== input.current)
                setSelection(undefined);
            }}
            onKeyDown={(event) => {
              if (event.key === "Escape") {
                event.preventDefault();
                input.current?.focus();
                setSelection(undefined);
              }
            }}
          >
            {(
              [
                ["**", "Bold", Bold],
                ["_", "Italic", Italic],
              ] as const
            ).map(([marker, label, Icon]) => {
              const result = toggleFormat(text, selection.start, selection.end, marker);
              return (
                <button
                  key={marker}
                  type="button"
                  aria-label={label}
                  title={`${label} (${navigator.platform.includes("Mac") ? "⌘" : "Ctrl+"}${label[0]})`}
                  disabled={result.text === text || Array.from(result.text).length > 4_000}
                  onClick={() => format(marker)}
                >
                  <Icon size={17} aria-hidden="true" />
                </button>
              );
            })}
          </div>
        </FloatingPortal>
      )}
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
