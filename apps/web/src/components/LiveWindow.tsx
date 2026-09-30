import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Hash, Mic, Pause, Play, SmilePlus } from "lucide-react";
import { attachLiveMotion } from "./liveMotion";
import "./live-window.css";

const people = [
  { name: "Maya", avatar: "0% 0%", note: "collecting tiny hats" },
  { name: "Theo", avatar: "100% 0%", note: "on aux duty" },
  { name: "June", avatar: "0% 100%", note: "here for the memes" },
  { name: "Leo", avatar: "100% 100%", note: "one more game?" },
];

const messages = [
  { person: 0, at: 0, text: "okay, this made my entire morning", meme: true, emoji: 0 },
  { person: 1, at: 3, text: "the tiny hat is doing a lot of work here", emoji: 2 },
  { person: 2, at: 6, text: "new group photo. no objections please", emoji: 1 },
  { person: 3, at: 9, text: "i leave for TWO minutes 😂", emoji: 4 },
  { person: 0, at: 12, text: "anyway… who’s up for a game?", emoji: 3 },
  { person: 1, at: 15, text: "already here. bringing the playlist 🎶", emoji: 2 },
];

const emoji = [
  { text: "😂", code: "1f602" }, { text: "❤️", code: "2764" },
  { text: "🔥", code: "1f525" }, { text: "👏", code: "1f44f" },
  { text: "💀", code: "1f480" },
];

function Avatar({ person }: { person: typeof people[number] }) {
  return <span className="sim-avatar" role="img" aria-label={`${person.name}'s caper avatar`} title={`${person.name} · ${person.note}`} style={{ backgroundPosition: person.avatar }} />;
}

/** A local-only, deliberately labelled illustration of a lively Caper room. */
export default function LiveWindow() {
  const stageRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [phase, setPhase] = useState(4);
  const [paused, setPaused] = useState(false);
  const [interacting, setInteracting] = useState(false);
  const [focused, setFocused] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const [picker, setPicker] = useState<number>();
  const [reactions, setReactions] = useState<Record<string, boolean>>({});

  useLayoutEffect(() => {
    const stage = stageRef.current;
    const scene = sceneRef.current;
    if (!stage || !scene) return;
    const motion = attachLiveMotion(stage, scene, {
      bounds: () => stage.closest<HTMLElement>("[data-live-bounds]"),
      activate: () => undefined,
      deactivate: () => undefined,
      interacted: () => undefined,
    });
    setReady(true);
    return () => motion.detach();
  }, []);

  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReducedMotion(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    if (paused || interacting || focused || reducedMotion || picker !== undefined) return;
    const timer = window.setInterval(() => setPhase((current) => (current + 1) % 19), 2200);
    return () => window.clearInterval(timer);
  }, [paused, interacting, focused, reducedMotion, picker]);

  const visiblePeople = Math.min(people.length, 2 + Math.floor(phase / 4));
  const visibleMessages = messages.filter((message) => message.at <= phase).slice(-3);
  const nextMessage = messages.find((message) => message.at > phase);
  const typing = nextMessage && nextMessage.at - phase <= 2;
  const speaking = phase % visiblePeople;
  const toggle = (key: string) => setReactions((current) => ({ ...current, [key]: !current[key] }));

  return (
    <div className="live-stage" ref={stageRef} suppressHydrationWarning data-ready={ready ? "" : undefined}>
      <div className="live-glow" aria-hidden="true" />
      <div className="live-scene" ref={sceneRef} suppressHydrationWarning tabIndex={-1}>
        <div className="live-shadow" aria-hidden="true" />
        {[5, 4, 3, 2, 1].map((depth) => <div key={depth} className="live-slab" style={{ "--z": -depth * 5 } as CSSProperties} aria-hidden="true" />)}
        <div className="live-window">
          <div className="sim-demo" aria-label="Simulated Caper conversation"
            onMouseEnter={() => setInteracting(true)} onMouseLeave={() => setInteracting(false)}
            onFocusCapture={() => setFocused(true)} onBlurCapture={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false); }}
            onKeyDown={(event) => { if (event.key === "Escape") { setPicker(undefined); event.currentTarget.querySelector<HTMLButtonElement>(`[data-picker-for="${picker}"]`)?.focus(); } }}>
            <aside className="sim-sidebar">
              <div className="sim-brand">Caper</div>
              <div className="sim-channel"><Hash aria-hidden="true" /> general</div>
              <p>In voice</p>
              <div className="sim-people">
                {people.slice(0, visiblePeople).map((person, index) => (
                  <div className="sim-person" data-speaking={index === speaking ? "" : undefined} key={person.name}>
                    <Avatar person={person} />
                    <strong>{person.name}</strong>
                    <Mic aria-label={index === speaking ? `${person.name} is speaking` : undefined} aria-hidden={index !== speaking} />
                  </div>
                ))}
              </div>
              <div className="sim-arrival" key={visiblePeople}>{people[visiblePeople - 1].name} joined voice</div>
            </aside>
            <section className="sim-chat">
              <header><div><Hash aria-hidden="true" /><strong>general</strong></div><span className="sim-label">Simulated demo</span></header>
              <div className="sim-messages" aria-live="off">
                {visibleMessages.map((message) => (
                  <article className="sim-message" key={message.at}>
                    <Avatar person={people[message.person]} />
                    <div><strong>{people[message.person].name}</strong><span className="sim-time">just now</span><p>{message.text}</p>{message.meme && <img className="sim-meme" src="/images/demo-tiny-hat.webp" width="384" height="384" alt="A capybara wearing a tiny hat. Caption: Tiny hat. Huge energy." />}
                      <div className="sim-reactions" data-live-control>
                        {emoji.map((item, index) => {
                          const key = `${message.at}:${index}`;
                          const mine = !!reactions[key];
                          const count = (index === message.emoji ? 1 + Math.min(3, Math.floor((phase - message.at) / 2)) : 0) + Number(mine);
                          return count > 0 && <button type="button" key={item.code} aria-pressed={mine} aria-label={`${item.text}, ${count} ${count === 1 ? "reaction" : "reactions"}${mine ? ", including you" : ""}`} title={mine ? "Remove your reaction" : "Add your reaction"} onClick={() => toggle(key)}><img src={`/images/demo-emoji/${item.code}.svg`} alt="" /><span>{count}</span></button>;
                        })}
                        <button type="button" className="sim-add-reaction" data-picker-for={message.at} aria-label={`Add reaction to ${people[message.person].name}'s message`} aria-expanded={picker === message.at} onClick={() => setPicker(picker === message.at ? undefined : message.at)}><SmilePlus aria-hidden="true" /></button>
                      </div>
                      {picker === message.at && <div className="sim-emoji-picker" data-live-control role="group" aria-label="Try a demo reaction">
                        {emoji.map((item, index) => <button type="button" key={item.code} aria-label={`React with ${item.text}`} onClick={() => { toggle(`${message.at}:${index}`); setPicker(undefined); }}><img src={`/images/demo-emoji/${item.code}.svg`} alt={item.text} /></button>)}
                      </div>}
                    </div>
                  </article>
                ))}
                <div className="sim-typing">{typing && <><i /><i /><i /> {people[nextMessage.person].name} is typing</>}</div>
              </div>
              <footer className="sim-footer" data-live-control><span>Try a reaction · just for fun</span><button type="button" disabled={reducedMotion} aria-label={paused ? "Play demo" : "Pause demo"} aria-pressed={paused || reducedMotion} onClick={() => setPaused(!paused)}>{paused || reducedMotion ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}{reducedMotion ? "Reduced motion" : paused ? "Play" : "Pause"}</button></footer>
            </section>
          </div>
        </div>
        <a className="live-activator" data-live-activator data-live-control href="/login" aria-label="Sign in with email to join Caper. Arrow keys tilt the window.">
          <span className="live-invite"><i aria-hidden="true" /><span className="live-invite-fine">Click to join</span><span className="live-invite-coarse">Tap to join</span></span>
        </a>
      </div>
    </div>
  );
}
