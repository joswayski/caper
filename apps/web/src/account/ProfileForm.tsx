import { useState, type FormEvent } from "react";
import {
  AccountApiError,
  displayNameError,
  normalizeUsername,
  updateProfile,
  usernameError,
  type Account,
} from "./client";
import { fieldClass, primaryButtonClass, Spinner } from "./controls";
import { ProfilePictureSettings } from "./PictureSettings";

export default function ProfileForm({
  account,
  onSaved,
}: {
  account: Account;
  onSaved: (account: Account) => void | Promise<void>;
}) {
  const [username, setUsername] = useState(account.username ?? "");
  const [displayName, setDisplayName] = useState(account.displayName ?? "");
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  async function save(event: FormEvent) {
    event.preventDefault();
    if (pending) return;
    // The form shows its own errors instead of the browser's validation bubbles.
    const invalid = usernameError(username) ?? displayNameError(displayName, account.displayName);
    if (invalid) return setError(invalid);
    setPending(true);
    setError(undefined);
    try {
      await onSaved(await updateProfile(username, displayName));
    } catch (saveError) {
      if (saveError instanceof AccountApiError && saveError.status === 409) setError("That username is already taken.");
      else if (saveError instanceof AccountApiError && saveError.status === 400)
        setError("Check the username and display name requirements.");
      else setError("Your profile could not be saved. Please try again.");
    } finally {
      setPending(false);
    }
  }

  return (
    <>
      {/* Pictures save on the editor's Done action; the form below saves names. */}
      {account.username && (
        <div className="mt-6">
          <ProfilePictureSettings account={account} />
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
        <button
          className={`${primaryButtonClass} mt-7 min-w-[12.5rem]`}
          type="submit"
          disabled={pending}
          aria-busy={pending}
        >
          {pending ? (
            <>
              <Spinner />
              Saving…
            </>
          ) : (
            <>
              {account.username ? "Save profile" : "Finish account"} <span aria-hidden="true">→</span>
            </>
          )}
        </button>
        {/* Below the button, so an error never moves anything above it. */}
        {error && (
          <p className="mt-4 mb-0 rounded-control border border-terracotta px-3.5 py-3 leading-[1.5]" role="alert">
            {error}
          </p>
        )}
      </form>
    </>
  );
}
