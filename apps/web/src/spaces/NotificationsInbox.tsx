import { useCallback, useEffect, useRef, useState } from "react";
import { Bell, Check } from "lucide-react";
import Avatar from "../components/Avatar";
import Tooltip from "../components/Tooltip";
import { setApplicationBadge, UNREAD_CHANGED } from "../account/app-badge";
import { Dialog } from "./Dialog";
import { listInbox, readInbox, type InboxItem, type InboxFilter } from "./client";
import NotificationActions from "./NotificationActions";
import "./notifications-inbox.css";

const filters: [InboxFilter, string][] = [
  ["all", "All"],
  ["dms", "DMs"],
  ["mentions", "Mentions"],
  ["threads", "Threads"],
  ["reactions", "Reactions"],
  ["invitations", "Invitations"],
];

type InboxPage = Awaited<ReturnType<typeof listInbox>>;
/** The default view's first page (All, Unread only), loaded before the inbox opens. */
export type PreloadedInbox = InboxPage & { loadedAt: number };
const freshness = 15_000;

/** Neighbouring rows about the same conversation (or reactions to the same message)
 * show as one, newest first; invitations and requests always stand alone. Every
 * client groups by the same rule, and the inbox API stays per notification. */
function groupKey(item: InboxItem) {
  if (item.kind === "reaction") return `reaction:${item.messageId}:${item.unread}`;
  if (item.kind !== "message" && item.kind !== "reply" && item.kind !== "mention") return undefined;
  return [item.kind, item.conversationId ?? item.channelId, item.threadRootId ?? "", item.unread].join(":");
}

function groupItems(items: InboxItem[]) {
  const groups: InboxItem[][] = [];
  for (const item of items) {
    const previous = groups.at(-1);
    const key = groupKey(item);
    // One read request takes at most 100 IDs.
    if (previous && key && key === groupKey(previous[0]) && previous.length < 100) previous.push(item);
    else groups.push([item]);
  }
  return groups;
}

function actorNames(group: InboxItem[]) {
  const names = [...new Set(group.map((item) => item.actorName))];
  if (names.length === 1) return names[0];
  if (names.length === 2) return `${names[0]} and ${names[1]}`;
  return `${names[0]} and ${names.length - 1} others`;
}

function summary(group: InboxItem[]) {
  const [item] = group;
  if (group.length === 1) return description(item);
  switch (item.kind) {
    case "message":
      return `${group.length} messages`;
    case "reply":
      return `${group.length} replies`;
    case "mention":
      return `Mentioned you ${group.length} times`;
    default:
      return `Reacted ${[...new Set(group.map((row) => row.emoji ?? ""))].join(" ")} to your message`;
  }
}

function description(item: InboxItem) {
  switch (item.kind) {
    case "reaction":
      return `Reacted ${item.emoji ?? ""} to your message`;
    case "direct.request":
      return "Wants to send you messages";
    case "space.invitation":
      return "Invited you to a space";
    case "channel.invitation":
      return "Invited you to a private channel";
    default:
      return undefined;
  }
}

export default function NotificationsInbox({
  accountId,
  onClose,
  onOpen,
  onCount,
  preloaded,
}: {
  accountId: string;
  preloaded?: PreloadedInbox;
  onClose: () => void;
  onOpen: (item: InboxItem) => void | Promise<void>;
  onCount: (count: number) => void;
}) {
  const [filter, setFilter] = useState<InboxFilter>("all");
  const [unread, setUnread] = useState(true);
  const [items, setItems] = useState<InboxItem[]>([]);
  const [cursor, setCursor] = useState<string | null>();
  const [count, setCount] = useState<number>();
  // A page loaded ahead shows at once (refreshing once it is old) instead of a loading state.
  const [loading, setLoading] = useState(!preloaded);
  // After a filter change the previous rows stay in place, dimmed and inert, until
  // the new ones arrive: emptying the list first collapsed it and reset the scroll.
  const [stale, setStale] = useState(false);
  const [saving, setSaving] = useState<string>();
  const [error, setError] = useState<string>();
  const [expanded, setExpanded] = useState<InboxItem>();
  const [composerBusy, setComposerBusy] = useState(false);
  const composerState = useCallback(({ picking, sending }: { picking: boolean; sending: boolean }) => {
    setComposerBusy(picking || sending);
  }, []);
  const revision = useRef(0);
  const mounted = useRef(true);
  const loadedItems = useRef<InboxItem[]>([]);
  const fetching = useRef(false);
  // Opens and mark-reads in flight; automatic refreshes wait for them.
  const busy = useRef(0);
  const countRef = useRef<number>(undefined);
  const opening = useRef(false);
  // This component is keyed by account and discarded when the inbox closes.
  const cache = useRef(new Map<string, PreloadedInbox>(preloaded ? [["all:true", preloaded]] : []));
  const pending = useRef(new Map<string, Promise<InboxPage>>());
  const key = `${filter}:${unread}`;
  const firstPage = useCallback((type: InboxFilter, unreadOnly: boolean, remember = true) => {
    const key = `${type}:${unreadOnly}`;
    const existing = pending.current.get(key);
    if (existing) return existing;
    const request = listInbox(type, unreadOnly).then(
      (result) => {
        if (pending.current.get(key) === request) {
          pending.current.delete(key);
          // A background refresh commits only after the entire loaded window
          // succeeds; do not replace its older pages with just this first page.
          if (remember && !cache.current.has(key)) cache.current.set(key, { ...result, loadedAt: Date.now() });
        }
        return result;
      },
      (error: unknown) => {
        if (pending.current.get(key) === request) pending.current.delete(key);
        throw error;
      },
    );
    pending.current.set(key, request);
    return request;
  }, []);
  const prefetch = (type: InboxFilter) => {
    if (busy.current > 0) return;
    const cached = cache.current.get(`${type}:${unread}`);
    // Keep a previously loaded window (including pagination) until selected.
    if (cached) return;
    void firstPage(type, unread).catch(() => {});
  };
  const updateCount = useCallback(
    (value: number) => {
      countRef.current = value;
      setCount(value);
      onCount(value);
      setApplicationBadge(value);
    },
    [onCount],
  );

  const load = useCallback(
    async (after?: string, background = false) => {
      if (background && (fetching.current || busy.current > 0)) return;
      const request = ++revision.current;
      fetching.current = true;
      if (!background) setLoading(true);
      setError(undefined);
      try {
        let result = after ? await listInbox(filter, unread, after) : await firstPage(filter, unread, !background);
        const rows = [...result.items];
        const oldest = background ? loadedItems.current.at(-1) : undefined;
        // Revalidate the loaded window, not just its first page. New arrivals
        // must not push an unseen gap between the head and retained older rows.
        while (
          oldest &&
          result.nextCursor &&
          rows.length &&
          !rows.some((item) => item.id === oldest.id) &&
          new Date(rows.at(-1)!.createdAt).getTime() >= new Date(oldest.createdAt).getTime()
        ) {
          if (!mounted.current || request !== revision.current) return;
          result = await listInbox(filter, unread, result.nextCursor);
          rows.push(...result.items);
        }
        if (!mounted.current || request !== revision.current) return;
        const combined = after ? [...loadedItems.current, ...rows] : rows;
        const ids = new Set<string>();
        loadedItems.current = combined.filter((item) => {
          if (ids.has(item.id)) return false;
          ids.add(item.id);
          return true;
        });
        setItems(loadedItems.current);
        setCursor(result.nextCursor);
        cache.current.set(key, { ...result, items: loadedItems.current, loadedAt: Date.now() });
        updateCount(result.unreadCount);
      } catch (reason) {
        if (!mounted.current || request !== revision.current) return;
        // Rows kept from the previous filter must not stand in for this one.
        if (!after && !background) {
          loadedItems.current = [];
          setItems([]);
          setCursor(undefined);
        }
        setError(reason instanceof Error ? reason.message : "Notifications couldn’t load.");
      } finally {
        if (mounted.current && request === revision.current) {
          fetching.current = false;
          setLoading(false);
          if (!after) setStale(false);
        }
      }
    },
    [filter, unread, key, firstPage, updateCount],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      revision.current++;
    };
  }, []);
  useEffect(() => {
    fetching.current = false;
    const cached = cache.current.get(key);
    setError(undefined);
    if (cached) {
      loadedItems.current = cached.items;
      setItems(cached.items);
      setCursor(cached.nextCursor);
      setStale(false);
      setLoading(false);
      if (countRef.current === undefined) updateCount(cached.unreadCount);
      if (Date.now() - cached.loadedAt >= freshness) void load(undefined, true);
    } else {
      setCursor(undefined);
      setStale(loadedItems.current.length > 0);
      void load();
    }
    const refresh = () => {
      if (document.visibilityState === "visible") void load(undefined, true);
    };
    const invalidate = () => {
      cache.current.clear();
      pending.current.clear();
      refresh();
    };
    const timer = setInterval(refresh, freshness);
    window.addEventListener("focus", refresh);
    window.addEventListener(UNREAD_CHANGED, invalidate);
    return () => {
      revision.current++;
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
      window.removeEventListener(UNREAD_CHANGED, invalidate);
    };
  }, [key, load, updateCount]);

  // Marking read updates the row and count at once and puts them back if the save fails,
  // so the rest of the list never waits on (or dims for) the request.
  // A group is marked read in one request; opening it lands on its oldest notification,
  // where the run starts. Opening navigates alongside the read rather than after it, and
  // the row stays in place (read) so the inbox closes over it instead of emptying first.
  async function act(group: InboxItem[], open: boolean) {
    if (open) {
      if (opening.current) return;
      opening.current = true;
      setSaving(group[0].id);
    }
    busy.current++;
    cache.current.clear();
    pending.current.clear();
    setError(undefined);
    revision.current++;
    fetching.current = false;
    setLoading(false);
    const reading = group.filter((item) => item.unread);
    const ids = new Set(reading.map((item) => item.id));
    const indexes = new Map(loadedItems.current.map((row, index) => [row.id, index]));
    const update = (rows: InboxItem[]) => {
      loadedItems.current = rows;
      setItems(rows);
    };
    const read = async () => {
      if (!reading.length) return;
      update(
        unread && !open
          ? loadedItems.current.filter((row) => !ids.has(row.id))
          : loadedItems.current.map((row) => (ids.has(row.id) ? { ...row, unread: false } : row)),
      );
      if (countRef.current !== undefined) updateCount(Math.max(0, countRef.current - reading.length));
      try {
        const result = await readInbox([...ids]);
        // With several reads in flight, only the last reply's count includes them all.
        // The badge takes it even once a successful open has closed the inbox.
        if (busy.current === 1) updateCount(result.unreadCount);
      } catch (reason) {
        if (countRef.current !== undefined) updateCount(countRef.current + reading.length);
        if (!mounted.current) throw reason;
        const rows = loadedItems.current.map((row) => (ids.has(row.id) ? { ...row, unread: true } : row));
        // Put removed rows back where they were, in their original order.
        for (const item of [...reading].sort((a, b) => (indexes.get(a.id) ?? 0) - (indexes.get(b.id) ?? 0))) {
          const index = indexes.get(item.id);
          if (index !== undefined && !rows.some((row) => row.id === item.id))
            rows.splice(Math.min(index, rows.length), 0, item);
        }
        update(rows);
        throw reason;
      }
    };
    try {
      const [opened, saved] = await Promise.allSettled([(async () => open && onOpen(group.at(-1)!))(), read()]);
      // A failed open says more than a failed read beside it.
      if (opened.status === "rejected") throw opened.reason;
      if (saved.status === "rejected") throw saved.reason;
    } catch (reason) {
      if (mounted.current) setError(reason instanceof Error ? reason.message : "That notification couldn’t open.");
    } finally {
      busy.current--;
      cache.current.clear();
      pending.current.clear();
      if (open) {
        opening.current = false;
        if (mounted.current) setSaving(undefined);
      }
    }
  }

  // Reading elsewhere or a live refresh must not evict the reply being drafted.
  const visibleItems =
    expanded && !items.some((item) => item.id === expanded.id) ? [...items, { ...expanded, unread: false }] : items;

  return (
    <Dialog
      title="Notifications"
      titleIcon={<Bell aria-hidden="true" />}
      description="Messages, replies, reactions, and invitations—all in one place."
      onClose={onClose}
      closeDisabled={composerBusy}
      // A tap outside closes it, as with any sheet, unless a reply is open there.
      dismissOnBackdrop={!composerBusy && !expanded}
      width="wide"
    >
      <div className="notifications-inbox">
        <div className="inbox-filters" role="group" aria-label="Notification type">
          {filters.map(([value, label]) => (
            <button
              type="button"
              key={value}
              aria-pressed={filter === value}
              disabled={composerBusy}
              onMouseEnter={() => prefetch(value)}
              onFocus={() => prefetch(value)}
              onClick={() => {
                setExpanded(undefined);
                setFilter(value);
              }}
            >
              {label}
            </button>
          ))}
        </div>
        <div className="inbox-toolbar">
          <label>
            <input
              type="checkbox"
              role="switch"
              aria-label="Unread only"
              checked={unread}
              disabled={composerBusy}
              onChange={(event) => {
                setExpanded(undefined);
                setUnread(event.target.checked);
              }}
            />
            Unread only
            <span aria-live="polite">{count === undefined ? "" : `(${count})`}</span>
          </label>
        </div>
        <div className="inbox-results">
          {error && (
            <p className="inbox-error" role="alert">
              {error}{" "}
              <button
                type="button"
                disabled={loading || !!saving}
                onClick={() => void load(undefined, items.length > 0)}
              >
                Retry
              </button>
            </p>
          )}
          <ul
            className="inbox-list"
            aria-label="Notifications"
            aria-busy={loading}
            data-stale={stale || undefined}
            inert={stale}
          >
            {groupItems(visibleItems).map((group) => {
              const [item] = group;
              const names = actorNames(group);
              const detail = summary(group);
              // A reply being drafted stays on the row it started on as newer notifications join it.
              const target = group.find((row) => row.id === expanded?.id) ?? item;
              return (
                <li key={item.id} data-unread={item.unread || undefined}>
                  <button
                    type="button"
                    className="inbox-open"
                    disabled={saving === item.id || composerBusy}
                    onClick={() => void act(group, true)}
                  >
                    <span className="inbox-avatar">
                      <Avatar name={item.actorName} person={{ id: item.actorId, avatarId: item.avatarId }} />
                    </span>
                    <span className="inbox-content">
                      <span className="inbox-line">
                        <strong>{names}</strong>
                        {item.unread && <span className="inbox-dot" aria-label="Unread" />}
                      </span>
                      {detail && <span className="inbox-description">{detail}</span>}
                      {item.body && <span className="inbox-preview">{item.body}</span>}
                      <small>
                        {item.spaceName
                          ? `${item.channelName ? `#${item.channelName} · ` : ""}${item.spaceName}`
                          : "Direct message"}
                      </small>
                    </span>
                  </button>
                  <div className="inbox-meta">
                    <time dateTime={item.createdAt} title={new Date(item.createdAt).toLocaleString()}>
                      {new Date(item.createdAt).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
                    </time>
                    {item.unread && (
                      <Tooltip content="Mark read without opening">
                        <button
                          type="button"
                          className="inbox-read"
                          aria-label={
                            group.length > 1
                              ? `Mark ${group.length} notifications from ${names} read`
                              : `Mark notification from ${names} read`
                          }
                          onClick={() => void act(group, false)}
                        >
                          <Check aria-hidden="true" />
                        </button>
                      </Tooltip>
                    )}
                  </div>
                  {target.messageId &&
                    (target.channelId || target.conversationId) &&
                    ["message", "mention", "reply", "reaction"].includes(target.kind) && (
                      <NotificationActions
                        accountId={accountId}
                        item={target}
                        expanded={expanded?.id === target.id}
                        disabled={composerBusy}
                        onExpand={() => setExpanded(expanded?.id === target.id ? undefined : target)}
                        onStateChange={composerState}
                      />
                    )}
                </li>
              );
            })}
          </ul>
          {loading && !visibleItems.length && (
            <p className="inbox-status" role="status">
              Loading notifications…
            </p>
          )}
          {!loading && !error && !visibleItems.length && !cursor && (
            <div className="inbox-empty">
              <Bell aria-hidden="true" />
              <strong>{unread ? "You’re all caught up" : "No notifications here yet"}</strong>
              <p>
                {unread
                  ? "No unread notifications in this view."
                  : "Messages, mentions, replies, reactions, and invitations will appear here."}
              </p>
            </div>
          )}
          {cursor && (
            <button
              type="button"
              className="inbox-more"
              disabled={loading || !!saving}
              onClick={() => void load(cursor)}
            >
              Load more
            </button>
          )}
        </div>
      </div>
    </Dialog>
  );
}
