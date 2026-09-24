import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type FormEvent, type ReactNode } from "react";
import { ArrowUpRight, Hash, HeadphoneOff, MicOff, SendHorizontal } from "lucide-react";
import type { Account } from "../account/client";
import { ChatClient, initialChatView, type ChatViewState } from "../chat/client";
import type { GeneralChatHistory } from "../chat/types";
import { watchPresence } from "../media/presence";
import type { Participant } from "../media/types";
import { applyBeat, cast, initialPreview, nextSpeakers, previewBeats, type PreviewState } from "./livePreview";
import { attachLiveMotion, type SceneSize } from "./liveMotion";
import "./live-channel.css";

const WIDE: SceneSize = { width: 780, height: 540 };
const MEDIUM: SceneSize = { width: 560, height: 420 };
const COMPACT: SceneSize = { width: 360, height: 486 };

function sceneFor(width: number, height: number) {
  if (height > width) return COMPACT;
  return width < 620 ? MEDIUM : WIDE;
}
const NUDGE_KEY = "caper:live-window-nudge";
const VISIBLE_VOICE = 4;

interface Person { id: string; name: string; guest?: boolean; avatar?: number; note?: string }
interface Line { key: string; author: Person; text: string; time?: string; label?: string; pending?: boolean }
interface VoicePerson extends Person { muted?: boolean; deafened?: boolean }

type LiveChannelProps = {
  account: Account | null;
  demoHref: string;
  history: GeneralChatHistory | null;
};

/**
 * A layered 3D window into the public #general channel. With the live channel
 * available it shows real messages, typing and the voice roster, and visitors
 * can post. Otherwise it plays a labeled preview of the same features.
 */
export default function LiveChannel({ account, demoHref, history }: LiveChannelProps) {
  const live = !!history;
  const stageRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const [size, setSize] = useState<SceneSize>(WIDE);
  const [ready, setReady] = useState(false);
  const [mounted, setMounted] = useState(false);
  const [nudge, setNudge] = useState(false);
  const nudgeRef = useRef(false);

  const dismissNudge = () => {
    if (!nudgeRef.current) return;
    nudgeRef.current = false;
    setNudge(false);
    try { localStorage.setItem(NUDGE_KEY, "seen"); } catch { /* Optional preference. */ }
  };

  useLayoutEffect(() => {
    const stage = stageRef.current;
    const scene = sceneRef.current;
    if (!stage || !scene) return;
    const detach = attachLiveMotion(stage, scene, {
      size: sceneFor,
      resized: setSize,
      interacted: dismissNudge,
    });
    setReady(true);
    setMounted(true);
    return detach;
  }, []);

  useEffect(() => {
    if (!live) return;
    let seen = false;
    try { seen = localStorage.getItem(NUDGE_KEY) === "seen"; } catch { /* Show it. */ }
    if (seen) return;
    const timer = setTimeout(() => { nudgeRef.current = true; setNudge(true); }, 1_600);
    return () => clearTimeout(timer);
  }, [live]);

  // Without room for a sidebar, the voice roster moves into the header.
  const compact = size !== WIDE;
  const channel = live ? <LiveBody account={account} demoHref={demoHref} history={history} compact={compact} mounted={mounted} nudge={nudge} onEngage={dismissNudge} />
    : <PreviewBody demoHref={demoHref} compact={compact} />;

  return (
    <div
      className="live-stage"
      ref={stageRef}
      data-ready={ready ? "" : undefined}
      data-variant={size === WIDE ? "wide" : size === MEDIUM ? "medium" : "compact"}
      tabIndex={0}
      role="region"
      aria-label={live
        ? "Live view of the public #general channel. Drag or use the arrow keys to tilt it."
        : "Animated preview of a Caper channel. Drag or use the arrow keys to tilt it."}
    >
      <div className="live-glow" aria-hidden="true" />
      <div className="live-scene" ref={sceneRef} style={{ width: size.width, height: size.height }}>
        <div className="live-shadow" aria-hidden="true" />
        {[5, 4, 3, 2, 1].map((depth) => <div key={depth} className="live-slab" style={{ "--z": -depth * 5 } as CSSProperties} aria-hidden="true" />)}
        {channel}
      </div>
    </div>
  );
}

function LiveBody({ account, demoHref, history, compact, mounted, nudge, onEngage }: {
  account: Account | null; demoHref: string; history: GeneralChatHistory; compact: boolean; mounted: boolean; nudge: boolean; onEngage: () => void;
}) {
  const [chat, setChat] = useState<ChatViewState>(() => initialChatView(history));
  const [voice, setVoice] = useState<Participant[]>([]);
  const [voiceEnabled, setVoiceEnabled] = useState(false);
  const [draft, setDraft] = useState("");
  const [validation, setValidation] = useState<string>();
  const clientRef = useRef<ChatClient | undefined>(undefined);
  const identified = useRef(false);

  useEffect(() => {
    const client = new ChatClient(setChat, undefined, { sounds: false });
    clientRef.current = client;
    client.start(history);
    return () => { client.stop(); clientRef.current = undefined; };
  }, []);

  useEffect(() => {
    let current = true;
    let stop = () => {};
    fetch("/api/media/status", { credentials: "same-origin", signal: AbortSignal.timeout(10_000) })
      .then(async (response) => response.ok ? await response.json() as { enabled?: boolean } : { enabled: false })
      .then((status) => {
        if (!current || !status.enabled) return;
        setVoiceEnabled(true);
        stop = watchPresence((snapshot) => { if (current) setVoice(snapshot.participants); }, () => undefined);
      })
      .catch(() => undefined);
    return () => { current = false; stop(); };
  }, []);

  const identify = async () => {
    if (identified.current) return;
    identified.current = true;
    let name = account?.displayName ?? account?.username ?? undefined;
    if (!name) {
      const { uniqueNamesGenerator, colors, animals } = await import("unique-names-generator");
      name = uniqueNamesGenerator({ dictionaries: [colors, animals], separator: " ", style: "capital" });
    }
    clientRef.current?.identify(name, !!account);
  };

  const sending = !!chat.pendingSend && !chat.sendError;
  const submit = async (event?: FormEvent) => {
    event?.preventDefault();
    onEngage();
    if (!chat.author || sending || chat.sendRejected) return;
    const text = chat.pendingSend?.text ?? draft;
    setValidation(undefined);
    try {
      if (await clientRef.current?.send(text)) setDraft((current) => current === text ? "" : current);
    } catch (error) {
      setValidation(error instanceof Error ? error.message : "Message could not be sent.");
    }
  };
  useEffect(() => {
    // The pending message is already visible in the list; empty the composer.
    if (chat.pendingSend) setDraft((current) => current === chat.pendingSend?.text ? "" : current);
  }, [chat.pendingSend?.clientMessageId]);

  const lines: Line[] = chat.messages.map((message) => ({
    key: `${message.author.id}:${message.clientMessageId}`,
    author: { id: message.author.id, name: message.author.name, guest: message.author.isGuest },
    text: message.content.text,
    time: message.createdAt,
  }));
  if (chat.pendingSend) {
    const author = chat.pendingSend.author ?? chat.author;
    lines.push({
      key: `${author?.id ?? "pending"}:${chat.pendingSend.clientMessageId}`,
      author: { id: author?.id ?? "you", name: author?.name ?? "You", guest: author?.isGuest },
      text: chat.pendingSend.text,
      label: chat.sendError ? (chat.sendRejected ? "Not sent" : "Not confirmed") : "Sending…",
      pending: true,
    });
  }
  const roster: VoicePerson[] = voice.map((participant) => ({ id: participant.id, name: participant.name, muted: participant.muted, deafened: participant.deafened }));
  const you = chat.author?.name ?? account?.displayName ?? account?.username;
  const error = validation ?? chat.sessionError ?? chat.sendError;
  const status = chat.phase === "error" ? "Offline" : chat.online ? "Live" : "Connecting";

  return (
    <ChannelWindow
      compact={compact}
      spaceName={chat.spaceName}
      channelName={chat.channelName}
      demoHref={demoHref}
      status={status}
      lines={lines}
      mounted={mounted}
      emptyLabel="No messages yet. Say the first hello."
      typing={chat.typingAuthors.map((author) => ({ id: author.id, name: author.name }))}
      voice={voiceEnabled ? roster : undefined}
      you={you ? { id: chat.author?.id ?? "you", name: you, guest: chat.author?.isGuest ?? !account } : undefined}
      nudge={nudge}
      composer={
        <form className="live-composer lz" style={{ "--z": 34 } as CSSProperties} onSubmit={(event) => void submit(event)}>
          {error && <p className="live-composer-error" role="alert">
            {error}
            {chat.sessionError && <button type="button" onClick={() => clientRef.current?.retrySession()}>Retry</button>}
            {chat.sendRejected && <button type="button" onClick={() => { const text = clientRef.current?.discardRejected(); if (text !== undefined) setDraft(text); }}>Edit</button>}
            {chat.sendError && !chat.sendRejected && <button type="button" onClick={() => void submit()}>Retry</button>}
          </p>}
          <label className="sr-only" htmlFor="live-message">Message #{chat.channelName}</label>
          <input
            id="live-message"
            value={draft}
            maxLength={4_000}
            autoComplete="off"
            enterKeyHint="send"
            disabled={chat.phase !== "ready"}
            placeholder={chat.author ? `Message #${chat.channelName} as ${chat.author.name}` : `Say hi in #${chat.channelName}`}
            onFocus={() => { onEngage(); void identify(); }}
            onChange={(event) => { setDraft(event.target.value); setValidation(undefined); clientRef.current?.setTyping(!!event.target.value.trim()); }}
            onBlur={() => clientRef.current?.setTyping(false)}
          />
          <button type="submit" aria-label="Send message" disabled={!chat.author || sending || (!draft.trim() && !chat.pendingSend)}>
            <SendHorizontal aria-hidden="true" />
          </button>
        </form>
      }
    />
  );
}

function PreviewBody({ demoHref, compact }: { demoHref: string; compact: boolean }) {
  const [state, setState] = useState<PreviewState>(initialPreview);
  const [speaking, setSpeaking] = useState<ReadonlySet<string>>(new Set());
  const voiceRef = useRef(state.voice);
  voiceRef.current = state.voice;

  useEffect(() => {
    const motion = window.matchMedia("(prefers-reduced-motion: reduce)");
    if (motion.matches) return;
    let timer: ReturnType<typeof setTimeout>;
    let step = 0;
    const next = () => {
      timer = setTimeout(() => {
        setState((current) => applyBeat(current));
        step++;
        next();
      }, previewBeats[step % previewBeats.length].wait);
    };
    next();
    let talk: ReturnType<typeof setTimeout>;
    let last: ReadonlySet<string> = new Set();
    const speak = () => {
      const turn = nextSpeakers(voiceRef.current, last);
      last = turn.speaking;
      setSpeaking(turn.speaking);
      talk = setTimeout(speak, turn.duration);
    };
    talk = setTimeout(speak, 900);
    return () => { clearTimeout(timer); clearTimeout(talk); };
  }, []);

  return (
    <ChannelWindow
      compact={compact}
      preview
      spaceName="Studio"
      channelName="general"
      demoHref={demoHref}
      status="Preview"
      mounted
      lines={state.messages.map((message) => ({ key: message.key, author: message.author, text: message.text, label: message.time }))}
      emptyLabel=""
      typing={state.typing}
      voice={state.voice.map((person) => ({ ...person }))}
      speaking={speaking}
      you={{ ...cast.jose, note: "Online" }}
      nudge={false}
      composer={
        <div className="live-composer live-composer-preview lz" style={{ "--z": 34 } as CSSProperties}>
          <span>This is a preview. The live channel is taking a break.</span>
          <a href={demoHref}>Open #general</a>
        </div>
      }
    />
  );
}

function ChannelWindow({ compact, preview = false, spaceName, channelName, demoHref, status, lines, mounted, emptyLabel, typing, voice, speaking, you, nudge, composer }: {
  compact: boolean; preview?: boolean; spaceName: string; channelName: string; demoHref: string;
  status: "Live" | "Connecting" | "Offline" | "Preview"; lines: Line[]; mounted: boolean; emptyLabel: string;
  typing: Person[]; voice?: VoicePerson[]; speaking?: ReadonlySet<string>; you?: Person; nudge: boolean; composer: ReactNode;
}) {
  const typingLabel = typing.length > 2 ? "Several people are typing"
    : typing.length ? `${typing.map((person) => person.name).join(" and ")} ${typing.length === 1 ? "is" : "are"} typing` : "";
  const inVoice = voice?.length ?? 0;

  return (
    <div className="live-window" data-preview={preview ? "" : undefined}>
      <div className="live-titlebar">
        <span className="live-lights" aria-hidden="true"><i /><i /><i /></span>
        <span className="live-wordmark" aria-hidden="true">caper<span>.</span></span>
        <span className="live-status lz" style={{ "--z": 18 } as CSSProperties} data-status={status.toLowerCase()} role="status">
          <i aria-hidden="true" />{status === "Connecting" ? "Connecting…" : status}
        </span>
      </div>
      <div className="live-body">
        {!compact && <aside className="live-sidebar lz" style={{ "--z": 18 } as CSSProperties}>
          <p className="live-space">{spaceName}</p>
          <p className="live-section-label">Channels</p>
          <a className="live-channel-link lz" style={{ "--z": 8 } as CSSProperties} href={demoHref} aria-current="location">
            <Hash aria-hidden="true" /><span>{channelName}</span>
          </a>
          {voice && <VoiceCard voice={voice} speaking={speaking} demoHref={demoHref} preview={preview} />}
          {you && <div className="live-you">
            <Orb person={you} size={30} />
            <span><strong>{you.name}</strong><small>{you.note ?? (you.guest ? "Guest" : "Signed in")}</small></span>
          </div>}
        </aside>}
        <section className="live-main lz" style={{ "--z": 8 } as CSSProperties} aria-label={`Messages in #${channelName}`}>
          <header className="live-header lz" style={{ "--z": 10 } as CSSProperties}>
            <Hash className="live-header-hash" aria-hidden="true" />
            <div>
              <h2>{channelName}</h2>
              <p>{preview ? "Example conversation" : "Public demo · open to everyone"}</p>
            </div>
            {compact && inVoice > 0 && <span className="live-header-voice lz" style={{ "--z": 14 } as CSSProperties} aria-label={`${inVoice} in voice`}>
              {voice!.slice(0, 3).map((person) => <Orb key={person.id} person={person} size={24} speaking={speaking?.has(person.id)} />)}
              <small>{inVoice} in voice</small>
            </span>}
            <a className="live-open lz" style={{ "--z": 14 } as CSSProperties} href={demoHref}>
              {compact ? "Open" : "Open channel"}<ArrowUpRight aria-hidden="true" />
            </a>
          </header>
          <MessageList lines={lines} mounted={mounted} emptyLabel={emptyLabel} />
          <div className="live-typing lz" style={{ "--z": 22 } as CSSProperties} data-visible={typingLabel ? "" : undefined} aria-live="off">
            {typingLabel && <><span className="live-typing-dots" aria-hidden="true"><i /><i /><i /></span>{typingLabel}…</>}
          </div>
          {composer}
          {nudge && <div className="live-nudge lz" style={{ "--z": 90 } as CSSProperties} role="note">
            <strong>This is the real #general.</strong>
            <span>Say hi, or drag the window to look around.</span>
          </div>}
        </section>
      </div>
    </div>
  );
}

function VoiceCard({ voice, speaking, demoHref, preview }: { voice: VoicePerson[]; speaking?: ReadonlySet<string>; demoHref: string; preview: boolean }) {
  const shown = voice.slice(0, VISIBLE_VOICE);
  return (
    <div className="live-voice lz" style={{ "--z": 40 } as CSSProperties}>
      <p className="live-section-label">{voice.length ? `In voice · ${voice.length}` : "Voice"}</p>
      {voice.length ? <ul>
        {shown.map((person) => {
          const talking = !!speaking?.has(person.id) && !person.muted;
          return <li key={person.id} className="lz" style={{ "--z": 6 } as CSSProperties}>
            <Orb person={person} size={30} speaking={talking} />
            <span className="live-voice-name">{person.name}</span>
            {person.deafened ? <HeadphoneOff aria-label="Deafened" /> : person.muted ? <MicOff aria-label="Muted" /> : null}
          </li>;
        })}
        {voice.length > shown.length && <li className="live-voice-more">+{voice.length - shown.length} more</li>}
      </ul> : <p className="live-voice-empty">Nobody's talking yet.</p>}
      {!preview && <a className="live-join lz" style={{ "--z": 10 } as CSSProperties} href={demoHref}>Join voice</a>}
    </div>
  );
}

function MessageList({ lines, mounted, emptyLabel }: { lines: Line[]; mounted: boolean; emptyLabel: string }) {
  const listRef = useRef<HTMLOListElement>(null);
  const known = useRef<Set<string> | null>(null);
  if (known.current === null) known.current = new Set(lines.map((line) => line.key));
  const fresh = new Set(lines.filter((line) => !known.current!.has(line.key)).map((line) => line.key));
  useEffect(() => { lines.forEach((line) => known.current!.add(line.key)); });

  // Messages stack from the bottom. Hide rows that no longer fit instead of
  // clipping the list, because clipping would flatten the 3D layers inside it.
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const fit = () => {
      const rows = [...list.children] as HTMLElement[];
      for (const row of rows) {
        if (row.offsetTop < 4) row.dataset.hidden = "";
        else delete row.dataset.hidden;
      }
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(list);
    return () => observer.disconnect();
  }, [lines]);

  if (!lines.length) return <div className="live-messages live-empty">{emptyLabel}</div>;
  return (
    <ol className="live-messages" ref={listRef}>
      {lines.map((line) => (
        <li key={line.key} className="live-message" data-fresh={fresh.has(line.key) ? "" : undefined} data-pending={line.pending ? "" : undefined}>
          <Orb person={line.author} size={34} />
          <div>
            <header>
              <strong>{line.author.name}</strong>
              {line.author.guest && <span className="live-badge">Guest</span>}
              {line.label ? <small>{line.label}</small> : line.time && mounted && <time dateTime={line.time}>{timeLabel(line.time)}</time>}
            </header>
            <p>{line.text}</p>
          </div>
        </li>
      ))}
    </ol>
  );
}

function Orb({ person, size, speaking = false }: { person: Person; size: number; speaking?: boolean }) {
  const style = { "--orb": `${size}px`, "--tint": tint(person.id) } as CSSProperties;
  return (
    <span className="live-orb" style={style} data-speaking={speaking ? "" : undefined} data-avatar={person.avatar} aria-hidden="true">
      {person.avatar === undefined && person.name.slice(0, 1).toUpperCase()}
    </span>
  );
}

/** Muted tones so real members read as distinct without loud color fields. */
const tints = ["#3d4a2e", "#4b3028", "#2f3b45", "#3c3448", "#45403a", "#2e4340"];
function tint(id: string) {
  let hash = 0;
  for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) >>> 0;
  return tints[hash % tints.length];
}

function timeLabel(value: string) {
  const date = new Date(value);
  if (Number.isNaN(date.valueOf())) return "";
  const today = new Date();
  const sameDay = date.toDateString() === today.toDateString();
  return new Intl.DateTimeFormat(undefined, sameDay ? { hour: "numeric", minute: "2-digit" } : { month: "short", day: "numeric" }).format(date);
}
