import { useEffect, useRef, useState } from "react";
import { PublicCallClient } from "../media/client";
import type { CallViewState, Participant, RemoteMedia } from "../media/types";
import "./call.css";

const initialState: CallViewState = { phase: "idle", participants: [], remoteMedia: [], localMedia: { camera: false, screen: false } };

function MediaOutput({ media, deafened, name, output }: { media: RemoteMedia; deafened: boolean; name: string; output: string }) {
  const ref = useRef<HTMLMediaElement>(null);
  const [blocked, setBlocked] = useState(false);
  const [deviceError, setDeviceError] = useState(false);
  useEffect(() => {
    const element = ref.current;
    if (element) {
      element.srcObject = media.stream;
      void element.play().catch(() => setBlocked(true));
    }
    return () => { if (element) element.srcObject = null; };
  }, [media.stream]);
  useEffect(() => {
    if (ref.current?.setSinkId) void ref.current.setSinkId(output).then(() => setDeviceError(false)).catch(() => setDeviceError(true));
  }, [output]);

  if (media.kind === "microphone" || media.kind === "screenAudio") {
    return <><audio ref={ref as React.RefObject<HTMLAudioElement | null>} autoPlay muted={deafened} />
      {blocked && <button onClick={() => void ref.current?.play().then(() => setBlocked(false))}>Play {name || "call"} audio</button>}
      {deviceError && <p role="alert">Audio output unavailable; choose another device.</p>}</>;
  }
  return (
    <figure className="call-media">
      <video ref={ref as React.RefObject<HTMLVideoElement | null>} autoPlay playsInline muted={deafened} />
      <figcaption>{name} · {media.kind === "screen" ? "screen" : "camera"}</figcaption>
    </figure>
  );
}

function ParticipantRow({ participant, self, watching, onWatch, disabled, speaking }: {
  participant: Participant;
  self: boolean;
  watching: boolean;
  disabled: boolean;
  speaking: boolean;
  onWatch: (value: boolean) => void;
}) {
  const watchable = participant.tracks.some((track) => track.kind !== "microphone");
  return (
    <li className="participant">
      <span className={`avatar ${speaking ? "speaking" : "quiet"}`} aria-hidden="true">
        {participant.name.slice(0, 1).toUpperCase()}
      </span>
      <span className="participant-name">
        <strong>{participant.name}{self ? " (you)" : ""}</strong>
        <small>{participant.deafened ? "Deafened" : participant.muted ? "Muted" : speaking ? "Speaking" : "In voice"}</small>
      </span>
      {!self && watchable && (
        <button disabled={disabled} className="watch-button" type="button" onClick={() => onWatch(!watching)}>
          {watching ? "Stop watching" : "Watch"}
        </button>
      )}
    </li>
  );
}

export default function Call() {
  const [state, setState] = useState(initialState);
  const [available, setAvailable] = useState<boolean>();
  const [name, setName] = useState("");
  const [muted, setMuted] = useState(false);
  const [deafened, setDeafened] = useState(false);
  const [devices, setDevices] = useState<MediaDeviceInfo[]>([]);
  const [deviceId, setDeviceId] = useState("");
  const [output, setOutput] = useState("");
  const [watched, setWatched] = useState<Set<string>>(new Set());
  const [actionError, setActionError] = useState<string>();
  const [actionPending, setActionPending] = useState(false);
  const clientRef = useRef<PublicCallClient | undefined>(undefined);
  if (!clientRef.current && typeof window !== "undefined") clientRef.current = new PublicCallClient(setState);
  const connected = state.phase === "connected";
  const controlsDisabled = state.phase !== "connected" || actionPending;

  useEffect(() => {
    let current = true;
    fetch("/api/media/status")
      .then(async (response) => response.ok ? response.json() as Promise<{ enabled: boolean }> : { enabled: false })
      .then((result) => { if (current) setAvailable(result.enabled); })
      .catch(() => { if (current) setAvailable(false); });
    const unload = () => clientRef.current?.leaveImmediately();
    window.addEventListener("pagehide", unload);
    return () => { current = false; window.removeEventListener("pagehide", unload); clientRef.current?.leaveImmediately(); };
  }, []);

  useEffect(() => {
    if (!connected) return;
    const update = () => void navigator.mediaDevices.enumerateDevices().then(setDevices).catch(() => setActionError("Device list unavailable. System defaults are still selected."));
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

  const leave = () => void act(() => clientRef.current!.leave(), () => {
    setWatched(new Set());
  });

  return (
    <main className="call-page">
      <header className="call-header">
        <a className="wordmark" href="/">caper</a>
        <span className="lobby-label"><i /> Public lobby</span>
      </header>

      {available === false ? (
        <section className="call-empty">
          <span className="empty-icon" aria-hidden="true">◌</span>
          <h1>Calls aren’t available yet.</h1>
          <p>The public lobby is currently switched off.</p>
          <a href="/">Return home</a>
        </section>
      ) : state.phase === "idle" || state.phase === "failed" ? (
        <section className="join-card">
          <p className="eyebrow">One room, open to everyone</p>
          <h1>Drop into the lobby.</h1>
          <p>Talk, share your screen, and hang out. No account needed.</p>
          <form onSubmit={(event) => { event.preventDefault(); void clientRef.current?.join(name, deviceId || undefined); }}>
            <label htmlFor="display-name">What should we call you?</label>
            <input id="display-name" value={name} maxLength={40} onChange={(event) => setName(event.target.value)} placeholder="Guest (optional)" autoComplete="nickname" />
            <button className="primary-button" disabled={available !== true} type="submit">Join with microphone</button>
          </form>
          {(state.error || actionError) && <p className="call-error" role="alert">{state.error || actionError}</p>}
          <div className="privacy-note"><strong>Public means public.</strong> Anyone with access can hear what you say or see what you choose to share. Calls are encrypted in transit, but are not end-to-end encrypted.</div>
        </section>
      ) : (
        <section className="call-room">
          <aside className="people-panel">
            <div className="panel-heading">
              <div><p className="eyebrow">Public lobby</p><h1>People</h1></div>
              <span>{state.participants.length}</span>
            </div>
            <ul>
              {state.participants.map((participant) => (
                <ParticipantRow key={participant.id} participant={participant} self={participant.id === state.selfId} disabled={controlsDisabled} speaking={state.speaking?.includes(participant.id) ?? false}
                  watching={watched.has(participant.id)} onWatch={(value) => {
                    void act(() => clientRef.current!.watch(participant.id, value), () => setWatched((old) => {
                      const next = new Set(old); value ? next.add(participant.id) : next.delete(participant.id); return next;
                    }));
                  }} />
              ))}
            </ul>
          </aside>

          <div className="stage">
            <div className="stage-title"><div><p className="eyebrow">Live now</p><h2>Public lobby</h2></div><p>{state.phase === "joining" ? "Joining…" : state.phase === "reconnecting" ? "Reconnecting…" : state.phase === "leaving" ? "Leaving…" : "Connected"}</p></div>
            <div className="media-grid">
              {state.remoteMedia.filter((media) => media.kind === "screen" || media.kind === "camera").map((media) => (
                <MediaOutput key={media.trackId} media={media} deafened={deafened} output={output} name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"} />
              ))}
              {!state.remoteMedia.some((media) => media.kind === "screen" || media.kind === "camera") && (
                <div className="stage-placeholder"><span>✦</span><h3>{connected ? "You’re in." : "Connecting…"}</h3><p>Shared screens and cameras you watch will appear here.</p></div>
              )}
            </div>
            {state.remoteMedia.filter((media) => media.kind === "microphone" || media.kind === "screenAudio").map((media) => (
              <MediaOutput key={media.trackId} media={media} deafened={deafened} output={output} name={state.participants.find((person) => person.id === media.participantId)?.name ?? "Guest"} />
            ))}
            {(state.error || actionError) && <p className="call-error room-error" role="alert">{state.error || actionError}</p>}
            {state.diagnostics && <details className="call-diagnostics"><summary>Connection diagnostics</summary><p>{state.diagnostics}</p><small>Local estimates, not billing totals. Counters reset on reconnect.</small></details>}
          </div>

          <footer className="call-controls" aria-label="Call controls">
            <label className="device-control"><span>Microphone</span><select disabled={controlsDisabled} value={deviceId} onChange={(event) => { setDeviceId(event.target.value); void act(() => clientRef.current!.changeMicrophone(event.target.value)); }}><option value="">System default</option>{devices.filter((device) => device.kind === "audioinput").map((device) => <option value={device.deviceId} key={device.deviceId}>{device.label || "Microphone"}</option>)}</select></label>
            {typeof HTMLMediaElement !== "undefined" && "setSinkId" in HTMLMediaElement.prototype && <label className="device-control"><span>Speakers</span><select value={output} onChange={(event) => setOutput(event.target.value)}><option value="">System default</option>{devices.filter((device) => device.kind === "audiooutput").map((device) => <option value={device.deviceId} key={device.deviceId}>{device.label || "Speakers"}</option>)}</select></label>}
            <div className="control-buttons">
              <button disabled={controlsDisabled} type="button" className={muted ? "active" : ""} aria-pressed={muted} onClick={() => void act(() => clientRef.current!.setMuted(!muted), () => setMuted(!muted))}>{muted ? "Unmute" : "Mute"}</button>
              <button disabled={controlsDisabled} type="button" className={deafened ? "active" : ""} aria-pressed={deafened} onClick={() => void act(() => clientRef.current!.setDeafened(!deafened), () => setDeafened(!deafened))}>{deafened ? "Listen" : "Deafen"}</button>
              <button disabled={controlsDisabled} type="button" className={state.localMedia.camera ? "active" : ""} aria-pressed={state.localMedia.camera} onClick={() => void act(() => clientRef.current!.toggleCamera())}>{state.localMedia.camera ? "Camera off" : "Camera"}</button>
              <button disabled={controlsDisabled} type="button" className={state.localMedia.screen ? "active" : ""} aria-pressed={state.localMedia.screen} onClick={() => void act(() => clientRef.current!.toggleScreen())}>{state.localMedia.screen ? "Stop sharing" : "Share screen"}</button>
              <button type="button" className="leave-button" onClick={leave}>Leave</button>
            </div>
          </footer>
        </section>
      )}
      {(state.phase === "joining" || state.phase === "leaving") && <div className="call-loading" role="status"><i />{state.phase === "joining" ? "Joining lobby…" : "Leaving…"}{state.phase === "joining" && <button onClick={leave}>Cancel</button>}</div>}
    </main>
  );
}
