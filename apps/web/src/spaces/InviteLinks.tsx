import { useNavigate } from "@tanstack/react-router";
import { useEffect, useId, useState } from "react";
import {
  type Channel,
  createInviteLink,
  DEFAULT_INVITE_MAX_AGE,
  INVITE_MAX_AGES,
  INVITE_MAX_USES,
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

function optionValue(value: number | null) {
  return value === null ? "" : String(value);
}

function parseOption(value: string) {
  return value === "" ? null : Number(value);
}

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
  const [maxAge, setMaxAge] = useState<number | null>(DEFAULT_INVITE_MAX_AGE);
  const [maxUses, setMaxUses] = useState<number | null>(null);
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
      .catch((reason) => current && setError(message(reason)));
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
          setPending(true);
          setError(undefined);
          void createInviteLink(spaceId, { channelId: landing, maxAge, maxUses })
            .then(async (link) => {
              setLinks((current) => [link, ...(current ?? [])]);
              await copy(link.code);
            })
            .catch((reason) => setError(message(reason)))
            .finally(() => setPending(false));
        }}
      >
        <label htmlFor={`${ids}-age`}>
          Expire after
          <select
            id={`${ids}-age`}
            value={optionValue(maxAge)}
            disabled={pending}
            onChange={(event) => setMaxAge(parseOption(event.target.value))}
          >
            {INVITE_MAX_AGES.map((option) => (
              <option key={option.label} value={optionValue(option.value)}>
                {option.label}
              </option>
            ))}
          </select>
        </label>
        <label htmlFor={`${ids}-uses`}>
          Max uses
          <select
            id={`${ids}-uses`}
            value={optionValue(maxUses)}
            disabled={pending}
            onChange={(event) => setMaxUses(parseOption(event.target.value))}
          >
            {INVITE_MAX_USES.map((option) => (
              <option key={option.label} value={optionValue(option.value)}>
                {option.label}
              </option>
            ))}
          </select>
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
                <button type="button" onClick={() => void copy(link.code)}>
                  {copied === link.code ? "Copied" : "Copy"}
                </button>
                <button
                  type="button"
                  disabled={pending}
                  onClick={() =>
                    void revokeInviteLink(spaceId, link.code)
                      .then(() => setLinks((current) => current?.filter((item) => item.code !== link.code)))
                      .catch((reason) => setError(message(reason)))
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
