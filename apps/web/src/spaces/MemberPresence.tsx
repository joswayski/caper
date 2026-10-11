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

/** Only the current member page has live presence subscriptions. */
export default function MemberPresence({
  spaceId,
  members,
  demo,
  accountId,
  onMessagePerson,
  onBlockAuthor,
  profileSpaces,
  onClose,
}: {
  spaceId: string;
  members: Member[];
  demo?: boolean;
  accountId?: string;
  onMessagePerson: (username: string) => Promise<void>;
  onBlockAuthor: (author: ChatAuthor) => void;
  profileSpaces: Space[];
  onClose: () => void;
}) {
  const [page, setPage] = useState(0);
  const [statuses, setStatuses] = useState<Record<string, Status>>({});
  const [live, setLive] = useState(false);
  const [profile, setProfile] = useState<MentionCardTarget>();
  const lastPage = Math.max(0, Math.ceil(members.length / PAGE_SIZE) - 1);
  const currentPage = Math.min(page, lastPage);
  const visible = members.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);
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
        <ul>
          {visible.map((member) => {
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
                    <Avatar avatarId={member.avatarId} name={member.displayName} />
                    <PresenceDot status={status} live={live} />
                  </span>
                  <strong title={member.displayName}>{member.displayName}</strong>
                </button>
              </li>
            );
          })}
        </ul>
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
          spaces={profileSpaces}
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
