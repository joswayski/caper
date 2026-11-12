import { useCallback, useEffect, useRef, useState, type ReactNode, type RefObject } from "react";
import { AudioLines, Check, ChevronUp, LockKeyhole, Mic, Square, Trash2 } from "lucide-react";
import type { ComposerInput } from "./MentionTextarea.tsx";
import {
  HOLD_TAP_MS,
  LIVE_BAR_MS,
  LiveBars,
  liveBarLevel,
  liveBarShape,
  loudness,
  dictationInsert,
  startVoiceRecording,
  stopVoicePlayback,
  transcribeVoice,
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

/** Sliding a held recording this far up locks it: it carries on hands-free. */
const LOCK_PX = 72;
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
/** What stopping does: insert the words into the draft, or keep the voice message. */
type Intent = "dictate" | "voice";
/** A dictation being transcribed, or one that failed and still holds its audio. */
type Dictation = { recording: VoiceRecording; error?: string };
/** Hands-free (tap or click, then Stop); a touch still down but not yet a
 * hold, shown like hands-free so a tap never flashes the hold hints; or
 * press-and-hold (release stops, slide up locks). */
type Mode = "free" | "press" | "hold";

/**
 * The composer's microphone. A tap or click dictates: the input row becomes the
 * recording bar (discard, timer, live level, "Voice note" and a pulsing
 * Done), and Done (or Enter) transcribes the speech and inserts the words at
 * the caret. "Voice note" keeps the recording as a voice message instead;
 * Escape discards. Touch layouts can also press and hold for a voice message,
 * as in WhatsApp and Telegram: release stops, and sliding up locks the
 * recording hands-free (then Stop ends it). Once the finger lifts from a
 * hands-free recording the keyboard closes, so nothing types into the hidden
 * field. A voice message is staged in the composer, or sent when the person
 * chose "Send voice messages as soon as I stop recording". A failed
 * transcription keeps the audio: retry, or keep it as a voice message.
 */
export function useVoiceRecorder({
  voice,
  disabled,
  returnFocus,
}: {
  voice: VoiceComposer | undefined;
  disabled: boolean;
  /** The draft: focus returns here and dictated words are inserted at its caret. */
  returnFocus?: RefObject<ComposerInput | null>;
}): { button: ReactNode; bar: ReactNode; notice: ReactNode; active: boolean } {
  const [phase, setPhase] = useState<Phase>("idle");
  const [mode, setMode] = useState<Mode>("free");
  const [intent, setIntent] = useState<Intent>("dictate");
  const [dictation, setDictation] = useState<Dictation>();
  const [elapsed, setElapsed] = useState(0);
  /** How far a hold has slid up (negative px); the lock follows it. */
  const [lift, setLift] = useState(0);
  const [error, setError] = useState<string>();
  const [hint, setHint] = useState<string>();
  const recorder = useRef<VoiceRecorder>(undefined);
  const session = useRef(0);
  const hold = useRef<{
    id: number;
    y: number;
    downAt: number;
    released: boolean;
    /** Down for longer than a tap: a voice message. */
    held: boolean;
    locked: boolean;
    timer: ReturnType<typeof setTimeout>;
  }>(undefined);
  const stop = useRef<HTMLButtonElement>(null);
  const meter = useRef<HTMLCanvasElement>(null);
  const bars = useRef(new LiveBars());
  /** The press that ends in this click was handled by the pointer, so the click is not a toggle. */
  const pressed = useRef(false);
  const transcribing = useRef<AbortController>(undefined);
  const intentRef = useRef(intent);
  intentRef.current = intent;
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
    clearTimeout(hold.current?.timer);
    hold.current = undefined;
    setPhase("idle");
    bars.current = new LiveBars();
    setElapsed(0);
    setLift(0);
    if (focus) requestAnimationFrame(() => returnFocus?.current?.focus());
  }, []);

  const discard = useCallback(
    (focus = true) => {
      recorder.current?.cancel();
      reset(focus);
    },
    [reset],
  );

  const insert = useCallback(async (recording: VoiceRecording) => {
    transcribing.current?.abort();
    const abort = new AbortController();
    transcribing.current = abort;
    setDictation({ recording });
    try {
      const text = await transcribeVoice(recording, abort.signal);
      if (abort.signal.aborted) return;
      const input = returnFocus?.current;
      if (!text) setDictation({ recording, error: "No words heard." });
      else if (!input || !dictationInsert(input, text))
        setDictation({ recording, error: "Messages must be 4,000 characters or fewer." });
      else setDictation(undefined);
    } catch (cause) {
      if (abort.signal.aborted) return;
      setDictation({ recording, error: cause instanceof Error ? cause.message : "Transcription failed. Try again." });
    }
  }, []);

  const finish = useCallback(
    async (focus: boolean, as: Intent) => {
      const current = recorder.current;
      if (!current) return;
      reset(focus);
      try {
        const recording = await current.stop();
        if (!recording) setHint("Too short to keep. Record for at least a second.");
        else if (as === "dictate") void insert(recording);
        else onRecorded.current?.(recording);
      } catch (cause) {
        setError(cause instanceof Error ? cause.message : "Recording failed. Try again.");
      }
    },
    [reset, insert],
  );

  const start = async (next: Mode) => {
    if (phase !== "idle" || disabled || !voice || dictation) return;
    stopVoicePlayback();
    setError(undefined);
    setHint(undefined);
    const id = ++session.current;
    setMode(next);
    setIntent("dictate");
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
        onLimit: (lost) => {
          if (session.current !== id) return;
          void finish(false, intentRef.current);
          // What was captured is kept; say why it ended early.
          if (lost) setError("The microphone disconnected, so the recording stopped.");
        },
      });
      // Discarded or unmounted while the microphone was starting (for example
      // during the permission prompt): record nothing.
      if (session.current !== id) {
        started.cancel();
        return;
      }
      recorder.current = started;
      // A press released before the microphone started was a tap: keep recording hands-free.
      const held = next === "press" && !!hold.current && !hold.current.released;
      if (next === "press" && !held) setMode("free");
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
      if (lastBarAt === undefined) return;
      const progress = still ? 0 : (performance.now() - lastBarAt) / LIVE_BAR_MS;
      // The forming bar sits at the right edge from the first reading, so speech
      // shows at once, and slides one step left until it is complete. Readings
      // land every 20 ms, so a bar can be late: keep sliding rather than pausing,
      // and the finished bar lands where the slide has got to (a stall restarts at 3 steps).
      const shift = Math.min(3, Math.max(0, progress)) * LIVE_STEP_PX;
      const pending = bars.current.pending;
      for (let back = 0; back <= values.length; back++) {
        const x = width - LIVE_BAR_PX - shift - back * LIVE_STEP_PX;
        if (x < -LIVE_STEP_PX) break;
        const { scale, alpha } = liveBarShape(x + LIVE_BAR_PX / 2, width);
        if (alpha < 0.02) continue;
        const level = liveBarLevel(back === 0 ? pending : values[values.length - back]);
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
      else if (event.key === "Enter" && !event.isComposing) void finish(true, intent);
      else return;
      event.preventDefault();
      event.stopPropagation();
    };
    window.addEventListener("keydown", keydown, true);
    return () => window.removeEventListener("keydown", keydown, true);
  }, [phase, mode, intent, discard, finish]);

  // Leaving the conversation never leaves the microphone on.
  useEffect(
    () => () => {
      session.current++;
      recorder.current?.cancel();
      transcribing.current?.abort();
    },
    [],
  );

  useEffect(() => {
    if (disabled && phase !== "idle") discard(false);
  }, [disabled]);

  const active = phase !== "idle";
  // A staged voice message only blocks recording another: dictation still works.
  const blocked = disabled || !!voice?.sending || !!dictation;
  const voiceBlocked = voice?.blockedReason;
  const releaseLabel = voice?.sendOnStop ? "Release to send" : "Release to add";
  // The finger is off the screen and the recording carries on: close the
  // keyboard so typing can't reach the field hidden under the bar.
  const dismissKeyboard = () => (document.activeElement as HTMLElement | null)?.blur?.();
  const micButton = voice ? (
    <button
      type="button"
      className="chat-composer-tool chat-voice-mic"
      aria-label="Dictate"
      aria-description={voiceBlocked ? undefined : "Hold to record a voice message"}
      title="Dictate"
      data-recording={active || undefined}
      disabled={blocked}
      onPointerDown={(event) => {
        pressed.current = false;
        if (active || blocked || !event.isPrimary || event.button !== 0 || !touchLayout()) return;
        pressed.current = true;
        // Keep the keyboard and caret where they are while holding.
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        const id = event.pointerId;
        hold.current = {
          id,
          y: event.clientY,
          downAt: performance.now(),
          released: false,
          held: false,
          locked: false,
          // Still down after a tap's length: a voice message. Show what
          // releasing and sliding do. Without room for one it stays dictation.
          timer: setTimeout(() => {
            const current = hold.current;
            if (current?.id !== id || current.released || voiceBlocked) return;
            current.held = true;
            haptic();
            setMode("hold");
            setIntent("voice");
          }, HOLD_TAP_MS),
        };
        setLift(0);
        void start("press");
      }}
      onPointerMove={(event) => {
        const current = hold.current;
        if (!current || current.id !== event.pointerId || current.released || !current.held) return;
        const dy = Math.min(0, event.clientY - current.y);
        setLift(dy);
        if (dy < -LOCK_PX) {
          current.released = true;
          current.locked = true;
          clearTimeout(current.timer);
          haptic();
          setLift(0);
          setMode("free");
        }
      }}
      onPointerUp={(event) => {
        const current = hold.current;
        if (!current || current.id !== event.pointerId) return;
        if (current.released) {
          if (current.locked) dismissKeyboard();
          return;
        }
        current.released = true;
        clearTimeout(current.timer);
        setLift(0);
        // A tap (or a release before the microphone started) dictates
        // hands-free until Done; releasing a real hold stops the voice message.
        if (!current.held || !recorder.current) {
          setMode("free");
          dismissKeyboard();
        } else void finish(false, "voice");
      }}
      onPointerCancel={() => {
        // The browser took the touch: keep recording hands-free rather than lose it.
        const current = hold.current;
        if (!current || current.released) return;
        current.released = true;
        clearTimeout(current.timer);
        setLift(0);
        setMode("free");
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
      {active && mode === "hold" && (
        <span className="chat-voice-lock" aria-hidden="true">
          <span style={{ translate: `0 ${(Math.max(-LOCK_PX, lift) / LOCK_PX) * 8}px` }}>
            <LockKeyhole size={14} />
          </span>
          <ChevronUp size={14} style={{ opacity: Math.max(0, 1 + lift / (LOCK_PX / 2)) }} />
        </span>
      )}
    </button>
  ) : null;

  const time = formatVoiceDuration(elapsed, "floor");
  // For diagnostics: the call's processing status, or why the browser's is used.
  const processing = recorder.current?.processing;
  const dictating = intent === "dictate";
  const bar = !active ? null : (
    <div
      className="chat-voice-recorder"
      data-mode={mode}
      data-intent={intent}
      data-processing={processing}
      role="group"
      aria-label={dictating ? "Dictating" : "Recording voice message"}
    >
      {mode === "hold" && (
        <span className="chat-voice-release" aria-hidden="true">
          {releaseLabel} · Slide up to lock
        </span>
      )}
      <button
        type="button"
        className="chat-voice-discard"
        aria-label={dictating ? "Discard dictation" : "Discard voice message"}
        title="Discard (Esc)"
        onClick={() => discard()}
      >
        <Trash2 size={16} aria-hidden="true" />
      </button>
      <span className="chat-voice-dot" data-live={phase === "recording" || undefined} aria-hidden="true" />
      <span className="chat-voice-elapsed" role="timer" aria-label={`Recording ${time}`}>
        {phase === "starting" ? "Starting…" : time}
      </span>
      <canvas ref={meter} className="chat-voice-levels" aria-hidden="true" />
      {dictating && (
        <button
          type="button"
          className="chat-voice-as-note"
          title={
            voiceBlocked ??
            (voice?.sendOnStop ? "Stop and send as a voice message" : "Stop and keep as a voice message")
          }
          disabled={phase !== "recording" || !!voiceBlocked}
          onClick={() => void finish(!touchLayout(), "voice")}
        >
          <AudioLines size={14} aria-hidden="true" />
          Voice note
        </button>
      )}
      <button
        ref={stop}
        type="button"
        className="chat-voice-stop"
        aria-label={dictating ? "Done, insert text" : "Stop recording"}
        title={dictating ? "Insert text (Enter)" : voice?.sendOnStop ? "Stop and send (Enter)" : "Stop (Enter)"}
        disabled={phase !== "recording"}
        onClick={() => void finish(!touchLayout(), intent)}
      >
        {dictating ? (
          <Check size={16} strokeWidth={2.5} aria-hidden="true" />
        ) : (
          <Square size={12} fill="currentColor" aria-hidden="true" />
        )}
      </button>
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
      {dictation && !dictation.error && (
        <p className="chat-voice-status" role="status">
          <span className="chat-spinner" aria-hidden="true" />
          Transcribing…
          <button
            type="button"
            onClick={() => {
              transcribing.current?.abort();
              setDictation(undefined);
            }}
          >
            Cancel
          </button>
        </p>
      )}
      {dictation?.error && (
        <p className="chat-inline-error" role="alert">
          Couldn't insert your words. {dictation.error}
          <button type="button" onClick={() => void insert(dictation.recording)}>
            Retry
          </button>
          {!voiceBlocked && (
            <button
              type="button"
              onClick={() => {
                setDictation(undefined);
                onRecorded.current?.(dictation.recording);
              }}
            >
              {voice?.sendOnStop ? "Send as voice note" : "Keep as voice note"}
            </button>
          )}
          <button type="button" onClick={() => setDictation(undefined)}>
            Discard
          </button>
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
