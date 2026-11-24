import { useState, type ReactNode } from "react";
import { ExternalLink } from "lucide-react";
import Avatar from "../components/Avatar";
import AppearanceSettings from "../theme/AppearanceSettings";
import { sourceRevisionValid } from "../updates/whats-new";
import { logout, type Account } from "./client";
import NotificationSettings from "./NotificationSettings";
import PrivacySettings from "./PrivacySettings";
import "./privacy.css";

/** The one Settings surface, opened from the account bar's gear: Profile, Appearance,
 * Notifications, Voice & audio, Privacy & blocked accounts, About and Log out. Guests
 * see only Voice & audio and Sign in. The editors it opens stack above it, so closing
 * them returns here. */
export default function Settings({
  account,
  identityName,
  identityReady,
  voice,
  onEditProfile,
  onWhatsNew,
}: {
  account?: Account;
  identityName: string;
  identityReady: boolean;
  /** The voice and audio controls, which belong to the call client. */
  voice: ReactNode;
  onEditProfile: () => void;
  onWhatsNew?: () => void;
}) {
  const [loggingOut, setLoggingOut] = useState(false);
  const [logoutError, setLogoutError] = useState<string>();
  const member = !!account?.username;
  const revision = __SOURCE_REVISION__;
  return (
    <div className="settings-sections">
      {account && (
        <section className="privacy-settings settings-profile" aria-labelledby="settings-profile">
          <h3 id="settings-profile">Profile</h3>
          <div className="settings-profile-row">
            <span className="privacy-avatar settings-profile-avatar">
              <Avatar person={account} name={identityName} />
            </span>
            <span className="settings-profile-name">
              <strong>{identityName}</strong>
              {account.username && <small>@{account.username}</small>}
            </span>
            <button type="button" className="privacy-notification-enable" onClick={onEditProfile}>
              Edit profile
            </button>
          </div>
        </section>
      )}
      {member && (
        <section className="privacy-settings" aria-labelledby="settings-appearance">
          <h3 id="settings-appearance">Appearance</h3>
          <AppearanceSettings compact />
        </section>
      )}
      {member && <NotificationSettings />}
      <section className="privacy-settings settings-voice" aria-labelledby="settings-voice">
        <h3 id="settings-voice">Voice &amp; audio</h3>
        {voice}
      </section>
      {member && <PrivacySettings />}
      {account && (
        <section className="privacy-settings" aria-labelledby="settings-about">
          <h3 id="settings-about">About</h3>
          <p className="settings-version">
            {sourceRevisionValid(revision) ? (
              <a href={`https://github.com/joswayski/caper/commit/${revision}`} target="_blank" rel="noreferrer">
                Web build {revision.slice(0, 7)}
                <ExternalLink size={12} aria-hidden="true" />
              </a>
            ) : (
              "Development build"
            )}
          </p>
          {onWhatsNew && (
            <div className="settings-actions">
              <button type="button" className="privacy-notification-enable" onClick={onWhatsNew}>
                What’s new
              </button>
            </div>
          )}
        </section>
      )}
      {identityReady && (
        <section className="privacy-settings settings-logout">
          {account ? (
            <button
              type="button"
              className="privacy-notification-enable"
              disabled={loggingOut}
              onClick={() => {
                setLoggingOut(true);
                setLogoutError(undefined);
                logout().then(
                  () => window.location.assign("/"),
                  () => {
                    // Stay signed in and say so, instead of a click that silently did nothing.
                    setLoggingOut(false);
                    setLogoutError("Couldn’t log out. Check your connection and try again.");
                  },
                );
              }}
            >
              {loggingOut ? "Logging out…" : "Log out"}
            </button>
          ) : (
            <a className="privacy-notification-enable" href="/login">
              Sign in
            </a>
          )}
          {logoutError && (
            <p className="privacy-error" role="alert">
              {logoutError}
            </p>
          )}
        </section>
      )}
    </div>
  );
}
