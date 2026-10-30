import { useEffect, useId, useLayoutEffect, useState, type KeyboardEvent } from "react";
import { rankPeople, type MentionCandidate } from "../chat/mentions.ts";
import Avatar from "./Avatar";

/** Avatar, display name, and the username beneath it; `@` suggestions use the same row. */
export function PersonSuggestion({ person }: { person: MentionCandidate }) {
  return (
    <>
      <i className="chat-mention-avatar">
        <Avatar person={person} name={person.displayName} />
      </i>
      <span className="person-suggestion-names">
        <span>{person.displayName}</span>
        <small>
          @{person.username}
          {person.invited && " · Already invited"}
        </small>
      </span>
    </>
  );
}

/**
 * Matches for a username field from people the client already has. Choosing
 * one fills in the exact username and never submits: invitations and DMs are
 * still looked up by exact username, and the user confirms with the form's button.
 * The list renders inline below the field because dialogs scroll and would clip a popup.
 */
export function usePeopleSuggestions({
  value,
  people,
  onChoose,
}: {
  value: string;
  /** Candidates already narrowed for this field; undefined until loaded. */
  people?: MentionCandidate[];
  onChoose: (username: string) => void;
}) {
  const id = useId();
  // The field value the list was closed for (chosen or Escape); typing reopens it.
  const [dismissed, setDismissed] = useState<string>();
  const [selected, setSelected] = useState(0);
  const query = value.trim().replace(/^@/, "");
  // This list occupies layout space. Collapsing it on blur moves the following
  // controls between pointer-down and click, swallowing their first activation.
  const options = query && dismissed !== value && people ? rankPeople(people, query) : [];
  const open = options.length > 0;
  const active = Math.min(selected, Math.max(0, options.length - 1));
  const activeId = open ? `${id}-${options[active].id}` : undefined;
  useEffect(() => {
    setSelected(0);
    setDismissed((current) => (current === value ? current : undefined));
  }, [value]);
  useLayoutEffect(() => {
    if (activeId) document.getElementById(activeId)?.scrollIntoView({ block: "nearest" });
  }, [activeId]);
  const choose = (person: MentionCandidate) => {
    setDismissed(person.username);
    onChoose(person.username);
  };
  return {
    list: open ? (
      <div className="chat-emoji-suggestions chat-mention-suggestions people-suggestions">
        <div id={`${id}-options`} role="listbox" aria-label="Matching people">
          {options.map((person, index) => (
            <button
              type="button"
              role="option"
              id={`${id}-${person.id}`}
              key={person.id}
              tabIndex={-1}
              aria-selected={index === active}
              aria-label={`${person.displayName}, @${person.username}${person.invited ? ", already invited" : ""}`}
              onPointerDown={(event) => event.preventDefault()}
              onMouseMove={() => setSelected(index)}
              onClick={() => choose(person)}
            >
              <PersonSuggestion person={person} />
            </button>
          ))}
        </div>
      </div>
    ) : null,
    /** Spread onto the username input. */
    input: {
      role: "combobox" as const,
      "aria-autocomplete": "list" as const,
      "aria-expanded": open,
      "aria-controls": open ? `${id}-options` : undefined,
      "aria-activedescendant": activeId,
      onKeyDown: (event: KeyboardEvent<HTMLInputElement>) => {
        if (!open || event.nativeEvent.isComposing) return;
        if (event.key === "ArrowDown" || event.key === "ArrowUp") {
          event.preventDefault();
          setSelected((active + (event.key === "ArrowDown" ? 1 : options.length - 1)) % options.length);
        } else if (event.key === "Enter") {
          event.preventDefault();
          choose(options[active]);
        } else if (event.key === "Escape") {
          // Close the list, not the dialog around it.
          event.preventDefault();
          event.stopPropagation();
          setDismissed(value);
        }
      },
    },
  };
}
