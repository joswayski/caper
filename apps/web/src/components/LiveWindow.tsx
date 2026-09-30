import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties } from "react";
import { Hash, Mic, Pause, Play, SkipForward, SmilePlus } from "lucide-react";
import { attachLiveMotion } from "./liveMotion";
import "./live-window.css";

const people = [
  { name: "Maya", avatar: "0% 0%", hue: 0, note: "collecting tiny hats" },
  { name: "Theo", avatar: "100% 0%", hue: 0, note: "on aux duty" },
  { name: "June", avatar: "0% 100%", hue: 0, note: "here for the memes" },
  { name: "Leo", avatar: "100% 100%", hue: 0, note: "one more game?" },
  { name: "Noor", avatar: "0% 0%", hue: 100, note: "snack coordinator" },
  { name: "Sam", avatar: "0% 100%", hue: 175, note: "always has a fun fact" },
];

const messages = [
  { person: 0, at: 0, text: "okay, this made my entire morning", meme: true, emoji: 0 },
  { person: 1, at: 1, text: "the tiny hat is doing a lot of work here", emoji: 2 },
  { person: 2, at: 3, text: "new group photo. no objections please", emoji: 1 },
  { person: 3, at: 5, text: "i leave for TWO minutes 😂", emoji: 4 },
  { person: 0, at: 7, text: "anyway… who’s up for a game?", emoji: 3 },
  { person: 4, at: 9, text: "did someone say game night? 👀", emoji: 2 },
  { person: 1, at: 11, text: "already here. bringing the playlist 🎶", emoji: 1 },
  { person: 3, at: 12, text: "joining voice. save me a spot", emoji: 3 },
  { person: 2, at: 14, text: "rule one: nobody lets me choose the map", emoji: 0 },
  { person: 5, at: 16, text: "hello hello! what did i miss?", emoji: 1 },
  { person: 4, at: 18, text: "a tiny hat and some very serious planning", emoji: 0 },
  { person: 5, at: 20, text: "excellent. i brought snacks 🍿", emoji: 2 },
  { person: 0, at: 22, text: "brb, getting tea. please behave", emoji: 4 },
  { person: 3, at: 24, text: "no promises", emoji: 0 },
  { person: 0, at: 26, text: "back! the kettle was faster than this lobby", emoji: 3 },
  { person: 2, at: 28, text: "we’re waiting for theo’s 400-song playlist", emoji: 0 },
  { person: 1, at: 30, text: "it’s called having range", emoji: 2 },
  { person: 4, at: 32, text: "i’m requesting exactly one ridiculous song", emoji: 1 },
  { person: 5, at: 34, text: "fun fact: capybaras are excellent swimmers", emoji: 3 },
  { person: 3, at: 36, text: "so our mascot can carry us on the water map?", emoji: 0 },
  { person: 5, at: 38, text: "joining voice to defend this theory", emoji: 2 },
  { person: 0, at: 40, text: "this is now a capybara appreciation channel", meme: true, emoji: 1 },
  { person: 4, at: 42, text: "the hat really ties the whole team together", emoji: 2 },
  { person: 2, at: 45, text: "back with cookies. let’s gooo", emoji: 3 },
];

// Arrivals and voice changes belong only to this local illustration.
const activity = [
  { at: 0, person: 2, online: true, voice: true, text: "June joined voice" },
  { at: 4, person: 3, online: true, voice: false, text: "Leo joined the channel" },
  { at: 8, person: 4, online: true, voice: false, text: "Noor joined the channel" },
  { at: 10, person: 3, online: true, voice: true, text: "Leo joined voice" },
  { at: 15, person: 5, online: true, voice: false, text: "Sam joined the channel" },
  { at: 18, person: 4, online: true, voice: true, text: "Noor joined voice" },
  { at: 22, person: 0, online: true, voice: false, text: "Maya stepped out of voice" },
  { at: 25, person: 0, online: true, voice: true, text: "Maya rejoined voice" },
  { at: 31, person: 1, online: true, voice: false, text: "Theo stepped out of voice" },
  { at: 35, person: 1, online: true, voice: true, text: "Theo rejoined voice" },
  { at: 38, person: 5, online: true, voice: true, text: "Sam joined voice" },
  { at: 39, person: 2, online: false, voice: false, text: "June stepped away from the channel" },
  { at: 44, person: 2, online: true, voice: true, text: "June rejoined the channel" },
];
const cycleLength = 49;

const emoji = [
  { text: "😂", code: "1f602" }, { text: "❤️", code: "2764" },
  { text: "🔥", code: "1f525" }, { text: "👏", code: "1f44f" },
  { text: "💀", code: "1f480" },
];

function Avatar({ person }: { person: typeof people[number] }) {
  return <span className="sim-avatar" role="img" aria-label={`${person.name}'s caper avatar`} title={`${person.name} · ${person.note}`} style={{ backgroundPosition: person.avatar, filter: person.hue ? `hue-rotate(${person.hue}deg)` : undefined }} />;
}

/** A local-only, deliberately labelled illustration of a lively Caper room. */
export default function LiveWindow() {
  const stageRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const [ready, setReady] = useState(false);
  const [phase, setPhase] = useState(3);
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
    const timer = window.setInterval(() => setPhase((current) => (current + 1) % cycleLength), 1600);
    return () => window.clearInterval(timer);
  }, [paused, interacting, focused, reducedMotion, picker]);

  const present = people.map((person, index) => {
    const last = activity.filter((event) => event.person === index && event.at <= phase).at(-1);
    return { ...person, online: last?.online ?? index < 3, voice: last?.voice ?? index < 2 };
  });
  const onlinePeople = present.filter((person) => person.online);
  const voicePeople = present.filter((person) => person.voice);
  const latestActivity = activity.filter((event) => event.at <= phase).at(-1)!;
  const visibleMessages = messages.filter((message) => message.at <= phase).slice(-3);
  const typingPeople = [...new Set(messages.filter((message) => message.at > phase && message.at <= phase + 3 && present[message.person].online).map((message) => people[message.person].name))];
  const speaking = phase % voicePeople.length;
  const toggle = (key: string) => setReactions((current) => ({ ...current, [key]: !current[key] }));

  return (
    <div className="live-stage" ref={stageRef} suppressHydrationWarning data-ready={ready ? "" : undefined}>
      <div className="live-glow" aria-hidden="true" />
      <div className="live-scene" ref={sceneRef} suppressHydrationWarning tabIndex={-1}>
        <div className="live-shadow" aria-hidden="true" />
        {[5, 4, 3, 2, 1].map((depth) => <div key={depth} className="live-slab" style={{ "--z": -depth * 5 } as CSSProperties} aria-hidden="true" />)}
        <div className="live-window">
          <div className="sim-demo" aria-label="Simulated Caper conversation"
            onMouseOver={(event) => setInteracting(!!(event.target as HTMLElement).closest(".sim-reactions, .sim-emoji-picker"))} onMouseLeave={() => setInteracting(false)}
            onFocusCapture={(event) => setFocused(!!event.target.closest(".sim-reactions, .sim-emoji-picker"))} onBlurCapture={(event) => { if (!event.currentTarget.contains(event.relatedTarget)) setFocused(false); }}
            onKeyDown={(event) => { if (event.key === "Escape") { setPicker(undefined); event.currentTarget.querySelector<HTMLButtonElement>(`[data-picker-for="${picker}"]`)?.focus(); } }}>
            <aside className="sim-rail" aria-label="Demo space"><span>C</span></aside>
            <aside className="sim-sidebar">
              <div className="sim-brand">Caper</div>
              <div className="sim-section-title">Channels</div>
              <div className="sim-channel"><Hash aria-hidden="true" /> general</div>
              <p>In voice</p>
              <div className="sim-people">
                {voicePeople.map((person, index) => (
                  <div className="sim-person" data-speaking={index === speaking ? "" : undefined} key={person.name}>
                    <Avatar person={person} />
                    <strong>{person.name}</strong>
                    <Mic aria-label={index === speaking ? `${person.name} is speaking` : undefined} aria-hidden={index !== speaking} />
                  </div>
                ))}
              </div>
              <div className="sim-speaking-caption">{voicePeople[speaking].name} is talking</div>
            </aside>
            <section className="sim-chat">
              <header><div><Hash aria-hidden="true" /><strong>general</strong></div><span className="sim-label">Simulated demo</span></header>
              <div className="sim-arrival" key={latestActivity.at}>{latestActivity.text}</div>
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
                <div className="sim-typing">{typingPeople.length > 0 && <><i /><i /><i /> {typingPeople.join(" and ")} {typingPeople.length === 1 ? "is" : "are"} typing</>}</div>
              </div>
              <footer className="sim-footer" data-live-control><a className="sim-composer" href="/spaces">Join to message #general</a><span>Try a reaction · just for fun</span><div className="sim-playback"><button type="button" aria-label="Next demo moment" onClick={() => { setPicker(undefined); setPhase((current) => (current + 1) % cycleLength); }}><SkipForward aria-hidden="true" />Next</button><button type="button" disabled={reducedMotion} aria-label={paused ? "Play demo" : "Pause demo"} aria-pressed={paused || reducedMotion} onClick={() => setPaused(!paused)}>{paused || reducedMotion ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}{reducedMotion ? "Reduced motion" : paused ? "Play" : "Pause"}</button></div></footer>
            </section>
            <aside className="sim-members" aria-label="Simulated members">
              <div className="sim-members-heading">Members <span>{onlinePeople.length}</span></div>
              {onlinePeople.map((person) => <div className="sim-member" key={person.name}><Avatar person={person} /><strong>{person.name}</strong></div>)}
            </aside>
          </div>
        </div>
        <a className="live-activator" data-live-activator data-live-control href="/spaces" aria-label="Join Caper. Arrow keys tilt the window.">
          <span className="live-invite"><i aria-hidden="true" /><span className="live-invite-fine">Click to join</span><span className="live-invite-coarse">Tap to join</span></span>
        </a>
      </div>
    </div>
  );
}
