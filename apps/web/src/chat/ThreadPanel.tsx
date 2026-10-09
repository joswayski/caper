import { useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, Send, X } from "lucide-react";
import type { ChatClient, ChatViewState } from "./client.ts";
import { sequence, type ChatMessage } from "./types.ts";

export default function ThreadPanel({
  state,
  client,
  channelName,
  direct = false,
  readOnly,
  renderMessage,
  onClose,
}: {
  state: ChatViewState;
  client?: ChatClient;
  channelName: string;
  direct?: boolean;
  readOnly: boolean;
  renderMessage: (index: number, message: ChatMessage, inThread: boolean) => ReactNode;
  onClose: () => void;
}) {
  const [drafts, setDrafts] = useState<Record<string, { text: string; broadcast: boolean }>>({});
  const [validation, setValidation] = useState<string>();
  const composer = useRef<HTMLTextAreaElement>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const olderAnchor = useRef<{ height: number; top: number }>(undefined);
  const follow = useRef(true);
  const returnFocus = useRef<HTMLElement>(null);
  const panel = useRef<HTMLElement>(null);
  const [mobile, setMobile] = useState(false);
  const rootId = state.thread?.rootId;
  const pending = rootId && state.pendingSend?.threadRootId === rootId ? state.pendingSend : undefined;
  const windowStart = state.thread?.windowStart;
  const windowEnd = state.thread?.windowEnd;
  const replies = useMemo(
    () =>
      rootId
        ? state.messages.filter(
            (message) =>
              message.threadRootId === rootId &&
              (!windowStart || sequence(message.seq) >= sequence(windowStart)) &&
              (!windowEnd || sequence(message.seq) <= sequence(windowEnd)),
          )
        : [],
    [state.messages, rootId, windowStart, windowEnd],
  );
  const draft = rootId ? (drafts[rootId] ?? { text: "", broadcast: false }) : { text: "", broadcast: false };
  const update = (change: Partial<typeof draft>) => {
    if (rootId) setDrafts((current) => ({ ...current, [rootId]: { ...draft, ...change } }));
    setValidation(undefined);
  };
  useEffect(() => {
    setDrafts({});
  }, [state.channelId]);
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
  const open = !!rootId;
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
    follow.current = true;
    composer.current?.focus();
    const escape = (event: KeyboardEvent) => {
      // Escape closes the innermost layer first: a menu, popover or dialog
      // over the thread (including sidebar menus and space dialogs) keeps it open.
      if (
        event.key === "Escape" &&
        !event.defaultPrevented &&
        !document.querySelector(
          ".chat-reaction-picker, .chat-message-actions, .chat-reactors, dialog[open], details[open], [popover]:popover-open",
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
  useEffect(() => {
    if (scroll.current && follow.current) scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [rootId, replies.at(-1)?.id, pending?.clientMessageId, state.thread?.loading]);
  useEffect(() => {
    const id = state.thread?.focusMessageId;
    if (!id || state.thread?.loading) return;
    follow.current = false;
    const frame = requestAnimationFrame(() =>
      scroll.current?.querySelector(`[data-message-id="${CSS.escape(id)}"]`)?.scrollIntoView({ block: "center" }),
    );
    return () => cancelAnimationFrame(frame);
  }, [state.thread?.focusMessageId, state.thread?.loading]);
  useLayoutEffect(() => {
    if (state.thread?.loadingOlder || !olderAnchor.current || !scroll.current) return;
    scroll.current.scrollTop = olderAnchor.current.top + scroll.current.scrollHeight - olderAnchor.current.height;
    olderAnchor.current = undefined;
  }, [state.thread?.loadingOlder, replies.length]);
  const root = useMemo(() => state.messages.find((message) => message.id === rootId), [state.messages, rootId]);
  // Typing a reply changes only this panel's draft; keep the rendered messages.
  const renderedRoot = useMemo(() => root && renderMessage(0, root, true), [root, renderMessage]);
  const renderedReplies = useMemo(
    () => replies.map((message, index) => renderMessage(index + 1, message, true)),
    [replies, renderMessage],
  );
  if (!state.thread) return null;
  const sending = !!state.pendingSend && !state.sendError;
  const blocked = !!state.pendingSend && !pending;
  const submit = async () => {
    if (!client || readOnly || blocked || sending || state.sendRejected) return;
    try {
      await client.send(pending?.text ?? draft.text, { threadRootId: rootId, broadcast: draft.broadcast });
      follow.current = true;
    } catch (error) {
      setValidation(error instanceof Error ? error.message : "Reply could not be sent.");
    }
  };
  return (
    <aside
      ref={panel}
      className="chat-thread-panel"
      aria-labelledby="chat-thread-heading"
      role={mobile ? "dialog" : undefined}
      aria-modal={mobile || undefined}
    >
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
          <div className="chat-thread-skeleton" role="status" aria-label="Loading thread replies">
            {Array.from({ length: Math.min(3, Math.max(1, root?.thread?.replyCount ?? 2)) }, (_, index) => (
              <div className="chat-thread-skeleton-row" key={index} aria-hidden="true">
                <span className="chat-thread-skeleton-avatar" />
                <div>
                  <span className="chat-thread-skeleton-name" />
                  <span className="chat-thread-skeleton-text" />
                </div>
              </div>
            ))}
          </div>
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
          <p className="chat-thread-status">No replies yet. Start the thread.</p>
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
          <article className="chat-message chat-message-pending">
            <div />
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
          <p>Join the channel to reply.</p>
        ) : (
          <>
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
              <textarea
                ref={composer}
                id="chat-thread-reply"
                rows={3}
                value={draft.text}
                placeholder="Reply to thread…"
                enterKeyHint="send"
                disabled={state.phase !== "ready" || state.thread.loading}
                onChange={(event) => update({ text: event.target.value })}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) {
                    event.preventDefault();
                    void submit();
                  }
                }}
              />
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
              {Array.from(draft.text).length >= 3000 && (
                <small>{Array.from(draft.text).length.toLocaleString()} / 4,000</small>
              )}
            </form>
          </>
        )}
      </div>
    </aside>
  );
}
