import type { ChatMention, ChatMessage } from "./types.ts";

/** A member who can be suggested after `@`; mirrors `GET /api/spaces/{space}` members. */
export interface MentionCandidate {
  id: string;
  username: string;
  displayName: string;
  avatarId?: number | null;
}

export type MentionSuggestion =
  | { kind: "member"; member: MentionCandidate }
  | { kind: "everyone" | "here" }
  | { kind: "feedback"; alias: string };

export interface MentionToken {
  start: number;
  end: number;
  query: string;
}

const nameCharacter = /^[A-Za-z0-9_]$/;
const startBoundary = /[\s([{]/u;
const MAX_NAME = 32;

/** `@` at the start, after whitespace or after `(`, `[`, `{`, then up to 32 name characters. */
export function mentionToken(text: string, start: number, end = start): MentionToken | undefined {
  if (start !== end || start < 0 || start > text.length) return;
  if (text[start] === "@" || nameCharacter.test(text[start] ?? "")) return;
  let at = start - 1;
  while (at >= 0 && nameCharacter.test(text[at])) at--;
  if (text[at] !== "@" || (at > 0 && !startBoundary.test(text[at - 1]))) return;
  const query = text.slice(at + 1, start);
  if (query.length > MAX_NAME) return;
  return { start: at, end: start, query };
}

export const specialMentionLabels = {
  everyone: "Everyone in this channel",
  here: "Everyone online in this channel",
  feedback: "Notify the Caper team",
} as const;

/** Every name that asks the Caper team to look; all reserved usernames, never people. Mirrors the API. */
export const feedbackAliases = [
  "feedback",
  "caper",
  "capers",
  "caperchat",
  "caper_feedback",
  "caper_support",
  "support",
  "team",
  "staff",
  "admin",
  "admins",
  "administrator",
  "administrators",
  "mod",
  "mods",
  "moderator",
  "moderators",
] as const;

/**
 * People matching `query`, best first: exact username, username prefix,
 * display-name (or display-name word) prefix, then username substring.
 */
export function rankPeople<T extends MentionCandidate>(people: T[], query: string, limit = 6): T[] {
  const needle = query.toLowerCase();
  return people
    .flatMap((person) => {
      const username = person.username.toLowerCase();
      const displayName = person.displayName.toLowerCase();
      const rank =
        username === needle
          ? 0
          : username.startsWith(needle)
            ? 1
            : displayName.startsWith(needle) || displayName.split(" ").some((word) => word.startsWith(needle))
              ? 2
              : username.includes(needle)
                ? 3
                : 4;
      return rank < 4 ? [{ person, rank, username }] : [];
    })
    .sort((a, b) => a.rank - b.rank || (a.username < b.username ? -1 : a.username > b.username ? 1 : 0))
    .slice(0, limit)
    .map(({ person }) => person);
}

/** `people` without anyone listed in `exclude` (members, pending invitations, you). */
export function excludePeople<T extends { id: string }>(people: T[], ...exclude: { id: string }[][]): T[] {
  const ids = new Set(exclude.flat().map((person) => person.id));
  return people.filter((person) => !ids.has(person.id));
}

export function mentionSuggestions(members: MentionCandidate[], query: string, specials: boolean): MentionSuggestion[] {
  const needle = query.toLowerCase();
  const special: MentionSuggestion[] = specials
    ? (["everyone", "here"] as const).filter((name) => name.startsWith(needle)).map((kind) => ({ kind }))
    : [];
  // A bare `@` leads with `@feedback`, so people find the Caper team. Once
  // something is typed people come first, so `@a` + Enter still picks Alex;
  // the team row then completes the name being typed (`@ad` → `@admin`).
  const alias = needle ? feedbackAliases.find((name) => name.startsWith(needle)) : "feedback";
  const team: MentionSuggestion[] = alias ? [{ kind: "feedback", alias }] : [];
  const people = rankPeople(members, query, 6 - special.length - team.length).map((member) => ({
    kind: "member" as const,
    member,
  }));
  return needle ? [...people, ...special, ...team] : [...team, ...people, ...special];
}

export function mentionName(suggestion: MentionSuggestion) {
  return suggestion.kind === "member"
    ? suggestion.member.username
    : suggestion.kind === "feedback"
      ? suggestion.alias
      : suggestion.kind;
}

export function insertMention(text: string, token: MentionToken, name: string) {
  const inserted = `@${name} `;
  const value = text.slice(0, token.start) + inserted + text.slice(token.end);
  if (Array.from(value).length > 4_000) return;
  return { value, caret: token.start + inserted.length };
}

/** Well-formed entries only; a malformed list must never hide the message itself. */
function mentionList(mentions: unknown): ChatMention[] {
  return Array.isArray(mentions)
    ? mentions.filter(
        (mention): mention is ChatMention =>
          !!mention && typeof mention === "object" && typeof mention.type === "string",
      )
    : [];
}

/** A person a user-mention pill points at; `everyone`/`here` pills have none. */
export interface MentionedUser {
  id: string;
  username: string;
}

export interface MentionSegment {
  text: string;
  mention: boolean;
  user?: MentionedUser;
}

/** Splits message text into plain runs and mentions the server resolved. */
export function mentionSegments(text: string, mentions: ChatMention[] | undefined): MentionSegment[] {
  const resolved = new Map<string, MentionedUser | undefined>();
  for (const mention of mentionList(mentions)) {
    if (mention.type === "user" && typeof mention.username === "string" && typeof mention.id === "string") {
      resolved.set(mention.username.toLowerCase(), { id: mention.id, username: mention.username });
    } else if (mention.type === "everyone" || mention.type === "here") resolved.set(mention.type, undefined);
    else if (mention.type === "feedback") for (const alias of feedbackAliases) resolved.set(alias, undefined);
  }
  if (!resolved.size) return [{ text, mention: false }];
  const segments: MentionSegment[] = [];
  let plainStart = 0;
  let index = 0;
  while (index < text.length) {
    if (text[index] !== "@" || (index > 0 && !startBoundary.test(text[index - 1]))) {
      index++;
      continue;
    }
    let end = index + 1;
    while (end < text.length && nameCharacter.test(text[end])) end++;
    const name = text.slice(index + 1, end).toLowerCase();
    if (name.length <= MAX_NAME && resolved.has(name)) {
      if (index > plainStart) segments.push({ text: text.slice(plainStart, index), mention: false });
      const user = resolved.get(name);
      segments.push(
        user ? { text: text.slice(index, end), mention: true, user } : { text: text.slice(index, end), mention: true },
      );
      plainStart = end;
    }
    index = Math.max(end, index + 1);
  }
  if (plainStart < text.length) segments.push({ text: text.slice(plainStart), mention: false });
  return segments;
}

/** The author asked the Caper team to look at this message (the server recorded a report). */
export function reportsFeedback(message: ChatMessage) {
  return mentionList(message.content.mentions).some((mention) => mention.type === "feedback");
}

/** Highlights a message that names the reader, or `@everyone`/`@here` from someone else. */
export function mentionsAccount(message: ChatMessage, accountId: string | undefined) {
  if (!accountId) return false;
  return mentionList(message.content.mentions).some((mention) =>
    mention.type === "user"
      ? mention.id === accountId
      : (mention.type === "everyone" || mention.type === "here") && message.author.id !== accountId,
  );
}

/** What the profile card shows: the best local match by id, never a network call. */
export interface MentionCardPerson {
  id: string;
  /** Pin authors need not still be in the local directory. */
  username?: string;
  displayName?: string;
  avatarId?: number | null;
  self: boolean;
}

/** `directory` is ordered by preference: space members, then people, then DM peers. */
export function mentionCardPerson(
  user: { id: string; username?: string; name?: string; avatarId?: number | null },
  directory: MentionCandidate[],
  accountId: string | undefined,
): MentionCardPerson {
  const known = directory.find((candidate) => candidate.id === user.id);
  return {
    id: user.id,
    username: known?.username ?? user.username,
    displayName: known?.displayName ?? user.name,
    avatarId: known ? known.avatarId : user.avatarId,
    self: !!accountId && user.id === accountId,
  };
}
