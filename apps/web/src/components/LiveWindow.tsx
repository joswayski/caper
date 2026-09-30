import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { ChevronDown, Hash, Pause, Play, SkipForward, SmilePlus } from "lucide-react";
import { attachLiveMotion } from "./liveMotion";
import { createDemoTiming } from "./demoTiming";
import "../pages/call.css";
import "../spaces/spaces.css";
import "./live-window.css";

const people = [
  { name: "Maya", avatar: "0% 0%", hue: 0, note: "collecting tiny hats", speech: [[0, 4.25], [6.5, 9], [12, 13.75], [18.25, 20.5], [26, 29.25], [40.25, 43]] },
  { name: "Theo", avatar: "100% 0%", hue: 0, note: "on aux duty", speech: [[1.5, 5.5], [8, 10.75], [14.5, 18], [27.25, 30.5], [35.5, 37.25], [43, 46]] },
  { name: "June", avatar: "0% 100%", hue: 0, note: "here for the memes", speech: [[2.75, 6.25], [10.5, 12.75], [15.75, 16.5], [22.5, 25], [33, 36.25], [44.25, 46.5]] },
  { name: "Leo", avatar: "100% 100%", hue: 0, note: "one more game?", speech: [[10.25, 13], [15.5, 19.25], [23, 27.5], [30.25, 32.5], [36, 39.25], [46, 48.5]] },
  { name: "Noor", avatar: "0% 0%", hue: 100, note: "snack coordinator", speech: [[18.5, 22.75], [25.5, 28], [29, 33.5], [37.25, 40.75], [44, 47]] },
  { name: "Sam", avatar: "0% 100%", hue: 175, note: "always has a fun fact", speech: [[38.5, 40.5], [42.25, 45.75], [47.25, 48.25]] },
];

const messages = [
  { person: 0, at: 0, text: "okay, this made my entire morning", meme: true, emoji: 0 },
  { person: 1, at: 1.25, text: "the tiny hat is doing a lot of work here", emoji: 2 },
  { person: 2, at: 3, text: "new group photo. no objections please", emoji: 1 },
  { person: 3, at: 4.75, text: "i leave for TWO minutes 😂", emoji: 4 },
  { person: 0, at: 7.5, text: "anyway… who’s up for a game?", emoji: 3 },
  { person: 4, at: 9.25, text: "did someone say game night? 👀", emoji: 2 },
  { person: 1, at: 11.25, text: "already here. bringing the playlist 🎶", emoji: 1 },
  { person: 3, at: 11.75, text: "save me a spot", emoji: 3 },
  { person: 2, at: 12.5, text: "rule one: nobody lets me choose the map", emoji: 0 },
  { person: 5, at: 16.25, text: "hello hello! what did i miss?", emoji: 1 },
  { person: 4, at: 16.75, text: "a tiny hat and some very serious planning", emoji: 0 },
  { person: 5, at: 20.25, text: "excellent. i brought snacks 🍿", emoji: 2 },
  { person: 0, at: 22, text: "brb, getting tea. please behave", emoji: 4 },
  { person: 3, at: 24, text: "no promises", emoji: 0 },
  { person: 0, at: 26.5, text: "back! the kettle was faster than this lobby", emoji: 3 },
  { person: 2, at: 28.25, text: "we’re waiting for theo’s 400-song playlist", emoji: 0 },
  { person: 1, at: 30.5, text: "it’s called having range", emoji: 2 },
  { person: 4, at: 32.25, text: "i’m requesting exactly one ridiculous song", emoji: 1 },
  { person: 5, at: 34.75, text: "fun fact: capybaras are excellent swimmers", emoji: 3 },
  { person: 3, at: 36.25, text: "so our mascot can carry us on the water map?", emoji: 0 },
  { person: 5, at: 38, text: "joining voice to defend this theory", emoji: 2 },
  { person: 0, at: 40.5, text: "this is now a capybara appreciation channel", meme: true, emoji: 1 },
  { person: 4, at: 42.75, text: "the hat really ties the whole team together", emoji: 2 },
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
  const [{ phase, cycle, reactions }, setDemo] = useState<{ phase: number; cycle: number; reactions: Record<string, boolean> }>({ phase: 3, cycle: 0, reactions: {} });
  const [paused, setPaused] = useState(false);
  const [interacting, setInteracting] = useState(false);
  const [focused, setFocused] = useState(false);
  const [reducedMotion, setReducedMotion] = useState(false);
  const [rosterOpen, setRosterOpen] = useState(true);
  const [picker, setPicker] = useState<number>();
  // Stable server/first-client render; randomize only once the local demo is
  // ready, then once per loop, never on ordinary React renders.
  const timing = useMemo(() => createDemoTiming(people.map((person) => person.speech), messages.map((message) => message.at), ready ? Math.random : () => .5), [ready, cycle]);
  const advance = useCallback((seconds: number) => {
    setDemo((current) => {
      const elapsed = current.phase + seconds;
      const loops = Math.floor(elapsed / cycleLength);
      return { phase: elapsed % cycleLength, cycle: current.cycle + loops, reactions: loops > 0 ? {} : current.reactions };
    });
  }, []);

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
    // Sample elapsed time rather than snapping every effect to a shared beat.
    let previous = performance.now();
    const timer = window.setInterval(() => {
      const now = performance.now();
      advance((now - previous) / 1000);
      previous = now;
    }, 75);
    return () => window.clearInterval(timer);
  }, [paused, interacting, focused, reducedMotion, picker, advance]);

  const present = people.map((person, index) => {
    const last = activity.filter((event) => event.person === index && event.at <= phase).at(-1);
    const voice = last?.voice ?? index < 2;
    return { ...person, online: last?.online ?? index < 3, voice, speaking: voice && timing.speech[index].some(([start, end]) => phase >= start && phase < end) };
  });
  const onlinePeople = present.filter((person) => person.online);
  const voicePeople = present.filter((person) => person.voice);
  const latestActivity = activity.filter((event) => event.at <= phase).at(-1)!;
  const timedMessages = messages.map((message, index) => ({ ...message, ...timing.messages[index] }));
  const visibleMessages = timedMessages.filter((message) => message.at <= phase).slice(-3);
  const typingPeople = [...new Set(timedMessages.filter((message) => message.at > phase && message.typing <= phase && present[message.person].online).map((message) => people[message.person].name))];
  const toggle = (key: string) => setDemo((current) => ({ ...current, reactions: { ...current.reactions, [key]: !current.reactions[key] } }));

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
            <aside className="sim-sidebar people-panel spaces-room">
              <div className="sidebar-channels">
                <nav className="channel-navigation" aria-label="Simulated channels">
                  <header><div className="sim-brand">Caper<ChevronDown aria-hidden="true" /></div></header>
                  <div className="channel-section-heading sim-section-title"><ChevronDown aria-hidden="true" />Channels<span className="section-count">2</span></div>
                  <ul>
                    <li data-voice="">
                      <div className="channel-line">
                        <div className="channel-select sim-channel" aria-current="page"><Hash aria-hidden="true" /><span>general</span></div>
                        <span className="channel-voice" data-live-control>
                          <button className="voice-stack" type="button" aria-expanded={rosterOpen} aria-controls="sim-voice-roster" aria-label={`${voicePeople.length} in demo voice. ${rosterOpen ? "Hide" : "Show"} participants.`} onClick={() => setRosterOpen(!rosterOpen)}>
                            <span className="voice-stack-faces" aria-hidden="true">{voicePeople.slice(0, 3).map((person) => <span className={`voice-stack-avatar${person.speaking && !rosterOpen ? " speaking" : ""}`} key={person.name}>{person.name[0]}</span>)}{voicePeople.length > 3 && <small>+{voicePeople.length - 3}</small>}</span>
                            <ChevronDown aria-hidden="true" />
                          </button>
                        </span>
                      </div>
                      <div className="voice-occupants" data-open={rosterOpen ? "" : undefined} id="sim-voice-roster">
                        <div className="voice-occupants-inner" inert={!rosterOpen}>
                          <ul className="sim-people" aria-label="People in demo voice in general">
                            {voicePeople.map((person) => <li className="sim-person participant" data-speaking={person.speaking ? "" : undefined} key={person.name} aria-label={`${person.name}${person.speaking ? ", speaking" : ""}`}>
                              <span className="participant-avatar"><span className={`avatar sim-voice-avatar ${person.speaking ? "speaking" : "quiet"}`}><Avatar person={person} /></span></span>
                              <span className="participant-name"><strong>{person.name}</strong></span>
                            </li>)}
                          </ul>
                        </div>
                      </div>
                    </li>
                    <li><div className="channel-select"><Hash aria-hidden="true" /><span>tomato-soup</span></div></li>
                  </ul>
                </nav>
              </div>
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
                          // Simulated readers react after the message appears,
                          // never bundled with its first frame. Viewer clicks
                          // still take effect immediately.
                          const count = (index === message.emoji ? message.reactions.filter((at) => at <= phase).length : 0) + Number(mine);
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
              <footer className="sim-footer" data-live-control><a className="sim-composer" href="/spaces">Join to message #general</a><span>Try a reaction · just for fun</span><div className="sim-playback"><button type="button" aria-label="Next demo moment" onClick={() => { setPicker(undefined); advance(1); }}><SkipForward aria-hidden="true" />Next</button><button type="button" disabled={reducedMotion} aria-label={paused ? "Play demo" : "Pause demo"} aria-pressed={paused || reducedMotion} onClick={() => setPaused(!paused)}>{paused || reducedMotion ? <Play aria-hidden="true" /> : <Pause aria-hidden="true" />}{reducedMotion ? "Reduced motion" : paused ? "Play" : "Pause"}</button></div></footer>
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
