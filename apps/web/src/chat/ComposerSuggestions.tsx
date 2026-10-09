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
 * `:` emoji and `@` mention suggestions for one composer textarea. The channel
 * and thread composers share this so both offer the same choices and keys.
 */
export function useComposerSuggestions({
  id,
  draft,
  input,
  people,
  specialMentions,
  onInsert,
  onTooLong,
}: {
  /** Prefix for the popup's element IDs; unique per composer. */
  id: string;
  draft: string;
  input: RefObject<HTMLTextAreaElement | null>;
  /** People `@` can suggest, without the author; undefined until loaded. */
  people?: MentionCandidate[];
  /** `@everyone` and `@here`, which DMs do not offer. */
  specialMentions: boolean;
  onInsert: (value: string) => void;
  onTooLong: () => void;
}) {
  const [selection, setSelection] = useState({ start: 0, end: 0 });
  const [focused, setFocused] = useState(false);
  const [composing, setComposing] = useState(false);
  const [emojiChoices, setEmojiChoices] = useState<EmojiChoice[]>();
  const [emojiError, setEmojiError] = useState(false);
  const [dismissed, setDismissed] = useState<string>();
  const [selected, setSelected] = useState(0);
  const selectionKey = `${draft}:${selection.start}:${selection.end}`;
  const suggesting = focused && !composing && dismissed !== selectionKey;
  const token = suggesting ? emojiToken(draft, selection.start, selection.end) : undefined;
  const emojiOpen = !!token;
  const emojiOptions = token && emojiChoices ? emojiSuggestions(emojiChoices, token.query) : [];
  // `:` and `@` tokens never overlap; only one popup can be open.
  const mention = suggesting && !token ? mentionToken(draft, selection.start, selection.end) : undefined;
  const mentionOptions = mention ? mentionSuggestions(people ?? [], mention.query, specialMentions) : [];
  const mentionOpen = mentionOptions.length > 0;
  const optionCount = token ? emojiOptions.length : mentionOptions.length;
  const activeOption = Math.min(selected, Math.max(0, optionCount - 1));
  const activeOptionId = token
    ? emojiOptions[activeOption] && `${id}-emoji-${emojiOptions[activeOption].id}`
    : mentionOptions[activeOption] && `${id}-mention-${mentionName(mentionOptions[activeOption])}`;
  useLayoutEffect(() => {
    if (activeOptionId) document.getElementById(activeOptionId)?.scrollIntoView({ block: "nearest" });
  }, [activeOptionId]);
  useEffect(() => {
    setSelected(0);
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
  const track = (element: HTMLTextAreaElement) => {
    setSelection({ start: element.selectionStart, end: element.selectionEnd });
    const nextKey = `${element.value}:${element.selectionStart}:${element.selectionEnd}`;
    setDismissed((current) => (current === nextKey ? current : undefined));
  };
  const apply = (result: { value: string; caret: number } | undefined) => {
    if (!result) {
      onTooLong();
      return;
    }
    onInsert(result.value);
    setSelection({ start: result.caret, end: result.caret });
    requestAnimationFrame(() => {
      input.current?.focus();
      input.current?.setSelectionRange(result.caret, result.caret);
    });
  };
  const chooseEmoji = (entry: EmojiChoice) => {
    if (token) apply(insertEmoji(draft, token, entry.emoji));
  };
  const chooseMention = (option: MentionSuggestion) => {
    if (mention) apply(insertMention(draft, mention, mentionName(option)));
  };
  /** Handles suggestion keys; true when the composer must not act on the key. */
  const keyDown = (event: KeyboardEvent<HTMLTextAreaElement>) => {
    if (event.nativeEvent.isComposing || composing) return true;
    if ((emojiOpen || mentionOpen) && event.key === "Escape") {
      event.preventDefault();
      setDismissed(selectionKey);
      return true;
    }
    if (optionCount && (event.key === "ArrowDown" || event.key === "ArrowUp")) {
      event.preventDefault();
      setSelected((activeOption + (event.key === "ArrowDown" ? 1 : optionCount - 1)) % optionCount);
      return true;
    }
    if (optionCount && !event.shiftKey && (event.key === "Enter" || event.key === "Tab")) {
      event.preventDefault();
      if (token) chooseEmoji(emojiOptions[activeOption]);
      else chooseMention(mentionOptions[activeOption]);
      return true;
    }
    if (emojiOpen && !emojiChoices && !emojiError && event.key === "Enter" && !event.shiftKey) {
      event.preventDefault();
      return true;
    }
    return false;
  };
  const popup = emojiOpen ? (
    <div className="chat-emoji-suggestions">
      {emojiOptions.length > 0 ? (
        <div id={`${id}-emoji-options`} role="listbox" aria-label="Emoji suggestions">
          {emojiOptions.map((entry, index) => (
            <button
              type="button"
              role="option"
              id={`${id}-emoji-${entry.id}`}
              key={entry.id}
              tabIndex={-1}
              aria-selected={index === activeOption}
              aria-label={`Insert ${entry.name} emoji`}
              // The pointer moves the highlight, so Enter inserts the row under it.
              onMouseMove={() => index !== activeOption && setSelected(index)}
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
  ) : mentionOpen ? (
    <div className="chat-emoji-suggestions chat-mention-suggestions">
      <div id={`${id}-mention-options`} role="listbox" aria-label="People to mention">
        {mentionOptions.map((option, index) => {
          const name = mentionName(option);
          return (
            <button
              type="button"
              role="option"
              id={`${id}-mention-${name}`}
              key={`${option.kind}:${name}`}
              tabIndex={-1}
              aria-selected={index === activeOption}
              aria-label={
                option.kind === "member"
                  ? `Mention ${option.member.displayName}, @${name}`
                  : `Mention @${name}, ${specialMentionLabels[option.kind].toLowerCase()}`
              }
              onMouseMove={() => index !== activeOption && setSelected(index)}
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
  ) : null;
  return {
    popup,
    keyDown,
    /** Call from the textarea's change handler. */
    change: (element: HTMLTextAreaElement) => {
      setDismissed(undefined);
      track(element);
    },
    /** Spread onto the textarea; callers that also handle blur call `blur()` themselves. */
    textarea: {
      "aria-autocomplete": "list" as const,
      "aria-controls": emojiOptions.length ? `${id}-emoji-options` : mentionOpen ? `${id}-mention-options` : undefined,
      "aria-activedescendant": activeOptionId || undefined,
      onFocus: (event: { currentTarget: HTMLTextAreaElement }) => {
        setFocused(true);
        track(event.currentTarget);
      },
      onSelect: (event: { currentTarget: HTMLTextAreaElement }) => track(event.currentTarget),
      onCompositionStart: () => setComposing(true),
      onCompositionEnd: (event: { currentTarget: HTMLTextAreaElement }) => {
        setComposing(false);
        track(event.currentTarget);
      },
    },
    blur: () => setFocused(false),
  };
}
