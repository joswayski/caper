import { useState } from "react";
import { Moon } from "lucide-react";
import type { MutedUntil } from "../spaces/client.ts";
import {
  isMuted,
  pauseLabel,
  pausePresets,
  pauseUntil,
  setPausedUntil,
  useNotificationSettings,
} from "../spaces/notifications.ts";

/** Do Not Disturb as the account bar and Settings show it. The store re-renders when it ends. */
export function useDoNotDisturb() {
  const settings = useNotificationSettings();
  const on = settings.loaded && isMuted(settings.pausedUntil);
  return { loaded: settings.loaded, on, until: on ? pauseLabel(settings.pausedUntil) : undefined };
}

function useSave(onSaved?: () => void) {
  const [error, setError] = useState<string>();
  const [saving, setSaving] = useState(false);
  const save = (pausedUntil: MutedUntil) => {
    if (saving) return;
    setSaving(true);
    setError(undefined);
    void setPausedUntil(pausedUntil).then(
      () => {
        setSaving(false);
        onSaved?.();
      },
      () => {
        setSaving(false);
        setError("That change wasn’t saved. Try again.");
      },
    );
  };
  return { error, saving, save };
}

/** The moon over your avatar while Do Not Disturb is on. */
export function DoNotDisturbBadge() {
  return (
    <span className="dnd-badge" aria-hidden="true">
      <Moon />
    </span>
  );
}

/** Settings → Notifications: status, the presets and Turn off. */
export function DoNotDisturbSettings() {
  const { loaded, on, until } = useDoNotDisturb();
  const { error, saving, save } = useSave();
  return (
    <fieldset className="dnd-settings" disabled={!loaded} aria-busy={saving}>
      <legend>Do Not Disturb</legend>
      <p className="dnd-status" role="status">
        {on ? (
          <>
            <Moon aria-hidden="true" />
            <strong>On</strong> · {until}
          </>
        ) : (
          "Off"
        )}
      </p>
      <small>Pauses notifications on all your devices. Unread counts still update.</small>
      <div className="dnd-presets" role="group" aria-label="Pause notifications for">
        {pausePresets.map((preset) => (
          <button key={preset.label} type="button" onClick={() => save(pauseUntil(preset))}>
            {preset.label}
          </button>
        ))}
        {on && (
          <button type="button" className="dnd-turn-off" onClick={() => save(null)}>
            Turn off
          </button>
        )}
      </div>
      {error && (
        <p className="privacy-error" role="alert">
          {error}
        </p>
      )}
    </fieldset>
  );
}

/** The account menu's quick toggle; the menu closes once a choice is saved. */
export function DoNotDisturbMenu({ onDone }: { onDone: () => void }) {
  const { loaded, on, until } = useDoNotDisturb();
  const { error, saving, save } = useSave(onDone);
  return (
    <div className="dnd-menu" role="group" aria-label="Do Not Disturb" aria-busy={saving}>
      <strong>{on ? `Do Not Disturb · ${until}` : "Pause notifications"}</strong>
      {on && (
        <button type="button" disabled={saving} onClick={() => save(null)}>
          Turn off Do Not Disturb
        </button>
      )}
      {pausePresets.map((preset) => (
        <button key={preset.label} type="button" disabled={!loaded || saving} onClick={() => save(pauseUntil(preset))}>
          {preset.label}
        </button>
      ))}
      {error && (
        <p className="dnd-error" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
