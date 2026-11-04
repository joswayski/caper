import { MAX_ATTACHMENTS } from "./uploads.ts";

/** The drop overlay's text. Every client produces the strings in
 * shared/messages/drop-cases.json. Change them together. */
export function dropText(channel: string, direct: boolean, attached: number): { title: string; detail: string } {
  const room = Math.max(0, MAX_ATTACHMENTS - attached);
  return {
    title: `Attach to ${direct ? "" : "#"}${channel}`,
    detail:
      room === 0
        ? `${MAX_ATTACHMENTS} files attached. Remove one to add another.`
        : room === MAX_ATTACHMENTS
          ? `Up to ${MAX_ATTACHMENTS} files. Add a message before sending.`
          : `${room} more file${room === 1 ? "" : "s"}. Add a message before sending.`,
  };
}

/* The overlay's motion. Each open picks a random seed, so the cards drift and
 * the character hops differently every time; every client derives the same
 * motion from a seed (shared/messages/drop-motion-cases.json). Change them
 * together. Times are seconds since the drag arrived; lengths are points. */

/** mulberry32: small, and identical on every client's integer arithmetic. */
export function dropRandom(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** Resting pose per card, left to right: offset and turn in degrees. */
export const DROP_POSES = [
  [-46, 10, -16],
  [0, -26, 0],
  [46, 10, 16],
] as const;
const HOPS = 32;
const GRAVITY = 420;
const CROUCH = 0.12;

interface CardPlan {
  /** Offset from the resting pose, so no two opens rest the same way. */
  jitter: [number, number, number];
  delay: number;
  duration: number;
  /** Two [period, phase] waves each for x, y and turn. */
  waves: number[];
}
export interface DropPlan {
  cards: CardPlan[];
  tilt: [number, number];
  hops: { start: number; height: number }[];
}
export interface DropPose {
  /** Offset from the stage's bottom centre, turn in degrees, and opacity. */
  cards: { x: number; y: number; turn: number; opacity: number }[];
  /** How far below its resting place, squash, turn and opacity. */
  character: { y: number; scaleX: number; scaleY: number; turn: number; opacity: number };
}

export function dropPlan(seed: number): DropPlan {
  const random = dropRandom(seed);
  const spread = (size: number) => (random() * 2 - 1) * size;
  const cards = DROP_POSES.map(() => ({
    jitter: [spread(3), spread(3), spread(3)] as [number, number, number],
    delay: 0.04 + random() * 0.14,
    duration: 1.3 + random() * 0.6,
    waves: Array.from({ length: 6 }, () => [4 + random() * 5, random() * 2 * Math.PI]).flat(),
  }));
  const tilt: [number, number] = [5 + random() * 3, random() * 2 * Math.PI];
  const hops = [];
  let start = 0.75 + random() * 0.5;
  for (let index = 0; index < HOPS; index++) {
    const height = 5 + random() * 7;
    const quick = random() < 0.25;
    const pause = random();
    hops.push({ start, height });
    start += CROUCH + air(height) + air(height * 0.2) + (quick ? 0.06 + pause * 0.1 : 0.7 + pause * 1.8);
  }
  return { cards, tilt, hops };
}

/** Seconds in the air for a hop `height` high. */
function air(height: number): number {
  return 2 * Math.sqrt((2 * height) / GRAVITY);
}

/** A damped spring's response `elapsed` after a kick, peaking near `size`. */
function spring(size: number, elapsed: number): number {
  return elapsed <= 0 ? 0 : (size * Math.exp(-6 * elapsed) * Math.sin(16 * elapsed)) / 0.6;
}

function progress(elapsed: number, delay: number, duration: number): number {
  return Math.min(Math.max((elapsed - delay) / duration, 0), 1);
}

function easeOutCubic(t: number): number {
  return 1 - (1 - t) ** 3;
}

/** Where everything is `elapsed` seconds in; `still` is the reduced-motion resting pose. */
export function dropPose(plan: DropPlan, elapsed: number, still = false): DropPose {
  const t = still ? 0 : elapsed;
  const wave = (waves: number[], at: number) =>
    0.6 * Math.sin((2 * Math.PI * t) / waves[at] + waves[at + 1]) +
    0.4 * Math.sin((2 * Math.PI * t) / waves[at + 2] + waves[at + 3]);
  const cards = plan.cards.map((card, index) => {
    const [x, y, turn] = DROP_POSES[index];
    const arrived = still ? 1 : progress(t, card.delay, card.duration);
    const out = easeOutCubic(arrived);
    const drift = still ? 0 : out;
    return {
      x: (x + card.jitter[0]) * out + 2.5 * wave(card.waves, 0) * drift,
      // Each card slides out and up from just below the middle.
      y: 24 + (y + card.jitter[1] - 24) * out + 3.5 * wave(card.waves, 4) * drift,
      turn: turn + card.jitter[2] + 2.5 * wave(card.waves, 8) * drift,
      opacity: still ? 1 : progress(t, card.delay, 0.3),
    };
  });
  const rise = still ? 1 : easeOutCubic(progress(t, 0.08, 0.5));
  let lift = 0;
  let squash = 0;
  if (!still) {
    for (const { start, height } of plan.hops) {
      const since = t - start;
      if (since < 0) break;
      const up = air(height);
      const rebound = air(height * 0.2);
      const flying = since - CROUCH;
      if (flying >= 0 && flying < up) lift = (GRAVITY / 2) * flying * (up - flying);
      else if (flying >= up && flying < up + rebound) lift = (GRAVITY / 2) * (flying - up) * (up + rebound - flying);
      // Crouch, spring up, land, and settle after the rebound; higher hops squash more.
      const size = height / 9;
      squash +=
        spring(-0.05 * size, since) +
        spring(0.09 * size, flying) +
        spring(-0.1 * size, flying - up) +
        spring(-0.035 * size, flying - up - rebound);
    }
  }
  return {
    cards,
    character: {
      y: 28 * (1 - rise) - lift,
      scaleX: 1 - 0.8 * squash,
      scaleY: 1 + squash,
      turn: still ? 0 : 2 * Math.sin((2 * Math.PI * t) / plan.tilt[0] + plan.tilt[1]) * rise,
      opacity: still ? 1 : progress(t, 0.08, 0.2),
    },
  };
}

/** Only drags carrying files (not selected text or links) open the overlay. */
export function carriesFiles(transfer: DataTransfer | null): boolean {
  return !!transfer && Array.from(transfer.types).includes("Files");
}
