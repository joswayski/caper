import { useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { type Account, getAccount } from "../account/client";
import { primaryButtonClass, Spinner } from "../account/controls";
import Avatar from "../components/Avatar";
import Wordmark from "../components/Wordmark";
import {
  appInviteUrl,
  invitedSpacePath,
  inviteExpiry,
  type InvitePreview,
  joinInvite,
  previewInvite,
  SpacesApiError,
} from "./client";

type State =
  | { status: "loading" }
  | { status: "invalid" }
  | { status: "unavailable" }
  | { status: "ready"; preview: InvitePreview; account: Account | null };

function sentence(message: string) {
  return message.charAt(0).toUpperCase() + message.slice(1).replace(/([^.!?])$/, "$1.");
}

/** The public landing page for `caper.chat/invite/<code>`, like Discord's invite page. */
export default function InvitePage({ code }: { code: string }) {
  const navigate = useNavigate();
  const [state, setState] = useState<State>({ status: "loading" });
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

  useEffect(() => {
    let current = true;
    setState({ status: "loading" });
    void Promise.all([previewInvite(code), getAccount().catch(() => null)])
      .then(([preview, account]) => current && setState({ status: "ready", preview, account }))
      .catch((reason) => {
        if (!current) return;
        setState({ status: reason instanceof SpacesApiError && reason.status === 404 ? "invalid" : "unavailable" });
      });
    return () => {
      current = false;
    };
  }, [code]);

  const join = async () => {
    if (pending || state.status !== "ready") return;
    const { account } = state;
    if (!account) return void navigate({ href: `/login?invite=${code}` });
    if (!account.username || !account.displayName) return void navigate({ href: `/profile?invite=${code}` });
    setPending(true);
    setError(undefined);
    try {
      const joined = await joinInvite(code);
      await navigate({ href: invitedSpacePath(joined.space.id, joined.channelId) });
    } catch (reason) {
      if (reason instanceof SpacesApiError && reason.status === 404) setState({ status: "invalid" });
      else
        setError(
          reason instanceof SpacesApiError ? sentence(reason.message) : "Something went wrong. Please try again.",
        );
      setPending(false);
    }
  };

  return (
    <main className="grid min-h-dvh place-items-start justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-12 max-[480px]:px-5 max-[480px]:pt-8">
      <section className="w-full max-w-[440px]">
        <Wordmark />
        {state.status === "loading" && (
          <p className="mt-14 flex items-center gap-3 text-content-muted" role="status">
            <Spinner /> Loading invite…
          </p>
        )}
        {(state.status === "invalid" || state.status === "unavailable") && (
          <>
            <h1 className="mt-14 mb-4 text-[clamp(2rem,6vw,2.6rem)] leading-[1.1] font-bold tracking-[-.05em]">
              {state.status === "invalid" ? "This invite link is invalid or has expired." : "Invites are unavailable."}
            </h1>
            <p className="leading-[1.65] text-content-muted">
              {state.status === "invalid"
                ? "Ask whoever shared it for a new link."
                : "We couldn’t load this invite. Please try again in a moment."}
            </p>
            <a className={`${primaryButtonClass} mt-8 no-underline`} href="/">
              Go to Caper <span aria-hidden="true">→</span>
            </a>
          </>
        )}
        {state.status === "ready" && (
          <>
            <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] text-content-muted max-[480px]:mt-[42px]">
              {state.preview.alreadyMember ? "YOU’RE ALREADY A MEMBER" : "YOU’RE INVITED TO JOIN"}
            </p>
            <h1 className="my-4 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em] wrap-anywhere">
              {state.preview.space.name}
            </h1>
            <div className="flex items-center gap-3 text-content-muted">
              <span className="size-8 shrink-0 overflow-hidden rounded-[var(--radius-avatar)]" aria-hidden="true">
                <Avatar avatarId={state.preview.inviter.avatarId} name={state.preview.inviter.displayName} />
              </span>
              <p className="leading-[1.5]">
                <strong className="text-content">{state.preview.inviter.displayName}</strong> invited you
                {state.preview.channel && (
                  <>
                    {" "}
                    to <strong className="text-content">#{state.preview.channel.name}</strong>
                  </>
                )}
                .
              </p>
            </div>
            <p className="mt-3 text-[.85rem] text-content-muted">
              {state.preview.memberCount} {state.preview.memberCount === 1 ? "member" : "members"}
              {!state.preview.alreadyMember && <> · {inviteExpiry(state.preview.expiresAt)}</>}
            </p>
            {error && (
              <p className="mt-6 rounded-control border border-terracotta px-3.5 py-3 leading-[1.5]" role="alert">
                {error}
              </p>
            )}
            <button
              className={`${primaryButtonClass} mt-8 min-w-[13rem]`}
              type="button"
              disabled={pending}
              aria-busy={pending}
              onClick={() => void join()}
            >
              {pending ? (
                <>
                  <Spinner />
                  {state.preview.alreadyMember ? "Opening…" : "Joining…"}
                </>
              ) : (
                <>
                  {!state.account
                    ? "Sign in to join"
                    : !state.account.username
                      ? "Finish your profile to join"
                      : state.preview.alreadyMember
                        ? "Open space"
                        : "Join space"}{" "}
                  <span aria-hidden="true">→</span>
                </>
              )}
            </button>
            <p className="mt-5 text-right text-[.85rem] text-content-muted">
              Have the app?{" "}
              <a
                className="rounded-control text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2"
                href={appInviteUrl(code)}
              >
                Open in the Caper app
              </a>
            </p>
          </>
        )}
      </section>
    </main>
  );
}
