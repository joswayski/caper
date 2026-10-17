import {
  Fragment,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  useSyncExternalStore,
  type PointerEvent as ReactPointerEvent,
  type ReactNode,
  type RefObject,
} from "react";
import { animals, colors, uniqueNamesGenerator } from "unique-names-generator";
import {
  ArrowLeft,
  AudioLines,
  ChevronDown,
  Hash,
  LockKeyhole,
  HeadphoneOff,
  Headphones,
  Mic,
  MicOff,
  PhoneOff,
  Settings,
  Users,
  Volume2,
  VolumeX,
  X,
} from "lucide-react";
import ProfileForm from "../account/ProfileForm";
import { hostProfileEditor } from "../account/profile-editor";
import NotificationSettings from "../account/NotificationSettings";
import PrivacySettings from "../account/PrivacySettings";
import WhatsNew from "../updates/WhatsNew";
import { getAccount, getRememberedAccount, logout, subscribeAccount, type Account } from "../account/client";
import { withPicture } from "../account/pictures";
import { routeOutput } from "../audio/output";
import { rosterChanges } from "../audio/roster";
import {
  getSystemSoundsEnabled,
  playSound,
  preloadSoundEffects,
  setSystemSoundsEnabled,
  subscribeSystemSounds,
} from "../audio/effects";
import Chat, { type FocusTarget } from "../chat/Chat";
import type { ChatAuthor, GeneralChatHistory } from "../chat/types";
import type { MentionCandidate } from "../chat/mentions";
import type { Space } from "../spaces/client";
import Slider from "../components/Slider";
import PresenceDot from "../components/PresenceDot";
import Avatar from "../components/Avatar";
import Tooltip from "../components/Tooltip";
import Wordmark from "../components/Wordmark";
import { useBackdropDismiss } from "../components/dialogBackdrop";
import { dialogKeyDown } from "../components/dialogKeyboard";
import { watchPresence as watchAccountPresence, type PresenceStatus } from "../gateway/client";
import { acquireAudioContext, releaseAudioContext, setPlaybackBlocked } from "../media/audio-context";
import { PublicCallClient, prepareVoiceJoin } from "../media/client";
import { hasWarmVoice, keepVoiceWarm, stopVoiceWarm } from "../media/warm";
import { watchPresence } from "../media/presence";
import { sessionDuration } from "../media/session-duration";
import type { CallViewState, Participant } from "../media/types";
import { DEFAULT_VOICE_PROCESSING_STRENGTH } from "../media/voice-processing";
import MicPlayback, { SpeakerTest } from "./MicPlayback";
import AudioDiagnostics from "./AudioDiagnostics";
import VoiceActivity from "./VoiceActivity";
import ChannelSidebar from "./ChannelSidebar";
import { useBrowseSwipe } from "../spaces/browseTransition";
import { useMembersDrawer } from "../spaces/membersDrawer";
import { STRIP_FACES, callCardFaces, liveNames } from "../media/live-now";
import "./call.css";

const initialState: CallViewState = {
  phase: "idle",
  muted: false,
  deafened: false,
  inputVolume: 100,
  voiceProcessingStrength: DEFAULT_VOICE_PROCESSING_STRENGTH,
  monitoring: false,
  participants: [],
  remoteMedia: [],
};
type PublicPresence = { participants: Array<Omit<Participant, "tracks">>; sessionStartedAt?: number | null };
const MAX_WATCHED_CHANNELS = 24;

function VoiceSessionTimer({ startedAt }: { startedAt: number }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const tick = () => setNow(Date.now());
    tick();
    const timer = window.setInterval(tick, 1_000);
    document.addEventListener("visibilitychange", tick);
    return () => {
      window.clearInterval(timer);
      document.removeEventListener("visibilitychange", tick);
    };
  }, [startedAt]);
  return (
    <span
      className="voice-session-timer"
      role="timer"
      aria-live="off"
      aria-label="Voice session duration"
      title="How long this voice session has been active"
    >
      {sessionDuration(startedAt, now)}
    </span>
  );
}

/** How close (px) the pointer must come to a Join button to start preparing the join. */
const JOIN_PREPARE_RADIUS = 120;

function formatBytes(bytes: number) {
  return `${(bytes / 1e6).toFixed(2)} MB`;
}

function formatBitrate(bitsPerSecond: number) {
  return `${Math.round(bitsPerSecond / 1_000)} kbps`;
}

function deviceOptions(devices: MediaDeviceInfo[], kind: MediaDeviceKind) {
  const labels = new Set<string>();
  return devices
    .filter((device) => device.kind === kind)
    .sort((left, right) => Number(right.deviceId === "default") - Number(left.deviceId === "default"))
    .flatMap((device) => {
      const label =
        device.label.replace(/^Default\s*[-–]\s*/i, "") || (kind === "audioinput" ? "Microphone" : "Audio output");
      const key = label.toLocaleLowerCase();
      if (labels.has(key)) return [];
      labels.add(key);
      return [{ device, label }];
    });
}

function ConnectionDiagnostics({ diagnostics }: { diagnostics: NonNullable<CallViewState["diagnostics"]> }) {
  const [copyStatus, setCopyStatus] = useState("");
  const values = [
    ["Joined", diagnostics.join],
    [
      "Microphone",
      `${Math.round(diagnostics.microphoneMs)} ms${diagnostics.microphoneDetail ? ` (${diagnostics.microphoneDetail})` : ""}`,
    ],
    ["Session + publish", `${Math.round(diagnostics.sessionMs)} ms`],
    ["Signaling + live updates", `${Math.round(diagnostics.signalingMs)} ms`],
    [
      "Transport + state",
      `${Math.round(diagnostics.transportMs)} ms${diagnostics.iceMs === undefined ? "" : ` (ICE ${Math.round(diagnostics.iceMs)} ms)`}`,
    ],
    ["Connectivity checks", diagnostics.checks ?? "Not observed yet"],
    ["Roster", `${Math.round(diagnostics.rosterMs)} ms`],
    ...(diagnostics.hearingMs === undefined
      ? []
      : [["Hearing others", `${Math.round(diagnostics.hearingMs)} ms from Join`]]),
    ["Received", formatBytes(diagnostics.receivedBytes)],
    ["Live receive", formatBitrate(diagnostics.receiveBitrate)],
    ["Sent", formatBytes(diagnostics.sentBytes)],
    ["Live send", formatBitrate(diagnostics.sendBitrate)],
    ["Packets lost", String(diagnostics.packetsLost)],
    ["Max jitter", `${Math.round(diagnostics.maxJitterMs)} ms`],
    ["RTT", `${Math.round(diagnostics.roundTripMs)} ms`],
    [
      "Route",
      diagnostics.route === "relay" ? "TURN relay" : diagnostics.route === "direct" ? "Direct" : "Not observed yet",
    ],
  ];
  return (
    <section className="call-diagnostics" aria-label="Connection statistics">
      <dl>
        {values.map(([label, value]) => (
          <div key={label}>
            <dt>{label}</dt>
            <dd>{value}</dd>
          </div>
        ))}
      </dl>
      <p>Counters reset on reconnect.</p>
      <button
        type="button"
        className="voice-button"
        onClick={() =>
          void navigator.clipboard.writeText(JSON.stringify(diagnostics, null, 2)).then(
            () => setCopyStatus("Copied connection details"),
            () => setCopyStatus("Copy failed; try again."),
          )
        }
      >
        Copy connection details
      </button>
      <span role="status">{copyStatus}</span>
    </section>
  );
}

function AudioMenu({
  label,
  settings,
  open,
  onOpenChange,
  menuRef,
  children,
}: {
  label: string;
  settings?: boolean;
  open: boolean;
  onOpenChange: (open: boolean) => void;
  menuRef?: RefObject<HTMLDivElement | null>;
  children: ReactNode;
}) {
  const panelId = useId();
  const panelRef = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    const panel = panelRef.current;
    const anchor = panel?.closest(".call-account");
    if (!open || !panel || !anchor) return;
    panel.showPopover();
    const position = () => {
      const viewport = window.visualViewport;
      const x = viewport?.offsetLeft ?? 0;
      const y = viewport?.offsetTop ?? 0;
      const width = viewport?.width ?? window.innerWidth;
      const height = viewport?.height ?? window.innerHeight;
      const bounds = anchor.getBoundingClientRect();
      const edge = 8;
      const panelWidth = Math.min(settings ? bounds.width : 280, width - edge * 2);
      panel.style.width = `${panelWidth}px`;
      const above = Math.max(0, bounds.top - y - edge * 2);
      const below = Math.max(0, y + height - bounds.bottom - edge * 2);
      const down = panel.scrollHeight + 2 > above && below > above;
      panel.style.maxHeight = `${Math.min(height - edge * 2, down ? below : above)}px`;
      const panelHeight = panel.getBoundingClientRect().height;
      panel.style.left = `${Math.max(x + edge, Math.min(settings ? bounds.left : bounds.right - panelWidth, x + width - panelWidth - edge))}px`;
      panel.style.top = `${Math.max(y + edge, Math.min(down ? bounds.bottom + edge : bounds.top - panelHeight - edge, y + height - panelHeight - edge))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    observer.observe(panel);
    observer.observe(anchor);
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    window.visualViewport?.addEventListener("resize", position);
    window.visualViewport?.addEventListener("scroll", position);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
      window.visualViewport?.removeEventListener("resize", position);
      window.visualViewport?.removeEventListener("scroll", position);
    };
  }, [open, settings]);
  return (
    <div
      ref={menuRef}
      className={`call-settings ${settings ? "" : "device-menu"}`}
      onKeyDown={(event) => {
        // Let native device pickers handle Escape without also removing their panel.
        if (event.target instanceof HTMLSelectElement) return;
        if (event.key === "Escape" && open && !event.defaultPrevented) {
          // Handled: an open thread beneath must not also close on this Escape.
          event.preventDefault();
          onOpenChange(false);
          event.currentTarget.querySelector<HTMLButtonElement>(".call-settings-trigger")?.focus();
        }
      }}
    >
      {/* The same styled tooltip as Mute/Deafen beside it, not the slower native title. */}
      <Tooltip content={open ? undefined : label}>
        <button
          type="button"
          className="call-settings-trigger"
          aria-label={label}
          aria-expanded={open}
          aria-controls={open ? panelId : undefined}
          onClick={() => onOpenChange(!open)}
        >
          {settings ? <Settings aria-hidden="true" /> : <ChevronDown aria-hidden="true" />}
        </button>
      </Tooltip>
      {open && (
        <div
          ref={panelRef}
          id={panelId}
          className="call-settings-panel"
          popover="manual"
          role="group"
          aria-label={`${label} panel`}
          tabIndex={-1}
        >
          {children}
        </div>
      )}
    </div>
  );
}

function AudioOutput({
  stream,
  muted,
  name,
  output,
  volume,
}: {
  stream: MediaStream;
  muted: boolean;
  name: string;
  output: string;
  volume: number;
}) {
  const ref = useRef<HTMLAudioElement>(null);
  const gainRef = useRef<GainNode | null>(null);
  const contextRef = useRef<AudioContext | null>(null);
  const [blocked, setBlocked] = useState(false);
  const [deviceError, setDeviceError] = useState(false);
  useEffect(() => {
    const key = {};
    setPlaybackBlocked(key, blocked);
    return () => setPlaybackBlocked(key, false);
  }, [blocked]);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    // Chromium delivers silence from a remote WebRTC stream into Web Audio unless
    // that stream is also attached to a media element. This muted sink never plays.
    const sink = new Audio();
    sink.muted = true;
    sink.srcObject = stream;
    void sink.play().catch(() => undefined);
    let context: AudioContext | undefined;
    let source: MediaStreamAudioSourceNode | undefined;
    let gain: GainNode | undefined;
    let destination: MediaStreamAudioDestinationNode | undefined;
    try {
      context = acquireAudioContext();
      source = context.createMediaStreamSource(stream);
      gain = context.createGain();
      destination = context.createMediaStreamDestination();
      source.connect(gain).connect(destination);
      gain.gain.value = volume / 100;
      gainRef.current = gain;
      contextRef.current = context;
      element.srcObject = destination.stream;
      void Promise.all([context.resume(), element.play()])
        .then(() => setBlocked(false))
        .catch(() => setBlocked(true));
    } catch {
      source?.disconnect();
      gain?.disconnect();
      destination?.stream.getTracks().forEach((track) => track.stop());
      if (context) releaseAudioContext(context);
      context = undefined;
      element.srcObject = stream;
      element.volume = Math.min(volume / 100, 1);
      void element
        .play()
        .then(() => setBlocked(false))
        .catch(() => setBlocked(true));
    }
    return () => {
      element.srcObject = null;
      sink.pause();
      sink.srcObject = null;
      source?.disconnect();
      gain?.disconnect();
      destination?.stream.getTracks().forEach((track) => track.stop());
      gainRef.current = null;
      contextRef.current = null;
      if (context) releaseAudioContext(context);
    };
  }, [stream]);
  useEffect(() => {
    const gain = gainRef.current;
    if (gain) gain.gain.setValueAtTime(volume / 100, gain.context.currentTime);
    else if (ref.current) ref.current.volume = Math.min(volume / 100, 1);
  }, [volume]);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    let current = true;
    void routeOutput(element, output).then((routed) => {
      if (current) setDeviceError(!routed);
    });
    return () => {
      current = false;
    };
  }, [output]);
  return (
    <>
      <audio ref={ref} autoPlay muted={muted} />
      {blocked && (
        <button
          type="button"
          className="voice-button audio-unblock"
          onClick={() =>
            void Promise.all([contextRef.current?.resume(), ref.current?.play()])
              .then(() => setBlocked(false))
              .catch(() => setBlocked(true))
          }
        >
          <Volume2 aria-hidden="true" />
          Play {name} audio
        </button>
      )}
      {deviceError && (
        <p className="call-error" role="alert">
          Audio output unavailable; choose another device.
        </p>
      )}
    </>
  );
}

/**
 * Voice for one channel row: a quiet "N in voice" mark while people talk there,
 * and the one-click join button (revealed on hover for fine pointers, always shown
 * on quiet rows for touch). A live row grows into a card around its line: your
 * call (faces, mute, deafen and Leave) or another room (faces, names and Join).
 */
export interface VoiceSlot {
  live: ReactNode;
  join: ReactNode;
  /** Set while the row is a card: your call, or a room with people in voice. */
  card?: "call" | "room";
  /** Wraps the row's line, always in the same element so the line survives becoming a card. */
  frame: (line: ReactNode) => ReactNode;
}

/** A long press on a face (touch) opens everyone's volume, as a right-click does. */
const LONG_PRESS_MS = 500;

interface CallProps {
  channel?: {
    id: string;
    name: string;
    spaceName: string;
    spaceId?: string;
    demo?: boolean;
    direct?: boolean;
    directPeerId?: string;
    /** A DM with the Caper account: messages go to the feedback team. */
    team?: boolean;
    joined?: boolean;
    /** A feedback team member taking part without having joined. */
    visitor?: boolean;
  };
  /**
   * Shown in place of the conversation when no channel is open (a space with no
   * joined channel). The room stays mounted, so voice in another channel keeps
   * playing with its dock; like a DM, this view has no voice of its own.
   */
  stage?: ReactNode;
  /** The space being viewed when `stage` replaces its conversation. */
  space?: { id: string; name: string; demo?: boolean };
  onReadCursor?: (seq: string) => void;
  channelActions?: ReactNode;
  /** Extra items for the phone header menu, where wide-header actions move. */
  headerMenuItems?: ReactNode;
  membersPanel?: (onClose: () => void) => ReactNode;
  spaceRail?: ReactNode;
  /** Channels in the current space, whose voice shows on their rows. */
  voiceChannels?: Array<{ id: string; name: string; private?: boolean }>;
  /**
   * Channel list. As a function it receives voiceFor, for each row's voice mark, join
   * button and live card, and the cards for voice without a row here (your call in
   * another space, a visited room) to place above the channels.
   */
  channelNavigation?:
    | ReactNode
    | ((voiceFor: (channelId: string) => VoiceSlot | null, liveNow: ReactNode) => ReactNode);
  /** Opens a channel's conversation from its live card, in any space. */
  onOpenChannel?: (spaceId: string | undefined, channelId: string) => void;
  navigationOpen?: boolean;
  onNavigationToggle?: () => void;
  /** Sets Browse at once; swipes on phones animate it themselves. */
  onNavigationChange?: (open: boolean) => void;
  initialAccount?: Account;
  initialHistory?: GeneralChatHistory;
  initialHistoryError?: string;
  onHistoryChange?: (history: GeneralChatHistory) => void;
  /** Rendered inside another page (the homepage window) rather than as the page itself. */
  embedded?: boolean;
  /**
   * False until the visitor engages with an embedded room. Until then the room
   * stays read-only: no guest chat session, microphone model downloads, or sounds.
   */
  engaged?: boolean;
  onChatOnlineChange?: (online: boolean) => void;
  /** People the composer's `@` can suggest; undefined until loaded. */
  mentionMembers?: MentionCandidate[];
  /** Profile-card lookup for mention pills, most specific first. */
  mentionDirectory?: MentionCandidate[];
  /** Opens (or starts) a DM from a mention's profile card. */
  onMessagePerson?: (username: string) => Promise<void>;
  profileSpaces?: (personId: string) => Space[];
  onOpenForward?: (message: GeneralChatHistory["messages"][number]) => Promise<boolean>;
  focusMessage?: FocusTarget;
  /** Also open the focused message's own thread (a feedback report's replies live there). */
  focusThread?: boolean;
  /** After the viewer's message is accepted. */
  onSent?: () => void;
  /** A notice above the composer, such as a DM request still waiting. */
  composerBanner?: ReactNode;
  /** Offers "Block" in message actions; the caller confirms. */
  onBlockAuthor?: (author: ChatAuthor) => void;
}

export default function Call({
  channel,
  stage,
  space,
  onReadCursor,
  channelActions,
  headerMenuItems,
  voiceChannels,
  spaceRail,
  channelNavigation,
  onOpenChannel,
  membersPanel,
  navigationOpen = false,
  onNavigationToggle,
  onNavigationChange,
  initialAccount,
  initialHistory,
  initialHistoryError,
  onHistoryChange,
  embedded = false,
  engaged = true,
  onChatOnlineChange,
  mentionMembers,
  mentionDirectory,
  onMessagePerson,
  profileSpaces,
  onOpenForward,
  focusMessage,
  focusThread,
  onSent,
  composerBanner,
  onBlockAuthor,
}: CallProps = {}) {
  const systemSounds = useSyncExternalStore(subscribeSystemSounds, getSystemSoundsEnabled, () => true);
  const [state, setState] = useState(initialState);
  const [name, setName] = useState(initialAccount?.displayName ?? "");
  const [loadedAccount, setAccount] = useState<Account | null>(initialAccount ?? null);
  const rememberedAccount = useSyncExternalStore(subscribeAccount, getRememberedAccount, () => undefined);
  const account = rememberedAccount ?? loadedAccount;
  // A display name saved elsewhere (another device, via a live profile update) renames you here too.
  useEffect(() => {
    if (rememberedAccount?.displayName) setName(rememberedAccount.displayName);
  }, [rememberedAccount?.displayName]);
  const [chatAuthor, setChatAuthor] = useState<ChatAuthor>();
  const [identityReady, setIdentityReady] = useState(!!initialAccount);
  const [availability, setAvailability] = useState<Record<string, boolean>>({});
  const [audioPanel, setAudioPanel] = useState<"mic" | "connection" | "debug">();
  const [audioMenu, setAudioMenu] = useState<"input" | "output" | "settings">();
  const [whatsNewOpen, setWhatsNewOpen] = useState(false);
  const audioMenuRef = useRef<HTMLDivElement>(null);
  const audioDialog = useRef<HTMLDialogElement>(null);
  const audioReturnFocus = useRef<HTMLElement | null>(null);
  const [profileOpen, setProfileOpen] = useState(false);
  const profileDialog = useRef<HTMLDialogElement>(null);
  const profileBackdrop = useBackdropDismiss(() => setProfileOpen(false));
  // Your own profile card in chat or the member list opens this dialog too.
  useEffect(() => (account ? hostProfileEditor(() => setProfileOpen(true)) : undefined), [!!account]);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = useState("");
  const [output, setOutput] = useState("");
  const [outputVolume, setOutputVolume] = useState(100);
  // Browser capability, so it is read after hydration to match server markup.
  const [outputSelectable, setOutputSelectable] = useState(false);
  useEffect(
    () => setOutputSelectable(typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype),
    [],
  );
  const [membersVisible, setMembersVisible] = useState(true);
  const wideMembersVisible = useRef(true);
  const [narrow, setNarrow] = useState(false);
  useEffect(() => {
    const media = window.matchMedia("(max-width: 760px)");
    // Phone layouts start, and narrowing windows return, with the conversation instead of
    // the wide member column reappearing as a drawer over it; widening restores the column.
    const update = () => {
      setNarrow(media.matches);
      setMembersVisible(media.matches ? false : wideMembersVisible.current);
    };
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);
  // Phones show the conversation without the sidebar, so voice errors move into it.
  // A phone showing the conversation, with Browse hidden behind it.
  const phoneReading = narrow && !navigationOpen && !!onNavigationToggle;
  const inlineVoiceError = phoneReading;
  const roomRef = useRef<HTMLElement>(null);
  const browsing = narrow && navigationOpen && !!onNavigationToggle;
  const members = useMembersDrawer(roomRef, narrow && !navigationOpen && !!membersPanel, membersVisible, (open) => {
    if (!narrow) wideMembersVisible.current = open;
    setMembersVisible(open);
  });
  // Like desktop: Members sit beside the conversation while it keeps 320px,
  // otherwise they float over its right edge below the header.
  const [membersBeside, setMembersBeside] = useState<boolean>();
  const membersShown = (membersVisible || members.closing) && !!membersPanel;
  useLayoutEffect(() => {
    const room = roomRef.current;
    if (!room || !membersShown || narrow) return setMembersBeside(undefined);
    const measure = () => {
      const stage = room.querySelector(":scope > .stage");
      // From the stage's left edge, so the members column never changes the answer.
      if (stage) setMembersBeside(room.getBoundingClientRect().right - stage.getBoundingClientRect().left >= 540);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(room);
    const sidebar = room.querySelector(":scope > .people-panel");
    if (sidebar) observer.observe(sidebar);
    return () => observer.disconnect();
  }, [membersShown, narrow]);
  // Like the other overlays, Escape closes the phone member drawer and returns
  // focus to the conversation title that opens it.
  const closeMembers = useRef(members.change);
  closeMembers.current = members.change;
  useEffect(() => {
    if (!narrow || !membersVisible) return;
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented) return;
      // A profile card or dialog opened from the list closes first.
      if (event.target instanceof Element && event.target.closest('dialog, [role="dialog"]')) return;
      event.preventDefault();
      closeMembers.current(false);
      roomRef.current?.querySelector<HTMLElement>(".chat-title-menu > summary")?.focus();
    };
    document.addEventListener("keydown", escape);
    return () => document.removeEventListener("keydown", escape);
  }, [narrow, membersVisible]);
  // The drawer owns touches until it has fully slid away.
  const navigationSwipe = useBrowseSwipe(
    narrow && !membersVisible && !members.closing,
    navigationOpen,
    onNavigationChange,
  );
  const [selfPresence, setSelfPresence] = useState<PresenceStatus>();
  const [localPresence, setLocalPresence] = useState<PresenceStatus>("offline");
  const [presenceLive, setPresenceLive] = useState(false);
  const [actionError, setActionError] = useState<string>();
  const [loggingOut, setLoggingOut] = useState(false);
  const [actionPending, setActionPending] = useState(false);
  const actionGeneration = useRef(0);
  const [activeParticipants, setActiveParticipants] = useState<Set<string>>(() => new Set());
  const [participantVolumes, setParticipantVolumes] = useState<Record<string, number>>({});
  const [mutedParticipants, setMutedParticipants] = useState<Set<string>>(() => new Set());
  const [volumeParticipant, setVolumeParticipant] = useState<string>();
  const volumeMenuRef = useRef<HTMLLIElement>(null);
  const previousVoiceRoster = useRef<{ selfId: string; ids: Set<string> } | undefined>(undefined);
  // Your call card's full list of people (with per-person audio), opened by a
  // right-click or long press on its faces; a plain press opens the call's chat.
  const [everyoneOpen, setEveryoneOpen] = useState(false);
  const everyoneRef = useRef<HTMLDivElement>(null);
  const facePress = useRef<{ timer?: ReturnType<typeof setTimeout>; x: number; y: number; opened: boolean }>({
    x: 0,
    y: 0,
    opened: false,
  });
  const [publicParticipants, setPublicParticipants] = useState<Record<string, PublicPresence>>({});
  const [voiceChannel, setVoiceChannel] = useState(channel);
  const clientRef = useRef<PublicCallClient | undefined>(undefined);
  const spaceId = channel?.spaceId ?? space?.id;
  const demo = channel?.demo ?? space?.demo;
  // DMs and a space without a joined channel have no voice of their own; the
  // space's joined channels still do.
  const voiceless = !!channel?.direct || stage !== undefined;
  const mediaChannelId = voiceless ? voiceChannels?.[0]?.id : channel?.id;
  const mediaRoot =
    mediaChannelId && !demo ? `/api/channels/${encodeURIComponent(mediaChannelId)}/media` : "/api/media";
  const channelJoined = channel?.joined !== false;
  const voiceJoined = voiceless ? !!voiceChannels?.length : channelJoined;
  const available = voiceJoined ? availability[mediaRoot] : false;
  const clientRoot = useRef(mediaRoot);
  const rootFor = (channelId?: string) =>
    !channelId || demo ? "/api/media" : `/api/channels/${encodeURIComponent(channelId)}/media`;
  const createClient = (root = mediaRoot) => {
    const client = new PublicCallClient((next) => {
      if (clientRef.current === client) setState(next);
    }, root);
    clientRef.current = client;
    clientRoot.current = root;
    return client;
  };
  if (!clientRef.current && typeof window !== "undefined") createClient();
  const connected = state.phase === "connected";
  // Live transport rates re-render the call each second only while shown.
  const diagnosticsShown = audioPanel === "connection";
  useEffect(() => {
    const client = clientRef.current;
    if (!diagnosticsShown || !client) return;
    client.watchDiagnostics(true);
    return () => client.watchDiagnostics(false);
  }, [diagnosticsShown, connected]);
  const idle = state.phase === "idle" || state.phase === "failed" || state.phase === "leaving";
  const joinUnavailable = available !== true;
  const controlsDisabled = (!idle && !connected) || actionPending;
  // Keep the public roster until our own call reports its participants, so
  // joining (or failing to join) never empties the list for a moment.
  const publicRoster = idle || (state.phase === "joining" && !state.participants.length);
  const roster = publicRoster ? (publicParticipants[mediaRoot]?.participants ?? []) : state.participants;
  const identityName = account?.displayName || chatAuthor?.name || name;
  const accountPresence = !!account && !!spaceId && !demo;
  const joined = useRef(false);
  const focusedJoin = useRef<HTMLButtonElement | null>(null);
  useLayoutEffect(() => {
    if (
      connected &&
      focusedJoin.current &&
      !focusedJoin.current.isConnected &&
      document.activeElement === document.body
    ) {
      // The Join that was pressed is gone; Leave on your call card takes its place,
      // or on a phone reading the conversation (Browse hidden), the call's strip.
      const strip = phoneReading && document.querySelector<HTMLButtonElement>(".voice-strip button.voice-strip-open");
      (strip || document.querySelector<HTMLButtonElement>(".live-call .voice-hangup"))?.focus();
      focusedJoin.current = null;
    }
  }, [connected, voiceChannel?.id, phoneReading]);
  // The client reports an error only on the update where it happens, so keep
  // it until the next voice action or until it is dismissed.
  const [voiceError, setVoiceError] = useState<string>();
  useEffect(() => {
    if (state.error) setVoiceError(state.error);
  }, [state.error]);
  // Show "Connecting…" only if joining takes a moment; an immediate failure
  // (such as a denied microphone) should not flash it.
  const [joiningShown, setJoiningShown] = useState(false);
  useEffect(() => {
    if (state.phase !== "joining") {
      setJoiningShown(false);
      return;
    }
    const timer = setTimeout(() => setJoiningShown(true), 200);
    return () => clearTimeout(timer);
  }, [state.phase]);
  const pendingJoin = state.phase === "joining" && !joiningShown;
  const Root = embedded ? "div" : "main";

  useEffect(() => {
    if (engaged) void preloadSoundEffects();
  }, [engaged]);

  useEffect(() => {
    setSelfPresence(undefined);
    setPresenceLive(false);
    if (!account || !spaceId || demo) return;
    return watchAccountPresence(
      spaceId,
      [account.id],
      (members) => {
        setSelfPresence(members.find((member) => member.userId === account.id)?.status);
      },
      setPresenceLive,
    );
  }, [account?.id, spaceId, demo]);

  useEffect(() => {
    if (connected && !joined.current) {
      joined.current = true;
      playSound("channel-join");
    } else if (idle) joined.current = false;
  }, [connected, idle]);

  useEffect(() => {
    const previous = previousVoiceRoster.current;
    if (!connected || !state.selfId) {
      previousVoiceRoster.current = undefined;
      return;
    }
    const ids = new Set(state.participants.map((person) => person.id));
    const { joined, left } = rosterChanges(
      previous?.selfId === state.selfId ? previous.ids : undefined,
      ids,
      state.selfId,
    );
    if (left) playSound("channel-leave");
    else if (joined) playSound("channel-join");
    previousVoiceRoster.current = { selfId: state.selfId, ids };
  }, [connected, state.selfId, state.participants]);

  // The full list closes on Escape or an outside press, and whenever the call ends.
  useEffect(() => {
    if (!everyoneOpen) return;
    const dismiss = (event: PointerEvent) => {
      const target = event.target as Node;
      const panel = everyoneRef.current;
      // A press on the phone sheet's backdrop reaches the sheet itself, above its top edge.
      if (panel && target === panel && event.clientY < panel.getBoundingClientRect().top) {
        setEveryoneOpen(false);
        return;
      }
      if (panel?.contains(target)) return;
      setEveryoneOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || volumeParticipant) return;
      event.preventDefault();
      setEveryoneOpen(false);
      document.querySelector<HTMLButtonElement>(".live-call :is(.channel-select, .live-channel)")?.focus();
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("keydown", escape);
    };
  }, [everyoneOpen, volumeParticipant]);
  useEffect(() => {
    if (idle) setEveryoneOpen(false);
  }, [idle]);

  useEffect(() => {
    if (!volumeParticipant) return;
    const dismiss = (event: PointerEvent | FocusEvent) => {
      if (!volumeMenuRef.current?.contains(event.target as Node)) setVolumeParticipant(undefined);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      volumeMenuRef.current?.querySelector<HTMLButtonElement>(".participant-menu-button")?.focus();
      setVolumeParticipant(undefined);
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("focusin", dismiss);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("focusin", dismiss);
      document.removeEventListener("keydown", escape);
    };
  }, [volumeParticipant]);

  // Participant audio menus sit in the top layer beside their row, like the
  // footer's audio menus. Positioned inside the channel list, a menu near its
  // bottom was clipped (Mute cut off) with no way to scroll to it. A callback
  // ref, so a row that remounts (inline to pinned) places its new menu too.
  const placeVolumeMenu = useCallback((panel: HTMLDivElement | null) => {
    const anchor = panel?.parentElement;
    if (!panel || !anchor) return;
    if (typeof panel.showPopover === "function" && !panel.matches(":popover-open")) panel.showPopover();
    const position = () => {
      const viewport = window.visualViewport;
      const x = viewport?.offsetLeft ?? 0;
      const y = viewport?.offsetTop ?? 0;
      const width = viewport?.width ?? window.innerWidth;
      const height = viewport?.height ?? window.innerHeight;
      const edge = 8;
      const bounds = anchor.getBoundingClientRect();
      const box = panel.getBoundingClientRect();
      const below = y + height - bounds.bottom - edge;
      const above = bounds.top - y - edge;
      const up = box.height + 4 > below && above > below;
      panel.style.left = `${Math.max(x + edge, Math.min(bounds.right - 8 - box.width, x + width - box.width - edge))}px`;
      panel.style.top = `${Math.max(y + edge, Math.min(up ? bounds.top - 4 - box.height : bounds.bottom + 4, y + height - box.height - edge))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    observer.observe(panel);
    observer.observe(anchor);
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    window.visualViewport?.addEventListener("resize", position);
    window.visualViewport?.addEventListener("scroll", position);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
      window.visualViewport?.removeEventListener("resize", position);
      window.visualViewport?.removeEventListener("scroll", position);
    };
  }, []);

  // Your call's full list opens beside the sidebar, level with the card, in the
  // top layer so the channel list never clips it. Phones show it as a bottom
  // sheet instead (CSS), so only wide layouts are positioned here.
  const placeEveryone = useCallback((panel: HTMLDivElement | null) => {
    everyoneRef.current = panel;
    const anchor = panel?.closest<HTMLElement>(".live-call");
    if (!panel || !anchor) return;
    if (typeof panel.showPopover === "function" && !panel.matches(":popover-open")) {
      panel.showPopover();
      // A person's audio menu opened with the list was shown first; lift it back above.
      panel.querySelectorAll<HTMLElement>(".participant-volume:popover-open").forEach((menu) => {
        menu.hidePopover();
        menu.showPopover();
      });
      // The list is a dialog: focus moves into it, onto that person's volume when a face
      // chose one (a frame later: the slider's thumb is hidden on its first commit).
      requestAnimationFrame(() => {
        if (!panel.isConnected || panel.contains(document.activeElement)) return;
        (
          panel.querySelector<HTMLElement>('.participant-volume [role="slider"]') ??
          panel.querySelector<HTMLElement>(".live-everyone-heading button")
        )?.focus({ preventScroll: true });
      });
    }
    const position = () => {
      if (window.matchMedia("(max-width: 760px)").matches) {
        panel.style.left = "";
        panel.style.top = "";
        return;
      }
      const edge = 8;
      const bounds = anchor.getBoundingClientRect();
      const box = panel.getBoundingClientRect();
      const sidebar = anchor.closest(".people-panel")?.getBoundingClientRect() ?? bounds;
      panel.style.left = `${Math.max(edge, Math.min(sidebar.right + edge, window.innerWidth - box.width - edge))}px`;
      panel.style.top = `${Math.max(edge, Math.min(bounds.top, window.innerHeight - box.height - edge))}px`;
    };
    position();
    const observer = new ResizeObserver(position);
    observer.observe(panel);
    observer.observe(anchor);
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    return () => {
      observer.disconnect();
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
    };
  }, []);

  useEffect(() => {
    let current = true;
    if (!initialAccount)
      setName(uniqueNamesGenerator({ dictionaries: [colors, animals], separator: " ", style: "capital" }));
    void (initialAccount ? Promise.resolve(initialAccount) : getAccount())
      .then((account) => {
        if (!current) return;
        setAccount(account);
        if (account?.displayName) setName(account.displayName);
      })
      .catch(() => undefined)
      .finally(() => {
        if (current) setIdentityReady(true);
      });
    const unload = () => clientRef.current?.leaveImmediately();
    window.addEventListener("pagehide", unload);
    return () => {
      current = false;
      window.removeEventListener("pagehide", unload);
      clientRef.current?.leaveImmediately();
    };
  }, []);

  // Server-side account flags such as debugEnabled can change while a tab stays
  // open. Re-read them when the visitor returns or opens User Settings, rather
  // than requiring a reload or a new login.
  const signedIn = !!account;
  const settingsOpen = audioMenu === "settings";
  useEffect(() => {
    if (!signedIn) return;
    let current = true;
    const refresh = () => {
      if (document.visibilityState !== "visible") return;
      void getAccount()
        .then((next) => {
          if (current && next) setAccount(next);
        })
        .catch(() => undefined);
    };
    if (settingsOpen) refresh();
    document.addEventListener("visibilitychange", refresh);
    return () => {
      current = false;
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [signedIn, settingsOpen]);

  useEffect(() => {
    if (!voiceJoined) return;
    const controller = new AbortController();
    fetch(`${mediaRoot}/status`, {
      credentials: "same-origin",
      signal: AbortSignal.any([controller.signal, AbortSignal.timeout(10_000)]),
    })
      .then(async (response) => (response.ok ? (response.json() as Promise<{ enabled: boolean }>) : { enabled: false }))
      .then((result) => {
        if (controller.signal.aborted) return;
        setAvailability((previous) => ({ ...previous, [mediaRoot]: result.enabled }));
      })
      .catch(() => {
        if (!controller.signal.aborted) setAvailability((previous) => ({ ...previous, [mediaRoot]: false }));
      });
    return () => controller.abort();
  }, [mediaRoot, voiceJoined]);

  useEffect(() => {
    if (
      voiceChannel?.id &&
      voiceChannels &&
      !channel?.direct &&
      voiceChannel.spaceId === spaceId &&
      !voiceChannels.some((item) => item.id === voiceChannel.id)
    ) {
      // leave() also publishes the idle state; leaveImmediately() alone left the
      // dock showing "Voice connected" with a hang-up button that did nothing.
      void clientRef.current?.leave();
    }
  }, [voiceChannel?.id, voiceChannel?.spaceId, voiceChannels, spaceId, channel?.direct]);

  useEffect(() => {
    // Download/compile only; never a permission prompt. Deferred for embedded
    // rooms so homepage visitors who only look do not fetch noise models.
    if (available === true && engaged) clientRef.current?.prepareMicrophone();
  }, [available, engaged, mediaRoot]);

  useEffect(() => {
    if (!idle || available !== true) return;
    return watchPresence(
      (snapshot) => setPublicParticipants((previous) => ({ ...previous, [mediaRoot]: snapshot })),
      (online) => {
        if (!online) setPublicParticipants((previous) => ({ ...previous, [mediaRoot]: { participants: [] } }));
      },
      mediaRoot,
    );
  }, [idle, available, mediaRoot]);

  // Who is in voice in each of the space's channels, so people can see where
  // others are and join them. One spectator subscription per channel, capped
  // to stay within the gateway's per-connection subscription limit.
  const [channelRosters, setChannelRosters] = useState<Record<string, PublicPresence>>({});
  const watchedChannels = demo
    ? ""
    : (voiceChannels ?? [])
        .slice(0, MAX_WATCHED_CHANNELS)
        .map((item) => item.id)
        .join(",");
  useEffect(() => {
    if (!watchedChannels || available !== true) return;
    const stops = watchedChannels.split(",").map((id) =>
      watchPresence(
        (snapshot) => setChannelRosters((previous) => ({ ...previous, [id]: snapshot })),
        (online) => {
          if (!online) setChannelRosters((previous) => ({ ...previous, [id]: { participants: [] } }));
        },
        `/api/channels/${encodeURIComponent(id)}/media`,
      ),
    );
    return () => stops.forEach((stop) => stop());
  }, [watchedChannels, available]);

  useEffect(() => {
    if (!navigator.mediaDevices) return;
    const update = () =>
      void navigator.mediaDevices
        .enumerateDevices()
        .then((next) => {
          setDevices(next);
          const inputs = deviceOptions(next, "audioinput");
          const outputs = deviceOptions(next, "audiooutput");
          setDeviceId((current) =>
            inputs.some(({ device }) => device.deviceId === current) ? current : inputs[0]?.device.deviceId || "",
          );
          setOutput((current) =>
            outputs.some(({ device }) => device.deviceId === current) ? current : outputs[0]?.device.deviceId || "",
          );
        })
        .catch(() => setActionError("Device list unavailable. Use system settings."));
    update();
    navigator.mediaDevices.addEventListener("devicechange", update);
    return () => navigator.mediaDevices.removeEventListener("devicechange", update);
  }, [connected, state.monitorStream]);

  useEffect(() => {
    if (!audioMenu) return;
    const dismiss = (event: PointerEvent | FocusEvent) => {
      if (!audioMenuRef.current?.contains(event.target as Node)) setAudioMenu(undefined);
    };
    // A mouse-opened menu may not hold focus (Safari, Firefox on macOS), so its
    // own Escape handler never runs; close it from anywhere instead.
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape" || event.defaultPrevented || event.target instanceof HTMLSelectElement) return;
      event.preventDefault();
      setAudioMenu(undefined);
      audioMenuRef.current?.querySelector<HTMLButtonElement>(".call-settings-trigger")?.focus();
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("focusin", dismiss);
    document.addEventListener("keydown", escape);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("focusin", dismiss);
      document.removeEventListener("keydown", escape);
    };
  }, [audioMenu]);

  useEffect(() => {
    if (audioPanel) {
      // The menu item becomes hidden when the dialog takes focus. Return to
      // its visible disclosure, rather than the browser's hidden opener. Audio
      // test disables its own item as it starts, which already moved focus to
      // the body, so fall back to the menu that is still open.
      audioReturnFocus.current =
        (document.activeElement?.closest(".call-settings") ?? audioMenuRef.current)?.querySelector<HTMLElement>(
          ".call-settings-trigger",
        ) ?? null;
      audioDialog.current?.showModal();
    } else {
      audioDialog.current?.close();
      audioReturnFocus.current?.focus();
      audioReturnFocus.current = null;
    }
  }, [audioPanel]);

  const act = async (operation: () => Promise<unknown>, success?: () => void) => {
    const generation = ++actionGeneration.current;
    setActionError(undefined);
    setActionPending(true);
    try {
      await operation();
      if (generation === actionGeneration.current) success?.();
    } catch (error) {
      if (generation === actionGeneration.current)
        setActionError(error instanceof Error ? error.message : "That action did not work.");
    } finally {
      if (generation === actionGeneration.current) setActionPending(false);
    }
  };
  const leave = () => {
    setVoiceError(undefined);
    if (connected) playSound("disconnect");
    void act(() => clientRef.current!.leave());
  };
  const leaveControl = (
    <Tooltip content={connected ? "Leave this voice channel" : "Cancel joining voice"}>
      <button
        type="button"
        className="voice-hangup voice-leave"
        aria-label={connected ? "Leave voice" : "Cancel joining voice"}
        onClick={leave}
      >
        <PhoneOff aria-hidden="true" />
        <span>{connected ? "Leave" : "Cancel"}</span>
      </button>
    </Tooltip>
  );
  const toggleMute = () => {
    if (state.monitoring) return;
    const muted = !state.muted;
    playSound(muted ? "toggle-off" : "toggle-on");
    setActionError(undefined);
    void clientRef
      .current!.setMuted(muted)
      .catch((error) => setActionError(error instanceof Error ? error.message : "Mute state could not be shared."));
  };
  const toggleDeafen = () => {
    if (state.monitoring) return;
    const deafened = !state.deafened;
    playSound(deafened ? "toggle-off" : "toggle-on");
    setActionError(undefined);
    void clientRef
      .current!.setDeafened(deafened)
      .catch((error) => setActionError(error instanceof Error ? error.message : "Deafen state could not be shared."));
  };
  const closeAudioPanel = () => {
    if (audioPanel === "mic") {
      ++actionGeneration.current;
      setActionPending(false);
      setActionError(undefined);
      clientRef.current?.stopLocalMicTest();
      if (state.monitoring) void act(() => clientRef.current!.setMonitoring(false));
    }
    setAudioPanel(undefined);
  };
  const joinBlocked = !identityReady || state.phase === "leaving" || available !== true || actionPending;
  /** Join (or switch voice to) any listed channel without leaving the one being read. */
  const joinChannel = (channelId?: string) => {
    if (voiceless && (!channelId || channelId === channel?.id)) return;
    const voiceName = voiceChannels?.find((item) => item.id === channelId)?.name ?? "voice";
    const target =
      channelId === channel?.id
        ? channel
        : channel && channelId
          ? { ...channel, id: channelId, name: voiceName, direct: false }
          : space && channelId
            ? { id: channelId, name: voiceName, spaceName: space.name, spaceId: space.id, demo: space.demo }
            : channel;
    const root = channelId === channel?.id ? mediaRoot : rootFor(channelId);
    if (joinBlocked || (!idle && clientRoot.current === root)) return;
    setActionError(undefined);
    setVoiceError(undefined);
    if (clientRoot.current !== root || !idle) {
      const previous = clientRef.current!;
      previous.leaveImmediately();
      const client = createClient(root);
      client.copyAudioPreferencesFrom(previous);
    }
    setVoiceChannel(target);
    const startedAt =
      (channelId ? channelRosters[channelId]?.sessionStartedAt : undefined) ??
      publicParticipants[root]?.sessionStartedAt;
    void clientRef.current!.join(identityName.trim(), deviceId, startedAt);
  };
  /** Signed-in members: create the provider session as the pointer or focus reaches Join. */
  const prepareChannel = (channelId?: string) => {
    const root = channelId === channel?.id ? mediaRoot : rootFor(channelId);
    if (!signedIn || demo || joinBlocked || (!idle && clientRoot.current === root)) return;
    if (hasWarmVoice()) return; // Join will use the connected pair instead.
    prepareVoiceJoin(root);
  };
  // Signed-in members keep a connected session pair ready while voice is idle,
  // including in the live demo. Anonymous visitors do not provision sessions.
  const keepWarm = signedIn && identityReady && available === true && idle;
  useEffect(() => {
    if (!keepWarm) {
      stopVoiceWarm();
      return;
    }
    keepVoiceWarm(mediaRoot);
  }, [keepWarm, mediaRoot]);
  useEffect(() => () => stopVoiceWarm(), []);
  const prepareRef = useRef(prepareChannel);
  prepareRef.current = prepareChannel;
  // Start preparing as the pointer approaches a Join button, not only on hover.
  // One listener, at most one geometry check per frame, and only while signed in.
  useEffect(() => {
    if (!signedIn || demo) return;
    let frame = 0,
      x = 0,
      y = 0;
    const check = () => {
      frame = 0;
      for (const button of document.querySelectorAll<HTMLElement>(
        '.channel-join[data-channel][aria-disabled="false"]:not([data-connected])',
      )) {
        const box = button.getBoundingClientRect();
        if (!box.width || !box.height) continue;
        const dx = Math.max(box.left - x, 0, x - box.right);
        const dy = Math.max(box.top - y, 0, y - box.bottom);
        if (dx * dx + dy * dy <= JOIN_PREPARE_RADIUS * JOIN_PREPARE_RADIUS)
          prepareRef.current(button.dataset.channel || undefined);
      }
    };
    const move = (event: PointerEvent) => {
      if (event.pointerType === "touch") return; // Touch has no approach; pointerdown covers it.
      x = event.clientX;
      y = event.clientY;
      frame ||= requestAnimationFrame(check);
    };
    document.addEventListener("pointermove", move, { passive: true });
    return () => {
      document.removeEventListener("pointermove", move);
      cancelAnimationFrame(frame);
    };
  }, [signedIn, demo]);
  // Touch has no approach, and pointerdown lands too close to the tap to help. On a
  // touch screen, prepare the viewed channel once when its Join button is shown;
  // Cloudflare keeps an unused session for 10-15 s, so this covers a prompt tap.
  const viewedJoin = useCallback(
    (button: HTMLButtonElement | null) => {
      if (
        !button ||
        typeof IntersectionObserver === "undefined" ||
        typeof matchMedia === "undefined" ||
        !matchMedia("(pointer: coarse)").matches
      )
        return;
      const observer = new IntersectionObserver((entries) => {
        if (!entries.some((entry) => entry.isIntersecting)) return;
        observer.disconnect();
        prepareRef.current(button.dataset.channel || undefined);
      });
      observer.observe(button);
      return () => observer.disconnect();
      // A new identity re-attaches (and observes again) when the viewed channel
      // changes with React reusing the button, or when Join becomes possible after
      // the button was already shown: preparation is skipped while it is blocked.
    },
    [channel?.id, joinBlocked],
  );
  const openMicTest = () => {
    setVoiceError(undefined);
    setAudioPanel("mic");
    void act(() =>
      connected ? clientRef.current!.setMonitoring(true) : clientRef.current!.startLocalMicTest(deviceId),
    );
  };

  useEffect(() => {
    if (profileOpen) profileDialog.current?.showModal();
    else profileDialog.current?.close();
  }, [profileOpen]);

  // Your own voice tile shows the picture you just saved, like your name.
  const voicePicture = <T extends Pick<Participant, "id" | "avatarId" | "avatarAsset" | "avatarUrl">>(
    participant: T,
  ) => (participant.id === state.selfId && account ? withPicture(participant, account) : participant);
  const isSpeaking = (participant: { id: string; muted: boolean }) => {
    const muted = participant.id === state.selfId ? state.muted : participant.muted;
    return activeParticipants.has(participant.id) && !muted;
  };
  const renderRoster = (people: typeof roster, own: boolean, label: string) => (
    <ul
      className={own && volumeParticipant ? "volume-menu-open" : undefined}
      aria-label={`People in voice in ${label}`}
    >
      {people.map((participant) => {
        const self = participant.id === state.selfId;
        const participantName = self ? (account?.displayName ?? participant.name) : participant.name;
        const participantMuted = self ? state.muted : participant.muted;
        const participantDeafened = self ? state.deafened : participant.deafened;
        const participantStatus = participantDeafened ? "Deafened" : participantMuted ? "Muted" : undefined;
        const speaking = isSpeaking(participant);
        return (
          <li
            ref={own && volumeParticipant === participant.id ? volumeMenuRef : undefined}
            className={`participant ${volumeParticipant === participant.id ? "volume-open" : ""}`}
            key={participant.id}
            onContextMenu={
              !own || self
                ? undefined
                : (event) => {
                    event.preventDefault();
                    setVolumeParticipant(participant.id);
                  }
            }
          >
            <span className="participant-avatar">
              <span className={`avatar ${speaking ? "speaking" : "quiet"}`}>
                <Avatar person={voicePicture(participant)} name={participantName} />
              </span>
            </span>
            <span className="participant-name">
              <strong>
                {participantName}
                {self ? " (you)" : ""}
              </strong>
              {participantStatus && (
                <span className="participant-status" title={participantStatus}>
                  {participantMuted && <MicOff aria-hidden="true" />}
                  {participantDeafened && <HeadphoneOff aria-hidden="true" />}
                  <span className="sr-only">{participantStatus}</span>
                </span>
              )}
              {!self && mutedParticipants.has(participant.id) && (
                <span className="participant-local-muted">
                  <VolumeX aria-hidden="true" />
                  You muted {participant.name}
                </span>
              )}
            </span>
            {own && !self && (
              <button
                className="participant-menu-button"
                type="button"
                aria-label={`Audio controls for ${participant.name}`}
                aria-expanded={volumeParticipant === participant.id}
                onClick={() =>
                  setVolumeParticipant((current) => (current === participant.id ? undefined : participant.id))
                }
              >
                Audio
              </button>
            )}
            {own && volumeParticipant === participant.id && (
              <div
                ref={placeVolumeMenu}
                className="participant-volume"
                popover="manual"
                role="group"
                aria-label={`${participant.name} local audio settings`}
              >
                <div>
                  <strong>User volume</strong>
                  <output>{participantVolumes[participant.id] ?? 100}%</output>
                </div>
                <Slider
                  label={`${participant.name} volume`}
                  value={participantVolumes[participant.id] ?? 100}
                  max={200}
                  onChange={(value) => setParticipantVolumes((current) => ({ ...current, [participant.id]: value }))}
                />
                <label className="participant-mute">
                  <span>Mute</span>
                  <input
                    type="checkbox"
                    checked={mutedParticipants.has(participant.id)}
                    onChange={(event) => {
                      playSound(event.target.checked ? "toggle-off" : "toggle-on");
                      setMutedParticipants((current) => {
                        const next = new Set(current);
                        event.target.checked ? next.add(participant.id) : next.delete(participant.id);
                        return next;
                      });
                    }}
                  />
                </label>
                <small>Only changes what you hear.</small>
              </div>
            )}
          </li>
        );
      })}
    </ul>
  );
  /** Who is in voice in a channel; your own call's roster (with controls) when you are in it. */
  const rosterFor = (channelId?: string) => {
    const watched = channelId ? channelRosters[channelId] : undefined;
    const snapshot = watched ?? (channelId === channel?.id ? publicParticipants[mediaRoot] : undefined);
    if (!idle && voiceChannel?.id === channelId)
      return {
        people: publicRoster ? (snapshot?.participants ?? []) : state.participants,
        startedAt: publicRoster ? (snapshot?.sessionStartedAt ?? state.sessionStartedAt) : state.sessionStartedAt,
        own: !publicRoster,
      };
    return { people: snapshot?.participants ?? [], startedAt: snapshot?.sessionStartedAt, own: false };
  };
  const channelLabel = (channelId?: string) =>
    channelId === channel?.id || !channelId
      ? (channel?.name ?? "general")
      : (voiceChannels?.find((item) => item.id === channelId)?.name ?? "voice");
  // The call's channel and space as they are named now: renames here, or live
  // from elsewhere, reach your call card as they reach the channel list.
  const voiceHere = !!voiceChannel && voiceChannel.spaceId === spaceId;
  const voiceChannelName =
    (voiceHere && voiceChannels?.find((item) => item.id === voiceChannel.id)?.name) || voiceChannel?.name;
  const voiceSpaceName = (voiceHere && space?.name) || voiceChannel?.spaceName;
  // Your call card appears once joining takes a moment, like the old dock did.
  const inCall = !idle && !pendingJoin;
  const personName = (person: { id: string; name: string }) =>
    person.id === state.selfId ? (account?.displayName ?? person.name) : person.name;
  // Whether the channel list has a row for a channel: its voice shows there, not above.
  const listed = (channelId?: string) =>
    typeof channelNavigation === "function"
      ? !!channelId && !!voiceChannels?.some((item) => item.id === channelId)
      : channelId === channel?.id;
  const voiceFor = (channelId?: string): VoiceSlot | null => {
    if ((voiceless && (!channelId || channelId === channel?.id)) || (channelId === channel?.id && !channelJoined))
      return null;
    // Your call's row becomes its card. Where a join would be, it shows the call's status
    // and time instead, so the row never shifts.
    if (inCall && voiceChannel?.id === channelId)
      return { live: null, join: callStatus(), card: "call", frame: callCard };
    const { people, startedAt } = rosterFor(channelId);
    const activeHere = !idle && voiceChannel?.id === channelId;
    const item = voiceChannels?.find((entry) => entry.id === channelId) ?? {
      id: channelId,
      name: channelLabel(channelId),
    };
    return {
      live:
        people.length > 0 ? (
          <span
            className="channel-live"
            data-active={activeHere ? "" : undefined}
            role="img"
            aria-label={`${people.length} in voice`}
          >
            <AudioLines aria-hidden="true" />
            <span aria-hidden="true">{people.length}</span>
          </span>
        ) : null,
      // Your call's channel keeps the button's slot, so connecting never shifts its row.
      join: activeHere ? (
        <span className="channel-quick-join" data-placeholder="" aria-hidden="true" />
      ) : (
        voiceAction(channelId, "row", people.length > 0)
      ),
      ...(people.length > 0
        ? { card: "room" as const, frame: (line: ReactNode) => roomCard(item, people, startedAt, line) }
        : { frame: (line: ReactNode) => <div className="channel-card">{line}</div> }),
    };
  };
  /**
   * Join or switch voice for a channel in one click. Rows carry an icon button
   * (revealed on hover for fine pointers, always shown on quiet rows for touch);
   * live room cards and the phone strip carry a labelled one. Leaving lives on your
   * call card.
   */
  const voiceAction = (channelId: string | undefined, placement: "row" | "card" | "strip", occupied = false) => {
    const label = channelLabel(channelId);
    const activeHere = !idle && voiceChannel?.id === channelId;
    const joiningHere = activeHere && state.phase === "joining";
    const leavingHere = state.phase === "leaving" && voiceChannel?.id === channelId;
    const switching = !idle && !activeHere;
    if (activeHere && (connected || state.phase === "reconnecting")) return null;
    const blocked = joiningHere || leavingHere || actionPending || (!activeHere && joinBlocked);
    const actionLabel = leavingHere ? "Leaving…" : joiningHere ? "Joining…" : switching ? "Switch" : "Join";
    const actionName = leavingHere
      ? `Leaving voice in #${label}`
      : joiningHere
        ? `Joining voice in #${label}`
        : switching
          ? `Switch voice to #${label}`
          : `Join voice in #${label}`;
    const row = placement === "row";
    return (
      <Tooltip
        content={
          joiningHere
            ? `Connecting to #${label}…`
            : joinUnavailable
              ? available === false
                ? "Joining is not available at this time."
                : "Checking voice availability…"
              : switching
                ? `Leave your current voice channel and join #${label}`
                : `Join voice in #${label}`
        }
        placement="top"
      >
        <button
          ref={placement === "strip" && !activeHere ? viewedJoin : undefined}
          className={`channel-join ${row ? "channel-quick-join" : `live-join${switching ? " live-switch" : ""}`}`}
          data-occupied={row && occupied ? "" : undefined}
          type="button"
          data-channel={channelId ?? ""}
          data-connected={activeHere ? "" : undefined}
          aria-label={actionName}
          aria-disabled={blocked}
          aria-busy={joiningHere || leavingHere}
          onPointerEnter={() => {
            if (!activeHere) prepareChannel(channelId);
          }}
          onPointerDown={() => {
            if (!activeHere) prepareChannel(channelId);
          }}
          onFocus={(event) => {
            focusedJoin.current = event.currentTarget;
            if (!activeHere) prepareChannel(channelId);
          }}
          onClick={() => {
            if (!blocked) joinChannel(channelId);
          }}
        >
          {row ? <Headphones aria-hidden="true" /> : <span className="channel-join-label">{actionLabel}</span>}
        </button>
      </Tooltip>
    );
  };
  const muteToggle = (
    <Tooltip content={state.monitoring ? "Stop mic test to change mute" : state.muted ? "Unmute" : "Mute"}>
      <button
        type="button"
        className={`voice-icon-button live-toggle${state.muted ? " active" : ""}`}
        aria-disabled={state.monitoring}
        aria-label={state.muted ? "Unmute microphone" : "Mute microphone"}
        aria-pressed={state.muted}
        onClick={toggleMute}
      >
        {state.muted ? <MicOff aria-hidden="true" /> : <Mic aria-hidden="true" />}
      </button>
    </Tooltip>
  );
  const deafenToggle = (
    <Tooltip content={state.monitoring ? "Stop mic test to change deafen" : state.deafened ? "Undeafen" : "Deafen"}>
      <button
        type="button"
        className={`voice-icon-button live-toggle${state.deafened ? " active" : ""}`}
        aria-disabled={state.monitoring}
        aria-label={state.deafened ? "Undeafen audio" : "Deafen audio"}
        aria-pressed={state.deafened}
        onClick={toggleDeafen}
      >
        {state.deafened ? <VolumeX aria-hidden="true" /> : <Headphones aria-hidden="true" />}
      </button>
    </Tooltip>
  );
  const liveFace = (participant: (typeof roster)[number]) => (
    <span className="live-avatar" key={participant.id} data-speaking={isSpeaking(participant) ? "" : undefined}>
      <Avatar person={voicePicture(participant)} name={personName(participant)} />
    </span>
  );
  // Your call's faces keep a stable order: you, then everyone else as they joined.
  const callPeople = () => {
    const { people, ...rest } = rosterFor(voiceChannel?.id);
    const self = people.filter((person) => person.id === state.selfId);
    return { people: [...self, ...people.filter((person) => person.id !== state.selfId)], ...rest };
  };
  const callStatusText = (count: number) =>
    connected
      ? `You’re in · ${count}`
      : state.phase === "joining"
        ? "Connecting…"
        : state.phase === "reconnecting"
          ? "Reconnecting…"
          : "Leaving…";
  /** Your call's status and running time, on its row or its card's heading. */
  const callStatus = () => {
    const { people, startedAt } = callPeople();
    return (
      <span className="live-call-status">
        <span className="live-status">{callStatusText(people.length)}</span>
        {startedAt != null && <VoiceSessionTimer startedAt={startedAt} />}
      </span>
    );
  };
  // Presses that open a card's chat skip its controls, menus and popovers.
  const cardControl = (target: EventTarget) =>
    !!(target as Element).closest("button, a, input, label, details, [popover]");
  // The face (or "+N", "") a press or right-click landed on.
  const pressedFace = (target: EventTarget) =>
    (target as Element).closest<HTMLElement>("[data-person]")?.dataset.person;
  /** A long press on a face opens everyone's volume on touch, where there is no right-click. */
  const longPress = (open: (personId: string) => void) => {
    const press = facePress.current;
    const cancel = () => clearTimeout(press.timer);
    return {
      onPointerDown: (event: ReactPointerEvent) => {
        cancel();
        press.opened = false;
        const person = pressedFace(event.target);
        if (event.pointerType === "mouse" || person === undefined) return;
        press.x = event.clientX;
        press.y = event.clientY;
        press.timer = setTimeout(() => {
          press.opened = true;
          open(person);
        }, LONG_PRESS_MS);
      },
      onPointerMove: (event: ReactPointerEvent) => {
        if (Math.hypot(event.clientX - press.x, event.clientY - press.y) > 10) cancel();
      },
      onPointerUp: cancel,
      onPointerCancel: cancel,
    };
  };
  /**
   * Your call: who is in it, with mute, deafen and Leave. Pressing it opens the call's
   * chat; a right-click or long press on its faces opens everyone's volume. In this
   * space's list it wraps the channel's own row line; elsewhere it has its own heading.
   */
  const callCard = (line?: ReactNode) => {
    const { people, startedAt, own } = callPeople();
    const name = voiceChannelName ?? "general";
    const { shown, hidden } = callCardFaces(people);
    const elsewhere = !!voiceChannel?.spaceId && !voiceHere;
    const openChat = () => {
      if (voiceChannel) onOpenChannel?.(voiceChannel.spaceId, voiceChannel.id);
    };
    const openEveryone = (personId?: string) => {
      if (!own) return;
      setEveryoneOpen(true);
      setVolumeParticipant(personId && personId !== state.selfId ? personId : undefined);
    };
    return (
      <div
        role="group"
        className="live-card live-call"
        data-phase={state.phase}
        aria-label={`Your call in #${name}`}
        onClick={(event) => {
          if (cardControl(event.target)) return;
          // The press that ended a long press on a face opened everyone's volume instead.
          if (facePress.current.opened && pressedFace(event.target) !== undefined) facePress.current.opened = false;
          else openChat();
        }}
        // A right-click, or the context menu key on the focused row, lists everyone.
        onContextMenu={(event) => {
          if (!own || (event.target as Element).closest("details, [popover], input")) return;
          event.preventDefault();
          openEveryone(pressedFace(event.target) || undefined);
        }}
      >
        {line ?? (
          <div className="live-card-heading">
            <AudioLines className="live-wave" aria-hidden="true" />
            <button
              type="button"
              className="live-channel"
              disabled={!onOpenChannel || !voiceChannel}
              onClick={openChat}
            >
              <span>{name}</span>
              {elsewhere && voiceSpaceName && <small>{voiceSpaceName}</small>}
            </button>
            <span className="live-status">{callStatusText(people.length)}</span>
            {startedAt != null && <VoiceSessionTimer startedAt={startedAt} />}
          </div>
        )}
        {people.length > 0 && (
          <ul
            className="live-faces"
            aria-label={`${people.length} in this call`}
            {...longPress((personId) => openEveryone(personId || undefined))}
          >
            {shown.map((participant) => {
              const self = participant.id === state.selfId;
              const muted = self ? state.muted : participant.muted;
              const deafened = self ? state.deafened : participant.deafened;
              const label = [
                `${personName(participant)}${self ? " (you)" : ""}`,
                isSpeaking(participant) && "speaking",
                deafened ? "deafened" : muted && "muted",
              ]
                .filter(Boolean)
                .join(", ");
              return (
                <li key={participant.id} data-person={participant.id}>
                  <Tooltip content={personName(participant)}>
                    <span className="live-face" role="img" aria-label={label}>
                      {liveFace(participant)}
                      {(muted || deafened) && (
                        <span className="live-badge">
                          {deafened ? <HeadphoneOff aria-hidden="true" /> : <MicOff aria-hidden="true" />}
                        </span>
                      )}
                    </span>
                  </Tooltip>
                </li>
              );
            })}
            {hidden.length > 0 && (
              <li data-person="">
                <span
                  className="live-more"
                  role="img"
                  data-speaking={hidden.some(isSpeaking) ? "" : undefined}
                  aria-label={`${hidden.length} more: ${liveNames(hidden.map(personName))}`}
                >
                  +{hidden.length}
                </span>
              </li>
            )}
          </ul>
        )}
        <div className="live-controls">
          {muteToggle}
          {deafenToggle}
          {leaveControl}
        </div>
        {everyoneOpen && own && (
          <div
            ref={placeEveryone}
            className="live-everyone"
            popover="manual"
            role="dialog"
            aria-label={`Everyone in voice in #${name}`}
          >
            <div className="live-everyone-heading">
              <p>
                <strong>In #{name}</strong>
                <span>
                  {people.length} {people.length === 1 ? "person" : "people"}
                </span>
              </p>
              <button
                type="button"
                className="voice-icon-button"
                aria-label="Close"
                onClick={() => {
                  setEveryoneOpen(false);
                  setVolumeParticipant(undefined);
                  document.querySelector<HTMLButtonElement>(".live-call :is(.channel-select, .live-channel)")?.focus();
                }}
              >
                <X aria-hidden="true" />
              </button>
            </div>
            {renderRoster(people, own, `#${name}`)}
          </div>
        )}
      </div>
    );
  };
  /**
   * Another room where people are talking: who is there, for how long, and Join (or
   * Switch). Pressing it opens the room's chat. In this space's list it wraps the
   * channel's own row line.
   */
  const roomCard = (
    item: { id?: string; name: string; private?: boolean },
    people: typeof roster,
    startedAt?: number | null,
    line?: ReactNode,
  ) => (
    <div
      role="group"
      className="live-card live-room"
      aria-label={`Voice in #${item.name}, ${people.length} in voice`}
      onClick={(event) => {
        // Its chat may already be open (a visit has no place in this space to open it from).
        if (!cardControl(event.target) && item.id && item.id !== channel?.id) onOpenChannel?.(spaceId, item.id);
      }}
    >
      {line ?? (
        <div className="live-card-heading">
          {item.private ? <LockKeyhole aria-hidden="true" /> : <Hash aria-hidden="true" />}
          <button
            type="button"
            className="live-channel"
            disabled={!onOpenChannel}
            onClick={() => item.id && item.id !== channel?.id && onOpenChannel?.(spaceId, item.id)}
          >
            <span>{item.name}</span>
          </button>
        </div>
      )}
      <div className="live-room-body">
        <span className="live-stack" aria-hidden="true">
          {people.slice(0, 3).map(liveFace)}
          {people.length > 3 && <span className="live-chip">+{people.length - 3}</span>}
        </span>
        <span className="live-names">{liveNames(people.map((person) => person.name))}</span>
        {startedAt != null && <VoiceSessionTimer startedAt={startedAt} />}
        {voiceAction(item.id, "card")}
      </div>
    </div>
  );
  // Voice without a row in this list sits above the channels: your call in another
  // space (or a visited channel), and the room of a channel you are visiting
  // (feedback), which has no row here.
  const visitedRooms =
    channel?.visitor && !channel.direct && !voiceChannels?.some((item) => item.id === channel.id)
      ? [{ item: { id: channel.id, name: channel.name }, ...rosterFor(channel.id) }].filter(
          (room) => room.people.length > 0 && !(inCall && voiceChannel?.id === room.item.id),
        )
      : [];
  const callAbove = inCall && !listed(voiceChannel?.id);
  const liveNow = (callAbove || visitedRooms.length > 0) && (
    <section className="live-now" aria-label="Live now">
      {callAbove && callCard()}
      {visitedRooms.map(({ item, people, startedAt }) => (
        <Fragment key={item.id}>{roomCard(item, people, startedAt)}</Fragment>
      ))}
    </section>
  );
  /**
   * Phones show the conversation without the sidebar, so a slim strip under its
   * header carries voice: your call (tap to open Browse, where its card is, or mute
   * in place), or the open channel's live room with Join.
   */
  const voiceStrip = (() => {
    if (!narrow || !onNavigationToggle) return null;
    if (inCall) {
      const { people, startedAt } = callPeople();
      const name = voiceChannelName ?? "general";
      // The card's faces in the card's order, folding sooner: the strip has one line.
      const { shown, hidden } = callCardFaces(people, STRIP_FACES);
      return (
        // Keyed apart from the Join strip, so its Join never becomes this Mute (keeping focus).
        <div key="call" className="voice-strip" data-phase={state.phase}>
          <button
            type="button"
            className="voice-strip-open"
            aria-label={`Your call in #${name}, ${people.length} ${people.length === 1 ? "person" : "people"}. Open Browse.`}
            onClick={onNavigationToggle}
          >
            <AudioLines className="live-wave" aria-hidden="true" />
            <strong>#{name}</strong>
            <span className="live-stack" aria-hidden="true">
              {shown.map(liveFace)}
              {hidden.length > 0 && <span className="live-chip">+{hidden.length}</span>}
            </span>
            <span className="voice-strip-detail">
              {connected ? "" : state.phase === "joining" ? "Connecting…" : "Reconnecting…"}
            </span>
            {startedAt != null && <VoiceSessionTimer startedAt={startedAt} />}
          </button>
          {muteToggle}
        </div>
      );
    }
    if (voiceless || !channelJoined) return null;
    const { people } = rosterFor(channel?.id);
    if (!people.length) return null;
    return (
      <div key="room" className="voice-strip">
        <p className="voice-strip-open">
          <AudioLines className="live-wave" aria-hidden="true" />
          <strong>{people.length} talking</strong>
          <span className="live-stack" aria-hidden="true">
            {people.slice(0, 3).map(liveFace)}
          </span>
          <span className="voice-strip-detail">{liveNames(people.map((person) => person.name))}</span>
        </p>
        {voiceAction(channel?.id, "strip")}
      </div>
    );
  })();
  // Speaking is detected from your call's audio, for its card, list and strip.
  const voiceActivity =
    !publicRoster &&
    state.participants.map((participant) => {
      const self = participant.id === state.selfId;
      return (
        <VoiceActivity
          key={participant.id}
          stream={
            self ? state.localMedia : state.remoteMedia.find((media) => media.participantId === participant.id)?.stream
          }
          muted={self ? state.muted : participant.muted}
          onActivityChange={(active) =>
            setActiveParticipants((current) => {
              if (current.has(participant.id) === active) return current;
              const next = new Set(current);
              active ? next.add(participant.id) : next.delete(participant.id);
              return next;
            })
          }
        />
      );
    });
  const memberCount = membersPanel && channelJoined ? mentionMembers?.length : undefined;
  const inVoiceHere = voiceless || !channelJoined ? 0 : rosterFor(channel?.id).people.length;
  const headerSubtitle =
    [
      memberCount != null && `${memberCount} ${memberCount === 1 ? "member" : "members"}`,
      inVoiceHere > 0 && `${inVoiceHere} in voice`,
    ]
      .filter(Boolean)
      .join(" · ") || undefined;
  let navigation: ReactNode;
  if (typeof channelNavigation === "function") navigation = channelNavigation(voiceFor, liveNow);
  else if (channelNavigation) navigation = channelNavigation;
  else {
    const voice = voiceFor(undefined);
    const line = (
      <div className="channel-row">
        <a className="channel-link" href="#chat-heading" aria-current="location">
          <Hash aria-hidden="true" />
          <span>general</span>
          {voice?.live}
        </a>
        {voice?.join}
      </div>
    );
    navigation = (
      <>
        {liveNow}
        <div className="channel-item" data-live={voice?.card ? "" : undefined}>
          {voice ? voice.frame(line) : line}
        </div>
      </>
    );
  }

  return (
    <Root className={`call-page${embedded ? " call-embedded" : ""}`}>
      {!embedded && (
        <header className="call-header">
          <Wordmark />
        </header>
      )}
      <section
        ref={roomRef}
        className={`call-room${channel || space ? " spaces-room" : ""}${navigationOpen ? " navigation-open" : ""}${
          membersBeside === undefined ? "" : membersBeside ? " members-beside" : " members-float"
        }`}
        data-direct={channel?.direct ? "" : undefined}
        {...navigationSwipe}
        onPointerDownCapture={(event) => {
          navigationSwipe.onPointerDownCapture(event);
          members.swipe.onPointerDownCapture(event);
        }}
        onClickCapture={(event) => {
          navigationSwipe.onClickCapture(event);
          members.swipe.onClickCapture(event);
        }}
      >
        {spaceRail}
        <ChannelSidebar>
          <div className="sidebar-channels">{navigation}</div>
          <div className="voice-panel">
            {/* Your call itself lives on its card, in place of its channel row. */}
            {voiceError && !audioPanel && !inlineVoiceError && (
              <div className="voice-dock">
                <p className="voice-error" role="alert">
                  <span>{voiceError}</span>
                  <button type="button" aria-label="Dismiss voice error" onClick={() => setVoiceError(undefined)}>
                    <X aria-hidden="true" />
                  </button>
                </p>
              </div>
            )}
            <div className="call-account">
              <button
                className="account-profile"
                type="button"
                disabled={!identityReady}
                aria-label={account ? `Edit profile for ${identityName}` : "Sign in to edit your profile"}
                onClick={() => {
                  if (account) setProfileOpen(true);
                  else window.location.assign("/login");
                }}
              >
                <span className="account-avatar">
                  <Avatar person={account} name={identityName} />
                  <PresenceDot
                    status={accountPresence ? selfPresence : localPresence}
                    live={accountPresence ? presenceLive : true}
                  />
                </span>
                <strong className="account-name" title={identityName}>
                  {identityName || "Loading…"}
                </strong>
              </button>
              <div className={`voice-action-group${state.muted ? " active" : ""}`}>
                <Tooltip content={state.monitoring ? "Stop mic test to change mute" : state.muted ? "Unmute" : "Mute"}>
                  <button
                    type="button"
                    className={`voice-icon-button ${state.muted ? "active" : ""}`}
                    aria-disabled={state.monitoring}
                    aria-label={state.muted ? "Unmute microphone" : "Mute microphone"}
                    aria-pressed={state.muted}
                    onClick={toggleMute}
                  >
                    {state.muted ? <MicOff aria-hidden="true" /> : <Mic aria-hidden="true" />}
                  </button>
                </Tooltip>
                <AudioMenu
                  label="Input Options"
                  open={audioMenu === "input"}
                  onOpenChange={(open) => setAudioMenu(open ? "input" : undefined)}
                  menuRef={audioMenu === "input" ? audioMenuRef : undefined}
                >
                  <label className="device-select">
                    Microphone
                    <select
                      name="input-device"
                      value={deviceId}
                      disabled={controlsDisabled}
                      onChange={(event) => {
                        const id = event.target.value;
                        if (connected)
                          void act(
                            () => clientRef.current!.changeMicrophone(id),
                            () => setDeviceId(id),
                          );
                        else setDeviceId(id);
                      }}
                    >
                      {deviceOptions(devices, "audioinput").map(({ device, label }) => (
                        <option key={device.deviceId} value={device.deviceId}>
                          {label}
                        </option>
                      ))}
                      {!deviceOptions(devices, "audioinput").length && <option value="">System default</option>}
                    </select>
                  </label>
                  <div className="volume-control output-volume">
                    <span>
                      Input volume <output>{state.inputVolume}%</output>
                    </span>
                    <Slider
                      label="Input volume"
                      value={state.inputVolume}
                      max={200}
                      onChange={(value) => clientRef.current?.setInputVolume(value)}
                    />
                  </div>
                </AudioMenu>
              </div>
              <div className={`voice-action-group${state.deafened ? " active" : ""}`}>
                <Tooltip
                  content={state.monitoring ? "Stop mic test to change deafen" : state.deafened ? "Undeafen" : "Deafen"}
                >
                  <button
                    type="button"
                    className={`voice-icon-button ${state.deafened ? "active" : ""}`}
                    aria-disabled={state.monitoring}
                    aria-label={state.deafened ? "Undeafen audio" : "Deafen audio"}
                    aria-pressed={state.deafened}
                    onClick={toggleDeafen}
                  >
                    {state.deafened ? <VolumeX aria-hidden="true" /> : <Headphones aria-hidden="true" />}
                  </button>
                </Tooltip>
                <AudioMenu
                  label="Output Options"
                  open={audioMenu === "output"}
                  onOpenChange={(open) => setAudioMenu(open ? "output" : undefined)}
                  menuRef={audioMenu === "output" ? audioMenuRef : undefined}
                >
                  {outputSelectable ? (
                    <label className="device-select">
                      Audio output
                      <select name="output-device" value={output} onChange={(event) => setOutput(event.target.value)}>
                        {deviceOptions(devices, "audiooutput").map(({ device, label }) => (
                          <option key={device.deviceId} value={device.deviceId}>
                            {label}
                          </option>
                        ))}
                        {!deviceOptions(devices, "audiooutput").length && <option value="">System default</option>}
                      </select>
                    </label>
                  ) : (
                    <p className="noise-status">Choose audio output in system settings.</p>
                  )}
                  <div className="volume-control output-volume">
                    <span>
                      Output volume <output>{outputVolume}%</output>
                    </span>
                    <Slider label="Output volume" value={outputVolume} max={200} onChange={setOutputVolume} />
                  </div>
                </AudioMenu>
              </div>
              <AudioMenu
                label="User Settings"
                settings
                open={audioMenu === "settings"}
                onOpenChange={(open) => setAudioMenu(open ? "settings" : undefined)}
                menuRef={audioMenu === "settings" ? audioMenuRef : undefined}
              >
                {!embedded && signedIn && (
                  <button
                    type="button"
                    onClick={() => {
                      setAudioMenu(undefined);
                      setWhatsNewOpen(true);
                    }}
                  >
                    What’s new
                  </button>
                )}
                <strong>Audio settings</strong>
                <fieldset className="device-options">
                  <label>
                    <input
                      type="checkbox"
                      role="switch"
                      checked={systemSounds}
                      onChange={(event) => {
                        setSystemSoundsEnabled(event.target.checked);
                        if (event.target.checked) {
                          void preloadSoundEffects();
                          playSound("toggle-on");
                        }
                      }}
                    />
                    <span>Caper sound effects</span>
                  </label>
                </fieldset>
                <button
                  disabled={!identityReady || controlsDisabled || state.phase === "leaving"}
                  type="button"
                  onClick={openMicTest}
                >
                  Audio test
                </button>
                {state.diagnostics && (
                  <button type="button" onClick={() => setAudioPanel("connection")}>
                    Connection details
                  </button>
                )}
                {account?.debugEnabled && (
                  <button type="button" onClick={() => setAudioPanel("debug")}>
                    Audio diagnostics
                  </button>
                )}
                {identityReady &&
                  (account ? (
                    <button
                      type="button"
                      disabled={loggingOut}
                      onClick={() => {
                        setLoggingOut(true);
                        setActionError(undefined);
                        logout().then(
                          () => window.location.assign("/"),
                          () => {
                            // Stay signed in and say so, instead of a click that silently did nothing.
                            setLoggingOut(false);
                            setActionError("Couldn’t log out. Check your connection and try again.");
                          },
                        );
                      }}
                    >
                      {loggingOut ? "Logging out…" : "Log out"}
                    </button>
                  ) : (
                    <a href="/login">Sign in</a>
                  ))}
              </AudioMenu>
            </div>
          </div>
        </ChannelSidebar>
        <div className="stage" inert={browsing}>
          {voiceActivity}
          {state.remoteMedia.map((media) => (
            <AudioOutput
              key={media.trackId}
              stream={media.stream}
              muted={state.deafened || mutedParticipants.has(media.participantId)}
              output={output}
              volume={(outputVolume * (participantVolumes[media.participantId] ?? 100)) / 100}
              name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"}
            />
          ))}
          {stage !== undefined ? (
            stage
          ) : (
            <Chat
              key={`${channel?.id ?? "general"}:${channelJoined}`}
              name={name}
              signedIn={!!account}
              accountId={account?.id}
              identityReady={identityReady && engaged}
              direct={channel?.direct}
              directPeerId={channel?.directPeerId}
              team={!!channel?.team}
              onReadCursor={onReadCursor}
              readActive={engaged && !browsing}
              readOnly={!channelJoined}
              composerNotice={channelActions}
              composerBanner={
                // Phones show the conversation alone, so voice errors come with it.
                inlineVoiceError && voiceError && !audioPanel ? (
                  <>
                    {composerBanner}
                    <p className="voice-error voice-error-inline" role="alert">
                      <span>{voiceError}</span>
                      <button type="button" aria-label="Dismiss voice error" onClick={() => setVoiceError(undefined)}>
                        <X aria-hidden="true" />
                      </button>
                    </p>
                  </>
                ) : (
                  composerBanner
                )
              }
              onBlockAuthor={onBlockAuthor}
              messageSounds={engaged && channelJoined}
              onOnlineChange={onChatOnlineChange}
              mentionMembers={mentionMembers}
              mentionDirectory={mentionDirectory}
              onMessagePerson={onMessagePerson}
              profileSpaces={profileSpaces}
              onOpenForward={onOpenForward}
              focusMessage={focusMessage}
              focusThread={focusThread}
              onSent={onSent}
              visitor={!!channel?.visitor}
              channelId={channel?.id}
              channelName={channel?.name}
              initialHistory={initialHistory}
              initialHistoryError={initialHistoryError}
              onHistoryChange={onHistoryChange}
              showTitle={!!channel || embedded}
              onAuthorChange={setChatAuthor}
              onLocalPresenceChange={accountPresence ? undefined : setLocalPresence}
              headerLeading={
                onNavigationToggle && (
                  <button
                    className="navigation-toggle"
                    type="button"
                    aria-label="Back to Browse"
                    onClick={onNavigationToggle}
                  >
                    <ArrowLeft aria-hidden="true" />
                  </button>
                )
              }
              headerBelow={voiceStrip}
              headerSubtitle={headerSubtitle}
              headerMenu={
                (membersPanel || headerMenuItems) && (
                  <>
                    {membersPanel && (
                      <button
                        type="button"
                        aria-expanded={membersVisible}
                        aria-controls={membersVisible ? "space-member-list" : undefined}
                        onClick={() => members.change(!membersVisible)}
                      >
                        <Users aria-hidden="true" />
                        {membersVisible ? "Hide members" : "Members"}
                        {memberCount != null && <small>{memberCount}</small>}
                      </button>
                    )}
                    {headerMenuItems}
                  </>
                )
              }
              headerActions={
                <div className="voice-actions">
                  {!audioPanel && actionError && (
                    <div className="room-error chat-refresh-error" role="alert">
                      {actionError}
                    </div>
                  )}
                  {channelJoined && channelActions}
                  {membersPanel && (
                    <Tooltip content={membersVisible ? "Hide member list" : "Show member list"}>
                      <button
                        type="button"
                        className="member-list-toggle"
                        aria-label={membersVisible ? "Hide member list" : "Show member list"}
                        aria-expanded={membersVisible}
                        aria-controls={membersVisible ? "space-member-list" : undefined}
                        onClick={() => members.change(!membersVisible)}
                      >
                        <Users aria-hidden="true" />
                      </button>
                    </Tooltip>
                  )}
                </div>
              }
            />
          )}
        </div>
        {(membersVisible || members.closing) && membersPanel && (
          <>
            <button
              type="button"
              className="member-list-backdrop"
              aria-label="Close member list"
              onClick={() => members.change(false)}
            />
            {membersPanel(() => members.change(false))}
          </>
        )}
      </section>
      <dialog
        ref={profileDialog}
        className="audio-dialog profile-dialog"
        aria-labelledby="profile-dialog-title"
        onKeyDown={dialogKeyDown}
        {...profileBackdrop}
        onCancel={(event) => {
          event.preventDefault();
          setProfileOpen(false);
        }}
      >
        <div className="audio-dialog-heading">
          <h2 id="profile-dialog-title">Edit profile</h2>
          <button
            type="button"
            className="voice-icon-button"
            aria-label="Close profile"
            onClick={() => setProfileOpen(false)}
          >
            <X aria-hidden="true" />
          </button>
        </div>
        {/* The body scrolls beneath a fixed heading, so a classic scrollbar's
            gutter never cuts the heading's divider short. */}
        <div className="audio-dialog-body">
          {profileOpen && account && (
            <ProfileForm
              account={account}
              onSaved={(updated) => {
                setAccount(updated);
                setName(updated.displayName ?? "");
                setProfileOpen(false);
              }}
            />
          )}
          {profileOpen && account?.username && <NotificationSettings />}
          {profileOpen && account?.username && <PrivacySettings />}
        </div>
      </dialog>
      {!embedded && (
        <WhatsNew
          ready={signedIn && identityReady}
          blocked={(state.phase !== "idle" && state.phase !== "failed") || !!audioPanel || !!audioMenu || profileOpen}
          manualOpen={whatsNewOpen}
          onManualClose={() => setWhatsNewOpen(false)}
        />
      )}
      <dialog
        ref={audioDialog}
        className="audio-dialog"
        data-panel={audioPanel}
        aria-labelledby="audio-dialog-title"
        onKeyDown={dialogKeyDown}
        onPointerDown={(event) => {
          if (event.target !== event.currentTarget) return;
          const bounds = event.currentTarget.getBoundingClientRect();
          if (
            event.clientX < bounds.left ||
            event.clientX > bounds.right ||
            event.clientY < bounds.top ||
            event.clientY > bounds.bottom
          ) {
            event.preventDefault();
            closeAudioPanel();
          }
        }}
        onCancel={(event) => {
          event.preventDefault();
          closeAudioPanel();
        }}
      >
        <div className="audio-dialog-heading">
          <h2 id="audio-dialog-title">
            {audioPanel === "mic" ? "Audio test" : audioPanel === "debug" ? "Audio diagnostics" : "Connection details"}
          </h2>
          <button
            type="button"
            className="voice-icon-button"
            aria-label={`Close ${audioPanel === "mic" ? "audio test" : audioPanel === "debug" ? "audio diagnostics" : "connection details"}`}
            onClick={closeAudioPanel}
          >
            <X aria-hidden="true" />
          </button>
        </div>
        <div className="audio-dialog-body">
          {audioPanel && (actionError || voiceError) && (
            <p className="call-error" role="alert">
              {actionError || voiceError}
            </p>
          )}
          {audioPanel === "mic" && (
            <>
              <p className="noise-status">Only you can hear these tests.</p>
              <div className="audio-test-devices">
                <div>
                  <label className="device-select">
                    Microphone
                    <select
                      aria-label="Test microphone device"
                      value={deviceId}
                      disabled={actionPending}
                      onChange={(event) => {
                        const id = event.target.value;
                        void act(
                          () => {
                            if (connected) return clientRef.current!.changeMicrophone(id);
                            clientRef.current!.stopLocalMicTest();
                            return clientRef.current!.startLocalMicTest(id);
                          },
                          () => setDeviceId(id),
                        );
                      }}
                    >
                      {deviceOptions(devices, "audioinput").map(({ device, label }) => (
                        <option key={device.deviceId} value={device.deviceId}>
                          {label}
                        </option>
                      ))}
                      {!deviceOptions(devices, "audioinput").length && <option value="">System default</option>}
                    </select>
                  </label>
                  <div className="volume-control">
                    <span>
                      Microphone volume <output>{state.inputVolume}%</output>
                    </span>
                    <Slider
                      label="Test microphone volume"
                      value={state.inputVolume}
                      max={200}
                      onChange={(value) => clientRef.current?.setInputVolume(value)}
                    />
                  </div>
                </div>
                <div>
                  <label className="device-select">
                    Speaker
                    <select
                      aria-label="Test speaker device"
                      value={output}
                      disabled={!outputSelectable}
                      onChange={(event) => setOutput(event.target.value)}
                    >
                      {deviceOptions(devices, "audiooutput").map(({ device, label }) => (
                        <option key={device.deviceId} value={device.deviceId}>
                          {label}
                        </option>
                      ))}
                      {!deviceOptions(devices, "audiooutput").length && <option value="">System default</option>}
                    </select>
                  </label>
                  {!outputSelectable && <p className="noise-status">Choose speakers in system settings.</p>}
                  <div className="volume-control">
                    <span>
                      Speaker volume <output>{outputVolume}%</output>
                    </span>
                    <Slider label="Test speaker volume" value={outputVolume} max={200} onChange={setOutputVolume} />
                  </div>
                  <SpeakerTest output={output} volume={outputVolume} />
                </div>
              </div>
              {actionPending && (
                <p className="noise-status" role="status">
                  Preparing microphone…
                </p>
              )}
              {!actionPending && actionError && !state.monitorStream && (
                <button type="button" className="voice-button" onClick={openMicTest}>
                  Try again
                </button>
              )}
              {state.monitorStream && !actionPending && (
                <MicPlayback
                  key={`${state.noiseSuppression}:${deviceId}`}
                  stream={state.monitorStream}
                  output={output}
                  volume={outputVolume}
                  processingStrength={state.voiceProcessingStrength ?? DEFAULT_VOICE_PROCESSING_STRENGTH}
                  onProcessingStrengthChange={(strength) => clientRef.current?.setVoiceProcessingStrength(strength)}
                />
              )}
              {account?.debugEnabled && (
                <details>
                  <summary>Audio diagnostics</summary>
                  <AudioDiagnostics client={clientRef.current} />
                </details>
              )}
            </>
          )}
          {audioPanel === "debug" && account?.debugEnabled && <AudioDiagnostics client={clientRef.current} />}
          {audioPanel === "connection" &&
            (state.diagnostics ? (
              <ConnectionDiagnostics diagnostics={state.diagnostics} />
            ) : (
              <p className="noise-status">Join voice to see connection details.</p>
            ))}
        </div>
      </dialog>
    </Root>
  );
}
