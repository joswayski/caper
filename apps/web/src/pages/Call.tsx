import { useEffect, useRef, useState } from "react";
import { acquireAudioContext, releaseAudioContext } from "../media/audio-context";
import { PublicCallClient } from "../media/client";
import type { CallViewState } from "../media/types";
import MicPlayback from "./MicPlayback";
import VoiceActivity from "./VoiceActivity";
import "./call.css";

const initialState: CallViewState = { phase: "idle", muted: false, deafened: false, monitoring: false, participants: [], remoteMedia: [] };
const regionNames = new Intl.DisplayNames(["en"], { type: "region" });

function ParticipantCountry({ code }: { code?: string }) {
  if (!code || !/^[A-Z]{2}$/.test(code)) return null;
  const name = regionNames.of(code) ?? code;
  const flag = String.fromCodePoint(...[...code].map((letter) => 0x1f1e6 + letter.charCodeAt(0) - 65));
  return <span className="participant-country" aria-label={`From ${name}`} title={name}>{flag}</span>;
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
  const clientRef = useRef<PublicCallClient | undefined>(undefined);
  if (!clientRef.current && typeof window !== "undefined") clientRef.current = new PublicCallClient(setState);
  const connected = state.phase === "connected";
  const idle = state.phase === "idle" || state.phase === "failed" || state.phase === "leaving";
  const controlsDisabled = !connected || actionPending || state.monitorConnecting;

  useEffect(() => {
    let current = true;
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
    if (!connected) return;
    const update = () => void navigator.mediaDevices.enumerateDevices().then(setDevices).catch(() => setActionError("Device list unavailable. Use system settings."));
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
        <a className="wordmark" href="/">caper</a>
      </header>
      <section className="call-room">
        <aside className="people-panel">
          <div className="panel-heading"><div><p className="eyebrow">Caper</p><h1>Voice channel</h1></div></div>
          <div className="voice-channel"><span aria-hidden="true">◖))</span> General {connected && <small aria-label={`${state.participants.length} in voice`}>{state.participants.length}</small>}</div>
          <ul className={volumeParticipant ? "volume-menu-open" : undefined} aria-label="People in voice">
            {state.participants.map((participant) => {
              const self = participant.id === state.selfId;
              const speaking = activeParticipants.has(participant.id);
              const stream = self ? state.localMedia : state.remoteMedia.find((media) => media.participantId === participant.id)?.stream;
              const participantMuted = self ? state.muted : participant.muted;
              const participantDeafened = self ? state.deafened : participant.deafened;
              const activityMuted = participantMuted && !state.monitoring;
              return <li className={`participant ${volumeParticipant === participant.id ? "volume-open" : ""}`} key={participant.id} onContextMenu={self ? undefined : (event) => { event.preventDefault(); setVolumeParticipant(participant.id); }}>
                <span className="participant-avatar">
                  <span className={`avatar ${speaking ? "speaking" : "quiet"}`} aria-hidden="true">{participant.name.slice(0, 1).toUpperCase()}</span>
                  <ParticipantCountry code={participant.countryCode} />
                </span>
                <span className="participant-name"><strong>{participant.name}{self ? " (you)" : ""}</strong>{participantDeafened ? <small>Deafened</small> : participantMuted ? <small>Muted</small> : null}</span>
                <VoiceActivity
                  stream={stream}
                  muted={activityMuted}
                  onActivityChange={(active) => setActiveParticipants((current) => {
                    if (current.has(participant.id) === active) return current;
                    const next = new Set(current);
                    active ? next.add(participant.id) : next.delete(participant.id);
                    return next;
                  })}
                />
                {!self && <button className="participant-menu-button" type="button" aria-label={`Volume for ${participant.name}`} aria-expanded={volumeParticipant === participant.id} onClick={() => setVolumeParticipant((current) => current === participant.id ? undefined : participant.id)}>•••</button>}
                {volumeParticipant === participant.id && <div className="participant-volume" role="group" aria-label={`${participant.name} local audio settings`}>
                  <div><strong>User volume</strong><output>{participantVolumes[participant.id] ?? 100}%</output></div>
                  <input
                    type="range"
                    min="0"
                    max="200"
                    step="5"
                    value={participantVolumes[participant.id] ?? 100}
                    aria-label={`${participant.name} volume`}
                    onChange={(event) => setParticipantVolumes((current) => ({ ...current, [participant.id]: Number(event.target.value) }))}
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
          {idle && <p className="roster-note">Join to see who’s here.</p>}
        </aside>
        <div className="stage">
          <div className="stage-title"><div><p className="eyebrow">One channel, open to everyone</p><h2>General</h2></div>{!idle && !connected && <p role="status">{state.phase === "joining" ? "Joining…" : state.phase === "reconnecting" ? "Reconnecting…" : "Leaving…"}</p>}</div>
          {idle ? <div className="join-card">
            <span className="voice-symbol" aria-hidden="true">◖))</span>
            <h1>Drop in. Talk. Head out.</h1>
            <p>One shared voice channel. No invites, accounts, or ringing anyone.</p>
            <p>You’ll get a random nickname when you join. Everyone sees the same name.</p>
            <p>Use headphones; echo cancellation is off. You can choose your microphone and output after joining.</p>
            {available === false ? <p className="call-error" role="alert">Voice is currently unavailable. Please try again later.</p> : <form onSubmit={(event) => { event.preventDefault(); void clientRef.current?.join("", deviceId || undefined); }}>
              <button className="primary-button" disabled={available !== true || state.phase === "leaving"} type="submit">{state.phase === "leaving" ? "Leaving voice…" : available === undefined ? "Checking voice…" : "Join voice"}</button>
            </form>}
            <div className="privacy-note">Your browser will ask for microphone access. Everyone in this public channel can hear you and see your approximate country, inferred from your IP address. Caper does not store your IP address. Mic test can keep a brief recording temporarily in this browser; Caper does not store recordings on its servers. Other visitors may record. Not end-to-end encrypted.</div>
          </div> : state.monitorStream && !actionPending
            ? <MicPlayback key={`${state.noiseSuppression}:${deviceId}`} stream={state.monitorStream} output={output} status={state.monitorStatus} />
            : <div className="stage-placeholder"><span aria-hidden="true">◖))</span><h3>{state.monitoring ? "Starting microphone test…" : connected ? "You’re in General." : "Connecting to voice…"}</h3><p>{state.monitoring ? "Creating a private return through the call service. Press Record when it’s ready." : connected ? "Your microphone is live unless muted. Stay as long as you like; leave whenever." : "Setting up your microphone and connection."}</p>{state.monitorStatus && <p role="status">{state.monitorStatus}</p>}{state.phase === "joining" && <button onClick={leave}>Cancel</button>}</div>}
          {state.remoteMedia.map((media) => <AudioOutput key={media.trackId} stream={media.stream} muted={state.deafened || mutedParticipants.has(media.participantId)} output={output} volume={participantVolumes[media.participantId] ?? 100} name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"} />)}
          {connected && actionPending && <p className="noise-status" role="status">Applying microphone settings… Record a new test once ready.</p>}
          {(state.error || actionError) && <p className="call-error room-error" role="alert">{state.error || actionError}</p>}
          {!idle && <p className="noise-status">Use headphones · natural input, without browser echo cancellation or automatic volume adjustment.</p>}
          {state.noiseSuppressionStatus && <p className="noise-status" role="status">{state.noiseSuppressionStatus}</p>}
          {state.diagnostics && <details className="call-diagnostics"><summary>Connection diagnostics</summary><p>{state.diagnostics}</p><small>Local estimates, not billing totals. Counters reset on reconnect.</small></details>}
        </div>
        {!idle && <footer className="call-controls" aria-label="Voice controls">
          <label className="device-control"><span>Microphone</span><select disabled={controlsDisabled} value={deviceId} onChange={(event) => { const value = event.target.value; void act(() => clientRef.current!.changeMicrophone(value), () => setDeviceId(value)); }}><option value="">System default</option>{devices.filter((device) => device.kind === "audioinput").map((device) => <option value={device.deviceId} key={device.deviceId}>{device.label || "Microphone"}</option>)}</select></label>
          {typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype ? <label className="device-control"><span>Audio output</span><select value={output} onChange={(event) => setOutput(event.target.value)}><option value="">System default</option>{devices.filter((device) => device.kind === "audiooutput").map((device) => <option value={device.deviceId} key={device.deviceId}>{device.label || "Audio output"}</option>)}</select></label> : <p className="noise-status">Choose your audio output in system settings; this browser cannot switch outputs.</p>}
          <div className="control-buttons">
            <button disabled={!connected || state.monitoring} type="button" className={state.muted ? "active" : ""} aria-pressed={state.muted} onClick={() => { setActionError(undefined); void clientRef.current!.setMuted(!state.muted).catch((error) => setActionError(error instanceof Error ? error.message : "Mute state could not be shared.")); }}>{state.muted ? "Unmute" : "Mute"}</button>
            <button disabled={!connected || state.monitoring} type="button" className={state.deafened ? "active" : ""} aria-pressed={state.deafened} onClick={() => { setActionError(undefined); void clientRef.current!.setDeafened(!state.deafened).catch((error) => setActionError(error instanceof Error ? error.message : "Deafen state could not be shared.")); }}>{state.deafened ? "Listen" : "Deafen"}</button>
            <button disabled={!connected || (!state.monitoring && actionPending)} type="button" className={state.monitoring ? "active" : ""} aria-pressed={state.monitoring} onClick={() => { setActionError(undefined); void clientRef.current!.setMonitoring(!state.monitoring).catch((error) => setActionError(error instanceof Error ? error.message : "Mic test could not start.")); }}>{state.monitoring ? "End mic test" : "Mic test"}</button>
            <button type="button" className="leave-button" onClick={leave}>Leave voice</button>
          </div>
        </footer>}
      </section>
    </main>
  );
}
