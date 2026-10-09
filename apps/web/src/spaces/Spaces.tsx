import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import {
  ArrowLeft,
  Ban,
  BellOff,
  ChevronDown,
  ChevronRight,
  Hash,
  LockKeyhole,
  LogOut,
  MessageCircle,
  MoreHorizontal,
  Plus,
  Search,
  Settings,
  X,
} from "lucide-react";
import { getAccount, normalizeUsername, usernameError, type Account } from "../account/client";
import { playSound, preloadSoundEffects } from "../audio/effects";
import { ChatHistoryError } from "../chat/client";
import Wordmark from "../components/Wordmark";
import Avatar, { initial } from "../components/Avatar";
import Call, { type VoiceSlot } from "../pages/Call";
import ChannelSidebar from "../pages/ChannelSidebar";
import MemberPresence from "./MemberPresence";
import { block, refreshBlocks, unblock, useBlockedIds } from "./blocks";
import { DirectNotificationItems, LevelNotificationItems } from "./NotificationMenu";
import {
  changeChannelNotifications,
  changeDirectNotifications,
  changeSpaceNotifications,
  inheritedLevel,
  isMuted,
  overrideFor,
  refreshNotificationSettings,
  useNotificationSettings,
} from "./notifications";
import Tooltip from "../components/Tooltip";
import { createSpaceNavigation, type PreparedSpace } from "./navigation";
import { transitionBrowse } from "./browseTransition";
import {
  acceptChannelInvitation,
  acceptDirectRequest,
  acceptSpaceInvitation,
  addChannelMember,
  addSpaceMember,
  cancelSpaceInvitation,
  channelNameError,
  createChannel,
  createSpace,
  createDirectConversation,
  declineDirectRequest,
  listDirectConversations,
  listPeople,
  readDirectConversation,
  directStatus,
  directUnread,
  deleteChannel,
  deleteSpace,
  declineChannelInvitation,
  declineSpaceInvitation,
  getSpace,
  joinChannel,
  leaveChannel,
  listChannelMembers,
  listSpaceInvitations,
  listSpaces,
  normalizeChannelName,
  removeChannelMember,
  removeSpaceMember,
  spaceNameError,
  updateChannel,
  updateSpace,
  SpacesApiError,
  type BlockedAccount,
  type Channel,
  type DirectConversation,
  type Person,
  type ChannelInvitation,
  type Member,
  type Space,
  type SpaceDetail,
  type SpaceLimits,
} from "./client";
import { friendlyError } from "./errors";
import "./spaces.css";

/** Readable text for server and network failures; native clients share the same sentences. */
const errorMessage = friendlyError;

/** Closes a disclosure menu and focuses its summary, so a dialog opened from it returns focus there. */
function closeMenu(menu: HTMLDetailsElement) {
  menu.open = false;
  menu.querySelector("summary")?.focus();
}

/** Row menus open upward when the scrolling sidebar has no room below them. */
function placeMenu(menu: HTMLDetailsElement) {
  delete menu.dataset.placement;
  const panel = menu.querySelector<HTMLElement>(".space-actions");
  const scroller = menu.closest<HTMLElement>(".channel-navigation");
  if (!menu.open || !panel || !scroller) return;
  const area = scroller.getBoundingClientRect();
  const row = menu.getBoundingClientRect();
  const below = Math.min(area.bottom, window.innerHeight) - row.bottom;
  const above = row.top - Math.max(area.top, 0);
  if (panel.offsetHeight + 8 > below && above > below) menu.dataset.placement = "above";
}

const INACCESSIBLE_DIRECT = "This conversation is not accessible.";

function selectedFromUrl() {
  if (typeof window === "undefined") return {};
  const query = new URLSearchParams(window.location.search);
  return {
    spaceId: query.get("space") ?? undefined,
    channelId: query.get("channel") ?? undefined,
    dmId: query.get("dm") ?? undefined,
  };
}

function Dialog({
  title,
  titleIcon,
  description,
  onClose,
  children,
  width = "standard",
  dismissOnBackdrop = false,
}: {
  title: string;
  titleIcon?: ReactNode;
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
        backdropPress.current =
          event.target === event.currentTarget &&
          (event.clientX < rect.left ||
            event.clientX > rect.right ||
            event.clientY < rect.top ||
            event.clientY > rect.bottom);
      }}
      onClick={(event) => {
        const rect = event.currentTarget.getBoundingClientRect();
        if (
          dismissOnBackdrop &&
          backdropPress.current &&
          event.target === event.currentTarget &&
          (event.clientX < rect.left ||
            event.clientX > rect.right ||
            event.clientY < rect.top ||
            event.clientY > rect.bottom)
        )
          onClose();
        backdropPress.current = false;
      }}
    >
      <header>
        <div>
          <h2 id={titleId}>
            {titleIcon}
            {title}
          </h2>
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

function DeleteConfirmation({
  kind,
  name,
  onClose,
  onDelete,
}: {
  kind: "space" | "channel";
  name: string;
  onClose: () => void;
  onDelete: () => Promise<void>;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const submitting = useRef(false);
  const warned = useRef(false);
  useEffect(() => {
    if (!warned.current) {
      warned.current = true;
      playSound("warning");
    }
  }, []);
  return (
    <Dialog
      title={`Delete ${kind}`}
      dismissOnBackdrop
      onClose={() => {
        if (!submitting.current) onClose();
      }}
    >
      <div className="delete-confirmation">
        <p>
          Delete{" "}
          <strong>
            {kind === "channel" ? "#" : ""}
            {name}
          </strong>{" "}
          for everyone?{" "}
          {kind === "space"
            ? "All its channels and their messages will disappear from the space."
            : "This channel and its messages will disappear from the space."}{" "}
          This cannot be undone.
        </p>
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <div className="space-dialog-actions">
          <button type="button" className="secondary" data-initial-focus disabled={pending} onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="danger"
            disabled={pending}
            onClick={(event) => {
              // A second click from the opener's double-click must not confirm deletion.
              if (event.detail > 1 || submitting.current) return;
              submitting.current = true;
              setPending(true);
              setError(undefined);
              void onDelete()
                .then(() => playSound("delete"))
                .catch((reason) => {
                  setError(errorMessage(reason));
                  submitting.current = false;
                  setPending(false);
                });
            }}
          >
            {pending ? "Deleting…" : `Delete ${kind}`}
          </button>
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
  focus = true,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
  channel?: boolean;
  privateChannel?: boolean;
  showIcon?: boolean;
  focus?: boolean;
}) {
  return (
    <label className="space-field">
      <span>{label}</span>
      <span className={showIcon ? "channel-name-input" : undefined}>
        {showIcon && (privateChannel ? <LockKeyhole aria-hidden="true" /> : <Hash aria-hidden="true" />)}
        <input
          autoFocus={focus}
          data-initial-focus={focus || undefined}
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
          onBlur={() => onChange(channel ? value.replace(/-$/, "") : value.trim())}
        />
      </span>
    </label>
  );
}

function ChannelPrivacy({
  spaceName,
  checked,
  onChange,
}: {
  spaceName: string;
  checked: boolean;
  onChange: (checked: boolean) => void;
}) {
  return (
    <div className="channel-privacy">
      <label>
        <span>
          <LockKeyhole aria-hidden="true" />
          Private channel
        </span>
        <input
          type="checkbox"
          role="switch"
          checked={checked}
          onChange={(event) => {
            playSound(event.target.checked ? "toggle-on" : "toggle-off");
            onChange(event.target.checked);
          }}
          aria-describedby="channel-privacy-help"
        />
      </label>
      <p id="channel-privacy-help">
        {checked ? (
          "Only you and the people you add can view or join."
        ) : (
          <>
            Anyone in <strong>{spaceName}</strong> can view or join this channel.
          </>
        )}
      </p>
    </div>
  );
}

function SubmitRow({
  pending,
  label,
  pendingLabel = "Saving…",
  onCancel,
  destructive = false,
  disabled = false,
}: {
  pending: boolean;
  label: string;
  pendingLabel?: string;
  onCancel: () => void;
  destructive?: boolean;
  disabled?: boolean;
}) {
  return (
    <div className="space-dialog-actions">
      <button type="button" className="secondary" disabled={pending} onClick={onCancel}>
        Cancel
      </button>
      <button type="submit" className={destructive ? "danger" : "primary"} disabled={pending || disabled}>
        {pending ? pendingLabel : label}
      </button>
    </div>
  );
}

function CreateSpaceForm({
  onCancel,
  onCreated,
  disabled = false,
}: {
  onCancel?: () => void;
  onCreated: (space: Space) => void;
  disabled?: boolean;
}) {
  const [name, setName] = useState("");
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (pending || disabled) return;
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
  const form = (
    <form onSubmit={(event) => void submit(event)}>
      <NameField
        label="Space name"
        value={name}
        onChange={(value) => {
          setName(value);
          setError(undefined);
        }}
      />
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      <div className="space-dialog-actions">
        {onCancel && (
          <button type="button" className="secondary" disabled={pending} onClick={onCancel}>
            Cancel
          </button>
        )}
        <button type="submit" className="primary" disabled={pending || disabled || !name.trim()}>
          {pending ? "Creating…" : "Create space"}
        </button>
      </div>
    </form>
  );
  return onCancel ? (
    <Dialog title="Create a space" dismissOnBackdrop={!pending} onClose={onCancel}>
      {form}
    </Dialog>
  ) : (
    form
  );
}

function CreateSpaceDialog({ onClose, onCreated }: { onClose: () => void; onCreated: (space: Space) => void }) {
  return <CreateSpaceForm onCancel={onClose} onCreated={onCreated} />;
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
    <Dialog title="Create a channel" dismissOnBackdrop={!pending} onClose={onClose}>
      <form onSubmit={(event) => void submit(event)}>
        <NameField
          channel
          showIcon
          privateChannel={privateChannel}
          label="Channel name"
          value={name}
          onChange={(value) => {
            setName(value);
            setError(undefined);
          }}
        />
        <p className="channel-name-guidance">
          Channels are where conversations happen around a topic. Use a name that is easy to find and understand.
        </p>
        <ChannelPrivacy spaceName={space.space.name} checked={privateChannel} onChange={setPrivateChannel} />
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <SubmitRow pending={pending} label="Create channel" pendingLabel="Creating…" onCancel={onClose} />
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
      dismissOnBackdrop={!pending}
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
        <SubmitRow pending={pending} label="Leave space" pendingLabel="Leaving…" destructive onCancel={onClose} />
      </form>
    </Dialog>
  );
}

/** Removing someone is easy to do by accident and hard for them to undo. */
function RemoveMemberDialog({
  title,
  description,
  onClose,
  onRemove,
}: {
  title: string;
  description: string;
  onClose: () => void;
  onRemove: () => Promise<void>;
}) {
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  const submitting = useRef(false);
  return (
    <Dialog
      title={title}
      onClose={() => {
        if (!submitting.current) onClose();
      }}
    >
      <div className="delete-confirmation">
        <p>{description}</p>
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <div className="space-dialog-actions">
          <button type="button" className="secondary" data-initial-focus disabled={pending} onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="danger"
            disabled={pending}
            onClick={() => {
              if (submitting.current) return;
              submitting.current = true;
              setPending(true);
              setError(undefined);
              void onRemove()
                .then(onClose)
                .catch((reason) => {
                  setError(errorMessage(reason));
                  submitting.current = false;
                  setPending(false);
                });
            }}
          >
            {pending ? "Removing…" : "Remove"}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

function MemberManager({
  members,
  invitations,
  channel = false,
  onCancel,
  onAdd,
  onRemove,
  removalCopy,
  pending,
  loading = false,
}: {
  members: Member[];
  invitations?: Member[];
  /** A private channel's grants rather than the space's members. */
  channel?: boolean;
  onCancel?: (member: Member) => Promise<void>;
  onAdd: (username: string) => Promise<void>;
  onRemove: (member: Member) => Promise<void>;
  /** Confirmation title and body for removing this member. */
  removalCopy: (member: Member) => { title: string; description: string };
  pending: boolean;
  loading?: boolean;
}) {
  const [username, setUsername] = useState("");
  const [error, setError] = useState<string>();
  const [success, setSuccess] = useState<string>();
  const [removing, setRemoving] = useState<Member>();
  const submitting = useRef(false);
  const input = useRef<HTMLInputElement>(null);
  // The input is disabled while a request runs, so it loses focus. Return focus
  // once it is enabled again so several people can be invited in a row.
  const refocus = useRef(false);
  useEffect(() => {
    if (pending || !refocus.current) return;
    refocus.current = false;
    input.current?.focus();
  });
  return (
    <section className="member-manager">
      <div className="dialog-section-heading">
        <h3>Members</h3>
        {!loading && <span>{members.length}</span>}
      </div>
      <form
        className="member-add"
        onSubmit={(event) => {
          event.preventDefault();
          if (submitting.current || pending) return;
          const invalid = usernameError(username);
          if (invalid) return setError(invalid);
          if (invitations && members.some((member) => member.username === username))
            return setError(
              channel ? "This person already has access to this channel." : "This person is already in the space.",
            );
          if (invitations?.some((member) => member.username === username))
            return setError("This person already has a pending invitation.");
          submitting.current = true;
          refocus.current = true;
          setError(undefined);
          setSuccess(undefined);
          void onAdd(username)
            .then(() => {
              setUsername("");
              setSuccess(invitations ? "Invitation sent. They must accept before joining." : "Access granted.");
            })
            .catch((reason) => setError(errorMessage(reason)))
            .finally(() => {
              submitting.current = false;
            });
        }}
      >
        <label className="sr-only" htmlFor="member-username">
          Exact username
        </label>
        <input
          ref={input}
          id="member-username"
          value={username}
          autoComplete="off"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          minLength={3}
          maxLength={32}
          pattern="[a-z0-9_]{3,32}"
          required
          disabled={pending}
          placeholder="Exact username"
          onChange={(event) => {
            setUsername(normalizeUsername(event.target.value));
            setError(undefined);
            setSuccess(undefined);
          }}
        />
        <button type="submit" disabled={pending}>
          {invitations ? "Invite" : "Add"}
        </button>
      </form>
      <p className="channel-name-guidance">
        3–32 lowercase letters, numbers, or underscores.{invitations && " Invitations expire after 7 days."}
      </p>
      {success && (
        <p className="channel-name-guidance" role="status">
          {success}
        </p>
      )}
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      {loading && (
        <p className="channel-name-guidance" role="status">
          Loading members…
        </p>
      )}
      <ul>
        {members.map((member) => (
          <li key={member.id}>
            {/* TODO show status indicator */}
            <span className="member-avatar" aria-hidden="true">
              <Avatar avatarId={member.avatarId} name={member.displayName} />
            </span>
            <span>
              <strong>{member.displayName}</strong>
              <small>
                @{member.username}
                {member.owner ? " · Owner" : ""}
              </small>
            </span>
            {!member.owner && (
              <button type="button" disabled={pending} onClick={() => setRemoving(member)}>
                Remove
              </button>
            )}
          </li>
        ))}
      </ul>
      {removing && (
        <RemoveMemberDialog
          {...removalCopy(removing)}
          onClose={() => setRemoving(undefined)}
          onRemove={async () => {
            await onRemove(removing);
            // The removed row (and its button) is gone; continue in the username field.
            refocus.current = true;
          }}
        />
      )}
      {!!invitations?.length && (
        <>
          <div className="dialog-section-heading">
            <h3>Pending invitations</h3>
            <span>{invitations.length}</span>
          </div>
          <ul>
            {invitations.map((member) => (
              <li key={member.id}>
                <span className="member-avatar" aria-hidden="true">
                  <Avatar avatarId={member.avatarId} name={member.displayName} />
                </span>
                <span>
                  <strong>{member.displayName}</strong>
                  <small>@{member.username} · Invited</small>
                </span>
                <button
                  type="button"
                  disabled={pending}
                  onClick={() => void onCancel?.(member).catch((reason) => setError(errorMessage(reason)))}
                >
                  Cancel invite
                </button>
              </li>
            ))}
          </ul>
        </>
      )}
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
  const [invitations, setInvitations] = useState<Member[]>([]);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string>();
  const [pending, setPending] = useState(false);
  useEffect(() => {
    let current = true;
    void listSpaceInvitations(detail.space.id)
      .then((result) => {
        if (current) setInvitations(result.members);
      })
      .catch((reason) => {
        if (current) setError(errorMessage(reason));
      });
    return () => {
      current = false;
    };
  }, [detail.space.id]);
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
      dismissOnBackdrop
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          const invalid = spaceNameError(name);
          if (invalid) return setError(invalid);
          void run(async () => {
            const space = await updateSpace(detail.space.id, name);
            setName(space.name);
            onChanged({ ...detail, space, members });
          }).catch((reason) => setError(errorMessage(reason)));
        }}
      >
        <fieldset disabled={pending}>
          <NameField
            label="Space name"
            focus={false}
            value={name}
            onChange={(value) => {
              setName(value);
              setError(undefined);
            }}
          />
          <div className="inline-save">
            <button className="secondary" disabled={pending || name.trim() === detail.space.name} type="submit">
              Save name
            </button>
          </div>
        </fieldset>
      </form>
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      <MemberManager
        members={members}
        invitations={invitations}
        pending={pending}
        onAdd={(username) =>
          run(async () => {
            const member = await addSpaceMember(detail.space.id, username);
            setInvitations((current) => [...current.filter((item) => item.id !== member.id), member]);
          })
        }
        onCancel={(member) =>
          run(async () => {
            await cancelSpaceInvitation(detail.space.id, member.id);
            setInvitations((current) => current.filter((item) => item.id !== member.id));
          })
        }
        removalCopy={(member) => ({
          title: `Remove ${member.displayName}?`,
          description: `They’ll lose access to ${detail.space.name} and its channels. You can invite them again later.`,
        })}
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
        <button className="danger-outline" type="button" disabled={pending} onClick={() => setConfirmDelete(true)}>
          Delete space
        </button>
      </section>
      {confirmDelete && (
        <DeleteConfirmation
          kind="space"
          name={detail.space.name}
          onClose={() => setConfirmDelete(false)}
          onDelete={async () => {
            await deleteSpace(detail.space.id);
            onDeleted();
          }}
        />
      )}
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
  const [invitations, setInvitations] = useState<Member[]>([]);
  const [membersLoaded, setMembersLoaded] = useState(false);
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
    if (!channel.private) return;
    let current = true;
    void listChannelMembers(detail.space.id, channel.id)
      .then((value) => {
        if (!current) return;
        setMembers(value.members);
        setInvitations(value.invitations ?? []);
      })
      .catch((reason) => {
        if (current) setError(errorMessage(reason));
      })
      .finally(() => {
        if (current) setMembersLoaded(true);
      });
    return () => {
      current = false;
    };
  }, [channel.id, channel.private, detail.space.id]);
  return (
    <Dialog title="Channel settings" onClose={onClose} width="wide" dismissOnBackdrop>
      <form
        id="channel-overview"
        onSubmit={(event) => {
          event.preventDefault();
          if (pending || !dirty) return;
          const channelName = name.replace(/-$/, "");
          const invalid = channelNameError(channelName);
          if (invalid) return setError(invalid);
          void run(async () => {
            const updated = await updateChannel(detail.space.id, channel.id, channelName, privateChannel);
            setName(updated.name);
            onChanged(updated);
          }).catch((reason) => setError(errorMessage(reason)));
        }}
      >
        <fieldset disabled={pending}>
          <NameField
            channel
            focus={false}
            label="Channel name"
            value={name}
            onChange={(value) => {
              setName(value);
              setError(undefined);
            }}
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
          loading={!membersLoaded}
          members={members}
          invitations={invitations}
          channel
          pending={pending}
          onAdd={async (username) =>
            run(async () => {
              const member = await addChannelMember(detail.space.id, channel.id, username);
              setInvitations((current) => [...current.filter((item) => item.id !== member.id), member]);
            })
          }
          onCancel={(member) =>
            run(async () => {
              await removeChannelMember(detail.space.id, channel.id, member.id);
              setInvitations((items) => items.filter((item) => item.id !== member.id));
            })
          }
          removalCopy={(member) => ({
            title: `Remove ${member.displayName} from #${channel.name}?`,
            description: "They’ll lose access to this private channel. You can add them again later.",
          })}
          onRemove={async (member) =>
            run(async () => {
              await removeChannelMember(detail.space.id, channel.id, member.id);
              setMembers((current) => current.filter((item) => item.id !== member.id));
            })
          }
        />
      )}
      <section className="danger-zone">
        <h3>Delete channel</h3>
        <p>Delete this channel for everyone in the space.</p>
        <button className="danger-outline" type="button" disabled={pending} onClick={() => setConfirmDelete(true)}>
          Delete channel
        </button>
      </section>
      <footer className="channel-save-bar" aria-hidden={!dirty} inert={!dirty}>
        <span role="status">You have unsaved changes.</span>
        <div>
          <button
            type="button"
            className="secondary"
            disabled={pending}
            onClick={() => {
              setName(channel.name);
              setPrivateChannel(channel.private);
              setError(undefined);
            }}
          >
            Reset
          </button>
          <button type="submit" form="channel-overview" className="primary" disabled={pending}>
            {pending ? "Saving…" : "Save changes"}
          </button>
        </div>
      </footer>
      {confirmDelete && (
        <DeleteConfirmation
          kind="channel"
          name={channel.name}
          onClose={() => setConfirmDelete(false)}
          onDelete={async () => {
            await deleteChannel(detail.space.id, channel.id);
            onDeleted();
          }}
        />
      )}
    </Dialog>
  );
}

function StartDirectDialog({
  onClose,
  onCreated,
}: {
  onClose: () => void;
  onCreated: (conversation: DirectConversation) => void;
}) {
  const [username, setUsername] = useState("");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  return (
    <Dialog
      title="New direct message"
      description="Enter an exact username. Conversations stay private across all your spaces."
      dismissOnBackdrop={!pending}
      onClose={onClose}
    >
      <form
        onSubmit={(event) => {
          event.preventDefault();
          if (pending || !username.trim()) return;
          setPending(true);
          setError(undefined);
          void createDirectConversation(username)
            .then(onCreated)
            .catch((reason) => {
              setError(errorMessage(reason));
              setPending(false);
            });
        }}
      >
        <label className="space-field">
          Username
          <input
            autoFocus
            data-initial-focus
            autoComplete="off"
            value={username}
            maxLength={33}
            placeholder="@username"
            onChange={(event) => {
              setUsername(event.target.value);
              setError(undefined);
            }}
          />
        </label>
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <SubmitRow
          pending={pending}
          label="Open conversation"
          pendingLabel="Opening…"
          disabled={!username.trim()}
          onCancel={onClose}
        />
      </form>
    </Dialog>
  );
}

/** Confirms a block. Blocking also declines any request they sent you. */
function BlockDialog({
  person,
  onClose,
  onBlocked,
}: {
  person: BlockedAccount;
  onClose: () => void;
  onBlocked: () => void;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  return (
    <Dialog
      title={`Block ${person.displayName}?`}
      dismissOnBackdrop
      onClose={() => {
        if (!pending) onClose();
      }}
    >
      <div className="delete-confirmation">
        <p>
          You won’t see their messages unless you choose to, and they can’t send you DMs or requests. They aren’t told.
          You can unblock them in Edit profile.
        </p>
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <div className="space-dialog-actions">
          <button type="button" className="secondary" data-initial-focus disabled={pending} onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="danger"
            disabled={pending}
            onClick={() => {
              setPending(true);
              setError(undefined);
              void block(person)
                .then(onBlocked)
                .catch((reason) => {
                  setError(errorMessage(reason));
                  setPending(false);
                });
            }}
          >
            {pending ? "Blocking…" : "Block"}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

/** Shown instead of the composer while a request for you is open. */
function RequestBar({
  conversation,
  onAccept,
  onDecline,
  onBlock,
}: {
  conversation: DirectConversation;
  onAccept: () => Promise<void>;
  onDecline: () => Promise<void>;
  onBlock: () => void;
}) {
  const [pending, setPending] = useState<"accept" | "decline">();
  const [error, setError] = useState<string>();
  const run = (action: "accept" | "decline", task: () => Promise<void>) => {
    if (pending) return;
    setPending(action);
    setError(undefined);
    void task()
      .catch((reason) => setError(errorMessage(reason)))
      .finally(() => setPending(undefined));
  };
  return (
    <div className="direct-request-bar" role="group" aria-label="Message request">
      <p>
        <strong>{conversation.peer.displayName}</strong> (@{conversation.peer.username}) wants to message you. You don’t
        share a space.
      </p>
      <div>
        <button type="button" className="primary" disabled={!!pending} onClick={() => run("accept", onAccept)}>
          {pending === "accept" ? "Accepting…" : "Accept"}
        </button>
        <button type="button" className="secondary" disabled={!!pending} onClick={() => run("decline", onDecline)}>
          {pending === "decline" ? "Declining…" : "Decline"}
        </button>
        <button type="button" className="danger-outline" disabled={!!pending} onClick={onBlock}>
          Block
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}

function BlockedBar({ conversation }: { conversation: DirectConversation }) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  return (
    <div className="direct-request-bar" role="group" aria-label="Blocked conversation">
      <p>You blocked @{conversation.peer.username}.</p>
      <div>
        <button
          type="button"
          className="secondary"
          disabled={pending}
          onClick={() => {
            setPending(true);
            setError(undefined);
            void unblock(conversation.peer.id)
              .catch((reason) => setError(errorMessage(reason)))
              .finally(() => setPending(false));
          }}
        >
          {pending ? "Unblocking…" : "Unblock"}
        </button>
      </div>
      {error && <p role="alert">{error}</p>}
    </div>
  );
}

function SpacesLoading() {
  return (
    <main className="call-page" aria-busy="true">
      <header className="call-header">
        <Wordmark />
      </header>
      <section className="call-room spaces-room spaces-loading">
        <div className="space-rail" aria-hidden="true" />
        <ChannelSidebar>
          <div className="sidebar-channels" />
        </ChannelSidebar>
        <div className="stage">
          <p className="sr-only" role="status">
            Loading your spaces…
          </p>
        </div>
      </section>
    </main>
  );
}

function InvitationDialog({
  space,
  onClose,
  onAccepted,
  onDeclined,
}: {
  space: Space;
  onClose: () => void;
  onAccepted: (space: Space) => void;
  onDeclined: () => void;
}) {
  const [pending, setPending] = useState<"accept" | "decline">();
  const [error, setError] = useState<string>();
  const submitting = useRef(false);
  const respond = async (accept: boolean) => {
    if (submitting.current) return;
    submitting.current = true;
    setPending(accept ? "accept" : "decline");
    setError(undefined);
    try {
      if (accept) onAccepted(await acceptSpaceInvitation(space.id));
      else {
        await declineSpaceInvitation(space.id);
        onDeclined();
      }
    } catch (reason) {
      if (reason instanceof SpacesApiError && reason.status === 404) {
        setError("This invitation is no longer available. Close this dialog to refresh your spaces.");
      } else setError(errorMessage(reason));
    } finally {
      submitting.current = false;
      setPending(undefined);
    }
  };
  return (
    <Dialog
      title="You’re invited!"
      titleIcon={<img src="/images/invitation/1f4e8.png" width={32} height={32} alt="" />}
      onClose={() => {
        if (!submitting.current) onClose();
      }}
    >
      <div className="invitation-consent">
        <h3>Join {space.name}?</h3>
        {space.inviter && (
          <p>
            <strong>{space.inviter.displayName}</strong> (@{space.inviter.username}) invited you.
          </p>
        )}
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <div className="space-dialog-actions">
          <button
            className="secondary"
            type="button"
            data-initial-focus
            disabled={!!pending}
            onClick={() => void respond(false)}
          >
            {pending === "decline" ? "Declining…" : "Decline"}
          </button>
          <button className="primary" type="button" disabled={!!pending} onClick={() => void respond(true)}>
            {pending === "accept" ? "Accepting…" : "Accept invitation"}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

function BrowseChannelsDialog({
  detail,
  onClose,
  onPreview,
  onManage,
}: {
  detail: SpaceDetail;
  onClose: () => void;
  onPreview: (channel: Channel) => void;
  onManage?: (channel: Channel) => void;
}) {
  const [query, setQuery] = useState("");
  // People often type the name as it is shown, with its `#`.
  const search = query.trim().replace(/^#+/, "").trim().toLowerCase();
  const channels = detail.channels.filter((item) => item.name.includes(search));
  return (
    <Dialog
      title="Browse channels"
      description={`Find conversations in ${detail.space.name}. Previewing a channel doesn’t join it.`}
      onClose={onClose}
      width="wide"
      dismissOnBackdrop
    >
      <div className="channel-directory">
        <label className="space-field">
          Search channels
          <input
            data-initial-focus
            value={query}
            onChange={(event) => setQuery(event.target.value)}
            placeholder="Search by name"
          />
        </label>
        <ul>
          {channels.map((item) => (
            <li key={item.id}>
              <button type="button" aria-label={`Preview #${item.name}`} onClick={() => onPreview(item)}>
                {item.private ? <LockKeyhole aria-hidden="true" /> : <Hash aria-hidden="true" />}
                <span>
                  {item.name}
                  <small>{item.joined !== false ? "Joined" : "Read-only preview"}</small>
                </span>
              </button>
              {onManage && (
                <button
                  className="directory-manage"
                  type="button"
                  aria-label={`Manage ${item.name}`}
                  onClick={() => onManage(item)}
                >
                  <Settings aria-hidden="true" />
                </button>
              )}
            </li>
          ))}
        </ul>
        {!channels.length && (
          <p role="status">{query.trim() ? "No channels match your search." : "This space has no channels yet."}</p>
        )}
      </div>
    </Dialog>
  );
}

function ChannelInvitationDialog({
  invitation,
  spaceName,
  onClose,
  onRespond,
}: {
  invitation: ChannelInvitation;
  spaceName: string;
  onClose: () => void;
  onRespond: (accept: boolean) => Promise<void>;
}) {
  const [pending, setPending] = useState<"accept" | "decline">();
  const [error, setError] = useState<string>();
  const submitting = useRef(false);
  const respond = async (accept: boolean) => {
    if (submitting.current) return;
    submitting.current = true;
    setPending(accept ? "accept" : "decline");
    setError(undefined);
    try {
      await onRespond(accept);
    } catch (reason) {
      setError(errorMessage(reason));
    } finally {
      submitting.current = false;
      setPending(undefined);
    }
  };
  return (
    <Dialog
      title="Private channel invitation"
      onClose={() => {
        if (!submitting.current) onClose();
      }}
    >
      <div className="invitation-consent channel-invitation-consent">
        <LockKeyhole aria-hidden="true" />
        <h3>Join #{invitation.channel.name}?</h3>
        <p>
          <strong>{invitation.inviter.displayName}</strong> (@{invitation.inviter.username}) invited you to this private
          channel in <strong>{spaceName}</strong>.
        </p>
        <p>
          Messages stay hidden until you accept. Acceptance joins the channel; it does not enter voice. Invitations
          expire seven days after they’re sent.
        </p>
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <div className="space-dialog-actions">
          <button
            type="button"
            className="secondary"
            data-initial-focus
            disabled={!!pending}
            onClick={() => void respond(false)}
          >
            {pending === "decline" ? "Declining…" : "Decline"}
          </button>
          <button type="button" className="primary" disabled={!!pending} onClick={() => void respond(true)}>
            {pending === "accept" ? "Accepting…" : "Accept invitation"}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

function LeaveChannelDialog({
  channel,
  owner,
  onClose,
  onLeave,
}: {
  channel: Channel;
  owner: boolean;
  onClose: () => void;
  onLeave: () => Promise<void>;
}) {
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const submitting = useRef(false);
  return (
    <Dialog
      title="Leave channel"
      dismissOnBackdrop={!pending}
      onClose={() => {
        if (!submitting.current) onClose();
      }}
    >
      <div className="delete-confirmation leave-channel-consent">
        <p>
          Leave <strong>#{channel.name}</strong>?{" "}
          {channel.private && !owner
            ? "You’ll lose access and need another invitation to return."
            : owner && channel.private
              ? "It will leave your sidebar. You keep owner access and can rejoin from Browse channels."
              : "It will leave your sidebar. You can still preview it and rejoin from Browse channels."}{" "}
          If you’re in its voice call, you’ll disconnect.
        </p>
        {error && (
          <p className="space-form-error" role="alert">
            {error}
          </p>
        )}
        <div className="space-dialog-actions">
          <button type="button" className="secondary" data-initial-focus disabled={pending} onClick={onClose}>
            Cancel
          </button>
          <button
            type="button"
            className="primary"
            disabled={pending}
            onClick={() => {
              if (submitting.current) return;
              submitting.current = true;
              setPending(true);
              setError(undefined);
              void onLeave()
                .catch((reason) => setError(errorMessage(reason)))
                .finally(() => {
                  submitting.current = false;
                  setPending(false);
                });
            }}
          >
            {pending ? "Leaving…" : "Leave channel"}
          </button>
        </div>
      </div>
    </Dialog>
  );
}

export default function Spaces({
  embedded = false,
  initialAccount,
  initialSpaceList,
  engaged = true,
  onChatOnlineChange,
}: {
  embedded?: boolean;
  initialAccount?: Account;
  initialSpaceList?: { spaces: Space[]; invitations?: Space[]; limits: SpaceLimits };
  engaged?: boolean;
  onChatOnlineChange?: (online: boolean) => void;
} = {}) {
  const [account, setAccount] = useState<Account | undefined>(initialAccount);
  const [spaces, setSpaces] = useState<Space[]>(initialSpaceList?.spaces ?? []);
  const [invitations, setInvitations] = useState<Space[]>(initialSpaceList?.invitations ?? []);
  // From the rail an invitation opens over the room instead of navigating away,
  // which would unmount the room and end a call in progress.
  const [railInvitation, setRailInvitation] = useState<Space>();
  const [limits, setLimits] = useState<SpaceLimits | undefined>(initialSpaceList?.limits);
  const [view, setView] = useState<PreparedSpace>();
  const [directs, setDirects] = useState<DirectConversation[]>([]);
  const [directError, setDirectError] = useState<string>();
  const [selfDirectPending, setSelfDirectPending] = useState(false);
  const [requestsOpen, setRequestsOpen] = useState<boolean>();
  const [blockTarget, setBlockTarget] = useState<BlockedAccount>();
  const blockedIds = useBlockedIds();
  const notifications = useNotificationSettings();
  // Shown under the row whose notification change failed; the store has already reverted it.
  const [notificationError, setNotificationError] = useState<string>();
  const saveNotifications = (key: string, save: () => Promise<void>) => {
    setNotificationError(undefined);
    void save().catch(() => setNotificationError(key));
  };
  const notificationAlert = (key: string) =>
    notificationError === key && (
      <p className="space-sidebar-error notification-error" role="alert">
        That change wasn’t saved. Try again.
        <button type="button" onClick={() => setNotificationError(undefined)}>
          Dismiss
        </button>
      </p>
    );
  const [directView, setDirectView] = useState<{ conversation: DirectConversation }>();
  // DM `@` suggestions and mention profile cards; the previous list stays while
  // a refresh is in flight. Loaded once signed in and again whenever a DM opens.
  const [people, setPeople] = useState<Person[]>();
  const directId = directView?.conversation.id;
  useEffect(() => {
    if (!account?.id) return;
    let current = true;
    void listPeople()
      .then((result) => {
        if (current) setPeople(result.people);
      })
      .catch(() => {});
    return () => {
      current = false;
    };
  }, [directId, account?.id]);
  const detail =
    view?.detail ??
    (!spaces.length
      ? { space: { id: "", name: "Direct messages", ownerId: "" }, channels: [], members: [] }
      : undefined);
  const navigation = useRef(createSpaceNavigation());
  // The rendered detail, for the poll below whose effect tracks only the space id.
  const shownDetail = useRef(detail);
  useEffect(() => {
    shownDetail.current = detail;
  });
  const [selected, setSelected] = useState<{ spaceId?: string; channelId?: string; dmId?: string }>(() =>
    embedded ? {} : selectedFromUrl(),
  );
  const navigationRevision = useRef(0);
  const activeSpace = useRef(selected.spaceId);
  activeSpace.current = selected.spaceId;
  const membershipRevision = useRef(0);
  const membershipSubmitting = useRef(false);
  const [membershipPending, setMembershipPending] = useState(false);
  const [membershipError, setMembershipError] = useState<string>();
  const [browseOpen, setBrowseOpen] = useState(false);
  const [channelInvitation, setChannelInvitation] = useState<ChannelInvitation>();
  const [leavingChannel, setLeavingChannel] = useState<Channel>();
  const [loading, setLoading] = useState(!initialSpaceList);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const invitation = invitations.find((item) => item.id === selected.spaceId);
  const [dialog, setDialog] = useState<"space" | "channel" | "manage-space" | "leave-space" | "direct">();
  const [manageChannel, setManageChannel] = useState<Channel>();
  const [navigationOpen, showNavigation] = useState(false);
  // Where Browse is headed: it changes inside a slide's view transition, after the request.
  const navigationTarget = useRef(false);
  const changeNavigation = useCallback((open: boolean) => {
    navigationTarget.current = open;
    showNavigation(open);
  }, []);
  const setNavigationOpen = useCallback((next: boolean | ((open: boolean) => boolean)) => {
    const open = typeof next === "function" ? next(navigationTarget.current) : next;
    if (open === navigationTarget.current) return;
    navigationTarget.current = open;
    transitionBrowse(open, () => showNavigation(open));
  }, []);
  const spaceMenu = useRef<HTMLDetailsElement>(null);
  const channelMenu = useRef<HTMLDetailsElement>(null);
  const channelNavigationRef = useRef<HTMLElement>(null);
  const [channelsExpanded, setChannelsExpanded] = useState(true);

  useEffect(() => {
    const dismiss = (event: PointerEvent) => {
      // Every menu, including DM rows outside the channel list and the narrow-layout
      // channel title menu: Safari does not focus a tapped <summary>, so onBlur alone
      // never closes them on an outside tap.
      for (const menu of [
        spaceMenu.current,
        channelMenu.current,
        ...document.querySelectorAll<HTMLDetailsElement>(".channel-menu[open], .chat-channel-menu[open]"),
      ]) {
        if (menu && !menu.contains(event.target as Node)) menu.open = false;
      }
    };
    document.addEventListener("pointerdown", dismiss);
    return () => document.removeEventListener("pointerdown", dismiss);
  }, []);

  const choose = (spaceId?: string, channelId?: string, replace = false, dmId?: string) => {
    navigationRevision.current++;
    if (spaceMenu.current) spaceMenu.current.open = false;
    if (channelMenu.current) channelMenu.current.open = false;
    channelNavigationRef.current?.querySelectorAll<HTMLDetailsElement>(".channel-menu[open]").forEach((menu) => {
      menu.open = false;
    });
    // A repeated click must not replace `selected`: that would restart the
    // navigation effect and remount Chat, losing its draft and scroll state.
    // Comparing with the requested selection (rather than `view`) also folds
    // duplicate clicks into one pending request. If another channel is pending,
    // clicking the still-visible channel differs from `selected` and therefore
    // intentionally cancels that navigation.
    if (spaceId === selected.spaceId && channelId === selected.channelId && dmId === selected.dmId) {
      setNavigationOpen(false);
      // Previewing the channel already on screen from Browse still closes it.
      setBrowseOpen(false);
      return;
    }
    activeSpace.current = spaceId;
    setMembershipError(undefined);
    setBrowseOpen(false);
    setChannelInvitation(undefined);
    setLeavingChannel(undefined);
    const query = new URLSearchParams();
    if (spaceId) query.set("space", spaceId);
    if (channelId) query.set("channel", channelId);
    if (dmId) query.set("dm", dmId);
    if (!embedded)
      window.history[replace ? "replaceState" : "pushState"]({}, "", `/spaces${query.size ? `?${query}` : ""}`);
    setSelected({ spaceId, channelId, dmId });
  };

  const openDirect = (conversation: DirectConversation) => {
    choose(detail?.space.id || undefined, undefined, false, conversation.id);
    setNavigationOpen(false);
  };

  const selfDirect = directs.find((conversation) => conversation.peer.id === account?.id);
  const openSelfDirect = async () => {
    if (!account?.username || selfDirectPending) return;
    if (selfDirect) return openDirect(selfDirect);
    const request = ++navigationRevision.current;
    setSelfDirectPending(true);
    setDirectError(undefined);
    try {
      const conversation = await createDirectConversation(account.username);
      setDirects((current) => [conversation, ...current.filter((item) => item.id !== conversation.id)]);
      if (request === navigationRevision.current) openDirect(conversation);
    } catch (reason) {
      setDirectError(errorMessage(reason));
    } finally {
      setSelfDirectPending(false);
    }
  };

  const refreshDirects = async () => {
    try {
      setDirects((await listDirectConversations()).conversations);
      setDirectError(undefined);
    } catch (reason) {
      setDirectError(errorMessage(reason));
    }
  };

  useEffect(() => {
    if (!account) return;
    let current = true;
    const refresh = () => {
      if (document.visibilityState !== "visible") return;
      void listDirectConversations()
        .then((result) => {
          if (current) {
            setDirects(result.conversations);
            setDirectError(undefined);
          }
        })
        .catch((reason) => {
          if (current) setDirectError(errorMessage(reason));
        });
    };
    // Account initialization below owns the first read. Start only the poll
    // here, otherwise mounting the page fetches the same DM list twice.
    const timer = setInterval(refresh, 15_000);
    document.addEventListener("visibilitychange", refresh);
    // Blocked accounts' messages collapse in every chat.
    void refreshBlocks().catch(() => undefined);
    // Mutes dim sidebar rows; menus and settings read the same copy.
    void refreshNotificationSettings().catch(() => undefined);
    return () => {
      current = false;
      clearInterval(timer);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [account?.id]);

  const directAccessible = directs.some((item) => item.id === selected.dmId);
  useEffect(() => {
    // The "not accessible" notice belongs to the DM that was asked for; leaving it clears it.
    const clearInaccessible = () =>
      setDirectError((current) => (current === INACCESSIBLE_DIRECT ? undefined : current));
    if (loading || !selected.dmId) {
      setDirectView(undefined);
      clearInaccessible();
      return;
    }
    const conversation = directs.find((item) => item.id === selected.dmId);
    if (!conversation) {
      setDirectView(undefined);
      setDirectError(INACCESSIBLE_DIRECT);
      return;
    }
    // A deep link checks before the list loads; drop that error once it arrives.
    clearInaccessible();
    // ChatClient owns the history read, its cancellation, and explicit retries.
    // Mounting it while also fetching here issued two reads for every DM open.
    setDirectView({ conversation });
    setNavigationOpen(false);
  }, [selected.dmId, loading, directAccessible]);

  const readDirect = (seq: string) => {
    const conversation = directView?.conversation;
    if (!conversation) return;
    const latest = directs.find((item) => item.id === conversation.id);
    if (latest && BigInt(latest.readSeq) >= BigInt(seq)) return;
    void readDirectConversation(conversation.id, seq)
      .then(() =>
        setDirects((current) =>
          current.map((item) =>
            item.id === conversation.id
              ? {
                  ...item,
                  readSeq: BigInt(item.readSeq) > BigInt(seq) ? item.readSeq : seq,
                  lastSeq: BigInt(item.lastSeq) > BigInt(seq) ? item.lastSeq : seq,
                }
              : item,
          ),
        ),
      )
      .catch(() => undefined);
  };

  const prefetch = (spaceId: string, channelId?: string) => {
    if (spaceId === detail?.space.id && (!channelId || channelId === view?.channelId)) return;
    void navigation.current.prepare(spaceId, channelId).catch(() => undefined);
  };

  useEffect(() => {
    if (embedded) return;
    const pop = () => {
      navigationRevision.current++;
      // Space and channel dialogs belong to the view being left; closing them
      // keeps a save from applying stale state to the space Back opens.
      setDialog(undefined);
      setManageChannel(undefined);
      setBrowseOpen(false);
      setChannelInvitation(undefined);
      setLeavingChannel(undefined);
      setSelected(selectedFromUrl());
    };
    window.addEventListener("popstate", pop);
    return () => window.removeEventListener("popstate", pop);
  }, []);

  useEffect(() => {
    let current = true;
    if (engaged) void preloadSoundEffects();
    void (initialAccount ? Promise.resolve(initialAccount) : getAccount())
      .then(async (nextAccount) => {
        if (!current) return;
        if (nextAccount && (!nextAccount.username || !nextAccount.displayName))
          return void window.location.assign("/profile");
        if (!nextAccount) return void window.location.replace("/login");
        const result = initialSpaceList ?? (await listSpaces());
        if (!current) return;
        setAccount(nextAccount);
        setSpaces(result.spaces);
        setInvitations(result.invitations ?? []);
        setLimits(result.limits);
        try {
          setDirects((await listDirectConversations()).conversations);
        } catch (reason) {
          if (current) setDirectError(errorMessage(reason));
        }
        if (!current) return;
        setLoading(false);
        if (![...result.spaces, ...(result.invitations ?? [])].some((space) => space.id === selected.spaceId))
          choose(result.spaces[0]?.id, undefined, true, selected.dmId);
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
    if (invitation) {
      setView(undefined);
      setPending(false);
      setError(undefined);
      return;
    }
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
    void navigation.current
      .take(selected.spaceId, selected.channelId)
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
        if (selected.dmId) {
          query.delete("channel");
          query.set("dm", selected.dmId);
        }
        if (!embedded) window.history.replaceState({}, "", `/spaces?${query}`);
      })
      .catch((reason) => {
        if (current) {
          if (
            (reason instanceof SpacesApiError || reason instanceof ChatHistoryError) &&
            [401, 403, 404].includes(reason.status)
          ) {
            navigation.current.forget(selected.spaceId!);
            setView((shown) => (shown?.detail.space.id === selected.spaceId ? undefined : shown));
            if (reason instanceof SpacesApiError && reason.status === 404 && reason.message === "resource not found") {
              const remaining = spaces.filter((space) => space.id !== selected.spaceId);
              setSpaces(remaining);
              setDialog(undefined);
              setManageChannel(undefined);
              setNotice("This space is no longer available.");
              choose(remaining[0]?.id, undefined, true);
              setPending(false);
              return;
            }
          }
          setError(errorMessage(reason));
          setPending(false);
        }
      });
    return () => {
      current = false;
    };
  }, [selected, loading, invitation?.id]);

  // Membership can change in another account/tab while this page is open.
  // Reconcile the rail on focus and every 15s; transport authorization still
  // enforces access independently of this UI refresh.
  useEffect(() => {
    if (loading || !account) return;
    let current = true;
    let refreshing = false;
    const refresh = async () => {
      if (document.hidden || refreshing) return;
      refreshing = true;
      try {
        const revision = membershipRevision.current;
        const result = await listSpaces();
        if (!current) return;
        const available = new Set(result.spaces.map((space) => space.id));
        for (const space of spaces) if (!available.has(space.id)) navigation.current.forget(space.id);
        setSpaces(result.spaces);
        setInvitations(result.invitations ?? []);
        if (detail?.space.id && !available.has(detail.space.id)) {
          setView(undefined);
          setDialog(undefined);
          setManageChannel(undefined);
          setNotice("This space is no longer available.");
          choose(result.spaces[0]?.id, undefined, true, selected.dmId);
        }
        if (invitation && !(result.invitations ?? []).some((item) => item.id === invitation.id))
          choose(result.spaces[0]?.id, undefined, true, selected.dmId);
        if (!selected.dmId && detail?.space.id && available.has(detail.space.id) && !membershipSubmitting.current) {
          const next = await getSpace(detail.space.id);
          // Replacing forgets every visited channel's saved history in the
          // space, so an unchanged poll result must not cost a refetch later.
          if (
            current &&
            revision === membershipRevision.current &&
            activeSpace.current === next.space.id &&
            JSON.stringify(next) !== JSON.stringify(shownDetail.current)
          )
            replaceDetail(next);
        }
      } catch {
        /* An outage is not revocation; keep known navigation. */
      } finally {
        refreshing = false;
      }
    };
    const interval = window.setInterval(() => void refresh(), 15_000);
    window.addEventListener("focus", refresh);
    document.addEventListener("visibilitychange", refresh);
    return () => {
      current = false;
      clearInterval(interval);
      window.removeEventListener("focus", refresh);
      document.removeEventListener("visibilitychange", refresh);
    };
  }, [loading, account?.id, detail?.space.id, selected.spaceId, selected.dmId, invitation?.id]);

  // The list is polled and updated on accept, so read the latest status from it.
  const currentDirect = directView
    ? (directs.find((conversation) => conversation.id === directView.conversation.id) ?? directView.conversation)
    : undefined;
  const directBlocked = !!currentDirect && blockedIds.has(currentDirect.peer.id);
  const directLocked = !!currentDirect && (directBlocked || directStatus(currentDirect) === "incoming");
  const acceptRequest = async (request: DirectConversation) => {
    const accepted = await acceptDirectRequest(request.id);
    setDirects((current) => current.map((conversation) => (conversation.id === accepted.id ? accepted : conversation)));
  };
  const declineRequest = async (request: DirectConversation) => {
    await declineDirectRequest(request.id);
    leaveRequest(request);
  };
  const directPeer = (conversation: DirectConversation): BlockedAccount => ({ ...conversation.peer });
  const directActions =
    !currentDirect || currentDirect.peer.id === account?.id ? undefined : directBlocked ? (
      <BlockedBar conversation={currentDirect} />
    ) : directStatus(currentDirect) === "incoming" ? (
      <RequestBar
        key={currentDirect.id}
        conversation={currentDirect}
        onAccept={() => acceptRequest(currentDirect)}
        onDecline={() => declineRequest(currentDirect)}
        onBlock={() => setBlockTarget(directPeer(currentDirect))}
      />
    ) : (
      <Tooltip content={`Block @${currentDirect.peer.username}`}>
        <button
          type="button"
          className="member-list-toggle"
          aria-label={`Block @${currentDirect.peer.username}`}
          onClick={() => setBlockTarget(directPeer(currentDirect))}
        >
          <Ban aria-hidden="true" />
        </button>
      </Tooltip>
    );
  const channel = directView
    ? { id: directView.conversation.id, name: directView.conversation.peer.displayName, spaceId: "", private: true }
    : detail?.channels.find((item) => item.id === view?.channelId);
  const owner = !!account && detail?.space.ownerId === account.id;
  const ownedCount = account ? spaces.filter((space) => space.ownerId === account.id).length : 0;
  const canCreateSpace =
    !!limits && ownedCount < limits.ownedSpaces && spaces.filter((space) => !space.demo).length < limits.totalSpaces;
  const canCreateChannel = !!limits && !!detail && detail.channels.length < limits.channelsPerSpace;
  const replaceDetail = (next: SpaceDetail) => {
    navigation.current.forget(next.space.id);
    setView((current) => {
      if (!current || current.detail.space.id !== next.space.id) return current;
      const retained = next.channels.some((item) => item.id === current.channelId);
      const updated = retained
        ? { ...current, detail: next }
        : { detail: next, channelId: next.channels.find((item) => item.joined !== false)?.id };
      navigation.current.remember(updated);
      return updated;
    });
    setSpaces((current) => current.map((space) => (space.id === next.space.id ? next.space : space)));
  };
  const changeChannelMembership = async (action: () => Promise<unknown>, openChannel?: string, left?: Channel) => {
    if (!detail || membershipSubmitting.current) return;
    const spaceId = detail.space.id;
    const navigationRequest = navigationRevision.current;
    membershipSubmitting.current = true;
    membershipRevision.current++;
    setMembershipPending(true);
    setMembershipError(undefined);
    try {
      await action();
      navigation.current.forget(spaceId);
      // Stop local participation after the acknowledged leave even if the
      // subsequent metadata refresh fails. Do not stop another channel's call.
      if (left && activeSpace.current === spaceId)
        replaceDetail({
          ...detail,
          channels:
            left.private && !owner
              ? detail.channels.filter((item) => item.id !== left.id)
              : detail.channels.map((item) => (item.id === left.id ? { ...item, joined: false } : item)),
        });
      const next = await getSpace(spaceId);
      if (activeSpace.current !== spaceId) return;
      replaceDetail(next);
      if (navigationRequest !== navigationRevision.current) return;
      setChannelInvitation((current) => (current === channelInvitation ? undefined : current));
      setLeavingChannel((current) => (current === leavingChannel ? undefined : current));
      if (openChannel) choose(spaceId, openChannel, true);
    } catch (reason) {
      // The membership write still completes, but its error belongs only to
      // the navigation that started it, including an away-and-back round trip.
      if (navigationRequest === navigationRevision.current) throw reason;
    } finally {
      membershipRevision.current++;
      membershipSubmitting.current = false;
      setMembershipPending(false);
    }
  };
  const forgetSpace = () => {
    const remaining = spaces.filter((space) => space.id !== detail?.space.id);
    if (detail) navigation.current.forget(detail.space.id);
    // Explicit removal must tear down the old Call immediately, even if the
    // next space is slow. It is no longer a conversation we can keep showing.
    setView(undefined);
    setSpaces(remaining);
    setDialog(undefined);
    choose(remaining[0]?.id, undefined, true, selected.dmId);
  };

  const incomingRequests = directs.filter((conversation) => directStatus(conversation) === "incoming");
  const requestsShown =
    requestsOpen ?? incomingRequests.some((conversation) => conversation.id === directView?.conversation.id);
  const leaveRequest = (request: DirectConversation) => {
    const next = incomingRequests.find((conversation) => conversation.id !== request.id);
    if (directView?.conversation.id === request.id) {
      if (next) openDirect(next);
      else choose(detail?.space.id || undefined);
    }
    setDirects((current) => current.filter((conversation) => conversation.id !== request.id));
  };
  const directNavigation = (
    <section className="direct-section" aria-label="Direct messages">
      <div className="channel-section-heading">
        <span className="direct-section-title">
          <MessageCircle aria-hidden="true" />
          Direct messages
        </span>
        <span className="channel-section-actions">
          <button
            type="button"
            aria-label="New direct message"
            title="New direct message"
            onClick={() => setDialog("direct")}
          >
            <Plus aria-hidden="true" />
          </button>
        </span>
      </div>
      <ul>
        {account && (
          <li>
            <button
              type="button"
              className="channel-select direct-select direct-self"
              disabled={selfDirectPending}
              aria-busy={selfDirectPending}
              aria-current={selfDirect && selfDirect.id === directView?.conversation.id ? "page" : undefined}
              title="Your private notes"
              onClick={() => void openSelfDirect()}
            >
              <span className="direct-avatar">
                <Avatar avatarId={account.avatarId} name={account.displayName ?? account.username ?? "You"} />
              </span>
              <span>{account.displayName ?? account.username ?? "You"}</span>
              <small>you</small>
              {selfDirect && directUnread(selfDirect) && (
                <span className="direct-unread" aria-label="Unread messages" />
              )}
            </button>
          </li>
        )}
        {incomingRequests.length > 0 && (
          <li>
            <button
              type="button"
              className="channel-select direct-select direct-requests"
              aria-expanded={requestsShown}
              aria-controls="direct-request-list"
              onClick={() => setRequestsOpen(!requestsShown)}
            >
              {requestsShown ? <ChevronDown aria-hidden="true" /> : <ChevronRight aria-hidden="true" />}
              <span>Message requests</span>
              <span
                className="direct-request-count"
                aria-label={`${incomingRequests.length} ${incomingRequests.length === 1 ? "request" : "requests"}`}
              >
                {incomingRequests.length}
              </span>
            </button>
          </li>
        )}
        {requestsShown && incomingRequests.length > 0 && (
          <li className="direct-request-group">
            <ul id="direct-request-list" aria-label="Message requests">
              {incomingRequests.map((conversation) => (
                <li key={conversation.id}>
                  <button
                    type="button"
                    className="channel-select direct-select"
                    aria-current={conversation.id === directView?.conversation.id ? "page" : undefined}
                    onClick={() => openDirect(conversation)}
                  >
                    <span className="direct-avatar">
                      <Avatar avatarId={conversation.peer.avatarId} name={conversation.peer.displayName} />
                    </span>
                    <span>{conversation.peer.displayName}</span>
                    <small>@{conversation.peer.username}</small>
                  </button>
                </li>
              ))}
            </ul>
          </li>
        )}
        {directs
          .filter((conversation) => conversation.peer.id !== account?.id && directStatus(conversation) !== "incoming")
          .map((conversation) => {
            const conversationNotifications = overrideFor(notifications, { conversationId: conversation.id });
            const muted = isMuted(conversationNotifications?.mutedUntil);
            return (
              <li key={conversation.id} data-muted={muted ? "" : undefined}>
                <div className="direct-line">
                  <button
                    type="button"
                    className="channel-select direct-select"
                    aria-current={conversation.id === directView?.conversation.id ? "page" : undefined}
                    title={`@${conversation.peer.username}`}
                    onClick={() => openDirect(conversation)}
                  >
                    <span className="direct-avatar" aria-hidden="true">
                      <Avatar avatarId={conversation.peer.avatarId} name={conversation.peer.displayName} />
                    </span>
                    <span>{conversation.peer.displayName}</span>
                    {blockedIds.has(conversation.peer.id) ? (
                      <small>Blocked</small>
                    ) : (
                      directStatus(conversation) === "outgoing" && <small>Request sent</small>
                    )}
                    {/* A muted DM shows no unread dot. */}
                    {muted ? (
                      <>
                        <BellOff className="muted-icon" aria-hidden="true" />
                        <span className="sr-only">, muted</span>
                      </>
                    ) : (
                      directUnread(conversation) && <span className="direct-unread" aria-label="Unread messages" />
                    )}
                  </button>
                  <details
                    className="channel-menu direct-menu"
                    onToggle={(event) => placeMenu(event.currentTarget)}
                    onKeyDown={(event) => {
                      if (event.key === "Escape" && event.currentTarget.open) {
                        // Handled here, so an open thread does not also close.
                        event.preventDefault();
                        event.currentTarget.open = false;
                        event.currentTarget.querySelector("summary")?.focus();
                      }
                    }}
                    onBlur={(event) => {
                      if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
                    }}
                  >
                    <summary
                      className="channel-manage"
                      aria-label={`Options for ${conversation.peer.displayName}`}
                      title={`Conversation options for ${conversation.peer.displayName}`}
                    >
                      <MoreHorizontal aria-hidden="true" />
                    </summary>
                    <div className="space-actions">
                      <DirectNotificationItems
                        loaded={notifications.loaded}
                        off={conversationNotifications?.level === "nothing"}
                        mutedUntil={conversationNotifications?.mutedUntil ?? null}
                        onChange={(change) =>
                          saveNotifications(`dm:${conversation.id}`, () =>
                            changeDirectNotifications(conversation.id, change),
                          )
                        }
                      />
                    </div>
                  </details>
                </div>
                {notificationAlert(`dm:${conversation.id}`)}
              </li>
            );
          })}
      </ul>
      <button
        type="button"
        className="channel-select direct-action"
        onClick={() => setDialog(owner ? "manage-space" : "direct")}
      >
        <Plus aria-hidden="true" />
        <span>{owner ? "Invite people" : "New message"}</span>
      </button>
      {directError && (
        <p className="space-sidebar-error" role="alert">
          {directError}
          <button type="button" onClick={() => void refreshDirects()}>
            Retry direct messages
          </button>
        </p>
      )}
      {dialog === "direct" && (
        <StartDirectDialog
          onClose={() => setDialog(undefined)}
          onCreated={(conversation) => {
            setDirects((current) => [conversation, ...current.filter((item) => item.id !== conversation.id)]);
            setDialog(undefined);
            openDirect(conversation);
          }}
        />
      )}
      {blockTarget && (
        <BlockDialog
          person={blockTarget}
          onClose={() => setBlockTarget(undefined)}
          onBlocked={() => {
            const request = directs.find(
              (conversation) => conversation.peer.id === blockTarget.id && directStatus(conversation) === "incoming",
            );
            setBlockTarget(undefined);
            // The server declines their request, so it leaves the list now.
            if (request) leaveRequest(request);
          }}
        />
      )}
    </section>
  );
  const invitationButtons = invitations.map((space) => (
    <button
      key={space.id}
      type="button"
      className="pending-space-invite"
      aria-label={`Invitation to ${space.name}`}
      onClick={() => choose(space.id)}
    >
      <LockKeyhole aria-hidden="true" />
      <span>{space.name}</span>
      <small>Invited</small>
    </button>
  ));

  // Closing an invitation re-reads the rail: it may have been withdrawn meanwhile.
  const refreshInvitations = () =>
    void listSpaces()
      .then((result) => {
        setSpaces(result.spaces);
        setInvitations(result.invitations ?? []);
      })
      .catch(() => undefined);
  const acceptedInvitation = (space: Space, replace: boolean) => {
    setInvitations((items) => items.filter((item) => item.id !== space.id));
    setSpaces((items) => [...items.filter((item) => item.id !== space.id), space]);
    navigation.current.forget(space.id);
    choose(space.id, undefined, replace);
  };

  if (loading) return <SpacesLoading />;
  if (invitation)
    return (
      <>
        <main className="call-page">
          <header className="call-header">
            <Wordmark />
          </header>
          <section className="call-room spaces-room invitation-shell" aria-hidden="true" inert>
            <div className="space-rail" />
            <ChannelSidebar>
              <div className="sidebar-channels" />
            </ChannelSidebar>
            <div className="stage" />
          </section>
        </main>
        <InvitationDialog
          key={invitation.id}
          space={invitation}
          onClose={() => {
            choose(spaces[0]?.id, undefined, true);
            refreshInvitations();
          }}
          onAccepted={(space) => acceptedInvitation(space, true)}
          onDeclined={() => {
            setInvitations((items) => items.filter((item) => item.id !== invitation.id));
            choose(spaces[0]?.id, undefined, true);
          }}
        />
      </>
    );
  if (error && !spaces.length)
    return (
      <main className="spaces-state">
        <Wordmark />
        <h1>Spaces are unavailable.</h1>
        <p role="alert">{error}</p>
        <button type="button" onClick={() => window.location.reload()}>
          Try again
        </button>
      </main>
    );
  if (!spaces.length && !selected.dmId)
    return (
      <main className="spaces-empty">
        <Wordmark />
        <section>
          {notice && <p role="status">{notice}</p>}
          {invitations.length > 0 && (
            <div className="pending-space-invites">
              <h2>Pending invitations</h2>
              {invitationButtons}
            </div>
          )}
          <h1>Name your space</h1>
          <p>Choose something you will recognize easily. You can always change it later!</p>
          <CreateSpaceForm
            disabled={!canCreateSpace}
            onCreated={(space) => {
              setSpaces([space]);
              choose(space.id);
            }}
          />
          {!canCreateSpace && <small>You have reached your space limit.</small>}
          {directNavigation}
        </section>
      </main>
    );
  if (!detail && !error) return <SpacesLoading />;
  if (!detail)
    return (
      <main className="spaces-state">
        <Wordmark />
        <p role="alert">{error}</p>
        <button type="button" onClick={() => setSelected({ ...selected })}>
          Try again
        </button>
      </main>
    );

  const joinedChannels = detail.channels.filter((item) => item.joined !== false);
  const spaceNotifications = overrideFor(notifications, { spaceId: detail.space.id });
  const spaceMuted = isMuted(spaceNotifications?.mutedUntil);
  const channelDialogs = (
    <>
      {browseOpen && (
        <BrowseChannelsDialog
          detail={detail}
          onClose={() => setBrowseOpen(false)}
          onPreview={(item) => choose(detail.space.id, item.id)}
          onManage={
            owner
              ? (item) => {
                  setBrowseOpen(false);
                  setManageChannel(item);
                }
              : undefined
          }
        />
      )}
      {channelInvitation && (
        <ChannelInvitationDialog
          key={channelInvitation.channel.id}
          invitation={channelInvitation}
          spaceName={detail.space.name}
          onClose={() => setChannelInvitation(undefined)}
          onRespond={(accept) =>
            changeChannelMembership(
              () =>
                accept
                  ? acceptChannelInvitation(detail.space.id, channelInvitation.channel.id)
                  : declineChannelInvitation(detail.space.id, channelInvitation.channel.id),
              accept ? channelInvitation.channel.id : undefined,
            )
          }
        />
      )}
      {leavingChannel && (
        <LeaveChannelDialog
          key={leavingChannel.id}
          channel={leavingChannel}
          owner={owner}
          onClose={() => setLeavingChannel(undefined)}
          onLeave={() =>
            changeChannelMembership(() => leaveChannel(detail.space.id, leavingChannel.id), undefined, leavingChannel)
          }
        />
      )}
    </>
  );

  const rail = (
    <nav className="space-rail" aria-label="Spaces">
      {spaces.map((space) => {
        const muted = isMuted(overrideFor(notifications, { spaceId: space.id })?.mutedUntil);
        return (
          <div
            className="space-rail-item"
            key={space.id}
            data-active={space.id === detail.space.id}
            data-muted={muted ? "" : undefined}
          >
            <button
              type="button"
              title={muted ? `${space.name} (muted)` : space.name}
              aria-label={muted ? `${space.name}, muted` : space.name}
              aria-current={space.id === detail.space.id ? "page" : undefined}
              aria-busy={pending && space.id === selected.spaceId}
              onMouseEnter={() => prefetch(space.id)}
              onFocus={() => prefetch(space.id)}
              onClick={() => choose(space.id)}
            >
              <span>{initial(space.name)}</span>
              {muted && <BellOff className="space-rail-muted" aria-hidden="true" />}
            </button>
          </div>
        );
      })}
      {invitations.map((space) => (
        <button
          key={space.id}
          className="invited-space"
          type="button"
          title={`Invitation to ${space.name}`}
          aria-label={`Invitation to ${space.name}`}
          onClick={() => (detail ? setRailInvitation(space) : choose(space.id))}
        >
          {/* The space's initial in a dashed tile, as on Apple, Android and desktop. */}
          <span aria-hidden="true">{initial(space.name)}</span>
        </button>
      ))}
      <button
        className="add-space"
        type="button"
        title={
          !account
            ? "Sign in to create a space"
            : canCreateSpace
              ? "Create space"
              : `Space limit reached (${limits?.ownedSpaces ?? 20} owned, ${limits?.totalSpaces ?? 100} total)`
        }
        aria-label="Create space"
        disabled={!!account && !canCreateSpace}
        onClick={() => (account ? setDialog("space") : window.location.assign("/login"))}
      >
        <Plus aria-hidden="true" />
      </button>
      {pending && (
        <span className="sr-only" role="status">
          Opening {spaces.find((space) => space.id === selected.spaceId)?.name}…
        </span>
      )}
    </nav>
  );
  // Outside the rail: phones hide the rail, and its tile styles must not apply.
  const accessNotice = notice && (
    <p className="space-access-notice" role="status">
      {notice}
      <button type="button" aria-label="Dismiss notice" onClick={() => setNotice(undefined)}>
        <X aria-hidden="true" />
      </button>
    </p>
  );
  const channelNavigation = (voiceFor: (channelId: string) => VoiceSlot | null) => (
    <nav
      ref={channelNavigationRef}
      className="channel-navigation"
      data-demo={detail.space.demo ? "" : undefined}
      aria-label={`${detail.space.name} channels`}
    >
      {(!detail.space.demo || navigationOpen) && (
        <header>
          {!detail.space.id && <h1 className="demo-space-title">Conversations</h1>}
          {!!detail.space.id && !detail.space.demo && (
            <details
              ref={spaceMenu}
              className="space-menu"
              onKeyDown={(event) => {
                if (event.key === "Escape" && event.currentTarget.open) {
                  // Handled here, so an open thread does not also close.
                  event.preventDefault();
                  event.currentTarget.open = false;
                  event.currentTarget.querySelector("summary")?.focus();
                }
              }}
              onBlur={(event) => {
                if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
              }}
            >
              <summary aria-label={`${detail.space.name} actions${spaceMuted ? ", muted" : ""}`}>
                <h1 title={detail.space.name}>{detail.space.name}</h1>
                {spaceMuted && (
                  <span className="space-muted" title="Muted">
                    <BellOff aria-hidden="true" />
                  </span>
                )}
                <ChevronDown aria-hidden="true" />
              </summary>
              <div className="space-actions">
                <button
                  type="button"
                  onClick={() => {
                    closeMenu(spaceMenu.current!);
                    setBrowseOpen(true);
                  }}
                >
                  <Search aria-hidden="true" />
                  Browse channels
                </button>
                <LevelNotificationItems
                  noun="space"
                  loaded={notifications.loaded}
                  level={spaceNotifications?.level ?? null}
                  inherited={inheritedLevel(notifications, { spaceId: detail.space.id })}
                  mutedUntil={spaceNotifications?.mutedUntil ?? null}
                  onChange={(change) => {
                    const spaceId = detail.space.id;
                    saveNotifications(`space:${spaceId}`, () => changeSpaceNotifications(spaceId, change));
                  }}
                />
                {owner ? (
                  <>
                    <button
                      type="button"
                      onClick={() => {
                        closeMenu(spaceMenu.current!);
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
                      closeMenu(spaceMenu.current!);
                      setDialog("leave-space");
                    }}
                  >
                    <LogOut aria-hidden="true" />
                    Leave space…
                  </button>
                )}
              </div>
            </details>
          )}
          {navigationOpen && (
            <button type="button" aria-label="Close navigation" onClick={() => setNavigationOpen(false)}>
              <X aria-hidden="true" />
            </button>
          )}
        </header>
      )}
      {notificationAlert(`space:${detail.space.id}`)}
      {!!detail.space.id && !detail.space.demo && (
        <div className="channel-section-heading">
          <button
            className="channel-section-toggle"
            type="button"
            aria-expanded={channelsExpanded}
            aria-controls="space-channel-list"
            onClick={() => setChannelsExpanded(!channelsExpanded)}
          >
            <ChevronDown aria-hidden="true" />
            Channels<span className="section-count">{joinedChannels.length}</span>
          </button>
          {owner && (
            <div className="channel-section-actions">
              <button
                type="button"
                aria-label="Create channel"
                title={
                  canCreateChannel ? "Create channel" : `Channel limit reached (${limits?.channelsPerSpace ?? 100})`
                }
                disabled={!canCreateChannel}
                onClick={() => setDialog("channel")}
              >
                <Plus aria-hidden="true" />
              </button>
              <details
                ref={channelMenu}
                className="channel-section-menu"
                onKeyDown={(event) => {
                  if (event.key === "Escape" && event.currentTarget.open) {
                    // Handled here, so an open thread does not also close.
                    event.preventDefault();
                    event.currentTarget.open = false;
                    event.currentTarget.querySelector("summary")?.focus();
                  }
                }}
                onBlur={(event) => {
                  if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
                }}
              >
                <summary aria-label="Channel options" title="Channel options">
                  <MoreHorizontal aria-hidden="true" />
                </summary>
                <div className="space-actions">
                  <button
                    type="button"
                    disabled={!canCreateChannel}
                    onClick={() => {
                      closeMenu(channelMenu.current!);
                      setDialog("channel");
                    }}
                  >
                    <Plus aria-hidden="true" />
                    Create channel
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      closeMenu(channelMenu.current!);
                      setChannelsExpanded((value) => !value);
                    }}
                  >
                    {channelsExpanded ? "Collapse" : "Expand"} channels
                  </button>
                </div>
              </details>
            </div>
          )}
        </div>
      )}
      <ul id="space-channel-list" data-collapsed={detail.space.demo || channelsExpanded ? undefined : ""}>
        {joinedChannels.map((item) => {
          const voice = voiceFor(item.id);
          const itemNotifications = overrideFor(notifications, { spaceId: detail.space.id, channelId: item.id });
          const itemMuted = isMuted(itemNotifications?.mutedUntil);
          return (
            <li key={item.id} data-voice={voice ? "" : undefined} data-muted={itemMuted || spaceMuted ? "" : undefined}>
              <div className="channel-line">
                <button
                  className="channel-select"
                  type="button"
                  aria-current={item.id === channel?.id ? "page" : undefined}
                  aria-busy={pending && detail.space.id === selected.spaceId && item.id === selected.channelId}
                  onMouseEnter={() => prefetch(detail.space.id, item.id)}
                  onFocus={() => prefetch(detail.space.id, item.id)}
                  onClick={() => choose(detail.space.id, item.id)}
                >
                  {item.private ? <LockKeyhole aria-hidden="true" /> : <Hash aria-hidden="true" />}
                  <span>{item.name}</span>
                  {itemMuted && (
                    <>
                      <BellOff className="muted-icon" aria-hidden="true" />
                      <span className="sr-only">, muted</span>
                    </>
                  )}
                  {voice?.timer}
                </button>
                {!detail.space.demo && (
                  <details
                    className="channel-menu"
                    onToggle={(event) => placeMenu(event.currentTarget)}
                    onKeyDown={(event) => {
                      if (event.key === "Escape" && event.currentTarget.open) {
                        // Handled here, so an open thread does not also close.
                        event.preventDefault();
                        event.currentTarget.open = false;
                        event.currentTarget.querySelector("summary")?.focus();
                      }
                    }}
                    onBlur={(event) => {
                      if (!event.currentTarget.contains(event.relatedTarget)) event.currentTarget.open = false;
                    }}
                  >
                    <summary
                      className="channel-manage"
                      aria-label={`Manage ${item.name}`}
                      title={`Channel actions for ${item.name}`}
                    >
                      <MoreHorizontal aria-hidden="true" />
                    </summary>
                    <div className="space-actions">
                      <LevelNotificationItems
                        noun="channel"
                        loaded={notifications.loaded}
                        level={itemNotifications?.level ?? null}
                        inherited={inheritedLevel(notifications, { spaceId: detail.space.id, channelId: item.id })}
                        mutedUntil={itemNotifications?.mutedUntil ?? null}
                        mutedWithSpace={spaceMuted}
                        onChange={(change) => {
                          const spaceId = detail.space.id;
                          saveNotifications(`channel:${item.id}`, () =>
                            changeChannelNotifications(spaceId, item.id, change),
                          );
                        }}
                      />
                      {owner && (
                        <button
                          type="button"
                          onClick={(event) => {
                            const menu = event.currentTarget.closest("details")!;
                            menu.open = false;
                            menu.querySelector("summary")?.focus();
                            setManageChannel(item);
                          }}
                        >
                          <Settings aria-hidden="true" />
                          Channel settings
                        </button>
                      )}
                      <button
                        type="button"
                        disabled={membershipPending}
                        onClick={(event) => {
                          const menu = event.currentTarget.closest("details")!;
                          menu.open = false;
                          menu.querySelector("summary")?.focus();
                          setLeavingChannel(item);
                        }}
                      >
                        <LogOut aria-hidden="true" />
                        Leave channel
                      </button>
                    </div>
                  </details>
                )}
                {voice?.summary}
              </div>
              {voice?.list}
              {notificationAlert(`channel:${item.id}`)}
            </li>
          );
        })}
      </ul>
      {!!detail.channelInvitations?.length && (
        <div className="pending-channel-invites">
          <h2>Invitations</h2>
          {detail.channelInvitations.map((item) => (
            <button
              className="pending-channel-invite"
              key={item.channel.id}
              type="button"
              onClick={() => setChannelInvitation(item)}
            >
              <LockKeyhole aria-hidden="true" />
              <span>{item.channel.name}</span>
              <small>Invited</small>
            </button>
          ))}
        </div>
      )}
      {directNavigation}
      {error && (
        <p className="space-sidebar-error" role="alert">
          {error}
          <button type="button" onClick={() => setSelected({ ...selected })}>
            Retry opening
          </button>
        </p>
      )}
    </nav>
  );

  // A space with no joined channel replaces the conversation inside the room
  // instead of rendering its own page: unmounting the room would end a call
  // running in another space or channel.
  const emptyStage = !channel && (
    <div className="empty-channel">
      <button
        className="navigation-toggle"
        type="button"
        aria-label="Back to Browse"
        aria-expanded={navigationOpen}
        onClick={() => setNavigationOpen((open) => !open)}
      >
        <ArrowLeft aria-hidden="true" />
      </button>
      <Hash aria-hidden="true" />
      <h2>{spaces.length ? "No joined channels" : "Start a conversation"}</h2>
      <p>
        {!spaces.length
          ? "Create a space for your people, or start a direct message."
          : owner
            ? "Browse channels or create one to start a conversation."
            : "Browse channels to find a conversation, or accept a private channel invitation."}
      </p>
      {/* One primary action; the rest are secondary. */}
      <div className="empty-channel-actions">
        {spaces.length > 0 && (
          <button type="button" className={owner ? "secondary" : "primary"} onClick={() => setBrowseOpen(true)}>
            Browse channels
          </button>
        )}
        {owner && (
          <button type="button" className="primary" onClick={() => setDialog("channel")}>
            Create channel
          </button>
        )}
        {!spaces.length && (
          <button type="button" className="primary" onClick={() => setDialog("space")}>
            Create your first space
          </button>
        )}
        <button type="button" className="secondary" onClick={() => setDialog("direct")}>
          New direct message
        </button>
      </div>
    </div>
  );

  return (
    <>
      <Call
        embedded={embedded}
        engaged={engaged}
        onChatOnlineChange={onChatOnlineChange}
        channel={
          channel && {
            id: channel.id,
            name: channel.name,
            spaceName: detail.space.name,
            spaceId: detail.space.id,
            demo: detail.space.demo,
            direct: !!directView,
            directPeerId: currentDirect?.peer.id,
            // An open request or a blocked DM replaces the composer.
            joined: currentDirect ? !directLocked : channel.joined,
          }
        }
        stage={emptyStage || undefined}
        space={{ id: detail.space.id, name: detail.space.name, demo: detail.space.demo }}
        voiceChannels={joinedChannels.map((item) => ({ id: item.id, name: item.name }))}
        channelActions={
          currentDirect ? (
            directActions
          ) : channel?.joined === false ? (
            <div className="channel-preview">
              <div>
                <strong>Preview</strong>
                <span>
                  Join <strong>#{channel.name}</strong> to interact with people here
                </span>
              </div>
              <button
                type="button"
                className="primary"
                disabled={membershipPending}
                onClick={() =>
                  void changeChannelMembership(() => joinChannel(detail.space.id, channel.id)).catch((reason) =>
                    setMembershipError(errorMessage(reason)),
                  )
                }
              >
                {membershipPending ? "Joining…" : "Join channel"}
              </button>
              {membershipError && <p role="alert">{membershipError}</p>}
            </div>
          ) : undefined
        }
        composerBanner={
          currentDirect && directStatus(currentDirect) === "outgoing" && !directLocked ? (
            <p className="direct-waiting" role="status">
              Waiting for @{currentDirect.peer.username} to accept. They’ll see your messages when they do.
            </p>
          ) : undefined
        }
        onBlockAuthor={(author) =>
          setBlockTarget({
            id: author.id,
            username: "",
            displayName: author.name,
            avatarId: author.avatarId ?? undefined,
          })
        }
        initialAccount={account}
        initialHistory={!directView && channel && view?.history?.channel.id === channel.id ? view.history : undefined}
        initialHistoryError={!directView && channel && view?.channelId === channel.id ? view.historyError : undefined}
        onReadCursor={directView ? readDirect : undefined}
        mentionMembers={
          directView ? (people ?? [directView.conversation.peer]) : view?.detail ? detail.members : undefined
        }
        mentionDirectory={[
          ...(directView ? [] : detail.members),
          ...(people ?? []),
          ...directs.map((conversation) => conversation.peer),
        ]}
        onMessagePerson={async (username) => {
          const conversation = await createDirectConversation(username);
          setDirects((current) => [conversation, ...current.filter((item) => item.id !== conversation.id)]);
          openDirect(conversation);
        }}
        onHistoryChange={navigation.current.rememberHistory}
        spaceRail={rail}
        channelNavigation={channelNavigation}
        membersPanel={
          directView || !channel || channel.joined === false
            ? undefined
            : (onClose) => (
                <MemberPresence
                  spaceId={detail.space.id}
                  members={detail.members}
                  demo={detail.space.demo}
                  onClose={onClose}
                />
              )
        }
        navigationOpen={navigationOpen}
        onNavigationToggle={() => setNavigationOpen((open) => !open)}
        onNavigationChange={changeNavigation}
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
              channels: detail.channels.map((item) => (item.id === updated.id ? updated : item)),
            });
            setManageChannel(updated);
          }}
          onDeleted={() => {
            const remaining = detail.channels.filter((item) => item.id !== manageChannel.id);
            replaceDetail({ ...detail, channels: remaining });
            setManageChannel(undefined);
            // Deleting another channel keeps you where you are.
            if (manageChannel.id === channel?.id)
              choose(detail.space.id, remaining.find((item) => item.joined !== false)?.id, true);
          }}
        />
      )}
      {channelDialogs}
      {accessNotice}
      {railInvitation && (
        <InvitationDialog
          key={railInvitation.id}
          space={railInvitation}
          onClose={() => {
            setRailInvitation(undefined);
            refreshInvitations();
          }}
          onAccepted={(space) => {
            setRailInvitation(undefined);
            acceptedInvitation(space, false);
          }}
          onDeclined={() => {
            setInvitations((items) => items.filter((item) => item.id !== railInvitation.id));
            setRailInvitation(undefined);
          }}
        />
      )}
    </>
  );
}
