import { useEffect, useRef, useState, useSyncExternalStore, type ReactNode, type RefObject } from "react";
import { animals, colors, uniqueNamesGenerator } from "unique-names-generator";
import { AudioLines, ChevronDown, Hash, Headphones, Menu, Mic, MicOff, PhoneOff, Speech, Settings, Users, Volume2, VolumeX, X } from "lucide-react";
import ProfileForm from "../account/ProfileForm";
import { getAccount, logout, type Account } from "../account/client";
import { getSystemSoundsEnabled, playSound, preloadSoundEffects, setSystemSoundsEnabled, subscribeSystemSounds } from "../audio/effects";
import Chat from "../chat/Chat";
import type { ChatAuthor, GeneralChatHistory } from "../chat/types";
import Slider from "../components/Slider";
import PresenceDot from "../components/PresenceDot";
import Tooltip from "../components/Tooltip";
import { watchPresence as watchAccountPresence, type PresenceStatus } from "../gateway/client";
import { acquireAudioContext, releaseAudioContext } from "../media/audio-context";
import { PublicCallClient } from "../media/client";
import { watchPresence } from "../media/presence";
import type { CallViewState, Participant } from "../media/types";
import { DEFAULT_VOICE_PROCESSING_STRENGTH } from "../media/voice-processing";
import MicPlayback from "./MicPlayback";
import AudioDiagnostics from "./AudioDiagnostics";
import VoiceActivity from "./VoiceActivity";
import ChannelSidebar from "./ChannelSidebar";
import "./call.css";

const initialState: CallViewState = { phase: "idle", muted: false, deafened: false, inputVolume: 100, voiceProcessingStrength: DEFAULT_VOICE_PROCESSING_STRENGTH, monitoring: false, participants: [], remoteMedia: [] };
type PublicPresence = { participants: Array<Omit<Participant, "tracks">> };
const regionNames = new Intl.DisplayNames(["en"], { type: "region" });
const flags = import.meta.glob<string>("../../../../node_modules/flag-icons/flags/4x3/*.svg", { import: "default", query: "?url" });

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
  const values = [
    ["Joined", diagnostics.join],
    ["Microphone + session", `${Math.round(diagnostics.microphoneSessionMs)} ms`],
    ["Signaling + live updates", `${Math.round(diagnostics.signalingMs)} ms`],
    ["Transport + state", `${Math.round(diagnostics.transportMs)} ms`],
    ["Roster", `${Math.round(diagnostics.rosterMs)} ms`],
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
    <p>Local estimates, not billing totals. Counters reset on reconnect.</p>
  </section>;
}

function AudioMenu({ label, settings, open, onOpenChange, menuRef, children }: { label: string; settings?: boolean; open: boolean; onOpenChange: (open: boolean) => void; menuRef?: RefObject<HTMLDetailsElement | null>; children: ReactNode }) {
  return <details ref={menuRef} open={open} className={`call-settings ${settings ? "" : "device-menu"}`} onKeyDown={(event) => {
    if (event.key === "Escape") { onOpenChange(false); event.currentTarget.querySelector("summary")?.focus(); }
  }}>
    <Tooltip content={label}><summary aria-label={label} onClick={(event) => { event.preventDefault(); onOpenChange(!open); }}>{settings ? <Settings aria-hidden="true" /> : <ChevronDown aria-hidden="true" />}</summary></Tooltip>
    <div className="call-settings-panel">{children}</div>
  </details>;
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
    const element = ref.current;
    if (!element) return;
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

/** Voice occupants for one channel: a summary for its row and a list beneath it. */
export interface VoiceSlot { summary: ReactNode; list: ReactNode }

interface CallProps {
  channel?: { id: string; name: string; spaceName: string; spaceId?: string; demo?: boolean };
  membersPanel?: ReactNode;
  onVoiceChannelOpen?: (channelId: string, spaceId?: string) => void;
  spaceRail?: ReactNode;
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

export default function Call({ channel, spaceRail, channelNavigation, membersPanel, onVoiceChannelOpen, navigationOpen = false, onNavigationToggle, initialAccount, initialHistory, initialHistoryError, onHistoryChange, embedded = false, engaged = true, onChatOnlineChange }: CallProps = {}) {
  const systemSounds = useSyncExternalStore(subscribeSystemSounds, getSystemSoundsEnabled, () => true);
  const [state, setState] = useState(initialState);
  const [name, setName] = useState(initialAccount?.displayName ?? "");
  const [account, setAccount] = useState<Account | null>(initialAccount ?? null);
  const [chatAuthor, setChatAuthor] = useState<ChatAuthor>();
  const [identityReady, setIdentityReady] = useState(!!initialAccount);
  const [availability, setAvailability] = useState<Record<string, boolean>>({});
  const [audioPanel, setAudioPanel] = useState<"mic" | "connection" | "debug">();
  const [audioMenu, setAudioMenu] = useState<"input" | "output" | "settings">();
  const audioMenuRef = useRef<HTMLDetailsElement>(null);
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
  const [rosterOpen, setRosterOpen] = useState(true);
  const [publicParticipants, setPublicParticipants] = useState<Record<string, PublicPresence["participants"]>>({});
  const [voiceChannel, setVoiceChannel] = useState(channel);
  const clientRef = useRef<PublicCallClient | undefined>(undefined);
  const mediaRoot = channel && !channel.demo ? `/api/channels/${encodeURIComponent(channel.id)}/media` : "/api/media";
  const available = availability[mediaRoot];
  const clientRoot = useRef(mediaRoot);
  const createClient = () => {
    const client = new PublicCallClient((next) => { if (clientRef.current === client) setState(next); }, mediaRoot);
    clientRef.current = client;
    clientRoot.current = mediaRoot;
    return client;
  };
  if (!clientRef.current && typeof window !== "undefined") createClient();
  const connected = state.phase === "connected";
  const idle = state.phase === "idle" || state.phase === "failed" || state.phase === "leaving";
  const viewingVoice = clientRoot.current === mediaRoot;
  const joinDisabled = !identityReady || state.phase === "leaving" || ((!viewingVoice || idle) && (available !== true || actionPending));
  const joinUnavailable = (!viewingVoice || idle) && available !== true;
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
  const pendingJoin = viewingVoice && state.phase === "joining" && !joiningShown;
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
      audioReturnFocus.current = document.activeElement?.closest("details")?.querySelector("summary") ?? null;
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
    if (connected) playSound("channel-leave");
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
  const joinVoice = () => {
    if (joinDisabled || (viewingVoice && !idle)) return;
    setActionError(undefined);
    setVoiceError(undefined);
    if (!viewingVoice) {
      const previous = clientRef.current!;
      previous.leaveImmediately();
      const client = createClient();
      client.copyAudioPreferencesFrom(previous);
    }
    setVoiceChannel(channel);
    void clientRef.current!.join(identityName.trim(), deviceId);
  };
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
    return activeParticipants.has(participant.id) && !(muted && !state.monitoring);
  };
  const rosterChannelId = publicRoster ? channel?.id : voiceChannel?.id;
  const rosterChannelName = (publicRoster ? channel : voiceChannel)?.name ?? "general";
  const rosterList = (
            <ul className={volumeParticipant ? "volume-menu-open" : undefined} aria-label={`People in voice in ${rosterChannelName}`}>
              {roster.map((participant) => {
                const self = participant.id === state.selfId;
                const stream = self ? state.localMedia : state.remoteMedia.find((media) => media.participantId === participant.id)?.stream;
                const participantMuted = self ? state.muted : participant.muted;
                const participantDeafened = self ? state.deafened : participant.deafened;
                const participantStatus = participantDeafened ? "Deafened" : participantMuted ? "Muted" : undefined;
                const activityMuted = participantMuted && !state.monitoring;
                const speaking = isSpeaking(participant);
                return <li className={`participant ${volumeParticipant === participant.id ? "volume-open" : ""}`} key={participant.id} onContextMenu={publicRoster || self ? undefined : (event) => { event.preventDefault(); setVolumeParticipant(participant.id); }}>
                  <span className="participant-avatar">
                    <span className={`avatar ${speaking ? "speaking" : "quiet"}`} aria-hidden="true">{participant.name.slice(0, 1).toUpperCase()}</span>
                  </span>
                  <span className="participant-name"><strong>{participant.name}{self ? " (you)" : ""}</strong><ParticipantCountry code={participant.countryCode} /><small aria-hidden={!participantStatus}>{participantStatus ?? "\u00a0"}</small></span>
                  {!publicRoster && <VoiceActivity
                    stream={stream}
                    muted={activityMuted}
                    onActivityChange={(active) => setActiveParticipants((current) => {
                      if (current.has(participant.id) === active) return current;
                      const next = new Set(current);
                      active ? next.add(participant.id) : next.delete(participant.id);
                      return next;
                    })}
                  />}
                  {!publicRoster && !self && <button className="participant-menu-button" type="button" aria-label={`Audio controls for ${participant.name}`} aria-expanded={volumeParticipant === participant.id} onClick={() => setVolumeParticipant((current) => current === participant.id ? undefined : participant.id)}>Audio</button>}
                  {!publicRoster && volumeParticipant === participant.id && <div className="participant-volume" role="group" aria-label={`${participant.name} local audio settings`}>
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
  // Voice occupants render under their channel: a stack of avatars on the
  // channel row (click to collapse or expand) and a compact list beneath it.
  let rosterPlaced = false;
  const voiceFor = (channelId?: string): VoiceSlot | null => {
    if (channelId !== rosterChannelId || !roster.length) return null;
    rosterPlaced = true;
    const open = rosterOpen || !!volumeParticipant;
    return {
      summary: <button className="voice-stack" type="button" aria-expanded={open} aria-controls="voice-occupants" aria-label={`${roster.length} in voice. ${open ? "Hide" : "Show"} who is in voice.`} onClick={() => { setRosterOpen(!open); setVolumeParticipant(undefined); }}>
        {roster.slice(0, 3).map((participant) => <span key={participant.id} className={`voice-stack-avatar${isSpeaking(participant) ? " speaking" : ""}`} aria-hidden="true">{participant.name.slice(0, 1).toUpperCase()}</span>)}
        <small aria-hidden="true">{roster.length}</small>
        <ChevronDown aria-hidden="true" />
      </button>,
      list: <div className="voice-occupants" id="voice-occupants" data-open={open ? "" : undefined}>
        <div className="voice-occupants-inner" inert={!open}>{rosterList}</div>
      </div>,
    };
  };
  let navigation: ReactNode;
  if (typeof channelNavigation === "function") navigation = channelNavigation(voiceFor);
  else if (channelNavigation) navigation = channelNavigation;
  else {
    const voice = voiceFor(undefined);
    navigation = <>
      <div className="channel-row">
        <a className="channel-link" href="#chat-heading" aria-current="location"><Hash aria-hidden="true" /><span>general</span></a>
        {voice?.summary}
      </div>
      {voice?.list}
    </>;
  }

  return (
    <Root className={`call-page${embedded ? " call-embedded" : ""}`}>
      {!embedded && <header className="call-header">
        <a className="wordmark" href="/">caper<span className="wordmark-dot">.</span></a>
      </header>}
      <section className={`call-room${channel ? " spaces-room" : ""}${navigationOpen ? " navigation-open" : ""}`}>
        {spaceRail}
        <ChannelSidebar>
          <div className="sidebar-channels">
          {navigation}
          {!rosterPlaced && roster.length > 0 && <div className="voice-elsewhere">
            <p className="voice-roster-label">In voice · {roster.length} · {rosterChannelName}</p>
            {rosterList}
          </div>}
          </div>
          <div className="voice-panel">
          <div className="voice-dock">
            {!idle && !pendingJoin && <div className="connected-channel" data-phase={state.phase} role="status">
              <button type="button" className="voice-dock-channel" onClick={() => voiceChannel && onVoiceChannelOpen?.(voiceChannel.id, voiceChannel.spaceId)}>
                <AudioLines aria-hidden="true" />
                <span>
                  <strong>{connected ? "Voice connected" : state.phase === "joining" ? "Connecting…" : "Reconnecting…"}</strong>
                  <small>{voiceChannel ? `${voiceChannel.spaceName} / ${voiceChannel.name}` : "#general"}</small>
                </span>
              </button>
              <Tooltip content={connected ? "Disconnect" : "Cancel"}><button type="button" className="voice-hangup" aria-label={connected ? "Leave voice" : "Cancel joining voice"} onClick={leave}><PhoneOff aria-hidden="true" /></button></Tooltip>
            </div>}
            {(idle || !viewingVoice || pendingJoin) && <div className="voice-join-row">
              <Volume2 aria-hidden="true" />
              <span>
                <strong>{channel?.name ?? "general"}</strong>
                <small>{!idle && !viewingVoice ? "Switch voice to this channel" : roster.length ? `${roster.length} in voice` : "No one in voice yet"}</small>
              </span>
              <Tooltip id="voice-availability" content={joinUnavailable ? available === false ? "Joining is not available at this time." : "Checking voice availability…" : undefined}><button className="voice-button" type="button" aria-label="Join voice" aria-disabled={joinDisabled || pendingJoin} aria-busy={pendingJoin} onClick={joinVoice}><Speech aria-hidden="true" />Join</button></Tooltip>
            </div>}
            {voiceError && !audioPanel && <p className="voice-error" role="alert">
              <span>{voiceError}</span>
              <button type="button" aria-label="Dismiss voice error" onClick={() => setVoiceError(undefined)}><X aria-hidden="true" /></button>
            </p>}
          </div>
          <div className="call-account">
            <button className="account-profile" type="button" disabled={!identityReady} aria-label={account ? `Edit profile for ${identityName}` : "Sign in to edit your profile"} onClick={() => { if (account) setProfileOpen(true); else window.location.assign("/login"); }}>
              <span className="account-avatar"><span aria-hidden="true">{identityName.slice(0, 1).toUpperCase()}</span><PresenceDot status={accountPresence ? selfPresence : localPresence} live={accountPresence ? presenceLive : true} /></span>
              <strong className="account-name" title={identityName}>{identityName || "Loading…"}</strong>
            </button>
            <div className={`voice-action-group${state.muted ? " active" : ""}`}>
              <Tooltip content={state.monitoring ? "Stop mic test to change mute" : state.muted ? "Unmute" : "Mute"}><button type="button" className={`voice-icon-button ${state.muted ? "active" : ""}`} aria-disabled={state.monitoring} aria-label={state.muted ? "Unmute microphone" : "Mute microphone"} aria-pressed={state.muted} onClick={() => { if (state.monitoring) return; const muted = !state.muted; playSound(muted ? "toggle-off" : "toggle-on"); setActionError(undefined); void clientRef.current!.setMuted(muted).catch((error) => setActionError(error instanceof Error ? error.message : "Mute state could not be shared.")); }}>{state.muted ? <MicOff aria-hidden="true" /> : <Mic aria-hidden="true" />}</button></Tooltip>
              <AudioMenu label="Input Options" open={audioMenu === "input"} onOpenChange={(open) => setAudioMenu(open ? "input" : undefined)} menuRef={audioMenu === "input" ? audioMenuRef : undefined}>
                <fieldset className="device-options" disabled={controlsDisabled}>
                  <legend>Microphone</legend>
                  {deviceOptions(devices, "audioinput").map(({ device, label }) => <label key={device.deviceId}><input type="radio" name="input-device" value={device.deviceId} checked={deviceId === device.deviceId} onChange={() => { if (connected) void act(() => clientRef.current!.changeMicrophone(device.deviceId), () => setDeviceId(device.deviceId)); else setDeviceId(device.deviceId); }} /><span>{label}</span></label>)}
                  {!deviceOptions(devices, "audioinput").length && <p>System default · test your mic to see available devices.</p>}
                </fieldset>
                <div className="volume-control output-volume"><span>Input volume <output>{state.inputVolume}%</output></span><Slider label="Input volume" value={state.inputVolume} max={200} onChange={(value) => clientRef.current?.setInputVolume(value)} /></div>
              </AudioMenu>
            </div>
            <div className={`voice-action-group${state.deafened ? " active" : ""}`}>
              <Tooltip content={state.monitoring ? "Stop mic test to change deafen" : state.deafened ? "Undeafen" : "Deafen"}><button type="button" className={`voice-icon-button ${state.deafened ? "active" : ""}`} aria-disabled={state.monitoring} aria-label={state.deafened ? "Undeafen audio" : "Deafen audio"} aria-pressed={state.deafened} onClick={() => { if (state.monitoring) return; const deafened = !state.deafened; playSound(deafened ? "toggle-off" : "toggle-on"); setActionError(undefined); void clientRef.current!.setDeafened(deafened).catch((error) => setActionError(error instanceof Error ? error.message : "Deafen state could not be shared.")); }}>{state.deafened ? <VolumeX aria-hidden="true" /> : <Headphones aria-hidden="true" />}</button></Tooltip>
              <AudioMenu label="Output Options" open={audioMenu === "output"} onOpenChange={(open) => setAudioMenu(open ? "output" : undefined)} menuRef={audioMenu === "output" ? audioMenuRef : undefined}>
                {outputSelectable ? <fieldset className="device-options">
                  <legend>Audio output</legend>
                  {deviceOptions(devices, "audiooutput").map(({ device, label }) => <label key={device.deviceId}><input type="radio" name="output-device" value={device.deviceId} checked={output === device.deviceId} onChange={() => setOutput(device.deviceId)} /><span>{label}</span></label>)}
                  {!deviceOptions(devices, "audiooutput").length && <p>System default · test your mic to see available devices.</p>}
                </fieldset> : <p className="noise-status">Choose audio output in system settings.</p>}
                <div className="volume-control output-volume"><span>Output volume <output>{outputVolume}%</output></span><Slider label="Output volume" value={outputVolume} max={200} onChange={setOutputVolume} /></div>
              </AudioMenu>
            </div>
            <AudioMenu label="User Settings" settings open={audioMenu === "settings"} onOpenChange={(open) => setAudioMenu(open ? "settings" : undefined)} menuRef={audioMenu === "settings" ? audioMenuRef : undefined}>
              <strong>Audio settings</strong>
              <fieldset className="device-options">
                <label><input type="checkbox" role="switch" checked={systemSounds} onChange={(event) => { setSystemSoundsEnabled(event.target.checked); if (event.target.checked) { void preloadSoundEffects(); playSound("toggle-on"); } }} /><span>Caper sound effects</span></label>
              </fieldset>
              <button disabled={!identityReady || controlsDisabled || state.phase === "leaving"} type="button" onClick={openMicTest}>Mic test</button>
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
      <dialog ref={audioDialog} className="audio-dialog" aria-labelledby="audio-dialog-title" onCancel={(event) => { event.preventDefault(); closeAudioPanel(); }}>
        <div className="audio-dialog-heading">
          <h2 id="audio-dialog-title">{audioPanel === "mic" ? "Mic test" : audioPanel === "debug" ? "Audio diagnostics" : "Connection details"}</h2>
          <button type="button" className="voice-icon-button" aria-label="Close audio settings" onClick={closeAudioPanel}><X aria-hidden="true" /></button>
        </div>
        {audioPanel && (actionError || voiceError) && <p className="call-error" role="alert">{actionError || voiceError}</p>}
        {audioPanel === "mic" && <>
          {actionPending && <p className="noise-status" role="status">Preparing microphone…</p>}
          {actionPending && <p className="noise-status">Allow microphone access if your browser asks. You can close this window to cancel.</p>}
          {!actionPending && actionError && !state.monitorStream && <button type="button" className="voice-button" onClick={openMicTest}>Try again</button>}
          {state.monitorStream && !actionPending && <MicPlayback key={`${state.noiseSuppression}:${deviceId}`} stream={state.monitorStream} output={output} volume={outputVolume} noiseStatus={state.noiseSuppressionStatus} processingStrength={state.voiceProcessingStrength ?? DEFAULT_VOICE_PROCESSING_STRENGTH} onProcessingStrengthChange={(strength) => clientRef.current?.setVoiceProcessingStrength(strength)} />}
          {account?.debugEnabled && <details><summary>Audio diagnostics</summary><AudioDiagnostics client={clientRef.current} /></details>}
        </>}
        {audioPanel === "debug" && account?.debugEnabled && <AudioDiagnostics client={clientRef.current} />}
        {audioPanel === "connection" && (state.diagnostics ? <ConnectionDiagnostics diagnostics={state.diagnostics} /> : <p className="noise-status">Join voice to see connection details.</p>)}
      </dialog>
    </Root>
  );
}
