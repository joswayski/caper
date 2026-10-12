import { useImperativeHandle, useLayoutEffect, useRef, type Ref, type TextareaHTMLAttributes } from "react";
import { draftMentionSegments, type MentionCandidate } from "./mentions.ts";

/** A background-only mirror keeps the native textarea's editing, caret and IME behavior. */
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
        {segments.map((segment, index) => (segment.mention ? <mark key={index}>{segment.text}</mark> : segment.text))}
        {"\n"}
      </div>
      <textarea
        {...props}
        ref={input}
        onScroll={(event) => {
          sync();
          props.onScroll?.(event);
        }}
      />
    </div>
  );
}
