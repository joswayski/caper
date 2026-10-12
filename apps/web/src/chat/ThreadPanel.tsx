import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, Send, X } from "lucide-react";
import Avatar from "../components/Avatar";
import MessageSkeleton from "./MessageSkeleton.tsx";
import type { ChatClient, ChatViewState } from "./client.ts";
import { COUNTER_START, counterTone } from "./counter.ts";
import { useComposerSuggestions } from "./ComposerSuggestions.tsx";
import { readThreadDrafts, saveThreadDrafts, type ThreadDraft } from "./drafts.ts";
import type { Arrivals } from "./arrivals.ts";
import EmojiButton from "./EmojiButton.tsx";
import { insertEmoji } from "./emoji-autocomplete.ts";
import MentionTextarea from "./MentionTextarea.tsx";
import type { MentionCandidate } from "./mentions.ts";
import { sequence, type ChatMessage, type ChatReplyReference } from "./types.ts";
import { ReplyContext, ReplyDraft } from "./Replies.tsx";

const DEFAULT_WIDTH = 340;
const MIN_WIDTH = 300;
const MAX_WIDTH = 600;
const WIDTH_KEY = "caper:thread-panel-width";

export default function ThreadPanel({
  state,
  client,
  channelName,
  direct = false,
  readOnly,
  mentionPeople,
  specialMentions,
  replyTo,
  onReplyChange,
  blockedIds,
  renderMessage,
  arrivals,
  onClose,
}: {
  state: ChatViewState;
  client?: ChatClient;
  channelName: string;
  direct?: boolean;
  readOnly: boolean;
  /** People `@` can suggest, without the author; undefined until loaded. */
  mentionPeople?: MentionCandidate[];
  specialMentions: boolean;
  replyTo?: ChatReplyReference;
  onReplyChange: (reference?: ChatReplyReference) => void;
  blockedIds: ReadonlySet<string>;
  renderMessage: (
    index: number,
    message: ChatMessage,
    inThread: boolean,
    previous?: ChatMessage,
    next?: ChatMessage,
  ) => ReactNode;
  /** Shared with the channel list, whose rows render through `renderMessage`. */
  arrivals: Arrivals;
  onClose: () => void;
}) {
  // Kept in the page-wide store, so switching conversations never drops a reply.
  const [drafts, setDrafts] = useState<Record<string, ThreadDraft>>(readThreadDrafts);
  useEffect(() => saveThreadDrafts(drafts), [drafts]);
  const [validation, setValidation] = useState<string>();
  const composer = useRef<HTMLTextAreaElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const olderAnchor = useRef<{ height: number; top: number }>(undefined);
  const follow = useRef(true);
  const returnFocus = useRef<HTMLElement>(null);
  const panel = useRef<HTMLElement>(null);
  const [mobile, setMobile] = useState(false);
  const [preferredWidth, setPreferredWidth] = useState(DEFAULT_WIDTH);
  const [maximumWidth, setMaximumWidth] = useState(MAX_WIDTH);
  const minimumWidth = Math.min(MIN_WIDTH, maximumWidth);
  const width = Math.max(minimumWidth, Math.min(maximumWidth, preferredWidth));
  const drag = useRef<{ x: number; width: number; scale: number; last?: number } | undefined>(undefined);
  const rootId = state.thread?.rootId;
  const open = !!rootId;
  useLayoutEffect(() => {
    try {
      const saved = Number(localStorage.getItem(WIDTH_KEY));
      if (saved >= MIN_WIDTH && saved <= MAX_WIDTH) setPreferredWidth(saved);
    } catch {
      /* Optional preference; resizing still works when storage is unavailable. */
    }
  }, []);
  useLayoutEffect(() => {
    if (!open || !panel.current) return;
    const room = panel.current.parentElement!;
    const measure = () => {
      // Reserve 300px for the conversation, or half the room in smaller split layouts.
      setMaximumWidth(Math.floor(Math.min(MAX_WIDTH, room.clientWidth - Math.min(MIN_WIDTH, room.clientWidth / 2))));
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(room);
    return () => observer.disconnect();
  }, [open]);
  const saveWidth = (value: number) => {
    try {
      localStorage.setItem(WIDTH_KEY, String(Math.max(MIN_WIDTH, value)));
    } catch {
      /* Optional preference. */
    }
  };
  const resize = (value: number, persist = true) => {
    const next = Math.round(Math.max(minimumWidth, Math.min(maximumWidth, value)));
    setPreferredWidth(Math.max(MIN_WIDTH, next));
    if (persist) saveWidth(next);
    return next;
  };
  const endDrag = () => {
    if (drag.current?.last !== undefined) saveWidth(drag.current.last);
    drag.current = undefined;
  };
  const pending = rootId && state.pendingSend?.threadRootId === rootId ? state.pendingSend : undefined;
  const windowStart = state.thread?.windowStart;
  const windowEnd = state.thread?.windowEnd;
  const previousReplies = useRef<ChatMessage[]>([]);
  const replies = useMemo(() => {
    const next = rootId
      ? state.messages.filter(
          (message) =>
            message.threadRootId === rootId &&
            (!windowStart || sequence(message.seq) >= sequence(windowStart)) &&
            (!windowEnd || sequence(message.seq) <= sequence(windowEnd)),
        )
      : [];
    // Timeline rows are immutable, so keep the array when no reply changed: unrelated
    // channel events then neither re-render the replies nor force a follow layout read.
    const previous = previousReplies.current;
    if (next.length === previous.length && next.every((message, index) => message === previous[index])) return previous;
    return (previousReplies.current = next);
  }, [state.messages, rootId, windowStart, windowEnd]);
  const draft = rootId ? (drafts[rootId] ?? { text: "", broadcast: false }) : { text: "", broadcast: false };
  const update = (change: Partial<typeof draft>) => {
    if (rootId) setDrafts((current) => ({ ...current, [rootId]: { ...draft, ...change } }));
    setValidation(undefined);
  };
  const chooseEmoji = (emoji: string) => {
    const input = composer.current;
    const start = input?.selectionStart ?? draft.text.length;
    const result = insertEmoji(draft.text, { start, end: input?.selectionEnd ?? start, query: "" }, emoji);
    if (!result) {
      setValidation("Messages must be 4,000 characters or fewer.");
      return;
    }
    update({ text: result.value });
    requestAnimationFrame(() => {
      input?.focus();
      input?.setSelectionRange(result.caret, result.caret);
    });
  };
  const suggestions = useComposerSuggestions({
    id: "chat-thread",
    draft: draft.text,
    input: composer,
    people: mentionPeople,
    specialMentions,
    onInsert: (text) => update({ text }),
    onTooLong: () => setValidation("Messages must be 4,000 characters or fewer."),
  });
  useEffect(() => {
    const query = window.matchMedia("(max-width: 760px)");
    const change = () => setMobile(query.matches);
    change();
    query.addEventListener("change", change);
    return () => query.removeEventListener("change", change);
  }, []);
  useEffect(() => {
    if (!rootId || !mobile || !panel.current) return;
    const backgrounds = new Map<HTMLElement, boolean>();
    let child: HTMLElement = panel.current;
    while (child.parentElement && child.parentElement !== document.body) {
      for (const sibling of child.parentElement.children) {
        if (sibling !== child && sibling instanceof HTMLElement) {
          backgrounds.set(sibling, sibling.inert);
          sibling.inert = true;
        }
      }
      child = child.parentElement;
    }
    return () => {
      for (const [element, inert] of backgrounds) element.inert = inert;
    };
  }, [rootId, mobile]);
  useEffect(() => {
    if (!open || !mobile) return;
    const key = crypto.randomUUID();
    window.history.pushState({ ...window.history.state, caperThread: key }, "");
    const back = () => {
      if (window.history.state?.caperThread !== key) onClose();
    };
    window.addEventListener("popstate", back);
    return () => {
      window.removeEventListener("popstate", back);
      if (window.history.state?.caperThread === key) window.history.back();
    };
  }, [open, mobile, onClose]);
  useEffect(() => {
    setValidation(undefined);
    if (!rootId) return;
    returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    olderAnchor.current = undefined;
    // A cached "go to message" page keeps the reader at that reply.
    follow.current = !state.thread?.focusMessageId;
    composer.current?.focus();
    const escape = (event: KeyboardEvent) => {
      // Escape closes the innermost layer first: a menu, popover or dialog
      // over the thread (including sidebar menus and space dialogs) keeps it open.
      if (
        event.key === "Escape" &&
        !event.defaultPrevented &&
        // Escape that cancels an input-method composition must not also close the thread.
        !event.isComposing &&
        !document.querySelector(
          ".chat-reaction-picker, .chat-message-actions, .chat-reactors, .chat-mention-card, .chat-forward-dialog, dialog[open], details[open], [popover]:popover-open",
        )
      )
        onClose();
    };
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("keydown", escape);
      if (returnFocus.current?.isConnected) returnFocus.current.focus();
    };
  }, [rootId, onClose]);
  useEffect(() => {
    if (rootId && !state.thread?.loading && !document.querySelector(".chat-edit-dialog[open]"))
      composer.current?.focus();
  }, [rootId, state.thread?.loading]);
  useEffect(() => {
    if (!pending) return;
    setDrafts((current) => {
      const old = current[pending.threadRootId!] ?? { text: "", broadcast: false };
      // "Also send to channel" applies to one reply, so it resets with the draft.
      return old.text === pending.text
        ? { ...current, [pending.threadRootId!]: { text: "", broadcast: false } }
        : current;
    });
  }, [pending?.clientMessageId]);
  useEffect(() => {
    if (!state.thread?.focusMessageId) follow.current = true;
  }, [state.thread?.focusMessageId]);
  // Reactions grow an existing reply (or the root) without changing its ID.
  const root = useMemo(() => state.messages.find((message) => message.id === rootId), [state.messages, rootId]);
  // Runs before the follow effect so a focused reply is not first scrolled past.
  // The focus stays through paging, so scrolling back to the bottom (or
  // sending) is what resumes following.
  useLayoutEffect(() => {
    const id = state.thread?.focusMessageId;
    if (!id || state.thread?.loading) return;
    follow.current = false;
    const frame = requestAnimationFrame(() =>
      scroll.current?.querySelector(`[data-message-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center" }),
    );
    return () => cancelAnimationFrame(frame);
  }, [state.thread?.focusMessageId, state.thread?.loading]);
  useLayoutEffect(() => {
    if (scroll.current && follow.current && !state.thread?.hasNewer)
      scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [rootId, replies, root, pending?.clientMessageId, state.thread?.loading]);
  useLayoutEffect(() => {
    if (state.thread?.loadingOlder || !olderAnchor.current || !scroll.current) return;
    scroll.current.scrollTop = olderAnchor.current.top + scroll.current.scrollHeight - olderAnchor.current.height;
    olderAnchor.current = undefined;
  }, [state.thread?.loadingOlder, replies.length]);
  arrivals.observe(
    "thread",
    rootId ?? "",
    pending
      ? [...replies.map((message) => message.clientMessageId), pending.clientMessageId]
      : replies.map((message) => message.clientMessageId),
    !!state.thread && !state.thread.loading && !state.thread.hasNewer,
  );
  // Typing a reply changes only this panel's draft; keep the rendered messages.
  const renderedRoot = useMemo(() => root && renderMessage(0, root, true), [root, renderMessage]);
  const renderedReplies = useMemo(
    // Replies group with the reply above them; the root never groups.
    () =>
      replies.map((message, index) => renderMessage(index + 1, message, true, replies[index - 1], replies[index + 1])),
    [replies, renderMessage],
  );
  if (!state.thread) return null;
  const count = Array.from(draft.text).length;
  const sending = !!state.pendingSend && !state.sendError;
  const blocked = !!state.pendingSend && !pending;
  const submit = async () => {
    if (!client || readOnly || blocked || sending || state.sendRejected) return;
    try {
      await client.send(pending?.text ?? draft.text, { threadRootId: rootId, broadcast: draft.broadcast, replyTo });
      follow.current = true;
    } catch (error) {
      setValidation(error instanceof Error ? error.message : "Reply could not be sent.");
    }
  };
  return (
    <aside
      ref={panel}
      className="chat-thread-panel"
      id="chat-thread-panel"
      style={{ flexBasis: width }}
      aria-labelledby="chat-thread-heading"
      role={mobile ? "dialog" : undefined}
      aria-modal={mobile || undefined}
    >
      <div
        className="chat-thread-resize"
        role="separator"
        aria-label="Thread width"
        aria-controls="chat-thread-panel"
        aria-orientation="vertical"
        aria-valuemin={minimumWidth}
        aria-valuemax={maximumWidth}
        aria-valuenow={width}
        aria-valuetext={`${width} pixels`}
        tabIndex={mobile ? -1 : 0}
        title="Drag to resize. Arrow keys to adjust. Double-click to reset."
        onPointerDown={(event) => {
          if (event.button !== 0 || !event.isPrimary) return;
          event.preventDefault();
          event.currentTarget.focus();
          const element = panel.current!;
          drag.current = {
            x: event.clientX,
            width: element.offsetWidth,
            scale: element.getBoundingClientRect().width / element.offsetWidth,
          };
          event.currentTarget.setPointerCapture(event.pointerId);
        }}
        onPointerMove={(event) => {
          if (drag.current)
            drag.current.last = resize(
              drag.current.width + (drag.current.x - event.clientX) / drag.current.scale,
              false,
            );
        }}
        onPointerUp={(event) => {
          endDrag();
          if (event.currentTarget.hasPointerCapture(event.pointerId))
            event.currentTarget.releasePointerCapture(event.pointerId);
        }}
        onPointerCancel={endDrag}
        onLostPointerCapture={endDrag}
        onDoubleClick={() => resize(DEFAULT_WIDTH)}
        onKeyDown={(event) => {
          const sizes: Record<string, number> = {
            ArrowLeft: width + 10,
            ArrowRight: width - 10,
            Home: minimumWidth,
            End: maximumWidth,
          };
          const next = sizes[event.key];
          if (next === undefined) return;
          event.preventDefault();
          resize(next);
        }}
      />
      <header className="chat-thread-heading">
        <button
          type="button"
          className="chat-thread-back"
          onClick={onClose}
          aria-label={direct ? "Back to conversation" : "Back to channel"}
        >
          <ArrowLeft size={20} />
        </button>
        <div>
          <h2 id="chat-thread-heading">Thread</h2>
          <span>{direct ? `with ${channelName}` : `in #${channelName}`}</span>
        </div>
        <button type="button" className="chat-thread-close" onClick={onClose} aria-label="Close thread">
          <X size={20} />
        </button>
      </header>
      <div
        className="chat-thread-messages"
        ref={scroll}
        role="region"
        aria-label="Thread replies"
        aria-busy={state.thread.loading}
        onScroll={(event) => {
          const element = event.currentTarget;
          follow.current = element.scrollHeight - element.scrollTop - element.clientHeight < 80;
        }}
      >
        {root && <div className="chat-thread-parent">{renderedRoot}</div>}
        {!!root?.thread?.replyCount && (
          <div className="chat-thread-divider">
            {root.thread.replyCount} {root.thread.replyCount === 1 ? "reply" : "replies"}
          </div>
        )}
        {state.thread.loading && !replies.length && (
          <MessageSkeleton
            label="Loading thread replies"
            rows={Math.min(3, Math.max(1, root?.thread?.replyCount ?? 2))}
          />
        )}
        {state.thread.error && (
          <p className="chat-thread-status chat-inline-error" role="alert">
            {state.thread.error}{" "}
            <button
              type="button"
              onClick={() => void (state.thread?.hasMore ? client?.loadOlderThread() : client?.retryThread())}
            >
              Retry
            </button>
          </p>
        )}
        {state.thread.hasMore && (
          <button
            type="button"
            className="chat-thread-older"
            disabled={state.thread.loadingOlder}
            onClick={() => {
              follow.current = false;
              if (scroll.current)
                olderAnchor.current = { height: scroll.current.scrollHeight, top: scroll.current.scrollTop };
              void client?.loadOlderThread();
            }}
          >
            {state.thread.loadingOlder ? "Loading…" : "Load older replies"}
          </button>
        )}
        {!state.thread.loading && !state.thread.error && !replies.length && (
          <p className="chat-thread-status">{readOnly ? "No replies yet." : "No replies yet. Start the thread."}</p>
        )}
        {renderedReplies}
        {state.thread.hasNewer && (
          <button
            type="button"
            className="chat-thread-older"
            disabled={state.thread.loadingNewer}
            onClick={() => void client?.loadNewerThread()}
          >
            {state.thread.loadingNewer ? "Loading…" : "Load newer replies"}
          </button>
        )}
        {pending && (
          <article className="chat-message chat-message-pending chat-message-arrive">
            {pending.replyTo && (
              <ReplyContext
                reference={pending.replyTo}
                hidden={blockedIds.has(pending.replyTo.author.id) && pending.replyTo.author.id !== state.author?.id}
                onOpen={() => void client?.loadMessageContext({ ...pending.replyTo!, channelId: state.channelId! })}
              />
            )}
            <div className="chat-avatar">
              <Avatar avatarId={pending.author?.avatarId} name={pending.author?.name ?? ""} />
            </div>
            <div>
              <header>
                <strong>{pending.author?.name}</strong>
              </header>
              <p>{pending.text}</p>
            </div>
          </article>
        )}
      </div>
      <div className="chat-composer chat-thread-composer">
        {readOnly ? (
          <p className="chat-thread-readonly">
            {direct ? "You can’t reply in this conversation." : "Join the channel to reply."}
          </p>
        ) : (
          <>
            {replyTo && (
              <ReplyDraft
                reference={replyTo}
                hidden={blockedIds.has(replyTo.author.id) && replyTo.author.id !== state.author?.id}
                onCancel={() => onReplyChange(undefined)}
              />
            )}
            {state.sessionError && (
              <p className="chat-inline-error" role="alert">
                {state.sessionError}{" "}
                <button type="button" onClick={() => client?.retrySession()}>
                  Retry session
                </button>
              </p>
            )}
            {validation && (
              <p className="chat-inline-error" role="alert">
                {validation}
              </p>
            )}
            {pending && state.sendError && (
              <p className="chat-inline-error" role="alert">
                {state.sendRejected ? "Not sent." : "Not confirmed yet."} {state.sendError}{" "}
                {state.sendRejected ? (
                  <>
                    <button
                      type="button"
                      disabled={!!draft.text}
                      onClick={() => {
                        const broadcast = pending.broadcast ?? false;
                        onReplyChange(pending.replyTo);
                        const text = client?.discardRejected();
                        if (text !== undefined) update({ text, broadcast });
                      }}
                    >
                      Edit
                    </button>
                    <button type="button" onClick={() => client?.discardRejected()}>
                      Dismiss
                    </button>
                  </>
                ) : (
                  <button type="button" onClick={() => void submit()}>
                    Retry send
                  </button>
                )}
              </p>
            )}
            {blocked && state.sendError && (
              <p className="chat-inline-error">Confirm or dismiss the pending message before sending a reply.</p>
            )}
            <form
              onSubmit={(event) => {
                event.preventDefault();
                void submit();
              }}
            >
              <label className="sr-only" htmlFor="chat-thread-reply">
                Reply to thread
              </label>
              {suggestions.popup}
              <MentionTextarea
                ref={composer}
                id="chat-thread-reply"
                people={mentionPeople}
                specialMentions={specialMentions}
                rows={3}
                value={draft.text}
                placeholder="Reply to thread…"
                enterKeyHint="send"
                aria-describedby="chat-thread-composer-hint"
                {...suggestions.textarea}
                disabled={state.phase !== "ready" || state.thread.loading}
                onChange={(event) => {
                  update({ text: event.target.value });
                  suggestions.change(event.target);
                }}
                onBlur={suggestions.blur}
                onKeyDown={(event) => {
                  if (suggestions.keyDown(event)) return;
                  if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    void submit();
                  }
                }}
              />
              <span id="chat-thread-composer-hint" className="sr-only">
                Type : to find emoji or @ to mention someone. Up and Down choose; Enter or Tab inserts; Escape closes
                suggestions. Enter to send. Shift+Enter for a new line.
              </span>
              <div className="chat-thread-send-row">
                <label>
                  <input
                    type="checkbox"
                    checked={pending?.broadcast ?? draft.broadcast}
                    disabled={!!pending}
                    onChange={(event) => update({ broadcast: event.target.checked })}
                  />
                  {direct ? "Also send to conversation" : `Also send to #${channelName}`}
                </label>
                <EmojiButton disabled={state.phase !== "ready" || state.thread.loading} onSelect={chooseEmoji} />
                <button
                  type="submit"
                  disabled={
                    sending || blocked || state.sendRejected || state.thread.loading || (!pending && !draft.text.trim())
                  }
                  aria-label="Send reply"
                >
                  <Send size={18} />
                </button>
              </div>
              {count >= COUNTER_START && (
                <small className="chat-counter" data-tone={counterTone(count)}>
                  {count.toLocaleString()} / 4,000
                </small>
              )}
            </form>
          </>
        )}
      </div>
    </aside>
  );
}
