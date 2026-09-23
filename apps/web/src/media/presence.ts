import { EventConnection } from "./event-connection.ts";
import type { CallSnapshot } from "./types.ts";

/** Read-only roster subscription; it does not join voice or renew anyone's lease. */
export function watchPresence(snapshot: (value: CallSnapshot) => void, live: (value: boolean) => void, apiRoot = "/api/media") {
  const owner = new AbortController();
  let stream: EventConnection | undefined;
  let timer: ReturnType<typeof setTimeout> | undefined;
  let failures = 0;
  let drainRetries = 0;
  const connect = () => {
    if (owner.signal.aborted) return;
    stream?.stop();
    const retry = (planned = false) => {
      if (owner.signal.aborted || stream !== current) return;
      live(false);
      current.stop();
      clearTimeout(timer);
      timer = setTimeout(connect, planned && drainRetries++ < 10 ? 50 : Math.min(250 * 2 ** Math.min(failures++, 5), 5_000));
    };
    const current = stream = new EventConnection(() => undefined, (_error, draining) => retry(draining), (value) => {
      if (owner.signal.aborted || stream !== current) return;
      failures = 0;
      drainRetries = 0;
      snapshot(value);
      live(true);
    }, () => retry(true), apiRoot);
    // Startup errors use the same lost callback as an established stream failure.
    void current.openPresence(owner.signal).catch(() => undefined);
  };
  live(false);
  connect();
  return () => {
    owner.abort();
    stream?.stop();
    clearTimeout(timer);
  };
}
