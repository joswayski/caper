import { EventConnection } from "./event-connection.ts";
import type { CallSnapshot } from "./types.ts";

/** Watch a channel's public voice roster, without participant track capabilities.
 * Account online/idle/offline presence lives in gateway/client.ts. */
export function watchPresence(
  callback: (snapshot: CallSnapshot) => void,
  status: (online: boolean) => void,
  apiRoot = "/api/media",
) {
  const owner = new AbortController();
  const stream = new EventConnection(() => undefined, () => undefined, callback, () => undefined,
    apiRoot, status);
  status(false);
  void stream.openPresence(owner.signal).catch(() => status(false));
  return () => { owner.abort(); stream.stop(); };
}
