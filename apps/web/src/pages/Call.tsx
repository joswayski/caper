import { useEffect, useRef, useState, type ReactNode } from "react";
import { animals, colors, uniqueNamesGenerator } from "unique-names-generator";
import { ChevronDown, Hash, Headphones, Mic, MicOff, Speech, Settings2, VolumeX, X } from "lucide-react";
import AccountNav from "../account/AccountNav";
import { getAccount, type Account } from "../account/client";
import Chat from "../chat/Chat";
import type { ChatAuthor } from "../chat/types";
import Slider from "../components/Slider";
import { acquireAudioContext, releaseAudioContext } from "../media/audio-context";
import { PublicCallClient } from "../media/client";
import { watchPresence } from "../media/presence";
import type { CallViewState, Participant } from "../media/types";
import { DEFAULT_VOICE_PROCESSING_STRENGTH } from "../media/voice-processing";
import MicPlayback from "./MicPlayback";
import VoiceActivity from "./VoiceActivity";
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

function AudioMenu({ label, settings, children }: { label: string; settings?: boolean; children: ReactNode }) {
  const ref = useRef<HTMLDetailsElement>(null);
  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) ref.current.open = false;
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, []);
  return <details ref={ref} className={`call-settings ${settings ? "" : "device-menu"}`} onKeyDown={(event) => {
    if (event.key === "Escape") { event.currentTarget.open = false; event.currentTarget.querySelector("summary")?.focus(); }
  }} onBlur={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false; }}>
    <summary aria-label={label} title={label}>{settings ? <Settings2 aria-hidden="true" /> : <ChevronDown aria-hidden="true" />}</summary>
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

export default function Call() {
  const [state, setState] = useState(initialState);
  const [name, setName] = useState("");
  const [account, setAccount] = useState<Account | null>(null);
  const [chatAuthor, setChatAuthor] = useState<ChatAuthor>();
  const [identityReady, setIdentityReady] = useState(false);
  const [available, setAvailable] = useState<boolean>();
  const [joinTooltipDismissed, setJoinTooltipDismissed] = useState(false);
  const [audioPanel, setAudioPanel] = useState<"mic" | "connection">();
  const audioDialog = useRef<HTMLDialogElement>(null);
  const audioReturnFocus = useRef<HTMLElement | null>(null);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = useState("");
  const [output, setOutput] = useState("");
  const [actionError, setActionError] = useState<string>();
  const [actionPending, setActionPending] = useState(false);
  const [activeParticipants, setActiveParticipants] = useState<Set<string>>(() => new Set());
  const [participantVolumes, setParticipantVolumes] = useState<Record<string, number>>({});
  const [mutedParticipants, setMutedParticipants] = useState<Set<string>>(() => new Set());
  const [volumeParticipant, setVolumeParticipant] = useState<string>();
  const [publicParticipants, setPublicParticipants] = useState<PublicPresence["participants"]>([]);
  const clientRef = useRef<PublicCallClient | undefined>(undefined);
  if (!clientRef.current && typeof window !== "undefined") clientRef.current = new PublicCallClient(setState);
  const connected = state.phase === "connected";
  const idle = state.phase === "idle" || state.phase === "failed" || state.phase === "leaving";
  const joinDisabled = !identityReady || state.phase === "leaving" || (idle && (available !== true || actionPending));
  const joinUnavailable = idle && available !== true;
  const controlsDisabled = (!idle && !connected) || actionPending;
  const roster = idle ? publicParticipants : state.participants;
  const identityName = account?.displayName || chatAuthor?.name || name;

  useEffect(() => {
    let current = true;
    setName(uniqueNamesGenerator({ dictionaries: [colors, animals], separator: " ", style: "capital" }));
    void getAccount()
      .then((account) => {
        if (!current) return;
        setAccount(account);
        if (account?.displayName) setName(account.displayName);
      })
      .catch(() => undefined)
      .finally(() => { if (current) setIdentityReady(true); });
    fetch("/api/media/status", { signal: AbortSignal.timeout(10_000) })
      .then(async (response) => response.ok ? response.json() as Promise<{ enabled: boolean }> : { enabled: false })
      .then((result) => {
        if (!current) return;
        setAvailable(result.enabled);
        if (result.enabled) clientRef.current?.prepareMicrophone();
      })
      .catch(() => { if (current) setAvailable(false); });
    const unload = () => clientRef.current?.leaveImmediately();
    window.addEventListener("pagehide", unload);
    return () => { current = false; window.removeEventListener("pagehide", unload); clientRef.current?.leaveImmediately(); };
  }, []);

  useEffect(() => {
    if (!idle || available !== true) return;
    return watchPresence((snapshot) => setPublicParticipants(snapshot.participants), () => undefined);
  }, [idle, available]);

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
    setActionError(undefined);
    setActionPending(true);
    try { await operation(); success?.(); } catch (error) {
      setActionError(error instanceof Error ? error.message : "That action did not work.");
    } finally { setActionPending(false); }
  };
  const leave = () => void act(() => clientRef.current!.leave());
  const closeAudioPanel = () => {
    if (audioPanel === "mic") {
      clientRef.current?.stopLocalMicTest();
      if (state.monitoring) void act(() => clientRef.current!.setMonitoring(false));
    }
    setAudioPanel(undefined);
  };
  const openMicTest = () => {
    setAudioPanel("mic");
    void act(() => connected ? clientRef.current!.setMonitoring(true) : clientRef.current!.startLocalMicTest(deviceId));
  };

  return (
    <main className="call-page">
      <header className="call-header">
        <a className="wordmark" href="/">caper<span className="wordmark-dot">.</span></a>
      </header>
      <section className="call-room">
        <aside className="people-panel">
          <div className="sidebar-channels">
          <div className="panel-heading"><h1>Channels</h1></div>
          <a className="channel-link" href="#chat-heading" aria-current="location"><Hash aria-hidden="true" /><span>general</span></a>
          {roster.length > 0 && <p className="voice-roster-label">In voice · {roster.length}</p>}
          <ul className={volumeParticipant ? "volume-menu-open" : undefined} aria-label="People talking in general">
            {roster.map((participant) => {
              const self = participant.id === state.selfId;
              const stream = self ? state.localMedia : state.remoteMedia.find((media) => media.participantId === participant.id)?.stream;
              const participantMuted = self ? state.muted : participant.muted;
              const participantDeafened = self ? state.deafened : participant.deafened;
              const participantStatus = participantDeafened ? "Deafened" : participantMuted ? "Muted" : undefined;
              const activityMuted = participantMuted && !state.monitoring;
              const speaking = activeParticipants.has(participant.id) && !activityMuted;
              return <li className={`participant ${volumeParticipant === participant.id ? "volume-open" : ""}`} key={participant.id} onContextMenu={idle || self ? undefined : (event) => { event.preventDefault(); setVolumeParticipant(participant.id); }}>
                <span className="participant-avatar">
                  <span className={`avatar ${speaking ? "speaking" : "quiet"}`} aria-hidden="true">{participant.name.slice(0, 1).toUpperCase()}</span>
                  <ParticipantCountry code={participant.countryCode} />
                </span>
                <span className="participant-name"><strong>{participant.name}{self ? " (you)" : ""}</strong><small aria-hidden={!participantStatus}>{participantStatus ?? "\u00a0"}</small></span>
                {!idle && <VoiceActivity
                  stream={stream}
                  muted={activityMuted}
                  onActivityChange={(active) => setActiveParticipants((current) => {
                    if (current.has(participant.id) === active) return current;
                    const next = new Set(current);
                    active ? next.add(participant.id) : next.delete(participant.id);
                    return next;
                  })}
                />}
                {!idle && !self && <button className="participant-menu-button" type="button" aria-label={`Audio controls for ${participant.name}`} aria-expanded={volumeParticipant === participant.id} onClick={() => setVolumeParticipant((current) => current === participant.id ? undefined : participant.id)}>Audio</button>}
                {!idle && volumeParticipant === participant.id && <div className="participant-volume" role="group" aria-label={`${participant.name} local audio settings`}>
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
                      onChange={(event) => setMutedParticipants((current) => {
                        const next = new Set(current);
                        event.target.checked ? next.add(participant.id) : next.delete(participant.id);
                        return next;
                      })}
                    />
                  </label>
                  <small>Only changes what you hear.</small>
                </div>}
              </li>;
            })}
          </ul>
          </div>
          <div className="call-account">
            <span className="account-avatar" aria-hidden="true">{identityName.slice(0, 1).toUpperCase()}</span>
            <strong className="account-name" title={identityName}>{identityName || "Loading…"}</strong>
            <div className="voice-action-group">
              <button disabled={!connected || state.monitoring} type="button" className={`voice-icon-button ${state.muted ? "active" : ""}`} aria-label={state.muted ? "Unmute microphone" : "Mute microphone"} aria-pressed={state.muted} title={!connected ? "Join voice to use your microphone" : state.muted ? "Unmute" : "Mute"} onClick={() => { setActionError(undefined); void clientRef.current!.setMuted(!state.muted).catch((error) => setActionError(error instanceof Error ? error.message : "Mute state could not be shared.")); }}>{state.muted ? <MicOff aria-hidden="true" /> : <Mic aria-hidden="true" />}</button>
              <AudioMenu label="Input options">
                <fieldset className="device-options" disabled={controlsDisabled}>
                  <legend>Microphone</legend>
                  {deviceOptions(devices, "audioinput").map(({ device, label }) => <label key={device.deviceId}><input type="radio" name="input-device" value={device.deviceId} checked={deviceId === device.deviceId} onChange={() => { if (connected) void act(() => clientRef.current!.changeMicrophone(device.deviceId), () => setDeviceId(device.deviceId)); else setDeviceId(device.deviceId); }} /><span>{label}</span></label>)}
                  {!deviceOptions(devices, "audioinput").length && <p>System default · test your mic to see available devices.</p>}
                </fieldset>
              </AudioMenu>
            </div>
            <div className="voice-action-group">
              <button disabled={!connected || state.monitoring} type="button" className={`voice-icon-button ${state.deafened ? "active" : ""}`} aria-label={state.deafened ? "Undeafen audio" : "Deafen audio"} aria-pressed={state.deafened} title={!connected ? "Join voice to control its audio" : state.deafened ? "Listen" : "Deafen"} onClick={() => { setActionError(undefined); void clientRef.current!.setDeafened(!state.deafened).catch((error) => setActionError(error instanceof Error ? error.message : "Deafen state could not be shared.")); }}>{state.deafened ? <VolumeX aria-hidden="true" /> : <Headphones aria-hidden="true" />}</button>
              <AudioMenu label="Output options">
                {typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype ? <fieldset className="device-options">
                  <legend>Audio output</legend>
                  {deviceOptions(devices, "audiooutput").map(({ device, label }) => <label key={device.deviceId}><input type="radio" name="output-device" value={device.deviceId} checked={output === device.deviceId} onChange={() => setOutput(device.deviceId)} /><span>{label}</span></label>)}
                  {!deviceOptions(devices, "audiooutput").length && <p>System default · test your mic to see available devices.</p>}
                </fieldset> : <p className="noise-status">Choose audio output in system settings.</p>}
              </AudioMenu>
            </div>
            <AudioMenu label="Settings" settings>
              <strong>Audio settings</strong>
              <div className="volume-control"><span>My voice level <output>{state.inputVolume}%</output></span><Slider label="My voice level" value={state.inputVolume} max={200} onChange={(value) => clientRef.current?.setInputVolume(value)} /><small>Changes how loud you sound to others.</small></div>
              <button disabled={!identityReady || controlsDisabled || state.phase === "leaving"} type="button" onClick={openMicTest}>Mic test</button>
              {state.diagnostics && <button type="button" onClick={() => setAudioPanel("connection")}>Connection details</button>}
              {identityReady && <AccountNav account={account} />}
            </AudioMenu>
          </div>
        </aside>
        <div className="stage">
          {state.remoteMedia.map((media) => <AudioOutput key={media.trackId} stream={media.stream} muted={state.deafened || mutedParticipants.has(media.participantId)} output={output} volume={participantVolumes[media.participantId] ?? 100} name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"} />)}
          {!audioPanel && (state.error || actionError) && <p className="call-error room-error" role="alert">{state.error || actionError}</p>}
          <Chat name={name} signedIn={!!account} identityReady={identityReady} onAuthorChange={setChatAuthor} headerActions={<div className="voice-actions">
            <span className="voice-join" data-tooltip-dismissed={joinTooltipDismissed} onMouseLeave={() => setJoinTooltipDismissed(false)} onBlur={() => setJoinTooltipDismissed(false)} onKeyDown={(event) => {
              if (event.key === "Escape") setJoinTooltipDismissed(true);
            }}>
              <button className="voice-button" type="button" aria-label={connected ? "Leave voice" : !idle ? "Cancel joining voice" : state.phase === "leaving" ? "Leaving voice" : "Join voice"} aria-describedby={joinUnavailable ? "voice-availability" : undefined} aria-disabled={joinDisabled} onClick={() => {
                if (joinDisabled) return;
                if (!idle) { leave(); return; }
                setActionError(undefined);
                void clientRef.current?.join(name.trim(), deviceId);
              }}><Speech aria-hidden="true" />{connected ? "Leave" : !idle ? "Cancel" : state.phase === "leaving" ? "Leaving…" : "Join"}</button>
              {joinUnavailable && <span id="voice-availability" className="voice-tooltip" role="tooltip">{available === false ? "Joining is not available at this time." : "Checking voice availability…"}</span>}
            </span>
            {!idle && <span className="voice-status" role="status">{connected ? "Voice connected" : state.phase === "joining" ? "Joining…" : "Reconnecting…"}</span>}
          </div>} />
        </div>
      </section>
      <dialog ref={audioDialog} className="audio-dialog" aria-labelledby="audio-dialog-title" onCancel={(event) => { event.preventDefault(); closeAudioPanel(); }}>
        <div className="audio-dialog-heading">
          <h2 id="audio-dialog-title">{audioPanel === "mic" ? "Mic test" : "Connection details"}</h2>
          <button type="button" className="voice-icon-button" aria-label="Close audio settings" onClick={closeAudioPanel}><X aria-hidden="true" /></button>
        </div>
        {(state.error || actionError) && <p className="call-error" role="alert">{state.error || actionError}</p>}
        {audioPanel === "mic" && <>
          {actionPending && <p className="noise-status" role="status">Preparing microphone…</p>}
          {state.monitorStream && !actionPending && <MicPlayback key={`${state.noiseSuppression}:${deviceId}`} stream={state.monitorStream} output={output} processingStrength={state.voiceProcessingStrength ?? DEFAULT_VOICE_PROCESSING_STRENGTH} onProcessingStrengthChange={(strength) => clientRef.current?.setVoiceProcessingStrength(strength)} />}
        </>}
        {audioPanel === "connection" && (state.diagnostics ? <ConnectionDiagnostics diagnostics={state.diagnostics} /> : <p className="noise-status">Join voice to see connection details.</p>)}
      </dialog>
    </main>
  );
}
