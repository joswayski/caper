import { useEffect, useState } from "react";
import { X } from "lucide-react";
import PresenceDot from "../components/PresenceDot";
import Avatar from "../components/Avatar";
import { watchPresence } from "../gateway/client";
import type { Member, Space } from "./client";
import type { ChatAuthor } from "../chat/types";
import MentionCard, { type MentionCardTarget } from "../chat/MentionCard";
import { mentionCardPerson } from "../chat/mentions";

const PAGE_SIZE = 25;
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

/** Only the current member page has live presence subscriptions. */
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
  const [page, setPage] = useState(0);
  const [statuses, setStatuses] = useState<Record<string, Status>>({});
  const [live, setLive] = useState(false);
  const [profile, setProfile] = useState<MentionCardTarget>();
  const { inChannel, others } = memberGroups(members, channel?.memberIds);
  const ordered = inChannel ? [...inChannel, ...others] : members;
  const lastPage = Math.max(0, Math.ceil(ordered.length / PAGE_SIZE) - 1);
  const currentPage = Math.min(page, lastPage);
  const start = currentPage * PAGE_SIZE;
  const visible = ordered.slice(start, start + PAGE_SIZE);
  // Where this page splits between the channel's group and the rest of the space.
  const split = inChannel ? Math.max(0, Math.min(visible.length, inChannel.length - start)) : 0;
  const ids = visible.map((member) => member.id).join(",");

  useEffect(() => {
    setPage(0);
    setProfile(undefined);
  }, [spaceId]);

  useEffect(() => {
    setLive(false);
    if (!ids) return;
    return watchPresence(
      spaceId,
      ids.split(","),
      (values) => {
        setStatuses((previous) => ({
          ...previous,
          ...Object.fromEntries(values.map((value) => [value.userId, value.status])),
        }));
      },
      setLive,
    );
  }, [spaceId, ids]);

  const row = (member: Member) => {
    const status = statuses[member.id];
    return (
      <li key={member.id}>
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

  return (
    <aside id="space-member-list" className="space-member-presence" aria-label="Space members">
      <h2 className="member-presence-heading">
        <span>Members</span>
        {!demo && <span className="section-count">{members.length}</span>}
        <button className="member-list-close" type="button" aria-label="Close member list" onClick={onClose}>
          <X aria-hidden="true" />
        </button>
      </h2>
      <div>
        {demo ? (
          <p className="member-presence-connecting">
            General is open to everyone. People in voice appear in the channel sidebar.
          </p>
        ) : (
          !members.length && <p className="member-presence-connecting">No members to show.</p>
        )}
        {inChannel ? (
          <>
            {split > 0 && (
              <section className="member-group member-group-channel" aria-label={`In #${channel!.name}`}>
                <h3 className="member-group-label">
                  <span>In #{channel!.name}</span>
                  <span className="section-count">{inChannel.length}</span>
                </h3>
                <ul>{visible.slice(0, split).map(row)}</ul>
              </section>
            )}
            {split < visible.length && (
              <section className="member-group member-group-others" aria-label="Others in this space">
                <h3 className="member-group-label">
                  <span>Others in this space</span>
                  <span className="section-count">{others.length}</span>
                </h3>
                <ul>{visible.slice(split).map(row)}</ul>
              </section>
            )}
          </>
        ) : (
          <ul>{visible.map(row)}</ul>
        )}
        {lastPage > 0 && (
          <div className="member-presence-pages">
            <button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>
              Previous
            </button>
            <span>
              {currentPage + 1} / {lastPage + 1}
            </span>
            <button type="button" disabled={currentPage === lastPage} onClick={() => setPage(currentPage + 1)}>
              Next
            </button>
          </div>
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
