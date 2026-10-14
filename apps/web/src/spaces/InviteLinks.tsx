import { useNavigate } from "@tanstack/react-router";
import { useEffect, useId, useState } from "react";
import {
  type Channel,
  createInviteLink,
  DEFAULT_INVITE_EXPIRY,
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
} from "./client";
import { friendlyError } from "./errors";

/** Owner controls for Discord-style shareable links in Manage space. */
export function InviteLinkManager({
  spaceId,
  channels,
  channelId,
}: {
  spaceId: string;
  channels: Channel[];
  /** The open channel; links land there by default when it is public. */
  channelId?: string;
}) {
  const publicChannels = channels.filter((channel) => !channel.private);
  const [links, setLinks] = useState<InviteLink[]>();
  const [expiryAmount, setExpiryAmount] = useState(DEFAULT_INVITE_EXPIRY.amount);
  const [expiryUnit, setExpiryUnit] = useState<InviteExpiryUnit>(DEFAULT_INVITE_EXPIRY.unit);
  const [usesText, setUsesText] = useState("");
  const [landing, setLanding] = useState<string | null>(
    publicChannels.some((channel) => channel.id === channelId) ? channelId! : null,
  );
  const [pending, setPending] = useState(false);
  const [copied, setCopied] = useState<string>();
  const [error, setError] = useState<string>();
  const ids = useId();
  useEffect(() => {
    let current = true;
    void listInviteLinks(spaceId)
      .then((result) => current && setLinks(result.links))
      .catch((reason) => current && setError(friendlyError(reason)));
    return () => {
      current = false;
    };
  }, [spaceId]);
  const copy = async (code: string) => {
    try {
      await navigator.clipboard.writeText(inviteUrl(code));
      setCopied(code);
    } catch {
      setError("Couldn’t copy. Check clipboard permission and try again.");
    }
  };
  return (
    <section className="member-manager invite-links">
      <div className="dialog-section-heading">
        <h3>Invite link</h3>
      </div>
      <p className="channel-name-guidance">Anyone with the link can join this space.</p>
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
            .then(async (link) => {
              setLinks((current) => [link, ...(current ?? [])]);
              await copy(link.code);
            })
            .catch((reason) => setError(friendlyError(reason)))
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
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
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
                <button
                  type="button"
                  disabled={pending}
                  onClick={() =>
                    void revokeInviteLink(spaceId, link.code)
                      .then(() => setLinks((current) => current?.filter((item) => item.code !== link.code)))
                      .catch((reason) => setError(friendlyError(reason)))
                  }
                >
                  Revoke
                </button>
              </span>
            </li>
          ))}
        </ul>
      )}
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
