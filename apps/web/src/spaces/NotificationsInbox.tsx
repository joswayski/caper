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
}: {
  accountId: string;
  onClose: () => void;
  onOpen: (item: InboxItem) => void | Promise<void>;
  onCount: (count: number) => void;
}) {
  const [filter, setFilter] = useState<InboxFilter>("all");
  const [unread, setUnread] = useState(true);
  const [items, setItems] = useState<InboxItem[]>([]);
  const [cursor, setCursor] = useState<string | null>();
  const [count, setCount] = useState<number>();
  const [loading, setLoading] = useState(true);
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
  const busy = useRef(false);
  const updateCount = useCallback(
    (value: number) => {
      setCount(value);
      onCount(value);
      setApplicationBadge(value);
    },
    [onCount],
  );

  const load = useCallback(
    async (after?: string, background = false) => {
      if (background && (fetching.current || busy.current)) return;
      const request = ++revision.current;
      fetching.current = true;
      if (!background) setLoading(true);
      setError(undefined);
      try {
        let result = await listInbox(filter, unread, after);
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
        updateCount(result.unreadCount);
      } catch (reason) {
        if (mounted.current && request === revision.current)
          setError(reason instanceof Error ? reason.message : "Notifications couldn’t load.");
      } finally {
        if (mounted.current && request === revision.current) {
          fetching.current = false;
          setLoading(false);
        }
      }
    },
    [filter, unread, updateCount],
  );

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      revision.current++;
    };
  }, []);
  useEffect(() => {
    loadedItems.current = [];
    setItems([]);
    setCursor(undefined);
    void load();
    const refresh = () => {
      if (document.visibilityState === "visible") void load(undefined, true);
    };
    const timer = setInterval(refresh, 15_000);
    window.addEventListener("focus", refresh);
    window.addEventListener(UNREAD_CHANGED, refresh);
    return () => {
      revision.current++;
      clearInterval(timer);
      window.removeEventListener("focus", refresh);
      window.removeEventListener(UNREAD_CHANGED, refresh);
    };
  }, [load]);

  async function act(item: InboxItem, open: boolean) {
    if (busy.current) return;
    busy.current = true;
    setSaving(item.id);
    setError(undefined);
    revision.current++;
    fetching.current = false;
    setLoading(false);
    try {
      if (item.unread) {
        const result = await readInbox([item.id]);
        if (!mounted.current) return;
        updateCount(result.unreadCount);
        loadedItems.current = unread
          ? loadedItems.current.filter((row) => row.id !== item.id)
          : loadedItems.current.map((row) => (row.id === item.id ? { ...row, unread: false } : row));
        setItems(loadedItems.current);
      }
      if (open && mounted.current) await onOpen(item);
    } catch (reason) {
      if (mounted.current) setError(reason instanceof Error ? reason.message : "That notification couldn’t open.");
    } finally {
      busy.current = false;
      if (mounted.current) setSaving(undefined);
    }
  }

  // Reading elsewhere or a live refresh must not evict the reply being drafted.
  const visibleItems =
    expanded && !items.some((item) => item.id === expanded.id) ? [...items, { ...expanded, unread: false }] : items;

  return (
    <Dialog
      title="Notifications"
      titleIcon={<Bell aria-hidden="true" />}
      description="Messages, replies and reactions. All in one place."
      onClose={onClose}
      closeDisabled={composerBusy}
      width="wide"
    >
      <div className="notifications-inbox">
        <div className="inbox-filters" role="group" aria-label="Notification type">
          {filters.map(([value, label]) => (
            <button
              type="button"
              key={value}
              aria-pressed={filter === value}
              disabled={!!saving || composerBusy}
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
              disabled={!!saving || composerBusy}
              onChange={(event) => {
                setExpanded(undefined);
                setUnread(event.target.checked);
              }}
            />
            Unread only
            <span aria-live="polite">{count === undefined ? "" : `(${count})`}</span>
          </label>
        </div>
        {error && (
          <p className="inbox-error" role="alert">
            {error}{" "}
            <button type="button" disabled={loading || !!saving} onClick={() => void load(undefined, items.length > 0)}>
              Retry
            </button>
          </p>
        )}
        <ul className="inbox-list" aria-label="Notifications" aria-busy={loading}>
          {visibleItems.map((item) => (
            <li key={item.id} data-unread={item.unread || undefined}>
              <button
                type="button"
                className="inbox-open"
                disabled={!!saving || composerBusy}
                onClick={() => void act(item, true)}
              >
                <span className="inbox-avatar">
                  <Avatar name={item.actorName} person={{ id: item.actorId, avatarId: item.avatarId }} />
                </span>
                <span className="inbox-content">
                  <span className="inbox-line">
                    <strong>{item.actorName}</strong>
                    {item.unread && <span className="inbox-dot" aria-label="Unread" />}
                  </span>
                  {description(item) && <span className="inbox-description">{description(item)}</span>}
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
                  <Tooltip content={saving === item.id ? "Saving…" : "Mark read without opening"}>
                    <button
                      type="button"
                      className="inbox-read"
                      aria-label={`Mark notification from ${item.actorName} read`}
                      disabled={!!saving}
                      onClick={() => void act(item, false)}
                    >
                      <Check aria-hidden="true" />
                    </button>
                  </Tooltip>
                )}
              </div>
              {item.messageId &&
                (item.channelId || item.conversationId) &&
                ["message", "mention", "reply", "reaction"].includes(item.kind) && (
                  <NotificationActions
                    accountId={accountId}
                    item={item}
                    expanded={expanded?.id === item.id}
                    disabled={!!saving || composerBusy}
                    onExpand={() => setExpanded(expanded?.id === item.id ? undefined : item)}
                    onStateChange={composerState}
                  />
                )}
            </li>
          ))}
        </ul>
        {loading && (
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
                : "Messages, mentions, replies and reactions will appear here."}
            </p>
          </div>
        )}
        {cursor && (
          <button type="button" className="inbox-more" disabled={loading || !!saving} onClick={() => void load(cursor)}>
            Load more
          </button>
        )}
      </div>
    </Dialog>
  );
}
