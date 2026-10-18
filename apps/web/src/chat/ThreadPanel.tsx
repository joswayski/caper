import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, Send, X } from "lucide-react";
import AttachmentPicker from "./AttachmentPicker.tsx";
import Avatar from "../components/Avatar";
import MessageSkeleton from "./MessageSkeleton.tsx";
import type { ChatClient, ChatViewState } from "./client.ts";
import { DraftAttachments, MessageAttachments, type LocalPreview } from "./Attachments.tsx";
import type { ComposerAttachments } from "./Composer.tsx";
import DropOverlay from "./DropOverlay.tsx";
import { carriesFiles } from "./drop.ts";
import { MAX_ATTACHMENTS } from "./uploads.ts";
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
import { useTimelineKeyboardDismiss } from "./useTimelineKeyboardDismiss.ts";

const DEFAULT_WIDTH = 340;
const MIN_WIDTH = 300;
const MAX_WIDTH = 600;
const WIDTH_KEY = "caper:thread-panel-width";
/** Phones and tablets: opening a thread to read it must not raise the keyboard (Reply focuses on purpose). */
const touchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;

export default function ThreadPanel({
  state,
  client,
  channelName,
  direct = false,
  readOnly,
  readActive = true,
  mentionPeople,
  specialMentions,
  replyTo,
  attachments,
  localPreviews,
  onSend,
  onReplyChange,
  onOpenReference,
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
  readActive?: boolean;
  /** People `@` can suggest, without the author; undefined until loaded. */
  mentionPeople?: MentionCandidate[];
  specialMentions: boolean;
  replyTo?: ChatReplyReference;
  attachments?: ComposerAttachments;
  localPreviews: Record<string, LocalPreview>;
  onSend: (text: string, broadcast: boolean) => Promise<unknown>;
  onReplyChange: (reference?: ChatReplyReference) => void;
  /** Jumps to a pending reply's original, like the replies rendered by `renderMessage`. */
  onOpenReference: (reference: ChatReplyReference) => void;
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
  const keyboardDismiss = useTimelineKeyboardDismiss("chat-thread-reply");
  const canAttach = !!attachments?.enabled;
  const dragDepth = useRef(0);
  const [dropping, setDropping] = useState(false);
  const endDrop = () => {
    dragDepth.current = 0;
    setDropping(false);
  };
  const scroll = useRef<HTMLDivElement>(null);
  const olderAnchor = useRef<{ height: number; top: number }>(undefined);
  const follow = useRef(true);
  const returnFocus = useRef<HTMLElement>(null);
  const panel = useRef<HTMLElement>(null);
  const [mobile, setMobile] = useState(false);
  const backSwipe = useRef<{ id: number; x: number; y: number; claimed: boolean } | undefined>(undefined);
  const suppressSwipeClick = useRef(false);
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
  // Start at one line like the channel composer, growing for typed or wrapped text.
  useLayoutEffect(() => {
    const input = composer.current;
    if (!input) return;
    const resizeInput = () => {
      const form = input.closest("form");
      if (form) form.style.minHeight = `${form.offsetHeight}px`;
      input.style.height = "0px";
      input.style.height = `${input.scrollHeight + input.offsetHeight - input.clientHeight}px`;
      if (form) form.style.minHeight = "";
      if (follow.current && scroll.current && !state.thread?.hasNewer)
        scroll.current.scrollTop = scroll.current.scrollHeight;
    };
    resizeInput();
    let width = input.clientWidth;
    const observer = new ResizeObserver(() => {
      if (input.clientWidth === width) return;
      width = input.clientWidth;
      resizeInput();
    });
    observer.observe(input);
    return () => observer.disconnect();
  }, [draft.text, rootId, readOnly, state.thread?.hasNewer]);
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
    endDrop();
    if (!rootId) return;
    returnFocus.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    olderAnchor.current = undefined;
    // A cached "go to message" page keeps the reader at that reply.
    follow.current = !state.thread?.focusMessageId;
    if (!touchLayout()) composer.current?.focus();
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
    if (rootId && !state.thread?.loading && !touchLayout() && !document.querySelector(".chat-edit-dialog[open]"))
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
  useEffect(() => {
    // Measure the newest unread reply, not a later own or collapsed blocked row.
    const latest = replies.find((message) => message.seq === (rootId ? state.unreadThreads?.[rootId] : undefined));
    const list = scroll.current;
    if (!readActive || !rootId || !latest || !list || state.thread?.loading || state.thread?.hasNewer) return;
    const row = list.querySelector<HTMLElement>(`[data-message-id="${CSS.escape(latest.id)}"]`);
    if (!row) return;
    const read = () => {
      if (document.visibilityState !== "visible" || !document.hasFocus()) return;
      const viewport = list.getBoundingClientRect(),
        bounds = row.getBoundingClientRect();
      const bottom = bounds.bottom - 1;
      if (
        bounds.height <= 0 ||
        bottom < Math.max(viewport.top, 0) ||
        bounds.bottom > Math.min(viewport.bottom, window.innerHeight)
      )
        return;
      // A covering dialog, mobile navigation, or hidden panel is not being read.
      const visible = document.elementFromPoint(bounds.left + bounds.width / 2, bottom);
      if (visible && row.contains(visible)) void client?.markThreadRead(rootId, latest.seq);
    };
    const frame = requestAnimationFrame(read);
    const observer = new ResizeObserver(read);
    observer.observe(list);
    observer.observe(row);
    list.addEventListener("scroll", read);
    document.addEventListener("visibilitychange", read);
    window.addEventListener("focus", read);
    const timer = setInterval(read, 15_000);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      list.removeEventListener("scroll", read);
      document.removeEventListener("visibilitychange", read);
      window.removeEventListener("focus", read);
      clearInterval(timer);
    };
  }, [client, rootId, replies, readActive, state.thread?.loading, state.thread?.hasNewer, state.unreadThreads]);
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
    if (
      !client ||
      readOnly ||
      blocked ||
      sending ||
      state.sendRejected ||
      state.phase !== "ready" ||
      state.thread?.loading
    )
      return;
    try {
      await onSend(pending?.text ?? draft.text, pending?.broadcast ?? draft.broadcast);
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
      onPointerDownCapture={(event) => {
        suppressSwipeClick.current = false;
        backSwipe.current = undefined;
        if (!mobile || event.pointerType !== "touch" || !event.isPrimary) return;
        if (
          !(event.target instanceof Element) ||
          !event.target.closest(".chat-thread-heading, .chat-thread-messages") ||
          event.target.closest("input, textarea, select, [contenteditable=true], [role=slider], audio, video, pre") ||
          document.querySelector(
            "dialog[open], details[open], [popover]:popover-open, .chat-message-actions, .chat-reaction-picker, .chat-mention-card, .chat-reactors",
          ) ||
          window.getSelection()?.type === "Range"
        )
          return;
        backSwipe.current = { id: event.pointerId, x: event.clientX, y: event.clientY, claimed: false };
      }}
      onPointerMoveCapture={(event) => {
        const swipe = backSwipe.current;
        if (!swipe || swipe.id !== event.pointerId) return;
        const dx = event.clientX - swipe.x,
          dy = event.clientY - swipe.y;
        if (!swipe.claimed) {
          if (Math.max(Math.abs(dx), Math.abs(dy)) <= 12) return;
          // Decide once: a vertical scroll or left swipe cannot turn into Back.
          if (dx <= 12 || dx <= Math.abs(dy) * 1.5) {
            backSwipe.current = undefined;
            return;
          }
          swipe.claimed = true;
          suppressSwipeClick.current = true;
          event.currentTarget.setPointerCapture(event.pointerId);
        }
      }}
      onPointerUpCapture={(event) => {
        const swipe = backSwipe.current;
        backSwipe.current = undefined;
        if (swipe?.id !== event.pointerId || !swipe.claimed) return;
        const dx = event.clientX - swipe.x,
          dy = event.clientY - swipe.y;
        if (dx >= 72 && dx > Math.abs(dy) * 1.5) onClose();
      }}
      onPointerCancelCapture={() => {
        backSwipe.current = undefined;
      }}
      onLostPointerCapture={(event) => {
        // Taking capture from the touched row also bubbles its lost-capture event.
        if (event.target === event.currentTarget) backSwipe.current = undefined;
      }}
      onClickCapture={(event) => {
        if (!suppressSwipeClick.current) return;
        event.preventDefault();
        event.stopPropagation();
      }}
      onDragEnter={(event) => {
        if (!canAttach || !carriesFiles(event.dataTransfer)) return;
        dragDepth.current += 1;
        setDropping(true);
      }}
      onDragLeave={() => {
        if (!dragDepth.current) return;
        dragDepth.current -= 1;
        if (!dragDepth.current) setDropping(false);
      }}
      onDragOver={(event) => {
        if (!canAttach || !carriesFiles(event.dataTransfer)) return;
        event.preventDefault();
        event.dataTransfer.dropEffect = "copy";
      }}
      onDrop={(event) => {
        endDrop();
        if (!canAttach || !event.dataTransfer.files.length) return;
        event.preventDefault();
        attachments.onAdd([...event.dataTransfer.files]);
      }}
    >
      {dropping && canAttach && <DropOverlay channelName="thread" direct attached={attachments.drafts.length} />}
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
        {...keyboardDismiss}
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
              onClick={() =>
                void (state.thread?.failedPage === "older"
                  ? client?.loadOlderThread()
                  : state.thread?.failedPage === "newer"
                    ? client?.loadNewerThread()
                    : client?.retryThread(state.thread?.focusMessageId))
              }
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
                motionKey={pending.clientMessageId}
                reference={pending.replyTo}
                hidden={blockedIds.has(pending.replyTo.author.id) && pending.replyTo.author.id !== state.author?.id}
                onOpen={() => onOpenReference(pending.replyTo!)}
              />
            )}
            <div className="chat-avatar">
              <Avatar person={pending.author} name={pending.author?.name ?? ""} />
            </div>
            <div>
              <header>
                <strong>{pending.author?.name}</strong>
              </header>
              {pending.text && <p>{pending.text}</p>}
              <MessageAttachments attachments={pending.attachments ?? []} localPreviews={localPreviews} />
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
            {(validation ?? attachments?.error) && (
              <p className="chat-inline-error" role="alert">
                {validation ?? attachments?.error}
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
            {attachments && (
              <DraftAttachments
                drafts={attachments.drafts}
                onRemove={attachments.onRemove}
                onRetry={attachments.onRetry}
                onToggleSpoiler={sending ? undefined : attachments.onToggleSpoiler}
              />
            )}
            <form
              className="chat-thread-send-row"
              data-attach={!!attachments || undefined}
              onSubmit={(event) => {
                event.preventDefault();
                void submit();
              }}
            >
              <label className="chat-thread-broadcast">
                <input
                  type="checkbox"
                  checked={pending?.broadcast ?? draft.broadcast}
                  disabled={!!pending}
                  onChange={(event) => update({ broadcast: event.target.checked })}
                />
                <span>
                  Also send to <strong>{direct ? channelName : `#${channelName}`}</strong>
                </span>
              </label>
              {attachments && (
                <AttachmentPicker
                  title={
                    !canAttach
                      ? (attachments.disabledReason ?? "Checking file uploads…")
                      : attachments.drafts.length >= MAX_ATTACHMENTS
                        ? `You can attach up to ${MAX_ATTACHMENTS} files.`
                        : "Attach files"
                  }
                  disabled={!canAttach || attachments.drafts.length >= MAX_ATTACHMENTS}
                  onAdd={attachments.onAdd}
                />
              )}
              <label className="sr-only" htmlFor="chat-thread-reply">
                Reply to thread
              </label>
              {suggestions.popup}
              <MentionTextarea
                ref={composer}
                id="chat-thread-reply"
                people={mentionPeople}
                specialMentions={specialMentions}
                rows={1}
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
                onPaste={(event) => {
                  if (event.clipboardData.files.length && canAttach) {
                    event.preventDefault();
                    attachments.onAdd([...event.clipboardData.files]);
                  }
                }}
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
              <EmojiButton
                disabled={state.phase !== "ready" || state.thread.loading}
                onSelect={chooseEmoji}
                returnFocus={composer}
              />
              <button
                type="submit"
                disabled={
                  sending ||
                  blocked ||
                  state.sendRejected ||
                  state.phase !== "ready" ||
                  state.thread.loading ||
                  (!pending && !draft.text.trim() && !attachments?.drafts.length)
                }
                aria-label="Send reply"
              >
                <Send size={18} />
              </button>
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
