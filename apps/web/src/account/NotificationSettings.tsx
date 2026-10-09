import { useEffect, useId, useRef, useState } from "react";
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
  // Only the latest load (initial or a retry) may report an error; unmounting invalidates it.
  const loadRequest = useRef(0);

  const load = () => {
    const request = ++loadRequest.current;
    setError(undefined);
    void refreshNotificationSettings().catch(() => {
      if (request === loadRequest.current) setError("Your notification setting couldn’t load.");
    });
  };

  useEffect(() => {
    load();
    return () => {
      loadRequest.current++;
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
      {/* Only while loading: disabling during a save would drop focus from the chosen radio. */}
      <fieldset disabled={!settings.loaded} aria-busy={saving}>
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
          {!settings.loaded && (
            <button type="button" onClick={load}>
              Try again
            </button>
          )}
        </p>
      )}
    </section>
  );
}
