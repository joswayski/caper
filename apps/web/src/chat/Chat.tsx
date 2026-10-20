import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
  type ReactNode,
} from "react";
import { ArrowRight, ChevronDown, MoreHorizontal, Pin, X } from "lucide-react";
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
import { subscribeProfiles } from "../account/profiles.ts";
import type { ReactionSave } from "./MessageReactions.tsx";
import MessageActions, { preloadReactionPicker, type MessageActionTarget } from "./MessageActions.tsx";
import ReactorsPanel, { type ReactorsTarget } from "./ReactorsPanel.tsx";
import ThreadPanel from "./ThreadPanel.tsx";
import MessageEditor from "./MessageEditor.tsx";
import DeleteMessage from "./DeleteMessage.tsx";
import MessageHistory from "./MessageHistory.tsx";
import MessageSkeleton from "./MessageSkeleton.tsx";
import { ForwardCard, ForwardConversation, ForwardPicker, type ForwardTarget } from "./Forwarding.tsx";
import { Arrivals } from "./arrivals.ts";
import { useTimelineMotion } from "./useTimelineMotion.ts";
import { dateDivider } from "./dates.ts";
import { channelNewMessagesId, groupsWithPrevious } from "./grouping.ts";
import { replyReference, ReplyContext, ReplyDraft } from "./Replies.tsx";
import MessageRow, {
  AuthorAvatar,
  AuthorName,
  isTouchLayout,
  MessageText,
  PinMarker,
  timeLabel,
  withFreshUrls,
  type FreshUrls,
  type RowActions,
} from "./MessageRow.tsx";
import {
  attachmentsOf,
  isChannelMessage,
  type ChatAttachment,
  type ChatAuthor,
  type ChatMessage,
  type ChatReplyReference,
  type GeneralChatHistory,
} from "./types.ts";
import { MessageAttachments, type DraftAttachment, type DraftEditing, type LocalPreview } from "./Attachments.tsx";
import MediaViewer, { type ViewerTarget } from "./MediaViewer.tsx";
import { MediaPreviewContext } from "./MediaPreviewContext.ts";
import DropOverlay from "./DropOverlay.tsx";
import { carriesFiles } from "./drop.ts";
import { MAX_ATTACHMENTS, AttachmentUpload, refreshAttachmentUrls, uploadSettings, UploadError } from "./uploads.ts";
import {
  getVoiceSendOnStop,
  isVoiceAttachment,
  subscribeVoiceSendOnStop,
  VOICE_LABEL,
  type VoiceRecording,
} from "./voice.ts";
import type { VoiceComposer } from "./VoiceRecorder.tsx";
import { editableKind, type CompressionSettings } from "./prepare.ts";
import { appGateway, type PresenceStatus } from "../gateway/client.ts";
import Avatar, { type AvatarPerson } from "../components/Avatar";
import DailyCaper from "../components/DailyCaper";
import Composer, { type ComposerHandle } from "./Composer.tsx";
import { useTimelineKeyboardDismiss } from "./useTimelineKeyboardDismiss.ts";
import { showStagedFiles, stagedFileChanged, stagedUrls, stashStagedFiles, takeStagedFiles } from "./drafts.ts";
import { mentionCardPerson, type MentionCandidate } from "./mentions.ts";
import MentionCard, { type MentionCardTarget } from "./MentionCard.tsx";
import { blockedAuthor, blockedRuns, type BlockedRun } from "./blocked.ts";
import { unblock, useBlockedIds } from "../spaces/blocks.ts";
import { listUnreadSpaces, type Space, type UnreadSpace } from "../spaces/client.ts";
import { klipyConfig, shareKlipy, type ChatGif } from "./klipy.ts";
import "./chat.css";

// Virtuoso's prepend index is local bookkeeping, never the bigint server cursor.
const INITIAL_ITEM_INDEX = 1_000_000_000;

const NO_CANDIDATES: MentionCandidate[] = [];

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
  // Bottom-aligned short histories keep the history header directly above the
  // first message (see .chat-scroller in chat.css), not at the top of the pane.
  const style = props.style?.marginTop === "auto" ? { ...props.style, marginTop: 0 } : props.style;
  return (
    <div {...props} style={style} className="chat-motion-clip">
      {children}
    </div>
  );
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

/**
 * A stable object whose methods call the latest committed render's handlers,
 * so memoized rows can keep one `actions` prop for the conversation's lifetime.
 */
function useLatestActions<T extends object>(handlers: T): T {
  const latest = useRef(handlers);
  useLayoutEffect(() => {
    latest.current = handlers;
  });
  const [stable] = useState(
    () =>
      Object.fromEntries(
        Object.keys(handlers).map((key) => [
          key,
          (...args: unknown[]) => (latest.current[key as keyof T] as (...args: unknown[]) => unknown)(...args),
        ]),
      ) as T,
  );
  return stable;
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
  directPeer,
  team = false,
  onReadCursor,
  readActive = true,
  initialHistory,
  initialHistoryError,
  showTitle = false,
  headerLeading,
  headerBelow,
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
  mentionDirectory = NO_CANDIDATES,
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
  directPeer?: AvatarPerson;
  /** A DM with the Caper account, which the feedback team answers. */
  team?: boolean;
  onReadCursor?: (seq: string) => void;
  readActive?: boolean;
  initialHistory?: GeneralChatHistory;
  initialHistoryError?: string;
  showTitle?: boolean;
  headerLeading?: ReactNode;
  /** Phones: a strip under the header carrying voice (your call, or this channel's live room). */
  headerBelow?: ReactNode;
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
    // Idle time, so ~130 image decodes don't compete with the first chat paint.
    if (!signedIn || readOnly) return;
    if (typeof requestIdleCallback !== "function") return preloadReactionPicker();
    const idle = requestIdleCallback(preloadReactionPicker, { timeout: 2_000 });
    return () => cancelIdleCallback(idle);
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
  const [deleteTarget, setDeleteTarget] = useState<string>();
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
    setDeleteTarget(undefined);
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
    setDeleteTarget(undefined);
    setReplyTargets({});
  }, [state.author?.id, state.channelId]);
  useEffect(() => {
    if (!readOnly) return;
    setEditTarget(undefined);
    setDeleteTarget(undefined);
  }, [readOnly]);
  useEffect(() => {
    if (state.phase === "error") {
      setEditTarget(undefined);
      setHistoryTarget(undefined);
      setDeleteTarget(undefined);
      // Pins close with the conversation rather than reopening after Try again.
      setShowPins(false);
    }
  }, [state.phase]);
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
    id === undefined
      ? undefined
      : (state.messages.find((message) => message.id === id) ??
        state.pinnedMessages.find((message) => message.id === id));
  const actionMessage = findMessage(actionTarget?.messageId);
  const reactorsMessage = findMessage(reactorsTarget?.messageId);
  const forwardMessage = findMessage(forwardTarget?.messageId);
  const conversationMessage = conversationTarget
    ? [...state.messages, ...state.pinnedMessages].find(
        (message) => message.id === conversationTarget.messageId && !message.deletedAt && message.forward?.message,
      )
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
  // A message deleted meanwhile, here or elsewhere, closes its editor and history.
  const editMessage = findMessage(editTarget);
  const historyMessage = findMessage(historyTarget);
  const deleteMessage = findMessage(deleteTarget);
  // Forwarded originals live on the forwarding message; the viewer closes if either goes.
  const viewerMessage = findMessage(viewerTarget?.messageId);
  const viewerSource = viewerMessage?.deletedAt
    ? undefined
    : viewerTarget?.forwarded
      ? (viewerMessage?.forward?.message ?? undefined)
      : viewerMessage;
  const closeViewer = useCallback(() => setViewerTarget(undefined), []);
  const viewFile =
    (messageId: string, forwarded = false) =>
    (attachmentId: string, anchor: HTMLElement, onHideSpoiler?: ViewerTarget["onHideSpoiler"]) =>
      setViewerTarget({ messageId, attachmentId, anchor, forwarded, onHideSpoiler });
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
  const [scroller, setScroller] = useState<HTMLElement | null>(null);
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
    // Pins shows its own error; elsewhere (a reply's original, a notification) a
    // failed jump must still say so instead of doing nothing.
    const fail = showPins ? setJumpError : setActionStatus;
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
      } else if (clientRef.current === client) fail("Message could not be loaded. Try again.");
    } catch (error) {
      if (clientRef.current === client) fail(error instanceof Error ? error.message : "Message could not be loaded.");
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
  const [freshUrls, setFreshUrls] = useState<FreshUrls>({});
  // Keep sender previews through processing and confirmation without reloading images.
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
    // A blocked person's message stays collapsed for screen readers too.
    if (
      state.phase === "ready" &&
      latestMessage &&
      !blockedAuthor(latestMessage.author, blockedIds, state.author?.id)
    ) {
      const attached = attachmentsOf(latestMessage);
      const files = attached.length;
      setAnnouncement(
        `${latestMessage.author.name}: ${latestMessage.content.text || (files && attached.every(isVoiceAttachment) ? "sent a voice message" : `sent ${files === 1 ? "a file" : `${files} files`}`)}`,
      );
    }
  }, [latestMessage?.id, state.phase]);
  // Reaction events advance the conversation stream without adding a message.
  // HTTP reaction snapshots do not advance this committed replay cursor.
  const readCursor = clientRef.current?.readCursor() ?? initialHistory?.cursor ?? latestMessage?.seq ?? "0";
  const readCallback = useRef(onReadCursor);
  readCallback.current = onReadCursor;
  const trackChannelVisit = !!accountId && !!channelId && !!onReadCursor && !direct && !visitor;
  const [unreadVisit, setUnreadVisit] = useState<{
    accountId: string;
    channelId: string;
    entry?: UnreadSpace["channels"][number];
  }>();
  const visitReady =
    !trackChannelVisit || (unreadVisit?.accountId === accountId && unreadVisit?.channelId === channelId);
  useEffect(() => {
    if (!trackChannelVisit || !accountId || !channelId) return;
    let cancelled = false;
    // Resolve once per visit, before acknowledgements erase the saved read position.
    void listUnreadSpaces()
      .then((snapshot) => {
        if (!cancelled)
          setUnreadVisit({
            accountId,
            channelId,
            entry: snapshot.spaces.flatMap((space) => space.channels).find((entry) => entry.id === channelId),
          });
      })
      .catch(() => {
        if (!cancelled) setUnreadVisit({ accountId, channelId });
      });
    return () => {
      cancelled = true;
    };
  }, [accountId, channelId, trackChannelVisit]);
  useEffect(() => {
    const read = () => {
      if (
        visitReady &&
        readActive &&
        state.phase === "ready" &&
        !state.hasNewer &&
        document.visibilityState === "visible"
      )
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
  }, [state.phase, state.hasNewer, readCursor, channelId, readActive, visitReady]);

  useEffect(() => {
    const refresh = () => {
      if (readActive && document.visibilityState === "visible") void clientRef.current?.refreshUnreadThreads();
    };
    refresh();
    document.addEventListener("visibilitychange", refresh);
    const timer = setInterval(refresh, 15_000);
    return () => {
      document.removeEventListener("visibilitychange", refresh);
      clearInterval(timer);
    };
  }, [channelId, readActive, signedIn]);

  // The composer resizes itself; keep the conversation pinned to the newest
  // message when it already was. Read the position before resizing, not a
  // delayed bottom callback, and correct only real height changes using the
  // new DOM extent rather than the virtualizer's previous measurements.
  useTimelineMotion(
    scrollerRef,
    state.channelId,
    state.phase === "ready" && !state.hasNewer && !state.loadingNewer,
    scroller,
  );
  const keepLatestInView = useCallback((resize: () => boolean) => {
    const scroller = scrollerRef.current;
    const atBottom = scroller && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 80;
    if (resize() && atBottom && allowFollow.current) scroller.scrollTop = scroller.scrollHeight;
  }, []);
  // A window resize, or a panel taking room from the list, keeps the newest
  // message in view when the reader was already there. The browser clamps
  // scrollTop while the viewport shrinks, so use the position from before it.
  useEffect(() => {
    if (!scroller) return;
    const gap = () => scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight;
    let width = scroller.clientWidth;
    let height = scroller.clientHeight;
    let atBottom = gap() <= 80;
    let frame = 0;
    const scrolled = () => {
      if (scroller.clientWidth === width && scroller.clientHeight === height) atBottom = gap() <= 80;
    };
    const observer = new ResizeObserver(() => {
      if (scroller.clientWidth === width && scroller.clientHeight === height) return;
      width = scroller.clientWidth;
      height = scroller.clientHeight;
      if (!atBottom || !allowFollow.current) return;
      scroller.scrollTop = scroller.scrollHeight;
      // Rewrapped rows are re-measured after this callback.
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => {
        if (atBottom) scroller.scrollTop = scroller.scrollHeight;
      });
    });
    scroller.addEventListener("scroll", scrolled, { passive: true });
    observer.observe(scroller);
    return () => {
      cancelAnimationFrame(frame);
      observer.disconnect();
      scroller.removeEventListener("scroll", scrolled);
    };
  }, [scroller]);
  const setTyping = useCallback((active: boolean) => {
    clientRef.current?.setTyping(active);
  }, []);
  // Someone's name or picture changed elsewhere: their messages here follow.
  useEffect(() => subscribeProfiles(() => clientRef.current?.refreshProfiles()), []);

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
  const toggleSpoiler = (key: string) => {
    if (state.pendingSend) return;
    setDrafts((current) => current.map((item) => (item.key === key ? { ...item, spoiler: !item.spoiler } : item)));
  };
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
      // This page now owns their previews, so leaving without them revokes them.
      for (const draft of saved.drafts)
        for (const url of [draft.localUrl, draft.thumbnailUrl]) if (url) objectUrls.current.add(url);
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
        // An edit replaces the local copy, so look it up rather than keep this one.
        const local = latestDrafts.current.find((item) => item.key === key);
        if (next.attachment && local?.localUrl && local.localKind)
          setLocalPreviews((current) => ({
            ...current,
            [next.attachment!.id]: { url: local.localUrl!, video: local.localKind === "video" },
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
      void editableKind(file).then((editable) => {
        if (editable) stagedFileChanged(key, { editable });
      });
    }
  };

  /** Stages a recorded voice message with the conversation's files: it uploads
   * now (so transcription starts) and goes with the next Send. One per message. */
  const stageVoice = (recording: VoiceRecording, threadRootId?: string) => {
    const channel = state.channelId;
    if (!signedIn || !channel) return;
    const staged = drafts.filter((draft) => draft.threadRootId === threadRootId);
    if (staged.length >= MAX_ATTACHMENTS) {
      setAttachError(`You can attach up to ${MAX_ATTACHMENTS} files.`, threadRootId);
      return;
    }
    if (staged.some((draft) => draft.voice)) return;
    const key = crypto.randomUUID();
    const localUrl = URL.createObjectURL(recording.blob);
    objectUrls.current.add(localUrl);
    const upload = AttachmentUpload.voice(channel, recording, (next) => stagedFileChanged(key, next));
    uploads.current.set(key, upload);
    setDrafts((current) => [
      ...current,
      {
        key,
        threadRootId,
        name: VOICE_LABEL,
        localUrl,
        sourceSize: recording.blob.size,
        progress: 0,
        voice: { durationMs: recording.durationMs, waveform: recording.waveform },
      },
    ]);
    void upload.ensure().catch(() => {});
  };

  /** Revokes a draft's local copy once nothing can show it. */
  const forgetLocalUrl = (url: string | undefined) => {
    if (!url) return;
    URL.revokeObjectURL(url);
    objectUrls.current.delete(url);
    setLocalPreviews((current) =>
      Object.values(current).some((preview) => preview.url === url)
        ? Object.fromEntries(Object.entries(current).filter(([, preview]) => preview.url !== url))
        : current,
    );
  };

  const removeDraft = (key: string) => {
    abandonUpload(key);
    // A removed file is never sent, so its preview can go now rather than when the page closes.
    const removed = latestDrafts.current.find((item) => item.key === key);
    forgetLocalUrl(removed?.localUrl);
    forgetLocalUrl(removed?.thumbnailUrl);
    setDrafts((current) => current.filter((item) => item.key !== key));
  };

  // Saving an edit uploads the original again with it; the tile shows the result.
  const draftEditing: DraftEditing = {
    source: (key) => uploads.current.get(key)?.source,
    save: (key, edit, preview) => {
      const upload = uploads.current.get(key);
      const draft = latestDrafts.current.find((item) => item.key === key);
      const source = upload?.source;
      if (!upload || !draft || !source) return;
      const own = (blob: Blob | undefined) => {
        if (!blob) return;
        const url = URL.createObjectURL(blob);
        objectUrls.current.add(url);
        return url;
      };
      forgetLocalUrl(draft.localUrl);
      forgetLocalUrl(draft.thumbnailUrl);
      // An edited image shows its render; an edited video shows its upload once it lands.
      const localUrl = !edit ? own(source) : draft.editable === "image" ? own(preview) : undefined;
      const thumbnailUrl = edit && draft.editable === "video" ? own(preview) : undefined;
      updateDraft(key, { edit, localUrl, thumbnailUrl, error: undefined });
      void upload.setEdit(edit).catch(() => {});
    },
  };

  const refreshUrls = useCallback((ids: string[]) => {
    void refreshAttachmentUrls(ids).then((urls) => {
      if (Object.keys(urls).length) setFreshUrls((current) => ({ ...current, ...urls }));
    });
  }, []);
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
  // Kept between unrelated renders (typing, presence, the composer), like the rows built from them.
  const channelMessages = useMemo(
    () => state.channelMessages ?? state.messages.filter(isChannelMessage),
    [state.channelMessages, state.messages],
  );
  const messages = useMemo(
    () =>
      state.pendingSend && !state.pendingSend.threadRootId ? [...channelMessages, state.pendingSend] : channelMessages,
    [channelMessages, state.pendingSend],
  );
  const newMessagesId =
    trackChannelVisit && visitReady
      ? channelNewMessagesId(channelMessages, unreadVisit?.entry, state.hasMore, viewerId, blockedIds)
      : undefined;
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
  useLayoutEffect(() => {
    // The typing row resizes the viewport; retain live follow without moving history readers.
    const scroller = scrollerRef.current;
    if (scroller && allowFollow.current && scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 80)
      scroller.scrollTop = scroller.scrollHeight;
  }, [typingLabel]);
  const sendDraft = async (text: string, threadRootId?: string, broadcast = false, suppressEmbeds = false) => {
    const client = clientRef.current;
    const files = drafts.filter((draft) => draft.threadRootId === threadRootId);
    const sources = !state.pendingSend ? files.map((draft) => uploads.current.get(draft.key)!) : [];
    setAttachError(undefined, threadRootId);
    let attachments: ChatAttachment[];
    try {
      attachments = (await Promise.all(sources.map((upload) => upload.ensure()))).map((attachment, index) => ({
        ...attachment,
        spoiler: latestDrafts.current.find((draft) => draft.key === files[index].key)?.spoiler,
      }));
    } catch {
      return;
    } // The file chip owns the upload error and its Retry action.
    if (client !== clientRef.current || sources.some((upload) => ![...uploads.current.values()].includes(upload)))
      return;
    // Retain local previews after Send; the viewer still uses the server's final files.
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
      suppressEmbeds,
    });
    if (sent && !threadRootId && state.hasNewer) {
      setJumpMessage(undefined);
      setScrollTarget("latest");
    }
    return sent;
  };

  // Voice messages upload as recorded, then send on their own: the composer's
  // text and staged files stay where they are. One per composer at a time.
  const voiceUploads = useRef(new Map<string, { upload: AttachmentUpload; broadcast: boolean }>());
  const [voiceSends, setVoiceSends] = useState<Record<string, { error?: string } | undefined>>({});
  const latestState = useRef(state);
  latestState.current = state;
  const setVoiceSend = (key: string, value: { error?: string } | undefined) =>
    setVoiceSends((current) => ({ ...current, [key]: value }));
  const sendVoice = async (key: string) => {
    const entry = voiceUploads.current.get(key);
    const client = clientRef.current;
    if (!entry || !client) return;
    setVoiceSend(key, {});
    let attachment: ChatAttachment;
    try {
      attachment = await entry.upload.ensure();
    } catch (error) {
      if (voiceUploads.current.get(key) === entry)
        setVoiceSend(key, { error: error instanceof UploadError ? error.message : "Failed to upload." });
      return;
    }
    if (voiceUploads.current.get(key) !== entry || client !== clientRef.current) return;
    // The client sends one message at a time and would retry the pending one instead.
    if (latestState.current.pendingSend) {
      setVoiceSend(key, { error: "Wait for your last message to send, then retry." });
      return;
    }
    // From here the pending row owns the file, with its own retry and errors.
    voiceUploads.current.delete(key);
    setVoiceSend(key, undefined);
    const threadRootId = key === "channel" ? undefined : key;
    await client
      .send("", {
        attachments: [attachment],
        uploads: [entry.upload],
        threadRootId,
        broadcast: entry.broadcast,
        replyTo: replyTargets[key],
      })
      .catch(() => false);
  };
  const discardVoice = (key: string) => {
    voiceUploads.current.get(key)?.upload.release(true);
    voiceUploads.current.delete(key);
    setVoiceSend(key, undefined);
  };
  const recordVoice = (recording: VoiceRecording, threadRootId?: string, broadcast = false) => {
    const channel = state.channelId;
    if (!signedIn || !channel) return;
    const key = threadRootId ?? "channel";
    voiceUploads.current.get(key)?.upload.release(true);
    voiceUploads.current.set(key, { upload: AttachmentUpload.voice(channel, recording), broadcast });
    void sendVoice(key);
  };
  useEffect(
    () => () => {
      for (const entry of voiceUploads.current.values()) entry.upload.release(true);
      voiceUploads.current.clear();
      setVoiceSends({});
    },
    [channelId],
  );
  const sendOnStop = useSyncExternalStore(subscribeVoiceSendOnStop, getVoiceSendOnStop, () => false);
  const voiceFor = (threadRootId?: string): VoiceComposer | undefined => {
    if (!signedIn || readOnly) return;
    const key = threadRootId ?? "channel";
    const status = voiceSends[key];
    const staged = drafts.filter((draft) => draft.threadRootId === threadRootId);
    return {
      // Read at the moment recording stops, so a changed preference applies at once.
      onRecorded: (recording, options) =>
        getVoiceSendOnStop()
          ? recordVoice(recording, threadRootId, options?.broadcast)
          : stageVoice(recording, threadRootId),
      sendOnStop,
      blockedReason: staged.some((draft) => draft.voice)
        ? "Remove the voice message to record another"
        : staged.length >= MAX_ATTACHMENTS
          ? `You can attach up to ${MAX_ATTACHMENTS} files.`
          : undefined,
      sending: !!status && !status.error,
      error: status?.error,
      onRetry: () => void sendVoice(key),
      onDiscard: () => discardVoice(key),
    };
  };

  /** A KLIPY GIF goes on its own, replying where the composer would; drafts stay. */
  const sendGif = async (gif: ChatGif, query: string, threadRootId?: string, broadcast = false) => {
    const client = clientRef.current;
    if (!client || state.pendingSend) throw new Error("Wait for the last message to send.");
    const sent = await client.send("", {
      gif,
      threadRootId,
      broadcast,
      replyTo: replyTargets[threadRootId ?? "channel"],
    });
    if (sent) {
      const key = await klipyConfig();
      if (key) shareKlipy(key, gif, query);
      if (!threadRootId && state.hasNewer) {
        setJumpMessage(undefined);
        setScrollTarget("latest");
      }
    }
    return sent;
  };

  // Share the formatters across visible rows (memoized rows compare them), but
  // follow a locale or timezone change.
  const { locale, timeZone } = new Intl.DateTimeFormat().resolvedOptions();
  const timeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(undefined, {
        hour: "numeric",
        minute: "2-digit",
      }),
    [locale, timeZone],
  );
  const pinTimeFormatter = useMemo(
    () =>
      new Intl.DateTimeFormat(undefined, {
        dateStyle: "medium",
        timeStyle: "short",
      }),
    [locale, timeZone],
  );
  // Mention labels name people from the directory; callers rebuild it on every
  // render, so rows get a lookup that only changes when the names do.
  const directoryNames = mentionDirectory
    .map((candidate) => `${candidate.id}\u0000${candidate.displayName}`)
    .join("\u0001");
  const mentionNames = useMemo(() => {
    const names = new Map<string, string>();
    for (const candidate of mentionDirectory)
      if (!names.has(candidate.id)) names.set(candidate.id, candidate.displayName);
    return names;
  }, [directoryNames]);
  // Same person, within five minutes, same day, and no blocked-run bar in between.
  const joinsPrevious = (
    previous: (typeof messages)[number] | undefined,
    message: (typeof messages)[number] | undefined,
    inThread: boolean,
  ) => {
    if (!hydrated || !previous || !message) return false;
    if (!inThread && "id" in message && message.id === newMessagesId) return false;
    const runs = inThread ? threadRuns : channelRuns;
    const run = "content" in message ? runs.get(message.clientMessageId) : undefined;
    if (run && (run.first === message.clientMessageId || !revealedRuns.has(run.first))) return false;
    const previousRun = "content" in previous ? runs.get(previous.clientMessageId) : undefined;
    if (previousRun && !revealedRuns.has(previousRun.first)) return false;
    return groupsWithPrevious(previous, message, { inThread });
  };
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
    const run = pending ? undefined : (inThread ? threadRuns : channelRuns).get(message.clientMessageId);
    const files = "content" in message ? attachmentsOf(message).length : (message.attachments?.length ?? 0);
    // A pending thread reply shows its status beside the thread composer.
    const channelPending = pending && !inThread;
    return (
      <MessageRow
        key={message.clientMessageId}
        message={message}
        inThread={inThread}
        divider={hydrated && !inThread ? dateDivider(message.createdAt, messages[index - 1]?.createdAt) : undefined}
        runFirst={run?.first}
        runCount={run?.count}
        runShown={!!run && revealedRuns.has(run.first)}
        grouped={joinsPrevious(inThread ? previousInThread : messages[index - 1], message, inThread)}
        // The row above a follow-up gives up its bottom padding so the run reads as one block.
        continued={joinsPrevious(message, inThread ? nextInThread : messages[index + 1], inThread)}
        newMessages={!inThread && !pending && message.id === newMessagesId}
        arriving={!!arrivals.current?.has(message.clientMessageId)}
        jumpTarget={!pending && jumpMessage === message.id}
        threadActive={!inThread && !pending && state.thread?.rootId === message.id}
        threadUnread={!pending && !!state.unreadThreads?.[message.id]}
        actionMode={!pending && actionTarget?.messageId === message.id ? actionTarget.mode : undefined}
        reactionSave={pending ? undefined : reactionSaves[message.id]}
        replyHidden={
          !!message.replyTo && blockedIds.has(message.replyTo.author.id) && message.replyTo.author.id !== viewerId
        }
        hydrated={hydrated}
        name={name}
        viewerId={viewerId}
        ownId={state.author?.id}
        readOnly={readOnly}
        replyDisabled={!!state.pendingSend}
        channelId={state.channelId}
        timeFormatter={timeFormatter}
        mentionNames={mentionNames}
        attachmentProgress={files ? state.attachmentProgress : undefined}
        freshUrls={files ? freshUrls : undefined}
        localPreviews={files ? localPreviews : undefined}
        sendError={channelPending ? state.sendError : undefined}
        sendUploadFailed={channelPending ? state.sendUploadFailed : undefined}
        sendRejected={channelPending ? state.sendRejected : undefined}
        editBlocked={channelPending && state.sendRejected ? hasDraft : undefined}
        actions={rowActions}
      />
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
  const rowActions = useLatestActions<RowActions>({
    cancelPress,
    holdPress: (event, onHold, row = false) => {
      const anchor = event.currentTarget;
      if (row) anchor.dataset.pressing = "true";
      press.current = {
        row: row ? anchor : undefined,
        x: event.clientX,
        y: event.clientY,
        pointerId: event.pointerId,
        timer: setTimeout(() => {
          suppressClick.current = true;
          onHold(anchor);
        }, 500),
      };
    },
    movePress: (event) => {
      const current = press.current;
      if (
        current &&
        (event.pointerId !== current.pointerId || Math.hypot(event.clientX - current.x, event.clientY - current.y) > 10)
      )
        cancelPress();
    },
    suppressNextClick: () => {
      suppressClick.current = true;
    },
    openAuthor,
    openMention: (user, anchor) => {
      setActionTarget(undefined);
      setMentionCard({
        person: mentionCardPerson(user, mentionDirectory, state.author?.id),
        anchor,
        drawer: isTouchLayout(),
      });
    },
    openActions,
    toggleActions: (messageId, anchor, inThread) =>
      actionTarget?.messageId === messageId && actionTarget.mode === "actions"
        ? setActionTarget(undefined)
        : openActions(messageId, anchor, inThread),
    toggleReactionPicker: (messageId, anchor, inThread) =>
      setActionTarget(
        actionTarget?.messageId === messageId && actionTarget.mode === "emoji"
          ? undefined
          : {
              messageId,
              anchor,
              anchorRect: anchor.getBoundingClientRect(),
              mode: "emoji",
              drawer: isTouchLayout(),
              inThread,
            },
      ),
    openThread,
    prefetchThread,
    openHistory,
    openReply: (reference) => void goToMessage({ ...reference, channelId: state.channelId! }),
    toggleRun,
    reply: replyToMessage,
    react,
    showReactors,
    dismissReactionError: (messageId) => setReactionSaves((current) => ({ ...current, [messageId]: undefined })),
    viewFile,
    openConversation: (message, anchor) => void openConversation(message, anchor),
    refreshUrls,
    editRejected: () => {
      setReplyTarget("channel", state.pendingSend?.replyTo);
      const text = clientRef.current?.discardRejected();
      if (text !== undefined) composerRef.current?.restore(text);
    },
    discardRejected: () => {
      clientRef.current?.discardRejected();
    },
    retrySend: () => composerRef.current?.submit(),
  });

  const pinsOpen = showPins && state.phase === "ready";
  // Opened from pins, the viewer renders inside that dialog, which then treats it as its own layer.
  const viewer = viewerTarget && viewerSource && (
    <MediaViewer
      // Forwarded originals have no URL refresh, like their inline previews.
      attachments={
        viewerTarget.forwarded ? attachmentsOf(viewerSource) : withFreshUrls(attachmentsOf(viewerSource), freshUrls)
      }
      attachmentId={viewerTarget.attachmentId}
      anchor={viewerTarget.anchor}
      author={viewerSource.author.name}
      sentAt={viewerSource.createdAt}
      onClose={closeViewer}
      onExpired={viewerTarget.forwarded ? undefined : refreshUrls}
      onHideSpoiler={viewerTarget.onHideSpoiler}
    />
  );

  const headingAlerts = (state.phase === "ready" && !!state.error) || !!pinError;
  const offlineStatus = (
    <span className="chat-offline" role="status">
      {state.phase === "error" ? "Offline" : "Connecting…"}
    </span>
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
            {direct && directPeer && (
              <span className="chat-peer-avatar">
                <Avatar person={directPeer} name={channelName} />
              </span>
            )}
            <span className="chat-title-text">
              {direct ? "" : "# "}
              {channelName}
              {visitor && !direct && <span className="chat-channel-space"> · {state.spaceName}</span>}
            </span>
          </h2>
          {/* Phones: the name opens the channel's menu, as in Slack; Pins and Members move there. */}
          {showTitle && state.phase === "ready" && (
            <details
              className="chat-title-menu"
              onClick={(event) => {
                // Choosing an item closes the menu. Focus its visible summary first, so a
                // dialog opened from the item (Pins) returns focus there, not to the hidden item.
                if ((event.target as HTMLElement).closest(".chat-header-menu button")) {
                  event.currentTarget.open = false;
                  event.currentTarget.querySelector("summary")?.focus();
                }
              }}
              onKeyDown={(event) => {
                if (event.key === "Escape" && event.currentTarget.open) {
                  event.preventDefault();
                  event.currentTarget.open = false;
                  event.currentTarget.querySelector("summary")?.focus();
                }
              }}
              onBlur={(event) => {
                // Safari can blur the summary without focusing a tapped menu button.
                // Keep the item mounted until its click; outside taps close the menu
                // through Spaces' pointer handler. A known focus destination still
                // closes it when keyboard navigation leaves the menu.
                if (event.relatedTarget && !event.currentTarget.contains(event.relatedTarget))
                  event.currentTarget.open = false;
              }}
            >
              <summary aria-label={`${direct ? "" : "#"}${channelName} options`}>
                <span className="chat-title-name">
                  {direct && directPeer && (
                    <span className="chat-peer-avatar">
                      <Avatar person={directPeer} name={channelName} />
                    </span>
                  )}
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
          {pinsToggle}
          {headerActions}

          {!state.online && showConnectionStatus && !headingAlerts && offlineStatus}
          {headingAlerts && (
            <div className="chat-heading-alerts">
              {/* Stacked with the alerts rather than drawn underneath them. */}
              {!state.online && showConnectionStatus && offlineStatus}
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
        {headerBelow}

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
                  deleteTarget ||
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
                    <article
                      className={`chat-message chat-pinned-message chat-message-pinned${message.pin?.author.id === viewerId ? " chat-message-pinned-own" : ""}`}
                      key={message.id}
                      aria-busy={jumping === message.id}
                      onClick={(event) => {
                        // The Go button is the keyboard action. Plain card clicks do
                        // the same; links, media, profiles and text selection keep theirs.
                        if (
                          event.defaultPrevented ||
                          (event.target as Element).closest(
                            "button, a, input, textarea, audio, video, [role=button]",
                          ) ||
                          window.getSelection()?.isCollapsed === false
                        )
                          return;
                        void goToMessage(message);
                      }}
                    >
                      {message.replyTo && (
                        <ReplyContext
                          motionKey={message.clientMessageId}
                          reference={message.replyTo}
                          hidden={blockedIds.has(message.replyTo.author.id) && message.replyTo.author.id !== viewerId}
                          onOpen={() => void goToMessage({ ...message.replyTo!, channelId: state.channelId! })}
                        />
                      )}
                      {message.pin && (
                        <PinMarker author={message.pin.author} viewerId={viewerId} actions={rowActions} />
                      )}
                      <AuthorAvatar author={message.author} name={name} actions={rowActions} />
                      <header>
                        <AuthorName author={message.author} name={name} actions={rowActions} />
                        <time dateTime={message.createdAt}>
                          {hydrated ? timeLabel(message.createdAt, pinTimeFormatter) : ""}
                        </time>
                      </header>
                      {message.content.text && (
                        <p>
                          <MessageText message={message} mentionNames={mentionNames} actions={rowActions} />
                        </p>
                      )}
                      <MessageAttachments
                        attachments={withFreshUrls(attachmentsOf(message), freshUrls)}
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
                        title="More actions"
                        aria-haspopup="dialog"
                        aria-expanded={actionTarget?.messageId === message.id && actionTarget.mode === "actions"}
                        onClick={(event) =>
                          actionTarget?.messageId === message.id && actionTarget.mode === "actions"
                            ? setActionTarget(undefined)
                            : openActions(message.id, event.currentTarget, false)
                        }
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
          <div className="chat-timeline" inert={pinsOpen} aria-hidden={pinsOpen} {...keyboardDismiss}>
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
                  setScroller(scrollerRef.current);
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
                  <MediaPreviewContext value={true}>
                    {messages.slice(previewStart).map((message, index) => renderMessage(previewStart + index, message))}
                  </MediaPreviewContext>
                </div>
              </div>
            )}
            <p className="sr-only" aria-live="polite" aria-atomic="true">
              {state.phase === "ready" && announcement}
            </p>
            <p className="sr-only" role="status">
              {actionStatus?.text}
            </p>
            <p className="chat-action-status" aria-hidden="true" key={actionStatus?.key}>
              {actionStatus?.text}
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

        {actionTarget && actionMessage && !actionMessage.deletedAt && (
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
            canDelete={
              !actionMessage.deletedAt &&
              !readOnly &&
              !state.author?.isGuest &&
              state.author?.id === actionMessage.author.id
            }
            onDelete={() => {
              actionTarget.anchor.focus({ preventScroll: true });
              setActionTarget(undefined);
              setEditTarget(undefined);
              setHistoryTarget(undefined);
              setDeleteTarget(actionMessage.id);
            }}
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
              onBlockAuthor &&
              !actionMessage.author.isGuest &&
              actionMessage.author.id !== state.author?.id &&
              (!actionMessage.author.admin || blockedIds.has(actionMessage.author.id))
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
        {forwardTarget && forwardMessage && !forwardMessage.deletedAt && (
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
        <p className="chat-typing" role="status" aria-atomic="true" data-visible={!!typingLabel}>
          {typingLabel && (
            <span className="chat-typing-content">
              <span className="chat-typing-dots" aria-hidden="true">
                <i />
                <i />
                <i />
              </span>
              <span>{typingLabel}</span>
            </span>
          )}
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
              onSend={(text, options) => sendDraft(text, undefined, false, options?.suppressEmbeds)}
              linkPreviews={signedIn}
              onSendGif={signedIn && !readOnly ? (gif, query) => sendGif(gif, query) : undefined}
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
                      onToggleSpoiler: toggleSpoiler,
                      // A pending send carries these files; editing would cancel them under it.
                      editing: state.pendingSend ? undefined : draftEditing,
                      voice: voiceFor(),
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
        readActive={readActive}
        mentionPeople={mentionPeople}
        specialMentions={!direct}
        replyTo={state.thread ? replyTargets[state.thread.rootId] : undefined}
        onSend={(text, broadcast) => sendDraft(text, state.thread?.rootId, broadcast)}
        onSendGif={
          signedIn && !readOnly
            ? (gif, query, broadcast) => sendGif(gif, query, state.thread?.rootId, broadcast)
            : undefined
        }
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
                onToggleSpoiler: toggleSpoiler,
                editing: state.pendingSend ? undefined : draftEditing,
                voice: state.thread ? voiceFor(state.thread.rootId) : undefined,
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
      {state.phase === "ready" && !readOnly && editMessage && !editMessage.deletedAt && clientRef.current && (
        <MessageEditor
          key={editMessage.id}
          message={editMessage}
          onSave={(text, revision) => clientRef.current!.editMessage(editMessage.id, text, revision)}
          onReload={() => clientRef.current!.reloadMessage(editMessage.id)}
          onClose={() => setEditTarget(undefined)}
        />
      )}
      {state.phase === "ready" && !readOnly && deleteMessage && !deleteMessage.deletedAt && clientRef.current && (
        <DeleteMessage
          key={deleteMessage.id}
          message={deleteMessage}
          onDelete={() => clientRef.current!.deleteMessage(deleteMessage.id)}
          onClose={() => setDeleteTarget(undefined)}
        />
      )}
      {state.phase === "ready" && historyMessage && !historyMessage.deletedAt && (
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
