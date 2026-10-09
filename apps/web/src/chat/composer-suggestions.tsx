import { useEffect, useLayoutEffect, useState, type KeyboardEvent, type RefObject } from "react";
import Avatar from "../components/Avatar";
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

/**
 * `:` emoji and `@` mention suggestions for a message textarea. The channel
 * composer and the thread reply box share it; `idPrefix` keeps their option
 * IDs apart when both are on screen.
 */
export function useComposerSuggestions({
  draft,
  idPrefix,
  textarea,
  mentionMembers,
  authorId,
  direct,
  onInsert,
}: {
  draft: string;
  idPrefix: string;
  textarea: RefObject<HTMLTextAreaElement | null>;
  mentionMembers?: MentionCandidate[];
  authorId?: string;
  direct: boolean;
  /** Applies an accepted suggestion; `undefined` means it would exceed 4,000 characters. */
  onInsert: (value: string | undefined) => void;
}) {
  const [selection, setSelection] = useState({ start: 0, end: 0 });
  const [focused, setFocused] = useState(false);
  const [composing, setComposing] = useState(false);
  const [emojiChoices, setEmojiChoices] = useState<EmojiChoice[]>();
  const [emojiError, setEmojiError] = useState(false);
  const [dismissedSuggestions, setDismissedSuggestions] = useState<string>();
  const [selectedSuggestion, setSelectedSuggestion] = useState(0);
  const selectionKey = `${draft}:${selection.start}:${selection.end}`;
  const suggesting = focused && !composing && dismissedSuggestions !== selectionKey;
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
    ? suggestions[activeOption] && `${idPrefix}-emoji-${suggestions[activeOption].id}`
    : mentionOptions[activeOption] && `${idPrefix}-mention-${mentionName(mentionOptions[activeOption])}`;
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

  const updateSelection = (input: HTMLTextAreaElement) => {
    setSelection({ start: input.selectionStart, end: input.selectionEnd });
    const nextKey = `${input.value}:${input.selectionStart}:${input.selectionEnd}`;
    setDismissedSuggestions((current) => (current === nextKey ? current : undefined));
  };
  const apply = (result: { value: string; caret: number } | undefined) => {
    if (!result) return onInsert(undefined);
    onInsert(result.value);
    setSelection({ start: result.caret, end: result.caret });
    requestAnimationFrame(() => {
      textarea.current?.focus();
      textarea.current?.setSelectionRange(result.caret, result.caret);
    });
  };
  const chooseEmoji = (entry: EmojiChoice) => {
    if (token) apply(insertEmoji(draft, token, entry.emoji));
  };
  const chooseMention = (option: MentionSuggestion) => {
    if (mention) apply(insertMention(draft, mention, mentionName(option)));
  };

  return {
    composing,
    /** Spread onto the textarea; its own onChange must also call `changed`. */
    textareaProps: {
      "aria-autocomplete": "list" as const,
      "aria-controls": suggestions.length
        ? `${idPrefix}-emoji-options`
        : mentionOpen
          ? `${idPrefix}-mention-options`
          : undefined,
      "aria-activedescendant": activeOptionId || undefined,
      onSelect: (event: { currentTarget: HTMLTextAreaElement }) => updateSelection(event.currentTarget),
      onCompositionStart: () => setComposing(true),
      onCompositionEnd: (event: { currentTarget: HTMLTextAreaElement }) => {
        setComposing(false);
        updateSelection(event.currentTarget);
      },
    },
    focused: (input: HTMLTextAreaElement) => {
      setFocused(true);
      updateSelection(input);
    },
    blurred: () => setFocused(false),
    changed: (input: HTMLTextAreaElement) => {
      setDismissedSuggestions(undefined);
      updateSelection(input);
    },
    /** Handles suggestion keys; returns true when the key was used. */
    keyDown: (event: KeyboardEvent<HTMLTextAreaElement>) => {
      if ((emojiOpen || mentionOpen) && event.key === "Escape") {
        event.preventDefault();
        setDismissedSuggestions(selectionKey);
        return true;
      }
      if (optionCount && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
        event.preventDefault();
        setSelectedSuggestion((activeOption + (event.key === "ArrowDown" ? 1 : optionCount - 1)) % optionCount);
        return true;
      }
      if (optionCount && !event.shiftKey && (event.key === "Enter" || event.key === "Tab")) {
        event.preventDefault();
        if (token) chooseEmoji(suggestions[activeOption]);
        else chooseMention(mentionOptions[activeOption]);
        return true;
      }
      // Enter waits while the emoji catalog loads rather than sending `:name`.
      if (emojiOpen && !emojiChoices && !emojiError && event.key === "Enter" && !event.shiftKey) {
        event.preventDefault();
        return true;
      }
      return false;
    },
    popup: (
      <>
        {emojiOpen && (
          <div className="chat-emoji-suggestions">
            {suggestions.length > 0 ? (
              <div id={`${idPrefix}-emoji-options`} role="listbox" aria-label="Emoji suggestions">
                {suggestions.map((entry, index) => (
                  <button
                    type="button"
                    role="option"
                    id={`${idPrefix}-emoji-${entry.id}`}
                    key={entry.id}
                    tabIndex={-1}
                    aria-selected={index === activeOption}
                    aria-label={`Insert ${entry.name} emoji`}
                    onPointerDown={(event) => event.preventDefault()}
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
            <div id={`${idPrefix}-mention-options`} role="listbox" aria-label="People to mention">
              {mentionOptions.map((option, index) => {
                const name = mentionName(option);
                return (
                  <button
                    type="button"
                    role="option"
                    id={`${idPrefix}-mention-${name}`}
                    key={`${option.kind}:${name}`}
                    tabIndex={-1}
                    aria-selected={index === activeOption}
                    aria-label={
                      option.kind === "member"
                        ? `Mention ${option.member.displayName}, @${name}`
                        : `Mention @${name}, ${specialMentionLabels[option.kind].toLowerCase()}`
                    }
                    onPointerDown={(event) => event.preventDefault()}
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
      </>
    ),
  };
}
