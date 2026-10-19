import { Images, ImageUp, Pencil, Trash2 } from "lucide-react";
import { useCallback, useEffect, useId, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import AttachmentEditor from "../chat/AttachmentEditor";
import type { AttachmentEdit } from "../chat/edit";
import { pngIsAnimated, webpInfo } from "../chat/prepare";
import "../chat/chat.css";
import Avatar from "../components/Avatar";
import SpaceIcon from "../components/SpaceIcon";
import { removeSpaceIcon, SpacesApiError, type Space } from "../spaces/client";
import { avatarUrl } from "./avatar";
import { AccountApiError, getRememberedAccount, removeAvatar, subscribeAccount, type Account } from "./client";
import AvatarHistory from "./AvatarHistory";
import { PictureError, uploadAvatar, uploadSpaceIcon } from "./picture-upload";
import { gifSize, pictureErrorMessage, pictureMessages } from "./pictures";
import "./pictures.css";

type Phase = { kind: "idle" } | { kind: "preparing" } | { kind: "uploading"; progress: number } | { kind: "removing" };

function uploadError(error: unknown) {
  if (error instanceof PictureError) return error.message;
  return pictureMessages.failed;
}

/** The current picture in a frame with an edit badge on it. Without a custom picture the frame opens the
 * system picker (GIFs allowed); with one it offers a new picture or removal. Progress and errors show beneath. */
function PictureEditor({
  title,
  preview,
  noun,
  removable,
  currentUrl,
  removeFailed,
  onUpload,
  onRemove,
  onHistory,
}: {
  title: string;
  preview: ReactNode;
  /** "picture" or "icon", for the menu and labels. */
  noun: string;
  removable: boolean;
  currentUrl?: string | null;
  removeFailed: (error: unknown) => string;
  onUpload: (file: Blob, progress: (fraction: number) => void, edit?: AttachmentEdit) => Promise<void>;
  onRemove: () => Promise<void>;
  onHistory?: () => void;
}) {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [error, setError] = useState<string>();
  const [done, setDone] = useState<string>();
  const [menu, setMenu] = useState(false);
  const [dropping, setDropping] = useState(false);
  const [draft, setDraft] = useState<{ file: Blob; animation: boolean; gif: boolean }>();
  const active = useRef(true);
  useEffect(() => {
    active.current = true;
    return () => {
      active.current = false;
    };
  }, []);
  // Uploads are optional server configuration (503): the picker steps back once the server says so.
  const [unavailable, setUnavailable] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const busy = phase.kind !== "idle" || !!draft;
  // The edit button is disabled while a request runs and the menu item that
  // started it is gone, so focus falls to the page. Keyboard users continue
  // from the edit button instead of the top of the page or dialog.
  const refocus = useRef(false);
  useEffect(() => {
    if (busy || !refocus.current) return;
    refocus.current = false;
    if (!document.activeElement || document.activeElement === document.body)
      frame.current?.querySelector<HTMLButtonElement>(".picture-settings-edit")?.focus();
  }, [busy]);
  const offersMenu = removable || !!onHistory;

  // A click or tap anywhere else closes the menu.
  useEffect(() => {
    if (!menu) return;
    const outside = (event: PointerEvent) => {
      if (!frame.current?.contains(event.target as Node)) setMenu(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [menu]);

  const cancelEdit = useCallback(() => setDraft(undefined), []);
  const choose = async (source: Blob | string) => {
    if (busy || unavailable) return;
    setMenu(false);
    refocus.current = true;
    setError(undefined);
    setDone(undefined);
    setPhase({ kind: "preparing" });
    try {
      let file: Blob;
      if (typeof source === "string") {
        const response = await fetch(source, {
          credentials: "omit",
          referrerPolicy: "no-referrer",
          signal: AbortSignal.timeout(15_000),
        });
        if (!response.ok) throw new PictureError("Couldn’t open this picture. Choose the original file again.");
        file = await response.blob();
      } else file = source;
      if (file.size > 64 * 1024 * 1024) throw new PictureError(pictureMessages.tooLarge);
      const head = new Uint8Array(await file.slice(0, 65536).arrayBuffer());
      const gif = !!gifSize(head);
      if (active.current) setDraft({ file, gif, animation: gif || pngIsAnimated(head) || !!webpInfo(head)?.animated });
    } catch (reason) {
      if (active.current) setError(uploadError(reason));
    } finally {
      if (active.current) setPhase({ kind: "idle" });
    }
  };
  const upload = async (file: Blob, edit?: AttachmentEdit) => {
    setDraft(undefined);
    setPhase({ kind: "preparing" });
    try {
      await onUpload(file, (fraction) => setPhase({ kind: "uploading", progress: fraction }), edit);
      setDone(`${title} updated.`);
    } catch (reason) {
      if (reason instanceof PictureError && reason.status === 503) setUnavailable(true);
      else setError(uploadError(reason));
    } finally {
      setPhase({ kind: "idle" });
    }
  };

  const remove = async () => {
    refocus.current = true;
    setError(undefined);
    setDone(undefined);
    setPhase({ kind: "removing" });
    try {
      await onRemove();
      setDone(`${title} removed.`);
    } catch (reason) {
      setError(removeFailed(reason));
    } finally {
      setPhase({ kind: "idle" });
    }
  };

  const status =
    phase.kind === "preparing"
      ? "Preparing…"
      : phase.kind === "uploading"
        ? `Uploading… ${Math.round(phase.progress * 100)}%`
        : phase.kind === "removing"
          ? "Removing…"
          : unavailable
            ? pictureMessages.unavailable
            : done;
  // With uploads unavailable, the frame still offers removal of a custom picture.
  const disabled = busy || (unavailable && !offersMenu);

  return (
    <section className="picture-settings" aria-label={title} aria-busy={busy}>
      <div
        ref={frame}
        className="picture-settings-frame"
        onKeyDown={(event) => {
          // The menu's Escape closes the menu, not the dialog around it.
          if (event.key !== "Escape" || !menu) return;
          event.preventDefault();
          setMenu(false);
          frame.current?.querySelector<HTMLButtonElement>(".picture-settings-edit")?.focus();
        }}
      >
        <button
          type="button"
          className="picture-settings-edit"
          data-dropping={dropping || undefined}
          onDragOver={(event) => {
            if (!event.dataTransfer.types.includes("Files")) return;
            event.preventDefault();
            event.stopPropagation();
            event.dataTransfer.dropEffect = busy || unavailable ? "none" : "copy";
            setDropping(!busy && !unavailable);
          }}
          onDragLeave={(event) => {
            if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDropping(false);
          }}
          onDrop={(event) => {
            event.preventDefault();
            event.stopPropagation();
            setDropping(false);
            const file = event.dataTransfer.files[0];
            if (file) void choose(file);
          }}
          aria-label={`Change ${title.toLowerCase()}`}
          title={`Change ${title.toLowerCase()}`}
          disabled={disabled}
          aria-haspopup={offersMenu ? "menu" : undefined}
          aria-expanded={offersMenu ? menu : undefined}
          aria-controls={offersMenu && menu ? menuId : undefined}
          onClick={() => {
            if (offersMenu) setMenu((open) => !open);
            else input.current?.click();
          }}
        >
          <span className="picture-settings-preview">{preview}</span>
          <span className="picture-settings-badge" aria-hidden="true">
            {dropping ? <ImageUp size={15} /> : <Pencil size={15} strokeWidth={2.4} />}
          </span>
        </button>
        {menu && (
          <div
            id={menuId}
            className="picture-settings-menu"
            role="menu"
            aria-label={title}
            onKeyDown={(event) => {
              if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
              event.preventDefault();
              const items = [...event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
              const at = items.indexOf(document.activeElement as HTMLButtonElement);
              const step = event.key === "ArrowDown" ? 1 : -1;
              items[(at + step + items.length) % items.length]?.focus();
            }}
          >
            {removable && (
              <button
                type="button"
                role="menuitem"
                autoFocus
                disabled={unavailable || !currentUrl}
                onClick={() => {
                  if (currentUrl) void choose(currentUrl);
                }}
              >
                <Pencil size={16} aria-hidden="true" />
                Edit {noun}
              </button>
            )}
            <button
              type="button"
              role="menuitem"
              autoFocus={!removable}
              disabled={unavailable}
              onClick={() => {
                setMenu(false);
                input.current?.click();
              }}
            >
              <ImageUp size={16} aria-hidden="true" />
              Choose a new {noun}
            </button>
            {onHistory && (
              <button
                type="button"
                role="menuitem"
                onClick={() => {
                  setMenu(false);
                  setDone(undefined);
                  setError(undefined);
                  frame.current?.querySelector<HTMLButtonElement>(".picture-settings-edit")?.focus();
                  onHistory();
                }}
              >
                <Images size={16} aria-hidden="true" />
                Previous pictures
              </button>
            )}
            {removable && (
              <button
                type="button"
                role="menuitem"
                className="picture-settings-remove"
                onClick={() => {
                  setMenu(false);
                  void remove();
                }}
              >
                <Trash2 size={16} aria-hidden="true" />
                Remove {noun}
              </button>
            )}
          </div>
        )}
      </div>
      <input
        ref={(element) => {
          input.current = element;
          // Canceling the native picker must not dismiss any enclosing dialogs.
          const cancel = (event: Event) => event.stopPropagation();
          element?.addEventListener("cancel", cancel);
          return () => element?.removeEventListener("cancel", cancel);
        }}
        type="file"
        accept="image/*"
        hidden
        tabIndex={-1}
        onChange={(event) => {
          const file = event.currentTarget.files?.[0];
          // Clear it, so choosing the same file again still counts as a change.
          event.currentTarget.value = "";
          if (file) void choose(file);
        }}
      />
      {draft?.animation ? (
        <div className="picture-settings-animation" role="group" aria-label="Animated picture">
          <p>
            Editing saves a still frame and removes animation.
            {!draft.gif && " To keep animation, choose a GIF instead."}
          </p>
          {draft.gif && (
            <button type="button" onClick={() => void upload(draft.file)}>
              Keep original animation
            </button>
          )}
          <button type="button" onClick={() => setDraft({ ...draft, animation: false })}>
            Edit still frame
          </button>
          <button type="button" onClick={cancelEdit}>
            Cancel
          </button>
        </div>
      ) : (
        draft && (
          <AttachmentEditor
            file={draft.file}
            kind="image"
            name={title}
            tool="crop"
            pictureMode
            anchor={frame.current?.querySelector<HTMLButtonElement>(".picture-settings-edit") ?? undefined}
            onCancel={cancelEdit}
            onSave={(edit) => void upload(draft.file, edit)}
          />
        )
      )}
      {error ? (
        <p className="picture-settings-error" role="alert">
          {error}
        </p>
      ) : (
        <p className="picture-settings-status" role="status">
          {status}
        </p>
      )}
    </section>
  );
}

/** A mascot holding up a "change me!" sign: one picture, drawn in the mascot's 256 × 256 space. */
function ChangeMe({ avatarId }: { avatarId?: number | null }) {
  const mascot = avatarUrl(avatarId);
  if (!mascot) return null;
  return (
    <svg className="picture-settings-change-me" viewBox="0 0 256 256" aria-hidden="true" data-avatar-id={avatarId}>
      <image href={mascot} width="256" height="256" />
      <image href="/images/avatars/change-me.svg" width="256" height="256" />
    </svg>
  );
}

function removalError(error: unknown, purpose: "avatar" | "spaceIcon", fallback: string) {
  const status = error instanceof AccountApiError || error instanceof SpacesApiError ? error.status : undefined;
  if (status === 401 || status === 404 || status === 503) return pictureErrorMessage(status, undefined, purpose);
  // fetch() rejects with a TypeError when the network or server can't be reached.
  if (error instanceof TypeError) return "Couldn’t reach Caper. Check your connection.";
  return fallback;
}

/** Profile settings: your picture in its frame; without one, your mascot asks to be changed. */
export function ProfilePictureSettings({ account: initial }: { account: Account }) {
  const remembered = useSyncExternalStore(subscribeAccount, getRememberedAccount, () => undefined);
  const account = remembered?.id === initial.id ? remembered : initial;
  const name = account.displayName ?? account.username ?? "You";
  const [history, setHistory] = useState<Account>();
  const closeHistory = useCallback(() => setHistory(undefined), []);
  return (
    <>
      <PictureEditor
        key={account.id}
        title="Profile picture"
        noun="picture"
        preview={
          account.avatarAsset || !avatarUrl(account.avatarId) ? (
            <Avatar person={account} name={name} />
          ) : (
            <ChangeMe avatarId={account.avatarId} />
          )
        }
        removable={!!account.avatarAsset}
        currentUrl={account.avatarUrl}
        removeFailed={(error) => removalError(error, "avatar", "Your picture couldn’t be removed. Try again.")}
        onUpload={async (file, progress, edit) => {
          await uploadAvatar(file, progress, undefined, edit);
        }}
        onRemove={async () => {
          await removeAvatar();
        }}
        onHistory={() => setHistory(account)}
      />
      {history?.id === account.id && <AvatarHistory account={history} onClose={closeHistory} />}
    </>
  );
}

/** Manage space (owners only): the icon or initial tile in its frame. */
export function SpaceIconSettings({ space, onChanged }: { space: Space; onChanged: (space: Space) => void }) {
  return (
    <PictureEditor
      key={space.id}
      title="Space icon"
      noun="icon"
      preview={
        <span className="picture-settings-tile">
          <SpaceIcon space={space} />
        </span>
      }
      removable={!!space.iconAsset}
      currentUrl={space.iconUrl}
      removeFailed={(error) => removalError(error, "spaceIcon", "The icon couldn’t be removed. Try again.")}
      onUpload={async (file, progress, edit) =>
        onChanged(await uploadSpaceIcon(space.id, file, progress, undefined, edit))
      }
      onRemove={async () => onChanged(await removeSpaceIcon(space.id))}
    />
  );
}
