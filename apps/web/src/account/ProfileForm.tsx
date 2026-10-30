import { useImperativeHandle, useRef, useState, type FormEvent, type Ref } from "react";
import { useBlocker } from "@tanstack/react-router";
import {
  AccountApiError,
  displayNameError,
  normalizeUsername,
  updateProfile,
  usernameError,
  type Account,
} from "./client";
import { fieldClass, primaryButtonClass, Spinner } from "./controls";
import { ProfilePictureSettings, type ProfilePictureDraft } from "./PictureSettings";
import { PictureError, uploadPicture } from "./picture-upload";
import { Dialog } from "../spaces/Dialog";

export interface ProfileFormHandle {
  requestClose: (close: () => void) => void;
}

export default function ProfileForm({
  account,
  onSaved,
  onCancel,
  ref,
}: {
  account: Account;
  onSaved: (account: Account) => void | Promise<void>;
  onCancel?: () => void;
  ref?: Ref<ProfileFormHandle>;
}) {
  // Live account updates must not move the baseline underneath an open draft.
  const [original] = useState(account);
  const [username, setUsername] = useState(account.username ?? "");
  const [displayName, setDisplayName] = useState(account.displayName ?? "");
  const [picture, setPicture] = useState<ProfilePictureDraft>({
    avatarId: account.avatarId ?? 0,
    avatarAsset: account.avatarAsset,
    avatarUrl: account.avatarUrl,
  });
  const [pictureBusy, setPictureBusy] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const [closeAction, setCloseAction] = useState<() => void>();
  const saving = useRef(false);
  const saved = useRef(false);
  const pictureChanged =
    !!picture.upload ||
    picture.avatarId !== (original.avatarId ?? 0) ||
    (picture.avatarAsset ?? null) !== (original.avatarAsset ?? null);
  const dirty =
    pictureChanged || username !== (original.username ?? "") || displayName.trim() !== (original.displayName ?? "");
  const blocker = useBlocker({
    shouldBlockFn: ({ current, next }) =>
      !saved.current &&
      (saving.current || dirty) &&
      (current.pathname !== next.pathname || JSON.stringify(current.search) !== JSON.stringify(next.search)),
    enableBeforeUnload: () => !saved.current && (saving.current || dirty),
    withResolver: true,
  });
  const requestClose = (close: () => void) => {
    if (saving.current || pictureBusy) return;
    if (dirty) setCloseAction(() => close);
    else close();
  };
  useImperativeHandle(ref, () => ({ requestClose }));
  const keepEditing = () => {
    setCloseAction(undefined);
    if (blocker.status === "blocked") blocker.reset();
  };

  async function save(event: FormEvent) {
    event.preventDefault();
    if (saving.current || pictureBusy || (!dirty && original.username)) return;
    // The form shows its own errors instead of the browser's validation bubbles.
    const invalid = usernameError(username) ?? displayNameError(displayName, original.displayName);
    if (invalid) return setError(invalid);
    saving.current = true;
    setPending(true);
    setError(undefined);
    try {
      let asset = picture.avatarAsset ?? null;
      if (picture.upload) {
        if (picture.uploadedAsset) asset = picture.uploadedAsset;
        else {
          const uploaded = await uploadPicture(picture.upload, { purpose: "avatar", draft: true }, () => {});
          if (typeof uploaded.id !== "string") throw new PictureError("Your picture couldn’t be uploaded. Try again.");
          asset = uploaded.id;
          setPicture({ ...picture, uploadedAsset: asset });
        }
      }
      const updated = await updateProfile(
        username,
        displayName,
        pictureChanged
          ? {
              avatarId: picture.avatarId,
              avatarAsset: asset,
              expectedAvatarId: original.avatarId ?? null,
              expectedAvatarAsset: original.avatarAsset ?? null,
            }
          : undefined,
      );
      saved.current = true;
      await onSaved(updated);
    } catch (saveError) {
      if (saveError instanceof AccountApiError && saveError.code === "profile_changed")
        setError("Your picture changed elsewhere. Close and reopen your profile to refresh.");
      else if (saveError instanceof AccountApiError && saveError.status === 409)
        setError("That username is already taken.");
      else if (saveError instanceof AccountApiError && saveError.status === 400)
        setError("Check the username and display name requirements.");
      else if (saveError instanceof PictureError) setError(saveError.message);
      else setError("Your profile could not be saved. Please try again.");
    } finally {
      saving.current = false;
      setPending(false);
    }
  }

  return (
    <>
      <fieldset disabled={pending} className="m-0 min-w-0 border-0 p-0">
        {account.username && (
          <div className="mt-6">
            <ProfilePictureSettings
              account={original}
              draft={picture}
              onChange={(draft) => {
                setPicture(draft);
                setError(undefined);
              }}
              onBusyChange={setPictureBusy}
            />
          </div>
        )}
        <form noValidate onSubmit={(event) => void save(event)}>
          <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="username">
            Username
          </label>
          <input
            className={fieldClass}
            id="username"
            aria-describedby="username-hint"
            name="username"
            value={username}
            onChange={(event) => {
              setUsername(normalizeUsername(event.target.value));
              setError(undefined);
            }}
            autoFocus={!account.username}
            autoComplete="username"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
            minLength={3}
            maxLength={32}
            pattern="[a-z0-9_]{3,32}"
            required
          />
          <small id="username-hint" className="mt-2.5 block text-[.8rem] leading-[1.5] text-content-muted">
            Must be unique across Caper. 3–32 lowercase letters, numbers, or underscores.
          </small>
          <label className="my-2 mt-6 block text-[.9rem] font-bold" htmlFor="display-name">
            Display name
          </label>
          <input
            className={fieldClass}
            id="display-name"
            aria-describedby="display-name-hint"
            name="displayName"
            value={displayName}
            onChange={(event) => {
              // Characters, like the server: a UTF-16 cut would shorten emoji names or split a pair.
              setDisplayName(Array.from(event.target.value).slice(0, 64).join(""));
              setError(undefined);
            }}
            autoComplete="name"
            required
          />
          <small id="display-name-hint" className="mt-2.5 block text-[.8rem] leading-[1.5] text-content-muted">
            This is what people see in conversations.
          </small>
          <div className="mt-7 flex flex-wrap items-center justify-end gap-3">
            <span className="mr-auto inline-flex items-center gap-2 text-[.85rem] text-content-muted" role="status">
              {dirty && (
                <>
                  <span className="size-1.5 rounded-full bg-terracotta" aria-hidden="true" />
                  Unsaved changes
                </>
              )}
            </span>
            {onCancel && (
              <button
                type="button"
                className="secondary"
                disabled={pending || pictureBusy}
                onClick={() => requestClose(onCancel)}
              >
                Cancel
              </button>
            )}
            <button
              className={`${primaryButtonClass} min-w-[10rem]`}
              type="submit"
              disabled={pending || pictureBusy || (!dirty && !!original.username)}
              aria-busy={pending}
            >
              {pending ? (
                <>
                  <Spinner />
                  Saving…
                </>
              ) : (
                <>
                  {account.username ? "Save profile" : "Finish setup"} <span aria-hidden="true">→</span>
                </>
              )}
            </button>
          </div>
          {/* Below the button, so an error never moves anything above it. */}
          {error && (
            <p className="mt-4 mb-0 rounded-control border border-terracotta px-3.5 py-3 leading-[1.5]" role="alert">
              {error}
            </p>
          )}
        </form>
      </fieldset>
      {(closeAction || blocker.status === "blocked") && (
        <Dialog title="Discard changes?" onClose={keepEditing}>
          <p>Your profile has unsaved changes. Discard them or keep editing.</p>
          <footer className="space-dialog-actions">
            <button type="button" className="secondary" data-initial-focus onClick={keepEditing}>
              Keep editing
            </button>
            <button
              type="button"
              className="primary"
              disabled={pending}
              onClick={() => {
                saved.current = true;
                if (blocker.status === "blocked") blocker.proceed();
                closeAction?.();
              }}
            >
              Discard
            </button>
          </footer>
        </Dialog>
      )}
    </>
  );
}
