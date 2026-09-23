import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  ChevronDown,
  Hash,
  LockKeyhole,
  LogOut,
  MoreHorizontal,
  Plus,
  Settings,
  X,
} from "lucide-react";
import { getAccount, type Account } from "../account/client";
import { ChatHistoryError, loadChatHistory } from "../chat/client";
import Call from "../pages/Call";
import ChannelSidebar from "../pages/ChannelSidebar";
import { createSpaceNavigation, type PreparedSpace } from "./navigation";
import {
  addChannelMember,
  addSpaceMember,
  channelNameError,
  createChannel,
  createSpace,
  deleteChannel,
  deleteSpace,
  listChannelMembers,
  listSpaces,
  normalizeChannelName,
  removeChannelMember,
  removeSpaceMember,
  spaceNameError,
  updateChannel,
  updateSpace,
  SpacesApiError,
  type Channel,
  type Member,
  type Space,
  type SpaceDetail,
  type SpaceLimits,
} from "./client";
import "./spaces.css";

function errorMessage(error: unknown) {
  return error instanceof Error ? error.message : "That request did not work.";
}

function selectedFromUrl() {
  if (typeof window === "undefined") return {};
  const query = new URLSearchParams(window.location.search);
  return {
    spaceId: query.get("space") ?? undefined,
    channelId: query.get("channel") ?? undefined,
  };
}

function Dialog({
  title,
  description,
  onClose,
  children,
  width = "standard",
  dismissOnBackdrop = false,
}: {
  title: string;
  description?: string;
  onClose: () => void;
  children: ReactNode;
  width?: "standard" | "wide";
  dismissOnBackdrop?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const backdropPress = useRef(false);
  useLayoutEffect(() => {
    const dialog = ref.current!;
    const opener = document.activeElement as HTMLElement | null;
    dialog.showModal();
    dialog.querySelector<HTMLElement>("[data-initial-focus]")?.focus();
    return () => {
      dialog.close();
      if (opener?.isConnected) opener.focus();
    };
  }, []);
  return (
    <dialog
      ref={ref}
      className={`space-dialog ${width === "wide" ? "space-dialog-wide" : ""}`}
      aria-labelledby={titleId}
      onCancel={(event) => {
        event.preventDefault();
        event.stopPropagation();
        onClose();
      }}
      onPointerDown={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        backdropPress.current = event.target === event.currentTarget &&
          (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom);
      }}
      onClick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        if (dismissOnBackdrop && backdropPress.current && event.target === event.currentTarget &&
          (event.clientX < rect.left || event.clientX > rect.right || event.clientY < rect.top || event.clientY > rect.bottom)) onClose();
        backdropPress.current = false;
      }}
    >
      <header>
        <div>
          <h2 id={titleId}>{title}</h2>
          {description && <p>{description}</p>}
        </div>
        <button type="button" aria-label={`Close ${title}`} onClick={onClose}>
          <X aria-hidden="true" />
        </button>
      </header>
      {children}
    </dialog>
  );
}

function DeleteConfirmation({ kind, name, onClose, onDelete }: {
  kind: "space" | "channel";
  name: string;
  onClose: () => void;
  onDelete: () => Promise<void>;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const submitting = useRef(false);
  return (
    <Dialog title={`Delete ${kind}`} dismissOnBackdrop onClose={() => { if (!submitting.current) onClose(); }}>
      <div className="delete-confirmation">
        <p>Delete <strong>{kind === "channel" ? "#" : ""}{name}</strong>{kind === "space" ? " and all its channels" : ""} for everyone? This cannot be undone.</p>
        {error && <p className="space-form-error" role="alert">{error}</p>}
        <div className="space-dialog-actions">
          <button type="button" className="secondary" data-initial-focus disabled={pending} onClick={onClose}>Cancel</button>
          <button type="button" className="danger" disabled={pending} onClick={(event) => {
            // A second click from the opener's double-click must not confirm deletion.
            if (event.detail > 1 || submitting.current) return;
            submitting.current = true;
            setPending(true);
            setError(undefined);
            void onDelete().catch((reason) => {
              setError(errorMessage(reason));
              submitting.current = false;
              setPending(false);
            });
          }}>{pending ? "Deleting…" : `Delete ${kind}`}</button>
        </div>
      </div>
    </Dialog>
  );
}

function NameField({
  label,
  value,
  onChange,
  channel = false,
  privateChannel = false,
  showIcon = false,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  channel?: boolean;
  privateChannel?: boolean;
  showIcon?: boolean;
}) {
  return (
    <label className="space-field">
      <span>{label}</span>
      <span className={showIcon ? "channel-name-input" : undefined}>
        {showIcon && (privateChannel ? <LockKeyhole aria-hidden="true" /> : <Hash aria-hidden="true" />)}
        <input
          autoFocus
          value={value}
          maxLength={80}
          autoComplete="off"
          spellCheck={!channel}
          placeholder={channel ? "project-updates" : "Studio"}
          onChange={(event) => {
            const input = event.currentTarget;
            if (!channel) return onChange(input.value);
            const caret = normalizeChannelName(input.value.slice(0, input.selectionStart ?? input.value.length)).length;
            onChange(normalizeChannelName(input.value));
            requestAnimationFrame(() => input.setSelectionRange(caret, caret));
          }}
          onBlur={() => { if (channel) onChange(value.replace(/-$/, "")); }}
        />
      </span>
    </label>
  );
}

function ChannelPrivacy({ spaceName, checked, onChange }: {
  spaceName: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="channel-privacy">
      <label>
        <span><LockKeyhole aria-hidden="true" />Private channel</span>
        <input type="checkbox" role="switch" checked={checked} onChange={(event) => onChange(event.target.checked)} aria-describedby="channel-privacy-help" />
      </label>
      <p id="channel-privacy-help">{checked ? "Only you and the people you add can view or join." : <>Anyone in <strong>{spaceName}</strong> can view or join this channel.</>}</p>
    </div>
  );
}

function SubmitRow({
  pending,
  label,
  onCancel,
  destructive = false,
}: {
  pending: boolean;
  label: string;
  onCancel: () => void;
  destructive?: boolean;
}) {
  return (
    <div className="space-dialog-actions">
      <button type="button" className="secondary" onClick={onCancel}>
        Cancel
      </button>
      <button
        type="submit"
        className={destructive ? "danger" : "primary"}
        disabled={pending}
      >
        {pending ? "Saving…" : label}
      </button>
    </div>
  );
}

function CreateSpaceDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (space: Space) => void;
}) {
  const [name, setName] = useState("");
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const invalid = spaceNameError(name);
    if (invalid) return setError(invalid);
    setPending(true);
    setError(undefined);
    try {
      onCreated(await createSpace(name));
    } catch (reason) {
      setError(errorMessage(reason));
      setPending(false);
    }
  };
  return (
    <Dialog
      title="Create a space"
      onClose={onClose}
    >
      <form onSubmit={(event) => void submit(event)}>
        <NameField label="Space name" value={name} onChange={setName} />
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <SubmitRow pending={pending} label="Create space" onCancel={onClose} />
      </form>
    </Dialog>
  );
}

function CreateChannelDialog({
  space,
  onClose,
  onCreated,
}: {
  space: SpaceDetail;
  onClose: () => void;
  onCreated: (channel: Channel) => void;
}) {
  const [name, setName] = useState("");
  const [privateChannel, setPrivateChannel] = useState(false);
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const channelName = name.replace(/-$/, "");
    const invalid = channelNameError(channelName);
    if (invalid) return setError(invalid);
    setPending(true);
    setError(undefined);
    try {
      onCreated(await createChannel(space.space.id, channelName, privateChannel));
    } catch (reason) {
      setError(errorMessage(reason));
      setPending(false);
    }
  };
  return (
    <Dialog
      title="Create a channel"
      onClose={onClose}
    >
      <form onSubmit={(event) => void submit(event)}>
        <NameField
          channel
          showIcon
          privateChannel={privateChannel}
          label="Channel name"
          value={name}
          onChange={setName}
        />
        <p className="channel-name-guidance">Channels are where conversations happen around a topic. Use a name that is easy to find and understand.</p>
        <ChannelPrivacy spaceName={space.space.name} checked={privateChannel} onChange={setPrivateChannel} />
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <SubmitRow
          pending={pending}
          label="Create channel"
          onCancel={onClose}
        />
      </form>
    </Dialog>
  );
}

function LeaveSpaceDialog({
  space,
  account,
  onClose,
  onLeft,
}: {
  space: Space;
  account: Account;
  onClose: () => void;
  onLeft: () => void;
}) {
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  return (
    <Dialog
      title={`Leave ${space.name}?`}
      description="You will lose access to its channels and conversations. An owner can add you again later."
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          setPending(true);
          setError(undefined);
          void removeSpaceMember(space.id, account.id)
            .then(onLeft)
            .catch((reason) => {
              setError(errorMessage(reason));
              setPending(false);
            });
        }}
      >
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <SubmitRow
          pending={pending}
          label="Leave space"
          destructive
          onCancel={onClose}
        />
      </form>
    </Dialog>
  );
}

function MemberManager({
  members,
  onAdd,
  onRemove,
  pending,
}: {
  members: Member[];
  onAdd: (username: string) => Promise<void>;
  onRemove: (member: Member) => Promise<void>;
  pending: boolean;
}) {
  const [username, setUsername] = useState("");
  const [error, setError] = useState<string>();
  return (
    <section className="member-manager">
      <div className="dialog-section-heading">
        <h3>Members</h3>
        <span>{members.length}</span>
      </div>
      <form
        className="member-add"
        onSubmit={(event) => {
          event.preventDefault();
          if (!username) return setError("Enter an exact username.");
          setError(undefined);
          void onAdd(username)
            .then(() => setUsername(""))
            .catch((reason) => setError(errorMessage(reason)));
        }}
      >
        <label className="sr-only" htmlFor="member-username">
          Exact username
        </label>
        <input
          id="member-username"
          value={username}
          autoComplete="off"
          placeholder="Exact username"
          onChange={(event) => setUsername(event.target.value)}
        />
        <button type="submit" disabled={pending}>
          Add
        </button>
      </form>
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      <ul>
        {members.map((member) => (
          <li key={member.id}>
            {/* TODO show status indicator */}
            <span className="member-avatar" aria-hidden="true">
              {member.displayName.slice(0, 1).toUpperCase()}
            </span>
            <span>
              <strong>{member.displayName}</strong>
              <small>
                @{member.username}
                {member.owner ? " · Owner" : ""}
              </small>
            </span>
            {!member.owner && (
              <button
                type="button"
                disabled={pending}
                onClick={() =>
                  void onRemove(member).catch((reason) =>
                    setError(errorMessage(reason)),
                  )
                }
              >
                Remove
              </button>
            )}
          </li>
        ))}
      </ul>
    </section>
  );
}

function ManageSpaceDialog({
  detail,
  onClose,
  onChanged,
  onDeleted,
}: {
  detail: SpaceDetail;
  onClose: () => void;
  onChanged: (detail: SpaceDetail) => void;
  onDeleted: () => void;
}) {
  const [name, setName] = useState(detail.space.name);
  const [members, setMembers] = useState(detail.members);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const run = async (action: () => Promise<void>) => {
    setPending(true);
    setError(undefined);
    try {
      await action();
    } finally {
      setPending(false);
    }
  };
  return (
    <Dialog
      title="Manage space"
      description="Only the owner can change this space and its membership."
      onClose={onClose}
      width="wide"
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          const invalid = spaceNameError(name);
          if (invalid) return setError(invalid);
          void run(async () => {
            const space = await updateSpace(detail.space.id, name);
            onChanged({ ...detail, space, members });
          }).catch((reason) => setError(errorMessage(reason)));
        }}
      >
        <NameField label="Space name" value={name} onChange={setName} />
        <div className="inline-save">
          <button
            className="secondary"
            disabled={pending || name.trim() === detail.space.name}
            type="submit"
          >
            Save name
          </button>
        </div>
      </form>
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      <MemberManager
        members={members}
        pending={pending}
        onAdd={(username) =>
          run(async () => {
            const member = await addSpaceMember(detail.space.id, username);
            const next = [
              ...members.filter((item) => item.id !== member.id),
              member,
            ];
            setMembers(next);
            onChanged({ ...detail, members: next });
          })
        }
        onRemove={async (member) =>
          run(async () => {
            await removeSpaceMember(detail.space.id, member.id);
            const next = members.filter((item) => item.id !== member.id);
            setMembers(next);
            onChanged({ ...detail, members: next });
          })
        }
      />
      <section className="danger-zone">
        <h3>Delete space</h3>
        <p>Delete this space and all its channels for every member.</p>
        <button className="danger-outline" type="button" disabled={pending} onClick={() => setConfirmDelete(true)}>Delete space</button>
      </section>
      {confirmDelete && <DeleteConfirmation kind="space" name={detail.space.name} onClose={() => setConfirmDelete(false)} onDelete={async () => {
        await deleteSpace(detail.space.id);
        onDeleted();
      }} />}
    </Dialog>
  );
}

function ManageChannelDialog({
  detail,
  channel,
  onClose,
  onChanged,
  onDeleted,
}: {
  detail: SpaceDetail;
  channel: Channel;
  onClose: () => void;
  onChanged: (channel: Channel) => void;
  onDeleted: () => void;
}) {
  const [name, setName] = useState(channel.name);
  const [privateChannel, setPrivateChannel] = useState(channel.private);
  const [members, setMembers] = useState<Member[]>([]);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const dirty = name !== channel.name || privateChannel !== channel.private;
  const run = async (action: () => Promise<void>) => {
    setPending(true);
    setError(undefined);
    try {
      await action();
    } finally {
      setPending(false);
    }
  };
  useEffect(() => {
    if (channel.private)
      void listChannelMembers(detail.space.id, channel.id)
        .then((value) => setMembers(value.members))
        .catch((reason) => setError(errorMessage(reason)));
  }, [channel.id, channel.private, detail.space.id]);
  return (
    <Dialog
      title="Overview"
      onClose={onClose}
      width="wide"
    >
      <form
        id="channel-overview"
        onSubmit={(event) => {
          event.preventDefault();
          if (pending || !dirty) return;
          const channelName = name.replace(/-$/, "");
          const invalid = channelNameError(channelName);
          if (invalid) return setError(invalid);
          void run(async () => {
            const updated = await updateChannel(
              detail.space.id,
              channel.id,
              channelName,
              privateChannel,
            );
            setName(updated.name);
            onChanged(updated);
          }).catch((reason) => setError(errorMessage(reason)));
        }}
      >
        <fieldset disabled={pending}>
          <NameField
            channel
            label="Channel name"
            value={name}
            onChange={setName}
          />
          <ChannelPrivacy spaceName={detail.space.name} checked={privateChannel} onChange={setPrivateChannel} />
        </fieldset>
      </form>
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      {channel.private && (
        <MemberManager
          members={members}
          pending={pending}
          onAdd={async (username) =>
            run(async () => {
              const member = await addChannelMember(
                detail.space.id,
                channel.id,
                username,
              );
              setMembers((current) => [
                ...current.filter((item) => item.id !== member.id),
                member,
              ]);
            })
          }
          onRemove={async (member) =>
            run(async () => {
              await removeChannelMember(detail.space.id, channel.id, member.id);
              setMembers((current) =>
                current.filter((item) => item.id !== member.id),
              );
            })
          }
        />
      )}
      <section className="danger-zone">
        <h3>Delete channel</h3>
        <p>Delete this channel for everyone in the space.</p>
        <button className="danger-outline" type="button" disabled={pending} onClick={() => setConfirmDelete(true)}>Delete channel</button>
      </section>
      {dirty && (
        <footer className="channel-save-bar">
          <span role="status">You have unsaved changes.</span>
          <div>
            <button type="button" className="secondary" disabled={pending} onClick={() => {
              setName(channel.name);
              setPrivateChannel(channel.private);
              setError(undefined);
            }}>Reset</button>
            <button type="submit" form="channel-overview" className="primary" disabled={pending}>{pending ? "Saving…" : "Save changes"}</button>
          </div>
        </footer>
      )}
      {confirmDelete && <DeleteConfirmation kind="channel" name={channel.name} onClose={() => setConfirmDelete(false)} onDelete={async () => {
        await deleteChannel(detail.space.id, channel.id);
        onDeleted();
      }} />}
    </Dialog>
  );
}

function SpacesLoading() {
  return <main className="call-page" aria-busy="true">
    <header className="call-header">
      <a className="wordmark" href="/">caper<span className="wordmark-dot">.</span></a>
    </header>
    <section className="call-room spaces-room spaces-loading">
      <div className="space-rail" aria-hidden="true" />
      <ChannelSidebar><div className="sidebar-channels" /></ChannelSidebar>
      <div className="stage"><p className="sr-only" role="status">Loading your spaces…</p></div>
    </section>
  </main>;
}

export default function Spaces() {
  const [account, setAccount] = useState<Account>();
  const [spaces, setSpaces] = useState<Space[]>([]);
  const [limits, setLimits] = useState<SpaceLimits>();
  const [view, setView] = useState<PreparedSpace>();
  const detail = view?.detail;
  const navigation = useRef(createSpaceNavigation());
  const [selected, setSelected] = useState(selectedFromUrl);
  const [loading, setLoading] = useState(true);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const [dialog, setDialog] = useState<
    "space" | "channel" | "manage-space" | "leave-space"
  >();
  const [manageChannel, setManageChannel] = useState<Channel>();
  const [navigationOpen, setNavigationOpen] = useState(false);
  const spaceMenu = useRef<HTMLDetailsElement>(null);
  const channelMenu = useRef<HTMLDetailsElement>(null);
  const [channelsExpanded, setChannelsExpanded] = useState(true);

  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      for (const menu of [spaceMenu.current, channelMenu.current]) {
        if (menu && !menu.contains(event.target as Node)) menu.open = false;
      }
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, []);

  const choose = (spaceId?: string, channelId?: string, replace = false) => {
    if (spaceMenu.current) spaceMenu.current.open = false;
    if (channelMenu.current) channelMenu.current.open = false;
    const query = new URLSearchParams();
    if (spaceId) query.set("space", spaceId);
    if (channelId) query.set("channel", channelId);
    window.history[replace ? "replaceState" : "pushState"](
      {},
      "",
      `/spaces${query.size ? `?${query}` : ""}`,
    );
    setSelected({ spaceId, channelId });
  };

  const prefetch = (spaceId: string, channelId?: string) => {
    if (spaceId === detail?.space.id && (!channelId || channelId === view?.channelId)) return;
    void navigation.current.prepare(spaceId, channelId).catch(() => undefined);
  };

  useEffect(() => {
    const pop = () => setSelected(selectedFromUrl());
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);

  useEffect(() => {
    let current = true;
    void getAccount()
      .then(async (nextAccount) => {
        if (!current) return;
        if (nextAccount && (!nextAccount.username || !nextAccount.displayName))
          return void window.location.assign("/profile");
        const [result, demoHistory] = await Promise.all([
          nextAccount ? listSpaces() : Promise.resolve({ spaces: [], limits: undefined }),
          loadChatHistory().catch((reason) => { if (!nextAccount) throw reason; return undefined; }),
        ]);
        if (!current) return;
        const demo = demoHistory ? navigation.current.setDemo(demoHistory) : undefined;
        const available = [...(demo ? [demo.detail.space] : []), ...result.spaces];
        setAccount(nextAccount ?? undefined);
        setSpaces(available);
        setLimits(result.limits);
        setLoading(false);
        if (!available.some((space) => space.id === selected.spaceId))
          choose(available[0]?.id, undefined, true);
      })
      .catch((reason) => {
        if (current) {
          setError(errorMessage(reason));
          setLoading(false);
        }
      });
    return () => {
      current = false;
    };
  }, []);

  useEffect(() => {
    if (loading || !selected.spaceId) {
      return;
    }
    let current = true;
    setError(undefined);
    setPending(true);
    const cached = navigation.current.peek(selected.spaceId, selected.channelId);
    if (cached) {
      setView(cached);
      setNavigationOpen(false);
    }
    void navigation.current.take(selected.spaceId, selected.channelId)
      .then((next) => {
        if (!current) return;
        // Commit the channel list and first history together. Until this point,
        // the existing Call (including its draft and live connections) stays put.
        navigation.current.remember(next);
        setView(next);
        setPending(false);
        setNavigationOpen(false);
        const query = new URLSearchParams({ space: next.detail.space.id });
        if (next.channelId) query.set("channel", next.channelId);
        window.history.replaceState({}, "", `/spaces?${query}`);
      })
      .catch((reason) => {
        if (current) {
          if ((reason instanceof SpacesApiError || reason instanceof ChatHistoryError) && [401, 403, 404].includes(reason.status)) {
            navigation.current.forget(selected.spaceId!);
            setView((shown) => shown?.detail.space.id === selected.spaceId ? undefined : shown);
          }
          setError(errorMessage(reason));
          setPending(false);
        }
      });
    return () => {
      current = false;
    };
  }, [selected, loading]);

  const channel = detail?.channels.find(
    (item) => item.id === view?.channelId,
  ) ?? detail?.channels[0];
  const owner = !!account && detail?.space.ownerId === account.id;
  const ownedCount = account
    ? spaces.filter((space) => space.ownerId === account.id).length
    : 0;
  const canCreateSpace =
    !!limits &&
    ownedCount < limits.ownedSpaces &&
    spaces.filter((space) => !space.demo).length < limits.totalSpaces;
  const canCreateChannel =
    !!limits && !!detail && detail.channels.length < limits.channelsPerSpace;
  const replaceDetail = (next: SpaceDetail) => {
    navigation.current.forget(next.space.id);
    if (view) {
      const retained = next.channels.some((item) => item.id === view.channelId);
      const updated = retained ? { ...view, detail: next } : { detail: next };
      navigation.current.remember(updated);
      setView(updated);
    }
    setSpaces((current) =>
      current.map((space) => (space.id === next.space.id ? next.space : space)),
    );
  };
  const forgetSpace = () => {
    const remaining = spaces.filter((space) => space.id !== detail?.space.id);
    if (detail) navigation.current.forget(detail.space.id);
    // Explicit removal must tear down the old Call immediately, even if the
    // next space is slow. It is no longer a conversation we can keep showing.
    setView(undefined);
    setSpaces(remaining);
    setDialog(undefined);
    choose(remaining[0]?.id, undefined, true);
  };

  if (loading) return <SpacesLoading />;
  if (error && !spaces.length)
    return (
      <main className="spaces-state">
        <a className="wordmark" href="/">
          caper<span className="wordmark-dot">.</span>
        </a>
        <h1>Spaces are unavailable.</h1>
        <p role="alert">{error}</p>
        <button type="button" onClick={() => window.location.reload()}>
          Try again
        </button>
      </main>
    );
  if (!spaces.length)
    return (
      <main className="spaces-empty">
        <a className="wordmark" href="/">
          caper<span className="wordmark-dot">.</span>
        </a>
        <section>
          <p className="eyebrow">YOUR SPACES</p>
          <h1>Start a conversation.</h1>
          <p>
            Create a space for your people. Every space begins with one unified
            text and voice channel.
          </p>
          <button
            type="button"
            disabled={!canCreateSpace}
            onClick={() => setDialog("space")}
          >
            Create your first space
          </button>
          {!canCreateSpace && <small>You have reached your space limit.</small>}
        </section>
        {dialog === "space" && (
          <CreateSpaceDialog
            onClose={() => setDialog(undefined)}
            onCreated={(space) => {
              setSpaces([space]);
              setDialog(undefined);
              choose(space.id);
            }}
          />
        )}
      </main>
    );
  if (!detail && !error) return <SpacesLoading />;
  if (!detail)
    return (
      <main className="spaces-state">
        <a className="wordmark" href="/">
          caper<span className="wordmark-dot">.</span>
        </a>
        <p role="alert">{error}</p>
        <button type="button" onClick={() => setSelected({ ...selected })}>Try again</button>
      </main>
    );

  const rail = (
    <nav className="space-rail" aria-label="Spaces">
      {spaces.map((space) => (
        <div
          className="space-rail-item"
          key={space.id}
          data-active={space.id === detail.space.id}
        >
          <button
            type="button"
            title={space.name}
            aria-label={space.name}
            aria-current={space.id === detail.space.id ? "page" : undefined}
            aria-busy={pending && space.id === selected.spaceId}
            onMouseEnter={() => prefetch(space.id)}
            onFocus={() => prefetch(space.id)}
            onClick={() => choose(space.id)}
          >
            <span>{space.name.slice(0, 1).toUpperCase()}</span>
          </button>
        </div>
      ))}
      <button
        className="add-space"
        type="button"
        title={
          !account ? "Sign in to create a space" : canCreateSpace
            ? "Create space"
            : `Space limit reached (${limits?.ownedSpaces ?? 20} owned, ${limits?.totalSpaces ?? 100} total)`
        }
        aria-label="Create space"
        disabled={!!account && !canCreateSpace}
        onClick={() => account ? setDialog("space") : window.location.assign("/login")}
      >
        <Plus aria-hidden="true" />
      </button>
      {pending && <span className="sr-only" role="status">Opening {spaces.find((space) => space.id === selected.spaceId)?.name}…</span>}
    </nav>
  );
  const channelNavigation = (
    <nav
      className="channel-navigation"
      aria-label={`${detail.space.name} channels`}
    >
      <header>
        {detail.space.demo ? <h1 className="demo-space-title">{detail.space.name}</h1> : <details
          ref={spaceMenu}
          className="space-menu"
          onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.currentTarget.open = false;
              event.currentTarget.querySelector("summary")?.focus();
            }
          }}
          onBlur={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget))
              event.currentTarget.open = false;
          }}
        >
          <summary aria-label={`${detail.space.name} actions`}>
            <h1 title={detail.space.name}>{detail.space.name}</h1>
            <ChevronDown aria-hidden="true" />
          </summary>
          <div className="space-actions">
            {owner ? (
              <>
                <button
                  type="button"
                  onClick={() => {
                    spaceMenu.current!.open = false;
                    setDialog("manage-space");
                  }}
                >
                  <Settings aria-hidden="true" />
                  Space settings
                </button>
              </>
            ) : (
              <button
                type="button"
                className="leave-space"
                onClick={() => {
                  spaceMenu.current!.open = false;
                  setDialog("leave-space");
                }}
              >
                <LogOut aria-hidden="true" />
                Leave space…
              </button>
            )}
          </div>
        </details>}
        {navigationOpen && (
          <button
            type="button"
            aria-label="Close navigation"
            onClick={() => setNavigationOpen(false)}
          >
            <X aria-hidden="true" />
          </button>
        )}
      </header>
      <div className="channel-section-heading">
        <button className="channel-section-toggle" type="button" aria-expanded={channelsExpanded} aria-controls="space-channel-list" onClick={() => setChannelsExpanded((value) => !value)}>
          <ChevronDown aria-hidden="true" />Channels
        </button>
        {owner && <div className="channel-section-actions">
          <button type="button" aria-label="Create channel" title={canCreateChannel ? "Create channel" : `Channel limit reached (${limits?.channelsPerSpace ?? 100})`} disabled={!canCreateChannel} onClick={() => setDialog("channel")}><Plus aria-hidden="true" /></button>
          <details ref={channelMenu} className="channel-section-menu" onKeyDown={(event) => {
            if (event.key === "Escape") {
              event.currentTarget.open = false;
              event.currentTarget.querySelector("summary")?.focus();
            }
          }} onBlur={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
          }}>
            <summary aria-label="Channel options" title="Channel options"><MoreHorizontal aria-hidden="true" /></summary>
            <div className="space-actions">
              <button type="button" disabled={!canCreateChannel} onClick={() => {
                channelMenu.current!.open = false;
                setDialog("channel");
              }}><Plus aria-hidden="true" />Create channel</button>
              <button type="button" onClick={() => {
                channelMenu.current!.open = false;
                setChannelsExpanded((value) => !value);
              }}>{channelsExpanded ? "Collapse" : "Expand"} channels</button>
            </div>
          </details>
        </div>}
      </div>
      <ul id="space-channel-list" hidden={!channelsExpanded}>
        {detail.channels.map((item) => (
          <li key={item.id}>
            <button
              className="channel-select"
              type="button"
              aria-current={item.id === channel?.id ? "page" : undefined}
              aria-busy={pending && detail.space.id === selected.spaceId && item.id === selected.channelId}
              onMouseEnter={() => prefetch(detail.space.id, item.id)}
              onFocus={() => prefetch(detail.space.id, item.id)}
              onClick={() => choose(detail.space.id, item.id)}
            >
              {item.private ? (
                <LockKeyhole aria-hidden="true" />
              ) : (
                <Hash aria-hidden="true" />
              )}
              <span>{item.name}</span>
            </button>
            {owner && (
              <button
                className="channel-manage"
                type="button"
                aria-label={`Manage ${item.name}`}
                onClick={() => setManageChannel(item)}
              >
                <Settings aria-hidden="true" />
              </button>
            )}
          </li>
        ))}
      </ul>
      {error && (
        <p className="space-sidebar-error" role="alert">
          {error}
          <button type="button" onClick={() => setSelected({ ...selected })}>Retry opening</button>
        </p>
      )}
    </nav>
  );

  if (!channel)
    return (
      <>
        <main className="call-page">
          <header className="call-header">
            <a className="wordmark" href="/">
              caper<span className="wordmark-dot">.</span>
            </a>
          </header>
          <section
            className={`call-room spaces-room empty-channel-room${navigationOpen ? " navigation-open" : ""}`}
          >
            {rail}
            <ChannelSidebar>
              <div className="sidebar-channels">{channelNavigation}</div>
              <div className="empty-channel-account">
                <span className="account-avatar" aria-hidden="true">
                  {account?.displayName?.slice(0, 1).toUpperCase()}
                </span>
                <strong>{account?.displayName}</strong>
              </div>
            </ChannelSidebar>
            <div className="stage empty-channel">
              <button
                className="navigation-toggle"
                type="button"
                aria-expanded={navigationOpen}
                onClick={() => setNavigationOpen((open) => !open)}
              >
                <Hash aria-hidden="true" />
                Browse spaces
              </button>
              <Hash aria-hidden="true" />
              <h2>No accessible channels</h2>
              <p>
                {owner
                  ? "Create a channel to start a conversation."
                  : "The owner has not shared a channel with you yet."}
              </p>
              {owner && (
                <button type="button" onClick={() => setDialog("channel")}>
                  Create channel
                </button>
              )}
            </div>
          </section>
        </main>
        {dialog === "space" && (
          <CreateSpaceDialog
            onClose={() => setDialog(undefined)}
            onCreated={(space) => {
              setSpaces((current) => [...current, space]);
              setDialog(undefined);
              choose(space.id);
            }}
          />
        )}
        {dialog === "channel" && (
          <CreateChannelDialog
            space={detail}
            onClose={() => setDialog(undefined)}
            onCreated={(created) => {
              replaceDetail({ ...detail, channels: [created] });
              setDialog(undefined);
              choose(detail.space.id, created.id);
            }}
          />
        )}
        {dialog === "manage-space" && (
          <ManageSpaceDialog
            detail={detail}
            onClose={() => setDialog(undefined)}
            onChanged={replaceDetail}
            onDeleted={forgetSpace}
          />
        )}
        {dialog === "leave-space" && account && (
          <LeaveSpaceDialog
            space={detail.space}
            account={account}
            onClose={() => setDialog(undefined)}
            onLeft={forgetSpace}
          />
        )}
      </>
    );

  return (
    <>
      <Call
        key={channel.id}
        channel={{
          id: channel.id,
          name: channel.name,
          spaceName: detail.space.name,
          demo: detail.space.demo,
        }}
        initialAccount={account}
        initialHistory={view?.history?.channel.id === channel.id ? view.history : undefined}
        initialHistoryError={view?.channelId === channel.id ? view.historyError : undefined}
        onHistoryChange={navigation.current.rememberHistory}
        spaceRail={rail}
        channelNavigation={channelNavigation}
        navigationOpen={navigationOpen}
        onNavigationToggle={() => setNavigationOpen((open) => !open)}
      />
      {dialog === "space" && (
        <CreateSpaceDialog
          onClose={() => setDialog(undefined)}
          onCreated={(space) => {
            setSpaces((current) => [...current, space]);
            setDialog(undefined);
            choose(space.id);
          }}
        />
      )}
      {dialog === "channel" && (
        <CreateChannelDialog
          space={detail}
          onClose={() => setDialog(undefined)}
          onCreated={(created) => {
            replaceDetail({
              ...detail,
              channels: [...detail.channels, created],
            });
            setDialog(undefined);
            choose(detail.space.id, created.id);
            if (created.private) setManageChannel(created);
          }}
        />
      )}
      {dialog === "manage-space" && (
        <ManageSpaceDialog
          detail={detail}
          onClose={() => setDialog(undefined)}
          onChanged={replaceDetail}
          onDeleted={forgetSpace}
        />
      )}
      {dialog === "leave-space" && account && (
        <LeaveSpaceDialog
          space={detail.space}
          account={account}
          onClose={() => setDialog(undefined)}
          onLeft={forgetSpace}
        />
      )}
      {manageChannel && (
        <ManageChannelDialog
          detail={detail}
          channel={manageChannel}
          onClose={() => setManageChannel(undefined)}
          onChanged={(updated) => {
            replaceDetail({
              ...detail,
              channels: detail.channels.map((item) =>
                item.id === updated.id ? updated : item,
              ),
            });
            setManageChannel(updated);
          }}
          onDeleted={() => {
            const remaining = detail.channels.filter(
              (item) => item.id !== manageChannel.id,
            );
            replaceDetail({ ...detail, channels: remaining });
            setManageChannel(undefined);
            choose(detail.space.id, remaining[0]?.id, true);
          }}
        />
      )}
    </>
  );
}
