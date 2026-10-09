import { useEffect, useImperativeHandle, useLayoutEffect, useRef, useState, type Ref } from "react";
import Avatar from "../components/Avatar";
import { COUNTER_START, counterTone } from "./counter.ts";
import { emojiAsset } from "./emoji.ts";
import { emojiToken, emojiSuggestions, insertEmoji, loadEmojiChoices, type EmojiChoice } from "./emoji-autocomplete.ts";
import {
  insertMention,
  mentionName,
  mentionSuggestions,
  mentionToken,
  specialMentionLabels,
  type MentionCandidate,
  type MentionSuggestion,
} from "./mentions.ts";

export interface ComposerHandle {
  /** Puts a rejected message back into the composer to edit. */
  restore(text: string): void;
  /** Sends the draft, or resends an unconfirmed message. */
  submit(): void;
}

interface ComposerProps {
  ref?: Ref<ComposerHandle>;
  channelName: string;
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
  const [draft, setDraft] = useState("");
  const [validationError, setValidationError] = useState<string>();
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const [selection, setSelection] = useState({ start: 0, end: 0 });
  const [composerFocused, setComposerFocused] = useState(false);
  const [composing, setComposing] = useState(false);
  const [emojiChoices, setEmojiChoices] = useState<EmojiChoice[]>();
  const [emojiError, setEmojiError] = useState(false);
  const [dismissedSuggestions, setDismissedSuggestions] = useState<string>();
  const [selectedSuggestion, setSelectedSuggestion] = useState(0);
  const selectionKey = `${draft}:${selection.start}:${selection.end}`;
  const suggesting = composerFocused && !composing && dismissedSuggestions !== selectionKey;
  const token = suggesting ? emojiToken(draft, selection.start, selection.end) : undefined;
  const emojiOpen = !!token;
  const suggestions = token && emojiChoices ? emojiSuggestions(emojiChoices, token.query) : [];
  // `:` and `@` tokens never overlap; only one popup can be open.
  const mention = suggesting && !token ? mentionToken(draft, selection.start, selection.end) : undefined;
  const mentionOptions = mention
    ? mentionSuggestions(
        (mentionMembers ?? []).filter((member) => member.id !== authorId),
        mention.query,
        !direct,
      )
    : [];
  const mentionOpen = mentionOptions.length > 0;
  const optionCount = token ? suggestions.length : mentionOptions.length;
  const activeOption = Math.min(selectedSuggestion, Math.max(0, optionCount - 1));
  const activeOptionId = token
    ? suggestions[activeOption] && `chat-emoji-${suggestions[activeOption].id}`
    : mentionOptions[activeOption] && `chat-mention-${mentionName(mentionOptions[activeOption])}`;
  useLayoutEffect(() => {
    if (activeOptionId) document.getElementById(activeOptionId)?.scrollIntoView({ block: "nearest" });
  }, [activeOptionId]);
  useEffect(() => {
    setSelectedSuggestion(0);
  }, [selectionKey]);
  useEffect(() => {
    if (!emojiOpen || emojiChoices) return;
    let active = true;
    setEmojiError(false);
    void loadEmojiChoices()
      .then((choices) => {
        if (active) setEmojiChoices(choices);
      })
      .catch(() => {
        if (active) setEmojiError(true);
      });
    return () => {
      active = false;
    };
  }, [emojiOpen, emojiChoices]);

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

  const updateSelection = (input: HTMLTextAreaElement) => {
    setSelection({ start: input.selectionStart, end: input.selectionEnd });
    const nextKey = `${input.value}:${input.selectionStart}:${input.selectionEnd}`;
    setDismissedSuggestions((current) => (current === nextKey ? current : undefined));
  };
  const chooseEmoji = (entry: EmojiChoice) => {
    if (token) applyInsertion(insertEmoji(draft, token, entry.emoji));
  };
  const chooseMention = (option: MentionSuggestion) => {
    if (mention) applyInsertion(insertMention(draft, mention, mentionName(option)));
  };
  const applyInsertion = (result: { value: string; caret: number } | undefined) => {
    if (!result) {
      setValidationError("Messages must be 4,000 characters or fewer.");
      return;
    }
    setDraft(result.value);
    setSelection({ start: result.caret, end: result.caret });
    setValidationError(undefined);
    onTyping(!!result.value.trim());
    requestAnimationFrame(() => {
      composerRef.current?.focus();
      composerRef.current?.setSelectionRange(result.caret, result.caret);
    });
  };

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
        {emojiOpen && (
          <div className="chat-emoji-suggestions">
            {suggestions.length > 0 ? (
              <div id="chat-emoji-options" role="listbox" aria-label="Emoji suggestions">
                {suggestions.map((entry, index) => (
                  <button
                    type="button"
                    role="option"
                    id={`chat-emoji-${entry.id}`}
                    key={entry.id}
                    tabIndex={-1}
                    aria-selected={index === activeOption}
                    aria-label={`Insert ${entry.name} emoji`}
                    onPointerDown={(event) => event.preventDefault()}
                    onMouseMove={() => setSelectedSuggestion(index)}
                    onClick={() => chooseEmoji(entry)}
                  >
                    <img src={emojiAsset(entry.id)} alt="" width="24" height="24" />
                    <span>:{entry.name.replaceAll(" ", "_")}:</span>
                  </button>
                ))}
              </div>
            ) : (
              <p role="status">
                {emojiError
                  ? "Emoji suggestions unavailable. You can still send text."
                  : emojiChoices
                    ? "No emoji found."
                    : "Loading emoji…"}
              </p>
            )}
          </div>
        )}
        {mentionOpen && (
          <div className="chat-emoji-suggestions chat-mention-suggestions">
            <div id="chat-mention-options" role="listbox" aria-label="People to mention">
              {mentionOptions.map((option, index) => {
                const name = mentionName(option);
                return (
                  <button
                    type="button"
                    role="option"
                    id={`chat-mention-${name}`}
                    key={`${option.kind}:${name}`}
                    tabIndex={-1}
                    aria-selected={index === activeOption}
                    aria-label={
                      option.kind === "member"
                        ? `Mention ${option.member.displayName}, @${name}`
                        : `Mention @${name}, ${specialMentionLabels[option.kind].toLowerCase()}`
                    }
                    onPointerDown={(event) => event.preventDefault()}
                    onMouseMove={() => setSelectedSuggestion(index)}
                    onClick={() => chooseMention(option)}
                  >
                    {option.kind === "member" ? (
                      <>
                        <i className="chat-mention-avatar">
                          <Avatar avatarId={option.member.avatarId} name={option.member.displayName} />
                        </i>
                        <span>{option.member.displayName}</span>
                        <small>@{name}</small>
                      </>
                    ) : (
                      <>
                        <i className="chat-mention-avatar chat-mention-special" aria-hidden="true">
                          @
                        </i>
                        <span>@{name}</span>
                        <small>{specialMentionLabels[option.kind]}</small>
                      </>
                    )}
                  </button>
                );
              })}
            </div>
          </div>
        )}
        <textarea
          ref={composerRef}
          id="chat-message"
          rows={1}
          value={draft}
          disabled={disabled}
          enterKeyHint="send"
          aria-describedby="chat-composer-hint"
          aria-autocomplete="list"
          aria-controls={suggestions.length ? "chat-emoji-options" : mentionOpen ? "chat-mention-options" : undefined}
          aria-activedescendant={activeOptionId || undefined}
          placeholder={`Message ${direct ? "" : "#"}${channelName}`}
          onFocus={(event) => {
            setComposerFocused(true);
            updateSelection(event.currentTarget);
          }}
          onSelect={(event) => updateSelection(event.currentTarget)}
          onCompositionStart={() => setComposing(true)}
          onCompositionEnd={(event) => {
            setComposing(false);
            updateSelection(event.currentTarget);
          }}
          onChange={(event) => {
            setDraft(event.target.value);
            setDismissedSuggestions(undefined);
            updateSelection(event.target);
            setValidationError(undefined);
            onTyping(!!event.target.value.trim());
          }}
          onBlur={() => {
            setComposerFocused(false);
            onTyping(false);
          }}
          onKeyDown={(event) => {
            if (event.nativeEvent.isComposing || composing) return;
            if ((emojiOpen || mentionOpen) && event.key === "Escape") {
              event.preventDefault();
              setDismissedSuggestions(selectionKey);
              return;
            }
            if (optionCount && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
              event.preventDefault();
              setSelectedSuggestion((activeOption + (event.key === "ArrowDown" ? 1 : optionCount - 1)) % optionCount);
              return;
            }
            if (optionCount && !event.shiftKey && (event.key === "Enter" || event.key === "Tab")) {
              event.preventDefault();
              if (token) chooseEmoji(suggestions[activeOption]);
              else chooseMention(mentionOptions[activeOption]);
              return;
            }
            if (emojiOpen && !emojiChoices && !emojiError && event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              return;
            }
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
