import { useEffect, useRef, useState, type MouseEvent } from "react";
import { Bell, BellMinus, BellOff, BellRing, Check, ChevronRight } from "lucide-react";
import type { MutedUntil, NotificationChange, NotificationLevel } from "./client.ts";
import { levelLabels, muteLabel, mutePresets, muteUntil, refreshNotificationSettings } from "./notifications.ts";

type Submenu = "level" | "mute";

// Like the menu's other actions: close it and return focus to its button.
function closeMenu(event: MouseEvent<HTMLElement>) {
  const menu = event.currentTarget.closest("details");
  if (!menu) return;
  menu.open = false;
  menu.querySelector("summary")?.focus();
}

/** Re-reads the settings each time the surrounding menu opens; the choice lists start collapsed. */
function useSubmenu() {
  const ref = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState<Submenu>();
  useEffect(() => {
    const menu = ref.current?.closest("details");
    if (!menu) return;
    const toggled = () => {
      if (menu.open) void refreshNotificationSettings().catch(() => undefined);
      else setOpen(undefined);
    };
    menu.addEventListener("toggle", toggled);
    return () => menu.removeEventListener("toggle", toggled);
  }, []);
  return { ref, open, toggle: (next: Submenu) => setOpen((current) => (current === next ? undefined : next)) };
}

function MuteItems({
  noun,
  mutedUntil,
  open,
  disabled,
  onToggle,
  onChange,
}: {
  noun: string;
  mutedUntil: MutedUntil;
  open: boolean;
  disabled: boolean;
  onToggle: () => void;
  onChange: (change: { mutedUntil: MutedUntil }) => void;
}) {
  const label = muteLabel(mutedUntil);
  if (label)
    return (
      <button
        type="button"
        disabled={disabled}
        onClick={(event) => {
          closeMenu(event);
          onChange({ mutedUntil: null });
        }}
      >
        <Bell aria-hidden="true" />
        <span className="notification-item">
          Unmute {noun}
          <small>{label}</small>
        </span>
      </button>
    );
  return (
    <>
      <button type="button" disabled={disabled} aria-expanded={open} onClick={onToggle}>
        <BellOff aria-hidden="true" />
        <span className="notification-item">Mute {noun}</span>
        <ChevronRight className="notification-chevron" aria-hidden="true" />
      </button>
      {open && (
        <div className="notification-choices" role="group" aria-label={`Mute ${noun}`}>
          {mutePresets.map((preset) => (
            <button
              key={preset.label}
              type="button"
              onClick={(event) => {
                closeMenu(event);
                onChange({ mutedUntil: muteUntil(preset.minutes) });
              }}
            >
              {preset.label}
            </button>
          ))}
        </div>
      )}
    </>
  );
}

/** `Notifications` and `Mute` items for a space or channel menu. Disabled until settings load. */
export function LevelNotificationItems({
  noun,
  loaded,
  level,
  inherited,
  mutedUntil,
  mutedWithSpace = false,
  onChange,
}: {
  noun: "space" | "channel";
  loaded: boolean;
  level: NotificationLevel | null;
  /** What `Default` stands for here. */
  inherited: NotificationLevel;
  mutedUntil: MutedUntil;
  mutedWithSpace?: boolean;
  onChange: (change: NotificationChange) => void;
}) {
  const { ref, open, toggle } = useSubmenu();
  const choices: Array<NotificationLevel | null> = [null, "all", "mentions", "nothing"];
  const name = (value: NotificationLevel | null) =>
    value ? levelLabels[value] : `Default (${levelLabels[inherited]})`;
  return (
    <div ref={ref} className="notification-actions">
      <button type="button" disabled={!loaded} aria-expanded={open === "level"} onClick={() => toggle("level")}>
        <Bell aria-hidden="true" />
        <span className="notification-item">
          Notifications
          {/* Until settings load, `level` and `inherited` are guesses. */}
          <small>{loaded ? name(level) : "Loading…"}</small>
        </span>
        <ChevronRight className="notification-chevron" aria-hidden="true" />
      </button>
      {open === "level" && (
        <div className="notification-choices" role="group" aria-label="Notifications">
          {choices.map((value) => (
            <button
              key={value ?? "default"}
              type="button"
              aria-pressed={value === level}
              onClick={(event) => {
                closeMenu(event);
                if (value !== level) onChange({ level: value });
              }}
            >
              <Check aria-hidden="true" />
              {name(value)}
            </button>
          ))}
        </div>
      )}
      <MuteItems
        noun={noun}
        mutedUntil={mutedUntil}
        open={open === "mute"}
        disabled={!loaded}
        onToggle={() => toggle("mute")}
        onChange={onChange}
      />
      {loaded && mutedWithSpace && <p className="notification-note">Muted with the space</p>}
    </div>
  );
}

/** A DM row's items: notifications on or off, then mute. */
export function DirectNotificationItems({
  loaded,
  off,
  mutedUntil,
  onChange,
}: {
  loaded: boolean;
  off: boolean;
  mutedUntil: MutedUntil;
  onChange: (change: { level?: "nothing" | null; mutedUntil?: MutedUntil }) => void;
}) {
  const { ref, open, toggle } = useSubmenu();
  return (
    <div ref={ref} className="notification-actions">
      <button
        type="button"
        disabled={!loaded}
        onClick={(event) => {
          closeMenu(event);
          onChange({ level: off ? null : "nothing" });
        }}
      >
        {off ? <BellRing aria-hidden="true" /> : <BellMinus aria-hidden="true" />}
        <span className="notification-item">{off ? "Turn on notifications" : "Turn off notifications"}</span>
      </button>
      <MuteItems
        noun="conversation"
        mutedUntil={mutedUntil}
        open={open === "mute"}
        disabled={!loaded}
        onToggle={() => toggle("mute")}
        onChange={onChange}
      />
    </div>
  );
}
