import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { ArrowRight, Ban, ChevronDown, MoreHorizontal, Pin, Reply, X } from "lucide-react";
import {
  FloatingFocusManager,
  FloatingOverlay,
  FloatingPortal,
  useDismiss,
  useFloating,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import { Virtuoso, type VirtuosoHandle, type ListProps, type ContextProp } from "react-virtuoso";
import { ChatClient, initialChatView } from "./client.ts";
import MessageReactions, { type ReactionSave } from "./MessageReactions.tsx";
import MessageActions, { preloadReactionPicker, type MessageActionTarget } from "./MessageActions.tsx";
import ReactorsPanel, { type ReactorsTarget } from "./ReactorsPanel.tsx";
import ThreadPanel from "./ThreadPanel.tsx";
import MessageEditor from "./MessageEditor.tsx";
import MessageHistory from "./MessageHistory.tsx";
import MessageSkeleton from "./MessageSkeleton.tsx";
import { MessageSquare } from "lucide-react";
import { ForwardCard, ForwardConversation, ForwardPicker, type ForwardTarget } from "./Forwarding.tsx";
import { Arrivals } from "./arrivals.ts";
import { dateDivider } from "./dates.ts";
import { groupsWithPrevious } from "./grouping.ts";
import { replyReference, ReplyContext, ReplyDraft } from "./Replies.tsx";
import LinkedText from "./LinkedText.tsx";
import {
  attachmentsOf,
  isChannelMessage,
  type ChatAttachment,
  type ChatAuthor,
  type ChatMessage,
  type ChatReplyReference,
  type GeneralChatHistory,
} from "./types.ts";
import { MessageAttachments, type DraftAttachment, type LocalPreview } from "./Attachments.tsx";
import MediaViewer, { type ViewerTarget } from "./MediaViewer.tsx";
import DropOverlay from "./DropOverlay.tsx";
import { carriesFiles } from "./drop.ts";
import { MAX_ATTACHMENTS, AttachmentUpload, refreshAttachmentUrls, uploadSettings } from "./uploads.ts";
import type { CompressionSettings } from "./prepare.ts";
import { appGateway, type PresenceStatus } from "../gateway/client.ts";
import Avatar from "../components/Avatar";
import DailyCaper from "../components/DailyCaper";
import Composer, { type ComposerHandle } from "./Composer.tsx";
import { useTimelineKeyboardDismiss } from "./useTimelineKeyboardDismiss.ts";
import { showStagedFiles, stagedFileChanged, stagedUrls, stashStagedFiles, takeStagedFiles } from "./drafts.ts";
import {
  feedbackAliases,
  mentionCardPerson,
  mentionSegments,
  mentionsAccount,
  reportsFeedback,
  type MentionCandidate,
} from "./mentions.ts";
import MentionCard, { type MentionCardTarget } from "./MentionCard.tsx";
import AdminBadge from "./AdminBadge.tsx";
import { blockedLabel, blockedRuns, type BlockedRun } from "./blocked.ts";
import { unblock, useBlockedIds } from "../spaces/blocks.ts";
import type { Space } from "../spaces/client.ts";
import "./chat.css";

// Virtuoso's prepend index is local bookkeeping, never the bigint server cursor.
const INITIAL_ITEM_INDEX = 1_000_000_000;

function timeLabel(value: string, formatter: Intl.DateTimeFormat) {
  const date = new Date(value);
  return Number.isNaN(date.valueOf()) ? "" : formatter.format(date);
}

/** Where to open a conversation: a loaded message or just its identity (a notification, a feedback report). */
export type FocusTarget = Pick<GeneralChatHistory["messages"][number], "id" | "channelId" | "threadRootId">;

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
// Computed border-box sizes retain subpixels without the homepage window's
// transform scale. Rounded offsets accumulate error between the preview and list.
const measureItem = (element: HTMLElement, field: "offsetHeight" | "offsetWidth") =>
  parseFloat(getComputedStyle(element)[field === "offsetHeight" ? "height" : "width"]);
/** A message row's top below the scroller's top edge, in layout pixels (the homepage window is scaled). */
function messageOffset(scroller: HTMLElement, key: string, row?: Element | null) {
  row ??= scroller.querySelector(`.chat-message[data-message-key="${CSS.escape(key)}"]`);
  if (!row) return undefined;
  const bounds = scroller.getBoundingClientRect();
  return (row.getBoundingClientRect().top - bounds.top) / (bounds.height / scroller.offsetHeight || 1);
}
/** The first message row the reader can see, with its offset. */
function topMessage(scroller: HTMLElement | null) {
  if (!scroller) return undefined;
  const top = scroller.getBoundingClientRect().top;
  const row = [...scroller.querySelectorAll<HTMLElement>(".chat-message[data-message-key]")].find(
    (item) => item.getBoundingClientRect().bottom > top,
  );
  const key = row?.dataset.messageKey;
  const offset = key ? messageOffset(scroller, key, row) : undefined;
  return key && offset !== undefined ? { key, offset } : undefined;
}

function PinsDialog({
  children,
  onClose,
  actionsOpen,
}: {
  children: ReactNode;
  onClose: () => void;
  actionsOpen: boolean;
}) {
  const { refs, context } = useFloating({
    open: true,
    onOpenChange: (open) => {
      if (!open) onClose();
    },
  });
  const { getFloatingProps } = useInteractions([useDismiss(context, { enabled: !actionsOpen }), useRole(context)]);
  return (
    <FloatingPortal>
      <FloatingOverlay lockScroll className="chat-forward-overlay chat-pins-overlay">
        <FloatingFocusManager context={context} disabled={actionsOpen}>
          <section
            ref={refs.setFloating}
            className="chat-forward-dialog chat-pins-dialog"
            aria-label="Pinned messages"
            {...getFloatingProps()}
          >
            <header className="chat-reaction-picker-heading">
              <strong>Pins</strong>
              <button type="button" aria-label="Close pins" onClick={onClose}>
                <X size={18} />
              </button>
            </header>
            {children}
          </section>
        </FloatingFocusManager>
      </FloatingOverlay>
    </FloatingPortal>
  );
}

export default function Chat({
  name,
  signedIn,
  accountId,
  identityReady,
  channelId,
  channelName: expectedChannelName,
  direct = false,
  directPeerId,
  team = false,
  onReadCursor,
  readActive = true,
  initialHistory,
  initialHistoryError,
  showTitle = false,
  headerLeading,
  headerVoice,
  headerActions,
  headerMenu,
  headerSubtitle,
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
  profileSpaces,
  onOpenForward,
  focusMessage,
  focusThread = false,
  visitor = false,
  onSent,
}: {
  name: string;
  signedIn: boolean;
  /** Known viewer identity, independent of the chat sending capability. */
  accountId?: string;
  identityReady: boolean;
  channelId?: string;
  channelName?: string;
  direct?: boolean;
  directPeerId?: string;
  /** A DM with the Caper account, which the feedback team answers. */
  team?: boolean;
  onReadCursor?: (seq: string) => void;
  readActive?: boolean;
  initialHistory?: GeneralChatHistory;
  initialHistoryError?: string;
  showTitle?: boolean;
  headerLeading?: ReactNode;
  /** The open channel's voice, right after its name. */
  headerVoice?: ReactNode;
  headerActions?: ReactNode;
  /** Phones fold Pins and these items into the menu their channel name opens. */
  headerMenu?: ReactNode;
  /** A line under the name on phones, such as member and voice counts. */
  headerSubtitle?: ReactNode;
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
  profileSpaces?: (personId: string) => Space[];
  onOpenForward?: (message: GeneralChatHistory["messages"][number]) => Promise<boolean>;
  focusMessage?: FocusTarget;
  /** Also open a focused channel message's own thread, where replies to it belong. */
  focusThread?: boolean;
  /** The Caper team, taking part without having joined: the header also names the space. */
  visitor?: boolean;
  /** After one of the viewer's messages (or thread replies) is accepted. */
  onSent?: () => void;
}) {
  const [state, setState] = useState(() => initialChatView(initialHistory, initialHistoryError));
  useEffect(() => {
    // Start code/data and opening-grid artwork together, without waiting for a
    // sending session or a pointer hover (which touch clients never provide).
    if (signedIn && !readOnly) preloadReactionPicker();
  }, [signedIn, readOnly]);
  const viewerId = accountId ?? state.author?.id;
  const [showConnectionStatus, setShowConnectionStatus] = useState(false);
  const [firstItemIndex, setFirstItemIndex] = useState(INITIAL_ITEM_INDEX);
  // The composer owns the draft; the conversation only needs to know one exists.
  const composerRef = useRef<ComposerHandle>(null);
  const keyboardDismiss = useTimelineKeyboardDismiss("chat-message");
  const [hasDraft, setHasDraft] = useState(false);
  const [replyTargets, setReplyTargets] = useState<Record<string, ChatReplyReference | undefined>>({});
  const setReplyTarget = (root: string, reference?: ChatReplyReference) =>
    setReplyTargets((current) => ({ ...current, [root]: reference }));
  useLayoutEffect(() => {
    if (state.pendingSend) setReplyTarget(state.pendingSend.threadRootId ?? "channel", undefined);
  }, [state.pendingSend?.clientMessageId]);
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const followUpdatedRows = useRef(false);
  const [actionTarget, setActionTarget] = useState<MessageActionTarget>();
  const [reactorsTarget, setReactorsTarget] = useState<ReactorsTarget>();
  const [forwardTarget, setForwardTarget] = useState<ForwardTarget>();
  const [conversationTarget, setConversationTarget] = useState<ForwardTarget>();
  const openingForward = useRef(false);
  const [editTarget, setEditTarget] = useState<string>();
  const [historyTarget, setHistoryTarget] = useState<string>();
  const [viewerTarget, setViewerTarget] = useState<ViewerTarget>();
  // Confirmations such as "Text copied." show briefly; `key` restarts the timer when repeated.
  const [actionStatus, setActionStatusState] = useState<{
    text: string;
    key: number;
  }>();
  const setActionStatus = (text: string) =>
    setActionStatusState(text ? (current) => ({ text, key: (current?.key ?? 0) + 1 }) : undefined);
  useEffect(() => {
    if (!actionStatus) return;
    const timer = setTimeout(() => setActionStatusState(undefined), 3_000);
    return () => clearTimeout(timer);
  }, [actionStatus?.key]);
  // Keeps the text while the visible confirmation fades out.
  const lastActionStatus = useRef("");
  if (actionStatus) lastActionStatus.current = actionStatus.text;
  const [reactionSaves, setReactionSaves] = useState<Record<string, ReactionSave | undefined>>({});
  const [mentionCard, setMentionCard] = useState<MentionCardTarget>();
  const [showPins, setShowPins] = useState(false);
  const [jumping, setJumping] = useState<string>();
  const [jumpError, setJumpError] = useState<string>();
  const [jumpMessage, setJumpMessage] = useState<string>();
  const [scrollTarget, setScrollTarget] = useState<string>();
  const [listWindow, setListWindow] = useState(0);
  const closePins = useCallback(() => {
    setMentionCard(undefined);
    setShowPins(false);
  }, []);
  const [pinning, setPinning] = useState<Set<string>>(() => new Set());
  const [pinError, setPinError] = useState<{
    messageId: string;
    active: boolean;
    text: string;
  }>();
  // Runs of blocked messages the reader chose to show, by their first message.
  const [revealedRuns, setRevealedRuns] = useState<ReadonlySet<string>>(() => new Set());
  const blockedIds = useBlockedIds();
  const press = useRef<{
    timer: ReturnType<typeof setTimeout>;
    x: number;
    y: number;
    pointerId: number;
    row?: HTMLElement;
  }>(undefined);
  const suppressClick = useRef(false);
  const cancelPress = () => {
    clearTimeout(press.current?.timer);
    press.current?.row?.removeAttribute("data-pressing");
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
    setViewerTarget(undefined);
    setActionStatus("");
    setReactionSaves({});
    setShowPins(false);
    setJumping(undefined);
    setJumpError(undefined);
    setJumpMessage(undefined);
    setScrollTarget(undefined);
    setPinning(new Set());
    setPinError(undefined);
    setRevealedRuns(new Set());
    // The drawer can appear under the held finger. Its release click must not
    // activate a newly rendered action, even though that action is in a portal.
    const resetClick = () => {
      cancelPress();
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
    setReplyTargets({});
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
    setActionTarget({
      messageId,
      anchor,
      anchorRect: anchor.getBoundingClientRect(),
      mode: "actions",
      drawer: isTouchLayout(),
      inThread,
    });
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
  const reactorsMessage = findMessage(reactorsTarget?.messageId);
  const forwardMessage = findMessage(forwardTarget?.messageId);
  const conversationMessage = conversationTarget
    ? [...state.messages, ...state.pinnedMessages].find((message) => message.id === conversationTarget.messageId)
    : undefined;
  const closeForward = useCallback(() => setForwardTarget(undefined), []);
  const closeConversation = useCallback(() => setConversationTarget(undefined), []);
  const openConversation = async (message: GeneralChatHistory["messages"][number], anchor: HTMLElement) => {
    if (openingForward.current) return;
    openingForward.current = true;
    const client = clientRef.current;
    try {
      const navigated = await onOpenForward?.(message);
      if (!navigated && clientRef.current === client) setConversationTarget({ messageId: message.id, anchor });
    } catch {
      if (clientRef.current === client) setActionStatus("Couldn’t open the conversation. Try again.");
    } finally {
      openingForward.current = false;
    }
  };
  const editMessage = findMessage(editTarget);
  const historyMessage = findMessage(historyTarget);
  // Forwarded originals live on the forwarding message; the viewer closes if either goes.
  const viewerMessage = findMessage(viewerTarget?.messageId);
  const viewerSource = viewerTarget?.forwarded ? (viewerMessage?.forward?.message ?? undefined) : viewerMessage;
  const closeViewer = useCallback(() => setViewerTarget(undefined), []);
  const viewFile =
    (messageId: string, forwarded = false) =>
    (attachmentId: string, anchor: HTMLElement) =>
      setViewerTarget({ messageId, attachmentId, anchor, forwarded });
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
  const scrollerRef = useRef<HTMLElement>(null);
  const arrivals = useRef<Arrivals>(null);
  arrivals.current ??= new Arrivals();
  // Virtuoso shifts the list by the prepended rows' estimated heights, but the
  // old first row also changes as an older page lands above it: it can join
  // that page's last group (losing its name and time) and loses its date
  // divider. Remember the top visible message so it can be held in place.
  const prependAnchor = useRef<{ key: string; offset: number } | undefined>(undefined);
  const goToMessage = async (message: FocusTarget) => {
    const client = clientRef.current;
    if (!client || jumping) return false;
    setJumping(message.id);
    setJumpError(undefined);
    setActionTarget(undefined);
    setMentionCard(undefined);
    try {
      if ((await client.loadMessageContext(message)) && clientRef.current === client) {
        setShowPins(false);
        setJumpMessage(message.id);
        if (!message.threadRootId) {
          // Cancel the previous window's queued follow/measurement corrections.
          setListWindow((window) => window + 1);
          setScrollTarget(message.id);
        }
        return true;
      } else if (clientRef.current === client) setJumpError("Message could not be loaded. Try again.");
    } catch (error) {
      if (clientRef.current === client)
        setJumpError(error instanceof Error ? error.message : "Message could not be loaded.");
    } finally {
      if (clientRef.current === client) setJumping(undefined);
    }
    return false;
  };
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
  const [drafts, setDrafts] = useState<DraftAttachment[]>([]);
  const [attachErrors, setAttachErrors] = useState<Record<string, string | undefined>>({});
  const channelDrafts = drafts.filter((draft) => !draft.threadRootId);
  const threadDrafts = drafts.filter((draft) => draft.threadRootId === state.thread?.rootId && !!draft.threadRootId);
  const setAttachError = (error: string | undefined, root?: string) =>
    setAttachErrors((current) => ({ ...current, [root ?? "channel"]: error }));
  const uploads = useRef(new Map<string, AttachmentUpload>());
  const abandonUpload = useCallback((key: string) => {
    uploads.current.get(key)?.release(true);
    uploads.current.delete(key);
  }, []);
  const objectUrls = useRef(new Set<string>());
  // undefined is checking; null means unavailable. Neither removes the attach button.
  const [compression, setCompression] = useState<CompressionSettings | null>();
  const uploadsEnabled = !!compression;
  const [freshUrls, setFreshUrls] = useState<Record<string, { url?: string; previewUrl?: string }>>({});
  // The sender's local copies of sent files, shown while the server processes them.
  const [localPreviews, setLocalPreviews] = useState<Record<string, LocalPreview>>({});
  // The thread composer suggests the same people as the channel composer.
  const mentionPeople = useMemo(
    () => mentionMembers?.filter((member) => member.id !== state.author?.id),
    [mentionMembers, state.author?.id],
  );
  const allowFollow = useRef(false);
  allowFollow.current = !state.hasNewer && !scrollTarget && !jumping;
  const latestMessage = state.messages.at(-1);
  const [announcement, setAnnouncement] = useState("");
  useEffect(() => {
    if (state.phase === "ready" && latestMessage) {
      const files = attachmentsOf(latestMessage).length;
      setAnnouncement(
        `${latestMessage.author.name}: ${latestMessage.content.text || `sent ${files === 1 ? "a file" : `${files} files`}`}`,
      );
    }
  }, [latestMessage?.id, state.phase]);
  // Reaction events advance the conversation stream without adding a message.
  // HTTP reaction snapshots do not advance this committed replay cursor.
  const readCursor = clientRef.current?.readCursor() ?? initialHistory?.cursor ?? latestMessage?.seq ?? "0";
  const readCallback = useRef(onReadCursor);
  readCallback.current = onReadCursor;
  useEffect(() => {
    const read = () => {
      if (readActive && state.phase === "ready" && !state.hasNewer && document.visibilityState === "visible")
        readCallback.current?.(readCursor);
    };
    read();
    document.addEventListener("visibilitychange", read);
    // Retry a failed acknowledgement even if no new message arrives.
    const timer = setInterval(read, 15_000);
    return () => {
      clearInterval(timer);
      document.removeEventListener("visibilitychange", read);
    };
  }, [state.phase, state.hasNewer, readCursor, channelId, readActive]);

  // The composer resizes itself; keep the conversation pinned to the newest
  // message when it already was. Read the position before resizing, not a
  // delayed bottom callback, and correct only real height changes using the
  // new DOM extent rather than the virtualizer's previous measurements.
  const keepLatestInView = useCallback((resize: () => boolean) => {
    const scroller = scrollerRef.current;
    const atBottom = scroller && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 80;
    if (resize() && atBottom && allowFollow.current) scroller.scrollTop = scroller.scrollHeight;
  }, []);
  const setTyping = useCallback((active: boolean) => {
    clientRef.current?.setTyping(active);
  }, []);

  useEffect(() => {
    let firstMessageId: string | undefined;
    let previousRows = state.channelMessages;
    const client = new ChatClient(
      (next) => {
        if (next.channelMessages !== previousRows) {
          const scroller = scrollerRef.current;
          // Read BEFORE React grows the row. Virtuoso's 80px tolerance can
          // otherwise consider a newly clipped reaction bar still at bottom.
          followUpdatedRows.current =
            !!scroller &&
            allowFollow.current &&
            !next.hasNewer &&
            previousRows?.length === next.channelMessages?.length &&
            previousRows?.[0]?.id === next.channelMessages?.[0]?.id &&
            scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 80;
          previousRows = next.channelMessages;
        }
        if (next.phase !== "ready") {
          firstMessageId = undefined;
          setListReady(false);
          setFirstItemIndex(INITIAL_ITEM_INDEX);
        } else if (next.channelMessages?.[0]?.id !== firstMessageId) {
          const prepended = next.channelMessages?.findIndex((message) => message.id === firstMessageId) ?? -1;
          if (prepended > 0) {
            prependAnchor.current = topMessage(scrollerRef.current);
            setFirstItemIndex((index) => index - prepended);
          }
          firstMessageId = next.channelMessages?.[0]?.id;
        }
        // The composer clears the draft a new local send came from.
        // Transfer ownership synchronously, before focus can renew an uncertain
        // send or a fast HTTP/gateway confirmation can hide the pending render.
        const sent = new Set(next.pendingSend?.attachments?.map((file) => file.id));
        for (const [key, upload] of uploads.current) {
          if (upload.state.attachment && sent.has(upload.state.attachment.id)) uploads.current.delete(key);
        }
        if (sent.size)
          setDrafts((current) => current.filter((item) => !item.attachment || !sent.has(item.attachment.id)));
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

  const handledFocus = useRef<{ client: ChatClient; message: FocusTarget } | undefined>(undefined);
  useEffect(() => {
    if (!focusMessage) {
      handledFocus.current = undefined;
      return;
    }
    const client = clientRef.current;
    if (
      !client ||
      state.phase !== "ready" ||
      jumping ||
      focusMessage.channelId !== state.channelId ||
      (handledFocus.current?.message === focusMessage && handledFocus.current.client === client)
    )
      return;
    handledFocus.current = { client, message: focusMessage };
    void goToMessage(focusMessage).then((found) => {
      if (found && focusThread && !focusMessage.threadRootId && clientRef.current === client)
        openThread(focusMessage.id);
    });
  }, [focusMessage, state.phase, state.channelId, jumping]);

  const sentCallback = useRef(onSent);
  sentCallback.current = onSent;
  const lastPending = useRef(state.pendingSend);
  useEffect(() => {
    if (lastPending.current && !state.pendingSend) sentCallback.current?.();
    lastPending.current = state.pendingSend;
  }, [state.pendingSend]);

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
    if (!signedIn) {
      setCompression(undefined);
      return;
    }
    let active = true;
    void uploadSettings().then((settings) => {
      if (active) setCompression(settings ?? null);
    });
    return () => {
      active = false;
    };
  }, [signedIn]);

  const updateDraft = useCallback((key: string, change: Partial<DraftAttachment>) => {
    setDrafts((current) => current.map((item) => (item.key === key ? { ...item, ...change } : item)));
  }, []);
  useEffect(() => {
    showStagedFiles(updateDraft);
    return () => showStagedFiles(undefined);
  }, [updateDraft]);

  // Staged files belong to their conversation, like its text: switching away
  // keeps them (uploads carry on) and coming back restores them. Only Remove
  // or Send gives them up. Without an account key there is nowhere to keep them.
  const draftKey = accountId && channelId ? `${accountId}:${channelId}` : undefined;
  const latestDrafts = useRef(drafts);
  latestDrafts.current = drafts;
  useEffect(() => {
    const saved = draftKey ? takeStagedFiles(draftKey) : undefined;
    if (saved) {
      uploads.current = saved.uploads;
      // Also the ref: a cleanup before the next render must stash these again.
      latestDrafts.current = saved.drafts;
      setDrafts(saved.drafts);
      // Renew anything that expired or failed while the conversation was closed.
      for (const upload of saved.uploads.values())
        if (upload.stale || upload.state.error) void upload.ensure().catch(() => {});
    }
    return () => {
      if (draftKey) stashStagedFiles(draftKey, { drafts: latestDrafts.current, uploads: uploads.current });
      else for (const key of uploads.current.keys()) abandonUpload(key);
      uploads.current = new Map();
      setDrafts([]);
    };
  }, [draftKey, abandonUpload]);
  useEffect(
    () => () => {
      const kept = stagedUrls();
      for (const url of objectUrls.current) if (!kept.has(url)) URL.revokeObjectURL(url);
    },
    [],
  );

  const retryUpload = (key: string) => {
    void uploads.current
      .get(key)
      ?.ensure()
      .catch(() => {});
  };
  useEffect(() => {
    const refresh = () => {
      if (document.visibilityState !== "visible") return;
      for (const upload of uploads.current.values())
        if (upload.stale || upload.state.error) void upload.ensure().catch(() => {});
      if (state.sendUploadFailed && state.pendingSend)
        void clientRef.current
          ?.send(state.pendingSend.text, {
            threadRootId: state.pendingSend.threadRootId,
            broadcast: state.pendingSend.broadcast,
          })
          // The pending message keeps its own retry state; this is only a background nudge.
          .catch(() => {});
    };
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [state.sendUploadFailed, state.pendingSend]);

  const addFiles = (files: File[], threadRootId?: string) => {
    const channel = state.channelId;
    const settings = compression;
    if (!signedIn || !settings || !channel || !files.length) return;
    const room = MAX_ATTACHMENTS - drafts.filter((draft) => draft.threadRootId === threadRootId).length;
    if (room <= 0) {
      setAttachError(`You can attach up to ${MAX_ATTACHMENTS} files.`, threadRootId);
      return;
    }
    setAttachError(
      files.length > room ? `Only ${room} more file${room === 1 ? "" : "s"} can be attached.` : undefined,
      threadRootId,
    );
    for (const file of files.slice(0, room)) {
      const key = crypto.randomUUID();
      const localKind =
        file.type.startsWith("image/") && file.type !== "image/svg+xml"
          ? ("image" as const)
          : file.type.startsWith("video/")
            ? ("video" as const)
            : undefined;
      // Keep active document formats from executing on the app's blob origin.
      const previewable =
        localKind || file.type.startsWith("audio/") || ["application/pdf", "text/plain"].includes(file.type);
      const localUrl = URL.createObjectURL(previewable ? file : new Blob([file], { type: "application/octet-stream" }));
      objectUrls.current.add(localUrl);
      const upload = new AttachmentUpload(channel, file, settings, (next) => {
        stagedFileChanged(key, next);
        if (next.attachment && localKind)
          setLocalPreviews((current) => ({
            ...current,
            [next.attachment!.id]: { url: localUrl, video: localKind === "video" },
          }));
      });
      uploads.current.set(key, upload);
      setDrafts((current) => [
        ...current,
        {
          key,
          threadRootId,
          name: file.name,
          localUrl,
          localKind,
          sourceSize: file.size,
          progress: 0,
        },
      ]);
      void upload.ensure().catch(() => {});
    }
  };

  const removeDraft = (key: string) => {
    abandonUpload(key);
    setDrafts((current) => current.filter((item) => item.key !== key));
  };

  const refreshUrls = useCallback((ids: string[]) => {
    void refreshAttachmentUrls(ids).then((urls) => {
      if (Object.keys(urls).length) setFreshUrls((current) => ({ ...current, ...urls }));
    });
  }, []);
  // Only ready files have URLs worth refreshing; processing ones get theirs from events.
  const withFreshUrls = (attachments: ChatAttachment[]) =>
    attachments.map((attachment) =>
      attachment.status === undefined || attachment.status === "ready"
        ? { ...attachment, ...freshUrls[attachment.id] }
        : attachment,
    );

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

  // An updated row that keeps the list height never reaches totalListHeightChanged.
  // Drop the follow intent after layout settles, so a later scrollbar drag
  // is not snapped back by an unrelated row resize.
  useLayoutEffect(() => {
    if (!followUpdatedRows.current) return;
    let frames = 3;
    let frame = requestAnimationFrame(function settle() {
      if (--frames) frame = requestAnimationFrame(settle);
      else followUpdatedRows.current = false;
    });
    return () => cancelAnimationFrame(frame);
  }, [state.channelMessages]);

  const loadOlder = () => {
    void clientRef.current?.loadOlder();
  };
  // After a prepend, keep the remembered message where the reader left it while
  // Virtuoso measures the new rows. Stops once it holds still, or as soon as the
  // reader scrolls themselves.
  useLayoutEffect(() => {
    const scroller = scrollerRef.current;
    const anchor = prependAnchor.current;
    prependAnchor.current = undefined;
    if (!scroller || !anchor) return;
    let frame = 0;
    let ticks = 0;
    let still = 0;
    let stopped = false;
    // Rendered rows' offsets from the anchor, from the last frame that showed it.
    let around = new Map<string, number>();
    // Virtuoso applies its measured-size corrections in animation frame
    // callbacks that run after one of ours would, so the reader saw each
    // half-corrected frame (the list jumped by up to a few hundred pixels).
    // Correct from a ResizeObserver callback instead: those run after every
    // animation frame callback and layout, just before paint. A hidden
    // sentinel resized each frame makes sure the callback runs every frame,
    // including frames where Virtuoso only changed scrollTop.
    const sentinel = document.createElement("div");
    sentinel.setAttribute("aria-hidden", "true");
    sentinel.style.cssText = "position:fixed;top:0;left:0;width:0;height:0;visibility:hidden;pointer-events:none";
    const anchorOffset = () => {
      const rows = new Map<string, number>();
      for (const row of scroller.querySelectorAll<HTMLElement>(".chat-message[data-message-key]")) {
        const offset = messageOffset(scroller, row.dataset.messageKey!, row);
        if (offset !== undefined) rows.set(row.dataset.messageKey!, offset);
      }
      const offset = rows.get(anchor.key);
      if (offset !== undefined) {
        around = new Map([...rows].map(([key, value]) => [key, value - offset]));
        return offset;
      }
      // Virtuoso can briefly render a range without the anchor while it
      // re-measures; place it from the nearest row seen beside it.
      let nearest: [string, number] | undefined;
      for (const [key, value] of around)
        if (rows.has(key) && (!nearest || Math.abs(value) < Math.abs(nearest[1]))) nearest = [key, value];
      return nearest && rows.get(nearest[0])! - nearest[1];
    };
    const hold = () => {
      if (stopped) return;
      const offset = anchorOffset();
      if (offset !== undefined && Math.abs(offset - anchor.offset) >= 1) {
        scroller.scrollTop += offset - anchor.offset;
        still = 0;
      } else if (++still >= 6) stop();
    };
    const observer = new ResizeObserver(hold);
    const tick = () => {
      if (++ticks > 60) return stop();
      sentinel.style.width = `${(ticks % 2) + 1}px`;
      frame = requestAnimationFrame(tick);
    };
    const inputs = ["wheel", "touchstart", "pointerdown", "keydown"] as const;
    const stop = () => {
      stopped = true;
      cancelAnimationFrame(frame);
      observer.disconnect();
      sentinel.remove();
      for (const type of inputs) scroller.removeEventListener(type, stop);
    };
    for (const type of inputs) scroller.addEventListener(type, stop, { passive: true });
    document.body.append(sentinel);
    observer.observe(sentinel);
    frame = requestAnimationFrame(tick);
    // Virtuoso re-renders the old first row (now without its date divider or
    // header) in a sync update right after this commit, before any paint.
    queueMicrotask(hold);
    return stop;
  }, [firstItemIndex]);

  const sending = !!state.pendingSend && !state.sendError;
  const channelName = direct
    ? (expectedChannelName ?? state.channelName)
    : (expectedChannelName ?? state.channelName).toLowerCase();
  const channelMessages = state.channelMessages ?? state.messages.filter(isChannelMessage);
  const messages =
    state.pendingSend && !state.pendingSend.threadRootId ? [...channelMessages, state.pendingSend] : channelMessages;
  arrivals.current.observe(
    "channel",
    state.channelId ?? "",
    messages.map((message) => message.clientMessageId),
    state.phase === "ready" && !state.hasNewer && !state.loadingNewer,
  );
  const scrollIndex =
    scrollTarget && scrollTarget !== "latest"
      ? messages.findIndex((message) => "id" in message && message.id === scrollTarget)
      : "LAST";
  useEffect(() => {
    if (!scrollTarget || !listReady) return;
    if (scrollTarget === "latest" && state.hasNewer) return;
    if (scrollIndex === -1) return;
    const frame = requestAnimationFrame(() => {
      listRef.current?.scrollToIndex({
        index: scrollIndex,
        align: scrollTarget === "latest" ? "end" : "center",
      });
      setScrollTarget(undefined);
    });
    return () => cancelAnimationFrame(frame);
  }, [scrollTarget, scrollIndex, messages, firstItemIndex, listReady, state.hasNewer]);
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
  useLayoutEffect(() => {
    const target = state.messages.find((message) => message.id === jumpMessage);
    const run = target && (target.threadRootId ? threadRuns : channelRuns).get(target.clientMessageId);
    if (run) setRevealedRuns((current) => (current.has(run.first) ? current : new Set([...current, run.first])));
  }, [jumpMessage, state.messages, threadRuns, channelRuns]);
  const toggleRun = (first: string, shown: boolean) => {
    if (!shown) setJumpMessage(undefined);
    setRevealedRuns((current) => {
      const next = new Set(current);
      if (shown) next.add(first);
      else next.delete(first);
      return next;
    });
  };
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
  useLayoutEffect(() => {
    // The typing row resizes the viewport; retain live follow without moving history readers.
    const scroller = scrollerRef.current;
    if (scroller && allowFollow.current && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 80)
      scroller.scrollTop = scroller.scrollHeight;
  }, [displayedTypingLabel]);
  const sendDraft = async (text: string, threadRootId?: string, broadcast = false) => {
    const client = clientRef.current;
    const files = drafts.filter((draft) => draft.threadRootId === threadRootId);
    const sources = !state.pendingSend ? files.map((draft) => uploads.current.get(draft.key)!) : [];
    setAttachError(undefined, threadRootId);
    let attachments: ChatAttachment[];
    try {
      attachments = await Promise.all(sources.map((upload) => upload.ensure()));
    } catch {
      return;
    } // The file chip owns the upload error and its Retry action.
    if (client !== clientRef.current || sources.some((upload) => ![...uploads.current.values()].includes(upload)))
      return;
    // Sent files show the sender's local copies until processing finishes.
    const local = Object.fromEntries(
      files.flatMap((item) =>
        item.attachment && item.localUrl && item.localKind
          ? [[item.attachment.id, { url: item.localUrl, video: item.localKind === "video" }] as const]
          : [],
      ),
    );
    if (Object.keys(local).length) setLocalPreviews((current) => ({ ...current, ...local }));
    const sent = await client?.send(text, {
      attachments,
      uploads: sources,
      threadRootId,
      broadcast,
      replyTo: replyTargets[threadRootId ?? "channel"],
    });
    if (sent && !threadRootId && state.hasNewer) {
      setJumpMessage(undefined);
      setScrollTarget("latest");
    }
    return sent;
  };

  // Share the formatter across visible rows, but refresh locale/timezone on render.
  const timeFormatter = new Intl.DateTimeFormat(undefined, {
    hour: "numeric",
    minute: "2-digit",
  });
  const pinTimeFormatter = new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  });
  // Same person, within five minutes, same day, and no blocked-run bar in between.
  const joinsPrevious = (
    previous: (typeof messages)[number] | undefined,
    message: (typeof messages)[number] | undefined,
    inThread: boolean,
  ) => {
    if (!hydrated || !previous || !message) return false;
    const runs = inThread ? threadRuns : channelRuns;
    const run = "content" in message ? runs.get(message.clientMessageId) : undefined;
    if (run && (run.first === message.clientMessageId || !revealedRuns.has(run.first))) return false;
    const previousRun = "content" in previous ? runs.get(previous.clientMessageId) : undefined;
    if (previousRun && !revealedRuns.has(previousRun.first)) return false;
    return groupsWithPrevious(previous, message, { inThread });
  };
  // "8:28" for the narrow gutter beside compact rows (no AM/PM).
  const compactTime = (value: string) =>
    timeFormatter
      .formatToParts(new Date(value))
      .filter((part) => part.type !== "dayPeriod")
      .map((part) => part.value)
      .join("")
      .trim();
  const replyToMessage = (message: ChatMessage, inThread: boolean) => {
    const root = inThread
      ? state.thread?.rootId
      : message.threadRootId && !message.broadcast
        ? message.threadRootId
        : "channel";
    if (!root) return;
    setReplyTarget(root, replyReference(message));
    setActionTarget(undefined);
    setShowPins(false);
    if (root !== "channel" && !inThread) openThread(root);
    requestAnimationFrame(() => {
      if (root !== "channel") document.getElementById("chat-thread-reply")?.focus();
      else composerRef.current?.focus();
    });
  };
  const renderMessage = (
    index: number,
    message: (typeof messages)[number],
    inThread = false,
    previousInThread?: (typeof messages)[number],
    nextInThread?: (typeof messages)[number],
  ) => {
    const pending = !("content" in message);
    const author = message.author;
    const divider = hydrated && !inThread ? dateDivider(message.createdAt, messages[index - 1]?.createdAt) : undefined;
    const run = pending ? undefined : (inThread ? threadRuns : channelRuns).get(message.clientMessageId);
    const runShown = !!run && revealedRuns.has(run.first);
    const grouped = joinsPrevious(inThread ? previousInThread : messages[index - 1], message, inThread);
    // The row above a follow-up gives up its bottom padding so the run reads as one block.
    const continued = joinsPrevious(message, inThread ? nextInThread : messages[index + 1], inThread);
    const edited =
      "content" in message && !message.forward && (message.revision ?? 1) > 1 ? (
        <button
          type="button"
          className="chat-edited"
          title={message.editedAt ? `Edited ${new Date(message.editedAt).toLocaleString()}` : undefined}
          aria-label={`Message history, version ${message.revision}`}
          onClick={() => openHistory(message.id)}
        >
          edited
        </button>
      ) : undefined;
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
              aria-expanded={false}
            >
              Show
            </button>
          </div>
        </div>
      );
    }
    return (
      <div key={message.clientMessageId}>
        {divider && (!run || run.first === message.clientMessageId) && (
          <div className="chat-date-divider">
            <time dateTime={message.createdAt}>{divider}</time>
          </div>
        )}
        <div className={run ? "chat-blocked-revealed" : undefined}>
          {divider && run && run.first !== message.clientMessageId && (
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
                aria-expanded={true}
              >
                Hide
              </button>
            </div>
          )}
          <article
            className={`chat-message${grouped ? " chat-message-grouped" : ""}${continued ? " chat-message-continued" : ""}${pending ? " chat-message-pending" : ""}${arrivals.current?.has(message.clientMessageId) ? " chat-message-arrive" : ""}${"id" in message && jumpMessage === message.id ? " chat-message-jump-target" : ""}${!pending && mentionsAccount(message, state.author?.id) ? " chat-message-mentioned" : ""}${!inThread && "id" in message && state.thread?.rootId === message.id ? " chat-message-thread-active" : ""}`}
            data-message-key={message.clientMessageId}
            data-message-id={"id" in message ? message.id : undefined}
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
              // Keep transient feedback on this exact row, without rerendering
              // the virtualized timeline (or highlighting its channel/thread copy).
              anchor.dataset.pressing = "true";
              press.current = {
                row: anchor,
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
            {message.replyTo && (
              <ReplyContext
                motionKey={message.clientMessageId}
                reference={message.replyTo}
                hidden={blockedIds.has(message.replyTo.author.id) && message.replyTo.author.id !== viewerId}
                onOpen={() => void goToMessage({ ...message.replyTo!, channelId: state.channelId! })}
              />
            )}
            {grouped ? (
              // The run's avatar is above; show the time here on hover instead.
              <div className="chat-grouped-time" aria-hidden="true">
                {compactTime(message.createdAt)}
              </div>
            ) : (
              authorAvatar(author)
            )}
            <div>
              <header className={grouped ? "sr-only" : undefined}>
                {grouped ? <strong>{author?.name ?? name}</strong> : authorName(author)}
                {author?.admin && <AdminBadge />}
                {author?.isGuest && <span>Guest</span>}
                <time dateTime={message.createdAt}>{hydrated ? timeLabel(message.createdAt, timeFormatter) : ""}</time>
                {!grouped && edited}
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
                        if (!segment.mention) return <LinkedText key={part} text={segment.text} />;
                        const user = segment.user;
                        if (!user)
                          return (
                            <span
                              key={part}
                              className="chat-mention"
                              title={
                                feedbackAliases.some((alias) => segment.text.slice(1).toLowerCase() === alias)
                                  ? "Asks the Caper team to look at this conversation"
                                  : undefined
                              }
                            >
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
                  {/* Compact rows have no header, so the marker follows the text. */}
                  {grouped && edited && <> {edited}</>}
                </p>
              )}
              <MessageAttachments
                attachments={"content" in message ? withFreshUrls(attachmentsOf(message)) : (message.attachments ?? [])}
                progress={state.attachmentProgress}
                localPreviews={localPreviews}
                onExpired={pending ? undefined : refreshUrls}
                onView={"content" in message ? viewFile(message.id) : undefined}
              />
              {"content" in message && reportsFeedback(message) && (
                <p className="chat-feedback-note">Caper team has been notified 🫡</p>
              )}
              {"content" in message && (
                <>
                  <ForwardCard
                    message={message}
                    onOpen={(anchor) => void openConversation(message, anchor)}
                    onView={viewFile(message.id, true)}
                  />
                  <button
                    type="button"
                    className="chat-message-actions-trigger"
                    aria-label={`Message actions for ${message.author.name}`}
                    title="More actions"
                    aria-haspopup="dialog"
                    aria-expanded={actionTarget?.messageId === message.id && actionTarget.mode === "actions"}
                    onClick={(event) =>
                      // A second click on the open menu's own trigger closes it, like a toggle.
                      actionTarget?.messageId === message.id && actionTarget.mode === "actions"
                        ? setActionTarget(undefined)
                        : openActions(message.id, event.currentTarget, inThread)
                    }
                  >
                    <MoreHorizontal size={14} aria-hidden="true" />
                  </button>
                  {!readOnly && state.author && (
                    <button
                      type="button"
                      className="chat-reply-message"
                      aria-label={`Reply to ${message.author.name}`}
                      title="Reply"
                      disabled={!!state.pendingSend}
                      onClick={() => replyToMessage(message, inThread)}
                    >
                      <Reply size={14} aria-hidden="true" />
                    </button>
                  )}
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
                      actionTarget?.messageId === message.id && actionTarget.mode === "emoji"
                        ? setActionTarget(undefined)
                        : setActionTarget({
                            messageId: message.id,
                            anchor,
                            anchorRect: anchor.getBoundingClientRect(),
                            mode: "emoji",
                            drawer: isTouchLayout(),
                            inThread,
                          })
                    }
                    onDismissError={() =>
                      setReactionSaves((current) => ({
                        ...current,
                        [message.id]: undefined,
                      }))
                    }
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
                    {!state.sendUploadFailed && (state.sendRejected ? "Not sent. " : "Not confirmed yet. ")}
                    {state.sendError}
                  </span>
                  {state.sendRejected ? (
                    <>
                      <button
                        type="button"
                        disabled={hasDraft}
                        title={hasDraft ? "Clear your current draft to edit this message." : undefined}
                        onClick={() => {
                          setReplyTarget("channel", state.pendingSend?.replyTo);
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
                      {state.sendUploadFailed ? "Retry" : "Retry send"}
                    </button>
                  )}
                </div>
              )}
            </div>
          </article>
        </div>
      </div>
    );
  };

  const canAttach = signedIn && uploadsEnabled && !readOnly && state.phase === "ready" && !!state.channelId;
  // Child elements fire their own enter/leave pairs; the overlay stays up until the last leave.
  const dragDepth = useRef(0);
  const [dropping, setDropping] = useState(false);
  const endDrop = () => {
    dragDepth.current = 0;
    setDropping(false);
  };
  useEffect(() => {
    // A file dropped beside the conversation would otherwise replace the app with it.
    const refuse = (event: DragEvent) => {
      if (event.defaultPrevented || !carriesFiles(event.dataTransfer)) return;
      event.preventDefault();
      if (event.type === "dragover" && event.dataTransfer) event.dataTransfer.dropEffect = "none";
      if (event.type === "drop") endDrop();
    };
    window.addEventListener("dragover", refuse);
    window.addEventListener("drop", refuse);
    return () => {
      window.removeEventListener("dragover", refuse);
      window.removeEventListener("drop", refuse);
    };
  }, []);
  const pinsToggle = state.phase === "ready" && (
    <button
      type="button"
      className="chat-pins-toggle"
      onClick={() => setShowPins(true)}
      aria-label="Pins"
      title="Pins"
      aria-haspopup="dialog"
    >
      <Pin size={18} aria-hidden="true" />
    </button>
  );
  const openAuthor = (author: ChatAuthor, anchor: HTMLElement, focusOnOpen = true) => {
    setActionTarget(undefined);
    setMentionCard({
      person: mentionCardPerson(author, mentionDirectory, viewerId),
      anchor,
      drawer: isTouchLayout(),
      focusOnOpen,
    });
  };
  const authorAvatar = (author: ChatAuthor | undefined) => {
    const image = <Avatar avatarId={author?.avatarId} name={author?.name ?? name} />;
    return author && !author.isGuest ? (
      <button
        type="button"
        className="chat-avatar chat-author-profile"
        aria-label={`Open profile for ${author.name}`}
        aria-haspopup="dialog"
        onClick={(event) => openAuthor(author, event.currentTarget)}
      >
        {image}
      </button>
    ) : (
      <div className="chat-avatar">{image}</div>
    );
  };
  const authorName = (author: ChatAuthor | undefined) =>
    author && !author.isGuest ? (
      <button
        type="button"
        className="chat-author-name chat-author-profile"
        aria-label={`Open profile for ${author.name}`}
        aria-haspopup="dialog"
        onClick={(event) => openAuthor(author, event.currentTarget)}
      >
        <strong>{author.name}</strong>
      </button>
    ) : (
      <strong>{author?.name ?? name}</strong>
    );

  const pinsOpen = showPins && state.phase === "ready";
  // Opened from pins, the viewer renders inside that dialog, which then treats it as its own layer.
  const viewer = viewerTarget && viewerSource && (
    <MediaViewer
      // Forwarded originals have no URL refresh, like their inline previews.
      attachments={viewerTarget.forwarded ? attachmentsOf(viewerSource) : withFreshUrls(attachmentsOf(viewerSource))}
      attachmentId={viewerTarget.attachmentId}
      anchor={viewerTarget.anchor}
      author={viewerSource.author.name}
      sentAt={viewerSource.createdAt}
      onClose={closeViewer}
      onExpired={viewerTarget.forwarded ? undefined : refreshUrls}
    />
  );

  return (
    <div className="chat-layout" data-thread-open={!!state.thread}>
      <section
        className="chat-panel"
        aria-labelledby="chat-heading"
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
          addFiles([...event.dataTransfer.files]);
        }}
      >
        {dropping && canAttach && (
          <DropOverlay channelName={channelName} direct={direct} attached={channelDrafts.length} />
        )}
        <header className="chat-heading">
          {headerLeading}
          <h2 id="chat-heading" className={showTitle ? "chat-channel-title" : "sr-only"}>
            {direct ? "" : "# "}
            {channelName}
            {visitor && !direct && <span className="chat-channel-space"> · {state.spaceName}</span>}
          </h2>
          {/* Phones: the name opens the channel's menu, as in Slack; Pins and Members move there. */}
          {showTitle && state.phase === "ready" && (
            <details
              className="chat-title-menu"
              onClick={(event) => {
                // Choosing an item closes the menu.
                if ((event.target as HTMLElement).closest(".chat-header-menu button")) event.currentTarget.open = false;
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape" && event.currentTarget.open) {
                  event.preventDefault();
                  event.currentTarget.open = false;
                  event.currentTarget.querySelector("summary")?.focus();
                }
              }}
              onBlur={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
              }}
            >
              <summary aria-label={`${direct ? "" : "#"}${channelName} options`}>
                <span className="chat-title-name">
                  <span className="chat-title-text">
                    {direct ? "" : "# "}
                    {channelName}
                  </span>
                  <ChevronDown size={16} aria-hidden="true" />
                </span>
                {headerSubtitle && <small className="chat-title-subtitle">{headerSubtitle}</small>}
              </summary>
              <div className="chat-header-menu">
                <button type="button" aria-haspopup="dialog" onClick={() => setShowPins(true)}>
                  <Pin size={16} aria-hidden="true" />
                  Pins
                </button>
                {headerMenu}
              </div>
            </details>
          )}
          {headerVoice}
          {pinsToggle}
          {headerActions}

          {!state.online && showConnectionStatus && (
            <span className="chat-offline" role="status">
              {state.phase === "error" ? "Offline" : "Connecting…"}
            </span>
          )}
          {((state.phase === "ready" && state.error) || pinError) && (
            <div className="chat-heading-alerts">
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
            </div>
          )}
        </header>

        <div className="chat-messages" aria-busy={state.phase === "loading"}>
          {pinsOpen && (
            <PinsDialog
              onClose={closePins}
              actionsOpen={
                !!(
                  actionTarget ||
                  reactorsTarget ||
                  forwardTarget ||
                  conversationTarget ||
                  editTarget ||
                  historyTarget ||
                  mentionCard
                )
              }
            >
              <div className="chat-pins">
                {jumpError && (
                  <p className="chat-inline-error" role="alert">
                    {jumpError}
                  </p>
                )}
                {!state.pinnedMessages.length ? (
                  <p className="chat-state">No pinned messages.</p>
                ) : (
                  state.pinnedMessages.map((message) => (
                    <article className="chat-message chat-pinned-message" key={message.id}>
                      {message.replyTo && (
                        <ReplyContext
                          motionKey={message.clientMessageId}
                          reference={message.replyTo}
                          hidden={blockedIds.has(message.replyTo.author.id) && message.replyTo.author.id !== viewerId}
                          onOpen={() => void goToMessage({ ...message.replyTo!, channelId: state.channelId! })}
                        />
                      )}
                      {message.pin && (
                        <small className="chat-pin-marker">
                          <Pin size={12} aria-hidden="true" />
                          <button
                            type="button"
                            className="chat-pin-author"
                            aria-label={`Open profile for ${message.pin.author.name}`}
                            aria-haspopup="dialog"
                            onClick={(event) => openAuthor(message.pin!.author, event.currentTarget)}
                            onPointerEnter={(event) => {
                              if (event.pointerType === "mouse")
                                openAuthor(message.pin!.author, event.currentTarget, false);
                            }}
                            onPointerDown={(event) => {
                              cancelPress();
                              if (event.pointerType === "mouse" || !event.isPrimary) return;
                              const anchor = event.currentTarget;
                              press.current = {
                                x: event.clientX,
                                y: event.clientY,
                                pointerId: event.pointerId,
                                timer: setTimeout(() => {
                                  suppressClick.current = true;
                                  openAuthor(message.pin!.author, anchor);
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
                              if (isTouchLayout()) event.preventDefault();
                            }}
                          >
                            Pinned by {message.pin.author.name}
                          </button>
                        </small>
                      )}
                      {authorAvatar(message.author)}
                      <header>
                        {authorName(message.author)}
                        <time dateTime={message.createdAt}>
                          {hydrated ? timeLabel(message.createdAt, pinTimeFormatter) : ""}
                        </time>
                      </header>
                      {message.content.text && (
                        <p>
                          <LinkedText text={message.content.text} />
                        </p>
                      )}
                      <MessageAttachments
                        attachments={withFreshUrls(attachmentsOf(message))}
                        progress={state.attachmentProgress}
                        localPreviews={localPreviews}
                        onExpired={refreshUrls}
                        onView={viewFile(message.id)}
                      />
                      <ForwardCard
                        message={message}
                        onOpen={(anchor) => void openConversation(message, anchor)}
                        onView={viewFile(message.id, true)}
                      />
                      <div className="chat-pinned-navigation">
                        <button type="button" disabled={!!jumping} onClick={() => void goToMessage(message)}>
                          {jumping === message.id ? "Loading message…" : "Go to message"}
                          <ArrowRight size={14} aria-hidden="true" />
                        </button>
                      </div>
                      <button
                        type="button"
                        className="chat-message-actions-trigger"
                        aria-label={`Message actions for ${message.author.name}`}
                        aria-haspopup="dialog"
                        aria-expanded={actionTarget?.messageId === message.id}
                        onClick={(event) => openActions(message.id, event.currentTarget, false)}
                      >
                        <MoreHorizontal size={14} aria-hidden="true" />
                      </button>
                    </article>
                  ))
                )}
              </div>
              {viewer}
            </PinsDialog>
          )}
          <div className="chat-timeline" inert={showPins} aria-hidden={showPins} {...keyboardDismiss}>
            {state.phase === "loading" && <MessageSkeleton />}
            {state.phase === "error" && (
              <div className="chat-state" role="alert">
                <p>{state.error}</p>
                <button type="button" onClick={() => clientRef.current?.retryLoad()}>
                  Try again
                </button>
              </div>
            )}
            {state.phase === "ready" && !messages.length && (
              <div className="chat-state chat-empty">
                <DailyCaper className="chat-empty-caper" />
                {direct && viewerId && directPeerId === viewerId ? (
                  <p>You can message yourself here to keep notes, reminders, and ideas.</p>
                ) : (
                  <>
                    <p>No messages yet.</p>
                    <small>
                      {team
                        ? "Messages here go to the Caper team."
                        : direct
                          ? `Only you and ${channelName} can read this conversation.`
                          : `Start the conversation in #${channelName}.`}
                    </small>
                  </>
                )}
              </div>
            )}
            {state.phase === "ready" && messages.length > 0 && hydrated && (
              <Virtuoso
                key={listWindow}
                ref={listRef}
                scrollerRef={(element) => {
                  scrollerRef.current = element instanceof HTMLElement ? element : null;
                }}
                data={messages}
                firstItemIndex={firstItemIndex}
                alignToBottom
                initialTopMostItemIndex={{
                  index: scrollIndex === -1 ? "LAST" : scrollIndex,
                  align: scrollTarget && scrollTarget !== "latest" ? "center" : "end",
                }}
                computeItemKey={(_, message) => `${message.author?.id ?? "pending"}:${message.clientMessageId}`}
                defaultItemHeight={70}
                itemSize={measureItem}
                // Measure before paint so the preview handoff and live follow
                // do not use the previous frame's estimated row heights.
                skipAnimationFrameInResizeObserver
                increaseViewportBy={{ top: 250, bottom: 150 }}
                followOutput={allowFollow.current ? "auto" : false}
                atBottomThreshold={80}
                totalListHeightChanged={() => {
                  if (followUpdatedRows.current && allowFollow.current)
                    listRef.current?.scrollToIndex({ index: "LAST", align: "end" });
                  followUpdatedRows.current = false;
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
                onWheel={() => {
                  followUpdatedRows.current = false;
                }}
                onTouchMove={() => {
                  followUpdatedRows.current = false;
                }}
                // Covers scrollbar drags, which send no wheel or touch events.
                onPointerDown={() => {
                  followUpdatedRows.current = false;
                }}
                onKeyDown={(event) => {
                  followUpdatedRows.current = false;
                  if (event.target === event.currentTarget && event.key === "End") {
                    event.preventDefault();
                    listRef.current?.scrollToIndex({
                      index: "LAST",
                      align: "end",
                    });
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
                <HistoryHeader
                  context={{
                    hasMore: state.hasMore,
                    loadingOlder: false,
                    loadOlder,
                  }}
                />
                <div className="chat-initial-rows">
                  {messages.slice(previewStart).map((message, index) => renderMessage(previewStart + index, message))}
                </div>
              </div>
            )}
            <p className="sr-only" aria-live="polite" aria-atomic="true">
              {state.phase === "ready" && announcement}
            </p>
            <p className="sr-only" role="status">
              {actionStatus?.text}
            </p>
            <p className="chat-action-status" aria-hidden="true" data-visible={actionStatus ? "" : undefined}>
              {lastActionStatus.current}
            </p>
          </div>
        </div>
        {state.hasNewer && (
          <div className="chat-history" role="status">
            {state.newerError && <span>{state.newerError}</span>}
            <button type="button" disabled={state.loadingNewer} onClick={() => void clientRef.current?.loadNewer()}>
              {state.loadingNewer ? "Loading…" : state.newerError ? "Retry newer messages" : "Load newer messages"}
            </button>
            <button
              type="button"
              onClick={() => {
                setJumpMessage(undefined);
                setScrollTarget("latest");
                clientRef.current?.retryLoad();
              }}
            >
              Back to latest
            </button>
          </div>
        )}

        {mentionCard && (
          <MentionCard
            key={mentionCard.person.id}
            target={mentionCard}
            onClose={() => setMentionCard(undefined)}
            onMessage={onMessagePerson}
            spaces={profileSpaces?.(mentionCard.person.id)}
            onBlock={
              onBlockAuthor &&
              ((author) => {
                setMentionCard(undefined);
                setShowPins(false);
                onBlockAuthor(author);
              })
            }
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
              setForwardTarget({
                messageId: actionMessage.id,
                anchor: actionTarget.anchor,
              });
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
            onReplyMessage={
              !readOnly && state.author && !state.pendingSend
                ? () => replyToMessage(actionMessage, actionTarget.inThread)
                : undefined
            }
            // The menu item is removed as the dialog opens; let the dialog return focus to the menu's trigger.
            onEdit={() => {
              actionTarget.anchor.focus({ preventScroll: true });
              openEdit(actionMessage.id);
            }}
            onHistory={() => {
              actionTarget.anchor.focus({ preventScroll: true });
              openHistory(actionMessage.id);
            }}
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
            onSent={(count) => {
              setActionStatus(`Forwarded to ${count} ${count === 1 ? "destination" : "destinations"}.`);
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
        <p className="chat-typing" role="status" aria-atomic="true" data-visible={!!displayedTypingLabel}>
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
            {replyTargets.channel && (
              <ReplyDraft
                reference={replyTargets.channel}
                hidden={blockedIds.has(replyTargets.channel.author.id) && replyTargets.channel.author.id !== viewerId}
                onCancel={() => setReplyTarget("channel", undefined)}
              />
            )}
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
              draftKey={draftKey}
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
              onResize={keepLatestInView}
              onDraftPresence={setHasDraft}
              attachments={
                signedIn || channelDrafts.length
                  ? {
                      enabled: canAttach,
                      // Like every client: checking while settings or the conversation load, else unavailable.
                      disabledReason:
                        compression === null || state.phase === "error" || readOnly || !signedIn
                          ? "File uploads are unavailable"
                          : "Checking file uploads…",
                      drafts: channelDrafts,
                      error: attachErrors.channel,
                      onAdd: addFiles,
                      onRemove: removeDraft,
                      onRetry: retryUpload,
                    }
                  : undefined
              }
            />
          </div>
        )}
      </section>
      <ThreadPanel
        state={state}
        client={clientRef.current}
        channelName={channelName}
        direct={direct}
        readOnly={readOnly}
        mentionPeople={mentionPeople}
        specialMentions={!direct}
        replyTo={state.thread ? replyTargets[state.thread.rootId] : undefined}
        onSend={(text, broadcast) => sendDraft(text, state.thread?.rootId, broadcast)}
        localPreviews={localPreviews}
        attachments={
          signedIn || threadDrafts.length
            ? {
                enabled: canAttach && !state.thread?.loading,
                disabledReason:
                  compression === null || state.phase === "error" || readOnly || !signedIn
                    ? "File uploads are unavailable"
                    : "Checking file uploads…",
                drafts: threadDrafts,
                error: state.thread ? attachErrors[state.thread.rootId] : undefined,
                onAdd: (files) => addFiles(files, state.thread?.rootId),
                onRemove: removeDraft,
                onRetry: retryUpload,
              }
            : undefined
        }
        onReplyChange={(reference) => {
          if (state.thread) setReplyTarget(state.thread.rootId, reference);
        }}
        blockedIds={blockedIds}
        renderMessage={renderMessage}
        arrivals={arrivals.current}
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
      {!pinsOpen && viewer}
    </div>
  );
}
