import { useEffect, useRef, useState, type ReactNode } from "react";
import { FloatingFocusManager, FloatingOverlay, FloatingPortal, useDismiss, useFloating, useInteractions, useRole } from "@floating-ui/react";
import { Forward, X } from "lucide-react";
import Avatar from "../components/Avatar";
import { apiError, ChatHistoryError } from "./client.ts";
import { emojiAsset, emojiCode } from "./emoji.ts";
import { isChatMessage, sequence, type ChatMessage } from "./types.ts";

export interface ForwardTarget { messageId: string; anchor: HTMLElement }
interface Destination { id: string; name: string; spaceName: string; direct: boolean }
interface Conversation { root: ChatMessage | null; messages: ChatMessage[]; cursor: string; hasMore: boolean }

function ForwardDialog({ title, anchor, onClose, children }: { title: string; anchor: HTMLElement; onClose: () => void; children: ReactNode }) {
  const returnFocus = useRef(anchor);
  const { refs, context } = useFloating({ open: true, onOpenChange: (open) => { if (!open) onClose(); } });
  const { getFloatingProps } = useInteractions([useDismiss(context), useRole(context)]);
  return <FloatingPortal><FloatingOverlay lockScroll className="chat-forward-overlay">
    <FloatingFocusManager context={context} returnFocus={returnFocus}>
      <section className="chat-forward-dialog" ref={refs.setFloating} aria-label={title} {...getFloatingProps()}>
        <header className="chat-reaction-picker-heading"><strong>{title}</strong><button type="button" aria-label={`Close ${title.toLowerCase()}`} onClick={onClose}><X size={18} /></button></header>
        {children}
      </section>
    </FloatingFocusManager>
  </FloatingOverlay></FloatingPortal>;
}

function OriginalMessage({ message }: { message: ChatMessage }) {
  return <article className="chat-forward-original">
    <header><span className="chat-avatar"><Avatar avatarId={message.author.avatarId} name={message.author.name} /></span>
      <strong>{message.author.name}</strong>{message.editedAt && <small title={message.editedAt}>edited</small>}</header>
    <p>{message.content.text}</p>
    {!!message.reactions?.length && <div className="chat-forward-reactions" aria-label="Original reactions">
      {message.reactions.map(({ emoji, authorIds }) => <span key={emoji} aria-label={`${emoji}, ${authorIds.length} reactions`}>
        <img src={emojiAsset(emojiCode(emoji))} width={18} height={18} alt={emoji} /><span>{authorIds.length}</span>
      </span>)}
    </div>}
  </article>;
}

export function ForwardCard({ message, onOpen }: { message: ChatMessage; onOpen: (anchor: HTMLElement) => void }) {
  if (!message.forward) return null;
  const original = message.forward.message;
  return <div className="chat-forward-card">
    <small className="chat-forward-label"><Forward size={12} aria-hidden="true" />Forwarded · live</small>
    {original ? <><OriginalMessage message={original} /><button type="button" onClick={(event) => onOpen(event.currentTarget)}>
      {original.thread?.replyCount ? `${original.thread.replyCount} ${original.thread.replyCount === 1 ? "reply" : "replies"} · ` : ""}View conversation</button></>
      : <p className="chat-forward-unavailable">Original conversation unavailable.</p>}
  </div>;
}

export function ForwardPicker({ message, target, onClose, onForward, onSent }: {
  message: ChatMessage; target: ForwardTarget; onClose: () => void;
  onForward: (destination: string, messageId: string, key: string, text: string) => Promise<ChatMessage>;
  onSent: (destination: Destination) => void;
}) {
  const [destinations, setDestinations] = useState<Destination[]>();
  const [search, setSearch] = useState("");
  const [destination, setDestination] = useState<Destination>();
  const [note, setNote] = useState("");
  const [attempt, setAttempt] = useState(0);
  const [error, setError] = useState<string>();
  const [sending, setSending] = useState(false);
  const [command, setCommand] = useState<{ destination: Destination; key: string; text: string }>();
  const mounted = useRef(true);
  useEffect(() => { mounted.current = true; return () => { mounted.current = false; }; }, []);
  useEffect(() => {
    const controller = new AbortController();
    setError(undefined);
    void (async () => {
      try {
        const response = await fetch("/api/chat/forward-destinations", { cache: "no-store", signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]) });
        if (!response.ok) throw await apiError(response, "Destinations are unavailable.");
        const data = await response.json() as { destinations?: Destination[] };
        if (!Array.isArray(data.destinations) || !data.destinations.every((item) => !!item && typeof item.id === "string" && typeof item.name === "string" && typeof item.spaceName === "string" && typeof item.direct === "boolean")) throw new Error("Invalid forward destinations.");
        if (!controller.signal.aborted) setDestinations(data.destinations.sort((a, b) => `${a.spaceName} ${a.name}`.localeCompare(`${b.spaceName} ${b.name}`)));
      } catch (reason) { if (!controller.signal.aborted) setError(reason instanceof Error ? reason.message : "Destinations are unavailable."); }
    })();
    return () => controller.abort();
  }, [attempt]);
  const send = async () => {
    if (sending || !destination) return;
    const intent = command ?? { destination, key: crypto.randomUUID(), text: note.trim() };
    setCommand(intent); setSending(true); setError(undefined);
    try {
      await onForward(intent.destination.id, message.id, intent.key, intent.text);
      if (mounted.current) onSent(intent.destination);
    } catch (reason) {
      if (mounted.current) {
        const rejected = reason instanceof ChatHistoryError && [400, 401, 403, 404, 409, 422].includes(reason.status);
        if (rejected) setCommand(undefined);
        setError(`${rejected ? "Not sent." : "Not confirmed yet. Retry checks the same forward."} ${reason instanceof Error ? reason.message : "Try again."}`);
      }
    }
    finally { if (mounted.current) setSending(false); }
  };
  const visible = destinations?.filter((item) => `${item.spaceName} ${item.name}`.toLocaleLowerCase().includes(search.toLocaleLowerCase()));
  return <ForwardDialog title="Forward message" anchor={target.anchor} onClose={onClose}>
    <div className="chat-forward-body">
      <p className="chat-forward-disclosure">Shares this message and its conversation live, including future edits, reactions and replies. People in the destination can read and forward it.</p>
      <OriginalMessage message={message.forward?.message ?? message} />
      <label>Send to<input type="search" placeholder="Find a channel or DM" value={search} disabled={!!command} onChange={(event) => setSearch(event.target.value)} /></label>
      <div className="chat-forward-destinations" role="radiogroup" aria-label="Forward destination">
        {visible?.map((item) => <label key={item.id}><input type="radio" name="forward-destination" checked={destination?.id === item.id} disabled={!!command} onChange={() => setDestination(item)} />
          <span><strong>{item.direct ? "" : "# "}{item.name}</strong><small>{item.spaceName}</small></span></label>)}
        {!destinations && !error && <p role="status">Loading destinations…</p>}
        {visible?.length === 0 && <p>{destinations?.length ? "No matching destination." : "Join a channel or start a DM to forward here."}</p>}
      </div>
      <label>Add a note (optional)<textarea rows={2} value={note} disabled={!!command} onChange={(event) => setNote(event.target.value)} /></label>
      {error && <p role="alert" className="chat-forward-error">{error}{!destinations && <button type="button" onClick={() => setAttempt((value) => value + 1)}>Retry loading</button>}</p>}
      <button type="button" className="chat-forward-send" disabled={!destination || sending || Array.from(note).length > 4000} onClick={() => void send()}>{sending ? "Forwarding…" : command ? "Retry forward" : "Forward"}</button>
    </div>
  </ForwardDialog>;
}

export function ForwardConversation({ message, target, onClose }: { message: ChatMessage; target: ForwardTarget; onClose: () => void }) {
  const [conversation, setConversation] = useState<Conversation>();
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string>();
  const [attempt, setAttempt] = useState(0);
  const oldest = useRef<string | undefined>(undefined);
  const controller = useRef<AbortController | undefined>(undefined);
  const path = `/api/chat/channels/${encodeURIComponent(message.channelId)}/forwards/${encodeURIComponent(message.id)}/thread`;
  const page = async (signal: AbortSignal, before?: string): Promise<Conversation> => {
    const response = await fetch(`${path}${before ? `?before=${encodeURIComponent(before)}` : ""}`, { cache: "no-store", signal: AbortSignal.any([signal, AbortSignal.timeout(10_000)]) });
    if (!response.ok) throw await apiError(response, "Conversation is unavailable.");
    const data = await response.json() as Conversation;
    if (!data || !(data.root === null || isChatMessage(data.root)) || !Array.isArray(data.messages) || !data.messages.every(isChatMessage) || typeof data.hasMore !== "boolean" || typeof data.cursor !== "string") throw new Error("Invalid forwarded conversation.");
    sequence(data.cursor);
    return data;
  };
  useEffect(() => {
    controller.current?.abort();
    const request = new AbortController(); controller.current = request;
    setLoading(true); setError(undefined);
    void (async () => {
      try {
        const fresh = await page(request.signal);
        // Refresh the loaded range too: edits and reactions on old replies do
        // not change their creation sequences or move them onto the last page.
        while (fresh.hasMore && oldest.current && fresh.messages[0] && sequence(fresh.messages[0].seq) > sequence(oldest.current)) {
          const earlier = await page(request.signal, fresh.messages[0].seq);
          if (!earlier.messages.length) { fresh.hasMore = false; break; }
          fresh.messages = [...earlier.messages, ...fresh.messages]; fresh.hasMore = earlier.hasMore;
        }
        if (!request.signal.aborted) { oldest.current = fresh.messages[0]?.seq; setConversation(fresh); }
      } catch (reason) { if (!request.signal.aborted) { setConversation(undefined); setError(reason instanceof Error ? reason.message : "Conversation is unavailable."); } }
      finally { if (!request.signal.aborted) setLoading(false); }
    })();
    return () => request.abort();
  }, [path, message.forward?.seq, attempt]);
  const loadOlder = async () => {
    if (!conversation?.messages[0] || loading || !controller.current) return;
    const request = controller.current;
    setLoading(true); setError(undefined);
    try {
      const earlier = await page(request.signal, conversation.messages[0].seq);
      if (!request.signal.aborted) {
        oldest.current = earlier.messages[0]?.seq ?? oldest.current;
        setConversation({ ...conversation, messages: [...earlier.messages, ...conversation.messages], hasMore: earlier.hasMore });
      }
    } catch (reason) { if (!request.signal.aborted) setError(reason instanceof Error ? reason.message : "Older replies are unavailable."); }
    finally { if (!request.signal.aborted) setLoading(false); }
  };
  const replyCount = conversation?.root?.thread?.replyCount ?? conversation?.messages.length ?? 0;
  return <ForwardDialog title="Forwarded conversation" anchor={target.anchor} onClose={onClose}>
    <div className="chat-forward-body">
      <p className="chat-forward-disclosure">Live · Read-only original. Replies you add to the forward stay in the destination conversation.</p>
      {conversation?.root ? <><OriginalMessage message={conversation.root} />
        <h3>{replyCount} {replyCount === 1 ? "reply" : "replies"}</h3>
        {conversation.hasMore && <button type="button" disabled={loading} onClick={() => void loadOlder()}>Load older replies</button>}
        {conversation.messages.map((reply) => <OriginalMessage key={reply.id} message={reply} />)}
        {!conversation.messages.length && <p>No replies yet.</p>}
      </> : !loading && !error && <p>Original conversation unavailable.</p>}
      {loading && <p role="status">Updating conversation…</p>}
      {error && <p role="alert" className="chat-forward-error">{error}<button type="button" onClick={() => setAttempt((value) => value + 1)}>Retry</button></p>}
    </div>
  </ForwardDialog>;
}
