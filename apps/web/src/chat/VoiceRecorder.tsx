import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { ChevronLeft, Mic, Square, Trash2 } from "lucide-react";
import {
  HOLD_TAP_MS,
  LIVE_BAR_MS,
  LiveBars,
  liveBarLevel,
  liveBarShape,
  loudness,
  startVoiceRecording,
  stopVoicePlayback,
  formatVoiceDuration,
  VoiceRecordingError,
  type VoiceRecorder,
  type VoiceRecording,
} from "./voice.ts";

/** What a conversation offers a composer for voice messages. */
export interface VoiceComposer {
  /** A finished recording: staged in the composer, or sent on its own when the
   * "send as soon as I stop recording" preference is on. */
  onRecorded: (recording: VoiceRecording, options?: { broadcast?: boolean }) => void;
  /** Whether stopping sends (the preference), for the bar's wording. */
  sendOnStop?: boolean;
  /** Why the microphone can't record right now, such as a staged voice message. */
  blockedReason?: string;
  /** A send-on-stop recording is uploading or being handed to the send. */
  sending?: boolean;
  /** Its upload failed; Retry sends the same recording again. */
  error?: string;
  onRetry?: () => void;
  onDiscard?: () => void;
}

/** Sliding this far left (or this far off the button) cancels a held recording. */
const SLIDE_CANCEL_PX = 96;
const DRAG_OFF_PX = 140;
const HINT_MS = 2_500;
/** Live meter bars: 3px pills 6px apart, never shorter than a 3px dot. */
const LIVE_BAR_PX = 3;
const LIVE_STEP_PX = 6;
const LIVE_MIN_PX = 3;
/** Quiet bars are neutral; loud ones warm toward the eggshell action colour. */
const QUIET_RGB = [150, 156, 158];
const LOUD_RGB = [241, 237, 228];

const touchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;
const reducedMotion = () => window.matchMedia("(prefers-reduced-motion: reduce)").matches;
const haptic = () => {
  try {
    navigator.vibrate?.(15);
  } catch {
    /* Haptics are optional. */
  }
};

type Phase = "idle" | "starting" | "recording";
/** Hands-free (tap or click, then Stop) or press-and-hold (release stops). */
type Mode = "free" | "hold";

/**
 * The composer's microphone. A tap or click records hands-free: the input row
 * becomes the recording bar (discard, timer, live level and Stop; Enter stops
 * and Escape discards). Touch layouts can also press and hold, release to stop
 * and slide left to cancel. Stopping stages the recording in the composer, or
 * sends it when the person chose "Send voice messages as soon as I stop recording".
 */
export function useVoiceRecorder({
  voice,
  disabled,
  returnFocus,
}: {
  voice: VoiceComposer | undefined;
  disabled: boolean;
  returnFocus?: RefObject<HTMLTextAreaElement | null>;
}): { button: ReactNode; bar: ReactNode; notice: ReactNode; active: boolean } {
  const [phase, setPhase] = useState<Phase>("idle");
  const [mode, setMode] = useState<Mode>("free");
  const [elapsed, setElapsed] = useState(0);
  const [slide, setSlide] = useState(0);
  const [error, setError] = useState<string>();
  const [hint, setHint] = useState<string>();
  const recorder = useRef<VoiceRecorder>(undefined);
  const session = useRef(0);
  const hold = useRef<{ id: number; x: number; y: number; downAt: number; released: boolean }>(undefined);
  const stop = useRef<HTMLButtonElement>(null);
  const meter = useRef<HTMLCanvasElement>(null);
  const bars = useRef(new LiveBars());
  /** The press that ends in this click was handled by the pointer, so the click is not a toggle. */
  const pressed = useRef(false);
  const onRecorded = useRef(voice?.onRecorded);
  onRecorded.current = voice?.onRecorded;

  useEffect(() => {
    if (!hint) return;
    const timer = setTimeout(() => setHint(undefined), HINT_MS);
    return () => clearTimeout(timer);
  }, [hint]);

  const reset = useCallback((focus: boolean) => {
    session.current++;
    recorder.current = undefined;
    hold.current = undefined;
    setPhase("idle");
    bars.current = new LiveBars();
    setElapsed(0);
    setSlide(0);
    if (focus) requestAnimationFrame(() => returnFocus?.current?.focus());
  }, []);

  const discard = useCallback(
    (focus = true) => {
      recorder.current?.cancel();
      reset(focus);
    },
    [reset],
  );

  const finish = useCallback(
    async (focus: boolean) => {
      const current = recorder.current;
      if (!current) return;
      reset(focus);
      try {
        const recording = await current.stop();
        if (!recording) setHint("Too short to keep. Record for at least a second.");
        else onRecorded.current?.(recording);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Recording failed. Try again.");
      }
    },
    [reset],
  );

  const start = async (next: Mode) => {
    if (phase !== "idle" || disabled || !voice || voice.blockedReason) return;
    stopVoicePlayback();
    setError(undefined);
    setHint(undefined);
    const id = ++session.current;
    setMode(next);
    setPhase("starting");
    bars.current = new LiveBars();
    setElapsed(0);
    try {
      const started = await startVoiceRecording({
        onLevel: (rms) => {
          if (session.current !== id) return;
          const now = performance.now();
          bars.current.sample(loudness(rms), now);
          // Re-render only when the shown second changes; the meter draws itself.
          const next = now - started.startedAt;
          setElapsed((shown) => (Math.floor(shown / 1000) === Math.floor(next / 1000) ? shown : next));
        },
        onLimit: () => {
          if (session.current === id) void finish(false);
        },
      });
      // Discarded or unmounted while the microphone was starting (for example
      // during the permission prompt): record nothing.
      if (session.current !== id) {
        started.cancel();
        return;
      }
      recorder.current = started;
      // A hold released before the microphone started was a tap: keep recording hands-free.
      const held = next === "hold" && !!hold.current && !hold.current.released;
      if (next === "hold" && !held) setMode("free");
      setPhase("recording");
      // Pointer layouts focus Stop for Enter; touch leaves focus (and the keyboard) alone.
      if (!held && next === "free") requestAnimationFrame(() => stop.current?.focus());
    } catch (cause) {
      if (session.current !== id) return;
      reset(false);
      setError(
        cause instanceof VoiceRecordingError || cause instanceof Error ? cause.message : "Recording failed. Try again.",
      );
    }
  };

  // Draw the live bars every frame: they scroll smoothly between additions
  // (reduced motion steps instead), arrive flat at the right and swell, and
  // pinch and fade toward the left.
  useEffect(() => {
    if (phase !== "recording") return;
    const still = reducedMotion();
    let frame = 0;
    const draw = () => {
      frame = requestAnimationFrame(draw);
      const canvas = meter.current;
      const context = canvas?.getContext("2d");
      if (!canvas || !context) return;
      const width = canvas.clientWidth;
      const height = canvas.clientHeight;
      const ratio = window.devicePixelRatio || 1;
      if (canvas.width !== Math.round(width * ratio) || canvas.height !== Math.round(height * ratio)) {
        canvas.width = Math.round(width * ratio);
        canvas.height = Math.round(height * ratio);
      }
      context.setTransform(ratio, 0, 0, ratio, 0, 0);
      context.clearRect(0, 0, width, height);
      const { values, lastBarAt } = bars.current;
      const progress = still || lastBarAt === undefined ? 1 : (performance.now() - lastBarAt) / LIVE_BAR_MS;
      // Readings land every 20 ms but bars are due every 25 ms, so a bar can be
      // late: keep sliding past one step rather than pausing, and the new bar
      // lands exactly where the slide has got to (a stall restarts at 3 steps).
      const shift = Math.min(3, Math.max(0, progress)) * LIVE_STEP_PX;
      for (let back = 0; back < values.length; back++) {
        const x = width - shift - back * LIVE_STEP_PX;
        if (x < -LIVE_STEP_PX) break;
        const { scale, alpha } = liveBarShape(x + LIVE_BAR_PX / 2, width);
        if (alpha < 0.02) continue;
        const level = liveBarLevel(values[values.length - 1 - back]);
        const barHeight = Math.max(LIVE_MIN_PX * Math.max(0.6, scale), level * scale * height * 0.94);
        const barWidth = LIVE_BAR_PX * Math.min(1, Math.max(0.6, scale + 0.4));
        const warmth = Math.min(1, 0.25 + level * 0.9);
        const [r, g, b] = QUIET_RGB.map((quiet, index) => Math.round(quiet + (LOUD_RGB[index] - quiet) * warmth));
        const left = x + (LIVE_BAR_PX - barWidth) / 2;
        const top = (height - barHeight) / 2;
        context.fillStyle = `rgba(${r}, ${g}, ${b}, ${alpha})`;
        context.beginPath();
        // Square bars where canvas has no roundRect (Safari before 16).
        if (context.roundRect) context.roundRect(left, top, barWidth, barHeight, Math.min(barWidth, barHeight) / 2);
        else context.rect(left, top, barWidth, barHeight);
        context.fill();
      }
    };
    frame = requestAnimationFrame(draw);
    return () => cancelAnimationFrame(frame);
  }, [phase]);

  // The hands-free bar's keys: Escape discards and Enter stops, wherever focus is.
  useEffect(() => {
    if (phase === "idle" || mode !== "free") return;
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") discard();
      else if (event.key === "Enter" && !event.isComposing) void finish(true);
      else return;
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [phase, mode, discard, finish]);

  // Leaving the conversation never leaves the microphone on.
  useEffect(
    () => () => {
      session.current++;
      recorder.current?.cancel();
    },
    [],
  );
  useEffect(() => {
    if (disabled && phase !== "idle") discard(false);
  }, [disabled]);

  const active = phase !== "idle";
  const blocked = disabled || !!voice?.sending || !!voice?.blockedReason;
  const releaseLabel = voice?.sendOnStop ? "Release to send" : "Release to stop";
  const micButton = voice ? (
    <button
      type="button"
      className="chat-composer-tool chat-voice-mic"
      aria-label="Record voice message"
      title={voice.blockedReason ?? "Record voice message"}
      data-recording={active || undefined}
      disabled={blocked}
      onPointerDown={(event) => {
        pressed.current = false;
        if (blocked || !event.isPrimary || event.button !== 0 || !touchLayout()) return;
        pressed.current = true;
        // Keep the keyboard and caret where they are while holding.
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        hold.current = {
          id: event.pointerId,
          x: event.clientX,
          y: event.clientY,
          downAt: performance.now(),
          released: false,
        };
        setSlide(0);
        void start("hold");
      }}
      onPointerMove={(event) => {
        const current = hold.current;
        if (!current || current.id !== event.pointerId || current.released) return;
        const dx = event.clientX - current.x;
        const dy = event.clientY - current.y;
        setSlide(Math.min(0, dx));
        if (dx < -SLIDE_CANCEL_PX || Math.hypot(dx, dy) > DRAG_OFF_PX) {
          current.released = true;
          haptic();
          discard(false);
        }
      }}
      onPointerUp={(event) => {
        const current = hold.current;
        if (!current || current.id !== event.pointerId || current.released) return;
        current.released = true;
        setSlide(0);
        // A tap (or a release before the microphone started) keeps recording
        // hands-free until Stop; releasing a real hold stops.
        if (performance.now() - current.downAt < HOLD_TAP_MS || !recorder.current) {
          setMode("free");
        } else void finish(false);
      }}
      onPointerCancel={() => {
        if (hold.current && !hold.current.released) discard(false);
      }}
      onContextMenu={(event) => event.preventDefault()}
      onClick={() => {
        if (pressed.current) {
          pressed.current = false;
          return;
        }
        if (phase === "idle") void start("free");
      }}
    >
      <Mic size={16} aria-hidden="true" />
    </button>
  ) : null;

  const time = formatVoiceDuration(elapsed, "floor");
  // For diagnostics: the call's processing status, or why the browser's is used.
  const processing = recorder.current?.processing;
  const levelMeter = <canvas ref={meter} className="chat-voice-levels" aria-hidden="true" />;
  const dot = <span className="chat-voice-dot" data-live={phase === "recording" || undefined} aria-hidden="true" />;
  const timer = (
    <span className="chat-voice-elapsed" role="timer" aria-label={`Recording ${time}`}>
      {phase === "starting" ? "Starting…" : time}
    </span>
  );
  const bar = !active ? null : mode === "free" ? (
    <div
      className="chat-voice-recorder"
      data-mode="free"
      data-processing={processing}
      role="group"
      aria-label="Recording voice message"
    >
      <button
        type="button"
        className="chat-voice-discard"
        aria-label="Discard voice message"
        title="Discard (Esc)"
        onClick={() => discard()}
      >
        <Trash2 size={16} aria-hidden="true" />
      </button>
      {dot}
      {timer}
      {levelMeter}
      <button
        ref={stop}
        type="button"
        className="chat-voice-stop"
        aria-label="Stop recording"
        title={voice?.sendOnStop ? "Stop and send (Enter)" : "Stop (Enter)"}
        disabled={phase !== "recording"}
        onClick={() => void finish(!touchLayout())}
      >
        <Square size={12} fill="currentColor" aria-hidden="true" />
      </button>
    </div>
  ) : (
    <div
      className="chat-voice-recorder"
      data-mode="hold"
      data-processing={processing}
      role="group"
      aria-label="Recording voice message"
    >
      <span className="chat-voice-release" aria-hidden="true">
        {releaseLabel}
      </span>
      {dot}
      {timer}
      {levelMeter}
      <span
        className="chat-voice-slide"
        style={{
          translate: `${Math.max(-16, slide / 4)}px 0`,
          opacity: Math.max(0.35, 1 + slide / SLIDE_CANCEL_PX),
        }}
      >
        <ChevronLeft size={14} aria-hidden="true" />
        Slide to cancel
      </span>
      <span className="chat-voice-held" aria-hidden="true">
        <Mic size={18} />
      </span>
    </div>
  );

  const notice = (
    <>
      {error && (
        <p className="chat-inline-error" role="alert">
          {error}
        </p>
      )}
      {voice?.error && (
        <p className="chat-inline-error" role="alert">
          Voice message not sent. {voice.error}
          {voice.onRetry && (
            <button type="button" onClick={voice.onRetry}>
              Retry
            </button>
          )}
          {voice.onDiscard && (
            <button type="button" onClick={voice.onDiscard}>
              Discard
            </button>
          )}
        </p>
      )}
      {voice?.sending && !voice.error && (
        <p className="chat-voice-status" role="status">
          <span className="chat-spinner" aria-hidden="true" />
          Sending voice message…
        </p>
      )}
      {hint && !error && (
        <p className="chat-voice-status" role="status">
          {hint}
        </p>
      )}
    </>
  );
  return { button: micButton, bar, notice, active };
}
