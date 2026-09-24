import { useEffect, useRef, useState, type CSSProperties } from "react";
import { Hash } from "lucide-react";
import { applyBeat, cast, initialPreview, nextSpeakers, previewBeats, type PreviewPerson, type PreviewState } from "./livePreview";

/**
 * Stand-in for the live room when the public channel cannot be loaded. It is
 * labeled as a preview and shows only shipped features: text, typing, voice.
 */
export default function LivePreview() {
  const [state, setState] = useState<PreviewState>(initialPreview);
  const [speaking, setSpeaking] = useState<ReadonlySet<string>>(new Set());
  const voiceRef = useRef(state.voice);
  voiceRef.current = state.voice;

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
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

  const typing = state.typing;
  const typingLabel = typing.length > 2 ? "Several people are typing"
    : typing.length ? `${typing.map((person) => person.name).join(" and ")} ${typing.length === 1 ? "is" : "are"} typing` : "";

  return (
    <div className="preview-room" aria-label="Preview conversation">
      <aside className="preview-sidebar lz" style={{ "--z": 16 } as CSSProperties}>
        <p className="preview-space">Studio</p>
        <p className="preview-label">Channels</p>
        <span className="preview-channel"><Hash aria-hidden="true" /><span>general</span></span>
        <div className="preview-voice lz" style={{ "--z": 26 } as CSSProperties}>
          <p className="preview-label">In voice · {state.voice.length}</p>
          <ul>
            {state.voice.map((person) => <li key={person.id}>
              <Orb person={person} size={30} speaking={speaking.has(person.id)} />
              <span>{person.name}</span>
            </li>)}
          </ul>
        </div>
        <div className="preview-you">
          <Orb person={cast.jose} size={30} />
          <span><strong>Jose</strong><small>Online</small></span>
        </div>
      </aside>
      <section className="preview-main lz" style={{ "--z": 6 } as CSSProperties}>
        <header className="preview-header">
          <Hash aria-hidden="true" />
          <div><h2>general</h2><p>Example conversation</p></div>
          <span className="preview-header-voice">
            {state.voice.slice(0, 3).map((person) => <Orb key={person.id} person={person} size={24} speaking={speaking.has(person.id)} />)}
            <small>{state.voice.length} in voice</small>
          </span>
        </header>
        <Messages messages={state.messages} />
        <p className="preview-typing" data-visible={typingLabel ? "" : undefined}>
          {typingLabel && <><span className="preview-dots" aria-hidden="true"><i /><i /><i /></span>{typingLabel}…</>}
        </p>
        <div className="preview-composer lz" style={{ "--z": 20 } as CSSProperties}>
          The live channel is offline right now. This is a preview.
        </div>
      </section>
    </div>
  );
}

function Messages({ messages }: { messages: PreviewState["messages"] }) {
  const listRef = useRef<HTMLOListElement>(null);
  const known = useRef<Set<string> | null>(null);
  if (known.current === null) known.current = new Set(messages.map((message) => message.key));
  const fresh = new Set(messages.filter((message) => !known.current!.has(message.key)).map((message) => message.key));
  useEffect(() => { messages.forEach((message) => known.current!.add(message.key)); });

  // Rows stack from the bottom. Rows that no longer fit are hidden by
  // measurement, because clipping the list would flatten the layers inside it.
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const fit = () => {
      for (const row of [...list.children] as HTMLElement[]) {
        if (row.offsetTop < 4) row.dataset.hidden = "";
        else delete row.dataset.hidden;
      }
    };
    fit();
    const observer = new ResizeObserver(fit);
    observer.observe(list);
    return () => observer.disconnect();
  }, [messages]);

  return (
    <ol className="preview-messages" ref={listRef}>
      {messages.map((message) => (
        <li key={message.key} className="preview-message" data-fresh={fresh.has(message.key) ? "" : undefined}>
          <Orb person={message.author} size={34} />
          <div>
            <header><strong>{message.author.name}</strong><small>{message.time}</small></header>
            <p>{message.text}</p>
          </div>
        </li>
      ))}
    </ol>
  );
}

function Orb({ person, size, speaking = false }: { person: PreviewPerson; size: number; speaking?: boolean }) {
  return <span className="preview-orb" style={{ "--orb": `${size}px` } as CSSProperties} data-speaking={speaking ? "" : undefined} data-avatar={person.avatar} aria-hidden="true" />;
}
