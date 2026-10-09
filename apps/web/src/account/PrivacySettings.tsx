import { useEffect, useId, useRef, useState } from "react";
import Avatar from "../components/Avatar";
import { refreshBlocks, unblock, useBlocks } from "../spaces/blocks.ts";
import { getDirectPrivacy, setDirectPrivacy, type DirectPrivacy } from "../spaces/client.ts";
import "./privacy.css";

const choices: Array<{ value: DirectPrivacy; label: string; hint?: string }> = [
  { value: "anyone", label: "Anyone", hint: "People outside your spaces send a message request first." },
  { value: "spaces", label: "People in my spaces" },
  { value: "nobody", label: "No one new", hint: "Conversations you already have stay open." },
];

/** "Who can start a DM with you" and the accounts you blocked. Saves as you choose. */
export default function PrivacySettings() {
  const [privacy, setPrivacy] = useState<DirectPrivacy>();
  const [saving, setSaving] = useState(false);
  const [privacyError, setPrivacyError] = useState<string>();
  const [blocksLoaded, setBlocksLoaded] = useState(false);
  const [blocksError, setBlocksError] = useState<string>();
  const [unblocking, setUnblocking] = useState<string>();
  const blocks = useBlocks();
  const name = useId();
  // Only the latest load (initial or a retry) may update state; unmounting invalidates both.
  const privacyRequest = useRef(0);
  const blocksRequest = useRef(0);

  const loadPrivacy = () => {
    const request = ++privacyRequest.current;
    setPrivacyError(undefined);
    void getDirectPrivacy()
      .then((result) => {
        if (request === privacyRequest.current) setPrivacy(result.directMessages);
      })
      .catch(() => {
        if (request === privacyRequest.current) setPrivacyError("Your DM setting couldn’t load.");
      });
  };
  const loadBlocks = () => {
    const request = ++blocksRequest.current;
    setBlocksError(undefined);
    void refreshBlocks()
      .then(() => {
        if (request === blocksRequest.current) setBlocksLoaded(true);
      })
      .catch(() => {
        if (request === blocksRequest.current) setBlocksError("Blocked accounts couldn’t load.");
      });
  };

  useEffect(() => {
    loadPrivacy();
    loadBlocks();
    return () => {
      privacyRequest.current++;
      blocksRequest.current++;
    };
  }, []);

  const choose = (next: DirectPrivacy) => {
    if (saving || next === privacy) return;
    const previous = privacy;
    setPrivacy(next);
    setSaving(true);
    setPrivacyError(undefined);
    void setDirectPrivacy(next)
      .then((result) => setPrivacy(result.directMessages))
      .catch(() => {
        setPrivacy(previous);
        setPrivacyError("That change wasn’t saved. Try again.");
      })
      .finally(() => setSaving(false));
  };

  return (
    <div className="privacy-settings">
      {/* Only while loading: disabling during a save would drop focus from the chosen radio. */}
      <fieldset disabled={!privacy} aria-busy={saving}>
        <legend>Who can start a DM with you</legend>
        {choices.map((choice) => (
          <label key={choice.value}>
            <input
              type="radio"
              name={name}
              value={choice.value}
              checked={privacy === choice.value}
              onChange={() => choose(choice.value)}
            />
            <span>
              <strong>{choice.label}</strong>
              {choice.hint && <small>{choice.hint}</small>}
            </span>
          </label>
        ))}
      </fieldset>
      {privacyError && (
        <p className="privacy-error" role="alert">
          {privacyError}
          {!privacy && (
            <button type="button" onClick={loadPrivacy}>
              Try again
            </button>
          )}
        </p>
      )}
      <section aria-labelledby={`${name}-blocked`}>
        <h3 id={`${name}-blocked`}>Blocked accounts</h3>
        {blocks.length ? (
          <ul>
            {blocks.map((account) => (
              <li key={account.id}>
                <span className="privacy-avatar">
                  <Avatar avatarId={account.avatarId} name={account.displayName} />
                </span>
                <span>
                  <strong>{account.displayName}</strong>
                  {account.username && <small>@{account.username}</small>}
                </span>
                <button
                  type="button"
                  disabled={unblocking === account.id}
                  aria-label={`Unblock ${account.displayName}`}
                  onClick={() => {
                    setUnblocking(account.id);
                    setBlocksError(undefined);
                    void unblock(account.id)
                      .catch(() => setBlocksError(`Couldn’t unblock ${account.displayName}. Try again.`))
                      .finally(() => setUnblocking(undefined));
                  }}
                >
                  {unblocking === account.id ? "Unblocking…" : "Unblock"}
                </button>
              </li>
            ))}
          </ul>
        ) : (
          blocksLoaded && <p>You haven’t blocked anyone.</p>
        )}
        {blocksError && (
          <p className="privacy-error" role="alert">
            {blocksError}
            {!blocksLoaded && (
              <button type="button" onClick={loadBlocks}>
                Try again
              </button>
            )}
          </p>
        )}
      </section>
    </div>
  );
}
