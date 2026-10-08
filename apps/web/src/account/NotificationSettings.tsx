import { useEffect, useId, useState } from "react";
import type { NotificationLevel } from "../spaces/client.ts";
import { refreshNotificationSettings, setNotificationLevel, useNotificationSettings } from "../spaces/notifications.ts";
import "./privacy.css";

const choices: Array<{ value: NotificationLevel; label: string }> = [
  { value: "all", label: "All messages" },
  { value: "mentions", label: "Only @mentions and DMs" },
  { value: "nothing", label: "Nothing" },
];

/** The account notification level. Saves as you choose; phone delivery settings live on phones. */
export default function NotificationSettings() {
  const settings = useNotificationSettings();
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string>();
  const name = useId();

  useEffect(() => {
    let current = true;
    void refreshNotificationSettings().catch(() => {
      if (current) setError("Your notification setting couldn’t load. Reopen this to try again.");
    });
    return () => {
      current = false;
    };
  }, []);

  const choose = (level: NotificationLevel) => {
    if (saving || level === settings.level) return;
    setSaving(true);
    setError(undefined);
    void setNotificationLevel(level)
      .catch(() => setError("That change wasn’t saved. Try again."))
      .finally(() => setSaving(false));
  };

  return (
    <section className="privacy-settings notification-settings" aria-labelledby={`${name}-title`}>
      <h3 id={`${name}-title`}>Notifications</h3>
      <fieldset disabled={!settings.loaded || saving} aria-busy={saving}>
        <legend>Notify me about</legend>
        {choices.map((choice) => (
          <label key={choice.value}>
            <input
              type="radio"
              name={name}
              value={choice.value}
              checked={settings.loaded && settings.level === choice.value}
              onChange={() => choose(choice.value)}
            />
            <span>
              <strong>{choice.label}</strong>
            </span>
          </label>
        ))}
      </fieldset>
      {error && (
        <p className="privacy-error" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
