import { useEffect, useState } from "react";
import PresenceDot from "../components/PresenceDot";
import { watchPresence } from "../gateway/client";
import type { Member } from "./client";

const PAGE_SIZE = 25;
type Status = "online" | "idle" | "offline";

/** Only the current member page has live presence subscriptions. */
export default function MemberPresence({ spaceId, members, demo }: { spaceId: string; members: Member[]; demo?: boolean }) {
  const [page, setPage] = useState(0);
  const [statuses, setStatuses] = useState<Record<string, Status>>({});
  const [live, setLive] = useState(false);
  const lastPage = Math.max(0, Math.ceil(members.length / PAGE_SIZE) - 1);
  const currentPage = Math.min(page, lastPage);
  const visible = members.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);
  const ids = visible.map((member) => member.id).join(",");

  useEffect(() => { setPage(0); }, [spaceId]);

  useEffect(() => {
    setLive(false);
    if (!ids) return;
    return watchPresence(spaceId, ids.split(","), (values) => {
      setStatuses((previous) => ({ ...previous, ...Object.fromEntries(values.map((value) => [value.userId, value.status])) }));
    }, setLive);
  }, [spaceId, ids]);

  return <aside id="space-member-list" className="space-member-presence" aria-label="Space members">
    <h2 className="member-presence-heading">
      <span>Members</span>{!demo && <span className="section-count">{members.length}</span>}
    </h2>
    <div>
      {demo ? <p className="member-presence-connecting">General is open to everyone. People in voice appear in the channel sidebar.</p> : !members.length && <p className="member-presence-connecting">No members to show.</p>}
      <ul>
        {visible.map((member) => {
          const status = statuses[member.id];
          return <li key={member.id}>
            <span className="member-presence-avatar">
              <span aria-hidden="true">{member.displayName.slice(0, 1).toUpperCase()}</span>
              <PresenceDot status={status} live={live} />
            </span>
            <span><strong title={member.displayName}>{member.displayName}</strong></span>
          </li>;
        })}
      </ul>
      {lastPage > 0 && <div className="member-presence-pages">
        <button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>Previous</button>
        <span>{currentPage + 1} / {lastPage + 1}</span>
        <button type="button" disabled={currentPage === lastPage} onClick={() => setPage(currentPage + 1)}>Next</button>
      </div>}
    </div>
  </aside>;
}
