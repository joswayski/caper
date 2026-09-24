/**
 * A scripted stand-in for the homepage window when the live #general channel is
 * unreachable (local web-only development, or an API outage). It only depicts
 * features Caper has today: text messages, typing, and voice presence.
 */

export interface PreviewPerson {
  id: string;
  name: string;
  /** Quadrant of /images/caper-avatars.webp. */
  avatar: 0 | 1 | 2 | 3;
}

export interface PreviewMessage {
  key: string;
  author: PreviewPerson;
  text: string;
  time: string;
}

export interface PreviewState {
  step: number;
  clock: number;
  serial: number;
  messages: PreviewMessage[];
  typing: PreviewPerson[];
  voice: PreviewPerson[];
}

export type PreviewBeat =
  | { wait: number; type: "typing"; who: PreviewPerson; on: boolean }
  | { wait: number; type: "message"; who: PreviewPerson; text: string }
  | { wait: number; type: "voice"; who: PreviewPerson; on: boolean };

export const cast = {
  maya: { id: "preview-maya", name: "Maya", avatar: 0 },
  jose: { id: "preview-jose", name: "Jose", avatar: 1 },
  alex: { id: "preview-alex", name: "Alex", avatar: 2 },
  sam: { id: "preview-sam", name: "Sam", avatar: 3 },
} as const satisfies Record<string, PreviewPerson>;

const { maya, jose, alex, sam } = cast;
const MAX_MESSAGES = 16;

export const previewBeats: readonly PreviewBeat[] = [
  { wait: 1200, type: "typing", who: maya, on: true },
  { wait: 1900, type: "message", who: maya, text: "Just got the new voice build running. It sounds so much cleaner 🎧" },
  { wait: 900, type: "typing", who: jose, on: true },
  { wait: 1700, type: "message", who: jose, text: "Hopping in to hear it" },
  { wait: 700, type: "voice", who: jose, on: true },
  { wait: 1600, type: "typing", who: sam, on: true },
  { wait: 1800, type: "message", who: sam, text: "Jose, you're crystal clear" },
  { wait: 1400, type: "typing", who: alex, on: true },
  { wait: 400, type: "typing", who: maya, on: true },
  { wait: 1500, type: "message", who: alex, text: "Same here. No echo at all" },
  { wait: 900, type: "message", who: maya, text: "Okay, joining too" },
  { wait: 600, type: "voice", who: maya, on: true },
  { wait: 4200, type: "typing", who: alex, on: true },
  { wait: 1600, type: "message", who: alex, text: "brb, grabbing a coffee ☕" },
  { wait: 800, type: "voice", who: alex, on: false },
  { wait: 3600, type: "voice", who: jose, on: false },
  { wait: 1200, type: "typing", who: jose, on: true },
  { wait: 1800, type: "message", who: jose, text: "Heading out. Same time tomorrow?" },
  { wait: 1500, type: "voice", who: alex, on: true },
  { wait: 1100, type: "message", who: alex, text: "back! who's still here?" },
  { wait: 2600, type: "voice", who: maya, on: false },
];

const START_CLOCK = 10 * 60 + 24;

function clockLabel(minutes: number) {
  const hours = Math.floor(minutes / 60) % 24;
  const minute = String(minutes % 60).padStart(2, "0");
  return `${hours % 12 || 12}:${minute} ${hours < 12 ? "AM" : "PM"}`;
}

export function initialPreview(): PreviewState {
  const opening: [PreviewPerson, string][] = [
    [jose, "Morning, everyone"],
    [sam, "Morning! Who's around today?"],
    [alex, "Here. Setting up in voice"],
    [sam, "Joining you"],
    [maya, "Two minutes, finishing a thing"],
  ];
  return {
    step: 0,
    clock: START_CLOCK,
    serial: opening.length,
    messages: opening.map(([author, text], index) => ({
      key: `preview-${index}`, author, text, time: clockLabel(START_CLOCK - opening.length + index),
    })),
    typing: [],
    voice: [alex, sam],
  };
}

/** Advance one beat. The script loops and returns voice to its opening roster. */
export function applyBeat(state: PreviewState, beat: PreviewBeat = previewBeats[state.step % previewBeats.length]): PreviewState {
  const step = (state.step + 1) % previewBeats.length;
  const without = (people: PreviewPerson[]) => people.filter((person) => person.id !== beat.who.id);
  if (beat.type === "typing") {
    return { ...state, step, typing: beat.on ? [...without(state.typing), beat.who] : without(state.typing) };
  }
  if (beat.type === "voice") {
    return { ...state, step, voice: beat.on ? [...without(state.voice), beat.who] : without(state.voice) };
  }
  const clock = state.clock + 1;
  const message = { key: `preview-${state.serial}`, author: beat.who, text: beat.text, time: clockLabel(clock) };
  return {
    ...state,
    step,
    clock,
    serial: state.serial + 1,
    typing: without(state.typing),
    messages: [...state.messages, message].slice(-MAX_MESSAGES),
  };
}

/**
 * Pick who is audibly talking next among the people in voice. Returns the
 * speaking set and how long it lasts. `random` is injectable for tests.
 */
export function nextSpeakers(voice: readonly PreviewPerson[], previous: ReadonlySet<string>, random = Math.random) {
  if (!voice.length) return { speaking: new Set<string>(), duration: 1_500 };
  const roll = random();
  if (roll < 0.22) return { speaking: new Set<string>(), duration: 500 + random() * 900 };
  const candidates = voice.filter((person) => !previous.has(person.id));
  const pool = candidates.length ? candidates : voice;
  const speaker = pool[Math.floor(random() * pool.length) % pool.length];
  const speaking = new Set([speaker.id]);
  // Occasional crosstalk, like a laugh over someone else.
  if (roll > 0.88 && voice.length > 1) {
    const other = voice.find((person) => person.id !== speaker.id);
    if (other) speaking.add(other.id);
  }
  return { speaking, duration: 900 + random() * 1_900 };
}
