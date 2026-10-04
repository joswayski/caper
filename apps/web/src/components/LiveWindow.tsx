import { useEffect, useLayoutEffect, useMemo, useRef, useState, type CSSProperties } from "react";
import { AudioLines, ChevronDown, Hash, Headphones, MessageCircle, Mic, MoreHorizontal, PhoneOff, Plus, Search, Settings, Speech, Users } from "lucide-react";
import { avatarUrl } from "../account/avatar";
import { sessionDuration } from "../media/session-duration";
import PresenceDot from "./PresenceDot";
import { attachLiveMotion } from "./liveMotion";
import { createDemoTiming } from "./demoTiming";
import "../pages/call.css";
import "../spaces/spaces.css";
import "../chat/chat.css";
import "./live-window.css";

const people = [
  { name: "Maya", avatar: 0, hue: 0, note: "collecting tiny hats", speech: [[0, 4.25], [6.5, 9], [12, 13.75], [18.25, 20.5], [26, 29.25], [40.25, 43]] },
  { name: "Theo", avatar: 1, hue: 0, note: "on aux duty", speech: [[1.5, 5.5], [8, 10.75], [14.5, 18], [27.25, 30.5], [35.5, 37.25], [43, 46]] },
  { name: "June", avatar: 2, hue: 0, note: "here for the memes", speech: [[2.75, 6.25], [10.5, 12.75], [15.75, 16.5], [22.5, 25], [33, 36.25], [44.25, 46.5]] },
  { name: "Leo", avatar: 3, hue: 0, note: "one more game?", speech: [[10.25, 13], [15.5, 19.25], [23, 27.5], [30.25, 32.5], [36, 39.25], [46, 48.5]] },
  { name: "Noor", avatar: 0, hue: 100, note: "snack coordinator", speech: [[18.5, 22.75], [25.5, 28], [29, 33.5], [37.25, 40.75], [44, 47]] },
  { name: "Sam", avatar: 2, hue: 175, note: "always has a fun fact", speech: [[38.5, 40.5], [42.25, 45.75], [47.25, 48.25]] },
];

const messages = [
  { person: 0, at: 0, text: "okay, this made my entire morning", meme: true, emoji: 0, reactionCount: 3 },
  { person: 1, at: 1.25, text: "the tiny hat is doing a lot of work here" },
  { person: 2, at: 3, text: "new group photo. no objections please" },
  { person: 3, at: 4.75, text: "i leave for TWO minutes 😂", emoji: 4, reactionCount: 1 },
  { person: 0, at: 7.5, text: "anyway… who’s up for a game?" },
  { person: 4, at: 9.25, text: "did someone say game night? 👀" },
  { person: 1, at: 11.25, text: "already here. bringing the playlist 🎶" },
  { person: 3, at: 11.75, text: "save me a spot" },
  { person: 2, at: 12.5, text: "rule one: nobody lets me choose the map", emoji: 0, reactionCount: 2 },
  { person: 5, at: 16.25, text: "hello hello! what did i miss?" },
  { person: 4, at: 16.75, text: "a tiny hat and some very serious planning" },
  { person: 5, at: 20.25, text: "excellent. i brought snacks 🍿" },
  { person: 0, at: 22, text: "brb, getting tea. please behave" },
  { person: 3, at: 24, text: "no promises", emoji: 0, reactionCount: 1 },
  { person: 0, at: 26.5, text: "back! the kettle was faster than this lobby" },
  { person: 2, at: 28.25, text: "we’re waiting for theo’s 400-song playlist" },
  { person: 1, at: 30.5, text: "it’s called having range" },
  { person: 4, at: 32.25, text: "i’m requesting exactly one ridiculous song" },
  { person: 5, at: 34.75, text: "fun fact: capybaras are excellent swimmers", emoji: 3, reactionCount: 2 },
  { person: 3, at: 36.25, text: "so our mascot can carry us on the water map?" },
  { person: 5, at: 38, text: "joining voice to defend this theory" },
  { person: 0, at: 40.5, text: "this is now a capybara appreciation channel", meme: true, emoji: 1, reactionCount: 3 },
  { person: 4, at: 42.75, text: "the hat really ties the whole team together" },
  { person: 2, at: 45, text: "back with cookies. let’s gooo", emoji: 3, reactionCount: 2 },
];

type DemoMessage = typeof messages[number] & { cycle: number; reactions: number[] };

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
  return <span className="sim-avatar" role="img" aria-label={`${person.name}'s caper avatar`} title={`${person.name} · ${person.note}`} style={{ backgroundImage: `url('${avatarUrl(person.avatar)}')`, filter: person.hue ? `hue-rotate(${person.hue}deg)` : undefined }} />;
}

/** A local-only, deliberately labelled illustration of a lively Caper room. */
export default function LiveWindow() {
  const stageRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const messagesRef = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  const [ready, setReady] = useState(false);
  const [{ phase, cycle, history }, setDemo] = useState<{ phase: number; cycle: number; history: DemoMessage[] }>({ phase: 3, cycle: 0, history: [] });
  const [reducedMotion, setReducedMotion] = useState(false);
  // Stable server/first-client render; randomize only once the local demo is
  // ready, then once per loop, never on ordinary React renders.
  const timing = useMemo(() => createDemoTiming(people.map((person) => person.speech), messages.map((message) => message.at), ready ? Math.random : () => .5), [ready, cycle]);
  const timedMessages = useMemo(() => messages.map((message, index) => ({ ...message, ...timing.messages[index], reactions: message.emoji === undefined ? [] : timing.messages[index].reactions.slice(0, message.reactionCount), cycle })), [timing, cycle]);

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

  useLayoutEffect(() => {
    const viewport = messagesRef.current;
    if (!viewport) return;
    const follow = () => {
      if (following.current) viewport.scrollTop = viewport.scrollHeight;
    };
    const observer = new ResizeObserver(follow);
    observer.observe(viewport);
    observer.observe(viewport.firstElementChild!);
    follow();
    return () => observer.disconnect();
  }, []);

  useEffect(() => {
    const media = window.matchMedia("(prefers-reduced-motion: reduce)");
    const update = () => setReducedMotion(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  useEffect(() => {
    if (reducedMotion) return;
    // Sample elapsed time rather than snapping every effect to a shared beat.
    let previous = performance.now();
    const timer = window.setInterval(() => {
      const now = performance.now();
      const seconds = (now - previous) / 1000;
      previous = now;
      setDemo((current) => {
        const elapsed = current.phase + seconds;
        const loops = Math.floor(elapsed / cycleLength);
        return {
          phase: elapsed % cycleLength,
          cycle: current.cycle + loops,
          history: loops > 0 ? [...current.history, ...timedMessages] : current.history,
        };
      });
    }, 75);
    return () => window.clearInterval(timer);
  }, [reducedMotion, timedMessages]);

  const present = people.map((person, index) => {
    const last = activity.filter((event) => event.person === index && event.at <= phase).at(-1);
    const voice = last?.voice ?? index < 2;
    return { ...person, online: last?.online ?? index < 3, voice, speaking: voice && timing.speech[index].some(([start, end]) => phase >= start && phase < end) };
  });
  const onlinePeople = present.filter((person) => person.online);
  const voicePeople = present.filter((person) => person.voice);
  const visibleMessages = [...history, ...timedMessages.filter((message) => message.at <= phase)];
  const typingPeople = [...new Set(timedMessages.filter((message) => message.at > phase && message.typing <= phase && present[message.person].online).map((message) => people[message.person].name))];

  return (
    <div className="live-stage" ref={stageRef} suppressHydrationWarning data-ready={ready ? "" : undefined}>
      <div className="live-glow" aria-hidden="true" />
      <div className="live-scene" ref={sceneRef} suppressHydrationWarning>
        <div className="live-shadow" aria-hidden="true" />
        {[5, 4, 3, 2, 1].map((depth) => <div key={depth} className="live-slab" style={{ "--z": -depth * 5 } as CSSProperties} aria-hidden="true" />)}
        <div className="live-window">
          <div className="sim-demo spaces-room navigation-open" aria-label="Simulated Caper conversation">
            <aside className="sim-rail" aria-label="Demo space"><span>C</span><span className="sim-add-space" aria-hidden="true"><Plus /></span></aside>
            <aside className="sim-sidebar people-panel">
              <div className="sidebar-channels">
                <nav className="channel-navigation" aria-label="Simulated channels">
                  <header><div className="sim-brand">Caper<ChevronDown aria-hidden="true" /></div></header>
                  <div className="channel-section-heading sim-section-title"><span className="sim-section-toggle"><ChevronDown aria-hidden="true" />Channels<span className="section-count">2</span></span><span className="sim-section-actions" aria-hidden="true"><Plus /><MoreHorizontal /></span></div>
                  <ul className="sim-channel-list">
                    <li data-voice="">
                      <div className="channel-line">
                        <div className="channel-select sim-channel" aria-current="page"><Hash aria-hidden="true" /><span>general</span><span className="voice-session-timer" aria-label="Simulated voice session duration">{sessionDuration(0, (cycle * cycleLength + phase) * 1_000)}</span></div>
                        <span className="channel-manage" aria-hidden="true"><MoreHorizontal /></span>
                        <span className="channel-voice">
                          <span className="voice-stack" aria-label={`${voicePeople.length} in demo voice`}>
                            <ChevronDown aria-hidden="true" />
                            <span className="voice-stack-faces" aria-hidden="true">{voicePeople.slice(0, 3).map((person) => <span className={`voice-stack-avatar${person.speaking ? " speaking" : ""}`} key={person.name}><Avatar person={person} /></span>)}{voicePeople.length > 3 && <small>+{voicePeople.length - 3}</small>}</span>
                            <span className="voice-stack-count">{voicePeople.length} in voice</span>
                          </span>
                          <span className="channel-join-slot" />
                        </span>
                      </div>
                      <div className="voice-occupants" data-open="">
                        <div className="voice-occupants-inner">
                          <ul className="sim-people" aria-label="People in demo voice in general">
                            {voicePeople.map((person) => <li className="sim-person participant" data-speaking={person.speaking ? "" : undefined} key={person.name} aria-label={`${person.name}${person.speaking ? ", speaking" : ""}`}>
                              <span className="participant-avatar"><span className={`avatar sim-voice-avatar ${person.speaking ? "speaking" : "quiet"}`}><Avatar person={person} /></span></span>
                              <span className="participant-name"><strong>{person.name}</strong></span>
                            </li>)}
                          </ul>
                        </div>
                      </div>
                    </li>
                    <li><div className="channel-line">
                      <div className="channel-select"><Hash aria-hidden="true" /><span>feedback</span></div>
                      <span className="channel-manage" aria-hidden="true"><MoreHorizontal /></span>
                      <span className="channel-voice"><span className="channel-join voice-button"><Speech aria-hidden="true" />Join voice</span></span>
                    </div></li>
                  </ul>
                  <section className="direct-section sim-direct" aria-label="Simulated direct messages">
                    <div className="channel-section-heading"><span className="direct-section-title"><MessageCircle aria-hidden="true" />Direct messages</span><Plus aria-hidden="true" /></div>
                    <ul><li><div className="channel-select direct-select"><span className="direct-avatar"><Avatar person={people[0]} /></span><span>Maya</span><small>you</small></div></li></ul>
                    <div className="channel-select direct-action"><Plus aria-hidden="true" /><span>Invite people</span></div>
                  </section>
                  <div className="browse-channels"><Search aria-hidden="true" />Browse channels</div>
                </nav>
              </div>
              <div className="voice-panel sim-voice-panel">
                {present[0].voice && <div className="voice-dock" role="img" aria-label="Simulated voice connection to general in Caper">
                  <div className="connected-channel" data-phase="connected">
                    <span className="voice-dock-channel"><AudioLines aria-hidden="true" /><span><strong>Voice connected</strong><small>general / Caper</small></span></span>
                    <span className="voice-hangup" aria-hidden="true"><PhoneOff /></span>
                  </div>
                </div>}
                <div className="call-account sim-account" role="img" aria-label="Demo profile: Maya, online, with microphone, headphones and user settings">
                  <span className="account-profile">
                    <span className="account-avatar"><Avatar person={people[0]} /><PresenceDot status="online" /></span>
                    <strong className="account-name">Maya</strong>
                  </span>
                  <span className="voice-action-group"><span className="voice-icon-button"><Mic aria-hidden="true" /></span><span className="device-menu"><span className="call-settings-trigger"><ChevronDown aria-hidden="true" /></span></span></span>
                  <span className="voice-action-group"><span className="voice-icon-button"><Headphones aria-hidden="true" /></span><span className="device-menu"><span className="call-settings-trigger"><ChevronDown aria-hidden="true" /></span></span></span>
                  <span className="call-settings-trigger"><Settings aria-hidden="true" /></span>
                </div>
              </div>
            </aside>
            <section className="sim-chat chat-panel">
              <header className="chat-heading"><h2 className="chat-channel-title"># general</h2><span className="member-list-toggle" aria-hidden="true"><Users /></span></header>
              <div className="sim-messages" ref={messagesRef} role="log" aria-label="Simulated message history" aria-live="off" tabIndex={0} onScroll={(event) => {
                const viewport = event.currentTarget;
                following.current = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight < 48;
              }}>
                <div className="sim-message-list">
                  {visibleMessages.map((message) => {
                    const item = message.emoji === undefined ? undefined : emoji[message.emoji];
                    const count = message.reactions.filter((at) => at + message.cycle * cycleLength <= phase + cycle * cycleLength).length;
                    return <article className="sim-message chat-message" key={`${message.cycle}:${message.at}`}>
                      <Avatar person={people[message.person]} />
                      <div><header><strong>{people[message.person].name}</strong><time>just now</time></header><p>{message.text}</p>{message.meme && <img className="sim-meme" src="/images/demo-tiny-hat.webp" width="384" height="384" draggable={false} alt="A capybara wearing a tiny hat. Caption: Tiny hat. Huge energy." />}
                        <div className="sim-reactions chat-reactions">
                          {item && count > 0 && <span className="sim-reaction chat-reaction" role="img" aria-label={`${item.text}, ${count} ${count === 1 ? "reaction" : "reactions"}`}><img src={`/images/demo-emoji/${item.code}.svg`} draggable={false} alt="" /><span>{count}</span></span>}
                        </div>
                      </div>
                    </article>;
                  })}
                </div>
              </div>
              <div className="sim-typing chat-typing"><span className="chat-typing-content" data-visible={typingPeople.length > 0}>{typingPeople.length > 0 && <><span className="chat-typing-dots" aria-hidden="true"><i /><i /><i /></span><span>{typingPeople.join(" and ")} {typingPeople.length === 1 ? "is" : "are"} typing</span></>}</span></div>
              <footer className="sim-footer chat-composer" aria-hidden="true"><div className="sim-composer">Message #general</div></footer>
            </section>
            <aside className="sim-members space-member-presence" aria-label="Simulated members">
              <h2 className="sim-members-heading member-presence-heading"><span>Members</span><span className="section-count">{onlinePeople.length}</span></h2>
              <div><ul>{onlinePeople.map((person) => <li className="sim-member" key={person.name}><span className="member-presence-avatar"><Avatar person={person} /><PresenceDot status="online" /></span><span><strong>{person.name}</strong></span></li>)}</ul></div>
            </aside>
          </div>
        </div>
        <a className="live-activator" data-live-activator data-live-control href="/spaces" aria-label="Join Caper">
          <span className="live-invite"><i aria-hidden="true" /><span className="live-invite-fine">Click to join</span><span className="live-invite-coarse">Tap to join</span></span>
        </a>
      </div>
    </div>
  );
}
