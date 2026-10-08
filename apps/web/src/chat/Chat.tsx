import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowLeft, Ban, ChevronDown, MoreHorizontal, Pin } from "lucide-react";
import { Virtuoso, type VirtuosoHandle, type ListProps, type ContextProp } from "react-virtuoso";
import { ChatClient, initialChatView } from "./client.ts";
import MessageReactions, { type ReactionSave } from "./MessageReactions.tsx";
import MessageActions, { type MessageActionTarget } from "./MessageActions.tsx";
import ReactorsPanel, { type ReactorsTarget } from "./ReactorsPanel.tsx";
import ThreadPanel from "./ThreadPanel.tsx";
import MessageEditor from "./MessageEditor.tsx";
import MessageHistory from "./MessageHistory.tsx";
import { MessageSquare } from "lucide-react";
import { ForwardCard, ForwardConversation, ForwardPicker, type ForwardTarget } from "./Forwarding.tsx";
import { dateDivider } from "./dates.ts";
import { isChannelMessage, type ChatAuthor, type GeneralChatHistory } from "./types.ts";
import { appGateway, type PresenceStatus } from "../gateway/client.ts";
import Avatar from "../components/Avatar";
import Composer, { type ComposerHandle } from "./Composer.tsx";
import { mentionCardPerson, mentionSegments, mentionsAccount, type MentionCandidate } from "./mentions.ts";
import MentionCard, { type MentionCardTarget } from "./MentionCard.tsx";
import { blockedLabel, blockedRuns, type BlockedRun } from "./blocked.ts";
import { unblock, useBlockedIds } from "../spaces/blocks.ts";
import "./chat.css";

// Virtuoso's prepend index is local bookkeeping, never the bigint server cursor.
const INITIAL_ITEM_INDEX = 1_000_000_000;

function timeLabel(value: string, formatter: Intl.DateTimeFormat) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "" : formatter.format(date);
}

interface HistoryContext {
  hasMore: boolean;
  loadingOlder: boolean;
  olderError?: string;
  loadOlder: () => void;
  onListReady?: () => void;
}

function HistoryHeader({ context }: { context?: HistoryContext }) {
  return (
    <div className="chat-history" role="status">
      {context?.olderError ? (
        <>
          <span>Couldn’t load older messages.</span>
          <button type="button" onClick={context.loadOlder}>
            Retry
          </button>
        </>
      ) : context?.hasMore ? (
        <button type="button" disabled={context.loadingOlder} onClick={context.loadOlder}>
          {context.loadingOlder ? "Loading…" : "Load older messages"}
        </button>
      ) : (
        <span>Beginning of conversation</span>
      )}
    </div>
  );
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

export default function Chat({
  name,
  signedIn,
  accountId,
  identityReady,
  channelId,
  channelName: expectedChannelName,
  direct = false,
  onReadCursor,
  initialHistory,
  initialHistoryError,
  showTitle = false,
  headerLeading,
  channelMenu,
  headerActions,
  readOnly = false,
  composerNotice,
  composerBanner,
  onBlockAuthor,
  messageSounds = true,
  onAuthorChange,
  onHistoryChange,
  onLocalPresenceChange,
  onOnlineChange,
  mentionMembers,
  mentionDirectory = [],
  onMessagePerson,
}: {
  name: string;
  signedIn: boolean;
  /** Known viewer identity, independent of the chat sending capability. */
  accountId?: string;
  identityReady: boolean;
  channelId?: string;
  channelName?: string;
  direct?: boolean;
  onReadCursor?: (seq: string) => void;
  initialHistory?: GeneralChatHistory;
  initialHistoryError?: string;
  showTitle?: boolean;
  headerLeading?: ReactNode;
  channelMenu?: ReactNode;
  headerActions?: ReactNode;
  readOnly?: boolean;
  composerNotice?: ReactNode;
  /** Shown above an active composer. */ composerBanner?: ReactNode;
  /** Offers Block in message actions; the caller confirms. */ onBlockAuthor?: (author: ChatAuthor) => void;
  messageSounds?: boolean;
  onAuthorChange?: (author: ChatAuthor) => void;
  onHistoryChange?: (history: GeneralChatHistory) => void;
  onLocalPresenceChange?: (status: PresenceStatus) => void;
  onOnlineChange?: (online: boolean) => void;
  /** People `@` can suggest; undefined until loaded. */ mentionMembers?: MentionCandidate[];
  /** Profile-card lookup, most specific first. */ mentionDirectory?: MentionCandidate[];
  onMessagePerson?: (username: string) => Promise<void>;
}) {
  const [state, setState] = useState(() => initialChatView(initialHistory, initialHistoryError));
  const viewerId = accountId ?? state.author?.id;
  const [showConnectionStatus, setShowConnectionStatus] = useState(false);
  const [firstItemIndex, setFirstItemIndex] = useState(INITIAL_ITEM_INDEX);
  // The composer owns the draft; the conversation only needs to know one exists.
  const composerRef = useRef<ComposerHandle>(null);
  const [hasDraft, setHasDraft] = useState(false);
  const channelMenuRef = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      const menu = channelMenuRef.current;
      if (menu && !menu.contains(event.target as Node)) menu.open = false;
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, []);
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const [actionTarget, setActionTarget] = useState<MessageActionTarget>();
  const [reactorsTarget, setReactorsTarget] = useState<ReactorsTarget>();
  const [forwardTarget, setForwardTarget] = useState<ForwardTarget>();
  const [conversationTarget, setConversationTarget] = useState<ForwardTarget>();
  const [editTarget, setEditTarget] = useState<string>();
  const [historyTarget, setHistoryTarget] = useState<string>();
  const [actionStatus, setActionStatus] = useState("");
  const [reactionSaves, setReactionSaves] = useState<Record<string, ReactionSave | undefined>>({});
  const [mentionCard, setMentionCard] = useState<MentionCardTarget>();
  const [showPins, setShowPins] = useState(false);
  const [pinning, setPinning] = useState<Set<string>>(() => new Set());
  const [pinError, setPinError] = useState<{ messageId: string; active: boolean; text: string }>();
  // Runs of blocked messages the reader chose to show, by their first message.
  const [revealedRuns, setRevealedRuns] = useState<ReadonlySet<string>>(() => new Set());
  const blockedIds = useBlockedIds();
  const press = useRef<{ timer: ReturnType<typeof setTimeout>; x: number; y: number; pointerId: number }>(undefined);
  const suppressClick = useRef(false);
  const cancelPress = () => {
    clearTimeout(press.current?.timer);
    press.current = undefined;
  };
  useEffect(() => {
    setActionTarget(undefined);
    setReactorsTarget(undefined);
    setMentionCard(undefined);
    setForwardTarget(undefined);
    setConversationTarget(undefined);
    setEditTarget(undefined);
    setHistoryTarget(undefined);
    setActionStatus("");
    setReactionSaves({});
    setShowPins(false);
    setPinning(new Set());
    setPinError(undefined);
    setRevealedRuns(new Set());
    // The drawer can appear under the held finger. Its release click must not
    // activate a newly rendered action, even though that action is in a portal.
    const resetClick = () => {
      suppressClick.current = false;
    };
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
  useEffect(() => {
    setActionTarget(undefined);
    setReactorsTarget(undefined);
    setForwardTarget(undefined);
    setConversationTarget(undefined);
    setEditTarget(undefined);
    setHistoryTarget(undefined);
  }, [state.author?.id, state.channelId]);
  useEffect(() => {
    if (readOnly) setEditTarget(undefined);
  }, [readOnly]);
  useEffect(() => {
    if (state.phase === "error") {
      setEditTarget(undefined);
      setHistoryTarget(undefined);
    }
  }, [state.phase]);
  const isTouchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;
  const openActions = (messageId: string, anchor: HTMLElement, inThread: boolean) =>
    setActionTarget({ messageId, anchor, mode: "actions", drawer: isTouchLayout(), inThread });
  const openThread = (rootId: string) => {
    setActionTarget(undefined);
    setReactorsTarget(undefined);
    clientRef.current?.setTyping(false);
    void clientRef.current?.openThread(rootId);
  };
  const prefetchThread = (rootId: string) => void clientRef.current?.prefetchThread(rootId);
  const closeThread = useCallback(() => clientRef.current?.closeThread(), []);
  const showReactors = (messageId: string, emoji: string, anchor: HTMLElement) => {
    const drawer = isTouchLayout();
    // Like the actions drawer, the sheet can open under a held finger.
    if (drawer) suppressClick.current = true;
    setActionTarget(undefined);
    setReactorsTarget({ messageId, emoji, anchor, drawer });
  };
  const closeReactors = useCallback(() => setReactorsTarget(undefined), []);
  const react = async (messageId: string, emoji: string, active: boolean) => {
    const client = clientRef.current;
    if (readOnly || !state.author || !client) return;
    setReactionSaves((current) => ({ ...current, [messageId]: undefined }));
    try {
      await client.setReaction(messageId, emoji, active);
    } catch (error) {
      if (clientRef.current === client)
        setReactionSaves((current) => ({
          ...current,
          [messageId]: {
            emoji,
            active,
            error: error instanceof Error ? error.message : "Reaction could not be saved.",
          },
        }));
    }
  };
  const findMessage = (id?: string) =>
    state.messages.find((message) => message.id === id) ?? state.pinnedMessages.find((message) => message.id === id);
  const actionMessage = findMessage(actionTarget?.messageId);
  const reactorsMessage = state.messages.find((message) => message.id === reactorsTarget?.messageId);
  const forwardMessage = state.messages.find((message) => message.id === forwardTarget?.messageId);
  const conversationMessage = conversationTarget
    ? [...state.messages, ...state.pinnedMessages].find((message) => message.id === conversationTarget.messageId)
    : undefined;
  const closeForward = useCallback(() => setForwardTarget(undefined), []);
  const closeConversation = useCallback(() => setConversationTarget(undefined), []);
  const editMessage = findMessage(editTarget);
  const historyMessage = findMessage(historyTarget);
  const openEdit = (messageId: string) => {
    if (findMessage(messageId)?.forward) return;
    setActionTarget(undefined);
    setHistoryTarget(undefined);
    setEditTarget(messageId);
  };
  const openHistory = (messageId: string) => {
    if (findMessage(messageId)?.forward) return;
    setActionTarget(undefined);
    setEditTarget(undefined);
    setHistoryTarget(messageId);
  };
  const pin = async (messageId: string, active: boolean) => {
    const client = clientRef.current;
    if (!client || readOnly) return;
    setPinning((current) => new Set(current).add(messageId));
    setPinError(undefined);
    try {
      await client.setPin(messageId, active);
      if (clientRef.current === client) setActionStatus(active ? "Message pinned." : "Message unpinned.");
    } catch (error) {
      if (clientRef.current === client) {
        const message = error instanceof Error ? error.message : "Pin could not be saved.";
        setPinError({ messageId, active, text: message });
        setActionStatus(message);
      }
    } finally {
      if (clientRef.current === client)
        setPinning((current) => {
          const next = new Set(current);
          next.delete(messageId);
          return next;
        });
    }
  };
  const listRef = useRef<VirtuosoHandle>(null);
  const initialListRef = useRef<HTMLDivElement>(null);
  const [listReady, setListReady] = useState(false);
  // Virtuoso needs browser APIs; the server and first client render use the plain list.
  const [hydrated, setHydrated] = useState(false);
  useEffect(() => setHydrated(true), []);
  useEffect(() => {
    if (!state.author || readOnly) return;
    // Warm the code/data after chat settles, without mounting the picker or
    // fetching the image catalog. Opening still handles a failed import.
    const timer = setTimeout(() => {
      void import("./ReactionPicker.tsx").catch(() => {});
    }, 1_000);
    return () => clearTimeout(timer);
  }, [state.author?.id, readOnly]);
  const followLatest = useRef(true);
  const latestMessage = state.messages.at(-1);
  const [announcement, setAnnouncement] = useState("");
  useEffect(() => {
    if (state.phase === "ready" && latestMessage)
      setAnnouncement(`${latestMessage.author.name}: ${latestMessage.content.text}`);
  }, [latestMessage?.id, state.phase]);
  // Reaction events advance the conversation stream without adding a message.
  // HTTP reaction snapshots do not advance this committed replay cursor.
  const readCursor = clientRef.current?.readCursor() ?? initialHistory?.cursor ?? latestMessage?.seq ?? "0";
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

  const keepLatestInView = useCallback(() => {
    if (followLatest.current) listRef.current?.autoscrollToBottom();
  }, []);
  const followOwnSend = useCallback(() => {
    followLatest.current = true;
  }, []);
  const setTyping = useCallback((active: boolean) => clientRef.current?.setTyping(active), []);
  const sendDraft = useCallback((text: string) => clientRef.current?.send(text), []);

  useEffect(() => {
    let pendingId: string | undefined;
    let firstMessageId: string | undefined;
    const client = new ChatClient(
      (next) => {
        if (next.phase !== "ready") {
          firstMessageId = undefined;
          setListReady(false);
          setFirstItemIndex(INITIAL_ITEM_INDEX);
        } else if (next.channelMessages?.[0]?.id !== firstMessageId) {
          const prepended = next.channelMessages?.findIndex((message) => message.id === firstMessageId) ?? -1;
          if (prepended > 0) setFirstItemIndex((index) => index - prepended);
          firstMessageId = next.channelMessages?.[0]?.id;
        }
        const pending = next.pendingSend;
        // The composer clears the draft this came from.
        if (pending && !pending.threadRootId && pending.clientMessageId !== pendingId) followLatest.current = true;
        pendingId = pending?.clientMessageId;
        setState(next);
      },
      channelId,
      { sounds: messageSounds },
    );
    clientRef.current = client;
    client.setSilencedAuthors(blockedIdsRef.current);
    client.start(initialHistory, initialHistoryError);
    return () => {
      const history = client.snapshotHistory();
      if (history) onHistoryChange?.(history);
      client.stop();
      clientRef.current = undefined;
    };
  }, [channelId]);

  const blockedIdsRef = useRef(blockedIds);
  useEffect(() => {
    blockedIdsRef.current = blockedIds;
    clientRef.current?.setSilencedAuthors(blockedIds);
  }, [blockedIds]);

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

  useEffect(() => {
    clientRef.current?.setSounds(messageSounds);
  }, [messageSounds]);

  useEffect(() => {
    onOnlineChange?.(state.online);
  }, [state.online, onOnlineChange]);

  useEffect(() => {
    if (identityReady && (!readOnly || signedIn)) clientRef.current?.identify(name, signedIn, accountId);
  }, [identityReady, name, signedIn, accountId, readOnly]);

  useEffect(() => {
    if (state.author) onAuthorChange?.(state.author);
  }, [state.author, onAuthorChange]);

  useEffect(() => {
    // Only local sends override the reader's position. Virtuoso follows incoming
    // messages using its immediate bottom state; atBottomStateChange is delayed.
    if (state.pendingSend && !state.pendingSend.threadRootId)
      listRef.current?.scrollToIndex({ index: "LAST", align: "end" });
  }, [state.pendingSend?.clientMessageId]);

  const loadOlder = () => {
    void clientRef.current?.loadOlder();
  };

  const sending = !!state.pendingSend && !state.sendError;
  const channelName = direct
    ? (expectedChannelName ?? state.channelName)
    : (expectedChannelName ?? state.channelName).toLowerCase();
  const channelMessages = state.channelMessages ?? state.messages.filter(isChannelMessage);
  const messages =
    state.pendingSend && !state.pendingSend.threadRootId ? [...channelMessages, state.pendingSend] : channelMessages;
  const previewStart = Math.max(
    0,
    messages.length - Math.max(20, Math.ceil((typeof window === "undefined" ? 800 : window.innerHeight) / 50)),
  );
  useLayoutEffect(() => {
    const list = initialListRef.current;
    if (list) list.scrollTop = list.scrollHeight;
  }, [messages, listReady]);
  const typingNames = state.typingAuthors.filter((author) => !blockedIds.has(author.id)).map((author) => author.name);
  const ownId = state.author?.id;
  const channelRuns = useMemo(() => blockedRuns(messages, blockedIds, ownId), [messages, blockedIds, ownId]);
  const threadRootId = state.thread?.rootId;
  const threadRuns = useMemo(() => {
    if (!threadRootId || !blockedIds.size) return new Map<string, BlockedRun>();
    const root = state.messages.find((message) => message.id === threadRootId);
    return new Map([
      ...blockedRuns(root ? [root] : [], blockedIds, ownId),
      ...blockedRuns(
        state.messages.filter((message) => message.threadRootId === threadRootId),
        blockedIds,
        ownId,
      ),
    ]);
  }, [state.messages, threadRootId, blockedIds, ownId]);
  const toggleRun = (first: string, shown: boolean) =>
    setRevealedRuns((current) => {
      const next = new Set(current);
      if (shown) next.add(first);
      else next.delete(first);
      return next;
    });
  const typingLabel =
    typingNames.length > 2
      ? "Several people are typing…"
      : typingNames.length
        ? `${typingNames.join(" and ")} ${typingNames.length === 1 ? "is" : "are"} typing…`
        : "";
  const [displayedTypingLabel, setDisplayedTypingLabel] = useState("");
  useEffect(() => {
    if (typingLabel) {
      setDisplayedTypingLabel(typingLabel);
      return;
    }
    const timer = setTimeout(() => setDisplayedTypingLabel(""), 180);
    return () => clearTimeout(timer);
  }, [typingLabel]);

  // Share the formatter across visible rows, but refresh locale/timezone on render.
  const timeFormatter = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });
  const pinTimeFormatter = new Intl.DateTimeFormat(undefined, { dateStyle: "medium", timeStyle: "short" });
  const renderMessage = (index: number, message: (typeof messages)[number], inThread = false) => {
    const pending = !("content" in message);
    const author = message.author;
    const divider = hydrated && !inThread ? dateDivider(message.createdAt, messages[index - 1]?.createdAt) : undefined;
    const run = pending ? undefined : (inThread ? threadRuns : channelRuns).get(message.clientMessageId);
    const runShown = !!run && revealedRuns.has(run.first);
    if (run && !runShown) {
      // Every message stays one list item, so history paging is unchanged.
      if (run.first !== message.clientMessageId)
        return <div key={message.clientMessageId} className="chat-blocked-hidden" />;
      return (
        <div key={message.clientMessageId}>
          {divider && (
            <div className="chat-date-divider">
              <time dateTime={message.createdAt}>{divider}</time>
            </div>
          )}
          <div className="chat-blocked-run">
            <Ban size={16} aria-hidden="true" />
            <span>{blockedLabel(run.count)}</span>
            <span aria-hidden="true">—</span>
            <button
              type="button"
              onClick={() => toggleRun(run.first, true)}
              aria-label={`Show ${blockedLabel(run.count)}`}
            >
              Show
            </button>
          </div>
        </div>
      );
    }
    return (
      <div key={message.clientMessageId}>
        {divider && (
          <div className="chat-date-divider">
            <time dateTime={message.createdAt}>{divider}</time>
          </div>
        )}
        {run?.first === message.clientMessageId && (
          <div className="chat-blocked-run">
            <Ban size={16} aria-hidden="true" />
            <span>{blockedLabel(run.count)}</span>
            <span aria-hidden="true">—</span>
            <button
              type="button"
              onClick={() => toggleRun(run.first, false)}
              aria-label={`Hide ${blockedLabel(run.count)}`}
            >
              Hide
            </button>
          </div>
        )}
        <article
          className={`chat-message${pending ? " chat-message-pending" : ""}${"content" in message && message.pin ? " chat-message-pinned" : ""}${!pending && mentionsAccount(message, state.author?.id) ? " chat-message-mentioned" : ""}${!inThread && "id" in message && state.thread?.rootId === message.id ? " chat-message-thread-active" : ""}`}
          data-message-key={message.clientMessageId}
          onPointerDown={(event) => {
            cancelPress();
            if (
              !("content" in message) ||
              event.pointerType === "mouse" ||
              !event.isPrimary ||
              (event.target as HTMLElement).closest("button, a")
            )
              return;
            const anchor = event.currentTarget;
            press.current = {
              x: event.clientX,
              y: event.clientY,
              pointerId: event.pointerId,
              timer: setTimeout(() => {
                suppressClick.current = true;
                window.getSelection()?.removeAllRanges();
                openActions(message.id, anchor, inThread);
              }, 500),
            };
          }}
          onPointerMove={(event) => {
            const current = press.current;
            if (
              current &&
              (event.pointerId !== current.pointerId ||
                Math.hypot(event.clientX - current.x, event.clientY - current.y) > 10)
            )
              cancelPress();
          }}
          onPointerUp={cancelPress}
          onPointerCancel={cancelPress}
          onContextMenu={(event) => {
            if (!("content" in message) || !isTouchLayout() || (event.target as HTMLElement).closest("button, a"))
              return;
            event.preventDefault();
            cancelPress();
            suppressClick.current = true;
            openActions(message.id, event.currentTarget, inThread);
          }}
          tabIndex={pending ? undefined : -1}
        >
          {"content" in message && message.pin && (
            <div className="chat-pin-marker">
              <Pin size={12} aria-hidden="true" />
              Pinned by {message.pin.author.name}
            </div>
          )}
          <div className="chat-avatar">
            <Avatar avatarId={author?.avatarId} name={author?.name ?? name} />
          </div>
          <div>
            <header>
              <strong>{author?.name ?? name}</strong>
              {author?.isGuest && <span>Guest</span>}
              <time dateTime={message.createdAt}>{hydrated ? timeLabel(message.createdAt, timeFormatter) : ""}</time>
              {"content" in message && !message.forward && (message.revision ?? 1) > 1 && (
                <button
                  type="button"
                  className="chat-edited"
                  title={message.editedAt ? `Edited ${new Date(message.editedAt).toLocaleString()}` : undefined}
                  aria-label={`Message history, version ${message.revision}`}
                  onClick={() => openHistory(message.id)}
                >
                  edited
                </button>
              )}
            </header>
            {!inThread && message.threadRootId && (
              <button
                type="button"
                className="chat-thread-context"
                onMouseEnter={() => prefetchThread(message.threadRootId!)}
                onFocus={() => prefetchThread(message.threadRootId!)}
                onClick={() => openThread(message.threadRootId!)}
              >
                Replied to a thread · View thread
              </button>
            )}
            {(!("content" in message) || message.content.text) && (
              <p>
                {"content" in message
                  ? mentionSegments(message.content.text, message.content.mentions).map((segment, part) => {
                      if (!segment.mention) return segment.text;
                      const user = segment.user;
                      if (!user)
                        return (
                          <span key={part} className="chat-mention">
                            {segment.text}
                          </span>
                        );
                      const known = mentionDirectory.find((candidate) => candidate.id === user.id);
                      return (
                        <button
                          type="button"
                          key={part}
                          className="chat-mention chat-mention-person"
                          aria-haspopup="dialog"
                          aria-label={`Open profile for ${known?.displayName ?? `@${user.username}`}`}
                          onClick={(event) => {
                            event.stopPropagation();
                            setActionTarget(undefined);
                            setMentionCard({
                              person: mentionCardPerson(user, mentionDirectory, state.author?.id),
                              anchor: event.currentTarget,
                              drawer: isTouchLayout(),
                            });
                          }}
                        >
                          {segment.text}
                        </button>
                      );
                    })
                  : message.text}
              </p>
            )}
            {"content" in message && (
              <>
                <ForwardCard
                  message={message}
                  onOpen={(anchor) => setConversationTarget({ messageId: message.id, anchor })}
                />
                <button
                  type="button"
                  className="chat-message-actions-trigger"
                  aria-label={`Message actions for ${message.author.name}`}
                  aria-haspopup="dialog"
                  aria-expanded={actionTarget?.messageId === message.id && actionTarget.mode === "actions"}
                  onClick={(event) => openActions(message.id, event.currentTarget, inThread)}
                >
                  <MoreHorizontal size={14} aria-hidden="true" />
                </button>
                {!inThread && (
                  <button
                    type="button"
                    className="chat-reply-thread"
                    aria-label={`Reply in thread to ${message.author.name}`}
                    title="Reply in thread"
                    onMouseEnter={() => prefetchThread(message.threadRootId ?? message.id)}
                    onFocus={() => prefetchThread(message.threadRootId ?? message.id)}
                    onClick={() => openThread(message.threadRootId ?? message.id)}
                  >
                    <MessageSquare size={14} aria-hidden="true" />
                  </button>
                )}
                <MessageReactions
                  message={message}
                  channelId={state.channelId}
                  authorId={viewerId}
                  readOnly={readOnly || !state.author}
                  save={reactionSaves[message.id]}
                  onReact={react}
                  onShowReactors={(emoji, anchor) => showReactors(message.id, emoji, anchor)}
                  pickerOpen={actionTarget?.messageId === message.id && actionTarget.mode === "emoji"}
                  onOpenPicker={(anchor) =>
                    setActionTarget({
                      messageId: message.id,
                      anchor,
                      anchorRect: anchor.getBoundingClientRect(),
                      mode: "emoji",
                      drawer: isTouchLayout(),
                      inThread,
                    })
                  }
                  onDismissError={() => setReactionSaves((current) => ({ ...current, [message.id]: undefined }))}
                />
                {!inThread && !message.threadRootId && !!message.thread?.replyCount && (
                  <button
                    type="button"
                    className="chat-thread-summary"
                    onMouseEnter={() => prefetchThread(message.id)}
                    onFocus={() => prefetchThread(message.id)}
                    onClick={() => openThread(message.id)}
                    aria-label={`View thread with ${message.thread.replyCount} ${message.thread.replyCount === 1 ? "reply" : "replies"}`}
                  >
                    <span className="chat-thread-avatars">
                      {message.thread.participants.map((person) => (
                        <span key={person.id} title={person.name}>
                          <Avatar avatarId={person.avatarId} name={person.name} />
                        </span>
                      ))}
                    </span>
                    <strong>
                      {message.thread.replyCount} {message.thread.replyCount === 1 ? "reply" : "replies"}
                    </strong>
                    <span>View thread</span>
                  </button>
                )}
              </>
            )}
            {pending && state.sendError && (
              <div className="chat-send-status chat-send-error" role="alert">
                <span>
                  {state.sendRejected ? "Not sent." : "Not confirmed yet."} {state.sendError}
                </span>
                {state.sendRejected ? (
                  <>
                    <button
                      type="button"
                      disabled={hasDraft}
                      title={hasDraft ? "Clear your current draft to edit this message." : undefined}
                      onClick={() => {
                        const text = clientRef.current?.discardRejected();
                        if (text !== undefined) composerRef.current?.restore(text);
                      }}
                    >
                      Edit
                    </button>
                    <button type="button" onClick={() => clientRef.current?.discardRejected()}>
                      Dismiss
                    </button>
                  </>
                ) : (
                  <button type="button" onClick={() => composerRef.current?.submit()}>
                    Retry send
                  </button>
                )}
              </div>
            )}
          </div>
        </article>
      </div>
    );
  };

  const pinsToggle =
    state.phase === "ready" &&
    (showPins ? (
      <button type="button" className="chat-pins-toggle" onClick={() => setShowPins(false)}>
        <ArrowLeft size={16} aria-hidden="true" />
        Messages
      </button>
    ) : (
      <button
        type="button"
        className="chat-pins-toggle"
        onClick={() => setShowPins(true)}
        aria-label={`Pins, ${state.pinnedMessages.length}`}
      >
        <Pin size={16} aria-hidden="true" />
        Pins{state.pinnedMessages.length ? ` ${state.pinnedMessages.length}` : ""}
      </button>
    ));

  return (
    <div className="chat-layout" data-thread-open={!!state.thread}>
      <section className="chat-panel" aria-labelledby="chat-heading">
        <header className="chat-heading">
          {headerLeading}
          {channelMenu ? (
            <details
              ref={channelMenuRef}
              className="chat-channel-menu"
              onKeyDown={(event) => {
                if (event.key === "Escape") {
                  event.currentTarget.open = false;
                  event.currentTarget.querySelector("summary")?.focus();
                }
              }}
              onBlur={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
              }}
            >
              <summary aria-label={`${direct ? "" : "# "}${channelName} channel menu`}>
                <h2 id="chat-heading" className="chat-channel-title">
                  {direct ? "" : "# "}
                  {channelName}
                </h2>
                <ChevronDown aria-hidden="true" />
              </summary>
              <div
                className="space-actions"
                onClick={(event) => {
                  if ((event.target as HTMLElement).closest("button")) channelMenuRef.current!.open = false;
                }}
              >
                {pinsToggle}
                {channelMenu}
              </div>
            </details>
          ) : (
            <h2 id="chat-heading" className={showTitle ? "chat-channel-title" : "sr-only"}>
              {direct ? "" : "# "}
              {channelName}
            </h2>
          )}
          {!channelMenu && pinsToggle}
          {headerActions}
          {!state.online && showConnectionStatus && (
            <span className="chat-offline" role="status">
              {state.phase === "error" ? "Offline" : "Connecting…"}
            </span>
          )}
          {state.phase === "ready" && state.error && (
            <div className="chat-refresh-error" role="alert">
              {state.error}{" "}
              <button type="button" onClick={() => clientRef.current?.retryLoad()}>
                Retry
              </button>
            </div>
          )}
          {pinError && (
            <div className="chat-refresh-error" role="alert">
              {pinError.text}{" "}
              <button
                type="button"
                disabled={pinning.has(pinError.messageId)}
                onClick={() => void pin(pinError.messageId, pinError.active)}
              >
                Retry
              </button>{" "}
              <button type="button" onClick={() => setPinError(undefined)}>
                Dismiss
              </button>
            </div>
          )}
        </header>

        <div className="chat-messages" aria-busy={state.phase === "loading"}>
          {showPins && state.phase === "ready" && (
            <div className="chat-pins" role="region" aria-label={`Pinned messages in ${channelName}`}>
              <h3>Pinned messages</h3>
              {!state.pinnedMessages.length ? (
                <p className="chat-state">No pinned messages.</p>
              ) : (
                state.pinnedMessages.map((message) => (
                  <article className="chat-pinned-message" key={message.id}>
                    {message.pin && (
                      <small className="chat-pin-marker">
                        <Pin size={12} aria-hidden="true" />
                        Pinned by {message.pin.author.name}
                      </small>
                    )}
                    <div className="chat-avatar chat-pinned-avatar">
                      <Avatar avatarId={message.author.avatarId} name={message.author.name} />
                    </div>
                    <header>
                      <strong>{message.author.name}</strong>
                      <time dateTime={message.createdAt}>
                        {hydrated ? timeLabel(message.createdAt, pinTimeFormatter) : ""}
                      </time>
                    </header>
                    <p>{message.content.text}</p>
                    <ForwardCard
                      message={message}
                      onOpen={(anchor) => setConversationTarget({ messageId: message.id, anchor })}
                    />
                    {!message.forward &&
                      ((message.revision ?? 1) > 1 || (!readOnly && state.author?.id === message.author.id)) && (
                        <div className="chat-pinned-edit-actions">
                          {(message.revision ?? 1) > 1 && (
                            <button type="button" className="chat-edited" onClick={() => openHistory(message.id)}>
                              edited · Message history
                            </button>
                          )}
                          {!readOnly && state.author?.id === message.author.id && (
                            <button type="button" onClick={() => openEdit(message.id)}>
                              Edit message
                            </button>
                          )}
                        </div>
                      )}
                    {!readOnly && !!state.author && (
                      <button
                        type="button"
                        disabled={pinning.has(message.id)}
                        onClick={() => void pin(message.id, false)}
                      >
                        {pinning.has(message.id) ? "Unpinning…" : "Unpin"}
                      </button>
                    )}
                  </article>
                ))
              )}
            </div>
          )}
          <div className="chat-timeline" inert={showPins} aria-hidden={showPins}>
            {state.phase === "loading" && (
              <p className="chat-state" role="status">
                Loading messages…
              </p>
            )}
            {state.phase === "error" && (
              <div className="chat-state" role="alert">
                <p>{state.error}</p>
                <button type="button" onClick={() => clientRef.current?.retryLoad()}>
                  Try again
                </button>
              </div>
            )}
            {state.phase === "ready" && !messages.length && (
              <div className="chat-state">
                <p>No messages yet.</p>
                <small>
                  {direct
                    ? `Only you and ${channelName} can read this conversation.`
                    : `Start the conversation in #${channelName}.`}
                </small>
              </div>
            )}
            {state.phase === "ready" && messages.length > 0 && hydrated && (
              <Virtuoso
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
                atBottomStateChange={(atBottom) => {
                  followLatest.current = atBottom;
                }}
                startReached={() => {
                  if (!state.olderError) loadOlder();
                }}
                components={listComponents}
                context={{
                  hasMore: state.hasMore,
                  loadingOlder: state.loadingOlder,
                  olderError: state.olderError,
                  loadOlder,
                  onListReady: listReady ? undefined : () => setListReady(true),
                }}
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
                itemContent={(index, message) => renderMessage(index - firstItemIndex, message)}
              />
            )}
            {state.phase === "ready" && messages.length > 0 && !listReady && (
              <div
                ref={initialListRef}
                className="chat-initial-messages"
                role="region"
                aria-label={`Messages in ${channelName}`}
              >
                <HistoryHeader context={{ hasMore: state.hasMore, loadingOlder: false, loadOlder }} />
                {messages.slice(previewStart).map((message, index) => renderMessage(previewStart + index, message))}
              </div>
            )}
            <p className="sr-only" aria-live="polite" aria-atomic="true">
              {state.phase === "ready" && announcement}
            </p>
            <p className="sr-only" role="status">
              {actionStatus}
            </p>
          </div>
        </div>

        {mentionCard && (
          <MentionCard
            key={mentionCard.person.id}
            target={mentionCard}
            onClose={() => setMentionCard(undefined)}
            onMessage={onMessagePerson}
          />
        )}

        {actionTarget && actionMessage && (
          <MessageActions
            key={actionMessage.id}
            message={actionMessage}
            target={actionTarget}
            authorId={viewerId}
            canReact={!readOnly && !!state.author}
            canPin={!readOnly && !!state.author}
            pinning={pinning.has(actionMessage.id)}
            onReact={react}
            onPin={pin}
            canForward={signedIn && !!state.author && !state.author.isGuest && actionMessage.forward?.message !== null}
            onForward={() => {
              setForwardTarget({ messageId: actionMessage.id, anchor: actionTarget.anchor });
              setActionTarget(undefined);
            }}
            canEdit={
              !actionMessage.forward &&
              !readOnly &&
              !state.author?.isGuest &&
              state.author?.id === actionMessage.author.id
            }
            onClose={() => setActionTarget(undefined)}
            onCopied={setActionStatus}
            onReply={
              actionTarget.inThread ? undefined : () => openThread(actionMessage.threadRootId ?? actionMessage.id)
            }
            onEdit={() => openEdit(actionMessage.id)}
            onHistory={() => openHistory(actionMessage.id)}
            onViewReactions={(emoji) => showReactors(actionMessage.id, emoji, actionTarget.anchor)}
            block={
              onBlockAuthor && !actionMessage.author.isGuest && actionMessage.author.id !== state.author?.id
                ? {
                    blocked: blockedIds.has(actionMessage.author.id),
                    name: actionMessage.author.name,
                    onBlock: () => onBlockAuthor(actionMessage.author),
                    onUnblock: () => unblock(actionMessage.author.id),
                  }
                : undefined
            }
          />
        )}
        {reactorsTarget && reactorsMessage && state.channelId && (
          <ReactorsPanel
            key={reactorsMessage.id}
            channelId={state.channelId}
            message={reactorsMessage}
            target={reactorsTarget}
            onClose={closeReactors}
          />
        )}
        {forwardTarget && forwardMessage && (
          <ForwardPicker
            key={forwardMessage.id}
            message={forwardMessage}
            target={forwardTarget}
            onClose={closeForward}
            onForward={(destination, messageId, key, text) => {
              if (!clientRef.current) return Promise.reject(new Error("Chat session is unavailable."));
              return clientRef.current.forward(destination, messageId, key, text);
            }}
            onSent={(destination) => {
              setActionStatus(`Forwarded to ${destination.direct ? "" : "#"}${destination.name}.`);
              setForwardTarget(undefined);
            }}
          />
        )}
        {conversationTarget && conversationMessage && (
          <ForwardConversation
            key={conversationMessage.id}
            message={conversationMessage}
            target={conversationTarget}
            onClose={closeConversation}
          />
        )}

        <p className="chat-typing" role="status" aria-atomic="true">
          <span className="chat-typing-content" data-visible={!!typingLabel} aria-hidden={!typingLabel}>
            {displayedTypingLabel && (
              <>
                <span className="chat-typing-dots" aria-hidden="true">
                  <i />
                  <i />
                  <i />
                </span>
                <span>{displayedTypingLabel}</span>
              </>
            )}
          </span>
        </p>

        {readOnly ? (
          <div className="chat-composer channel-preview">{composerNotice}</div>
        ) : (
          <div className="chat-composer">
            {composerBanner}
            {state.pendingSend?.threadRootId &&
              state.sendError &&
              state.thread?.rootId !== state.pendingSend.threadRootId && (
                <p className="chat-inline-error" role="alert">
                  {state.sendRejected ? "A thread reply wasn’t sent." : "A thread reply couldn’t be confirmed."}{" "}
                  <button type="button" onClick={() => openThread(state.pendingSend!.threadRootId!)}>
                    Review reply
                  </button>
                </p>
              )}
            {state.sessionError && (
              <p className="chat-inline-error" role="alert">
                {state.sessionError}{" "}
                <button type="button" onClick={() => clientRef.current?.retrySession()}>
                  Retry session
                </button>
              </p>
            )}
            <Composer
              ref={composerRef}
              channelName={channelName}
              direct={direct}
              disabled={state.phase !== "ready"}
              identityReady={identityReady}
              sending={sending}
              sendRejected={!!state.sendRejected}
              pendingSend={state.pendingSend}
              authorId={state.author?.id}
              mentionMembers={mentionMembers}
              onSend={sendDraft}
              onTyping={setTyping}
              onWillSend={followOwnSend}
              onResize={keepLatestInView}
              onDraftPresence={setHasDraft}
            />
          </div>
        )}
      </section>
      <ThreadPanel
        state={state}
        client={clientRef.current}
        channelName={channelName}
        readOnly={readOnly}
        renderMessage={renderMessage}
        onClose={closeThread}
      />
      {state.phase === "ready" && !readOnly && editMessage && clientRef.current && (
        <MessageEditor
          key={editMessage.id}
          message={editMessage}
          onSave={(text, revision) => clientRef.current!.editMessage(editMessage.id, text, revision)}
          onReload={() => clientRef.current!.reloadMessage(editMessage.id)}
          onClose={() => setEditTarget(undefined)}
        />
      )}
      {state.phase === "ready" && historyMessage && (
        <MessageHistory
          key={`${historyMessage.id}:${historyMessage.revision ?? 1}`}
          message={historyMessage}
          onClose={() => setHistoryTarget(undefined)}
        />
      )}
    </div>
  );
}
