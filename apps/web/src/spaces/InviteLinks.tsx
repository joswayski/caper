import { useNavigate } from "@tanstack/react-router";
import { useEffect, useId, useState } from "react";
import {
  type Channel,
  createInviteLink,
  DEFAULT_INVITE_EXPIRY,
  DEFAULT_INVITE_MAX_USES,
  ensureDefaultInviteLink,
  INVITE_EXPIRY_UNITS,
  type InviteExpiryUnit,
  inviteExpirySeconds,
  inviteMaxUses,
  MAX_INVITE_USES,
  type InviteLink,
  inviteCode,
  inviteLinkSummary,
  inviteUrl,
  listInviteLinks,
  revokeInviteLink,
  SpacesApiError,
} from "./client";

function message(reason: unknown) {
  return reason instanceof SpacesApiError ? reason.message : "That request did not work.";
}

/** Reuses the default link when inviting; custom links stay behind settings. */
export function InviteLinkManager({
  spaceId,
  channels,
  prepareDefault = false,
}: {
  spaceId: string;
  channels: Channel[];
  prepareDefault?: boolean;
}) {
  const publicChannels = channels.filter((channel) => !channel.private);
  const [links, setLinks] = useState<InviteLink[]>();
  const [defaultCode, setDefaultCode] = useState<string>();
  const [expiryAmount, setExpiryAmount] = useState(DEFAULT_INVITE_EXPIRY.amount);
  const [expiryUnit, setExpiryUnit] = useState<InviteExpiryUnit>(DEFAULT_INVITE_EXPIRY.unit);
  const [usesText, setUsesText] = useState(DEFAULT_INVITE_MAX_USES);
  const [landing, setLanding] = useState<string | null>(null);
  const [pending, setPending] = useState(true);
  const [copied, setCopied] = useState<string>();
  const [error, setError] = useState<string>();
  const ids = useId();
  useEffect(() => {
    let current = true;
    void (async () => {
      try {
        if (prepareDefault) {
          const link = await ensureDefaultInviteLink(spaceId);
          if (!current) return;
          setDefaultCode(link.code);
          setLinks([link]);
        }
        const result = await listInviteLinks(spaceId);
        if (current) setLinks(result.links);
      } catch (reason) {
        if (current) setError(message(reason));
      } finally {
        if (current) setPending(false);
      }
    })();
    return () => {
      current = false;
    };
  }, [spaceId, prepareDefault]);
  useEffect(() => {
    if (!copied) return;
    const timeout = setTimeout(() => setCopied(undefined), 2_000);
    return () => clearTimeout(timeout);
  }, [copied]);
  const defaultLink = links?.find((link) => link.code === defaultCode);
  const prepare = async () => {
    setPending(true);
    setError(undefined);
    try {
      const link = await ensureDefaultInviteLink(spaceId);
      setDefaultCode(link.code);
      setLinks((current) => [link, ...(current ?? []).filter((item) => item.code !== link.code)]);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setPending(false);
    }
  };
  const deactivate = async (code: string) => {
    setPending(true);
    setError(undefined);
    try {
      await revokeInviteLink(spaceId, code);
      setLinks((current) => current?.filter((item) => item.code !== code));
    } catch (reason) {
      if (reason instanceof SpacesApiError && reason.status === 404)
        setLinks((current) => current?.filter((item) => item.code !== code));
      else setError(message(reason));
    } finally {
      setPending(false);
    }
  };
  const copy = async (code: string) => {
    try {
      await navigator.clipboard.writeText(inviteUrl(code));
      setError(undefined);
      setCopied(code);
    } catch {
      setError("Couldn’t copy. Check clipboard permission and try again.");
    }
  };
  return (
    <section className="member-manager invite-links">
      {prepareDefault && (
        <>
          <div className="dialog-section-heading">
            <h3>Share an invite link</h3>
          </div>
          <p className="channel-name-guidance">
            Up to 100 people can join within 16 days of creation. Opens in the default channel.
          </p>
          {defaultLink ? (
            <>
              <div className="invite-link-share">
                <input
                  aria-label="Invite link"
                  readOnly
                  value={inviteUrl(defaultLink.code)}
                  onFocus={(event) => event.target.select()}
                />
                <button className="primary" type="button" onClick={() => void copy(defaultLink.code)}>
                  {copied === defaultLink.code ? "Copied" : "Copy link"}
                </button>
              </div>
              <p className="channel-name-guidance">{inviteLinkSummary(defaultLink)}</p>
            </>
          ) : pending ? (
            <p className="channel-name-guidance" role="status">
              Preparing invite link…
            </p>
          ) : (
            <div className="invite-link-share">
              <p className="channel-name-guidance">
                {error ? "Couldn’t prepare the invite link." : "No active default link."}
              </p>
              <button className="primary" type="button" onClick={() => void prepare()}>
                {error ? "Try again" : "Create another link"}
              </button>
            </div>
          )}
          <p className="channel-name-guidance">Reopening keeps the same live link and its original expiry.</p>
        </>
      )}
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      <details className="invite-link-settings">
        <summary>Link settings</summary>
        {defaultLink && (
          <div className="invite-link-deactivate">
            <p className="channel-name-guidance">
              Deactivate this link to stop new joins. Existing members stay. Opening Invite people again prepares a new
              link.
            </p>
            <button
              type="button"
              className="danger-outline"
              disabled={pending}
              onClick={() => void deactivate(defaultLink.code)}
            >
              Deactivate link
            </button>
          </div>
        )}
        <div className="dialog-section-heading">
          <h3>Create a separate link</h3>
        </div>
        <p className="channel-name-guidance">
          Choose different limits or a landing channel. Existing links keep their settings.
        </p>
        <form
          className="invite-link-options"
          onSubmit={(event) => {
            event.preventDefault();
            if (pending) return;
            const maxAge = inviteExpirySeconds(expiryAmount, expiryUnit);
            const maxUses = inviteMaxUses(usesText);
            const invalid = maxAge.error ?? maxUses.error;
            if (invalid) return setError(invalid);
            setPending(true);
            setError(undefined);
            void createInviteLink(spaceId, { channelId: landing, maxAge: maxAge.value!, maxUses: maxUses.value! })
              .then((link) => {
                setLinks((current) => [link, ...(current ?? [])]);
              })
              .catch((reason) => setError(message(reason)))
              .finally(() => setPending(false));
          }}
        >
          <div className="invite-link-expiry" role="group" aria-labelledby={`${ids}-age-label`}>
            <span id={`${ids}-age-label`}>Expire after</span>
            <div>
              {expiryUnit !== "never" && (
                <input
                  aria-label="Expire after amount"
                  inputMode="numeric"
                  pattern="[0-9]*"
                  value={expiryAmount}
                  disabled={pending}
                  onChange={(event) => {
                    setExpiryAmount(event.target.value.replace(/\D/g, "").slice(0, 6));
                    setError(undefined);
                  }}
                />
              )}
              <select
                aria-label="Expire after unit"
                value={expiryUnit}
                disabled={pending}
                onChange={(event) => {
                  setExpiryUnit(event.target.value as InviteExpiryUnit);
                  setError(undefined);
                }}
              >
                {INVITE_EXPIRY_UNITS.map((option) => (
                  <option key={option.value} value={option.value}>
                    {option.label}
                  </option>
                ))}
              </select>
            </div>
          </div>
          <label htmlFor={`${ids}-uses`}>
            Max uses
            <input
              id={`${ids}-uses`}
              inputMode="numeric"
              pattern="[0-9]*"
              placeholder="No limit"
              title={`1 to ${MAX_INVITE_USES.toLocaleString("en-US")}, or empty for no limit`}
              value={usesText}
              disabled={pending}
              onChange={(event) => {
                setUsesText(event.target.value.replace(/\D/g, "").slice(0, 6));
                setError(undefined);
              }}
            />
          </label>
          <label htmlFor={`${ids}-channel`}>
            Opens in
            <select
              id={`${ids}-channel`}
              value={landing ?? ""}
              disabled={pending}
              onChange={(event) => setLanding(event.target.value || null)}
            >
              <option value="">Default channel</option>
              {publicChannels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  #{channel.name}
                </option>
              ))}
            </select>
          </label>
          <button type="submit" disabled={pending}>
            {pending ? "Creating…" : "Create link"}
          </button>
        </form>
        {!links && !error && (
          <p className="channel-name-guidance" role="status">
            Loading invite links…
          </p>
        )}
        {!!links?.length && (
          <ul>
            {links.map((link) => (
              <li key={link.code}>
                <span className="invite-link-row">
                  <strong>{inviteUrl(link.code).replace(/^https?:\/\//, "")}</strong>
                  <small>{inviteLinkSummary(link)}</small>
                </span>
                <span className="invite-link-actions">
                  <button type="button" className="invite-link-copy" onClick={() => void copy(link.code)}>
                    {copied === link.code ? "Copied" : "Copy"}
                  </button>
                  <button type="button" disabled={pending} onClick={() => void deactivate(link.code)}>
                    Deactivate
                  </button>
                </span>
              </li>
            ))}
          </ul>
        )}
      </details>
      {copied && (
        <p className="sr-only" role="status">
          Invite link copied.
        </p>
      )}
    </section>
  );
}

/** "Have an invite link?" beneath Create a space, like Discord's Join a Server. */
export function JoinInviteLinkForm({ disabled = false }: { disabled?: boolean }) {
  const navigate = useNavigate();
  const [value, setValue] = useState("");
  const [error, setError] = useState<string>();
  const id = useId();
  return (
    <form
      className="join-invite-link"
      onSubmit={(event) => {
        event.preventDefault();
        const code = inviteCode(value);
        if (!code) return setError("Enter an invite link like https://caper.chat/invite/AbCdE12345.");
        void navigate({ to: "/invite/$code", params: { code } });
      }}
    >
      <div className="dialog-section-heading">
        <h3>Have an invite link?</h3>
      </div>
      <div className="member-add">
        <label className="sr-only" htmlFor={id}>
          Invite link
        </label>
        <input
          id={id}
          value={value}
          autoComplete="off"
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          disabled={disabled}
          placeholder="https://caper.chat/invite/…"
          onChange={(event) => {
            setValue(event.target.value);
            setError(undefined);
          }}
        />
        <button type="submit" disabled={disabled || !value.trim()}>
          Join
        </button>
      </div>
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
    </form>
  );
}
