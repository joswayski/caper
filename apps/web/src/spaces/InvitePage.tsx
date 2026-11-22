import { useNavigate } from "@tanstack/react-router";
import { useState } from "react";
import { primaryButtonClass, Spinner } from "../account/controls";
import AdminBadge from "../chat/AdminBadge";
import "../chat/chat.css";
import Avatar from "../components/Avatar";
import SpaceIcon from "../components/SpaceIcon";
import Wordmark from "../components/Wordmark";
import { appInviteUrl, invitedSpacePath, joinInvite, SpacesApiError } from "./client";

/** "You’re already a member, along with 3 others" */
export function alreadyMemberLabel(memberCount: number) {
  const others = memberCount - 1;
  return others > 0
    ? `You’re already a member, along with ${others} ${others === 1 ? "other" : "others"}`
    : "You’re already a member";
}

/**
 * Hands an invite to the installed app. A page can't tell whether it is installed:
 * this resolves true once the page loses focus or visibility (the app, or a browser
 * or OS prompt, took over) and false if nothing happened within 1.5 seconds.
 */
function openInApp(code: string, fallback: string): Promise<boolean> {
  // Chrome on Android opens the app, or the web space when it isn't installed.
  location.href = /Android/i.test(navigator.userAgent)
    ? `intent://invite/${code}#Intent;scheme=caper;S.browser_fallback_url=${encodeURIComponent(fallback)};end`
    : appInviteUrl(code);
  return new Promise((resolve) => {
    const left = () => {
      if (document.hidden || !document.hasFocus()) finish(true);
    };
    const finish = (opened: boolean) => {
      clearTimeout(timer);
      window.removeEventListener("blur", left);
      document.removeEventListener("visibilitychange", left);
      resolve(opened);
    };
    const timer = setTimeout(() => finish(false), 1500);
    window.addEventListener("blur", left);
    document.addEventListener("visibilitychange", left);
  });
}

/** iPhones with the app open invite links in it directly; Safari alerts on an unknown scheme. */
const appleMobile = () =>
  /iPhone|iPad|iPod/.test(navigator.userAgent) ||
  (/Macintosh/.test(navigator.userAgent) && navigator.maxTouchPoints > 1);
import { friendlyError } from "./errors";
import type { InvitePageState } from "./invite-server";

/** The public landing page for `caper.chat/invite/<code>`, like Discord's invite page. */
export default function InvitePage({ code, state: loadedState }: { code: string; state: InvitePageState }) {
  const navigate = useNavigate();
  const [invalid, setInvalid] = useState(false);
  const state: InvitePageState = invalid ? { status: "invalid" } : loadedState;
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string>();
  const [openedInApp, setOpenedInApp] = useState<string>();

  const join = async () => {
    if (pending || state.status !== "ready") return;
    const { account } = state;
    if (!account) return void navigate({ href: `/login?invite=${code}` });
    if (!account.username || !account.displayName) return void navigate({ href: `/profile?invite=${code}` });
    setPending(true);
    setError(undefined);
    try {
      const joined = await joinInvite(code);
      const path = invitedSpacePath(joined.space.id, joined.channelId);
      // Members open the space in the app when it's installed, else here.
      if (state.preview.alreadyMember && !appleMobile() && (await openInApp(code, new URL(path, location.href).href))) {
        setOpenedInApp(path);
        setPending(false);
        return;
      }
      await navigate({ href: path });
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
            <h1 className="mt-14 mb-4 text-[clamp(2rem,6vw,2.6rem)] leading-[1.1] font-bold tracking-[-.025em]">
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
            {state.preview.space.iconUrl && (
              <span
                className="relative mt-14 grid max-[480px]:mt-[42px] size-16 place-items-center overflow-hidden rounded-[30%] border border-border bg-surface-raised text-[1.4rem] font-bold text-content-muted"
                aria-hidden="true"
              >
                <SpaceIcon space={state.preview.space} />
              </span>
            )}
            <h1
              className={`${state.preview.space.iconUrl ? "mt-4" : "mt-14 max-[480px]:mt-[42px]"} mb-4 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.025em] wrap-anywhere`}
            >
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
              {state.preview.alreadyMember
                ? alreadyMemberLabel(state.preview.memberCount)
                : `${state.preview.memberCount} ${state.preview.memberCount === 1 ? "member" : "members"}`}
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
            <p className="mt-5 text-right text-[.85rem] text-content-muted" role={openedInApp ? "status" : undefined}>
              {openedInApp ? "Opening in the Caper app… Didn’t open?" : "Have the app?"}{" "}
              <a
                className="rounded-control text-content underline-offset-4 hover:underline focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2"
                href={openedInApp ?? appInviteUrl(code)}
              >
                {openedInApp ? "Continue in the browser" : "Open in the Caper app"}
              </a>
            </p>
          </>
        )}
      </section>
    </main>
  );
}
