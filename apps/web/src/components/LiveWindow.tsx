import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Hash, Mic } from "lucide-react";
import { attachLiveMotion } from "./liveMotion";
import "./live-window.css";

const people = [
  { name: "Maya", initials: "MK", color: "#b64d32" },
  { name: "Theo", initials: "TR", color: "#637a43" },
  { name: "June", initials: "JL", color: "#6f667d" },
];

const messages = [
  { name: "Maya", text: "okay, this made my entire morning", meme: true },
  { name: "Theo", text: "the tiny hat is doing a lot of work here" },
  { name: "June", text: "sending this to the group chat immediately" },
];

/** A local-only, deliberately labelled illustration of a lively Caper room. */
export default function LiveWindow() {
  const stageRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [phase, setPhase] = useState(2);

  useLayoutEffect(() => {
    const stage = stageRef.current;
    const scene = sceneRef.current;
    if (!stage || !scene) return;
    const motion = attachLiveMotion(stage, scene, {
      bounds: () => stage.closest<HTMLElement>("[data-live-bounds]"),
      activate: () => window.location.assign("/login"),
      deactivate: () => undefined,
      interacted: () => undefined,
    });
    setReady(true);
    return () => motion.detach();
  }, []);

  useEffect(() => {
    if (window.matchMedia("(prefers-reduced-motion: reduce)").matches) return;
    const timer = window.setInterval(() => setPhase((current) => (current + 1) % 6), 2600);
    return () => window.clearInterval(timer);
  }, []);

  const visiblePeople = Math.min(people.length, 1 + Math.floor(phase / 2));
  const visibleMessages = Math.min(messages.length, 1 + Math.floor(phase / 2));
  const typing = phase === 1 || phase === 3 || phase === 5;
  const speaking = phase % visiblePeople;

  return (
    <div className="live-stage" ref={stageRef} suppressHydrationWarning data-ready={ready ? "" : undefined}>
      <div className="live-glow" aria-hidden="true" />
      <div className="live-scene" ref={sceneRef} suppressHydrationWarning tabIndex={-1}>
        <div className="live-shadow" aria-hidden="true" />
        {[5, 4, 3, 2, 1].map((depth) => <div key={depth} className="live-slab" style={{ "--z": -depth * 5 } as CSSProperties} aria-hidden="true" />)}
        <div className="live-window">
          <div className="sim-demo" aria-label="Simulated Caper conversation">
            <aside className="sim-sidebar">
              <div className="sim-brand">Caper</div>
              <div className="sim-channel"><Hash aria-hidden="true" /> general</div>
              <p>In voice</p>
              <div className="sim-people">
                {people.slice(0, visiblePeople).map((person, index) => (
                  <div className="sim-person" data-speaking={index === speaking ? "" : undefined} key={person.name}>
                    <span className="sim-avatar" style={{ "--avatar": person.color } as CSSProperties}>{person.initials}</span>
                    <strong>{person.name}</strong>
                    <Mic aria-label={index === speaking ? `${person.name} is speaking` : undefined} aria-hidden={index !== speaking} />
                  </div>
                ))}
              </div>
            </aside>
            <section className="sim-chat">
              <header><div><Hash aria-hidden="true" /><strong>general</strong></div><span className="sim-label">Simulated demo</span></header>
              <div className="sim-messages" aria-live="off">
                {messages.slice(0, visibleMessages).map((message, index) => (
                  <article className="sim-message" key={message.name}>
                    <span className="sim-avatar" style={{ "--avatar": people[index].color } as CSSProperties}>{people[index].initials}</span>
                    <div><strong>{message.name}</strong><p>{message.text}</p>{message.meme && <img className="sim-meme" src="/images/demo-tiny-hat.webp" width="384" height="384" alt="A capybara wearing a tiny hat. Caption: Tiny hat. Huge energy." />}</div>
                  </article>
                ))}
                {typing && <div className="sim-typing"><i /><i /><i /> {people[visiblePeople % people.length].name} is typing</div>}
              </div>
              <div className="sim-join-space" aria-hidden="true" />
            </section>
          </div>
        </div>
        <a className="live-activator" data-live-activator href="/login" aria-label="Sign in with email to join Caper. Arrow keys tilt the window.">
          <span className="live-invite"><i aria-hidden="true" /><span className="live-invite-fine">Click to join</span><span className="live-invite-coarse">Tap to join</span></span>
        </a>
      </div>
    </div>
  );
}
