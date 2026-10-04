import { SmilePlus } from "lucide-react";
import type { ChatMessage } from "./types.ts";
import { emojiAsset, emojiCode } from "./emoji.ts";

export interface ReactionSave {
  emoji: string;
  active: boolean;
  saving: boolean;
  error?: string;
}

export default function MessageReactions({ message, authorId, readOnly = false, save, pickerOpen, onReact, onOpenPicker, onDismissError }: {
  message: ChatMessage;
  authorId?: string;
  readOnly?: boolean;
  save?: ReactionSave;
  pickerOpen: boolean;
  onReact: (messageId: string, emoji: string, active: boolean) => Promise<void>;
  onOpenPicker: (anchor: HTMLButtonElement) => void;
  onDismissError: () => void;
}) {
  const canReact = !!authorId && !readOnly;
  const saving = !!save?.saving;

  return <>
    <button type="button" className="chat-add-reaction" onClick={(event) => onOpenPicker(event.currentTarget)}
      disabled={!canReact || saving} aria-label="Add reaction" title="Add reaction" aria-haspopup="dialog" aria-expanded={pickerOpen}>
      <SmilePlus size={18} aria-hidden="true" />
    </button>
    {!!message.reactions?.length && <div className="chat-reactions" aria-label="Reactions">
      {message.reactions.map(({ emoji, authorIds }) => {
        const mine = !!authorId && authorIds.includes(authorId);
        return <button type="button" key={emoji} className="chat-reaction" aria-pressed={mine}
          disabled={!canReact || saving} aria-label={`${emoji}, ${authorIds.length} ${authorIds.length === 1 ? "reaction" : "reactions"}${mine ? ", including you" : ""}`}
          title={mine ? "Remove your reaction" : "Add your reaction"} onClick={() => void onReact(message.id, emoji, !mine)}>
          <img src={emojiAsset(emojiCode(emoji))} width={18} height={18} alt={emoji} loading="lazy" />
          <span>{authorIds.length}</span>
        </button>;
      })}
    </div>}
    {saving && <div className="chat-send-status" role="status">Saving reaction…</div>}
    {save?.error && <div className="chat-send-status chat-send-error" role="alert">
      <span>{save.error}</span><button type="button" disabled={!canReact} onClick={() => void onReact(message.id, save.emoji, save.active)}>Retry reaction</button>
      <button type="button" onClick={onDismissError}>Dismiss</button>
    </div>}
  </>;
}
