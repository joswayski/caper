import { useId, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import Avatar from "../components/Avatar";
import SpaceIcon from "../components/SpaceIcon";
import { removeSpaceIcon, SpacesApiError, type Space } from "../spaces/client";
import { AccountApiError, getRememberedAccount, removeAvatar, subscribeAccount, type Account } from "./client";
import { PictureError, uploadAvatar, uploadSpaceIcon } from "./picture-upload";
import { pictureErrorMessage, pictureMessages } from "./pictures";
import "./pictures.css";

type Phase = { kind: "idle" } | { kind: "preparing" } | { kind: "uploading"; progress: number } | { kind: "removing" };

function uploadError(error: unknown) {
  if (error instanceof PictureError) return error.message;
  return pictureMessages.failed;
}

/** The large current picture, Change (system picker, GIFs allowed) and Remove, with progress and inline errors. */
function PictureEditor({
  title,
  hint,
  preview,
  changeLabel,
  removeLabel,
  removable,
  removeFailed,
  onUpload,
  onRemove,
}: {
  title: string;
  hint: string;
  preview: ReactNode;
  changeLabel: string;
  removeLabel: string;
  removable: boolean;
  removeFailed: (error: unknown) => string;
  onUpload: (file: File, progress: (fraction: number) => void) => Promise<void>;
  onRemove: () => Promise<void>;
}) {
  const [phase, setPhase] = useState<Phase>({ kind: "idle" });
  const [error, setError] = useState<string>();
  const [done, setDone] = useState<string>();
  // Uploads are optional server configuration (503): the picker steps back once the server says so.
  const [unavailable, setUnavailable] = useState(false);
  const input = useRef<HTMLInputElement>(null);
  const titleId = useId();
  const busy = phase.kind !== "idle";

  const choose = async (file: File) => {
    setError(undefined);
    setDone(undefined);
    setPhase({ kind: "preparing" });
    try {
      await onUpload(file, (fraction) => setPhase({ kind: "uploading", progress: fraction }));
      setDone(`${title} updated.`);
    } catch (reason) {
      // The hint then says uploads are unavailable; the error would only repeat it.
      if (reason instanceof PictureError && reason.status === 503) setUnavailable(true);
      else setError(uploadError(reason));
    } finally {
      setPhase({ kind: "idle" });
    }
  };

  const remove = async () => {
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
          : done;

  return (
    <section className="picture-settings" aria-labelledby={titleId} aria-busy={busy}>
      <span className="picture-settings-preview">{preview}</span>
      <div className="picture-settings-body">
        <h3 id={titleId}>{title}</h3>
        <p>{unavailable ? pictureMessages.unavailable : hint}</p>
        <div className="picture-settings-actions">
          <button
            type="button"
            disabled={busy || unavailable}
            aria-busy={phase.kind === "preparing" || phase.kind === "uploading"}
            onClick={() => input.current?.click()}
          >
            {phase.kind === "preparing" || phase.kind === "uploading" ? "Uploading…" : changeLabel}
          </button>
          {removable && (
            <button type="button" disabled={busy} aria-busy={phase.kind === "removing"} onClick={() => void remove()}>
              {phase.kind === "removing" ? "Removing…" : removeLabel}
            </button>
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
        <p className="picture-settings-status" role="status">
          {status}
        </p>
      </div>
      {error && (
        <p className="picture-settings-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}

function removalError(error: unknown, purpose: "avatar" | "spaceIcon", fallback: string) {
  const status = error instanceof AccountApiError || error instanceof SpacesApiError ? error.status : undefined;
  if (status === 401 || status === 404 || status === 503) return pictureErrorMessage(status, undefined, purpose);
  // fetch() rejects with a TypeError when the network or server can't be reached.
  if (error instanceof TypeError) return "Couldn’t reach Caper. Check your connection.";
  return fallback;
}

/** Profile settings: your picture, shown large, with Change and Remove. */
export function ProfilePictureSettings({ account: initial }: { account: Account }) {
  const remembered = useSyncExternalStore(subscribeAccount, getRememberedAccount, () => undefined);
  const account = remembered?.id === initial.id ? remembered : initial;
  const name = account.displayName ?? account.username ?? "You";
  return (
    <PictureEditor
      title="Profile picture"
      hint="PNG, JPEG, WebP or GIF. GIFs animate for everyone."
      preview={<Avatar person={account} name={name} />}
      changeLabel="Change picture"
      removeLabel="Remove picture"
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

/** Manage space (owners only): the icon or initial tile, with Change and Remove. */
export function SpaceIconSettings({ space, onChanged }: { space: Space; onChanged: (space: Space) => void }) {
  return (
    <PictureEditor
      title="Space icon"
      hint="PNG, JPEG, WebP or GIF. Without one, the space shows its initial."
      preview={
        <span className="picture-settings-tile">
          <SpaceIcon space={space} />
        </span>
      }
      changeLabel="Change icon"
      removeLabel="Remove icon"
      removable={!!space.iconAsset}
      removeFailed={(error) => removalError(error, "spaceIcon", "The icon couldn’t be removed. Try again.")}
      onUpload={async (file, progress) => onChanged(await uploadSpaceIcon(space.id, file, progress))}
      onRemove={async () => onChanged(await removeSpaceIcon(space.id))}
    />
  );
}
