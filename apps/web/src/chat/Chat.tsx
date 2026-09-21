import { useEffect, useRef, useState, type ReactNode } from "react";
import { ChatClient, type ChatViewState } from "./client.ts";
import type { ChatAuthor } from "./types.ts";
import "./chat.css";

const initialView: ChatViewState = {
  phase: "loading", online: false, spaceName: "Caper", channelName: "General",
  messages: [], hasMore: false, loadingOlder: false,
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
  const followLatest = useRef(true);

  useEffect(() => {
    const client = new ChatClient(setState);
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
  }, [state.messages.length]);

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
  const submit = async () => {
    if (!identityReady) return;
    setValidationError(undefined);
    const submitted = state.pendingSend?.text ?? draft;
    try {
      const sent = await clientRef.current?.send(submitted);
      if (sent) setDraft((current) => current === submitted ? "" : current);
    } catch (error) {
      setValidationError(error instanceof Error ? error.message : "Message could not be sent.");
    }
  };

  return <section className="chat-panel" aria-labelledby="chat-heading">
    <header className="chat-heading">
      <div className="chat-heading-title"><h2 id="chat-heading"># {channelName}</h2>
        {!state.online && <span className="chat-offline" role="status">{state.phase === "loading" ? "Loading" : "Offline"}</span>}
      </div>
      {headerActions}
    </header>

    <div className="chat-messages" ref={listRef} aria-live="polite" aria-busy={state.phase === "loading"} onScroll={(event) => {
      const list = event.currentTarget;
      followLatest.current = list.scrollHeight - list.scrollTop - list.clientHeight < 80;
    }}>
      {state.hasMore && <button className="chat-history-button" type="button" disabled={state.loadingOlder} onClick={() => void loadOlder()}>{state.loadingOlder ? "Loading…" : "Load older messages"}</button>}
      {state.phase === "loading" && !state.messages.length && <p className="chat-state" role="status">Loading messages…</p>}
      {state.phase === "error" && <div className="chat-state" role="alert"><p>{state.error}</p><button type="button" onClick={() => clientRef.current?.retryLoad()}>Try again</button></div>}
      {state.phase === "ready" && !state.messages.length && <div className="chat-state"><p>No messages yet.</p><small>Start the conversation in #{channelName}.</small></div>}
      {state.messages.map((message) => <article className="chat-message" key={message.id}>
        <div className="chat-avatar" aria-hidden="true">{message.author.name.slice(0, 1).toUpperCase()}</div>
        <div><header><strong>{message.author.name}</strong>{message.author.isGuest && <span>Guest</span>}<time dateTime={message.createdAt}>{timeLabel(message.createdAt)}</time></header><p>{message.content.text}</p></div>
      </article>)}
    </div>

    <div className="chat-composer">
      {state.phase === "ready" && state.error && <p className="chat-inline-error" role="alert">{state.error}</p>}
      {state.sessionError && <p className="chat-inline-error" role="alert">{state.sessionError} <button type="button" onClick={() => clientRef.current?.retrySession()}>Retry session</button></p>}
      {(state.sendError || validationError) && <p className="chat-inline-error" role="alert">{state.sendError || validationError} {state.sendError && state.pendingSend && <button type="button" onClick={() => void submit()}>Retry send</button>}</p>}
      <form onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <label className="sr-only" htmlFor="chat-message">Message {channelName}</label>
        <textarea id="chat-message" rows={1} value={draft} disabled={state.phase !== "ready"} placeholder={`Message #${channelName}`} onChange={(event) => { setDraft(event.target.value); setValidationError(undefined); }} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!sending) void submit(); }
        }} />
        <button type="submit" disabled={!identityReady || state.phase !== "ready" || sending || !draft.trim() || characterCount > 4_000}>{sending ? "Sending…" : "Send"}</button>
        {characterCount >= 3000 && <small className="chat-counter" data-tone={counterTone}>{characterCount.toLocaleString()} / 4,000</small>}
      </form>
    </div>
  </section>;
}
