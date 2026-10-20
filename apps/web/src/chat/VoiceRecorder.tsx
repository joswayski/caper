import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { ChevronLeft, Mic, Send, Trash2 } from "lucide-react";
import {
  HOLD_TAP_MS,
  levelHeight,
  startVoiceRecording,
  stopVoicePlayback,
  formatVoiceDuration,
  VoiceRecordingError,
  type VoiceRecorder,
  type VoiceRecording,
} from "./voice.ts";

/** What a conversation offers a composer for voice messages. */
export interface VoiceComposer {
  /** A finished recording; the conversation uploads and sends it on its own. */
  onRecorded: (recording: VoiceRecording, options?: { broadcast?: boolean }) => void;
  /** A recording is uploading or being handed to the send. */
  sending?: boolean;
  /** Its upload failed; Retry sends the same recording again. */
  error?: string;
  onRetry?: () => void;
  onDiscard?: () => void;
}

/** Bars in the live level meter; the newest is on the right. */
const LIVE_BARS = 120;
/** Sliding this far left (or this far off the button) cancels a held recording. */
const SLIDE_CANCEL_PX = 96;
const DRAG_OFF_PX = 140;
const HINT_MS = 2_500;

const touchLayout = () => window.matchMedia("(max-width: 760px), (pointer: coarse)").matches;
const haptic = () => {
  try {
    navigator.vibrate?.(15);
  } catch {
    /* Haptics are optional. */
  }
};

type Phase = "idle" | "starting" | "recording";
type Mode = "click" | "hold";

/**
 * The composer's microphone. Desktop layouts click to record (the input row
 * becomes the recording bar: discard, timer, live level and send; Escape
 * discards and Enter sends). Touch layouts press and hold, release to send and
 * slide left to cancel. Keyboard activation always uses the click flow.
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
  const [mode, setMode] = useState<Mode>("click");
  const [levels, setLevels] = useState<number[]>([]);
  const [elapsed, setElapsed] = useState(0);
  const [slide, setSlide] = useState(0);
  const [error, setError] = useState<string>();
  const [hint, setHint] = useState<string>();
  const recorder = useRef<VoiceRecorder>(undefined);
  const session = useRef(0);
  const hold = useRef<{ id: number; x: number; y: number; downAt: number; released: boolean }>(undefined);
  const button = useRef<HTMLButtonElement>(null);
  const send = useRef<HTMLButtonElement>(null);
  /** The press that ends in this click was a hold, so the click is not a toggle. */
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
    setLevels([]);
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
    async (held: boolean) => {
      const current = recorder.current;
      if (!current) return;
      reset(!held);
      try {
        const recording = await current.stop();
        if (!recording) setHint(held ? "Hold to record" : "Too short to send");
        else onRecorded.current?.(recording);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Recording failed. Try again.");
      }
    },
    [reset],
  );

  const start = async (next: Mode) => {
    if (phase !== "idle" || disabled || !voice) return;
    stopVoicePlayback();
    setError(undefined);
    setHint(undefined);
    const id = ++session.current;
    setMode(next);
    setPhase("starting");
    setLevels([]);
    setElapsed(0);
    try {
      const started = await startVoiceRecording({
        onLevel: (peak) => {
          if (session.current !== id) return;
          setLevels((current) => [...current.slice(1 - LIVE_BARS), levelHeight(peak)]);
          setElapsed(performance.now() - started.startedAt);
        },
        onLimit: () => {
          if (session.current === id) void finish(next === "hold");
        },
      });
      // Released, discarded or unmounted while the microphone was starting
      // (for example during the permission prompt): record nothing.
      if (session.current !== id || (next === "hold" && (!hold.current || hold.current.released))) {
        started.cancel();
        if (session.current === id) reset(false);
        return;
      }
      recorder.current = started;
      setPhase("recording");
      if (next === "click") requestAnimationFrame(() => send.current?.focus());
    } catch (cause) {
      if (session.current !== id) return;
      reset(false);
      setError(
        cause instanceof VoiceRecordingError || cause instanceof Error ? cause.message : "Recording failed. Try again.",
      );
    }
  };

  // The desktop bar's keys: Escape discards and Enter sends, wherever focus is.
  useEffect(() => {
    if (phase === "idle" || mode !== "click") return;
    const keydown = (event: KeyboardEvent) => {
      if (event.key === "Escape") discard();
      else if (event.key === "Enter" && !event.isComposing) void finish(false);
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
  const blocked = disabled || !!voice?.sending;
  const micButton = voice ? (
    <button
      ref={button}
      type="button"
      className="chat-composer-tool chat-voice-mic"
      aria-label="Record voice message"
      title="Record voice message"
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
        const pressed = hold.current;
        if (!pressed || pressed.id !== event.pointerId || pressed.released) return;
        const dx = event.clientX - pressed.x;
        const dy = event.clientY - pressed.y;
        setSlide(Math.min(0, dx));
        if (dx < -SLIDE_CANCEL_PX || Math.hypot(dx, dy) > DRAG_OFF_PX) {
          pressed.released = true;
          haptic();
          discard(false);
        }
      }}
      onPointerUp={(event) => {
        const pressed = hold.current;
        if (!pressed || pressed.id !== event.pointerId || pressed.released) return;
        pressed.released = true;
        // A tap, or a release before the microphone started (such as during the
        // permission prompt), records nothing.
        if (performance.now() - pressed.downAt >= HOLD_TAP_MS && recorder.current) void finish(true);
        else {
          discard(false);
          setHint("Hold to record");
        }
      }}
      onPointerCancel={() => {
        if (hold.current && !hold.current.released) discard(false);
      }}
      onContextMenu={(event) => event.preventDefault()}
      onClick={() => {
        // Touch layouts record while held; a keyboard press still uses the click flow.
        if (pressed.current) {
          pressed.current = false;
          return;
        }
        if (phase === "idle") void start("click");
      }}
    >
      <Mic size={16} aria-hidden="true" />
    </button>
  ) : null;

  const time = formatVoiceDuration(elapsed, "floor");
  const meter = (
    <span className="chat-voice-levels" aria-hidden="true">
      {Array.from({ length: LIVE_BARS }, (_, index) => {
        const level = levels[index - (LIVE_BARS - levels.length)];
        return <span key={index} style={{ height: `${Math.max(0.08, level ?? 0) * 100}%` }} />;
      })}
    </span>
  );
  const bar = !active ? null : mode === "click" ? (
    <div className="chat-voice-recorder" data-mode="click" role="group" aria-label="Recording voice message">
      <button
        type="button"
        className="chat-voice-discard"
        aria-label="Discard voice message"
        title="Discard (Esc)"
        onClick={() => discard()}
      >
        <Trash2 size={16} aria-hidden="true" />
      </button>
      <span className="chat-voice-dot" data-live={phase === "recording" || undefined} aria-hidden="true" />
      <span className="chat-voice-elapsed" role="timer" aria-label={`Recording ${time}`}>
        {phase === "starting" ? "Starting…" : time}
      </span>
      {meter}
      <button
        ref={send}
        type="button"
        className="chat-voice-send"
        aria-label="Send voice message"
        title="Send (Enter)"
        disabled={phase !== "recording"}
        onClick={() => void finish(false)}
      >
        <Send size={16} aria-hidden="true" />
      </button>
    </div>
  ) : (
    <div className="chat-voice-recorder" data-mode="hold" role="group" aria-label="Recording voice message">
      <span className="chat-voice-release" aria-hidden="true">
        Release to send
      </span>
      <span className="chat-voice-dot" data-live={phase === "recording" || undefined} aria-hidden="true" />
      <span className="chat-voice-elapsed" role="timer" aria-label={`Recording ${time}`}>
        {phase === "starting" ? "Starting…" : time}
      </span>
      {meter}
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
