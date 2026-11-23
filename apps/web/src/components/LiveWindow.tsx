import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Link } from "@tanstack/react-router";
import {
  AudioLines,
  Bell,
  ChevronDown,
  Forward,
  Hash,
  Headphones,
  MessageCircle,
  Mic,
  MoreHorizontal,
  PhoneOff,
  Pin,
  Play,
  Plus,
  Search,
  Settings,
  Smile,
  Users,
  Volume2,
} from "lucide-react";
import { avatarUrl } from "../account/avatar";
import LinkedText from "../chat/LinkedText";
import { JumpToast } from "../chat/JumpToLatest";
import { liveNames } from "../media/live-now";
import { sessionDuration } from "../media/session-duration";
import PresenceDot from "./PresenceDot";
import { attachLiveMotion } from "./liveMotion";
import { createDemoTiming } from "./demoTiming";
import "../pages/call.css";
import "../spaces/spaces.css";
import "../chat/chat.css";
import "../spaces/notifications-inbox.css";
import "./picture.css";
import "./live-window.css";

const people = [
  {
    name: "Maya",
    avatar: 0,
    hue: 0,
    note: "collecting tiny hats",
    speech: [
      [0, 4.25],
      [6.5, 9],
      [12, 13.75],
      [18.25, 20.5],
      [26, 29.25],
      [40.25, 43],
    ],
  },
  {
    name: "Theo",
    avatar: 1,
    hue: 0,
    note: "on aux duty",
    speech: [
      [1.5, 5.5],
      [8, 10.75],
      [14.5, 18],
      [27.25, 30.5],
      [35.5, 37.25],
      [43, 46],
    ],
  },
  {
    name: "June",
    avatar: 2,
    hue: 0,
    note: "here for the memes",
    speech: [
      [2.75, 6.25],
      [10.5, 12.75],
      [15.75, 16.5],
      [22.5, 25],
      [33, 36.25],
      [44.25, 46.5],
    ],
  },
  {
    name: "Leo",
    avatar: 3,
    hue: 0,
    note: "one more game?",
    speech: [
      [10.25, 13],
      [15.5, 19.25],
      [23, 27.5],
      [30.25, 32.5],
      [36, 39.25],
      [46, 48.5],
    ],
  },
  {
    name: "Noor",
    avatar: 0,
    hue: 100,
    note: "snack coordinator",
    speech: [
      [18.5, 22.75],
      [25.5, 28],
      [29, 33.5],
      [37.25, 40.75],
      [44, 47],
    ],
  },
  {
    name: "Sam",
    avatar: 2,
    hue: 175,
    note: "always has a fun fact",
    speech: [
      [38.5, 40.5],
      [42.25, 45.75],
      [47.25, 48.25],
    ],
  },
];

type DemoPost = {
  person: number;
  at: number;
  text: string;
  meme?: boolean;
  emoji?: number;
  reactionCount?: number;
  pinnedBy?: number;
  edit?: { at: number; text: string };
  thread?: { at: number; person: number }[];
  forward?: { person: number; text: string; replies: number[] };
  /** A voice message: its length and the transcript that lands a moment after it. */
  voice?: { duration: string; transcript: string };
};

// Who talks where: people in voice talk to each other there, so the typing
// comes from people outside voice (before they join, or after stepping out).
// People in voice only drop the things that belong in text: memes and pins.
const messages: DemoPost[] = [
  {
    person: 0,
    at: 0,
    text: "okay, this made my entire morning",
    meme: true,
    emoji: 0,
    reactionCount: 3,
    thread: [
      { at: 2, person: 3 },
      { at: 8, person: 4 },
      { at: 16.5, person: 5 },
    ],
  },
  {
    person: 3,
    at: 1.25,
    text: "game night at 8. who’s in? 🎲",
    pinnedBy: 0,
    edit: { at: 6, text: "game night at 8:30. who’s in? 🎲" },
  },
  { person: 4, at: 3, text: "in! sounds like the voice crew started without us" },
  { person: 3, at: 4.75, text: "they ALWAYS do 😂", emoji: 4, reactionCount: 1 },
  { person: 4, at: 7.5, text: "anyway… whose turn is it to pick the game?" },
  { person: 3, at: 9.25, text: "rule one: **nobody** lets june choose the map", emoji: 0, reactionCount: 2 },
  { person: 4, at: 11.25, text: "the lava one. never again" },
  { person: 3, at: 11.75, text: "save me a spot, hopping in voice" },
  { person: 4, at: 12.5, text: "_rude_. leaving me alone in here" },
  { person: 5, at: 16.25, text: "hello hello! what did i miss?" },
  {
    person: 4,
    at: 16.75,
    text: "bringing this over from #feedback",
    forward: { person: 2, text: "petition for everyone to wear a tiny hat to game night", replies: [21, 29] },
  },
  { person: 5, at: 20.25, text: "excellent. i brought snacks 🍿" },
  { person: 0, at: 22, text: "brb, getting tea. please behave" },
  { person: 4, at: 24, text: "no promises", emoji: 0, reactionCount: 1 },
  { person: 0, at: 26.5, text: "back! the kettle was faster than this lobby" },
  { person: 5, at: 28.25, text: "we’re waiting for theo’s 400-song playlist" },
  { person: 1, at: 30.5, text: "it’s called having range" },
  { person: 4, at: 32.25, text: "i’m requesting exactly one ridiculous song: ||the capybara song, on loop||" },
  {
    person: 5,
    at: 34.75,
    text: "fun fact: capybaras are excellent swimmers https://en.wikipedia.org/wiki/Capybara",
    emoji: 3,
    reactionCount: 2,
  },
  { person: 4, at: 36.25, text: "so our mascot can carry us on the water map?" },
  {
    person: 4,
    at: 37.25,
    text: "",
    voice: {
      duration: "0:07",
      transcript: "okay real talk. whoever loses tonight is on snack duty next week. no take-backs.",
    },
  },
  { person: 5, at: 38, text: "joining voice to defend this theory" },
  { person: 0, at: 40.5, text: "this is now a capybara appreciation channel", meme: true, emoji: 1, reactionCount: 3 },
  { person: 4, at: 42.75, text: "okay fine, joining. the hat really ties the whole team together" },
  { person: 2, at: 45, text: "back with cookies. let’s gooo", emoji: 3, reactionCount: 2 },
];

type DemoMessage = (typeof messages)[number] & { cycle: number; reactions: number[] };

// Arrivals and voice changes belong only to this local illustration. Each
// author is out of voice when they type (see messages above).
const activity = [
  { at: 0, person: 0, online: true, voice: true },
  { at: 0, person: 1, online: true, voice: true },
  { at: 0, person: 2, online: true, voice: true },
  { at: 0, person: 3, online: true, voice: false },
  { at: 0, person: 4, online: true, voice: false },
  { at: 13, person: 3, online: true, voice: true },
  { at: 15, person: 5, online: true, voice: false },
  { at: 21.5, person: 0, online: true, voice: false },
  { at: 27, person: 0, online: true, voice: true },
  { at: 29.5, person: 1, online: true, voice: false },
  { at: 35, person: 1, online: true, voice: true },
  { at: 38.5, person: 5, online: true, voice: true },
  { at: 39, person: 2, online: false, voice: false },
  { at: 43.5, person: 4, online: true, voice: true },
  { at: 44, person: 2, online: true, voice: false },
  { at: 46, person: 2, online: true, voice: true },
];
const cycleLength = 49;
const initialPhase = 3;
const timeFormatter = new Intl.DateTimeFormat(undefined, { hour: "numeric", minute: "2-digit" });

const emoji = [
  { text: "😂", code: "1f602" },
  { text: "❤️", code: "2764" },
  { text: "🔥", code: "1f525" },
  { text: "👏", code: "1f44f" },
  { text: "💀", code: "1f480" },
];

// Owner-set icons on the rail's tiles, as real spaces can have.
const caperIcon = "/images/demo-emoji/1f913.svg";
// Other spaces in the rail: one with new messages, one with mentions of you.
const otherSpaces = [
  { name: "Iced out", icon: "/images/demo-spaces/dario-bling.webp", unread: true, mentions: 2 },
  { name: "Smackdown", icon: "/images/demo-spaces/jbl-cowboy.webp", unread: true, mentions: 0 },
];

function SpaceTile({ name, icon }: { name: string; icon: string }) {
  return (
    <>
      <span>{name[0]}</span>
      <span className="caper-picture" data-state="ready">
        <img src={icon} alt="" draggable={false} />
      </span>
    </>
  );
}
// A fixed, speech-like waveform for the voice message.
const voiceBars = Array.from({ length: 40 }, (_, index) =>
  Math.max(0.12, Math.abs(Math.sin(index * 0.9) * Math.cos(index * 0.37)) * 0.9 + (index % 3) * 0.04),
);
const transcriptDelay = 2.5;

const messageKey = (message: DemoMessage) => `${message.cycle}:${message.at}`;

function Avatar({ person }: { person: (typeof people)[number] }) {
  return (
    <span
      className="sim-avatar"
      role="img"
      aria-label={`${person.name}'s caper avatar`}
      title={`${person.name} · ${person.note}`}
      style={{
        backgroundImage: `url('${avatarUrl(person.avatar)}')`,
        filter: person.hue ? `hue-rotate(${person.hue}deg)` : undefined,
      }}
    />
  );
}

/** A local-only, deliberately labelled illustration of a lively Caper room. */
export default function LiveWindow({ joinTo }: { joinTo: "/login" | "/profile" | "/spaces" }) {
  const stageRef = useRef<HTMLDivElement>(null);
  const sceneRef = useRef<HTMLDivElement>(null);
  const messagesRef = useRef<HTMLDivElement>(null);
  const following = useRef(true);
  // The newest message when the reader scrolled away from the live end.
  const [awayFrom, setAwayFrom] = useState<string>();
  // Wall-clock time of the loop's start, so messages show times like the app's.
  const [startedAt, setStartedAt] = useState<number>();
  const [ready, setReady] = useState(false);
  const [{ phase, cycle, history }, setDemo] = useState<{ phase: number; cycle: number; history: DemoMessage[] }>({
    phase: initialPhase,
    cycle: 0,
    history: [],
  });
  const [reducedMotion, setReducedMotion] = useState(false);
  // Stable server/first-client render; randomize only once the local demo is
  // ready, then once per loop, never on ordinary React renders.
  const timing = useMemo(
    () =>
      createDemoTiming(
        people.map((person) => person.speech),
        messages.map((message) => message.at),
        ready ? Math.random : () => 0.5,
      ),
    [ready, cycle],
  );
  const timedMessages = useMemo(
    () =>
      messages.map((message, index) => ({
        ...message,
        ...timing.messages[index],
        reactions: message.emoji === undefined ? [] : timing.messages[index].reactions.slice(0, message.reactionCount),
        cycle,
      })),
    [timing, cycle],
  );

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
    setStartedAt(Date.now() - initialPhase * 1_000);
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
    const stage = stageRef.current;
    // Pause off screen or in a hidden tab: the clock simply stops, so the demo
    // resumes where it was instead of re-rendering the whole room 13 times a second.
    let visible = true;
    const observer = stage
      ? new IntersectionObserver(([entry]) => {
          visible = entry?.isIntersecting ?? true;
        })
      : undefined;
    if (stage) observer?.observe(stage);
    // Sample elapsed time rather than snapping every effect to a shared beat.
    let previous = performance.now();
    const timer = window.setInterval(() => {
      const now = performance.now();
      const seconds = (now - previous) / 1000;
      previous = now;
      if (!visible || document.hidden) return;
      setDemo((current) => {
        const elapsed = current.phase + seconds;
        const loops = Math.floor(elapsed / cycleLength);
        return {
          phase: elapsed % cycleLength,
          cycle: current.cycle + loops,
          // Keep one earlier loop for scrollback; unbounded history slowly made
          // a long-open homepage heavier and the animation janky.
          history: loops > 0 ? [...current.history, ...timedMessages].slice(-messages.length) : current.history,
        };
      });
    }, 75);
    return () => {
      window.clearInterval(timer);
      observer?.disconnect();
    };
  }, [reducedMotion, timedMessages]);

  const present = people.map((person, index) => {
    const last = activity.filter((event) => event.person === index && event.at <= phase).at(-1);
    const voice = last?.voice ?? false;
    return {
      ...person,
      online: last?.online ?? false,
      voice,
      speaking: voice && timing.speech[index].some(([start, end]) => phase >= start && phase < end),
    };
  });
  const onlinePeople = present.filter((person) => person.online);
  // People in voice in their (approximate) join order: you, Maya, first, as on your call card.
  const voicePeople = present.filter((person) => person.voice);
  const inCall = present[0].voice;
  const voiceTimer = (
    <span className="voice-session-timer">{sessionDuration(0, (cycle * cycleLength + phase) * 1_000)}</span>
  );
  const visibleMessages = [...history, ...timedMessages.filter((message) => message.at <= phase)];
  const latestKey = useRef<string>(undefined);
  latestKey.current = visibleMessages.length ? messageKey(visibleMessages.at(-1)!) : undefined;
  const jumping = useRef(false);
  // As in the app, Jump to latest counts what other people posted while you read.
  const awayIndex =
    awayFrom === undefined ? -1 : visibleMessages.findIndex((message) => messageKey(message) === awayFrom);
  const newWhileAway =
    awayIndex === -1 ? 0 : visibleMessages.slice(awayIndex + 1).filter((message) => message.person !== 0).length;
  const typingPeople = [
    ...new Set(
      timedMessages
        .filter(
          // Never yourself (Maya), and nobody in voice: they say it out loud instead.
          (message) =>
            message.person !== 0 &&
            !message.voice &&
            !message.meme &&
            message.at > phase &&
            message.typing <= phase &&
            present[message.person].online &&
            !present[message.person].voice,
        )
        .map((message) => people[message.person].name),
    ),
  ];
  // Same wording and fade-out as the real conversation's typing line (Chat.tsx).
  const typingLabel =
    typingPeople.length > 2
      ? "Several people are typing…"
      : typingPeople.length
        ? `${typingPeople.join(" and ")} ${typingPeople.length === 1 ? "is" : "are"} typing…`
        : "";
  const [displayedTypingLabel, setDisplayedTypingLabel] = useState("");
  useEffect(() => {
    if (typingLabel) {
      setDisplayedTypingLabel(typingLabel);
      return;
    }
    const timer = setTimeout(() => setDisplayedTypingLabel(""), 180);
    return () => clearTimeout(timer);
  }, [typingLabel]);

  return (
    <div className="live-stage" ref={stageRef} suppressHydrationWarning data-ready={ready ? "" : undefined}>
      <div className="live-glow" aria-hidden="true" />
      <div className="live-scene" ref={sceneRef} suppressHydrationWarning>
        <div className="live-shadow" aria-hidden="true" />
        <div className="live-chassis" aria-hidden="true">
          <span data-side="top" />
          <span data-side="right" />
          <span data-side="bottom" />
          <span data-side="left" />
          {["top-left", "top-right", "bottom-right", "bottom-left"].map((corner) => (
            <span className="live-chassis-corner" data-corner={corner} key={corner}>
              <span />
              <span />
              <span />
            </span>
          ))}
        </div>
        <div className="live-window">
          <div className="sim-demo spaces-room" aria-label="Simulated Caper conversation">
            <nav
              className="space-rail"
              role="img"
              aria-label={`Demo spaces: Caper${inCall ? ", connected to voice" : ""}; Iced out, 2 mentions; Smackdown, unread messages`}
            >
              <div className="space-rail-item" data-active="true" data-voice={inCall ? "" : undefined}>
                <button type="button" tabIndex={-1}>
                  <SpaceTile name="Caper" icon={caperIcon} />
                  {inCall && <Volume2 className="space-rail-voice" data-connected="" aria-hidden="true" />}
                </button>
              </div>
              {otherSpaces.map((space) => (
                <div className="space-rail-item" data-active="false" key={space.name}>
                  {space.unread && <i className="space-unread" aria-hidden="true" />}
                  <button type="button" tabIndex={-1}>
                    <SpaceTile name={space.name} icon={space.icon} />
                    {space.mentions > 0 && <i className="space-mentions">{space.mentions}</i>}
                  </button>
                </div>
              ))}
              <button className="add-space" type="button" tabIndex={-1}>
                <Plus aria-hidden="true" />
              </button>
            </nav>
            <aside className="sim-sidebar people-panel">
              <div className="sidebar-channels">
                <nav className="channel-navigation" aria-label="Simulated channels">
                  <header>
                    <div className="sim-brand">
                      Caper
                      <ChevronDown aria-hidden="true" />
                    </div>
                  </header>
                  <div className="channel-section-heading sim-section-title">
                    <span className="sim-section-toggle">
                      <ChevronDown aria-hidden="true" />
                      Channels<span className="section-count">2</span>
                    </span>
                    <span className="sim-section-actions" aria-hidden="true">
                      <Plus />
                      <MoreHorizontal />
                    </span>
                  </div>
                  <ul className="sim-channel-list">
                    {/* As in the app, #general's row grows into a card while people talk there: your
                        call while Maya is in voice, the room with Join while she steps out. */}
                    <li data-live="">
                      <div
                        className={`live-card ${inCall ? "live-call" : "live-room"}`}
                        data-phase={inCall ? "connected" : undefined}
                        data-row=""
                        role="img"
                        aria-label={
                          inCall
                            ? `Simulated call in general with ${liveNames(voicePeople.slice(1).map((person) => person.name))}; mute, deafen and Leave`
                            : `Simulated voice in general: ${voicePeople.length} in voice, with Join`
                        }
                      >
                        <div className="channel-line">
                          <div className="channel-select sim-channel" aria-current="page">
                            <Hash aria-hidden="true" />
                            <span>general</span>
                            {!inCall && (
                              <span className="channel-live">
                                <AudioLines aria-hidden="true" />
                                <span>{voicePeople.length}</span>
                              </span>
                            )}
                          </div>
                          {inCall && <span className="live-call-status">{voiceTimer}</span>}
                          <span className="channel-manage" aria-hidden="true">
                            <MoreHorizontal />
                          </span>
                        </div>
                        {inCall ? (
                          <>
                            <ul className="live-faces">
                              {voicePeople.map((person) => (
                                <li key={person.name}>
                                  <span className="live-face">
                                    <span className="live-avatar" data-speaking={person.speaking ? "" : undefined}>
                                      <Avatar person={person} />
                                    </span>
                                  </span>
                                </li>
                              ))}
                            </ul>
                            <span className="live-people-count">
                              {voicePeople.length} {voicePeople.length === 1 ? "person" : "people"}
                            </span>
                            <div className="live-controls">
                              <span className="voice-icon-button live-toggle">
                                <Mic aria-hidden="true" />
                              </span>
                              <span className="voice-icon-button live-toggle">
                                <Headphones aria-hidden="true" />
                              </span>
                              <span className="voice-hangup voice-leave">
                                <PhoneOff aria-hidden="true" />
                                <span>Leave</span>
                              </span>
                            </div>
                          </>
                        ) : (
                          <div className="live-room-body">
                            <span className="live-stack">
                              {voicePeople.slice(0, 3).map((person) => (
                                <span className="live-avatar" key={person.name}>
                                  <Avatar person={person} />
                                </span>
                              ))}
                              {voicePeople.length > 3 && <span className="live-chip">+{voicePeople.length - 3}</span>}
                            </span>
                            <span className="live-names">{liveNames(voicePeople.map((person) => person.name))}</span>
                            {voiceTimer}
                            <span className="channel-join live-join">
                              <span className="channel-join-label">Join</span>
                            </span>
                          </div>
                        )}
                      </div>
                    </li>
                    <li>
                      <div className="channel-card">
                        <div className="channel-line">
                          <div className="channel-select">
                            <Hash aria-hidden="true" />
                            <span>feedback</span>
                            <i className="channel-unread" aria-hidden="true" />
                            <span className="sr-only">, unread activity</span>
                          </div>
                          <span className="channel-manage" aria-hidden="true">
                            <MoreHorizontal />
                          </span>
                        </div>
                      </div>
                    </li>
                  </ul>
                  <section className="direct-section sim-direct" aria-label="Simulated direct messages">
                    <div className="channel-section-heading">
                      <span className="direct-section-title">
                        <MessageCircle aria-hidden="true" />
                        Direct messages
                      </span>
                      <Plus aria-hidden="true" />
                    </div>
                    <ul>
                      <li>
                        <div className="channel-select direct-select direct-self">
                          <span className="direct-avatar">
                            <Avatar person={people[0]} />
                          </span>
                          <span>Maya</span>
                          <small>you</small>
                        </div>
                      </li>
                      {[1, 2].map((index) => (
                        <li key={people[index].name}>
                          <div className="channel-select direct-select" data-unread={index === 1 ? "" : undefined}>
                            <span className="direct-avatar">
                              <Avatar person={people[index]} />
                            </span>
                            <span>{people[index].name}</span>
                            {index === 1 && (
                              <span className="direct-unread-count" aria-label="2 unread messages">
                                2
                              </span>
                            )}
                          </div>
                        </li>
                      ))}
                    </ul>
                    <div className="channel-select direct-action">
                      <Plus aria-hidden="true" />
                      <span>Invite people</span>
                    </div>
                  </section>
                  <div className="browse-channels">
                    <Search aria-hidden="true" />
                    Browse channels
                  </div>
                </nav>
              </div>
              <div className="voice-panel sim-voice-panel">
                <div className="account-shortcuts">
                  <span className="notifications-entry" role="img" aria-label="Notifications, 3 unread">
                    <Bell aria-hidden="true" />
                    Notifications
                    <small aria-hidden="true">3</small>
                  </span>
                </div>
                <div
                  className="call-account sim-account"
                  role="img"
                  aria-label="Demo profile: Maya, online, with microphone, headphones and user settings"
                >
                  <span className="account-profile">
                    <span className="account-avatar">
                      <Avatar person={people[0]} />
                      <PresenceDot status="online" />
                    </span>
                    <strong className="account-name">Maya</strong>
                  </span>
                  <span className="voice-action-group">
                    <span className="voice-icon-button">
                      <Mic aria-hidden="true" />
                    </span>
                    <span className="device-menu">
                      <span className="call-settings-trigger">
                        <ChevronDown aria-hidden="true" />
                      </span>
                    </span>
                  </span>
                  <span className="voice-action-group">
                    <span className="voice-icon-button">
                      <Headphones aria-hidden="true" />
                    </span>
                    <span className="device-menu">
                      <span className="call-settings-trigger">
                        <ChevronDown aria-hidden="true" />
                      </span>
                    </span>
                  </span>
                  <span className="call-settings-trigger">
                    <Settings aria-hidden="true" />
                  </span>
                </div>
              </div>
            </aside>
            <section className="sim-chat chat-panel">
              <header className="chat-heading">
                <div className="sim-channel-heading">
                  <h2 className="chat-channel-title"># general</h2>
                  <ChevronDown aria-hidden="true" />
                </div>
                <span
                  className="member-list-toggle"
                  role="img"
                  aria-label={`Simulated members: ${onlinePeople.length} online; member list closed`}
                >
                  <Users aria-hidden="true" />
                </span>
              </header>
              <div className="sim-timeline">
                <div
                  className="sim-messages"
                  ref={messagesRef}
                  role="log"
                  aria-label="Simulated message history"
                  aria-live="off"
                  tabIndex={0}
                  onScroll={(event) => {
                    const viewport = event.currentTarget;
                    const gap = viewport.scrollHeight - viewport.scrollTop - viewport.clientHeight;
                    // A jump's own smooth scroll must not bring the button straight back.
                    if (jumping.current) {
                      if (gap >= 48) return;
                      jumping.current = false;
                    }
                    following.current = gap < 48;
                    if (gap <= viewport.clientHeight / 2) setAwayFrom(undefined);
                    else setAwayFrom((current) => current ?? latestKey.current);
                  }}
                >
                  <div className="sim-message-list">
                    {visibleMessages.map((message) => {
                      const item = message.emoji === undefined ? undefined : emoji[message.emoji];
                      // Keep edits, replies and reactions on the original message's
                      // clock so retained history never rewinds at a loop boundary.
                      const elapsed = phase + (cycle - message.cycle) * cycleLength;
                      const count = message.reactions.filter((at) => at <= elapsed).length;
                      const edited = message.edit && message.edit.at <= elapsed ? message.edit : undefined;
                      const replies = message.thread?.filter((reply) => reply.at <= elapsed) ?? [];
                      const forwardReplies = message.forward?.replies.filter((at) => at <= elapsed).length ?? 0;
                      return (
                        <article
                          className={`sim-message chat-message${message.pinnedBy !== undefined ? " chat-message-pinned" : ""}`}
                          key={`${message.cycle}:${message.at}`}
                        >
                          {message.pinnedBy !== undefined && (
                            <div className="chat-pin-marker">
                              <Pin size={12} aria-hidden="true" />
                              Pinned by {people[message.pinnedBy].name}
                            </div>
                          )}
                          <div className="chat-avatar">
                            <Avatar person={people[message.person]} />
                          </div>
                          <div>
                            <header>
                              <strong>{people[message.person].name}</strong>
                              <time>
                                {startedAt === undefined
                                  ? ""
                                  : timeFormatter.format(
                                      startedAt + (message.cycle * cycleLength + message.at) * 1_000,
                                    )}
                              </time>
                              {edited && <small className="chat-edited">edited</small>}
                            </header>
                            {message.voice ? (
                              <div className="chat-voice-message">
                                <div className="chat-voice">
                                  <span className="chat-voice-play">
                                    <Play size={16} fill="currentColor" aria-hidden="true" />
                                  </span>
                                  <div className="chat-voice-wave" aria-hidden="true">
                                    {voiceBars.map((height, index) => (
                                      <span key={index} style={{ height: `${height * 100}%` }} />
                                    ))}
                                  </div>
                                  <span className="chat-voice-time">{message.voice.duration}</span>
                                  <span className="sr-only">Voice message, 7 seconds</span>
                                </div>
                                {/* Transcribing first, then the transcript fades in, as in the app. */}
                                <div
                                  className="chat-voice-transcript"
                                  data-arrive={elapsed >= message.at + transcriptDelay || undefined}
                                >
                                  {elapsed >= message.at + transcriptDelay ? (
                                    <p>{message.voice.transcript}</p>
                                  ) : (
                                    <p className="chat-voice-transcribing">Transcribing…</p>
                                  )}
                                </div>
                              </div>
                            ) : (
                              <p>
                                <LinkedText text={edited?.text ?? message.text} />
                              </p>
                            )}
                            {message.meme && (
                              <img
                                className="sim-meme"
                                src="/images/demo-tiny-hat.webp"
                                width="384"
                                height="384"
                                draggable={false}
                                alt="A capybara wearing a tiny hat. Caption: Tiny hat. Huge energy."
                              />
                            )}
                            {message.forward && (
                              <div className="chat-forward-card">
                                <small className="chat-forward-label">
                                  <Forward size={12} aria-hidden="true" />
                                  Forwarded
                                </small>
                                <div className="chat-forward-original">
                                  <header>
                                    <div className="chat-avatar">
                                      <Avatar person={people[message.forward.person]} />
                                    </div>
                                    <strong>{people[message.forward.person].name}</strong>
                                  </header>
                                  <p>{message.forward.text}</p>
                                </div>
                                <span className="sim-forward-summary">
                                  {forwardReplies > 0 &&
                                    `${forwardReplies} ${forwardReplies === 1 ? "reply" : "replies"} · `}
                                  View conversation
                                </span>
                              </div>
                            )}
                            <div className="sim-reactions chat-reactions">
                              {item && count > 0 && (
                                <span
                                  className="sim-reaction chat-reaction"
                                  role="img"
                                  aria-label={`${item.text}, ${count} ${count === 1 ? "reaction" : "reactions"}`}
                                >
                                  <img src={`/images/demo-emoji/${item.code}.svg`} draggable={false} alt="" />
                                  <span>{count}</span>
                                </span>
                              )}
                            </div>
                            {replies.length > 0 && (
                              <div className="chat-thread-summary">
                                <span className="chat-thread-avatars">
                                  {replies.map((reply) => (
                                    <span key={reply.person}>
                                      <Avatar person={people[reply.person]} />
                                    </span>
                                  ))}
                                </span>
                                <strong>
                                  {replies.length} {replies.length === 1 ? "reply" : "replies"}
                                </strong>
                                <span>View thread</span>
                              </div>
                            )}
                          </div>
                        </article>
                      );
                    })}
                  </div>
                </div>
                {awayFrom !== undefined && (
                  <JumpToast
                    count={newWhileAway}
                    onJump={() => {
                      const viewport = messagesRef.current;
                      if (!viewport) return;
                      jumping.current = true;
                      following.current = true;
                      setAwayFrom(undefined);
                      viewport.scrollTo({ top: viewport.scrollHeight, behavior: reducedMotion ? "auto" : "smooth" });
                    }}
                  />
                )}
              </div>
              <div className="sim-typing chat-typing">
                <span className="chat-typing-content" data-visible={!!typingLabel}>
                  {displayedTypingLabel && (
                    <>
                      <img
                        className="chat-typing-echo"
                        src="/caper-echo.svg"
                        width={14}
                        height={14}
                        alt=""
                        aria-hidden="true"
                      />
                      <span className="chat-typing-dots" aria-hidden="true">
                        <i />
                        <i />
                        <i />
                      </span>
                      <span>{displayedTypingLabel}</span>
                    </>
                  )}
                </span>
              </div>
              <footer className="sim-footer chat-composer" aria-hidden="true">
                <form data-attach="" data-voice="">
                  <button type="button" className="chat-attach" tabIndex={-1}>
                    <Plus size={18} aria-hidden="true" />
                  </button>
                  <div className="chat-mention-input">
                    <div className="chat-rich-input" data-empty="" data-placeholder="Message #general" />
                  </div>
                  <div className="chat-composer-tools">
                    <button type="button" className="chat-composer-tool" tabIndex={-1}>
                      <Smile size={18} aria-hidden="true" />
                    </button>
                    <button type="button" className="chat-composer-tool chat-voice-mic" tabIndex={-1}>
                      <Mic size={16} aria-hidden="true" />
                    </button>
                  </div>
                </form>
              </footer>
            </section>
          </div>
        </div>
        <Link
          className="live-activator"
          data-live-activator
          data-live-control
          to={joinTo}
          preload={joinTo === "/spaces" ? "intent" : false}
          aria-label={joinTo === "/spaces" ? "Open Caper" : joinTo === "/profile" ? "Finish setup" : "Join Caper"}
          draggable={false}
        >
          <span className="live-invite">
            <i aria-hidden="true" />
            <span className="live-invite-fine">
              {joinTo === "/spaces" ? "Click to open" : joinTo === "/profile" ? "Finish setup" : "Click to join"}
            </span>
            <span className="live-invite-coarse">
              {joinTo === "/spaces" ? "Tap to open" : joinTo === "/profile" ? "Finish setup" : "Tap to join"}
            </span>
          </span>
        </Link>
      </div>
    </div>
  );
}
