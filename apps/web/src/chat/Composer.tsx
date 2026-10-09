import { useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type Ref } from "react";
import { useComposerSuggestions } from "./ComposerSuggestions.tsx";
import { COUNTER_START, counterTone } from "./counter.ts";
import { readDraft, saveDraft } from "./drafts.ts";
import type { MentionCandidate } from "./mentions.ts";

export interface ComposerHandle {
  /** Puts a rejected message back into the composer to edit. */
  restore(text: string): void;
  /** Sends the draft, or resends an unconfirmed message. */
  submit(): void;
}

interface ComposerProps {
  ref?: Ref<ComposerHandle>;
  channelName: string;
  /** Keeps unsent text for this conversation across channel and DM switches. */
  draftKey?: string;
  direct: boolean;
  disabled: boolean;
  identityReady: boolean;
  sending: boolean;
  sendRejected: boolean;
  /** The pending send, if any. A new one clears the draft it was sent from. */
  pendingSend?: { clientMessageId: string; text: string; threadRootId?: string };
  authorId?: string;
  mentionMembers?: MentionCandidate[];
  onSend: (text: string) => Promise<unknown> | undefined;
  onTyping: (active: boolean) => void;
  /** Wraps each resize, which reports whether the height changed, so the
   * conversation can measure its position first and stay on the newest message. */
  onResize: (resize: () => boolean) => void;
  /** Whether a draft exists, for actions that would replace it. */
  onDraftPresence: (hasDraft: boolean) => void;
}

/**
 * The channel composer owns its draft, caret and suggestion state, so typing
 * re-renders only this form rather than the whole conversation.
 */
export default function Composer({
  ref,
  channelName,
  draftKey,
  direct,
  disabled,
  identityReady,
  sending,
  sendRejected,
  pendingSend,
  authorId,
  mentionMembers,
  onSend,
  onTyping,
  onResize,
  onDraftPresence,
}: ComposerProps) {
  const [draft, setDraft] = useState(() => (draftKey ? readDraft(draftKey) : ""));
  const [validationError, setValidationError] = useState<string>();
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const people = useMemo(() => mentionMembers?.filter((member) => member.id !== authorId), [mentionMembers, authorId]);
  const suggestions = useComposerSuggestions({
    id: "chat",
    draft,
    input: composerRef,
    people,
    specialMentions: !direct,
    onInsert: (value) => {
      setDraft(value);
      setValidationError(undefined);
      onTyping(!!value.trim());
    },
    onTooLong: () => setValidationError("Messages must be 4,000 characters or fewer."),
  });

  useEffect(() => {
    if (draftKey) saveDraft(draftKey, draft);
  }, [draftKey, draft]);
  const hasDraft = draft !== "";
  useEffect(() => {
    onDraftPresence(hasDraft);
  }, [hasDraft, onDraftPresence]);
  // A new local send clears the draft it came from, before the next paint.
  const pendingId = pendingSend && !pendingSend.threadRootId ? pendingSend.clientMessageId : undefined;
  useLayoutEffect(() => {
    if (!pendingId || !pendingSend) return;
    const sent = pendingSend.text;
    setDraft((current) => (current === sent ? "" : current));
  }, [pendingId]);

  const resize = () =>
    onResize(() => {
      const composer = composerRef.current;
      if (!composer) return false;
      const height = composer.offsetHeight;
      composer.style.height = "0px";
      composer.style.height = `${composer.scrollHeight + composer.offsetHeight - composer.clientHeight}px`;
      return composer.offsetHeight !== height;
    });
  const resizeRef = useRef(resize);
  resizeRef.current = resize;
  useLayoutEffect(resize, [draft]);
  // Width changes rewrap the draft; observe them once rather than per keystroke.
  useLayoutEffect(() => {
    const composer = composerRef.current;
    if (!composer) return;
    let width = composer.clientWidth;
    const observer = new ResizeObserver(() => {
      if (composer.clientWidth === width) return;
      width = composer.clientWidth;
      resizeRef.current();
    });
    observer.observe(composer);
    return () => observer.disconnect();
  }, []);

  const submit = async () => {
    if (!identityReady || sending || sendRejected || pendingSend?.threadRootId) return;
    setValidationError(undefined);
    const submitted = pendingSend?.text ?? draft;
    try {
      await onSend(submitted);
    } catch (error) {
      setValidationError(error instanceof Error ? error.message : "Message could not be sent.");
    }
  };
  useImperativeHandle(ref, () => ({
    restore(text: string) {
      setDraft(text);
      composerRef.current?.focus();
    },
    submit() {
      void submit();
    },
  }));

  const characterCount = Array.from(draft).length;

  return (
    <>
      {validationError && (
        <p className="chat-inline-error" role="alert">
          {validationError}
        </p>
      )}
      <form
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        <label className="sr-only" htmlFor="chat-message">
          Message {channelName}
        </label>
        {suggestions.popup}
        <textarea
          ref={composerRef}
          id="chat-message"
          rows={1}
          value={draft}
          disabled={disabled}
          enterKeyHint="send"
          aria-describedby="chat-composer-hint"
          {...suggestions.textarea}
          placeholder={`Message ${direct ? "" : "#"}${channelName}`}
          onChange={(event) => {
            setDraft(event.target.value);
            suggestions.change(event.target);
            setValidationError(undefined);
            onTyping(!!event.target.value.trim());
          }}
          onBlur={() => {
            suggestions.blur();
            onTyping(false);
          }}
          onKeyDown={(event) => {
            if (suggestions.keyDown(event)) return;
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              if (!sending) void submit();
            }
          }}
        />
        <span id="chat-composer-hint" className="sr-only">
          Type : to find emoji or @ to mention someone. Up and Down choose; Enter or Tab inserts; Escape closes
          suggestions. Enter to send. Shift+Enter for a new line.
        </span>
        {characterCount >= COUNTER_START && (
          <small className="chat-counter" data-tone={counterTone(characterCount)}>
            {characterCount.toLocaleString()} / 4,000
          </small>
        )}
      </form>
    </>
  );
}
