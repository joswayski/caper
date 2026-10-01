import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { Paperclip } from "lucide-react";
import { Virtuoso, type VirtuosoHandle, type ListProps, type ContextProp } from "react-virtuoso";
import { ChatClient, initialChatView } from "./client.ts";
import { dateDivider } from "./dates.ts";
import { attachmentsOf, type ChatAttachment, type ChatAuthor, type GeneralChatHistory } from "./types.ts";
import { DraftAttachments, MessageAttachments, type DraftAttachment } from "./Attachments.tsx";
import { MAX_ATTACHMENTS, browserTransport, prepareFile, refreshAttachmentUrls, uploadPrepared } from "./uploads.ts";
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

export default function Chat({ name, signedIn, identityReady, channelId, channelName: expectedChannelName, initialHistory, initialHistoryError, showTitle = false, headerActions, messageSounds = true, onAuthorChange, onHistoryChange, onLocalPresenceChange, onOnlineChange }: { name: string; signedIn: boolean; identityReady: boolean; channelId?: string; channelName?: string; initialHistory?: GeneralChatHistory; initialHistoryError?: string; showTitle?: boolean; headerActions?: ReactNode; messageSounds?: boolean; onAuthorChange?: (author: ChatAuthor) => void; onHistoryChange?: (history: GeneralChatHistory) => void; onLocalPresenceChange?: (status: PresenceStatus) => void; onOnlineChange?: (online: boolean) => void }) {
  const [state, setState] = useState(() => initialChatView(initialHistory, initialHistoryError));
  const [showConnectionStatus, setShowConnectionStatus] = useState(false);
  const [firstItemIndex, setFirstItemIndex] = useState(INITIAL_ITEM_INDEX);
  const [draft, setDraft] = useState("");
  const [validationError, setValidationError] = useState<string>();
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const listRef = useRef<VirtuosoHandle>(null);
  const initialListRef = useRef<HTMLDivElement>(null);
  const [listReady, setListReady] = useState(false);
  // Virtuoso needs browser APIs; the server and first client render use the plain list.
  const [hydrated, setHydrated] = useState(false);
  useEffect(() => setHydrated(true), []);
  const composerRef = useRef<HTMLTextAreaElement>(null);
  const fileInputRef = useRef<HTMLInputElement>(null);
  const [drafts, setDrafts] = useState<DraftAttachment[]>([]);
  const uploads = useRef(new Map<string, AbortController>());
  const objectUrls = useRef(new Set<string>());
  const [freshUrls, setFreshUrls] = useState<Record<string, { url: string; previewUrl?: string }>>({});
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
        const sent = new Set((pending.attachments ?? []).map((attachment) => attachment.id));
        if (sent.size) setDrafts((current) => current.filter((item) => !item.attachment || !sent.has(item.attachment.id)));
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

  // Uploads belong to one channel; abandon them when it changes or unmounts.
  useEffect(() => () => {
    for (const controller of uploads.current.values()) controller.abort();
    uploads.current.clear();
    setDrafts([]);
  }, [channelId]);
  useEffect(() => () => { for (const url of objectUrls.current) URL.revokeObjectURL(url); }, []);

  const updateDraft = useCallback((key: string, change: Partial<DraftAttachment>) => {
    setDrafts((current) => current.map((item) => item.key === key ? { ...item, ...change } : item));
  }, []);

  const addFiles = (files: File[]) => {
    const channel = state.channelId;
    if (!signedIn || !channel || !files.length) return;
    const room = MAX_ATTACHMENTS - drafts.length;
    if (room <= 0) { setValidationError(`You can attach up to ${MAX_ATTACHMENTS} files.`); return; }
    setValidationError(files.length > room ? `Only ${room} more file${room === 1 ? "" : "s"} can be attached.` : undefined);
    for (const file of files.slice(0, room)) {
      const key = crypto.randomUUID();
      const localUrl = file.type.startsWith("image/") || file.type.startsWith("video/") ? URL.createObjectURL(file) : undefined;
      if (localUrl) objectUrls.current.add(localUrl);
      const controller = new AbortController();
      uploads.current.set(key, controller);
      setDrafts((current) => [...current, { key, name: file.name, kind: file.type.startsWith("image/") ? "image" : "file", localUrl, sourceSize: file.size, progress: 0 }]);
      void (async () => {
        try {
          const prepared = await prepareFile(file);
          updateDraft(key, { name: prepared.name, kind: prepared.kind, storedSize: prepared.blob.size });
          const attachment = await uploadPrepared(channel, prepared, browserTransport, (progress) => updateDraft(key, { progress }), controller.signal);
          updateDraft(key, { attachment, progress: 1 });
        } catch (error) {
          if (!controller.signal.aborted) updateDraft(key, { error: error instanceof Error ? error.message : "Upload failed." });
        } finally {
          uploads.current.delete(key);
        }
      })();
    }
  };

  const removeDraft = (key: string) => {
    uploads.current.get(key)?.abort();
    uploads.current.delete(key);
    setDrafts((current) => current.filter((item) => item.key !== key));
  };

  const refreshUrls = useCallback((ids: string[]) => {
    void refreshAttachmentUrls(ids).then((urls) => { if (Object.keys(urls).length) setFreshUrls((current) => ({ ...current, ...urls })); });
  }, []);
  const withFreshUrls = (attachments: ChatAttachment[]) => attachments.map((attachment) => ({ ...attachment, ...freshUrls[attachment.id] }));

  useEffect(() => { onOnlineChange?.(state.online); }, [state.online, onOnlineChange]);

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
  const channelName = (expectedChannelName ?? state.channelName).toLowerCase();
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
    if (!identityReady || sending || state.sendRejected) return;
    setValidationError(undefined);
    followLatest.current = true;
    const submitted = state.pendingSend?.text ?? draft;
    if (!state.pendingSend && drafts.some((item) => !item.attachment)) {
      setValidationError(drafts.some((item) => item.error) ? "Remove files that failed to upload first." : "Wait for files to finish uploading.");
      return;
    }
    // Pending messages show local copies until the server's signed URLs arrive.
    const attachments = drafts.flatMap((item) => item.attachment
      ? [{ ...item.attachment, url: item.localUrl, previewUrl: item.localUrl }] : []);
    try {
      await clientRef.current?.send(submitted, attachments);
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
      <article className={`chat-message${pending ? " chat-message-pending" : ""}`} data-message-key={message.clientMessageId}>
      <div className="chat-avatar"><Avatar avatarId={author?.avatarId} name={author?.name ?? name} /></div>
      <div>
        <header><strong>{author?.name ?? name}</strong>{author?.isGuest && <span>Guest</span>}<time dateTime={message.createdAt}>{hydrated ? timeLabel(message.createdAt) : ""}</time></header>
        {("content" in message ? message.content.text : message.text) && <p>{"content" in message ? message.content.text : message.text}</p>}
        <MessageAttachments attachments={"content" in message ? withFreshUrls(attachmentsOf(message)) : message.attachments ?? []} onExpired={pending ? undefined : refreshUrls} />
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

  const canAttach = signedIn && state.phase === "ready" && !!state.channelId;
  return <section className="chat-panel" aria-labelledby="chat-heading"
    onDragOver={(event) => { if (canAttach && event.dataTransfer.types.includes("Files")) event.preventDefault(); }}
    onDrop={(event) => { if (!canAttach || !event.dataTransfer.files.length) return; event.preventDefault(); addFiles([...event.dataTransfer.files]); }}>
    <header className="chat-heading">
      <h2 id="chat-heading" className={showTitle ? "chat-channel-title" : "sr-only"}># {channelName}</h2>
      {headerActions}
      {!state.online && showConnectionStatus && <span className="chat-offline" role="status">{state.phase === "error" ? "Offline" : "Connecting…"}</span>}
      {state.phase === "ready" && state.error && <div className="chat-refresh-error" role="alert">{state.error} <button type="button" onClick={() => clientRef.current?.retryLoad()}>Retry</button></div>}
    </header>

    <div className="chat-messages" aria-busy={state.phase === "loading"}>
      {state.phase === "loading" && <p className="chat-state" role="status">Loading messages…</p>}
      {state.phase === "error" && <div className="chat-state" role="alert"><p>{state.error}</p><button type="button" onClick={() => clientRef.current?.retryLoad()}>Try again</button></div>}
      {state.phase === "ready" && !messages.length && <div className="chat-state"><p>No messages yet.</p><small>Start the conversation in #{channelName}.</small></div>}
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
      <p className="sr-only" aria-live="polite" aria-atomic="true">{state.phase === "ready" && latestMessage && `${latestMessage.author.name}: ${latestMessage.content.text || `sent ${attachmentsOf(latestMessage).length === 1 ? "a file" : `${attachmentsOf(latestMessage).length} files`}`}`}</p>
    </div>

    <p className="chat-typing" role="status" aria-atomic="true">
      <span className="chat-typing-content" data-visible={!!typingLabel} aria-hidden={!typingLabel}>
        {displayedTypingLabel && <><span className="chat-typing-dots" aria-hidden="true"><i /><i /><i /></span><span>{displayedTypingLabel}</span></>}
      </span>
    </p>

    <div className="chat-composer">
      {state.sessionError && <p className="chat-inline-error" role="alert">{state.sessionError} <button type="button" onClick={() => clientRef.current?.retrySession()}>Retry session</button></p>}
      {validationError && <p className="chat-inline-error" role="alert">{validationError}</p>}
      <DraftAttachments drafts={drafts} onRemove={removeDraft} />
      <form data-attach={canAttach || undefined} onSubmit={(event) => { event.preventDefault(); void submit(); }}>
        {canAttach && <>
          <button type="button" className="chat-attach" aria-label="Attach files" title="Attach files" disabled={drafts.length >= MAX_ATTACHMENTS} onClick={() => fileInputRef.current?.click()}><Paperclip size={17} aria-hidden="true" /></button>
          <input ref={fileInputRef} type="file" multiple hidden onChange={(event) => { addFiles([...(event.target.files ?? [])]); event.target.value = ""; }} />
        </>}
        <label className="sr-only" htmlFor="chat-message">Message {channelName}</label>
        <textarea ref={composerRef} id="chat-message" rows={1} value={draft} disabled={state.phase !== "ready"} enterKeyHint="send" aria-describedby="chat-composer-hint" placeholder={`Message #${channelName}`} onChange={(event) => { setDraft(event.target.value); setValidationError(undefined); clientRef.current?.setTyping(!!event.target.value.trim()); }} onPaste={(event) => {
          const files = [...event.clipboardData.files];
          if (canAttach && files.length) { event.preventDefault(); addFiles(files); }
        }} onBlur={() => clientRef.current?.setTyping(false)} onKeyDown={(event) => {
          if (event.key === "Enter" && !event.shiftKey && !event.nativeEvent.isComposing) { event.preventDefault(); if (!sending) void submit(); }
        }} />
        <span id="chat-composer-hint" className="sr-only">Enter to send. Shift+Enter for a new line.</span>
        {characterCount >= 3000 && <small className="chat-counter" data-tone={counterTone}>{characterCount.toLocaleString()} / 4,000</small>}
      </form>
    </div>
  </section>;
}
