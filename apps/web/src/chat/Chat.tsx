import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Virtuoso, type VirtuosoHandle } from "react-virtuoso";
import { ChatClient, type ChatViewState } from "./client.ts";
import type { ChatAuthor } from "./types.ts";
import "./chat.css";

// Virtuoso's prepend index is local bookkeeping, never the bigint server cursor.
const INITIAL_ITEM_INDEX = 1_000_000_000;
const initialView: ChatViewState = {
  phase: "loading", online: false, spaceName: "Caper", channelName: "General",
  messages: [], typingAuthors: [], hasMore: false, loadingOlder: false,
};

function timeLabel(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "" : new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(date);
}

interface HistoryContext {
  hasMore: boolean;
  loadingOlder: boolean;
  olderError?: string;
  loadOlder: () => void;
}

function HistoryHeader({ context }: { context?: HistoryContext }) {
  return <div className="chat-history" role="status">
    {context?.olderError ? <><span>Couldn’t load older messages.</span><button type="button" onClick={context.loadOlder}>Retry</button></>
      : context?.hasMore ? <button type="button" disabled={context.loadingOlder} onClick={context.loadOlder}>{context.loadingOlder ? "Loading…" : "Load older messages"}</button>
      : <span>Beginning of conversation</span>}
  </div>;
}

const listComponents = { Header: HistoryHeader };

export default function Chat({ name, signedIn, identityReady, headerActions, onAuthorChange }: { name: string; signedIn: boolean; identityReady: boolean; headerActions?: ReactNode; onAuthorChange?: (author: ChatAuthor) => void }) {
  const [state, setState] = useState(initialView);
  const [firstItemIndex, setFirstItemIndex] = useState(INITIAL_ITEM_INDEX);
  const [draft, setDraft] = useState("");
  const [validationError, setValidationError] = useState<string>();
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const listRef = useRef<VirtuosoHandle>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const followLatest = useRef(true);
  const latestMessage = state.messages.at(-1);

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
    });
    clientRef.current = client;
    client.start();
    return () => { client.stop(); clientRef.current = undefined; };
  }, []);

  useEffect(() => {
    if (identityReady) clientRef.current?.identify(name, signedIn);
  }, [identityReady, name, signedIn]);

  useEffect(() => {
    if (state.author) onAuthorChange?.(state.author);
  }, [state.author, onAuthorChange]);

  useEffect(() => {
    if (followLatest.current) listRef.current?.scrollToIndex({ index: "LAST", align: "end" });
  }, [state.pendingSend?.clientMessageId, latestMessage?.clientMessageId]);

  const loadOlder = () => { void clientRef.current?.loadOlder(); };

  const sending = !!state.pendingSend && !state.sendError;
  const channelName = state.channelName.toLowerCase();
  const characterCount = Array.from(draft).length;
  const counterTone = characterCount >= 3900 ? "red" : characterCount >= 3750 ? "orange" : characterCount >= 3500 ? "yellow" : "gray";
  const messages = state.pendingSend ? [...state.messages, state.pendingSend] : state.messages;
  const typingNames = state.typingAuthors.map((author) => author.name);
  const typingLabel = typingNames.length > 2 ? "Several people are typing…"
    : typingNames.length ? `${typingNames.join(" and ")} ${typingNames.length === 1 ? "is" : "are"} typing…` : "";
  const submit = async () => {
    if (!identityReady || sending || state.sendRejected) return;
    setValidationError(undefined);
    followLatest.current = true;
    const submitted = state.pendingSend?.text ?? draft;
    try {
      await clientRef.current?.send(submitted);
    } catch (error) {
      setValidationError(error instanceof Error ? error.message : "Message could not be sent.");
    }
  };

  return <section className="chat-panel" aria-labelledby="chat-heading">
    <header className="chat-heading">
      <h2 id="chat-heading" className="sr-only"># {channelName}</h2>
      {headerActions}
      {!state.online && <span className="chat-offline" role="status">{state.phase === "loading" ? "Loading" : "Offline"}</span>}
    </header>

    <div className="chat-messages" aria-busy={state.phase === "loading"}>
      {state.phase === "loading" && <p className="chat-state" role="status">Loading messages…</p>}
      {state.phase === "error" && <div className="chat-state" role="alert"><p>{state.error}</p><button type="button" onClick={() => clientRef.current?.retryLoad()}>Try again</button></div>}
      {state.phase === "ready" && !messages.length && <div className="chat-state"><p>No messages yet.</p><small>Start the conversation in #{channelName}.</small></div>}
      {state.phase === "ready" && messages.length > 0 && <Virtuoso
        ref={listRef}
        data={messages}
        firstItemIndex={firstItemIndex}
        initialTopMostItemIndex={{ index: "LAST", align: "end" }}
        computeItemKey={(_, message) => `${message.author?.id ?? "pending"}:${message.clientMessageId}`}
        defaultItemHeight={70}
        increaseViewportBy={{ top: 250, bottom: 150 }}
        followOutput="auto"
        atBottomThreshold={80}
        atBottomStateChange={(atBottom) => { followLatest.current = atBottom; }}
        startReached={() => { if (!state.olderError) loadOlder(); }}
        components={listComponents}
        context={{ hasMore: state.hasMore, loadingOlder: state.loadingOlder, olderError: state.olderError, loadOlder }}
        className="chat-scroller"
        tabIndex={0}
        role="region"
        aria-label={`Messages in ${channelName}`}
        onKeyDown={(event) => {
          if (event.target === event.currentTarget && event.key === "End") {
            event.preventDefault();
            listRef.current?.scrollToIndex({ index: "LAST", align: "end" });
          }
        }}
        itemContent={(_, message) => {
          const pending = !("content" in message);
          const author = message.author;
          return <article className={`chat-message${pending ? " chat-message-pending" : ""}`} data-message-key={message.clientMessageId}>
            <div className="chat-avatar" aria-hidden="true">{(author?.name ?? name).slice(0, 1).toUpperCase()}</div>
            <div>
              <header><strong>{author?.name ?? name}</strong>{author?.isGuest && <span>Guest</span>}<time dateTime={message.createdAt}>{timeLabel(message.createdAt)}</time></header>
              <p>{"content" in message ? message.content.text : message.text}</p>
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
          </article>;
        }} />}
      <p className="sr-only" aria-live="polite" aria-atomic="true">{state.phase === "ready" && latestMessage && `${latestMessage.author.name}: ${latestMessage.content.text}`}</p>
    </div>

    <p className="chat-typing" role="status" aria-atomic="true">
      {typingLabel && <><span className="chat-typing-dots" aria-hidden="true"><i /><i /><i /></span><span>{typingLabel}</span></>}
    </p>

    <div className="chat-composer">
      {state.phase === "ready" && state.error && <p className="chat-inline-error" role="alert">{state.error}</p>}
      {state.sessionError && <p className="chat-inline-error" role="alert">{state.sessionError} <button type="button" onClick={() => clientRef.current?.retrySession()}>Retry session</button></p>}
      {validationError && <p className="chat-inline-error" role="alert">{validationError}</p>}
      <form onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <label className="sr-only" htmlFor="chat-message">Message {channelName}</label>
        <textarea ref={composerRef} id="chat-message" rows={1} value={draft} disabled={state.phase !== "ready"} enterKeyHint="send" aria-describedby="chat-composer-hint" placeholder={`Message #${channelName}`} onChange={(event) => { setDraft(event.target.value); setValidationError(undefined); clientRef.current?.setTyping(!!event.target.value.trim()); }} onBlur={() => clientRef.current?.setTyping(false)} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!sending) void submit(); }
        }} />
        <span id="chat-composer-hint" className="sr-only">Enter to send. Shift+Enter for a new line.</span>
        {characterCount >= 3000 && <small className="chat-counter" data-tone={counterTone}>{characterCount.toLocaleString()} / 4,000</small>}
      </form>
    </div>
  </section>;
}
