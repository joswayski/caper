//! Your messages on their way. Each conversation sends one at a time, in
//! the order you sent them, so a message typed while an earlier one is in
//! flight waits for it instead of racing it or being refused. A send whose
//! outcome is unknown (a network drop, a timeout, a 5xx) stays first and goes
//! again unchanged, with the same client message ID: at once when the
//! gateway reconnects or you retry, and by itself after [`AUTO_RETRY`] while
//! the gateway stays live.

use crate::worker::{Outgoing, SendFailure};
use std::time::{Duration, Instant};

/// After a failure with an unknown outcome, while the gateway stays live, a
/// send or upload goes again by itself this long after each failure. After
/// the last one fails it waits for you or for a reconnect.
pub const AUTO_RETRY: [Duration; 3] = [
    Duration::from_secs(3),
    Duration::from_secs(10),
    Duration::from_secs(30),
];

/// The automatic retries of one send or upload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Backoff {
    retries: usize,
    due: Option<Instant>,
}

impl Backoff {
    /// An attempt failed with an unknown outcome at `now`: the next
    /// automatic retry is due, if the gateway is live and one is left.
    pub fn failed(&mut self, live: bool, now: Instant) {
        self.due = AUTO_RETRY
            .get(self.retries)
            .filter(|_| live)
            .map(|delay| now + *delay);
    }

    /// Whether the retry is due at `now`, counting it if so. One that comes
    /// due while the gateway is down is dropped: the reconnect retries.
    pub fn take_due(&mut self, live: bool, now: Instant) -> bool {
        if self.due.is_none_or(|due| due > now) {
            return false;
        }
        self.due = None;
        if live {
            self.retries += 1;
        }
        live
    }

    /// It goes again for another reason (a reconnect, or you): the next
    /// failure starts the schedule over.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// When the next automatic retry is due.
    pub fn due(&self) -> Option<Instant> {
        self.due
    }
}

/// What a queued send carries besides its request, such as its draft row.
pub trait Queued {
    fn outgoing(&self) -> &Outgoing;
}

impl Queued for Outgoing {
    fn outgoing(&self) -> &Outgoing {
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Attempt {
    /// Behind an earlier send in its conversation.
    Waiting,
    Sending,
    /// The last attempt's outcome is unknown: it goes again.
    Unconfirmed,
}

struct Entry<T> {
    item: T,
    attempt: Attempt,
    backoff: Backoff,
}

/// A send's result, for the one it belonged to.
pub struct Settled<T> {
    /// Confirmed or refused: done with. None when the outcome is unknown:
    /// it stays first and goes again.
    pub done: Option<T>,
    /// The conversation's next send, to send now.
    pub next: Option<Outgoing>,
}

/// Every send on its way, oldest first.
pub struct Sends<T> {
    entries: Vec<Entry<T>>,
}

impl<T> Default for Sends<T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl<T: Queued> Sends<T> {
    /// Queues `item`, returning its request when it goes now: nothing
    /// earlier in its conversation is still on its way.
    pub fn add(&mut self, item: T) -> Option<Outgoing> {
        let channel = &item.outgoing().channel;
        let first = !self
            .entries
            .iter()
            .any(|entry| entry.item.outgoing().channel == *channel);
        let outgoing = first.then(|| item.outgoing().clone());
        self.entries.push(Entry {
            item,
            attempt: if first {
                Attempt::Sending
            } else {
                Attempt::Waiting
            },
            backoff: Backoff::default(),
        });
        outgoing
    }

    /// The result of `client_id`'s send (`Ok` also for its gateway echo).
    /// `None` when it is not on its way, such as a result after its echo.
    pub fn settle(
        &mut self,
        client_id: &str,
        result: Result<(), &SendFailure>,
        live: bool,
        now: Instant,
    ) -> Option<Settled<T>> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.item.outgoing().client_id == client_id)?;
        if result.is_err_and(|failure| !failure.rejected()) {
            let entry = &mut self.entries[index];
            entry.attempt = Attempt::Unconfirmed;
            entry.backoff.failed(live, now);
            return Some(Settled {
                done: None,
                next: None,
            });
        }
        let item = self.entries.remove(index).item;
        let channel = &item.outgoing().channel;
        let next = self
            .entries
            .iter_mut()
            .find(|entry| entry.item.outgoing().channel == *channel)
            .filter(|entry| entry.attempt == Attempt::Waiting)
            .map(|entry| {
                entry.attempt = Attempt::Sending;
                entry.item.outgoing().clone()
            });
        Some(Settled {
            done: Some(item),
            next,
        })
    }

    /// Sends whose outcome is unknown go again now (in `channel`, or
    /// everywhere): the gateway reconnected or you retried.
    pub fn resend(&mut self, channel: Option<&str>) -> Vec<Outgoing> {
        self.entries
            .iter_mut()
            .filter(|entry| {
                entry.attempt == Attempt::Unconfirmed
                    && channel.is_none_or(|channel| entry.item.outgoing().channel == channel)
            })
            .map(|entry| {
                entry.attempt = Attempt::Sending;
                entry.backoff.reset();
                entry.item.outgoing().clone()
            })
            .collect()
    }

    /// Sends whose automatic retry is due at `now`, to send again.
    pub fn due(&mut self, live: bool, now: Instant) -> Vec<Outgoing> {
        self.entries
            .iter_mut()
            .filter_map(|entry| {
                (entry.attempt == Attempt::Unconfirmed && entry.backoff.take_due(live, now)).then(
                    || {
                        entry.attempt = Attempt::Sending;
                        entry.item.outgoing().clone()
                    },
                )
            })
            .collect()
    }

    /// When the next automatic retry is due.
    pub fn next_due(&self) -> Option<Instant> {
        self.entries
            .iter()
            .filter_map(|entry| entry.backoff.due())
            .min()
    }

    /// Whether a send's outcome is unknown, waiting to go again.
    pub fn unconfirmed(&self) -> bool {
        self.entries
            .iter()
            .any(|entry| entry.attempt == Attempt::Unconfirmed)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Oldest first.
    pub fn iter(&self) -> impl Iterator<Item = &T> {
        self.entries.iter().map(|entry| &entry.item)
    }

    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut T> {
        self.entries.iter_mut().map(|entry| &mut entry.item)
    }

    /// Every send, oldest first, with whether its request is in flight, as
    /// when leaving the conversation.
    pub fn take(&mut self) -> Vec<(T, bool)> {
        std::mem::take(&mut self.entries)
            .into_iter()
            .map(|entry| (entry.item, entry.attempt == Attempt::Sending))
            .collect()
    }

    /// The results of requests in flight will not arrive (their conversation
    /// was left): their outcome is unknown, so they go again on
    /// [`Sends::resend`] and the sends behind them follow.
    pub fn orphan(&mut self) {
        for entry in &mut self.entries {
            if entry.attempt == Attempt::Sending {
                entry.attempt = Attempt::Unconfirmed;
                entry.backoff.reset();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failure(status: Option<u16>) -> SendFailure {
        SendFailure {
            status,
            message: "failed".into(),
            code: None,
        }
    }

    fn ids(sends: &[Outgoing]) -> Vec<&str> {
        sends.iter().map(|send| send.text.as_str()).collect()
    }

    #[test]
    fn a_message_sent_while_another_is_in_flight_goes_after_it_in_order() {
        let mut sends = Sends::default();
        let first = Outgoing::new("c".into(), "first".into());
        let second = Outgoing::new("c".into(), "second".into());
        let third = Outgoing::new("c".into(), "third".into());
        let elsewhere = Outgoing::new("d".into(), "elsewhere".into());
        assert_eq!(
            sends.add(first.clone()).map(|o| o.text),
            Some("first".into())
        );
        // Neither is dropped; both wait for the first.
        assert_eq!(sends.add(second.clone()), None);
        assert_eq!(sends.add(third.clone()), None);
        // Another conversation does not wait.
        assert!(sends.add(elsewhere).is_some());
        let now = Instant::now();
        // A network drop holds the line: the first goes again before the second.
        assert!(
            sends
                .settle(&first.client_id, Err(&failure(None)), true, now)
                .is_some_and(|settled| settled.done.is_none() && settled.next.is_none())
        );
        assert_eq!(ids(&sends.resend(Some("c"))), ["first"]);
        let Some(Settled {
            done: Some(item),
            next,
        }) = sends.settle(&first.client_id, Ok(()), true, now)
        else {
            panic!("the first send is done");
        };
        assert_eq!(item.text, "first");
        assert_eq!(next.map(|o| o.client_id), Some(second.client_id.clone()));
        // A refusal is done with too: the next one still goes.
        let Some(Settled {
            done: Some(_),
            next,
        }) = sends.settle(&second.client_id, Err(&failure(Some(403))), true, now)
        else {
            panic!("a refused send is done");
        };
        assert_eq!(next.map(|o| o.client_id), Some(third.client_id.clone()));
        // Its result after its gateway echo confirmed it belongs to nothing.
        assert!(sends.settle(&third.client_id, Ok(()), true, now).is_some());
        assert!(sends.settle(&third.client_id, Ok(()), true, now).is_none());
    }

    #[test]
    fn an_unconfirmed_send_retries_itself_after_3_10_then_30_seconds_while_live() {
        let mut sends = Sends::default();
        let send = Outgoing::new("c".into(), "hello".into());
        sends.add(send.clone());
        let mut now = Instant::now();
        for (delay, status) in AUTO_RETRY.into_iter().zip([None, Some(502), Some(503)]) {
            sends.settle(&send.client_id, Err(&failure(status)), true, now);
            assert_eq!(sends.next_due(), Some(now + delay));
            assert!(
                sends
                    .due(true, now + delay - Duration::from_millis(1))
                    .is_empty()
            );
            now += delay;
            // The same request: the same client message ID and text.
            assert_eq!(sends.due(true, now), vec![send.clone()]);
        }
        // The third retry failed too: it waits for you or a reconnect.
        sends.settle(&send.client_id, Err(&failure(None)), true, now);
        assert_eq!(sends.next_due(), None);
        assert!(sends.due(true, now + Duration::from_secs(3600)).is_empty());
        assert!(sends.unconfirmed());
        // A reconnect sends it at once and starts the schedule over.
        assert_eq!(sends.resend(None), vec![send.clone()]);
        sends.settle(&send.client_id, Err(&failure(None)), true, now);
        assert_eq!(sends.next_due(), Some(now + AUTO_RETRY[0]));
    }

    #[test]
    fn without_a_live_gateway_an_unconfirmed_send_waits_for_the_reconnect() {
        let mut sends = Sends::default();
        let send = Outgoing::new("c".into(), "hello".into());
        sends.add(send.clone());
        let now = Instant::now();
        sends.settle(&send.client_id, Err(&failure(None)), false, now);
        assert_eq!(sends.next_due(), None);
        // Live at the failure, down when the retry comes due: no attempt.
        sends.resend(None);
        sends.settle(&send.client_id, Err(&failure(None)), true, now);
        assert!(sends.due(false, now + AUTO_RETRY[0]).is_empty());
        assert_eq!(sends.next_due(), None);
        assert_eq!(sends.resend(None), [send]);
    }
}
