import { ImageUp, Pencil, Trash2 } from "lucide-react";
import { useEffect, useId, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import Avatar from "../components/Avatar";
import SpaceIcon from "../components/SpaceIcon";
import { removeSpaceIcon, SpacesApiError, type Space } from "../spaces/client";
import { avatarUrl } from "./avatar";
import { AccountApiError, getRememberedAccount, removeAvatar, subscribeAccount, type Account } from "./client";
import { PictureError, uploadAvatar, uploadSpaceIcon } from "./picture-upload";
import { pictureErrorMessage, pictureMessages } from "./pictures";
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
  removeFailed,
  onUpload,
  onRemove,
}: {
  title: string;
  preview: ReactNode;
  /** "picture" or "icon", for the menu and labels. */
  noun: string;
  removable: boolean;
  removeFailed: (error: unknown) => string;
  onUpload: (file: File, progress: (fraction: number) => void) => Promise<void>;
  onRemove: () => Promise<void>;
}) {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [error, setError] = useState<string>();
  const [done, setDone] = useState<string>();
  const [menu, setMenu] = useState(false);
  // Uploads are optional server configuration (503): the picker steps back once the server says so.
  const [unavailable, setUnavailable] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const frame = useRef<HTMLDivElement>(null);
  const menuId = useId();
  const busy = phase.kind !== "idle";
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
  const offersMenu = removable;

  // A click or tap anywhere else closes the menu.
  useEffect(() => {
    if (!menu) return;
    const outside = (event: PointerEvent) => {
      if (!frame.current?.contains(event.target as Node)) setMenu(false);
    };
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [menu]);

  const choose = async (file: File) => {
    refocus.current = true;
    setError(undefined);
    setDone(undefined);
    setPhase({ kind: "preparing" });
    try {
      await onUpload(file, (fraction) => setPhase({ kind: "uploading", progress: fraction }));
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
            <Pencil size={15} strokeWidth={2.4} />
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
            <button
              type="button"
              role="menuitem"
              autoFocus
              disabled={unavailable}
              onClick={() => {
                setMenu(false);
                input.current?.click();
              }}
            >
              <ImageUp size={16} aria-hidden="true" />
              Choose a new {noun}
            </button>
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
          </div>
        )}
      </div>
      <input
        ref={input}
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
  return (
    <PictureEditor
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
      removeFailed={(error) => removalError(error, "avatar", "Your picture couldn’t be removed. Try again.")}
      onUpload={async (file, progress) => {
        await uploadAvatar(file, progress);
      }}
      onRemove={async () => {
        await removeAvatar();
      }}
    />
  );
}

/** Manage space (owners only): the icon or initial tile in its frame. */
export function SpaceIconSettings({ space, onChanged }: { space: Space; onChanged: (space: Space) => void }) {
  return (
    <PictureEditor
      title="Space icon"
      noun="icon"
      preview={
        <span className="picture-settings-tile">
          <SpaceIcon space={space} />
        </span>
      }
      removable={!!space.iconAsset}
      removeFailed={(error) => removalError(error, "spaceIcon", "The icon couldn’t be removed. Try again.")}
      onUpload={async (file, progress) => onChanged(await uploadSpaceIcon(space.id, file, progress))}
      onRemove={async () => onChanged(await removeSpaceIcon(space.id))}
    />
  );
}
