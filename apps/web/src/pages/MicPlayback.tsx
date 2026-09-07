import { useEffect, useRef, useState } from "react";
import { MAX_RECORDING_SECONDS, recordReceivedAudio, type ReceivedRecording } from "../media/recording";

function InputMeter({ active, stream }: { active: boolean; stream: MediaStream }) {
  const [level, setLevel] = useState(0);

  useEffect(() => {
    if (!active || typeof AudioContext === "undefined") { setLevel(0); return; }
    const context = new AudioContext();
    const source = context.createMediaStreamSource(stream);
    const analyser = context.createAnalyser();
    analyser.fftSize = 256;
    source.connect(analyser);
    const samples = new Uint8Array(analyser.fftSize);
    let frame = 0;
    let lastUpdate = 0;
    const read = (now: number) => {
      analyser.getByteTimeDomainData(samples);
      if (now - lastUpdate > 80) {
        const rms = Math.sqrt(samples.reduce((sum, sample) => sum + ((sample - 128) / 128) ** 2, 0) / samples.length);
        setLevel(Math.min(1, rms * 5));
        lastUpdate = now;
      }
      frame = requestAnimationFrame(read);
    };
    void context.resume();
    frame = requestAnimationFrame(read);
    return () => {
      cancelAnimationFrame(frame);
      source.disconnect();
      analyser.disconnect();
      void context.close();
    };
  }, [active, stream]);

  return <div className="mic-meter" aria-label={active ? "Received microphone level" : "Microphone level inactive"}>
    {Array.from({ length: 24 }, (_, index) => <i key={index} className={active && index / 24 < level ? "lit" : ""} />)}
  </div>;
}

export default function MicPlayback({ stream, output, status }: { stream: MediaStream; output: string; status?: string }) {
  const playbackRef = useRef<HTMLAudioElement>(null);
  const urlRef = useRef<string | undefined>(undefined);
  const sessionRef = useRef<ReceivedRecording | undefined>(undefined);
  const generation = useRef(0);
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [url, setUrl] = useState<string>();
  const [playbackStatus, setPlaybackStatus] = useState<"starting" | "playing" | "ready">();
  const [error, setError] = useState<string>();
  const [deviceError, setDeviceError] = useState(false);
  const [silent, setSilent] = useState(false);

  const discard = () => {
    generation.current++;
    sessionRef.current?.cancel();
    sessionRef.current = undefined;
    playbackRef.current?.pause();
    if (urlRef.current) URL.revokeObjectURL(urlRef.current);
    urlRef.current = undefined;
    setRecording(false);
    setUrl(undefined);
    setPlaybackStatus(undefined);
    setSilent(false);
    setError(undefined);
    setDeviceError(false);
  };

  const startRecording = () => {
    discard();
    setError(undefined);
    setElapsed(0);
    const current = generation.current;
    try {
      const session = recordReceivedAudio(stream);
      sessionRef.current = session;
      setRecording(true);
      void session.result.then(async (blob) => {
        if (generation.current !== current) return;
        sessionRef.current = undefined;
        setRecording(false);
        // A nonempty Opus container can contain only silence. Check the actual
        // decoded recording, not merely connection state or packet counts.
        const context = new AudioContext();
        let hasSignal = false;
        try {
          const audio = await context.decodeAudioData(await blob.arrayBuffer());
          for (let channel = 0; channel < audio.numberOfChannels; channel++) {
            hasSignal ||= audio.getChannelData(channel).some((sample) => Math.abs(sample) > 0.001);
          }
        } finally { void context.close(); }
        if (generation.current !== current) return;
        setSilent(!hasSignal);
        urlRef.current = URL.createObjectURL(blob);
        setUrl(urlRef.current);
        setPlaybackStatus("starting");
      }).catch((reason) => {
        if (generation.current !== current) return;
        sessionRef.current = undefined;
        setRecording(false);
        setError(reason instanceof Error ? reason.message : "Audio recording failed.");
      });
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : "Audio recording failed.");
    }
  };

  useEffect(() => {
    // Reconnects and device changes must never start a new recording implicitly.
    return () => discard();
  }, [stream]);
  useEffect(() => {
    if (!recording) return;
    const started = performance.now();
    const timer = window.setInterval(() => setElapsed(Math.min(MAX_RECORDING_SECONDS, (performance.now() - started) / 1_000)), 100);
    return () => window.clearInterval(timer);
  }, [recording]);
  useEffect(() => {
    const element = playbackRef.current;
    if (!element || !url) return;
    let current = true;
    const play = async () => {
      try {
        if (element.setSinkId) await element.setSinkId(output);
      } catch {
        if (!current) return;
        setDeviceError(true);
        setPlaybackStatus("ready");
        return;
      }
      if (!current) return;
      setDeviceError(false);
      if (playbackStatus !== "starting") return;
      try { await element.play(); }
      catch {
        if (current) setPlaybackStatus("ready");
      }
    };
    void play();
    return () => { current = false; };
  }, [url, output]);

  return <section className="mic-test-card" aria-labelledby="mic-test-heading">
    <div className="mic-test-heading">
      <div>
        <p className="eyebrow">Private microphone check</p>
        <h3 id="mic-test-heading">{recording ? "Recording your microphone" : url ? "Here’s how you sound" : "Microphone test"}</h3>
      </div>
      {recording && <p className="recording-clock"><i aria-hidden="true" />{elapsed.toFixed(1)}s</p>}
    </div>
    <p className="mic-test-copy">{recording
      ? `Speak normally, then stop when you’re ready. Recording ends automatically after ${MAX_RECORDING_SECONDS} seconds.`
      : url ? "Playback starts automatically. Listen for clarity, volume, and background noise."
      : "Press Record, speak normally, then stop to hear the audio returned through Cloudflare."}</p>
    <InputMeter active={recording} stream={stream} />
    {status && <p className="noise-status">{status}</p>}
    {!recording && !url && <button type="button" className="stop-recording-button" onClick={startRecording}>Record microphone</button>}
    {recording && <button type="button" className="stop-recording-button" onClick={() => sessionRef.current?.finish()}>Stop &amp; play back</button>}
    {error && <p className="call-error" role="alert">{error}</p>}
    {silent && <p className="call-error" role="alert">No audible signal was detected in the returned recording. Check your selected microphone and hardware mute, or try Browser noise suppression.</p>}
    {deviceError && <p className="call-error" role="alert">Audio output unavailable; choose another device.</p>}
    {url && <div className="mic-playback">
      <p className="noise-status" role="status">{playbackStatus === "playing" ? "Playing your recording…" : playbackStatus === "ready" ? "Recording ready. Press play to listen." : "Starting playback…"}</p>
      <audio ref={playbackRef} aria-label="Recorded microphone test" controls src={url} onPlay={() => setPlaybackStatus("playing")} onPause={() => setPlaybackStatus("ready")} onEnded={() => setPlaybackStatus("ready")} onError={() => setError("The recording could not be played. Please record another test.")} />
      <button type="button" onClick={startRecording}>Test again</button>
    </div>}
    <small className="mic-test-privacy">The recording stays in this tab and is deleted when you test again or end the mic test.</small>
  </section>;
}
