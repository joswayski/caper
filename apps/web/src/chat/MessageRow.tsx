import { memo, type PointerEvent as ReactPointerEvent } from "react";
import { Ban, MoreHorizontal, Pin, SmilePlus } from "lucide-react";
import { Reply, Thread } from "../../../../shared/message-icons.ts";
import MessageReactions, { type ReactionSave } from "./MessageReactions.tsx";
import { preloadReactionPicker, quickReactions } from "./MessageActions.tsx";
import { emojiAsset, emojiCode } from "./emoji.ts";
import { ForwardCard } from "./Forwarding.tsx";
import { ReplyContext } from "./Replies.tsx";
import LinkedText from "./LinkedText.tsx";
import {
  attachmentsOf,
  type ChatAttachment,
  type ChatAuthor,
  type ChatMessage,
  type ChatReplyReference,
} from "./types.ts";
import type { PendingChatMessage } from "./client.ts";
import { MessageAttachments, type LocalPreview } from "./Attachments.tsx";
import { MessageEmbeds } from "./Embeds.tsx";
import { embedsOf } from "./embeds.ts";
import MessageGif from "./MessageGif.tsx";
import type { ViewAttachment } from "./MediaViewer.tsx";
import Avatar from "../components/Avatar";
import AdminBadge from "./AdminBadge.tsx";
import { blockedLabel } from "./blocked.ts";
import { feedbackAliases, mentionSegments, mentionsAccount, reportsFeedback, type MentionedUser } from "./mentions.ts";

export type FreshUrls = Record<string, { url?: string; previewUrl?: string }>;

export const isTouchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;

/** Only ready files have URLs worth refreshing; processing ones get theirs from events. */
export const withFreshUrls = (attachments: ChatAttachment[], freshUrls: FreshUrls = {}) =>
  attachments.map((attachment) =>
    attachment.status === undefined || attachment.status === "ready"
      ? { ...attachment, ...freshUrls[attachment.id] }
      : attachment,
  );

/**
 * What a message row can ask of its conversation. Chat hands rows one stable
 * object whose methods call the latest render's handlers, so a row re-renders
 * only when its own data changes, never because the conversation did.
 */
export interface RowActions {
  cancelPress(): void;
  /** A touch hold on `event.currentTarget` runs `onHold` unless the press is cancelled first. */
  holdPress(event: ReactPointerEvent<HTMLElement>, onHold: (anchor: HTMLElement) => void, row?: boolean): void;
  movePress(event: ReactPointerEvent<HTMLElement>): void;
  /** The click that ends a touch hold must not activate what opened under it. */
  suppressNextClick(): void;
  openAuthor(author: ChatAuthor, anchor: HTMLElement, focusOnOpen?: boolean): void;
  openMention(user: MentionedUser, anchor: HTMLElement): void;
  openActions(messageId: string, anchor: HTMLElement, inThread: boolean): void;
  /** A second click on the open menu's own trigger closes it, like a toggle. */
  toggleActions(messageId: string, anchor: HTMLElement, inThread: boolean): void;
  toggleReactionPicker(messageId: string, anchor: HTMLElement, inThread: boolean): void;
  openThread(rootId: string): void;
  prefetchThread(rootId: string): void;
  openHistory(messageId: string): void;
  openReply(reference: ChatReplyReference): void;
  toggleRun(first: string, shown: boolean): void;
  reply(message: ChatMessage, inThread: boolean): void;
  react(messageId: string, emoji: string, active: boolean): Promise<void>;
  showReactors(messageId: string, emoji: string, anchor: HTMLElement): void;
  dismissReactionError(messageId: string): void;
  viewFile(messageId: string, forwarded?: boolean): ViewAttachment;
  openConversation(message: ChatMessage, anchor: HTMLElement): void;
  refreshUrls(ids: string[]): void;
  /** A rejected pending send: back into the composer, or away. */
  editRejected(): void;
  discardRejected(): void;
  retrySend(): void;
}

/** Message text with links, and @mentions that open the person's profile card. */
export function MessageText({
  message,
  mentionNames,
  actions,
}: {
  message: ChatMessage;
  mentionNames: ReadonlyMap<string, string>;
  actions: RowActions;
}) {
  return mentionSegments(message.content.text, message.content.mentions).map((segment, part) => {
    if (!segment.mention) return <LinkedText key={part} text={segment.text} />;
    const user = segment.user;
    if (!user)
      return (
        <span
          key={part}
          className="chat-mention"
          title={
            feedbackAliases.some((alias) => segment.text.slice(1).toLowerCase() === alias)
              ? "Asks the Caper team to look at this conversation"
              : undefined
          }
        >
          {segment.text}
        </span>
      );
    return (
      <button
        type="button"
        key={part}
        className="chat-mention chat-mention-person"
        aria-haspopup="dialog"
        aria-label={`Open profile for ${mentionNames.get(user.id) ?? `@${user.username}`}`}
        onClick={(event) => {
          event.stopPropagation();
          actions.openMention(user, event.currentTarget);
        }}
      >
        {segment.text}
      </button>
    );
  });
}

export function PinMarker({
  author,
  viewerId,
  actions,
}: {
  author: ChatAuthor;
  viewerId?: string;
  actions: RowActions;
}) {
  return (
    <small className="chat-pin-marker">
      <Pin size={12} aria-hidden="true" />
      <button
        type="button"
        className="chat-pin-author"
        aria-label={`Open profile for ${author.name}`}
        aria-haspopup="dialog"
        onClick={(event) => actions.openAuthor(author, event.currentTarget)}
        onPointerEnter={(event) => {
          if (event.pointerType === "mouse" && !isTouchLayout()) actions.openAuthor(author, event.currentTarget, false);
        }}
        onPointerDown={(event) => {
          actions.cancelPress();
          if (event.pointerType === "mouse" || !event.isPrimary) return;
          actions.holdPress(event, (anchor) => actions.openAuthor(author, anchor));
        }}
        onPointerMove={actions.movePress}
        onPointerUp={actions.cancelPress}
        onPointerCancel={actions.cancelPress}
        onContextMenu={(event) => {
          if (isTouchLayout()) event.preventDefault();
        }}
      >
        Pinned by {author.id === viewerId ? "you" : author.name}
      </button>
    </small>
  );
}

export function AuthorAvatar({
  author,
  name,
  actions,
}: {
  author: ChatAuthor | undefined;
  name: string;
  actions: RowActions;
}) {
  const image = <Avatar person={author} name={author?.name ?? name} />;
  return author && !author.isGuest ? (
    <button
      type="button"
      className="chat-avatar chat-author-profile"
      aria-label={`Open profile for ${author.name}`}
      aria-haspopup="dialog"
      onClick={(event) => actions.openAuthor(author, event.currentTarget)}
    >
      {image}
    </button>
  ) : (
    <div className="chat-avatar">{image}</div>
  );
}

export function AuthorName({
  author,
  name,
  actions,
}: {
  author: ChatAuthor | undefined;
  name: string;
  actions: RowActions;
}) {
  return author && !author.isGuest ? (
    <button
      type="button"
      className="chat-author-name chat-author-profile"
      aria-label={`Open profile for ${author.name}`}
      aria-haspopup="dialog"
      onClick={(event) => actions.openAuthor(author, event.currentTarget)}
    >
      <strong>{author.name}</strong>
    </button>
  ) : (
    <strong>{author?.name ?? name}</strong>
  );
}

export function timeLabel(value: string, formatter: Intl.DateTimeFormat) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "" : formatter.format(date);
}

/** "8:28" for the narrow gutter beside compact rows (no AM/PM). */
const compactTime = (value: string, formatter: Intl.DateTimeFormat) =>
  formatter
    .formatToParts(new Date(value))
    .filter((part) => part.type !== "dayPeriod")
    .map((part) => part.value)
    .join("")
    .trim();

/**
 * Everything a row shows arrives as its own props: the conversation derives
 * each row's state (grouping, selection, unread, arrival) and passes values
 * that stay equal while that row is unchanged, so typing, presence and voice
 * activity elsewhere never re-render the rows.
 */
export interface MessageRowProps {
  message: ChatMessage | PendingChatMessage;
  inThread: boolean;
  /** A date divider above the row, already resolved. */
  divider?: string;
  /** The blocked run this message belongs to, by its first message. */
  runFirst?: string;
  runCount?: number;
  runShown: boolean;
  grouped: boolean;
  /** The row below joins this one. */
  continued: boolean;
  /** The channel's "New messages" divider sits above this row. */
  newMessages: boolean;
  /** Arrived while the reader watched the live end, so it animates in. */
  arriving: boolean;
  jumpTarget: boolean;
  threadActive: boolean;
  threadUnread: boolean;
  /** Which of this message's menus is open. */
  actionMode?: "actions" | "emoji";
  reactionSave?: ReactionSave;
  replyHidden: boolean;
  hydrated: boolean;
  /** Names an author-less row (the viewer's pending send before identity). */
  name: string;
  viewerId?: string;
  /** The chat author, when the viewer has one. */
  ownId?: string;
  readOnly: boolean;
  /** A send is pending; replies wait for it. */
  replyDisabled: boolean;
  channelId?: string;
  timeFormatter: Intl.DateTimeFormat;
  /** Profile-card display names by account, most specific first. */
  mentionNames: ReadonlyMap<string, string>;
  /** Only rows with files receive these. */
  attachmentProgress?: Record<string, number>;
  freshUrls?: FreshUrls;
  localPreviews?: Record<string, LocalPreview>;
  /** Only the channel's pending row receives its send status. */
  sendError?: string;
  sendUploadFailed?: boolean;
  sendRejected?: boolean;
  /** A draft in the composer: a rejected send can't be edited over it. */
  editBlocked?: boolean;
  actions: RowActions;
}

function MessageRowView({
  message,
  inThread,
  divider,
  runFirst,
  runCount = 0,
  runShown,
  grouped,
  continued,
  newMessages,
  arriving,
  jumpTarget,
  threadActive,
  threadUnread,
  actionMode,
  reactionSave,
  replyHidden,
  hydrated,
  name,
  viewerId,
  ownId,
  readOnly,
  replyDisabled,
  channelId,
  timeFormatter,
  mentionNames,
  attachmentProgress,
  freshUrls,
  localPreviews,
  sendError,
  sendUploadFailed,
  sendRejected,
  editBlocked,
  actions,
}: MessageRowProps) {
  const pending = !("content" in message);
  const deleted = !pending && !!message.deletedAt;
  const author = message.author;
  const canAct = !!ownId;
  const run = runFirst === undefined ? undefined : { first: runFirst, count: runCount };
  const edited =
    "content" in message && !message.forward && !message.deletedAt && (message.revision ?? 1) > 1 ? (
      <button
        type="button"
        className="chat-edited"
        title={message.editedAt ? `Edited ${new Date(message.editedAt).toLocaleString()}` : undefined}
        aria-label={`Message history, version ${message.revision}`}
        onClick={() => actions.openHistory(message.id)}
      >
        edited
      </button>
    ) : undefined;
  if (run && !runShown) {
    // Every message stays one list item, so history paging is unchanged.
    if (run.first !== message.clientMessageId) return <div className="chat-blocked-hidden" />;
    return (
      <div>
        {divider && (
          <div className="chat-date-divider">
            <time dateTime={message.createdAt}>{divider}</time>
          </div>
        )}
        <div className="chat-blocked-run">
          <Ban size={16} aria-hidden="true" />
          <span>{blockedLabel(run.count)}</span>
          <span aria-hidden="true">—</span>
          <button
            type="button"
            onClick={() => actions.toggleRun(run.first, true)}
            aria-label={`Show ${blockedLabel(run.count)}`}
            aria-expanded={false}
          >
            Show
          </button>
        </div>
      </div>
    );
  }
  const emojiOpen = actionMode === "emoji";
  return (
    <div data-motion-key={message.clientMessageId}>
      {divider && (!run || run.first === message.clientMessageId) && (
        <div className="chat-date-divider">
          <time dateTime={message.createdAt}>{divider}</time>
        </div>
      )}
      {newMessages && (
        <div className="chat-date-divider chat-new-messages" role="separator" aria-label="New messages">
          <span>New messages</span>
        </div>
      )}
      <div className={run ? "chat-blocked-revealed" : undefined}>
        {divider && run && run.first !== message.clientMessageId && (
          <div className="chat-date-divider">
            <time dateTime={message.createdAt}>{divider}</time>
          </div>
        )}
        {run?.first === message.clientMessageId && (
          <div className="chat-blocked-run">
            <Ban size={16} aria-hidden="true" />
            <span>{blockedLabel(run.count)}</span>
            <span aria-hidden="true">—</span>
            <button
              type="button"
              onClick={() => actions.toggleRun(run.first, false)}
              aria-label={`Hide ${blockedLabel(run.count)}`}
              aria-expanded={true}
            >
              Hide
            </button>
          </div>
        )}
        <article
          className={`chat-message${grouped ? " chat-message-grouped" : ""}${continued ? " chat-message-continued" : ""}${pending ? " chat-message-pending" : ""}${!pending && message.pin ? ` chat-message-pinned${message.pin.author.id === viewerId ? " chat-message-pinned-own" : ""}` : ""}${arriving ? " chat-message-arrive" : ""}${jumpTarget ? " chat-message-jump-target" : ""}${!pending && !message.pin && mentionsAccount(message, ownId) ? " chat-message-mentioned" : ""}${threadActive ? " chat-message-thread-active" : ""}`}
          data-message-key={message.clientMessageId}
          data-message-id={"id" in message ? message.id : undefined}
          data-reply-to-me={
            !pending && !deleted && !!viewerId && message.replyTo?.author.id === viewerId && author?.id !== viewerId
          }
          onPointerDown={(event) => {
            actions.cancelPress();
            if (
              !("content" in message) ||
              message.deletedAt ||
              event.pointerType === "mouse" ||
              !event.isPrimary ||
              (event.target as HTMLElement).closest("button, a")
            )
              return;
            // Keep transient feedback on this exact row, without rerendering
            // the virtualized timeline (or highlighting its channel/thread copy).
            actions.holdPress(
              event,
              (anchor) => {
                window.getSelection()?.removeAllRanges();
                actions.openActions(message.id, anchor, inThread);
              },
              true,
            );
          }}
          onPointerMove={actions.movePress}
          onPointerUp={actions.cancelPress}
          onPointerCancel={actions.cancelPress}
          onContextMenu={(event) => {
            if (
              !("content" in message) ||
              message.deletedAt ||
              !isTouchLayout() ||
              (event.target as HTMLElement).closest("button, a")
            )
              return;
            event.preventDefault();
            actions.cancelPress();
            actions.suppressNextClick();
            actions.openActions(message.id, event.currentTarget, inThread);
          }}
          tabIndex={pending ? undefined : -1}
        >
          {!pending && !deleted && message.pin && (
            <PinMarker author={message.pin.author} viewerId={viewerId} actions={actions} />
          )}
          {message.replyTo && !deleted && (
            <ReplyContext
              motionKey={message.clientMessageId}
              reference={message.replyTo}
              hidden={replyHidden}
              onOpen={() => actions.openReply(message.replyTo!)}
            />
          )}
          {grouped ? (
            // The run's avatar is above; show the time here on hover instead.
            <div className="chat-grouped-time" aria-hidden="true">
              {compactTime(message.createdAt, timeFormatter)}
            </div>
          ) : (
            <AuthorAvatar author={author} name={name} actions={actions} />
          )}
          <div>
            <header className={grouped ? "sr-only" : undefined}>
              {grouped ? (
                <strong>{author?.name ?? name}</strong>
              ) : (
                <AuthorName author={author} name={name} actions={actions} />
              )}
              {author?.admin && <AdminBadge />}
              {author?.isGuest && <span>Guest</span>}
              <time dateTime={message.createdAt}>{hydrated ? timeLabel(message.createdAt, timeFormatter) : ""}</time>
              {!grouped && edited}
            </header>
            {!inThread && message.threadRootId && (
              <button
                type="button"
                className="chat-thread-context"
                onMouseEnter={() => actions.prefetchThread(message.threadRootId!)}
                onFocus={() => actions.prefetchThread(message.threadRootId!)}
                onClick={() => actions.openThread(message.threadRootId!)}
              >
                Replied to a thread · View thread
              </button>
            )}
            {deleted && <p className="chat-message-deleted">This message was deleted.</p>}
            {(!("content" in message) || message.content.text) && (
              <p>
                {"content" in message ? (
                  <MessageText message={message} mentionNames={mentionNames} actions={actions} />
                ) : (
                  message.text
                )}
                {/* Compact rows have no header, so the marker follows the text. */}
                {grouped && edited && <> {edited}</>}
              </p>
            )}
            {!deleted && <MessageGif content={"content" in message ? message.content : message} />}
            <MessageAttachments
              attachments={
                "content" in message ? withFreshUrls(attachmentsOf(message), freshUrls) : (message.attachments ?? [])
              }
              progress={attachmentProgress}
              localPreviews={localPreviews}
              onExpired={pending ? undefined : actions.refreshUrls}
              onView={"content" in message ? actions.viewFile(message.id) : undefined}
            />
            {"content" in message && <MessageEmbeds embeds={embedsOf(message)} />}
            {"content" in message && reportsFeedback(message) && (
              <p className="chat-feedback-note">Caper team has been notified 🫡</p>
            )}
            {"content" in message && (
              <>
                {!deleted && (
                  <ForwardCard
                    message={message}
                    onOpen={(anchor) => actions.openConversation(message, anchor)}
                    onView={actions.viewFile(message.id, true)}
                  />
                )}
                <div
                  className="chat-message-tools"
                  role="group"
                  aria-label={`Actions for ${message.author.name}'s message`}
                >
                  {!readOnly &&
                    canAct &&
                    viewerId &&
                    !deleted &&
                    quickReactions.map((emoji) => {
                      const mine = !!message.reactions?.some(
                        (reaction) => reaction.emoji === emoji && reaction.authorIds.includes(viewerId),
                      );
                      return (
                        <button
                          type="button"
                          key={emoji}
                          className="chat-quick-reaction"
                          aria-label={`React with ${emoji}`}
                          title={`React with ${emoji}`}
                          aria-pressed={mine}
                          onClick={() => void actions.react(message.id, emoji, !mine)}
                        >
                          <img src={emojiAsset(emojiCode(emoji))} width={18} height={18} alt="" />
                        </button>
                      );
                    })}
                  {!inThread && (
                    <button
                      type="button"
                      className="chat-reply-thread"
                      aria-label={`Reply in thread to ${message.author.name}`}
                      title="Reply in thread"
                      onMouseEnter={() => actions.prefetchThread(message.threadRootId ?? message.id)}
                      onFocus={() => actions.prefetchThread(message.threadRootId ?? message.id)}
                      onClick={() => actions.openThread(message.threadRootId ?? message.id)}
                    >
                      <Thread size={14} aria-hidden="true" />
                    </button>
                  )}
                  {!readOnly && canAct && !deleted && (
                    <button
                      type="button"
                      className="chat-reply-message"
                      aria-label={`Reply to ${message.author.name}`}
                      title="Reply"
                      disabled={replyDisabled}
                      onClick={() => actions.reply(message, inThread)}
                    >
                      <Reply size={14} aria-hidden="true" />
                    </button>
                  )}
                  {!deleted && (
                    <>
                      <button
                        type="button"
                        className="chat-add-reaction"
                        onClick={(event) => actions.toggleReactionPicker(message.id, event.currentTarget, inThread)}
                        onMouseEnter={() => {
                          if (viewerId && !readOnly && canAct) preloadReactionPicker();
                        }}
                        onFocus={preloadReactionPicker}
                        disabled={!viewerId || readOnly || !canAct}
                        aria-label="Add reaction"
                        title="Add reaction"
                        aria-haspopup="dialog"
                        aria-expanded={emojiOpen}
                      >
                        <SmilePlus size={14} aria-hidden="true" />
                      </button>
                      <button
                        type="button"
                        className="chat-message-actions-trigger"
                        aria-label={`Message actions for ${message.author.name}`}
                        title="More actions"
                        aria-haspopup="dialog"
                        aria-expanded={actionMode === "actions"}
                        onClick={(event) => actions.toggleActions(message.id, event.currentTarget, inThread)}
                      >
                        <MoreHorizontal size={14} aria-hidden="true" />
                      </button>
                    </>
                  )}
                </div>
                {!deleted && (
                  <MessageReactions
                    message={message}
                    channelId={channelId}
                    authorId={viewerId}
                    readOnly={readOnly || !canAct}
                    save={reactionSave}
                    onReact={actions.react}
                    onShowReactors={(emoji, anchor) => actions.showReactors(message.id, emoji, anchor)}
                    pickerOpen={emojiOpen}
                    onAddReaction={(anchor) => {
                      preloadReactionPicker();
                      actions.toggleReactionPicker(message.id, anchor, inThread);
                    }}
                    onDismissError={() => actions.dismissReactionError(message.id)}
                  />
                )}
                {!inThread && !message.threadRootId && !!message.thread?.replyCount && (
                  <button
                    type="button"
                    className="chat-thread-summary"
                    onMouseEnter={() => actions.prefetchThread(message.id)}
                    onFocus={() => actions.prefetchThread(message.id)}
                    onPointerDown={(event) => {
                      if (event.pointerType === "mouse" || !event.isPrimary || event.button !== 0) return;
                      event.currentTarget.dataset.pressing = "true";
                      event.currentTarget.setPointerCapture(event.pointerId);
                    }}
                    onPointerUp={(event) => delete event.currentTarget.dataset.pressing}
                    onPointerCancel={(event) => delete event.currentTarget.dataset.pressing}
                    onLostPointerCapture={(event) => delete event.currentTarget.dataset.pressing}
                    onContextMenu={(event) => event.preventDefault()}
                    onClick={() => actions.openThread(message.id)}
                    aria-label={`View thread with ${message.thread.replyCount} ${message.thread.replyCount === 1 ? "reply" : "replies"}${threadUnread ? ", unread" : ""}`}
                  >
                    <span className="chat-thread-avatars">
                      {message.thread.participants.map((person) => (
                        <span key={person.id} title={person.name}>
                          <Avatar person={person} name={person.name} />
                        </span>
                      ))}
                    </span>
                    <strong>
                      {message.thread.replyCount} {message.thread.replyCount === 1 ? "reply" : "replies"}
                    </strong>
                    {threadUnread && <span className="chat-thread-unread">Unread</span>}
                    <span>View thread</span>
                  </button>
                )}
              </>
            )}
            {/* A pending thread reply shows its status beside the thread composer. */}
            {pending && !inThread && sendError && (
              <div className="chat-send-status chat-send-error" role="alert">
                <span>
                  {!sendUploadFailed && (sendRejected ? "Not sent. " : "Not confirmed yet. ")}
                  {sendError}
                </span>
                {sendRejected ? (
                  <>
                    <button
                      type="button"
                      disabled={editBlocked}
                      title={editBlocked ? "Clear your current draft to edit this message." : undefined}
                      onClick={actions.editRejected}
                    >
                      Edit
                    </button>
                    <button type="button" onClick={actions.discardRejected}>
                      Dismiss
                    </button>
                  </>
                ) : (
                  <button type="button" onClick={actions.retrySend}>
                    {sendUploadFailed ? "Retry" : "Retry send"}
                  </button>
                )}
              </div>
            )}
          </div>
        </article>
      </div>
    </div>
  );
}

/** One conversation row, re-rendered only when one of its own props changes. */
const MessageRow = memo(MessageRowView);
export default MessageRow;
