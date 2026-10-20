import { useEffect, useId, useLayoutEffect, useMemo, useRef, useState, type PointerEvent } from "react";
import { Pause, Play } from "lucide-react";
import type { ChatAttachment } from "./types.ts";
import {
  claimVoicePlayback,
  decodeWaveform,
  formatVoiceDuration,
  releaseVoicePlayback,
  resampleWaveform,
  spokenDuration,
  transcriptView,
} from "./voice.ts";

/** Bars drawn for any waveform length; the card keeps one width on desktop and phones. */
export const VOICE_BARS = 40;
const SEEK_STEP_MS = 5_000;
/** Without a waveform, a flat placeholder row. */
const FLAT_BAR = 0.12;

/** A sent voice message: play/pause, a seekable waveform and its time, then the
 * transcript. Only one voice message plays at a time across the app. */
export function VoiceMessage({ attachment, onError }: { attachment: ChatAttachment; onError?: () => void }) {
  return (
    <div className="chat-voice-message">
      <VoicePlayer attachment={attachment} onError={onError} />
      <VoiceTranscript attachment={attachment} />
    </div>
  );
}

function VoicePlayer({ attachment, onError }: { attachment: ChatAttachment; onError?: () => void }) {
  const audio = useRef<HTMLAudioElement>(null);
  const [playing, setPlaying] = useState(false);
  const [position, setPosition] = useState(0);
  const [mediaDuration, setMediaDuration] = useState<number>();
  const pendingSeek = useRef<number>(undefined);
  const duration = attachment.durationMs || mediaDuration || 0;
  const waveform = useMemo(() => decodeWaveform(attachment.waveform), [attachment.waveform]);
  const bars = useMemo(
    () => (waveform ? resampleWaveform(waveform, VOICE_BARS) : Array<number>(VOICE_BARS).fill(FLAT_BAR)),
    [waveform],
  );
  const progress = duration ? Math.min(1, position / duration) : 0;
  const started = playing || position > 0;

  useEffect(() => {
    if (!playing) return;
    let frame = 0;
    const tick = () => {
      const element = audio.current;
      if (element) setPosition(element.currentTime * 1_000);
      frame = requestAnimationFrame(tick);
    };
    frame = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(frame);
  }, [playing]);
  useEffect(() => {
    const element = audio.current;
    return () => {
      if (!element) return;
      element.pause();
      releaseVoicePlayback(element);
    };
  }, []);

  const toggle = () => {
    const element = audio.current;
    if (!element) return;
    if (!element.paused) {
      element.pause();
      return;
    }
    claimVoicePlayback(element);
    void element.play().catch(() => setPlaying(false));
  };
  const seek = (ms: number) => {
    const element = audio.current;
    const target = Math.max(0, Math.min(duration, ms));
    setPosition(target);
    if (!element) return;
    if (element.readyState >= HTMLMediaElement.HAVE_METADATA) element.currentTime = target / 1_000;
    else pendingSeek.current = target;
  };
  const seekToPointer = (event: PointerEvent<HTMLDivElement>) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    if (!bounds.width || !duration) return;
    seek(((event.clientX - bounds.left) / bounds.width) * duration);
  };

  const total = formatVoiceDuration(duration);
  const time = started ? formatVoiceDuration(Math.max(0, duration - position), "ceil") : total;
  return (
    <div className="chat-voice" data-playing={playing || undefined}>
      <button
        type="button"
        className="chat-voice-play"
        aria-label={playing ? "Pause voice message" : "Play voice message"}
        onClick={toggle}
        disabled={!attachment.url}
      >
        {playing ? (
          <Pause size={16} fill="currentColor" aria-hidden="true" />
        ) : (
          <Play size={16} fill="currentColor" aria-hidden="true" />
        )}
      </button>
      <div
        className="chat-voice-wave"
        role="slider"
        tabIndex={0}
        aria-label="Voice message position"
        aria-valuemin={0}
        aria-valuemax={Math.round(duration / 1_000)}
        aria-valuenow={Math.round(position / 1_000)}
        aria-valuetext={`${spokenDuration(position)} of ${spokenDuration(duration)}`}
        data-flat={!waveform || undefined}
        onPointerDown={(event) => {
          if (!event.isPrimary || event.button !== 0) return;
          event.currentTarget.setPointerCapture(event.pointerId);
          seekToPointer(event);
        }}
        onPointerMove={(event) => {
          if (event.currentTarget.hasPointerCapture(event.pointerId)) seekToPointer(event);
        }}
        onKeyDown={(event) => {
          const step =
            event.key === "ArrowLeft" || event.key === "ArrowDown"
              ? -SEEK_STEP_MS
              : event.key === "ArrowRight" || event.key === "ArrowUp"
                ? SEEK_STEP_MS
                : undefined;
          if (step !== undefined) seek(position + step);
          else if (event.key === "Home") seek(0);
          else if (event.key === "End") seek(duration);
          else if (event.key === " " || event.key === "Enter") toggle();
          else return;
          event.preventDefault();
        }}
      >
        {bars.map((height, index) => (
          <span
            key={index}
            data-played={(index + 0.5) / bars.length <= progress || undefined}
            style={{ height: `${Math.max(FLAT_BAR, height) * 100}%` }}
          />
        ))}
      </div>
      <span className="chat-voice-time" aria-hidden="true">
        {time}
      </span>
      <span className="sr-only">Voice message, {spokenDuration(duration)}</span>
      <audio
        ref={audio}
        preload="metadata"
        src={attachment.url}
        onPlay={() => setPlaying(true)}
        onPause={() => {
          setPlaying(false);
          if (audio.current) setPosition(audio.current.currentTime * 1_000);
        }}
        onEnded={() => {
          const element = audio.current;
          setPlaying(false);
          setPosition(0);
          if (element) {
            element.currentTime = 0;
            releaseVoicePlayback(element);
          }
        }}
        onLoadedMetadata={() => {
          const element = audio.current;
          if (!element) return;
          if (Number.isFinite(element.duration) && element.duration > 0) setMediaDuration(element.duration * 1_000);
          if (pendingSeek.current !== undefined) {
            element.currentTime = pendingSeek.current / 1_000;
            pendingSeek.current = undefined;
          }
        }}
        onError={onError}
      />
    </div>
  );
}

/** Slack-style transcript: three lines, then View/Hide transcript. Pending shows
 * "Transcribing…"; failed, missing, unknown or empty transcripts show nothing. */
export function VoiceTranscript({ attachment }: { attachment: Pick<ChatAttachment, "transcript"> }) {
  const view = transcriptView(attachment);
  const root = useRef<HTMLDivElement>(null);
  const body = useRef<HTMLParagraphElement>(null);
  const bodyId = useId();
  const [expanded, setExpanded] = useState(false);
  const [clamped, setClamped] = useState(false);
  const [arrived, setArrived] = useState(false);
  const previous = useRef(view.state);
  const text = view.state === "ready" ? view.text : "";
  useLayoutEffect(() => {
    // A transcript that lands while the reader watches the live end fades in;
    // anywhere else in history it simply appears.
    if (previous.current === "pending" && view.state === "ready") {
      const scroller = root.current?.closest<HTMLElement>(".chat-scroller, .chat-thread-messages");
      setArrived(!scroller || scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight <= 80);
    }
    previous.current = view.state;
  }, [view.state]);
  useLayoutEffect(() => {
    const element = body.current;
    if (!element || expanded) return;
    const measure = () => setClamped(element.scrollHeight > element.clientHeight + 1);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [text, expanded]);
  if (view.state === "none") return null;
  return (
    <div ref={root} className="chat-voice-transcript" data-arrive={arrived || undefined}>
      {view.state === "pending" ? (
        <p className="chat-voice-transcribing">Transcribing…</p>
      ) : (
        <>
          <p ref={body} id={bodyId} data-expanded={expanded || undefined}>
            {view.text}
          </p>
          {(clamped || expanded) && (
            <button
              type="button"
              aria-expanded={expanded}
              aria-controls={bodyId}
              onClick={() => setExpanded(!expanded)}
            >
              {expanded ? "Hide transcript" : "View transcript"}
            </button>
          )}
        </>
      )}
    </div>
  );
}
