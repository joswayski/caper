/**
 * Timer polls for sidebar state the gateway also reports. While the account's
 * notification feed is live it reports activity within a second, so a poll only
 * catches what never notifies (muted conversations, membership, a call ending)
 * and runs once a minute. Without the feed the poll is the only source and runs
 * every 15 seconds.
 */
export const POLL_MS = 15_000;
export const LIVE_POLL_MS = 60_000;

let live = false;

/** Reported by the notification feed's subscription, once per tab. */
export function setAccountFeedLive(next: boolean) {
  live = next;
}

export function shellPoll(run: () => void): () => void {
  let last = Date.now();
  const timer = setInterval(() => {
    // Timers drift: a tick slightly short of the minute still counts as due.
    if (live && Date.now() - last < LIVE_POLL_MS - POLL_MS / 2) return;
    last = Date.now();
    run();
  }, POLL_MS);
  return () => clearInterval(timer);
}
