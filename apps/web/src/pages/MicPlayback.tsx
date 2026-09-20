import { useEffect, useRef, useState } from "react";
import { MAX_RECORDING_SECONDS, recordReceivedAudio, type ReceivedRecording } from "../media/recording";
import type { VoiceEnhancement } from "../media/microphone";

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

interface Clip {
  url: string;
  silent: boolean;
}

const voiceModes: Array<{ id: VoiceEnhancement; label: string; description: string; article: string }> = [
  { id: "natural", label: "Natural", description: "Noise cleanup only", article: "a" },
  { id: "enhanced", label: "Enhanced", description: "Fuller and more even", article: "an" },
];

function RecordingPlayback({ clip, label, output, autoPlay, onDeviceError }: {
  clip: Clip;
  label: string;
  output: string;
  autoPlay: boolean;
  onDeviceError(): void;
}) {
  const ref = useRef<HTMLAudioElement>(null);
  useEffect(() => {
    const element = ref.current;
    if (!element) return;
    let current = true;
    const prepare = async () => {
      const setSinkId = (element as HTMLMediaElement & { setSinkId?: (id: string) => Promise<void> }).setSinkId?.bind(element);
      try {
        if (setSinkId) await setSinkId(output);
      } catch {
        if (current) onDeviceError();
        return;
      }
      if (autoPlay) void element.play().catch(() => undefined);
    };
    void prepare();
    return () => { current = false; };
  }, [autoPlay, clip.url, output]);
  return <audio ref={ref} aria-label={`${label} microphone sample`} controls src={clip.url} />;
}

export default function MicPlayback({ stream, output, status, enhancement, onEnhancementChange }: {
  stream: MediaStream;
  output: string;
  status?: string;
  enhancement: VoiceEnhancement;
  onEnhancementChange(mode: VoiceEnhancement): void;
}) {
  const urlsRef = useRef<Partial<Record<VoiceEnhancement, string>>>({});
  const sessionRef = useRef<ReceivedRecording | undefined>(undefined);
  const generation = useRef(0);
  const recordingMode = useRef<VoiceEnhancement>(enhancement);
  const [recording, setRecording] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [clips, setClips] = useState<Partial<Record<VoiceEnhancement, Clip>>>({});
  const [latest, setLatest] = useState<VoiceEnhancement>();
  const [error, setError] = useState<string>();
  const [deviceError, setDeviceError] = useState(false);

  const discardAll = () => {
    generation.current++;
    sessionRef.current?.cancel();
    sessionRef.current = undefined;
    Object.values(urlsRef.current).forEach((url) => { if (url) URL.revokeObjectURL(url); });
    urlsRef.current = {};
    setRecording(false);
    setClips({});
    setLatest(undefined);
    setError(undefined);
    setDeviceError(false);
  };

  const startRecording = () => {
    generation.current++;
    sessionRef.current?.cancel();
    sessionRef.current = undefined;
    const mode = enhancement;
    recordingMode.current = mode;
    const previous = urlsRef.current[mode];
    if (previous) URL.revokeObjectURL(previous);
    delete urlsRef.current[mode];
    setClips((current) => ({ ...current, [mode]: undefined }));
    setLatest(undefined);
    setError(undefined);
    setDeviceError(false);
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
        const url = URL.createObjectURL(blob);
        urlsRef.current[mode] = url;
        setClips((existing) => ({ ...existing, [mode]: { url, silent: !hasSignal } }));
        setLatest(mode);
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
    return () => discardAll();
  }, [stream]);
  useEffect(() => {
    if (!recording) return;
    const started = performance.now();
    const timer = window.setInterval(() => setElapsed(Math.min(MAX_RECORDING_SECONDS, (performance.now() - started) / 1_000)), 100);
    return () => window.clearInterval(timer);
  }, [recording]);
  return <section className="mic-test-card" aria-labelledby="mic-test-heading">
    <div className="mic-test-heading">
      <div>
        <p className="eyebrow">Microphone test</p>
        <h3 id="mic-test-heading">{recording ? `Recording ${voiceModes.find((mode) => mode.id === recordingMode.current)?.label}` : "Find your voice"}</h3>
      </div>
      {recording && <p className="recording-clock"><i aria-hidden="true" />{elapsed.toFixed(1)}s</p>}
    </div>
    <p className="mic-test-copy">Choose a sound, record a short sample, then switch modes and record again to compare them.</p>
    <div className="voice-mode-picker" role="radiogroup" aria-label="Voice sound">
      {voiceModes.map((mode) => <button
        key={mode.id}
        type="button"
        role="radio"
        aria-checked={enhancement === mode.id}
        disabled={recording}
        onClick={() => onEnhancementChange(mode.id)}
      >
        <span>{mode.label}{mode.id === "enhanced" && <small>Recommended</small>}</span>
        <em>{mode.description}</em>
      </button>)}
    </div>
    <div className="mic-meter-label"><span>Input level</span><small>Lights up while recording</small></div>
    <InputMeter active={recording} stream={stream} />
    {status && <p className="mic-test-status"><i aria-hidden="true" />{status}</p>}
    {!recording && <button type="button" className="stop-recording-button" onClick={startRecording}>Record {enhancement} sample</button>}
    {recording && <button type="button" className="stop-recording-button" onClick={() => sessionRef.current?.finish()}>Stop &amp; play back</button>}
    {error && <p className="call-error" role="alert">{error}</p>}
    {deviceError && <p className="call-error" role="alert">Audio output unavailable; choose another device.</p>}
    <div className="mic-comparison" aria-label="Recorded samples">
      {voiceModes.map((mode) => <article key={mode.id} className={latest === mode.id ? "latest" : undefined}>
        <div><strong>{mode.label}</strong>{latest === mode.id && <small>New</small>}</div>
        {clips[mode.id]
          ? <><RecordingPlayback clip={clips[mode.id]!} label={mode.label} output={output} autoPlay={latest === mode.id} onDeviceError={() => setDeviceError(true)} />
            {clips[mode.id]!.silent && <p role="alert">No audible signal detected. Check your mic and try again.</p>}</>
          : <p>Record {mode.article} {mode.label.toLowerCase()} sample to compare.</p>}
      </article>)}
    </div>
    <small className="mic-test-privacy">Samples stay in this tab and disappear when you end the test.</small>
  </section>;
}
