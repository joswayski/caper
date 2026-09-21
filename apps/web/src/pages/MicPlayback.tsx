import { useEffect, useRef, useState } from "react";
import Slider from "../components/Slider";
import { createVoiceComparison, MAX_RECORDING_SECONDS, recordReceivedAudio, type ReceivedRecording } from "../media/recording";

const INPUT_METER_SEGMENTS = 40;

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
        // Browser microphone samples are quiet for normal speaking. Scale the
        // display independently from the recorded signal so speech is legible.
        setLevel(Math.min(1, rms * 32));
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
    {Array.from({ length: INPUT_METER_SEGMENTS }, (_, index) => <i key={index} className={active && index / INPUT_METER_SEGMENTS < level ? "lit" : ""} />)}
  </div>;
}

interface Clip {
  url: string;
  silent: boolean;
}

function RecordingPlayback({ clip, label, output, autoPlay, onEnded, onPlay, onAudioElement, onDeviceError }: {
  clip: Clip;
  label: string;
  output: string;
  autoPlay: boolean;
  onEnded?(): void;
  onPlay?(): void;
  onAudioElement?(element: HTMLAudioElement | null): void;
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
  return <audio ref={(element) => { ref.current = element; onAudioElement?.(element); }} aria-label={`${label} microphone sample`} controls src={clip.url} onEnded={onEnded} onPlay={onPlay} />;
}

function ProcessingDetail({ label, id, children }: { label: string; id: string; children: string }) {
  return <span className="processing-detail">
    {label}
    <button type="button" aria-label={`About ${label}`} aria-describedby={id}>?</button>
    <span id={id} role="tooltip">{children}</span>
  </span>;
}

export default function MicPlayback({ stream, output, processingStrength, onProcessingStrengthChange, onClose }: {
  stream: MediaStream;
  output: string;
  processingStrength: number;
  onProcessingStrengthChange(strength: number): void;
  onClose?(): void;
}) {
  const urlsRef = useRef<{ natural?: string; processed?: string }>({});
  const playbackRefs = useRef<{ natural?: HTMLAudioElement; processed?: HTMLAudioElement }>({});
  const sessionRef = useRef<ReceivedRecording | undefined>(undefined);
  const generation = useRef(0);
  const [recording, setRecording] = useState(false);
  const [processing, setProcessing] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const [recordedBlob, setRecordedBlob] = useState<Blob>();
  const [clips, setClips] = useState<{ natural?: Clip; processed?: Clip }>({});
  const [naturalPlaybackEnded, setNaturalPlaybackEnded] = useState(false);
  const [error, setError] = useState<string>();
  const [deviceError, setDeviceError] = useState(false);

  const pauseSample = (sample: "natural" | "processed") => playbackRefs.current[sample]?.pause();

  const discardAll = () => {
    generation.current++;
    sessionRef.current?.cancel();
    sessionRef.current = undefined;
    Object.values(urlsRef.current).forEach((url) => { if (url) URL.revokeObjectURL(url); });
    urlsRef.current = {};
    setRecording(false);
    setProcessing(false);
    setRecordedBlob(undefined);
    setClips({});
    setNaturalPlaybackEnded(false);
    setError(undefined);
    setDeviceError(false);
  };

  const startRecording = () => {
    generation.current++;
    sessionRef.current?.cancel();
    sessionRef.current = undefined;
    Object.values(urlsRef.current).forEach((url) => { if (url) URL.revokeObjectURL(url); });
    urlsRef.current = {};
    setRecordedBlob(undefined);
    setClips({});
    setNaturalPlaybackEnded(false);
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
        const url = URL.createObjectURL(blob);
        urlsRef.current.natural = url;
        setClips({ natural: { url, silent: false } });
        setRecordedBlob(blob);
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
    if (!recordedBlob) return;
    const current = ++generation.current;
    const previous = urlsRef.current.processed;
    if (previous) URL.revokeObjectURL(previous);
    delete urlsRef.current.processed;
    setClips((existing) => ({ natural: existing.natural }));
    setProcessing(true);
    setError(undefined);
    const timer = window.setTimeout(() => {
      void createVoiceComparison(recordedBlob, processingStrength).then(({ processed, hasSignal }) => {
        if (generation.current !== current) return;
        const url = URL.createObjectURL(processed);
        urlsRef.current.processed = url;
        setClips((existing) => ({
          natural: existing.natural && { ...existing.natural, silent: !hasSignal },
          processed: { url, silent: !hasSignal },
        }));
        setProcessing(false);
      }).catch((reason) => {
        if (generation.current !== current) return;
        setProcessing(false);
        setError(reason instanceof Error ? reason.message : "Voice comparison could not be prepared.");
      });
    }, 120);
    return () => window.clearTimeout(timer);
  }, [processingStrength, recordedBlob]);
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
        <h3 id="mic-test-heading">{recording ? "Recording your voice" : "Find your voice"}</h3>
      </div>
      {recording && <p className="recording-clock"><i aria-hidden="true" />{elapsed.toFixed(1)}s</p>}
      {onClose && !recording && <button type="button" className="mic-test-close" onClick={onClose}>Back to join</button>}
    </div>
    <p className="mic-test-copy">Record once to compare the same sample with and without voice enhancement.</p>
    <div className="voice-processing-control">
      <div className="voice-processing-heading"><ProcessingDetail label="Voice processing" id="voice-processing-detail">Adds high-pass filtering, warmth and presence EQ, compression, makeup gain, and peak limiting. The slider controls the strength.</ProcessingDetail><output>{processingStrength}%</output></div>
      <Slider label="Voice processing" value={processingStrength} disabled={recording} onChange={onProcessingStrengthChange} />
      <div><small>Natural</small><small>Enhanced</small></div>
    </div>
    <div className="mic-test-action-row">
      {!recording && <button type="button" className="mic-test-button" disabled={processing} onClick={startRecording}>{processing ? "Preparing…" : "Mic Test"}</button>}
      {recording && <button type="button" className="mic-test-button" onClick={() => sessionRef.current?.finish()}>Stop Testing</button>}
      <div className="mic-level">
        <div className="mic-meter-label"><span>Input level</span></div>
        <InputMeter active={recording} stream={stream} />
      </div>
    </div>
    {error && <p className="call-error" role="alert">{error}</p>}
    {deviceError && <p className="call-error" role="alert">Audio output unavailable; choose another device.</p>}
    <div className="mic-comparison" aria-label="Recorded samples">
      <article>
        <div><strong>Natural</strong></div>
        {clips.natural
          ? <><RecordingPlayback clip={clips.natural} label="Natural" output={output} autoPlay onEnded={() => setNaturalPlaybackEnded(true)} onPlay={() => pauseSample("processed")} onAudioElement={(element) => { playbackRefs.current.natural = element ?? undefined; }} onDeviceError={() => setDeviceError(true)} />
            {clips.natural.silent && <p role="alert">No audible signal detected. Check your mic and try again.</p>}</>
          : <p>Your natural recording will appear here.</p>}
      </article>
      <article className={clips.processed ? "latest" : undefined}>
        <div><strong>Enhanced</strong></div>
        {clips.processed
          ? <><RecordingPlayback clip={clips.processed} label="Enhanced" output={output} autoPlay={naturalPlaybackEnded} onPlay={() => pauseSample("natural")} onAudioElement={(element) => { playbackRefs.current.processed = element ?? undefined; }} onDeviceError={() => setDeviceError(true)} />
            {clips.processed.silent && <p role="alert">No audible signal detected. Check your mic and try again.</p>}</>
          : <p>{processing ? "Applying voice enhancement…" : "Your enhanced comparison will appear here."}</p>}
      </article>
    </div>
    <div className="processing-details" aria-label="Audio processing details">
      <ProcessingDetail label="On-device noise cancellation" id="noise-cancellation-detail">DPDFNet-8 HR removes background noise locally before your voice is sent.</ProcessingDetail>
    </div>
  </section>;
}
