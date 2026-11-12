import { createFileRoute, Link, redirect, useNavigate } from "@tanstack/react-router";
import { useEffect, useRef, useState } from "react";
import { getAccount, getRememberedAccount, logout, type Account } from "../account/client";
import { getInitialAccount } from "../account/server";
import ProfileForm, { type ProfileFormHandle } from "../account/ProfileForm";
import NotificationSettings from "../account/NotificationSettings";
import PrivacySettings from "../account/PrivacySettings";
import StatusPage from "../components/StatusPage";
import Wordmark from "../components/Wordmark";
import { carryPending, signedInPath } from "../spaces/client";

export const Route = createFileRoute("/profile")({
  head: () => ({ meta: [{ title: "Profile - Caper" }] }),
  // Render the form on the server instead of an empty page that fills in
  // after hydration. In the browser, the account the app already has wins.
  loader: async ({ location }) => {
    const account = (typeof window === "undefined" ? undefined : getRememberedAccount()) ?? (await getInitialAccount());
    if (account === null) {
      throw redirect({ href: carryPending("/login", location.searchStr), replace: true });
    }
    return { initialAccount: account };
  },
  component: Profile,
});

function Profile() {
  const navigate = useNavigate();
  const profileForm = useRef<ProfileFormHandle>(null);
  const { initialAccount } = Route.useLoaderData();
  const [account, setAccount] = useState<Account | undefined>(() => getRememberedAccount() ?? initialAccount);
  // Only a missing session means signed out; an outage keeps you here to retry.
  const [failed, setFailed] = useState(false);
  const [loggingOut, setLoggingOut] = useState(false);
  const [logoutError, setLogoutError] = useState<string>();

  useEffect(() => {
    if (account || failed) return;
    void getAccount()
      .then((result) => {
        if (!result) return void navigate({ href: carryPending("/login", window.location.search), replace: true });
        setAccount(result);
      })
      .catch(() => setFailed(true));
  }, [account, failed, navigate]);

  if (failed)
    return (
      <StatusPage title="Your account didn’t load.">
        <p role="alert">Check your connection, then try again.</p>
        <div className="status-page-actions">
          <button type="button" className="primary" onClick={() => setFailed(false)}>
            Try again
          </button>
        </div>
      </StatusPage>
    );
  if (!account) return null;

  return (
    <main className="grid min-h-dvh place-items-start justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-12 max-[480px]:px-5 max-[480px]:pt-8">
      <section className="w-full max-w-[440px]">
        {/* The profile form guards navigation while its draft is unsaved. */}
        <div className="flex items-center justify-between gap-4">
          <Wordmark />
          {account.username && (
            <Link
              className="-my-2 -mr-2 inline-flex min-h-11 items-center gap-1.5 rounded-control px-2 text-[.85rem] text-content-muted no-underline transition-colors hover:text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2"
              to="/spaces"
            >
              <span aria-hidden="true">←</span> Back to spaces
            </Link>
          )}
        </div>
        <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] text-content-muted max-[480px]:mt-[42px]">
          {account.username ? "YOUR ACCOUNT" : "ONE LAST THING"}
        </p>
        <h1 className="my-5 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em]">
          {account.username ? "Make it yours." : "Choose how you show up."}
        </h1>
        <ProfileForm
          ref={profileForm}
          account={account}
          onCancel={account.username ? () => void navigate({ to: "/spaces" }) : undefined}
          onSaved={() => navigate({ href: signedInPath({ username: "saved" }, window.location.search) })}
        />
        {account.username && <NotificationSettings />}
        {account.username && <PrivacySettings />}
        <div className="mt-8 flex flex-wrap items-center justify-end gap-x-4 border-t border-border pt-3 text-[.85rem]">
          {logoutError && (
            <p className="m-0 text-[var(--error-text)]" role="alert">
              {logoutError}
            </p>
          )}
          <button
            className="-mr-2 min-h-11 cursor-pointer rounded-control border-0 bg-transparent px-2 text-content-muted transition-colors hover:text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2 disabled:cursor-wait"
            type="button"
            disabled={loggingOut}
            onClick={() =>
              profileForm.current?.requestClose(() => {
                setLoggingOut(true);
                setLogoutError(undefined);
                return logout().then(
                  () => window.location.assign("/"),
                  (reason) => {
                    // Stay signed in and say so, instead of a click that silently did nothing.
                    setLoggingOut(false);
                    setLogoutError("Couldn’t log out. Check your connection and try again.");
                    throw reason;
                  },
                );
              })
            }
          >
            {loggingOut ? "Logging out…" : "Log out"}
          </button>
        </div>
      </section>
    </main>
  );
}
