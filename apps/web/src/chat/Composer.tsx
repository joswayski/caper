import { useEffect, useImperativeHandle, useLayoutEffect, useMemo, useRef, useState, type Ref } from "react";
import { Paperclip, Send } from "lucide-react";
import { DraftAttachments, type DraftAttachment } from "./Attachments.tsx";
import { useComposerSuggestions } from "./ComposerSuggestions.tsx";
import { COUNTER_START, counterTone } from "./counter.ts";
import { readDraft, saveDraft } from "./drafts.ts";
import EmojiButton from "./EmojiButton.tsx";
import { insertEmoji } from "./emoji-autocomplete.ts";
import MentionTextarea from "./MentionTextarea.tsx";
import type { MentionCandidate } from "./mentions.ts";
import { MAX_ATTACHMENTS } from "./uploads.ts";

/** Files picked for the next message; the conversation owns their uploads. */
export interface ComposerAttachments {
  /** Whether new files can be added (uploads configured, conversation writable). */
  enabled: boolean;
  /** Why the stable attachment control cannot be used yet. */
  disabledReason?: string;
  drafts: DraftAttachment[];
  /** A problem adding files, such as too many. */
  error?: string;
  onAdd: (files: File[]) => void;
  onRemove: (key: string) => void;
  onRetry: (key: string) => void;
}

export interface ComposerHandle {
  focus(): void;
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
  pendingSend?: {
    clientMessageId: string;
    text: string;
    threadRootId?: string;
  };
  authorId?: string;
  mentionMembers?: MentionCandidate[];
  onSend: (text: string) => Promise<unknown> | undefined;
  onTyping: (active: boolean) => void;
  /** Wraps each resize, which reports whether the height changed, so the
   * conversation can measure its position first and stay on the newest message. */
  onResize: (resize: () => boolean) => void;
  /** Whether a draft exists, for actions that would replace it. */
  onDraftPresence: (hasDraft: boolean) => void;
  attachments?: ComposerAttachments;
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
  attachments,
}: ComposerProps) {
  const [draft, setDraft] = useState(() => (draftKey ? readDraft(draftKey) : ""));
  const [validationError, setValidationError] = useState<string>();
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
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
  const previousPending = useRef<string | undefined>(undefined);
  useLayoutEffect(() => {
    if (pendingId && pendingSend && !previousPending.current) {
      const sent = pendingSend.text;
      setDraft((current) => (current === sent ? "" : current));
    }
    // Renewing a pending upload changes its wire UUID, not the user's next draft.
    previousPending.current = pendingId;
  }, [pendingId]);

  const resize = () =>
    onResize(() => {
      const composer = composerRef.current;
      if (!composer) return false;
      const height = composer.offsetHeight;
      // Collapsing the textarea to measure it must not shrink the form for that
      // moment: the conversation would grow and clamp its scroll position, so
      // every keystroke in a multi-line draft scrolled the messages up.
      const form = composer.closest("form");
      if (form) form.style.minHeight = `${form.offsetHeight}px`;
      composer.style.height = "0px";
      composer.style.height = `${composer.scrollHeight + composer.offsetHeight - composer.clientHeight}px`;
      if (form) form.style.minHeight = "";
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
    focus() {
      composerRef.current?.focus();
    },
    restore(text: string) {
      setDraft(text);
      composerRef.current?.focus();
    },
    submit() {
      void submit();
    },
  }));

  const chooseEmoji = (emoji: string) => {
    const input = composerRef.current;
    const start = input?.selectionStart ?? draft.length;
    const result = insertEmoji(draft, { start, end: input?.selectionEnd ?? start, query: "" }, emoji);
    if (!result) {
      setValidationError("Messages must be 4,000 characters or fewer.");
      return;
    }
    setDraft(result.value);
    setValidationError(undefined);
    onTyping(true);
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(result.caret, result.caret);
    });
  };
  const characterCount = Array.from(draft).length;
  const error = validationError ?? attachments?.error;
  const canAttach = !!attachments?.enabled;

  return (
    <>
      {error && (
        <p className="chat-inline-error" role="alert">
          {error}
        </p>
      )}
      {attachments && (
        <DraftAttachments drafts={attachments.drafts} onRemove={attachments.onRemove} onRetry={attachments.onRetry} />
      )}
      <form
        data-attach={!!attachments || undefined}
        onSubmit={(event) => {
          event.preventDefault();
          void submit();
        }}
      >
        {attachments && (
          <>
            <button
              type="button"
              className="chat-attach"
              aria-label="Attach files"
              title={
                !canAttach
                  ? (attachments.disabledReason ?? "Checking file uploads…")
                  : attachments.drafts.length >= MAX_ATTACHMENTS
                    ? `You can attach up to ${MAX_ATTACHMENTS} files.`
                    : "Attach files"
              }
              disabled={!canAttach || disabled || attachments.drafts.length >= MAX_ATTACHMENTS}
              onClick={() => fileInputRef.current?.click()}
            >
              <Paperclip size={17} aria-hidden="true" />
            </button>
            <input
              ref={fileInputRef}
              type="file"
              multiple
              hidden
              disabled={!canAttach || disabled}
              onChange={(event) => {
                if (!canAttach || disabled) return;
                attachments.onAdd([...(event.target.files ?? [])]);
                event.target.value = "";
              }}
            />
          </>
        )}
        <label className="sr-only" htmlFor="chat-message">
          Message {channelName}
        </label>
        {suggestions.popup}
        <MentionTextarea
          ref={composerRef}
          id="chat-message"
          people={people}
          specialMentions={!direct}
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
          onPaste={(event) => {
            if (event.clipboardData.files.length && canAttach && !disabled) {
              event.preventDefault();
              attachments.onAdd([...event.clipboardData.files]);
            }
          }}
          onKeyDown={(event) => {
            if (suggestions.keyDown(event)) return;
            if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
              event.preventDefault();
              if (!sending) void submit();
            }
          }}
        />
        <div className="chat-composer-tools">
          <EmojiButton disabled={disabled} onSelect={chooseEmoji} returnFocus={composerRef} />
          {/* Touch layouts get a visible send button, as in the native apps; a
              keyboard's Enter still sends everywhere. */}
          <button
            type="submit"
            className="chat-composer-send chat-composer-send-touch"
            aria-label="Send message"
            title="Send"
            disabled={
              disabled ||
              !identityReady ||
              sending ||
              sendRejected ||
              (!draft.trim() && !attachments?.drafts.length && !(pendingSend && !pendingSend.threadRootId))
            }
            // Keep the keyboard open: tapping send must not blur the textarea.
            onMouseDown={(event) => event.preventDefault()}
          >
            <Send size={16} aria-hidden="true" />
          </button>
        </div>
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
