import { useEffect, useState } from "react";
import { watchPresence } from "../gateway/client";
import type { Member } from "./client";

const PAGE_SIZE = 25;
type Status = "online" | "idle" | "offline";

/** Only the current member page has live presence subscriptions. */
export default function MemberPresence({ spaceId, members }: { spaceId: string; members: Member[] }) {
  const [open, setOpen] = useState(true);
  const [page, setPage] = useState(0);
  const [statuses, setStatuses] = useState<Record<string, Status>>({});
  const [live, setLive] = useState(false);
  const lastPage = Math.max(0, Math.ceil(members.length / PAGE_SIZE) - 1);
  const currentPage = Math.min(page, lastPage);
  const visible = members.slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE);
  const ids = visible.map((member) => member.id).join(",");

  useEffect(() => {
    setStatuses({});
    setLive(false);
    if (!open || !ids) return;
    return watchPresence(spaceId, ids.split(","), (values) => {
      setStatuses(Object.fromEntries(values.map((value) => [value.userId, value.status])));
    }, setLive);
  }, [spaceId, ids, open]);

  return <section className="space-member-presence" aria-label="Space members">
    <button type="button" className="member-presence-heading" aria-expanded={open} onClick={() => setOpen(!open)}>
      <span>Members</span><span>{members.length}</span>
    </button>
    {open && <>
      {!live && <p className="member-presence-connecting" role="status">Updating statuses…</p>}
      <ul>
        {visible.map((member) => {
          const status = live ? statuses[member.id] : undefined;
          return <li key={member.id}>
            <span className="member-presence-avatar" aria-hidden="true">
              {member.displayName.slice(0, 1).toUpperCase()}
              <i data-status={status ?? "unknown"} />
            </span>
            <span><strong title={member.displayName}>{member.displayName}</strong><small>{status ?? "Updating"}</small></span>
          </li>;
        })}
      </ul>
      {lastPage > 0 && <div className="member-presence-pages">
        <button type="button" disabled={currentPage === 0} onClick={() => setPage(currentPage - 1)}>Previous</button>
        <span>{currentPage + 1} / {lastPage + 1}</span>
        <button type="button" disabled={currentPage === lastPage} onClick={() => setPage(currentPage + 1)}>Next</button>
      </div>}
    </>}
  </section>;
}
