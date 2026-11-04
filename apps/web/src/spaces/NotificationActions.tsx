import { useEffect, useRef, useState } from "react";
import { Reply } from "../../../../shared/message-icons";
import { getAccount, getRememberedAccount } from "../account/client";
import { apiError, forgetAccountSession, sharedAccountSession } from "../chat/client";
import EmojiButton from "../chat/EmojiButton";
import { emojiAsset, emojiCode } from "../chat/emoji";
import type { ComposerInput } from "../chat/MentionTextarea";
import ProfileComposer, { type InboxReplyTarget } from "../chat/ProfileComposer";
import { isChatReactionEvent, type ChatReaction } from "../chat/types";
import type { InboxItem } from "./client";

/** Actions never open chat, acknowledge activity, or accept message requests. */
export default function NotificationActions({
  accountId,
  item,
  expanded,
  disabled,
  onExpand,
  onStateChange,
}: {
  accountId: string;
  item: InboxItem;
  expanded: boolean;
  disabled: boolean;
  onExpand: () => void;
  onStateChange: (state: { picking: boolean; sending: boolean }) => void;
}) {
  const input = useRef<ComposerInput>(null);
  const mounted = useRef(true);
  const reacting = useRef(false);
  const [saving, setSaving] = useState(false);
  const [reactions, setReactions] = useState<ChatReaction[]>([]);
  const [status, setStatus] = useState<string>();
  const [error, setError] = useState<string>();
  const target: InboxReplyTarget = {
    accountId,
    channelId: (item.channelId ?? item.conversationId)!,
    messageId: item.messageId!,
    threadRootId: item.threadRootId ?? undefined,
  };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (expanded) input.current?.focus({ preventScroll: true });
  }, [expanded]);

  async function react(emoji: string, active: boolean) {
    if (reacting.current) return;
    reacting.current = true;
    setSaving(true);
    setError(undefined);
    setStatus(undefined);
    const current = () => mounted.current && getRememberedAccount()?.id === accountId;
    try {
      const account = await getAccount();
      if (account?.id !== accountId || !account.displayName)
        throw new Error("Your account changed. Reopen Notifications to react.");
      const session = await sharedAccountSession(account.id, account.displayName);
      if (!current()) return;
      const response = await fetch(
        `/api/chat/channels/${encodeURIComponent(target.channelId)}/messages/${encodeURIComponent(target.messageId)}/reactions`,
        {
          method: "PUT",
          headers: { "content-type": "application/json", "x-caper-chat-token": session.token },
          body: JSON.stringify({ emoji, active }),
          signal: AbortSignal.timeout(10_000),
        },
      );
      if (!response.ok) {
        if (response.status === 401) forgetAccountSession(session.token);
        throw await apiError(response, "Reaction couldn’t be saved. Choose it again to retry.");
      }
      const event: unknown = await response.json();
      if (!isChatReactionEvent(event) || event.channelId !== target.channelId || event.messageId !== target.messageId)
        throw new Error("Couldn’t confirm the reaction. Choose it again to retry.");
      if (current()) {
        setReactions(event.reactions);
        setStatus(active ? "Reaction added" : "Reaction removed");
      }
    } catch (reason) {
      if (current()) setError(reason instanceof Error ? reason.message : "Reaction couldn’t be saved.");
    } finally {
      reacting.current = false;
      if (mounted.current) setSaving(false);
    }
  }

  return (
    <div className="inbox-message-actions">
      <div className="inbox-action-bar">
        <button type="button" aria-expanded={expanded} disabled={disabled} onClick={onExpand}>
          <Reply size={16} aria-hidden="true" />
          {expanded ? "Close reply" : "Reply"}
        </button>
        {reactions.map((reaction) => (
          <button
            type="button"
            key={reaction.emoji}
            aria-label={`React ${reaction.emoji}`}
            aria-pressed={reaction.authorIds.includes(accountId)}
            disabled={disabled || saving}
            onClick={() => void react(reaction.emoji, !reaction.authorIds.includes(accountId))}
          >
            <img src={emojiAsset(emojiCode(reaction.emoji))} width={16} height={16} alt="" />
            {reaction.authorIds.length}
          </button>
        ))}
        <EmojiButton label="Add reaction" disabled={disabled || saving} onSelect={(emoji) => void react(emoji, true)} />
      </div>
      {expanded && (
        <div className="inbox-inline-reply">
          <ProfileComposer
            personId={item.actorId}
            username={item.actorName}
            inputRef={input}
            replyTarget={target}
            onSent={() => mounted.current && setStatus("Reply sent")}
            onStateChange={onStateChange}
          />
        </div>
      )}
      {status && (
        <p className="inbox-action-status" role="status">
          {status}
        </p>
      )}
      {error && (
        <p className="inbox-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
