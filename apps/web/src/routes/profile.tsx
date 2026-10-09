import { createFileRoute, useNavigate } from "@tanstack/react-router";
import { useEffect, useState } from "react";
import { getAccount, getRememberedAccount, logout, type Account } from "../account/client";
import ProfileForm from "../account/ProfileForm";
import NotificationSettings from "../account/NotificationSettings";
import PrivacySettings from "../account/PrivacySettings";
import Wordmark from "../components/Wordmark";

export const Route = createFileRoute("/profile")({
  head: () => ({ meta: [{ title: "Profile - Caper" }] }),
  component: Profile,
});

function Profile() {
  const navigate = useNavigate();
  const [account, setAccount] = useState<Account | undefined>(() => getRememberedAccount());

  useEffect(() => {
    if (account) return;
    void getAccount()
      .then((result) => {
        if (!result) return void navigate({ to: "/login", replace: true });
        setAccount(result);
      })
      .catch(() => void navigate({ to: "/login", replace: true }));
  }, [account, navigate]);

  if (!account) return null;

  return (
    <main className="grid min-h-dvh place-items-start justify-items-center px-6 pt-[clamp(48px,10vh,96px)] pb-12 max-[480px]:px-5 max-[480px]:pt-8">
      <section className="w-full max-w-[440px]">
        {/* Settings save as they change, so leaving needs no Save: the way back
            sits beside the wordmark rather than below every setting. */}
        <div className="flex items-center justify-between gap-4">
          <Wordmark />
          {account.username && (
            <a
              className="-my-2 -mr-2 inline-flex min-h-11 items-center gap-1.5 rounded-control px-2 text-[.85rem] text-content-muted no-underline transition-colors hover:text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2"
              href="/spaces"
            >
              <span aria-hidden="true">←</span> Back to spaces
            </a>
          )}
        </div>
        <p className="mt-14 text-[.7rem] font-bold tracking-[.14em] text-content-muted max-[480px]:mt-[42px]">
          {account.username ? "YOUR ACCOUNT" : "ONE LAST THING"}
        </p>
        <h1 className="my-5 text-[clamp(2.2rem,7vw,3.1rem)] leading-[1.08] font-bold tracking-[-.055em]">
          {account.username ? "Make it yours." : "Choose how you show up."}
        </h1>
        <p className="leading-[1.65] text-content-muted">
          Your username is unique. Your display name is what people see in conversations.
        </p>
        <ProfileForm account={account} onSaved={() => navigate({ to: "/spaces" })} />
        {account.username && <NotificationSettings />}
        {account.username && <PrivacySettings />}
        <div className="mt-8 flex justify-end border-t border-border pt-3 text-[.85rem]">
          <button
            className="-mr-2 min-h-11 cursor-pointer rounded-control border-0 bg-transparent px-2 text-content-muted transition-colors hover:text-content focus-visible:outline-2 focus-visible:outline-terracotta focus-visible:outline-offset-2"
            type="button"
            onClick={() => void logout().then(() => window.location.assign("/"))}
          >
            Log out
          </button>
        </div>
      </section>
    </main>
  );
}
