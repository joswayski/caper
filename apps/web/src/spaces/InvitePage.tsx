import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { primaryButtonClass, Spinner } from "../account/controls";
import AdminBadge from "../chat/AdminBadge";
import "../chat/chat.css";
import Avatar from "../components/Avatar";
import SpaceIcon from "../components/SpaceIcon";
import Wordmark from "../components/Wordmark";
import { appInviteUrl, invitedSpacePath, joinInvite, SpacesApiError } from "./client";
import { friendlyError } from "./errors";
import type { InvitePageState } from "./invite-server";

/** The public landing page for `caper.chat/invite/<code>`, like Discord's invite page. */
export default function InvitePage({ code, state: loadedState }: { code: string; state: InvitePageState }) {
  const navigate = useNavigate();
  const [invalid, setInvalid] = useState(false);
  const state: InvitePageState = invalid ? { status: "invalid" } : loadedState;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();

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
      if (reason instanceof SpacesApiError && reason.status === 404) setInvalid(true);
      else setError(friendlyError(reason));
      setPending(false);
    }
  };

  return (
    <main className="grid min-h-dvh place-items-start justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-12 max-[480px]:px-5 max-[480px]:pt-8">
      <section className="w-full max-w-[440px]">
        <Wordmark />
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
            {state.preview.space.iconUrl && (
              <span
                className="relative mt-5 grid size-16 place-items-center overflow-hidden rounded-[30%] border border-border bg-surface-raised text-[1.4rem] font-bold text-content-muted"
                aria-hidden="true"
              >
                <SpaceIcon space={state.preview.space} />
              </span>
            )}
            <h1 className="my-4 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em] wrap-anywhere">
              {state.preview.space.name}
            </h1>
            <div className="flex items-center gap-2 text-content-muted">
              <span className="size-8 shrink-0 overflow-hidden rounded-[var(--radius-avatar)]" aria-hidden="true">
                <Avatar person={state.preview.inviter} name={state.preview.inviter.displayName} />
              </span>
              <p className="leading-[1.5]">
                <span className="inline-flex items-center gap-1.5">
                  <strong className="text-content">{state.preview.inviter.displayName}</strong>
                  {state.preview.inviter.admin && <AdminBadge />}
                </span>{" "}
                invited you
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
              {!state.preview.alreadyMember && <> · {state.expiry}</>}
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
                    : !state.account.username || !state.account.displayName
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
