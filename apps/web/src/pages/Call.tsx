import { useEffect, useRef, useState } from "react";
import { animals, colors, uniqueNamesGenerator } from "unique-names-generator";
import { Headphones, Mic, MicOff, Volume2, VolumeX } from "lucide-react";
import AccountNav from "../account/AccountNav";
import { getAccount } from "../account/client";
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
const flags = import.meta.glob<string>("../../../../node_modules/flag-icons/flags/4x3/*.svg", { eager: true, import: "default", query: "?url" });

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
  return <details className="call-diagnostics">
    <summary><span>Connection details</span><small>For troubleshooting</small></summary>
    <dl>{values.map(([label, value]) => <div key={label}><dt>{label}</dt><dd>{value}</dd></div>)}</dl>
    <p>Local estimates, not billing totals. Counters reset on reconnect.</p>
  </details>;
}

function ParticipantCountry({ code }: { code?: string }) {
  if (!code || !/^[A-Z]{2}$/.test(code)) return null;
  const name = regionNames.of(code) ?? code;
  const source = flags[`../../../../node_modules/flag-icons/flags/4x3/${code.toLowerCase()}.svg`];
  if (!source) return null;
  return <img className="participant-country" src={source} alt={`From ${name}`} title={name} />;
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
  const [accountDisplayName, setAccountDisplayName] = useState<string>();
  const [identityReady, setIdentityReady] = useState(false);
  const [available, setAvailable] = useState<boolean>();
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
  const controlsDisabled = !connected || actionPending;
  const roster = idle ? publicParticipants : state.participants;

  useEffect(() => {
    let current = true;
    setName(uniqueNamesGenerator({ dictionaries: [colors, animals], separator: " ", style: "capital" }));
    void getAccount()
      .then((account) => {
        if (!current || !account?.displayName) return;
        setName(account.displayName);
        setAccountDisplayName(account.displayName);
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
    if (!connected) return;
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
  }, [connected]);

  const act = async (operation: () => Promise<unknown>, success?: () => void) => {
    setActionError(undefined);
    setActionPending(true);
    try { await operation(); success?.(); } catch (error) {
      setActionError(error instanceof Error ? error.message : "That action did not work.");
    } finally { setActionPending(false); }
  };
  const leave = () => void act(() => clientRef.current!.leave());

  return (
    <main className="call-page">
      <header className="call-header">
        <a className="wordmark" href="/">caper<span className="wordmark-dot">.</span></a>
        <AccountNav />
      </header>
      <section className="call-room">
        <aside className="people-panel">
          <div className="panel-heading"><div><p className="eyebrow">Caper</p><h1>Voice channel</h1></div></div>
          <div className="voice-channel"><span aria-hidden="true">◖))</span> General {roster.length > 0 && <small aria-label={`${roster.length} in voice`}>{roster.length}</small>}</div>
          <ul className={volumeParticipant ? "volume-menu-open" : undefined} aria-label="People in voice">
            {roster.map((participant) => {
              const self = participant.id === state.selfId;
              const speaking = activeParticipants.has(participant.id);
              const stream = self ? state.localMedia : state.remoteMedia.find((media) => media.participantId === participant.id)?.stream;
              const participantMuted = self ? state.muted : participant.muted;
              const participantDeafened = self ? state.deafened : participant.deafened;
              const activityMuted = participantMuted && !state.monitoring;
              return <li className={`participant ${volumeParticipant === participant.id ? "volume-open" : ""}`} key={participant.id} onContextMenu={idle || self ? undefined : (event) => { event.preventDefault(); setVolumeParticipant(participant.id); }}>
                <span className="participant-avatar">
                  <span className={`avatar ${speaking ? "speaking" : "quiet"}`} aria-hidden="true">{participant.name.slice(0, 1).toUpperCase()}</span>
                  <ParticipantCountry code={participant.countryCode} />
                </span>
                <span className="participant-name"><strong>{participant.name}{self ? " (you)" : ""}</strong>{participantDeafened ? <small>Deafened</small> : participantMuted ? <small>Muted</small> : null}</span>
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
        </aside>
        <div className="stage">
          <div className="stage-title"><div><h2>General</h2></div>{!idle && !connected && <p role="status">{state.phase === "joining" ? "Joining…" : state.phase === "reconnecting" ? "Reconnecting…" : "Leaving…"}</p>}</div>
          {state.monitorStream && !actionPending
            ? <MicPlayback key={`${state.noiseSuppression}:${deviceId}`} stream={state.monitorStream} output={output} processingStrength={state.voiceProcessingStrength ?? DEFAULT_VOICE_PROCESSING_STRENGTH} onProcessingStrengthChange={(strength) => clientRef.current?.setVoiceProcessingStrength(strength)} onClose={idle ? () => clientRef.current?.stopLocalMicTest() : undefined} />
            : idle ? <div className="join-card">
            <span className="voice-symbol" aria-hidden="true">◖))</span>
            <h1>Drop in. Talk. Head out.</h1>
            <p>{accountDisplayName ? "Join the shared General channel using your Caper display name." : "Join the shared General channel as a guest. No account or invite needed."}</p>
            {available === false ? <p className="call-error" role="alert">Voice is currently unavailable. Please try again later.</p> : <form onSubmit={(event) => { event.preventDefault(); void clientRef.current?.join(name.trim(), deviceId || undefined); }}>
              <p>Joining as <strong>{identityReady ? name : "…"}</strong></p>
              <button className="primary-button" disabled={available !== true || !identityReady || !name.trim() || state.phase === "leaving"} type="submit">{state.phase === "leaving" ? "Leaving voice…" : available === undefined ? "Checking voice…" : !identityReady ? "Checking profile…" : "Join voice"}</button>
            </form>}
            <button className="mic-test-link" disabled={!identityReady} type="button" onClick={() => { setActionError(undefined); void act(() => clientRef.current!.startLocalMicTest()); }}>Test your mic first</button>
            <p className="privacy-note">You’ll be asked for microphone access when you join.</p>
          </div>
            : <div className="stage-placeholder"><span aria-hidden="true">◖))</span><h3>{connected ? "You’re in General." : "Connecting to voice…"}</h3><p>{connected ? "Say hello, or run a mic test to hear yourself first." : "Getting everything ready."}</p>{state.phase === "joining" && <button onClick={leave}>Cancel</button>}</div>}
          {state.remoteMedia.map((media) => <AudioOutput key={media.trackId} stream={media.stream} muted={state.deafened || mutedParticipants.has(media.participantId)} output={output} volume={participantVolumes[media.participantId] ?? 100} name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"} />)}
          {connected && actionPending && <p className="noise-status" role="status">Applying microphone settings… Record a new test once ready.</p>}
          {(state.error || actionError) && <p className="call-error room-error" role="alert">{state.error || actionError}</p>}
          {state.diagnostics && <ConnectionDiagnostics diagnostics={state.diagnostics} />}
        </div>
        {!idle && <footer className="call-controls" aria-label="Voice controls">
          <div className="voice-action-group">
            <button disabled={!connected || state.monitoring} type="button" className={`voice-icon-button ${state.muted ? "active" : ""}`} aria-label={state.muted ? "Unmute microphone" : "Mute microphone"} aria-pressed={state.muted} title={state.muted ? "Unmute" : "Mute"} onClick={() => { setActionError(undefined); void clientRef.current!.setMuted(!state.muted).catch((error) => setActionError(error instanceof Error ? error.message : "Mute state could not be shared.")); }}>{state.muted ? <MicOff aria-hidden="true" /> : <Mic aria-hidden="true" />}</button>
            <label className="device-select"><span className="sr-only">Microphone</span><select aria-label="Microphone" disabled={controlsDisabled || !deviceOptions(devices, "audioinput").length} value={deviceId} onChange={(event) => { const value = event.target.value; void act(() => clientRef.current!.changeMicrophone(value), () => setDeviceId(value)); }}>{!deviceOptions(devices, "audioinput").length && <option value="">Loading…</option>}{deviceOptions(devices, "audioinput").map(({ device, label }) => <option value={device.deviceId} key={device.deviceId}>{label}</option>)}</select></label>
          </div>
          <div className="voice-action-group">
            <button disabled={!connected || state.monitoring} type="button" className={`voice-icon-button ${state.deafened ? "active" : ""}`} aria-label={state.deafened ? "Undeafen audio" : "Deafen audio"} aria-pressed={state.deafened} title={state.deafened ? "Listen" : "Deafen"} onClick={() => { setActionError(undefined); void clientRef.current!.setDeafened(!state.deafened).catch((error) => setActionError(error instanceof Error ? error.message : "Deafen state could not be shared.")); }}>{state.deafened ? <VolumeX aria-hidden="true" /> : <Headphones aria-hidden="true" />}</button>
            {typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype ? <label className="device-select"><span className="sr-only">Audio output</span><select aria-label="Audio output" disabled={!deviceOptions(devices, "audiooutput").length} value={output} onChange={(event) => setOutput(event.target.value)}>{!deviceOptions(devices, "audiooutput").length && <option value="">Loading…</option>}{deviceOptions(devices, "audiooutput").map(({ device, label }) => <option value={device.deviceId} key={device.deviceId}>{label}</option>)}</select></label> : <p className="noise-status">Choose audio output in system settings.</p>}
          </div>
          <div className="volume-control"><span>My voice level <output>{state.inputVolume}%</output></span><Slider label="My voice level" value={state.inputVolume} max={200} onChange={(value) => clientRef.current?.setInputVolume(value)} /><small>Changes how loud you sound to others.</small></div>
          <div className="control-buttons">
            <button disabled={!connected || (!state.monitoring && actionPending)} type="button" className={state.monitoring ? "active" : ""} aria-pressed={state.monitoring} onClick={() => { setActionError(undefined); void clientRef.current!.setMonitoring(!state.monitoring).catch((error) => setActionError(error instanceof Error ? error.message : "Mic test could not start.")); }}><Volume2 aria-hidden="true" /> <span>{state.monitoring ? "End mic test" : "Mic test"}</span></button>
            <button type="button" className="leave-button" onClick={leave}>Leave voice</button>
          </div>
        </footer>}
      </section>
    </main>
  );
}
