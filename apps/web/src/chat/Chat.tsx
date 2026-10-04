import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Virtuoso, type VirtuosoHandle, type ListProps, type ContextProp } from "react-virtuoso";
import { ChatClient, initialChatView } from "./client.ts";
import MessageReactions, { type ReactionSave } from "./MessageReactions.tsx";
import MessageActions, { type MessageActionTarget } from "./MessageActions.tsx";
import { dateDivider } from "./dates.ts";
import type { ChatAuthor, GeneralChatHistory } from "./types.ts";
import { appGateway, type PresenceStatus } from "../gateway/client.ts";
import Avatar from "../components/Avatar";
import "./chat.css";

// Virtuoso's prepend index is local bookkeeping, never the bigint server cursor.
const INITIAL_ITEM_INDEX = 1_000_000_000;

function timeLabel(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "" : new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(date);
}

interface HistoryContext {
  hasMore: boolean;
  loadingOlder: boolean;
  olderError?: string;
  loadOlder: () => void;
  onListReady?: () => void;
}

function HistoryHeader({ context }: { context?: HistoryContext }) {
  return <div className="chat-history" role="status">
    {context?.olderError ? <><span>Couldn’t load older messages.</span><button type="button" onClick={context.loadOlder}>Retry</button></>
      : context?.hasMore ? <button type="button" disabled={context.loadingOlder} onClick={context.loadOlder}>{context.loadingOlder ? "Loading…" : "Load older messages"}</button>
      : <span>Beginning of conversation</span>}
  </div>;
}

function MessageList({ context, children, ...props }: ListProps & ContextProp<HistoryContext>) {
  // Virtuoso deliberately hides its items while finding the initial scroll
  // position. Hand over from the real-message preview before the next paint.
  useLayoutEffect(() => {
    if (Array.isArray(children) && children.length && props.style?.visibility !== "hidden") context?.onListReady?.();
  }, [children, props.style?.visibility, context?.onListReady]);
  return <div {...props}>{children}</div>;
}

const listComponents = { Header: HistoryHeader, List: MessageList };
const measureItem = (element: HTMLElement, field: "offsetHeight" | "offsetWidth") => element[field];

export default function Chat({ name, signedIn, identityReady, channelId, channelName: expectedChannelName, direct = false, onReadCursor, initialHistory, initialHistoryError, showTitle = false, headerActions, readOnly = false, composerNotice, messageSounds = true, onAuthorChange, onHistoryChange, onLocalPresenceChange, onOnlineChange }: { name: string; signedIn: boolean; identityReady: boolean; channelId?: string; channelName?: string; direct?: boolean; onReadCursor?: (seq: string) => void; initialHistory?: GeneralChatHistory; initialHistoryError?: string; showTitle?: boolean; headerActions?: ReactNode; readOnly?: boolean; composerNotice?: ReactNode; messageSounds?: boolean; onAuthorChange?: (author: ChatAuthor) => void; onHistoryChange?: (history: GeneralChatHistory) => void; onLocalPresenceChange?: (status: PresenceStatus) => void; onOnlineChange?: (online: boolean) => void }) {
  const [state, setState] = useState(() => initialChatView(initialHistory, initialHistoryError));
  const [showConnectionStatus, setShowConnectionStatus] = useState(false);
  const [firstItemIndex, setFirstItemIndex] = useState(INITIAL_ITEM_INDEX);
  const [draft, setDraft] = useState("");
  const [validationError, setValidationError] = useState<string>();
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const [actionTarget, setActionTarget] = useState<MessageActionTarget>();
  const [actionStatus, setActionStatus] = useState("");
  const [reactionSaves, setReactionSaves] = useState<Record<string, ReactionSave | undefined>>({});
  const reactionBusy = useRef(new Set<string>());
  const press = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number; pointerId: number }>(undefined);
  const suppressClick = useRef(false);
  const cancelPress = () => { clearTimeout(press.current?.timer); press.current = undefined; };
  useEffect(() => {
    setActionTarget(undefined);
    setActionStatus("");
    setReactionSaves({});
    reactionBusy.current.clear();
    // The drawer can appear under the held finger. Its release click must not
    // activate a newly rendered action, even though that action is in a portal.
    const resetClick = () => { suppressClick.current = false; };
    const suppressReleaseClick = (event: MouseEvent) => {
      if (!suppressClick.current) return;
      suppressClick.current = false;
      event.preventDefault();
      event.stopPropagation();
    };
    document.addEventListener("pointerdown", resetClick, true);
    document.addEventListener("keydown", resetClick, true);
    document.addEventListener("click", suppressReleaseClick, true);
    window.addEventListener("blur", cancelPress);
    window.addEventListener("scroll", cancelPress, true);
    return () => {
      cancelPress();
      document.removeEventListener("pointerdown", resetClick, true);
      document.removeEventListener("keydown", resetClick, true);
      document.removeEventListener("click", suppressReleaseClick, true);
      window.removeEventListener("blur", cancelPress);
      window.removeEventListener("scroll", cancelPress, true);
    };
  }, [channelId]);
  useEffect(() => { setActionTarget(undefined); }, [state.author?.id]);
  const isTouchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;
  const openActions = (messageId: string, anchor: HTMLElement) => setActionTarget({ messageId, anchor, mode: "actions", drawer: true });
  const react = async (messageId: string, emoji: string, active: boolean) => {
    const client = clientRef.current;
    if (readOnly || !state.author || !client || reactionBusy.current.has(messageId)) return;
    reactionBusy.current.add(messageId);
    setReactionSaves((current) => ({ ...current, [messageId]: { emoji, active, saving: true } }));
    try {
      await client.setReaction(messageId, emoji, active);
      if (clientRef.current === client) setReactionSaves((current) => ({ ...current, [messageId]: undefined }));
    } catch (error) {
      if (clientRef.current === client) setReactionSaves((current) => ({ ...current, [messageId]: { emoji, active, saving: false, error: error instanceof Error ? error.message : "Reaction could not be saved." } }));
    } finally { if (clientRef.current === client) reactionBusy.current.delete(messageId); }
  };
  const actionMessage = state.messages.find((message) => message.id === actionTarget?.messageId);
  const listRef = useRef<VirtuosoHandle>(null);
  const initialListRef = useRef<HTMLDivElement>(null);
  const [listReady, setListReady] = useState(false);
  // Virtuoso needs browser APIs; the server and first client render use the plain list.
  const [hydrated, setHydrated] = useState(false);
  useEffect(() => setHydrated(true), []);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const followLatest = useRef(true);
  const latestMessage = state.messages.at(-1);
  // Reaction events advance the conversation stream without adding a message.
  // HTTP reaction snapshots do not advance this committed replay cursor.
  const readCursor = clientRef.current?.snapshotHistory()?.cursor ?? initialHistory?.cursor ?? latestMessage?.seq ?? "0";
  const readCallback = useRef(onReadCursor);
  readCallback.current = onReadCursor;
  useEffect(() => {
    const read = () => {
      if (state.phase === "ready" && document.visibilityState === "visible") readCallback.current?.(readCursor);
    };
    read();
    document.addEventListener("visibilitychange", read);
    return () => document.removeEventListener("visibilitychange", read);
  }, [state.phase, readCursor, channelId]);

  useLayoutEffect(() => {
    const composer = composerRef.current;
    if (!composer) return;
    const resize = () => {
      composer.style.height = "0px";
      composer.style.height = `${composer.scrollHeight + composer.offsetHeight - composer.clientHeight}px`;
      if (followLatest.current) listRef.current?.autoscrollToBottom();
    };
    resize();
    let width = composer.clientWidth;
    const observer = new ResizeObserver(() => {
      if (composer.clientWidth === width) return;
      width = composer.clientWidth;
      resize();
    });
    observer.observe(composer);
    return () => observer.disconnect();
  }, [draft]);

  useEffect(() => {
    let pendingId: string | undefined;
    let firstMessageId: string | undefined;
    const client = new ChatClient((next) => {
      if (next.phase !== "ready") {
        firstMessageId = undefined;
        setListReady(false);
        setFirstItemIndex(INITIAL_ITEM_INDEX);
      } else if (next.messages[0]?.id !== firstMessageId) {
        const prepended = next.messages.findIndex((message) => message.id === firstMessageId);
        if (prepended > 0) setFirstItemIndex((index) => index - prepended);
        firstMessageId = next.messages[0]?.id;
      }
      const pending = next.pendingSend;
      if (pending && pending.clientMessageId !== pendingId) {
        setDraft((current) => current === pending.text ? "" : current);
        followLatest.current = true;
      }
      pendingId = pending?.clientMessageId;
      setState(next);
    }, channelId, { sounds: messageSounds });
    clientRef.current = client;
    client.start(initialHistory, initialHistoryError);
    return () => {
      const history = client.snapshotHistory();
      if (history) onHistoryChange?.(history);
      client.stop();
      clientRef.current = undefined;
    };
  }, [channelId]);

  useEffect(() => {
    setShowConnectionStatus(false);
    if (state.online) return;
    const timer = setTimeout(() => setShowConnectionStatus(true), 1_000);
    return () => clearTimeout(timer);
  }, [state.online, channelId]);

  useEffect(() => {
    if (!onLocalPresenceChange) return;
    const update = () => onLocalPresenceChange(appGateway().localPresence(state.online));
    update();
    const timer = setInterval(update, 1_000);
    return () => clearInterval(timer);
  }, [state.online, onLocalPresenceChange]);

  useEffect(() => { clientRef.current?.setSounds(messageSounds); }, [messageSounds]);

  useEffect(() => { onOnlineChange?.(state.online); }, [state.online, onOnlineChange]);

  useEffect(() => {
    if (identityReady && !readOnly) clientRef.current?.identify(name, signedIn);
  }, [identityReady, name, signedIn, readOnly]);

  useEffect(() => {
    if (state.author) onAuthorChange?.(state.author);
  }, [state.author, onAuthorChange]);

  useEffect(() => {
    if (followLatest.current) listRef.current?.scrollToIndex({ index: "LAST", align: "end" });
  }, [state.pendingSend?.clientMessageId, latestMessage?.clientMessageId]);

  const loadOlder = () => { void clientRef.current?.loadOlder(); };

  const sending = !!state.pendingSend && !state.sendError;
  const channelName = direct ? expectedChannelName ?? state.channelName : (expectedChannelName ?? state.channelName).toLowerCase();
  const characterCount = Array.from(draft).length;
  const counterTone = characterCount >= 3900 ? "red" : characterCount >= 3750 ? "orange" : characterCount >= 3500 ? "yellow" : "gray";
  const messages = state.pendingSend ? [...state.messages, state.pendingSend] : state.messages;
  const previewStart = Math.max(0, messages.length - Math.max(20, Math.ceil((typeof window === "undefined" ? 800 : window.innerHeight) / 50)));
  useLayoutEffect(() => {
    const list = initialListRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [messages, listReady]);
  const typingNames = state.typingAuthors.map((author) => author.name);
  const typingLabel = typingNames.length > 2 ? "Several people are typing…"
    : typingNames.length ? `${typingNames.join(" and ")} ${typingNames.length === 1 ? "is" : "are"} typing…` : "";
  const [displayedTypingLabel, setDisplayedTypingLabel] = useState("");
  useEffect(() => {
    if (typingLabel) {
      setDisplayedTypingLabel(typingLabel);
      return;
    }
    const timer = setTimeout(() => setDisplayedTypingLabel(""), 180);
    return () => clearTimeout(timer);
  }, [typingLabel]);
  const submit = async () => {
    if (readOnly || !identityReady || sending || state.sendRejected) return;
    setValidationError(undefined);
    followLatest.current = true;
    const submitted = state.pendingSend?.text ?? draft;
    try {
      await clientRef.current?.send(submitted);
    } catch (error) {
      setValidationError(error instanceof Error ? error.message : "Message could not be sent.");
    }
  };

  const renderMessage = (index: number, message: (typeof messages)[number]) => {
    const pending = !("content" in message);
    const author = message.author;
    const divider = hydrated ? dateDivider(message.createdAt, messages[index - 1]?.createdAt) : undefined;
    return <div key={message.clientMessageId}>
      {divider && <div className="chat-date-divider"><time dateTime={message.createdAt}>{divider}</time></div>}
      <article className={`chat-message${pending ? " chat-message-pending" : ""}`} data-message-key={message.clientMessageId}
        onPointerDown={(event) => {
          cancelPress();
          if (!("content" in message) || event.pointerType === "mouse" || !event.isPrimary || (event.target as HTMLElement).closest("button, a")) return;
          const anchor = event.currentTarget;
          press.current = { x: event.clientX, y: event.clientY, pointerId: event.pointerId, timer: setTimeout(() => {
            suppressClick.current = true;
            window.getSelection()?.removeAllRanges();
            openActions(message.id, anchor);
          }, 500) };
        }}
        onPointerMove={(event) => {
          const current = press.current;
          if (current && (event.pointerId !== current.pointerId || Math.hypot(event.clientX - current.x, event.clientY - current.y) > 10)) cancelPress();
        }}
        onPointerUp={cancelPress} onPointerCancel={cancelPress}
        onContextMenu={(event) => {
          if (!("content" in message) || !isTouchLayout() || (event.target as HTMLElement).closest("button, a")) return;
          event.preventDefault();
          cancelPress();
          suppressClick.current = true;
          openActions(message.id, event.currentTarget);
        }}
        tabIndex={pending ? undefined : -1}>
      <div className="chat-avatar"><Avatar avatarId={author?.avatarId} name={author?.name ?? name} /></div>
      <div>
        <header><strong>{author?.name ?? name}</strong>{author?.isGuest && <span>Guest</span>}<time dateTime={message.createdAt}>{hydrated ? timeLabel(message.createdAt) : ""}</time></header>
        <p>{"content" in message ? message.content.text : message.text}</p>
        {"content" in message && <>
          <button type="button" className="chat-message-actions-trigger sr-only" aria-haspopup="dialog" onClick={(event) => openActions(message.id, event.currentTarget)}>Message actions for {message.author.name}</button>
          <MessageReactions message={message} authorId={state.author?.id} readOnly={readOnly} save={reactionSaves[message.id]} onReact={react}
            pickerOpen={actionTarget?.messageId === message.id && actionTarget.mode === "emoji"}
            onOpenPicker={(anchor) => setActionTarget({ messageId: message.id, anchor, anchorRect: anchor.getBoundingClientRect(), mode: "emoji", drawer: isTouchLayout() })}
            onDismissError={() => setReactionSaves((current) => ({ ...current, [message.id]: undefined }))} />
        </>}
        {pending && state.sendError && <div className="chat-send-status chat-send-error" role="alert">
          <span>{state.sendRejected ? "Not sent." : "Not confirmed yet."} {state.sendError}</span>
          {state.sendRejected ? <>
            <button type="button" disabled={!!draft} title={draft ? "Clear your current draft to edit this message." : undefined} onClick={() => {
              const text = clientRef.current?.discardRejected();
              if (text !== undefined) { setDraft(text); composerRef.current?.focus(); }
            }}>Edit</button>
            <button type="button" onClick={() => clientRef.current?.discardRejected()}>Dismiss</button>
          </> : <button type="button" onClick={() => void submit()}>Retry send</button>}
        </div>}
      </div>
    </article></div>;
  };

  return <section className="chat-panel" aria-labelledby="chat-heading">
    <header className="chat-heading">
      <h2 id="chat-heading" className={showTitle ? "chat-channel-title" : "sr-only"}>{direct ? "" : "# "}{channelName}</h2>
      {headerActions}
      {!state.online && showConnectionStatus && <span className="chat-offline" role="status">{state.phase === "error" ? "Offline" : "Connecting…"}</span>}
      {state.phase === "ready" && state.error && <div className="chat-refresh-error" role="alert">{state.error} <button type="button" onClick={() => clientRef.current?.retryLoad()}>Retry</button></div>}
    </header>

    <div className="chat-messages" aria-busy={state.phase === "loading"}>
      {state.phase === "loading" && <p className="chat-state" role="status">Loading messages…</p>}
      {state.phase === "error" && <div className="chat-state" role="alert"><p>{state.error}</p><button type="button" onClick={() => clientRef.current?.retryLoad()}>Try again</button></div>}
      {state.phase === "ready" && !messages.length && <div className="chat-state"><p>No messages yet.</p><small>{direct ? `Only you and ${channelName} can read this conversation.` : `Start the conversation in #${channelName}.`}</small></div>}
      {state.phase === "ready" && messages.length > 0 && hydrated && <Virtuoso
        ref={listRef}
        data={messages}
        firstItemIndex={firstItemIndex}
        initialTopMostItemIndex={{ index: "LAST", align: "end" }}
        computeItemKey={(_, message) => `${message.author?.id ?? "pending"}:${message.clientMessageId}`}
        defaultItemHeight={70}
        // Layout sizes, not getBoundingClientRect: inside the homepage's tilted
        // window the rect is scaled, which would hide the newest messages.
        itemSize={measureItem}
        increaseViewportBy={{ top: 250, bottom: 150 }}
        followOutput="auto"
        atBottomThreshold={80}
        atBottomStateChange={(atBottom) => { followLatest.current = atBottom; }}
        startReached={() => { if (!state.olderError) loadOlder(); }}
        components={listComponents}
        context={{ hasMore: state.hasMore, loadingOlder: state.loadingOlder, olderError: state.olderError, loadOlder, onListReady: listReady ? undefined : () => setListReady(true) }}
        className="chat-scroller"
        aria-hidden={!listReady}
        tabIndex={listReady ? 0 : -1}
        role="region"
        aria-label={`Messages in ${channelName}`}
        onKeyDown={(event) => {
          if (event.target === event.currentTarget && event.key === "End") {
            event.preventDefault();
            listRef.current?.scrollToIndex({ index: "LAST", align: "end" });
          }
        }}
        itemContent={(index, message) => renderMessage(index - firstItemIndex, message)} />}
      {state.phase === "ready" && messages.length > 0 && !listReady && <div ref={initialListRef} className="chat-initial-messages" role="region" aria-label={`Messages in ${channelName}`}>
        <HistoryHeader context={{ hasMore: state.hasMore, loadingOlder: false, loadOlder }} />
        {messages.slice(previewStart).map((message, index) => renderMessage(previewStart + index, message))}
      </div>}
      <p className="sr-only" aria-live="polite" aria-atomic="true">{state.phase === "ready" && latestMessage && `${latestMessage.author.name}: ${latestMessage.content.text}`}</p>
      <p className="sr-only" role="status">{actionStatus}</p>
    </div>

    {actionTarget && actionMessage && <MessageActions key={actionMessage.id} message={actionMessage} target={actionTarget} authorId={state.author?.id}
      canReact={!readOnly && !!state.author && !reactionSaves[actionMessage.id]?.saving} onReact={react} onClose={() => setActionTarget(undefined)} onCopied={setActionStatus} />}

    <p className="chat-typing" role="status" aria-atomic="true">
      <span className="chat-typing-content" data-visible={!!typingLabel} aria-hidden={!typingLabel}>
        {displayedTypingLabel && <><span className="chat-typing-dots" aria-hidden="true"><i /><i /><i /></span><span>{displayedTypingLabel}</span></>}
      </span>
    </p>

    {readOnly ? <div className="chat-composer channel-preview">{composerNotice}</div> : <div className="chat-composer">
      {state.sessionError && <p className="chat-inline-error" role="alert">{state.sessionError} <button type="button" onClick={() => clientRef.current?.retrySession()}>Retry session</button></p>}
      {validationError && <p className="chat-inline-error" role="alert">{validationError}</p>}
      <form onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <label className="sr-only" htmlFor="chat-message">Message {channelName}</label>
        <textarea ref={composerRef} id="chat-message" rows={1} value={draft} disabled={state.phase !== "ready"} enterKeyHint="send" aria-describedby="chat-composer-hint" placeholder={`Message ${direct ? "" : "#"}${channelName}`} onChange={(event) => { setDraft(event.target.value); setValidationError(undefined); clientRef.current?.setTyping(!!event.target.value.trim()); }} onBlur={() => clientRef.current?.setTyping(false)} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!sending) void submit(); }
        }} />
        <span id="chat-composer-hint" className="sr-only">Enter to send. Shift+Enter for a new line.</span>
        {characterCount >= 3000 && <small className="chat-counter" data-tone={counterTone}>{characterCount.toLocaleString()} / 4,000</small>}
      </form>
    </div>}
  </section>;
}
