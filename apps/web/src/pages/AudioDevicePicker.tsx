import { useId, useLayoutEffect, useRef, useState } from "react";
import { Bluetooth, ChevronRight, Headphones, Headset, Mic, Monitor, Speaker, X } from "lucide-react";

export function audioDeviceOptions(devices: MediaDeviceInfo[], input: boolean) {
  const matching = devices
    .filter((device) => device.kind === (input ? "audioinput" : "audiooutput"))
    .map((device) => ({
      deviceId: device.deviceId,
      // Chromium appends USB vendor/product IDs; keep model names and routing IDs intact.
      label: device.label.replace(/\s*\([0-9a-f]{4}:[0-9a-f]{4}\)\s*$/i, ""),
    }));
  const defaultName = matching
    .find((device) => device.deviceId === "default")
    ?.label.replace(/^Default\s*[-–]\s*/i, "");
  return [
    { id: "", label: "System default", detail: defaultName || "Follows system sound settings" },
    ...matching
      .filter((device) => device.deviceId && device.deviceId !== "default")
      .map((device) => ({
        id: device.deviceId,
        label: device.label || (input ? "Microphone" : "Audio output"),
        detail: "",
      })),
  ];
}

// Browsers expose labels, not transport/device types. Only classify explicit
// evidence; an unfamiliar model name is not proof of Bluetooth or a headset.
function DeviceIcon({ label, input }: { label: string; input: boolean }) {
  const Icon = /bluetooth|\bBT\b/i.test(label)
    ? Bluetooth
    : /headset/i.test(label)
      ? Headset
      : /headphones|earbuds|airpods/i.test(label)
        ? Headphones
        : !input && /HDMI|displayport|monitor/i.test(label)
          ? Monitor
          : input
            ? Mic
            : Speaker;
  return <Icon aria-hidden="true" />;
}

export default function AudioDevicePicker({
  title,
  devices,
  input = false,
  value,
  disabled,
  onChange,
}: {
  title: string;
  devices: MediaDeviceInfo[];
  input?: boolean;
  value: string;
  disabled?: boolean;
  onChange: (id: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const root = useRef<HTMLDivElement>(null);
  const id = useId();
  const options = audioDeviceOptions(devices, input);
  const selected = options.find((option) => option.id === value) ?? options[0]!;
  const close = () => {
    setOpen(false);
    trigger.current?.focus();
  };
  useLayoutEffect(() => {
    const popup = panel.current;
    const button = trigger.current;
    if (!open || !popup || !button) return;
    popup.showPopover();
    const viewport = window.visualViewport;
    const position = () => {
      const bounds = button.getBoundingClientRect();
      const parent = button.closest(".call-settings-panel")?.getBoundingClientRect() ?? bounds;
      const width = viewport?.width ?? window.innerWidth;
      const height = viewport?.height ?? window.innerHeight;
      const x = viewport?.offsetLeft ?? 0;
      const y = viewport?.offsetTop ?? 0;
      const panelWidth = Math.min(300, width - 16);
      popup.style.width = `${panelWidth}px`;
      popup.style.maxHeight = `${Math.min(360, height - 16)}px`;
      const left = parent.right + 8 + panelWidth <= x + width - 8 ? parent.right + 8 : parent.left - panelWidth - 8;
      popup.style.left = `${Math.max(x + 8, left)}px`;
      popup.style.top = `${width < 640 ? y + height - popup.offsetHeight - 8 : Math.max(y + 8, Math.min(bounds.top, y + height - popup.offsetHeight - 8))}px`;
    };
    position();
    popup.querySelector<HTMLElement>('[aria-checked="true"]')?.focus();
    const observer = new ResizeObserver(position);
    observer.observe(popup);
    const dismiss = (event: Event) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    document.addEventListener("pointerdown", dismiss);
    document.addEventListener("focusin", dismiss);
    window.addEventListener("resize", position);
    window.addEventListener("scroll", position, true);
    viewport?.addEventListener("resize", position);
    viewport?.addEventListener("scroll", position);
    return () => {
      observer.disconnect();
      document.removeEventListener("pointerdown", dismiss);
      document.removeEventListener("focusin", dismiss);
      window.removeEventListener("resize", position);
      window.removeEventListener("scroll", position, true);
      viewport?.removeEventListener("resize", position);
      viewport?.removeEventListener("scroll", position);
    };
  }, [open]);
  return (
    <div
      ref={root}
      className="audio-device-picker"
      onKeyDown={(event) => {
        if (open && event.key === "Escape") {
          event.preventDefault();
          event.stopPropagation();
          close();
        }
      }}
    >
      <button
        ref={trigger}
        type="button"
        className="audio-device-summary"
        disabled={disabled}
        aria-label={title}
        aria-describedby={`${id}-summary`}
        aria-haspopup="menu"
        aria-expanded={open}
        aria-controls={open ? id : undefined}
        onClick={() => setOpen(!open)}
      >
        <DeviceIcon label={selected.detail || selected.label} input={input} />
        <span>
          <strong>{title}</strong>
          <small id={`${id}-summary`}>
            {selected.label}
            {selected.id === "" && selected.detail !== "Follows system sound settings" ? ` — ${selected.detail}` : ""}
          </small>
        </span>
        <ChevronRight aria-hidden="true" />
      </button>
      {open && (
        <div ref={panel} id={id} popover="manual" className="audio-device-drawer">
          <header>
            <strong>{title}</strong>
            <button type="button" aria-label="Close device picker" onClick={close}>
              <X aria-hidden="true" />
            </button>
          </header>
          <div
            className="audio-device-choices"
            role="menu"
            aria-label={title}
            onKeyDown={(event) => {
              const radios = Array.from(
                event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="menuitemradio"]'),
              );
              const current = radios.indexOf(document.activeElement as HTMLButtonElement);
              const index =
                event.key === "Home"
                  ? 0
                  : event.key === "End"
                    ? radios.length - 1
                    : event.key === "ArrowDown" || event.key === "ArrowRight"
                      ? (current + 1) % radios.length
                      : event.key === "ArrowUp" || event.key === "ArrowLeft"
                        ? (current - 1 + radios.length) % radios.length
                        : -1;
              if (index >= 0) {
                event.preventDefault();
                radios[index]?.focus();
              }
            }}
          >
            {options.map((option) => (
              <button
                key={option.id}
                type="button"
                role="menuitemradio"
                aria-checked={selected.id === option.id}
                tabIndex={selected.id === option.id ? 0 : -1}
                className="audio-device-choice"
                onClick={() => {
                  onChange(option.id);
                  close();
                }}
              >
                <span className="audio-device-radio" aria-hidden="true" />
                <DeviceIcon label={option.detail || option.label} input={input} />
                <span>
                  {option.label}
                  {option.detail && <small>{option.detail}</small>}
                </span>
              </button>
            ))}
          </div>
        </div>
      )}
    </div>
  );
}
