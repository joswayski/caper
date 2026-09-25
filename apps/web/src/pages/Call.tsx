import { useCallback, useEffect, useId, useLayoutEffect, useRef, useState, useSyncExternalStore, type ReactNode, type RefObject } from "react";
import { animals, colors, uniqueNamesGenerator } from "unique-names-generator";
import { AudioLines, ChevronDown, Hash, HeadphoneOff, Headphones, Menu, Mic, MicOff, PhoneOff, Speech, Settings, Users, VolumeX, X } from "lucide-react";
import ProfileForm from "../account/ProfileForm";
import { getAccount, logout, type Account } from "../account/client";
import { getSystemSoundsEnabled, playSound, preloadSoundEffects, setSystemSoundsEnabled, subscribeSystemSounds } from "../audio/effects";
import Chat from "../chat/Chat";
import type { ChatAuthor, GeneralChatHistory } from "../chat/types";
import Slider from "../components/Slider";
import PresenceDot from "../components/PresenceDot";
import Tooltip from "../components/Tooltip";
import { watchPresence as watchAccountPresence, type PresenceStatus } from "../gateway/client";
import { acquireAudioContext, releaseAudioContext, setPlaybackBlocked } from "../media/audio-context";
import { PublicCallClient, prepareVoiceJoin } from "../media/client";
import { watchPresence } from "../media/presence";
import type { CallViewState, Participant } from "../media/types";
import { DEFAULT_VOICE_PROCESSING_STRENGTH } from "../media/voice-processing";
import MicPlayback, { SpeakerTest } from "./MicPlayback";
import AudioDiagnostics from "./AudioDiagnostics";
import VoiceActivity from "./VoiceActivity";
import ChannelSidebar from "./ChannelSidebar";
import "./call.css";

const initialState: CallViewState = { phase: "idle", muted: false, deafened: false, inputVolume: 100, voiceProcessingStrength: DEFAULT_VOICE_PROCESSING_STRENGTH, monitoring: false, participants: [], remoteMedia: [] };
type PublicPresence = { participants: Array<Omit<Participant, "tracks">> };
const regionNames = new Intl.DisplayNames(["en"], { type: "region" });
const MAX_WATCHED_CHANNELS = 24;
const flags = import.meta.glob<string>("../../../../node_modules/flag-icons/flags/4x3/*.svg", { import: "default", query: "?url" });

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
  return devices.filter((device) => device.kind === kind).sort((left, right) => Number(right.deviceId === "default") - Number(left.deviceId === "default")).flatMap((device) => {
    const label = device.label.replace(/^Default\s*[-–]\s*/i, "") || (kind === "audioinput" ? "Microphone" : "Audio output");
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
    ["Microphone", `${Math.round(diagnostics.microphoneMs)} ms${diagnostics.microphoneDetail ? ` (${diagnostics.microphoneDetail})` : ""}`],
    ["Session + publish", `${Math.round(diagnostics.sessionMs)} ms`],
    ["Signaling + live updates", `${Math.round(diagnostics.signalingMs)} ms`],
    ["Transport + state", `${Math.round(diagnostics.transportMs)} ms${diagnostics.iceMs === undefined ? "" : ` (ICE ${Math.round(diagnostics.iceMs)} ms)`}`],
    ["Connectivity checks", diagnostics.checks ?? "Not observed yet"],
    ["Roster", `${Math.round(diagnostics.rosterMs)} ms`],
    ...(diagnostics.hearingMs === undefined ? [] : [["Hearing others", `${Math.round(diagnostics.hearingMs)} ms from Join`]]),
    ["Received", formatBytes(diagnostics.receivedBytes)],
    ["Live receive", formatBitrate(diagnostics.receiveBitrate)],
    ["Sent", formatBytes(diagnostics.sentBytes)],
    ["Live send", formatBitrate(diagnostics.sendBitrate)],
    ["Packets lost", String(diagnostics.packetsLost)],
    ["Max jitter", `${Math.round(diagnostics.maxJitterMs)} ms`],
    ["RTT", `${Math.round(diagnostics.roundTripMs)} ms`],
    ["Route", diagnostics.route === "relay" ? "TURN relay" : diagnostics.route === "direct" ? "Direct" : "Not observed yet"],
  ];
  return <section className="call-diagnostics" aria-label="Connection statistics">
    <dl>{values.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
    <p>Counters reset on reconnect.</p>
    <button type="button" className="voice-button" onClick={() => void navigator.clipboard.writeText(JSON.stringify(diagnostics, null, 2)).then(() => setCopyStatus("Copied connection details"), () => setCopyStatus("Copy failed; try again."))}>Copy connection details</button>
    <span role="status">{copyStatus}</span>
  </section>;
}

function AudioMenu({ label, settings, open, onOpenChange, menuRef, children }: { label: string; settings?: boolean; open: boolean; onOpenChange: (open: boolean) => void; menuRef?: RefObject<HTMLDivElement | null>; children: ReactNode }) {
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
  return <div ref={menuRef} className={`call-settings ${settings ? "" : "device-menu"}`} onKeyDown={(event) => {
    // Let native device pickers handle Escape without also removing their panel.
    if (event.target instanceof HTMLSelectElement) return;
    if (event.key === "Escape" && open && !event.defaultPrevented) { onOpenChange(false); event.currentTarget.querySelector<HTMLButtonElement>(".call-settings-trigger")?.focus(); }
  }}>
    <button type="button" className="call-settings-trigger" title={label} aria-label={label} aria-expanded={open} aria-controls={open ? panelId : undefined} onClick={() => onOpenChange(!open)}>{settings ? <Settings aria-hidden="true" /> : <ChevronDown aria-hidden="true" />}</button>
    {open && <div ref={panelRef} id={panelId} className="call-settings-panel" popover="manual" role="group" aria-label={`${label} panel`} tabIndex={-1}>{children}</div>}
  </div>;
}

function ParticipantCountry({ code }: { code?: string }) {
  const [flag, setFlag] = useState<{ code: string; source: string }>();
  useEffect(() => {
    if (!code || !/^[A-Z]{2}$/.test(code)) return;
    let current = true;
    const load = flags[`../../../../node_modules/flag-icons/flags/4x3/${code.toLowerCase()}.svg`];
    void load?.().then((source) => { if (current) setFlag({ code, source }); }).catch(() => undefined);
    return () => { current = false; };
  }, [code]);
  if (!code || flag?.code !== code) return null;
  const name = regionNames.of(code) ?? code;
  return <img className="participant-country" src={flag.source} alt={`From ${name}`} title={name} />;
}

function AudioOutput({ stream, muted, name, output, volume }: { stream: MediaStream; muted: boolean; name: string; output: string; volume: number }) {
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
      void Promise.all([context.resume(), element.play()]).then(() => setBlocked(false)).catch(() => setBlocked(true));
    } catch {
      source?.disconnect();
      gain?.disconnect();
      destination?.stream.getTracks().forEach((track) => track.stop());
      if (context) releaseAudioContext(context);
      context = undefined;
      element.srcObject = stream;
      element.volume = Math.min(volume / 100, 1);
      void element.play().then(() => setBlocked(false)).catch(() => setBlocked(true));
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
    if (ref.current?.setSinkId) void ref.current.setSinkId(output).then(() => setDeviceError(false)).catch(() => setDeviceError(true));
  }, [output]);
  return <><audio ref={ref} autoPlay muted={muted} />
    {blocked && <button onClick={() => void Promise.all([contextRef.current?.resume(), ref.current?.play()]).then(() => setBlocked(false)).catch(() => setBlocked(true))}>Play {name} audio</button>}
    {deviceError && <p role="alert">Audio output unavailable; choose another device.</p>}</>;
}

/** Voice occupants for one channel: a summary for its line and a list beneath it. */
export interface VoiceSlot { summary: ReactNode; list: ReactNode }

interface CallProps {
  channel?: { id: string; name: string; spaceName: string; spaceId?: string; demo?: boolean };
  membersPanel?: ReactNode;
  onVoiceChannelOpen?: (channelId: string, spaceId?: string) => void;
  spaceRail?: ReactNode;
  /** Channels in the current space, whose voice rosters appear under them. */
  voiceChannels?: Array<{ id: string; name: string }>;
  /** Channel list. As a function it receives voiceFor, to place the voice roster under its channel. */
  channelNavigation?: ReactNode | ((voiceFor: (channelId: string) => VoiceSlot | null) => ReactNode);
  navigationOpen?: boolean;
  onNavigationToggle?: () => void;
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
}

export default function Call({ channel, voiceChannels, spaceRail, channelNavigation, membersPanel, onVoiceChannelOpen, navigationOpen = false, onNavigationToggle, initialAccount, initialHistory, initialHistoryError, onHistoryChange, embedded = false, engaged = true, onChatOnlineChange }: CallProps = {}) {
  const systemSounds = useSyncExternalStore(subscribeSystemSounds, getSystemSoundsEnabled, () => true);
  const [state, setState] = useState(initialState);
  const [name, setName] = useState(initialAccount?.displayName ?? "");
  const [account, setAccount] = useState<Account | null>(initialAccount ?? null);
  const [chatAuthor, setChatAuthor] = useState<ChatAuthor>();
  const [identityReady, setIdentityReady] = useState(!!initialAccount);
  const [availability, setAvailability] = useState<Record<string, boolean>>({});
  const [audioPanel, setAudioPanel] = useState<"mic" | "connection" | "debug">();
  const [audioMenu, setAudioMenu] = useState<"input" | "output" | "settings">();
  const audioMenuRef = useRef<HTMLDivElement>(null);
  const audioDialog = useRef<HTMLDialogElement>(null);
  const audioReturnFocus = useRef<HTMLElement | null>(null);
  const [profileOpen, setProfileOpen] = useState(false);
  const profileDialog = useRef<HTMLDialogElement>(null);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = useState("");
  const [output, setOutput] = useState("");
  const [outputVolume, setOutputVolume] = useState(100);
  // Browser capability, so it is read after hydration to match server markup.
  const [outputSelectable, setOutputSelectable] = useState(false);
  useEffect(() => setOutputSelectable(typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype), []);
  const [membersVisible, setMembersVisible] = useState(true);
  const [selfPresence, setSelfPresence] = useState<PresenceStatus>();
  const [localPresence, setLocalPresence] = useState<PresenceStatus>("offline");
  const [presenceLive, setPresenceLive] = useState(false);
  const [actionError, setActionError] = useState<string>();
  const [actionPending, setActionPending] = useState(false);
  const actionGeneration = useRef(0);
  const [activeParticipants, setActiveParticipants] = useState<Set<string>>(() => new Set());
  const [participantVolumes, setParticipantVolumes] = useState<Record<string, number>>({});
  const [mutedParticipants, setMutedParticipants] = useState<Set<string>>(() => new Set());
  const [volumeParticipant, setVolumeParticipant] = useState<string>();
  const volumeMenuRef = useRef<HTMLLIElement>(null);
  const previousVoiceRoster = useRef<{ selfId: string; ids: Set<string> } | undefined>(undefined);
  const [collapsedRosters, setCollapsedRosters] = useState<ReadonlySet<string>>(() => new Set());
  const [publicParticipants, setPublicParticipants] = useState<Record<string, PublicPresence["participants"]>>({});
  const [voiceChannel, setVoiceChannel] = useState(channel);
  const clientRef = useRef<PublicCallClient | undefined>(undefined);
  const mediaRoot = channel && !channel.demo ? `/api/channels/${encodeURIComponent(channel.id)}/media` : "/api/media";
  const available = availability[mediaRoot];
  const clientRoot = useRef(mediaRoot);
  const rootFor = (channelId?: string) => !channelId || channel?.demo ? "/api/media" : `/api/channels/${encodeURIComponent(channelId)}/media`;
  const createClient = (root = mediaRoot) => {
    const client = new PublicCallClient((next) => { if (clientRef.current === client) setState(next); }, root);
    clientRef.current = client;
    clientRoot.current = root;
    return client;
  };
  if (!clientRef.current && typeof window !== "undefined") createClient();
  const connected = state.phase === "connected";
  const idle = state.phase === "idle" || state.phase === "failed" || state.phase === "leaving";
  const joinUnavailable = available !== true;
  const controlsDisabled = (!idle && !connected) || actionPending;
  // Keep the public roster until our own call reports its participants, so
  // joining (or failing to join) never empties the list for a moment.
  const publicRoster = idle || (state.phase === "joining" && !state.participants.length);
  const roster = publicRoster ? publicParticipants[mediaRoot] ?? [] : state.participants;
  const identityName = account?.displayName || chatAuthor?.name || name;
  const accountPresence = !!account && !!channel?.spaceId && !channel.demo;
  const joined = useRef(false);
  // The client reports an error only on the update where it happens, so keep
  // it until the next voice action or until it is dismissed.
  const [voiceError, setVoiceError] = useState<string>();
  useEffect(() => { if (state.error) setVoiceError(state.error); }, [state.error]);
  // Show "Connecting…" only if joining takes a moment; an immediate failure
  // (such as a denied microphone) should not flash it.
  const [joiningShown, setJoiningShown] = useState(false);
  useEffect(() => {
    if (state.phase !== "joining") { setJoiningShown(false); return; }
    const timer = setTimeout(() => setJoiningShown(true), 200);
    return () => clearTimeout(timer);
  }, [state.phase]);
  const pendingJoin = state.phase === "joining" && !joiningShown;
  const Root = embedded ? "div" : "main";

  useEffect(() => {
    if (window.matchMedia("(max-width: 760px)").matches) setMembersVisible(false);
  }, []);

  useEffect(() => {
    if (engaged) void preloadSoundEffects();
  }, [engaged]);

  useEffect(() => {
    setSelfPresence(undefined);
    setPresenceLive(false);
    if (!account || !channel?.spaceId || channel.demo) return;
    return watchAccountPresence(channel.spaceId, [account.id], (members) => {
      setSelfPresence(members.find((member) => member.userId === account.id)?.status);
    }, setPresenceLive);
  }, [account?.id, channel?.spaceId, channel?.demo]);

  useEffect(() => {
    if (connected && !joined.current) {
      joined.current = true;
      playSound("channel-join");
    } else if (idle) joined.current = false;
  }, [connected, idle]);

  useEffect(() => {
    const previous = previousVoiceRoster.current;
    if (!connected || !state.selfId) { previousVoiceRoster.current = undefined; return; }
    const ids = new Set(state.participants.map((person) => person.id));
    if (previous?.selfId === state.selfId && [...previous.ids].some((id) => id !== state.selfId && !ids.has(id))) playSound("channel-leave");
    previousVoiceRoster.current = { selfId: state.selfId, ids };
  }, [connected, state.selfId, state.participants]);

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

  useEffect(() => {
    let current = true;
    if (!initialAccount) setName(uniqueNamesGenerator({ dictionaries: [colors, animals], separator: " ", style: "capital" }));
    void (initialAccount ? Promise.resolve(initialAccount) : getAccount())
      .then((account) => {
        if (!current) return;
        setAccount(account);
        if (account?.displayName) setName(account.displayName);
      })
      .catch(() => undefined)
      .finally(() => { if (current) setIdentityReady(true); });
    const unload = () => clientRef.current?.leaveImmediately();
    window.addEventListener("pagehide", unload);
    return () => { current = false; window.removeEventListener("pagehide", unload); clientRef.current?.leaveImmediately(); };
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
      void getAccount().then((next) => { if (current && next) setAccount(next); }).catch(() => undefined);
    };
    if (settingsOpen) refresh();
    document.addEventListener("visibilitychange", refresh);
    return () => { current = false; document.removeEventListener("visibilitychange", refresh); };
  }, [signedIn, settingsOpen]);

  useEffect(() => {
    let current = true;
    fetch(`${mediaRoot}/status`, { credentials: "same-origin", signal: AbortSignal.timeout(10_000) })
      .then(async (response) => response.ok ? response.json() as Promise<{ enabled: boolean }> : { enabled: false })
      .then((result) => {
        if (!current) return;
        setAvailability((previous) => ({ ...previous, [mediaRoot]: result.enabled }));
      })
      .catch(() => { if (current) setAvailability((previous) => ({ ...previous, [mediaRoot]: false })); });
    return () => { current = false; };
  }, [mediaRoot]);

  useEffect(() => {
    // Download/compile only; never a permission prompt. Deferred for embedded
    // rooms so homepage visitors who only look do not fetch noise models.
    if (available === true && engaged) clientRef.current?.prepareMicrophone();
  }, [available, engaged, mediaRoot]);

  useEffect(() => {
    if (!idle || available !== true) return;
    return watchPresence((snapshot) => setPublicParticipants((previous) => ({ ...previous, [mediaRoot]: snapshot.participants })), () => undefined, mediaRoot);
  }, [idle, available, mediaRoot]);

  // Who is in voice in each of the space's channels, so people can see where
  // others are and join them. One spectator subscription per channel, capped
  // to stay within the gateway's per-connection subscription limit.
  const [channelRosters, setChannelRosters] = useState<Record<string, PublicPresence["participants"]>>({});
  const watchedChannels = channel?.demo ? "" : (voiceChannels ?? []).slice(0, MAX_WATCHED_CHANNELS).map((item) => item.id).join(",");
  useEffect(() => {
    if (!watchedChannels || available !== true) return;
    const stops = watchedChannels.split(",").map((id) => watchPresence(
      (snapshot) => setChannelRosters((previous) => ({ ...previous, [id]: snapshot.participants })),
      () => undefined,
      `/api/channels/${encodeURIComponent(id)}/media`,
    ));
    return () => stops.forEach((stop) => stop());
  }, [watchedChannels, available]);

  useEffect(() => {
    if (!navigator.mediaDevices) return;
    const update = () => void navigator.mediaDevices.enumerateDevices().then((next) => {
      setDevices(next);
      const inputs = deviceOptions(next, "audioinput");
      const outputs = deviceOptions(next, "audiooutput");
      setDeviceId((current) => inputs.some(({ device }) => device.deviceId === current) ? current : inputs[0]?.device.deviceId || "");
      setOutput((current) => outputs.some(({ device }) => device.deviceId === current) ? current : outputs[0]?.device.deviceId || "");
    }).catch(() => setActionError("Device list unavailable. Use system settings."));
    update();
    navigator.mediaDevices.addEventListener("devicechange", update);
    return () => navigator.mediaDevices.removeEventListener("devicechange", update);
  }, [connected, state.monitorStream]);

  useEffect(() => {
    if (!audioMenu) return;
    const dismiss = (event: PointerEvent | FocusEvent) => {
      if (!audioMenuRef.current?.contains(event.target as Node)) setAudioMenu(undefined);
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("focusin", dismiss);
    return () => {
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("focusin", dismiss);
    };
  }, [audioMenu]);

  useEffect(() => {
    if (audioPanel) {
      // The menu item becomes hidden when the dialog takes focus. Return to
      // its visible disclosure, rather than the browser's hidden opener.
      audioReturnFocus.current = document.activeElement?.closest(".call-settings")?.querySelector(".call-settings-trigger") ?? null;
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
    try { await operation(); if (generation === actionGeneration.current) success?.(); } catch (error) {
      if (generation === actionGeneration.current) setActionError(error instanceof Error ? error.message : "That action did not work.");
    } finally { if (generation === actionGeneration.current) setActionPending(false); }
  };
  const leave = () => {
    setVoiceError(undefined);
    if (connected) playSound("disconnect");
    void act(() => clientRef.current!.leave());
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
    const target = channelId === channel?.id ? channel : channel && channelId ? {
      ...channel, id: channelId, name: voiceChannels?.find((item) => item.id === channelId)?.name ?? "voice",
    } : channel;
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
    void clientRef.current!.join(identityName.trim(), deviceId);
  };
  /** Signed-in members: create the provider session as the pointer or focus reaches Join. */
  const prepareChannel = (channelId?: string) => {
    const root = channelId === channel?.id ? mediaRoot : rootFor(channelId);
    if (!signedIn || channel?.demo || joinBlocked || (!idle && clientRoot.current === root)) return;
    prepareVoiceJoin(root);
  };
  const prepareRef = useRef(prepareChannel);
  prepareRef.current = prepareChannel;
  // Start preparing as the pointer approaches a Join button, not only on hover.
  // One listener, at most one geometry check per frame, and only while signed in.
  useEffect(() => {
    if (!signedIn || channel?.demo) return;
    let frame = 0, x = 0, y = 0;
    const check = () => {
      frame = 0;
      for (const button of document.querySelectorAll<HTMLElement>(".channel-join[data-channel]")) {
        const box = button.getBoundingClientRect();
        if (!box.width || !box.height) continue;
        const dx = Math.max(box.left - x, 0, x - box.right);
        const dy = Math.max(box.top - y, 0, y - box.bottom);
        if (dx * dx + dy * dy <= JOIN_PREPARE_RADIUS * JOIN_PREPARE_RADIUS) prepareRef.current(button.dataset.channel || undefined);
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
  }, [signedIn, channel?.demo]);
  // Touch has no approach, and pointerdown lands too close to the tap to help. On a
  // touch screen, prepare the viewed channel once when its Join button is shown;
  // Cloudflare keeps an unused session for 10-15 s, so this covers a prompt tap.
  const viewedJoin = useCallback((button: HTMLButtonElement | null) => {
    if (!button || typeof IntersectionObserver === "undefined" || typeof matchMedia === "undefined" || !matchMedia("(pointer: coarse)").matches) return;
    const observer = new IntersectionObserver((entries) => {
      if (!entries.some((entry) => entry.isIntersecting)) return;
      observer.disconnect();
      prepareRef.current(button.dataset.channel || undefined);
    });
    observer.observe(button);
    return () => observer.disconnect();
    // A new identity re-attaches when the viewed channel changes and React reuses the button.
  }, [channel?.id]);
  const openMicTest = () => {
    setVoiceError(undefined);
    setAudioPanel("mic");
    void act(() => connected ? clientRef.current!.setMonitoring(true) : clientRef.current!.startLocalMicTest(deviceId));
  };

  useEffect(() => {
    if (profileOpen) profileDialog.current?.showModal();
    else profileDialog.current?.close();
  }, [profileOpen]);

  const isSpeaking = (participant: { id: string; muted: boolean }) => {
    const muted = participant.id === state.selfId ? state.muted : participant.muted;
    return activeParticipants.has(participant.id) && !muted;
  };
  const renderRoster = (people: typeof roster, own: boolean, label: string) => (
    <ul className={own && volumeParticipant ? "volume-menu-open" : undefined} aria-label={`People in voice in ${label}`}>
              {people.map((participant) => {
                const self = participant.id === state.selfId;
                const stream = self ? state.localMedia : state.remoteMedia.find((media) => media.participantId === participant.id)?.stream;
                const participantMuted = self ? state.muted : participant.muted;
                const participantDeafened = self ? state.deafened : participant.deafened;
                const participantStatus = participantDeafened ? "Deafened" : participantMuted ? "Muted" : undefined;
                const activityMuted = participantMuted;
                const speaking = isSpeaking(participant);
                return <li ref={own && volumeParticipant === participant.id ? volumeMenuRef : undefined} className={`participant ${volumeParticipant === participant.id ? "volume-open" : ""}`} key={participant.id} onContextMenu={!own || self ? undefined : (event) => { event.preventDefault(); setVolumeParticipant(participant.id); }}>
                  <span className="participant-avatar">
                    <span className={`avatar ${speaking ? "speaking" : "quiet"}`} aria-hidden="true">{participant.name.slice(0, 1).toUpperCase()}</span>
                  </span>
                  <span className="participant-name"><strong>{participant.name}{self ? " (you)" : ""}</strong><ParticipantCountry code={participant.countryCode} />{participantStatus && <span className="participant-status" title={participantStatus}>{participantMuted && <MicOff aria-hidden="true" />}{participantDeafened && <HeadphoneOff aria-hidden="true" />}<span className="sr-only">{participantStatus}</span></span>}
                    {!self && mutedParticipants.has(participant.id) && <span className="participant-local-muted"><VolumeX aria-hidden="true" />You muted {participant.name}</span>}
                  </span>
                  {own && <VoiceActivity
                    stream={stream}
                    muted={activityMuted}
                    onActivityChange={(active) => setActiveParticipants((current) => {
                      if (current.has(participant.id) === active) return current;
                      const next = new Set(current);
                      active ? next.add(participant.id) : next.delete(participant.id);
                      return next;
                    })}
                  />}
                  {own && !self && <button className="participant-menu-button" type="button" aria-label={`Audio controls for ${participant.name}`} aria-expanded={volumeParticipant === participant.id} onClick={() => setVolumeParticipant((current) => current === participant.id ? undefined : participant.id)}>Audio</button>}
                  {own && volumeParticipant === participant.id && <div className="participant-volume" role="group" aria-label={`${participant.name} local audio settings`}>
                    <div><strong>User volume</strong><output>{participantVolumes[participant.id] ?? 100}%</output></div>
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
                  </div>}
                </li>;
              })}
    </ul>
  );
  const voiceChannelKey = (channelId?: string) => channelId ?? "general";
  const inVoiceHere = (channelId?: string) => !idle && !pendingJoin && voiceChannel?.id === channelId;
  /** Who is in voice in a channel; your own call's roster (with controls) when you are in it. */
  const rosterFor = (channelId?: string) => {
    if (!publicRoster && voiceChannel?.id === channelId) return { people: state.participants, own: true };
    const watched = channelId ? channelRosters[channelId] : undefined;
    if (channelId === channel?.id) return { people: watched ?? publicParticipants[mediaRoot] ?? [], own: false };
    return { people: watched ?? [], own: false };
  };
  const channelLabel = (channelId?: string) => channelId === channel?.id || !channelId ? channel?.name ?? "general" : voiceChannels?.find((item) => item.id === channelId)?.name ?? "voice";
  // Each channel line carries its voice: a quiet stack of who is in it (toggles
  // the list below) and Join, which glows softly while people are in there.
  // When the name leaves no room, both wrap onto an indented row beneath it.
  let rosterPlaced = false;
  const voiceFor = (channelId?: string): VoiceSlot | null => {
    const { people, own } = rosterFor(channelId);
    if (own) rosterPlaced = true;
    const key = voiceChannelKey(channelId);
    const label = channelLabel(channelId);
    const open = !collapsedRosters.has(key);
    const listId = `voice-occupants-${key.replace(/[^\w-]/g, "")}`;
    const busy = pendingJoin && voiceChannel?.id === channelId;
    const switching = !idle && !inVoiceHere(channelId);
    const stack = people.length > 0 && <button className="voice-stack" type="button" aria-expanded={open} aria-controls={listId} aria-label={`${people.length} in voice in ${label}. ${open ? "Hide" : "Show"} who is in voice.`} onClick={() => {
      setCollapsedRosters((current) => { const next = new Set(current); open ? next.add(key) : next.delete(key); return next; });
      setVolumeParticipant(undefined);
    }}>
      <span className="voice-stack-faces" aria-hidden="true">
        {people.slice(0, 3).map((participant) => <span key={participant.id} className={`voice-stack-avatar${own && isSpeaking(participant) ? " speaking" : ""}`}>{participant.name.slice(0, 1).toUpperCase()}</span>)}
        {people.length > 3 && <small>+{people.length - 3}</small>}
      </span>
      <ChevronDown aria-hidden="true" />
    </button>;
    // Join shows on the channel being viewed, and on channels with people in
    // voice when you hover or focus their line (hidden on touch screens).
    const viewed = channelId === channel?.id;
    const join = !inVoiceHere(channelId) && (viewed || people.length > 0) && <Tooltip content={joinUnavailable ? available === false ? "Joining is not available at this time." : "Checking voice availability…" : switching ? `Switch voice to #${label}` : `Join voice in #${label}`}>
      <button ref={channelId === channel?.id ? viewedJoin : undefined} className="voice-button channel-join" type="button" data-channel={channelId ?? ""} data-live={people.length > 0 ? "" : undefined} data-hover-only={viewed ? undefined : ""} aria-label={channelId === channel?.id ? "Join voice" : `Join voice in #${label}`} aria-disabled={joinBlocked || busy} aria-busy={busy} onPointerEnter={() => prepareChannel(channelId)} onPointerDown={() => prepareChannel(channelId)} onFocus={() => prepareChannel(channelId)} onClick={() => joinChannel(channelId)}><Speech aria-hidden="true" /><span className="channel-join-label">Join</span></button>
    </Tooltip>;
    if (!stack && !join) return null;
    return {
      summary: <span className="channel-voice">{stack}{join}</span>,
      list: people.length > 0 ? <div className="voice-occupants" id={listId} data-open={open ? "" : undefined}>
        <div className="voice-occupants-inner" inert={!open}>{renderRoster(people, own, label)}</div>
      </div> : null,
    };
  };
  let navigation: ReactNode;
  if (typeof channelNavigation === "function") navigation = channelNavigation(voiceFor);
  else if (channelNavigation) navigation = channelNavigation;
  else {
    const voice = voiceFor(undefined);
    navigation = <div className="channel-item">
      <div className="channel-row">
        <a className="channel-link" href="#chat-heading" aria-current="location"><Hash aria-hidden="true" /><span>general</span></a>
        {voice?.summary}
      </div>
      {voice?.list}
    </div>;
  }

  return (
    <Root className={`call-page${embedded ? " call-embedded" : ""}`}>
      {!embedded && <header className="call-header">
        <a className="wordmark" href="/">caper<span className="wordmark-dot">.</span></a>
      </header>}
      <section className={`call-room${channel ? " spaces-room" : ""}${navigationOpen ? " navigation-open" : ""}`}>
        {spaceRail}
        <ChannelSidebar defaultWidth={embedded ? 330 : undefined}>
          <div className="sidebar-channels">
          {navigation}
          {!rosterPlaced && !publicRoster && roster.length > 0 && <div className="voice-elsewhere">
            <p className="voice-roster-label">In voice · {roster.length} · {voiceChannel?.name ?? "general"}</p>
            {renderRoster(roster, true, voiceChannel?.name ?? "general")}
          </div>}
          </div>
          <div className="voice-panel">
          {((!idle && !pendingJoin) || (voiceError && !audioPanel)) && <div className="voice-dock">
            {!idle && !pendingJoin && <div className="connected-channel" data-phase={state.phase} role="status">
              <button type="button" className="voice-dock-channel" onClick={() => {
                if (voiceChannel && onVoiceChannelOpen) onVoiceChannelOpen(voiceChannel.id, voiceChannel.spaceId);
                else setAudioPanel("connection");
              }}>
                <AudioLines aria-hidden="true" />
                <span>
                  <strong>{connected ? "Voice connected" : state.phase === "joining" ? "Connecting…" : "Reconnecting…"}</strong>
                  <small>{voiceChannel?.name ?? initialHistory?.channel.name ?? "general"} / {voiceChannel?.spaceName ?? initialHistory?.space.name ?? "Public demo"}</small>
                </span>
              </button>
              <Tooltip content={connected ? "Disconnect" : "Cancel"}><button type="button" className="voice-hangup" aria-label={connected ? "Leave voice" : "Cancel joining voice"} onClick={leave}><PhoneOff aria-hidden="true" /></button></Tooltip>
            </div>}
            {voiceError && !audioPanel && <p className="voice-error" role="alert">
              <span>{voiceError}</span>
              <button type="button" aria-label="Dismiss voice error" onClick={() => setVoiceError(undefined)}><X aria-hidden="true" /></button>
            </p>}
          </div>}
          <div className="call-account">
            <button className="account-profile" type="button" disabled={!identityReady} aria-label={account ? `Edit profile for ${identityName}` : "Sign in to edit your profile"} onClick={() => { if (account) setProfileOpen(true); else window.location.assign("/login"); }}>
              <span className="account-avatar"><span aria-hidden="true">{identityName.slice(0, 1).toUpperCase()}</span><PresenceDot status={accountPresence ? selfPresence : localPresence} live={accountPresence ? presenceLive : true} /></span>
              <strong className="account-name" title={identityName}>{identityName || "Loading…"}</strong>
            </button>
            <div className={`voice-action-group${state.muted ? " active" : ""}`}>
              <Tooltip content={state.monitoring ? "Stop mic test to change mute" : state.muted ? "Unmute" : "Mute"}><button type="button" className={`voice-icon-button ${state.muted ? "active" : ""}`} aria-disabled={state.monitoring} aria-label={state.muted ? "Unmute microphone" : "Mute microphone"} aria-pressed={state.muted} onClick={() => { if (state.monitoring) return; const muted = !state.muted; playSound(muted ? "toggle-off" : "toggle-on"); setActionError(undefined); void clientRef.current!.setMuted(muted).catch((error) => setActionError(error instanceof Error ? error.message : "Mute state could not be shared.")); }}>{state.muted ? <MicOff aria-hidden="true" /> : <Mic aria-hidden="true" />}</button></Tooltip>
              <AudioMenu label="Input Options" open={audioMenu === "input"} onOpenChange={(open) => setAudioMenu(open ? "input" : undefined)} menuRef={audioMenu === "input" ? audioMenuRef : undefined}>
                <label className="device-select">Microphone
                  <select name="input-device" value={deviceId} disabled={controlsDisabled} onChange={(event) => { const id = event.target.value; if (connected) void act(() => clientRef.current!.changeMicrophone(id), () => setDeviceId(id)); else setDeviceId(id); }}>
                    {deviceOptions(devices, "audioinput").map(({ device, label }) => <option key={device.deviceId} value={device.deviceId}>{label}</option>)}
                    {!deviceOptions(devices, "audioinput").length && <option value="">System default</option>}
                  </select>
                </label>
                <div className="volume-control output-volume"><span>Input volume <output>{state.inputVolume}%</output></span><Slider label="Input volume" value={state.inputVolume} max={200} onChange={(value) => clientRef.current?.setInputVolume(value)} /></div>
              </AudioMenu>
            </div>
            <div className={`voice-action-group${state.deafened ? " active" : ""}`}>
              <Tooltip content={state.monitoring ? "Stop mic test to change deafen" : state.deafened ? "Undeafen" : "Deafen"}><button type="button" className={`voice-icon-button ${state.deafened ? "active" : ""}`} aria-disabled={state.monitoring} aria-label={state.deafened ? "Undeafen audio" : "Deafen audio"} aria-pressed={state.deafened} onClick={() => { if (state.monitoring) return; const deafened = !state.deafened; playSound(deafened ? "toggle-off" : "toggle-on"); setActionError(undefined); void clientRef.current!.setDeafened(deafened).catch((error) => setActionError(error instanceof Error ? error.message : "Deafen state could not be shared.")); }}>{state.deafened ? <VolumeX aria-hidden="true" /> : <Headphones aria-hidden="true" />}</button></Tooltip>
              <AudioMenu label="Output Options" open={audioMenu === "output"} onOpenChange={(open) => setAudioMenu(open ? "output" : undefined)} menuRef={audioMenu === "output" ? audioMenuRef : undefined}>
                {outputSelectable ? <label className="device-select">Audio output
                  <select name="output-device" value={output} onChange={(event) => setOutput(event.target.value)}>
                    {deviceOptions(devices, "audiooutput").map(({ device, label }) => <option key={device.deviceId} value={device.deviceId}>{label}</option>)}
                    {!deviceOptions(devices, "audiooutput").length && <option value="">System default</option>}
                  </select>
                </label> : <p className="noise-status">Choose audio output in system settings.</p>}
                <div className="volume-control output-volume"><span>Output volume <output>{outputVolume}%</output></span><Slider label="Output volume" value={outputVolume} max={200} onChange={setOutputVolume} /></div>
              </AudioMenu>
            </div>
            <AudioMenu label="User Settings" settings open={audioMenu === "settings"} onOpenChange={(open) => setAudioMenu(open ? "settings" : undefined)} menuRef={audioMenu === "settings" ? audioMenuRef : undefined}>
              <strong>Audio settings</strong>
              <fieldset className="device-options">
                <label><input type="checkbox" role="switch" checked={systemSounds} onChange={(event) => { setSystemSoundsEnabled(event.target.checked); if (event.target.checked) { void preloadSoundEffects(); playSound("toggle-on"); } }} /><span>Caper sound effects</span></label>
              </fieldset>
              <button disabled={!identityReady || controlsDisabled || state.phase === "leaving"} type="button" onClick={openMicTest}>Audio test</button>
              {state.diagnostics && <button type="button" onClick={() => setAudioPanel("connection")}>Connection details</button>}
              {account?.debugEnabled && <button type="button" onClick={() => setAudioPanel("debug")}>Audio diagnostics</button>}
              {identityReady && (account ? <button type="button" onClick={() => void logout().then(() => window.location.assign("/"))}>Log out</button> : <a href="/login">Sign in</a>)}
            </AudioMenu>
          </div>
          </div>
        </ChannelSidebar>
        <div className="stage">
          {state.remoteMedia.map((media) => <AudioOutput key={media.trackId} stream={media.stream} muted={state.deafened || mutedParticipants.has(media.participantId)} output={output} volume={outputVolume * (participantVolumes[media.participantId] ?? 100) / 100} name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"} />)}
          <Chat key={channel?.id ?? "general"} name={name} signedIn={!!account} identityReady={identityReady && engaged} messageSounds={engaged} onOnlineChange={onChatOnlineChange} channelId={channel?.id} channelName={channel?.name} initialHistory={initialHistory} initialHistoryError={initialHistoryError} onHistoryChange={onHistoryChange} showTitle={!!channel || embedded} onAuthorChange={setChatAuthor} onLocalPresenceChange={accountPresence ? undefined : setLocalPresence} headerActions={<div className="voice-actions">
            {!audioPanel && actionError && <div className="room-error chat-refresh-error" role="alert">{actionError}</div>}
            {onNavigationToggle && <button className="navigation-toggle" type="button" aria-expanded={navigationOpen} onClick={onNavigationToggle}><Menu aria-hidden="true" />Browse</button>}
            {membersPanel && <Tooltip content={membersVisible ? "Hide member list" : "Show member list"}><button type="button" className="member-list-toggle" aria-label={membersVisible ? "Hide member list" : "Show member list"} aria-expanded={membersVisible} aria-controls={membersVisible ? "space-member-list" : undefined} onClick={() => setMembersVisible(!membersVisible)}><Users aria-hidden="true" /></button></Tooltip>}
          </div>} />
        </div>
        {membersVisible && membersPanel}
      </section>
      <dialog ref={profileDialog} className="audio-dialog profile-dialog" aria-labelledby="profile-dialog-title" onCancel={(event) => { event.preventDefault(); setProfileOpen(false); }}>
        <div className="audio-dialog-heading">
          <h2 id="profile-dialog-title">Edit profile</h2>
          <button type="button" className="voice-icon-button" aria-label="Close profile" onClick={() => setProfileOpen(false)}><X aria-hidden="true" /></button>
        </div>
        <p className="noise-status">Your username is unique. Your display name is what people see in conversations.</p>
        {profileOpen && account && <ProfileForm account={account} onSaved={(updated) => { setAccount(updated); setName(updated.displayName ?? ""); setProfileOpen(false); }} />}
      </dialog>
      <dialog ref={audioDialog} className="audio-dialog" aria-labelledby="audio-dialog-title" onPointerDown={(event) => {
        if (event.target !== event.currentTarget) return;
        const bounds = event.currentTarget.getBoundingClientRect();
        if (event.clientX < bounds.left || event.clientX > bounds.right || event.clientY < bounds.top || event.clientY > bounds.bottom) {
          event.preventDefault();
          closeAudioPanel();
        }
      }} onCancel={(event) => { event.preventDefault(); closeAudioPanel(); }}>
        <div className="audio-dialog-heading">
          <h2 id="audio-dialog-title">{audioPanel === "mic" ? "Audio test" : audioPanel === "debug" ? "Audio diagnostics" : "Connection details"}</h2>
          <button type="button" className="voice-icon-button" aria-label="Close audio settings" onClick={closeAudioPanel}><X aria-hidden="true" /></button>
        </div>
        {audioPanel && (actionError || voiceError) && <p className="call-error" role="alert">{actionError || voiceError}</p>}
        {audioPanel === "mic" && <>
          <p className="noise-status">Only you can hear these tests.</p>
          <div className="audio-test-devices">
            <div>
              <label className="device-select">Microphone
                <select aria-label="Test microphone device" value={deviceId} disabled={actionPending} onChange={(event) => {
                  const id = event.target.value;
                  void act(() => {
                    if (connected) return clientRef.current!.changeMicrophone(id);
                    clientRef.current!.stopLocalMicTest();
                    return clientRef.current!.startLocalMicTest(id);
                  }, () => setDeviceId(id));
                }}>
                  {deviceOptions(devices, "audioinput").map(({ device, label }) => <option key={device.deviceId} value={device.deviceId}>{label}</option>)}
                  {!deviceOptions(devices, "audioinput").length && <option value="">System default</option>}
                </select>
              </label>
              <div className="volume-control"><span>Microphone volume <output>{state.inputVolume}%</output></span><Slider label="Test microphone volume" value={state.inputVolume} max={200} onChange={(value) => clientRef.current?.setInputVolume(value)} /></div>
            </div>
            <div>
              <label className="device-select">Speaker
                <select aria-label="Test speaker device" value={output} disabled={!outputSelectable} onChange={(event) => setOutput(event.target.value)}>
                  {deviceOptions(devices, "audiooutput").map(({ device, label }) => <option key={device.deviceId} value={device.deviceId}>{label}</option>)}
                  {!deviceOptions(devices, "audiooutput").length && <option value="">System default</option>}
                </select>
              </label>
              {!outputSelectable && <p className="noise-status">Choose speakers in system settings.</p>}
              <div className="volume-control"><span>Speaker volume <output>{outputVolume}%</output></span><Slider label="Test speaker volume" value={outputVolume} max={200} onChange={setOutputVolume} /></div>
              <SpeakerTest output={output} volume={outputVolume} />
            </div>
          </div>
          {actionPending && <p className="noise-status" role="status">Preparing microphone…</p>}
          {!actionPending && actionError && !state.monitorStream && <button type="button" className="voice-button" onClick={openMicTest}>Try again</button>}
          {state.monitorStream && !actionPending && <MicPlayback key={`${state.noiseSuppression}:${deviceId}`} stream={state.monitorStream} output={output} volume={outputVolume} processingStrength={state.voiceProcessingStrength ?? DEFAULT_VOICE_PROCESSING_STRENGTH} onProcessingStrengthChange={(strength) => clientRef.current?.setVoiceProcessingStrength(strength)} />}
          {account?.debugEnabled && <details><summary>Audio diagnostics</summary><AudioDiagnostics client={clientRef.current} /></details>}
        </>}
        {audioPanel === "debug" && account?.debugEnabled && <AudioDiagnostics client={clientRef.current} />}
        {audioPanel === "connection" && (state.diagnostics ? <ConnectionDiagnostics diagnostics={state.diagnostics} /> : <p className="noise-status">Join voice to see connection details.</p>)}
      </dialog>
    </Root>
  );
}
