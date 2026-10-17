import { useNavigate } from "@tanstack/react-router";
import { type KeyboardEvent, useEffect, useId, useState } from "react";
import {
  autoUpdate,
  flip,
  FloatingFocusManager,
  offset,
  shift,
  useClick,
  useDismiss,
  useFloating,
  useInteractions,
  useRole,
} from "@floating-ui/react";
import { Ban, ChevronDown, Copy, RefreshCw, Settings } from "lucide-react";
import { Dialog } from "./Dialog";
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
  regenerateInviteLink,
  revokeInviteLink,
  SpacesApiError,
} from "./client";

function message(reason: unknown) {
  return reason instanceof SpacesApiError ? reason.message : "That request did not work.";
}

function InviteLinkActions({
  disabled,
  regenerate,
  settings,
  deactivate,
}: {
  disabled: boolean;
  regenerate: () => void;
  settings: () => void;
  deactivate: () => void;
}) {
  const [open, setOpen] = useState(false);
  const warningId = useId();
  const { refs, floatingStyles, context } = useFloating({
    open,
    onOpenChange: setOpen,
    placement: "bottom-end",
    strategy: "fixed",
    middleware: [offset(8), flip(), shift({ padding: 12, crossAxis: true })],
    whileElementsMounted: autoUpdate,
  });
  const { getReferenceProps, getFloatingProps } = useInteractions([
    useClick(context),
    useDismiss(context),
    useRole(context),
  ]);
  const choose = (action: () => void) => {
    setOpen(false);
    action();
  };
  const dismissOnEscape = (event: KeyboardEvent<HTMLElement>) => {
    if (!open || event.key !== "Escape") return;
    // The surrounding native dialog handles Escape on key-down too.
    event.preventDefault();
    event.stopPropagation();
    if (!event.nativeEvent.isComposing) setOpen(false);
  };
  return (
    <>
      <button
        ref={refs.setReference}
        type="button"
        className="primary invite-link-more"
        disabled={disabled}
        aria-label="More invite options"
        title="More invite options"
        {...getReferenceProps({ onKeyDown: dismissOnEscape })}
      >
        <ChevronDown size={16} aria-hidden="true" />
      </button>
      {open && (
        <FloatingFocusManager context={context} modal={false}>
          <div
            ref={refs.setFloating}
            style={floatingStyles}
            className="invite-link-menu"
            aria-label="More invite options"
            {...getFloatingProps({ onKeyDown: dismissOnEscape })}
          >
            <button
              type="button"
              disabled={disabled}
              aria-label="Regenerate link"
              aria-describedby={warningId}
              onClick={() => choose(regenerate)}
            >
              <RefreshCw size={16} aria-hidden="true" />
              <span>
                Regenerate link
                <small id={warningId}>Replaces this link with 30 days and 100 joins. The old link stops working.</small>
              </span>
            </button>
            <button type="button" onClick={() => choose(settings)}>
              <Settings size={16} aria-hidden="true" /> Custom links
            </button>
            <button
              type="button"
              className="invite-link-menu-danger"
              disabled={disabled}
              onClick={() => choose(deactivate)}
            >
              <Ban size={16} aria-hidden="true" /> Deactivate link
            </button>
          </div>
        </FloatingFocusManager>
      )}
    </>
  );
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
  const [selectedChannel, setSelectedChannel] = useState<string | null>(publicChannels[0]?.id ?? null);
  const [expiryAmount, setExpiryAmount] = useState(DEFAULT_INVITE_EXPIRY.amount);
  const [expiryUnit, setExpiryUnit] = useState<InviteExpiryUnit>(DEFAULT_INVITE_EXPIRY.unit);
  const [usesText, setUsesText] = useState(DEFAULT_INVITE_MAX_USES);
  const [landing, setLanding] = useState<string | null>(publicChannels[0]?.id ?? null);
  const [pending, setPending] = useState(true);
  const [copied, setCopied] = useState<string>();
  const [error, setError] = useState<string>();
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [confirmation, setConfirmation] = useState<{ action: "regenerate" | "deactivate"; code: string }>();
  const [now, setNow] = useState(Date.now);
  const ids = useId();
  useEffect(() => {
    const interval = setInterval(() => setNow(Date.now()), 30_000);
    return () => clearInterval(interval);
  }, []);
  useEffect(() => {
    let current = true;
    void (async () => {
      try {
        if (prepareDefault) {
          const link = await ensureDefaultInviteLink(spaceId);
          if (!current) return;
          setDefaultCode(link.code);
          setSelectedChannel(link.channel?.id ?? null);
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
  const chosenChannel =
    publicChannels.find((channel) => channel.id === selectedChannel)?.id ?? publicChannels[0]?.id ?? null;
  const customChannel = publicChannels.find((channel) => channel.id === landing)?.id ?? publicChannels[0]?.id ?? null;
  const prepare = async (channelId: string | null = chosenChannel) => {
    if (pending) return;
    setPending(true);
    setError(undefined);
    try {
      const link = await ensureDefaultInviteLink(spaceId, channelId);
      setDefaultCode(link.code);
      setSelectedChannel(link.channel?.id ?? null);
      setCopied(undefined);
      setLinks((current) => [link, ...(current ?? []).filter((item) => item.code !== link.code)]);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setPending(false);
    }
  };
  const regenerate = async (code: string) => {
    if (pending) return;
    setPending(true);
    setError(undefined);
    try {
      const link = await regenerateInviteLink(spaceId, code);
      setDefaultCode(link.code);
      setSelectedChannel(link.channel?.id ?? null);
      setCopied(undefined);
      setNow(Date.now());
      setLinks((current) => [link, ...(current ?? []).filter((item) => item.code !== code)]);
    } catch (reason) {
      setError(message(reason));
    } finally {
      setPending(false);
    }
  };
  const deactivate = async (code: string) => {
    if (pending) return;
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
            Anyone with this link can join this space. New links allow {DEFAULT_INVITE_MAX_USES} joins for{" "}
            {DEFAULT_INVITE_EXPIRY.amount} days.
          </p>
          <label className="invite-link-destination" htmlFor={`${ids}-destination`}>
            Opens in
            <select
              id={`${ids}-destination`}
              value={chosenChannel ?? ""}
              disabled={pending || publicChannels.length === 0}
              onChange={(event) => void prepare(event.target.value || null)}
            >
              {publicChannels.length === 0 && <option value="">No public channels</option>}
              {publicChannels.map((channel) => (
                <option key={channel.id} value={channel.id}>
                  #{channel.name}
                </option>
              ))}
            </select>
          </label>
          {publicChannels.length === 0 && (
            <p className="channel-name-guidance">People will join the space without opening a channel.</p>
          )}
          {defaultLink ? (
            <>
              <div className="invite-link-share">
                <input
                  aria-label="Invite link"
                  readOnly
                  value={inviteUrl(defaultLink.code)}
                  onFocus={(event) => event.target.select()}
                />
                <div className="invite-link-split">
                  <button
                    className="primary"
                    type="button"
                    disabled={pending}
                    onClick={() => void copy(defaultLink.code)}
                  >
                    <Copy size={16} aria-hidden="true" />
                    {copied === defaultLink.code ? "Copied" : "Copy"}
                  </button>
                  <InviteLinkActions
                    disabled={pending}
                    regenerate={() => setConfirmation({ action: "regenerate", code: defaultLink.code })}
                    settings={() => setSettingsOpen(true)}
                    deactivate={() => setConfirmation({ action: "deactivate", code: defaultLink.code })}
                  />
                </div>
              </div>
              <p className="channel-name-guidance" role="status">
                {inviteLinkSummary(defaultLink, now)}
              </p>
            </>
          ) : pending ? (
            <p className="channel-name-guidance" role="status">
              Preparing invite link…
            </p>
          ) : (
            <div className="invite-link-share">
              <p className="channel-name-guidance">
                {error ? "Couldn’t prepare the invite link." : "No active invite link."}
              </p>
              <button className="primary" type="button" onClick={() => void prepare()}>
                {error ? "Try again" : "Create another link"}
              </button>
            </div>
          )}
          {!defaultLink && !pending && !settingsOpen && (
            <button type="button" onClick={() => setSettingsOpen(true)}>
              Custom links
            </button>
          )}
        </>
      )}
      {error && (
        <p className="space-form-error" role="alert">
          {error}
        </p>
      )}
      <details
        className="invite-link-settings"
        open={settingsOpen}
        hidden={prepareDefault && !settingsOpen}
        onToggle={(event) => setSettingsOpen(event.currentTarget.open)}
      >
        <summary>Custom links</summary>
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
            void createInviteLink(spaceId, { channelId: customChannel, maxAge: maxAge.value!, maxUses: maxUses.value! })
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
              value={customChannel ?? ""}
              disabled={pending || publicChannels.length === 0}
              onChange={(event) => setLanding(event.target.value || null)}
            >
              {publicChannels.length === 0 && <option value="">No public channels</option>}
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
                  <small>{inviteLinkSummary(link, now)}</small>
                </span>
                <span className="invite-link-actions">
                  <button type="button" className="invite-link-copy" onClick={() => void copy(link.code)}>
                    {copied === link.code ? "Copied" : "Copy"}
                  </button>
                  <button
                    type="button"
                    disabled={pending}
                    onClick={() => setConfirmation({ action: "deactivate", code: link.code })}
                  >
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
      {confirmation && (
        <Dialog
          title={confirmation.action === "regenerate" ? "Regenerate invite link?" : "Deactivate invite link?"}
          dismissOnBackdrop
          onClose={() => setConfirmation(undefined)}
        >
          <div className="delete-confirmation">
            <p>This shared URL will stop accepting new joins. Existing members stay.</p>
            {confirmation.action === "regenerate" && <p>The replacement allows 100 joins for 30 days.</p>}
            <p className="channel-name-guidance">Current link: {inviteUrl(confirmation.code)}</p>
            <div className="space-dialog-actions">
              <button type="button" className="secondary" data-initial-focus onClick={() => setConfirmation(undefined)}>
                Cancel
              </button>
              <button
                type="button"
                className="danger"
                disabled={pending}
                onClick={(event) => {
                  if (event.detail > 1) return;
                  const { action, code } = confirmation;
                  setConfirmation(undefined);
                  void (action === "regenerate" ? regenerate(code) : deactivate(code));
                }}
              >
                {confirmation.action === "regenerate" ? "Regenerate link" : "Deactivate link"}
              </button>
            </div>
          </div>
        </Dialog>
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
