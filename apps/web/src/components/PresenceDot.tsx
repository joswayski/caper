import type { PresenceStatus } from "../gateway/client";
import "./presence-dot.css";

export default function PresenceDot({ status, live = true }: { status?: PresenceStatus; live?: boolean }) {
  const label = status ? `${status[0].toUpperCase()}${status.slice(1)}${live ? "" : " (last known; reconnecting)"}` : "Status unavailable";
  return <span className="presence-dot" data-status={status ?? "unknown"} role="img" aria-label={label} title={label} />;
}
