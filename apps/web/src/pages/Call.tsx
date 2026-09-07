import { useEffect, useRef, useState } from "react";
import { PublicCallClient } from "../media/client";
import type { NoiseSuppression } from "../media/microphone";
import type { CallViewState } from "../media/types";
import MicPlayback from "./MicPlayback";
import VoiceWaveform from "./VoiceWaveform";
import "./call.css";

const initialState: CallViewState = { phase: "idle", muted: false, deafened: false, monitoring: false, participants: [], remoteMedia: [] };

function AudioOutput({ stream, muted, name, output }: { stream: MediaStream; muted: boolean; name: string; output: string }) {
  const ref = useRef<HTMLAudioElement>(null);
  const [blocked, setBlocked] = useState(false);
  const [deviceError, setDeviceError] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (element) {
      element.srcObject = stream;
      void element.play().catch(() => setBlocked(true));
    }
    return () => { if (element) element.srcObject = null; };
  }, [stream]);
  useEffect(() => {
    if (ref.current?.setSinkId) void ref.current.setSinkId(output).then(() => setDeviceError(false)).catch(() => setDeviceError(true));
  }, [output]);
  return <><audio ref={ref} autoPlay muted={muted} />
    {blocked && <button onClick={() => void ref.current?.play().then(() => setBlocked(false)).catch(() => setBlocked(true))}>Play {name} audio</button>}
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
          <ul aria-label="People in voice">
            {state.participants.map((participant) => {
              const self = participant.id === state.selfId;
              const speaking = activeParticipants.has(participant.id);
              const stream = self ? state.localMedia : state.remoteMedia.find((media) => media.participantId === participant.id)?.stream;
              const participantMuted = self ? state.muted : participant.muted;
              const participantDeafened = self ? state.deafened : participant.deafened;
              const waveformMuted = participantMuted && !state.monitoring;
              return <li className="participant" key={participant.id}>
                <span className={`avatar ${speaking ? "speaking" : "quiet"}`} aria-hidden="true">{participant.name.slice(0, 1).toUpperCase()}</span>
                <span className="participant-name"><strong>{participant.name}{self ? " (you)" : ""}</strong>{participantDeafened ? <small>Deafened</small> : participantMuted ? <small>Muted</small> : null}</span>
                <VoiceWaveform
                  stream={stream}
                  muted={waveformMuted}
                  label={`${participant.name} live audio level`}
                  onActivityChange={(active) => setActiveParticipants((current) => {
                    if (current.has(participant.id) === active) return current;
                    const next = new Set(current);
                    active ? next.add(participant.id) : next.delete(participant.id);
                    return next;
                  })}
                />
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
            <div className="privacy-note">Your browser will ask for microphone access. Everyone in this public channel can hear you. Mic test can keep a brief recording temporarily in this browser; Caper does not store recordings on its servers. Other visitors may record. Not end-to-end encrypted.</div>
          </div> : state.monitorStream && !actionPending
            ? <MicPlayback key={`${state.noiseSuppression}:${deviceId}`} stream={state.monitorStream} output={output} status={state.monitorStatus} />
            : <div className="stage-placeholder"><span aria-hidden="true">◖))</span><h3>{state.monitoring ? "Starting microphone test…" : connected ? "You’re in General." : "Connecting to voice…"}</h3><p>{state.monitoring ? "Creating a private return through the call service. Press Record when it’s ready." : connected ? "Your microphone is live unless muted. Stay as long as you like; leave whenever." : "Setting up your microphone and connection."}</p>{state.monitorStatus && <p role="status">{state.monitorStatus}</p>}{state.phase === "joining" && <button onClick={leave}>Cancel</button>}</div>}
          {state.remoteMedia.map((media) => <AudioOutput key={media.trackId} stream={media.stream} muted={state.deafened} output={output} name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"} />)}
          {connected && actionPending && <p className="noise-status" role="status">Applying microphone settings… Record a new test once ready.</p>}
          {(state.error || actionError) && <p className="call-error room-error" role="alert">{state.error || actionError}</p>}
          {!idle && <p className="noise-status">Use headphones · natural input, without browser echo cancellation or automatic volume adjustment.</p>}
          {state.noiseSuppressionStatus && <p className="noise-status" role="status">{state.noiseSuppressionStatus}</p>}
          {state.diagnostics && <details className="call-diagnostics"><summary>Connection diagnostics</summary><p>{state.diagnostics}</p><small>Local estimates, not billing totals. Counters reset on reconnect.</small></details>}
        </div>
        {!idle && <footer className="call-controls" aria-label="Voice controls">
          <label className="device-control"><span>Noise suppression</span><select disabled={controlsDisabled} value={state.noiseSuppression ?? "dpdfnet8"} onChange={(event) => { const value = event.target.value as NoiseSuppression; void act(() => clientRef.current!.setNoiseSuppression(value)); }}><option value="dpdfnet8">DPDFNet-8 (default)</option><option value="browser">Browser</option><option value="off">Off</option></select></label>
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
