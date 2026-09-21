import { useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { ChatClient, type ChatViewState } from "./client.ts";
import type { ChatAuthor } from "./types.ts";
import "./chat.css";

const initialView: ChatViewState = {
  phase: "loading", online: false, spaceName: "Caper", channelName: "General",
  messages: [], typingAuthors: [], hasMore: false, loadingOlder: false,
};

function timeLabel(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "" : new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(date);
}

export default function Chat({ name, signedIn, identityReady, headerActions, onAuthorChange }: { name: string; signedIn: boolean; identityReady: boolean; headerActions?: ReactNode; onAuthorChange?: (author: ChatAuthor) => void }) {
  const [state, setState] = useState(initialView);
  const [draft, setDraft] = useState("");
  const [validationError, setValidationError] = useState<string>();
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const listRef = useRef<HTMLDivElement>(null);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const followLatest = useRef(true);

  useLayoutEffect(() => {
    const composer = composerRef.current;
    if (!composer) return;
    const resize = () => {
      composer.style.height = "0px";
      composer.style.height = `${composer.scrollHeight + composer.offsetHeight - composer.clientHeight}px`;
      const list = listRef.current;
      if (list && followLatest.current) list.scrollTop = list.scrollHeight;
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
    const client = new ChatClient((next) => {
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
    const list = listRef.current;
    if (list && followLatest.current) list.scrollTop = list.scrollHeight;
  }, [state.messages.length, state.pendingSend?.clientMessageId, state.sendError]);

  const loadOlder = async () => {
    const list = listRef.current;
    if (!list) return;
    followLatest.current = false;
    const height = list.scrollHeight;
    const top = list.scrollTop;
    await clientRef.current?.loadOlder();
    requestAnimationFrame(() => { list.scrollTop = top + list.scrollHeight - height; });
  };

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

    <div className="chat-messages" ref={listRef} aria-live="polite" aria-busy={state.phase === "loading"} onScroll={(event) => {
      const list = event.currentTarget;
      followLatest.current = list.scrollHeight - list.scrollTop - list.clientHeight < 80;
    }}>
      {state.hasMore && <button className="chat-history-button" type="button" disabled={state.loadingOlder} onClick={() => void loadOlder()}>{state.loadingOlder ? "Loading…" : "Load older messages"}</button>}
      {state.phase === "loading" && !state.messages.length && <p className="chat-state" role="status">Loading messages…</p>}
      {state.phase === "error" && <div className="chat-state" role="alert"><p>{state.error}</p><button type="button" onClick={() => clientRef.current?.retryLoad()}>Try again</button></div>}
      {state.phase === "ready" && !messages.length && <div className="chat-state"><p>No messages yet.</p><small>Start the conversation in #{channelName}.</small></div>}
      {messages.map((message) => {
        const pending = !("content" in message);
        const author = message.author;
        return <article className={`chat-message${pending ? " chat-message-pending" : ""}`} key={`${author?.id ?? "pending"}:${message.clientMessageId}`}>
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
      })}
    </div>

    <div className="chat-composer">
      {state.phase === "ready" && state.error && <p className="chat-inline-error" role="alert">{state.error}</p>}
      {state.sessionError && <p className="chat-inline-error" role="alert">{state.sessionError} <button type="button" onClick={() => clientRef.current?.retrySession()}>Retry session</button></p>}
      {validationError && <p className="chat-inline-error" role="alert">{validationError}</p>}
      <form onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <label className="sr-only" htmlFor="chat-message">Message {channelName}</label>
        <textarea ref={composerRef} id="chat-message" rows={1} value={draft} disabled={state.phase !== "ready"} placeholder={`Message #${channelName}`} onChange={(event) => { setDraft(event.target.value); setValidationError(undefined); clientRef.current?.setTyping(!!event.target.value.trim()); }} onBlur={() => clientRef.current?.setTyping(false)} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!sending) void submit(); }
        }} />
        <button type="submit" disabled={!identityReady || state.phase !== "ready" || !!state.pendingSend || !draft.trim() || characterCount > 4_000}>Send</button>
        {characterCount >= 3000 && <small className="chat-counter" data-tone={counterTone}>{characterCount.toLocaleString()} / 4,000</small>}
      </form>
      <p className="chat-typing" role="status" aria-atomic="true">
        {typingLabel && <><span aria-hidden="true">•••</span><span>{typingLabel}</span></>}
      </p>
    </div>
  </section>;
}
