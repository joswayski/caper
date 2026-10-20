import { useEffect, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import PresenceDot from "../components/PresenceDot";
import Avatar from "../components/Avatar";
import { watchPresence } from "../gateway/client";
import type { Member, Space } from "./client";
import type { ChatAuthor } from "../chat/types";
import MentionCard, { type MentionCardTarget } from "../chat/MentionCard";
import { mentionCardPerson } from "../chat/mentions";

type Status = "online" | "idle" | "offline";

/** A private channel's readers lead its member list; the rest of the space follows. */
export function memberGroups(members: Member[], channelMemberIds?: readonly string[]) {
  if (!channelMemberIds) return { inChannel: undefined, others: members };
  const ids = new Set(channelMemberIds);
  return {
    inChannel: members.filter((member) => ids.has(member.id)),
    others: members.filter((member) => !ids.has(member.id)),
  };
}

/** One space-scoped subscription; filtering never changes who has live presence. */
export default function MemberPresence({
  spaceId,
  members,
  channel,
  demo,
  accountId,
  onMessagePerson,
  onBlockAuthor,
  profileSpaces,
  onClose,
}: {
  spaceId: string;
  members: Member[];
  /** A private channel: its readers lead the list in their own group. */
  channel?: { name: string; memberIds: readonly string[] };
  demo?: boolean;
  accountId?: string;
  onMessagePerson: (username: string) => Promise<void>;
  onBlockAuthor: (author: ChatAuthor) => void;
  profileSpaces: (personId: string) => Space[];
  onClose: () => void;
}) {
  const [searching, setSearching] = useState(false);
  const [query, setQuery] = useState("");
  const searchButton = useRef<HTMLButtonElement>(null);
  const searchInput = useRef<HTMLInputElement>(null);
  const [statuses, setStatuses] = useState<Record<string, Status>>({});
  const [live, setLive] = useState(false);
  const [profile, setProfile] = useState<MentionCardTarget>();
  const filter = query.trim().toLocaleLowerCase();
  const visible = members.filter(
    (member) =>
      member.displayName.toLocaleLowerCase().includes(filter) || member.username.toLocaleLowerCase().includes(filter),
  );
  const { inChannel, others } = memberGroups(visible, channel?.memberIds);

  const closeSearch = () => {
    setQuery("");
    setSearching(false);
    requestAnimationFrame(() => searchButton.current?.focus());
  };

  useEffect(() => {
    if (searching) searchInput.current?.focus();
  }, [searching]);

  useEffect(() => {
    setSearching(false);
    setQuery("");
    setProfile(undefined);
  }, [spaceId]);

  useEffect(() => {
    setLive(false);
    setStatuses({});
    if (demo) return;
    let active = true;
    const stop = watchPresence(
      spaceId,
      "all",
      (values) => {
        if (active) setStatuses(Object.fromEntries(values.map((value) => [value.userId, value.status])));
      },
      (connected) => {
        if (!active) return;
        setLive(connected);
        if (!connected) setStatuses({});
      },
    );
    return () => {
      active = false;
      stop();
    };
  }, [spaceId, demo]);

  const row = (member: Member) => {
    const status = statuses[member.id];
    return (
      <li key={member.id} className={status === "offline" ? "member-offline" : undefined}>
        <button
          type="button"
          className="member-profile-trigger"
          aria-label={`Open profile for ${member.displayName}`}
          aria-haspopup="dialog"
          onClick={(event) =>
            setProfile({
              person: mentionCardPerson(member, members, accountId),
              anchor: event.currentTarget,
              drawer: matchMedia("(max-width: 760px), (pointer: coarse)").matches,
            })
          }
        >
          <span className="member-presence-avatar">
            <Avatar person={member} name={member.displayName} />
            <PresenceDot status={status} live={live} />
          </span>
          <strong title={member.displayName}>{member.displayName}</strong>
        </button>
      </li>
    );
  };

  const PresenceHeading = inChannel ? "h4" : "h3";
  const presenceGroups = (members: Member[]) =>
    (["Online", "Offline", "Status unavailable"] as const).map((label) => {
      const grouped = members.filter((member) => {
        const status = statuses[member.id];
        return label === "Online"
          ? status === "online" || status === "idle"
          : label === "Offline"
            ? status === "offline"
            : !status;
      });
      if (!grouped.length) return null;
      return (
        <section className="member-presence-group" aria-label={label} key={label}>
          <PresenceHeading className="member-group-label">
            <span>{label}</span>
            <span className="section-count">{grouped.length}</span>
          </PresenceHeading>
          <ul>{grouped.map(row)}</ul>
        </section>
      );
    });

  return (
    <aside id="space-member-list" className="space-member-presence" aria-label="Space members">
      <h2 className="member-presence-heading">
        <span>Members</span>
        {!demo && <span className="section-count">{members.length}</span>}
        <button className="member-list-close" type="button" aria-label="Close member list" onClick={onClose}>
          <X aria-hidden="true" />
        </button>
      </h2>
      <div className={`member-roster${searching ? " member-roster-searching" : ""}`}>
        {!demo &&
          (searching ? (
            <div
              className="member-search"
              onKeyDown={(event) => {
                if (event.key === "Escape" && !event.nativeEvent.isComposing) {
                  event.preventDefault();
                  event.stopPropagation();
                  closeSearch();
                }
              }}
            >
              <Search aria-hidden="true" />
              <input
                ref={searchInput}
                type="text"
                aria-label="Filter members"
                placeholder="Find a member…"
                value={query}
                onChange={(event) => setQuery(event.target.value)}
              />
              <button
                type="button"
                className="member-search-button"
                aria-label="Clear and close member search"
                onClick={closeSearch}
              >
                <X aria-hidden="true" />
              </button>
            </div>
          ) : (
            <button
              ref={searchButton}
              type="button"
              className="member-search-button member-search-open"
              aria-label="Search members"
              onClick={() => setSearching(true)}
            >
              <Search aria-hidden="true" />
            </button>
          ))}
        {demo ? (
          <p className="member-presence-connecting">
            General is open to everyone. People in voice appear in the channel sidebar.
          </p>
        ) : (
          !visible.length && (
            <p className="member-presence-connecting" role="status">
              {filter ? "No members found." : "No members to show."}
            </p>
          )
        )}
        {inChannel ? (
          <>
            {inChannel.length > 0 && (
              <section className="member-group member-group-channel" aria-label={`In #${channel!.name}`}>
                <h3 className="member-group-label">
                  <span>In #{channel!.name}</span>
                  <span className="section-count">{inChannel.length}</span>
                </h3>
                {presenceGroups(inChannel)}
              </section>
            )}
            {others.length > 0 && (
              <section className="member-group member-group-others" aria-label="Others in this space">
                <h3 className="member-group-label">
                  <span>Others in this space</span>
                  <span className="section-count">{others.length}</span>
                </h3>
                {presenceGroups(others)}
              </section>
            )}
          </>
        ) : (
          presenceGroups(visible)
        )}
      </div>
      {profile && (
        <MentionCard
          key={profile.person.id}
          target={profile}
          onClose={() => setProfile(undefined)}
          spaces={profileSpaces(profile.person.id)}
          onBlock={(author) => {
            setProfile(undefined);
            onBlockAuthor(author);
          }}
          onMessage={async (username) => {
            await onMessagePerson(username);
            onClose();
          }}
        />
      )}
    </aside>
  );
}
