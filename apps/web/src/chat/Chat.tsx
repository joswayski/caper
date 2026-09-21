import { useEffect, useRef, useState } from "react";
import { ChatClient, type ChatViewState } from "./client.ts";
import "./chat.css";

const initialView: ChatViewState = {
  phase: "loading", online: false, spaceName: "Caper", channelName: "General",
  messages: [], hasMore: false, loadingOlder: false,
};

function timeLabel(value: string) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "" : new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" }).format(date);
}

export default function Chat({ name, signedIn }: { name: string; signedIn: boolean }) {
  const [state, setState] = useState(initialView);
  const [draft, setDraft] = useState("");
  const [validationError, setValidationError] = useState<string>();
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const listRef = useRef<HTMLDivElement>(null);
  const followLatest = useRef(true);

  useEffect(() => {
    const client = new ChatClient(setState);
    clientRef.current = client;
    client.start(name, signedIn);
    return () => { client.stop(); clientRef.current = undefined; };
  }, [name, signedIn]);

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
  const submit = async () => {
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
      <div><p className="eyebrow">{state.spaceName}</p><h2 id="chat-heading"># {state.channelName}</h2></div>
      <span className={state.online ? "chat-live" : "chat-offline"}>{state.online ? "Live" : state.phase === "loading" ? "Loading" : "Offline"}</span>
    </header>

    <div className="chat-messages" ref={listRef} aria-live="polite" aria-busy={state.phase === "loading"} onScroll={(event) => {
      const list = event.currentTarget;
      followLatest.current = list.scrollHeight - list.scrollTop - list.clientHeight < 80;
    }}>
      {state.hasMore && <button className="chat-history-button" type="button" disabled={state.loadingOlder} onClick={() => void loadOlder()}>{state.loadingOlder ? "Loading…" : "Load older messages"}</button>}
      {state.phase === "loading" && !state.messages.length && <p className="chat-state" role="status">Loading messages…</p>}
      {state.phase === "error" && <div className="chat-state" role="alert"><p>{state.error}</p><button type="button" onClick={() => clientRef.current?.retryLoad()}>Try again</button></div>}
      {state.phase === "ready" && !state.messages.length && <div className="chat-state"><p>No messages yet.</p><small>Start the conversation in General.</small></div>}
      {state.messages.map((message) => <article className="chat-message" key={message.id}>
        <div className="chat-avatar" aria-hidden="true">{message.author.name.slice(0, 1).toUpperCase()}</div>
        <div><header><strong>{message.author.name}</strong>{message.author.isGuest && <span>Guest</span>}<time dateTime={message.createdAt}>{timeLabel(message.createdAt)}</time></header><p>{message.content.text}</p></div>
      </article>)}
    </div>

    <div className="chat-composer">
      <p className="chat-identity">Messaging as <strong>{state.author?.name ?? name}</strong>{state.author?.isGuest !== false && <span>Guest</span>}</p>
      {state.phase === "ready" && state.error && <p className="chat-inline-error" role="alert">{state.error}</p>}
      {state.sessionError && <p className="chat-inline-error" role="alert">{state.sessionError} <button type="button" onClick={() => clientRef.current?.retrySession()}>Retry session</button></p>}
      {(state.sendError || validationError) && <p className="chat-inline-error" role="alert">{state.sendError || validationError} {state.sendError && state.pendingSend && <button type="button" onClick={() => void submit()}>Retry send</button>}</p>}
      <form onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        <label className="sr-only" htmlFor="chat-message">Message General</label>
        <textarea id="chat-message" rows={2} value={draft} disabled={state.phase !== "ready"} placeholder={`Message #${state.channelName}`} onChange={(event) => { setDraft(event.target.value); setValidationError(undefined); }} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!sending) void submit(); }
        }} />
        <div><small>{Array.from(draft).length.toLocaleString()} / 4,000</small><button type="submit" disabled={state.phase !== "ready" || sending || !draft.trim() || Array.from(draft).length > 4_000}>{sending ? "Sending…" : "Send"}</button></div>
      </form>
      <p className="chat-public-note">Public demo. Messages are saved and visible to everyone.</p>
    </div>
  </section>;
}
