//! Consecutive messages from the same person read as one block: a row is
//! "grouped" (no avatar or name/time header) when the previous visible row in
//! the same list is by the same account, at most five minutes away, on the
//! same local day. Presentation only: ordering, paging, unread state and what
//! actions target never change. Shared rules: docs/media.md, "Message text:
//! links and grouping".
//!
//! Callers keep the previous row and forget it (pass `None`) at anything that
//! separates rows: a date divider, a blocked-messages placeholder, a thread
//! root above its replies, or the start of a list.

use chrono::{DateTime, Local, TimeZone};

/// At most this far apart (either direction) to share a header.
const WINDOW_MILLISECONDS: i64 = 5 * 60 * 1_000;

/// What grouping needs to know about one visible message row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// The author's account id; a pending own message uses yours.
    pub author: String,
    /// RFC 3339.
    pub created_at: String,
    /// Always keeps its header, like a "Replied to a thread" broadcast in the
    /// timeline. It may still head a group for the rows after it.
    pub header: bool,
}

impl Row {
    pub fn new(author: &str, created_at: &str, header: bool) -> Self {
        Self {
            author: author.to_owned(),
            created_at: created_at.to_owned(),
            header,
        }
    }
}

/// Whether `current` is drawn compactly under `previous`, in local time.
pub fn grouped(previous: Option<&Row>, current: &Row) -> bool {
    grouped_in(previous, current, &Local)
}

pub fn grouped_in<Tz: TimeZone>(previous: Option<&Row>, current: &Row, timezone: &Tz) -> bool {
    let Some(previous) = previous else {
        return false;
    };
    if current.header || previous.author != current.author {
        return false;
    }
    let (Ok(before), Ok(after)) = (
        DateTime::parse_from_rfc3339(&previous.created_at),
        DateTime::parse_from_rfc3339(&current.created_at),
    ) else {
        return false;
    };
    (after - before).num_milliseconds().abs() <= WINDOW_MILLISECONDS
        && before.with_timezone(timezone).date_naive() == after.with_timezone(timezone).date_naive()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn row(author: &str, created_at: &str) -> Row {
        Row::new(author, created_at, false)
    }

    #[test]
    fn same_author_within_five_minutes_groups() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let first = row("maya", "2026-09-23T09:41:00Z");
        assert!(grouped_in(
            Some(&first),
            &row("maya", "2026-09-23T09:42:00Z"),
            &utc
        ));
        // Exactly five minutes still groups; a millisecond more does not.
        assert!(grouped_in(
            Some(&first),
            &row("maya", "2026-09-23T09:46:00Z"),
            &utc
        ));
        assert!(!grouped_in(
            Some(&first),
            &row("maya", "2026-09-23T09:46:00.001Z"),
            &utc
        ));
        // Clock skew: an earlier timestamp counts by its distance.
        assert!(grouped_in(
            Some(&first),
            &row("maya", "2026-09-23T09:38:00Z"),
            &utc
        ));
        // Offsets are compared as instants.
        assert!(grouped_in(
            Some(&first),
            &row("maya", "2026-09-23T11:42:00+02:00"),
            &utc
        ));
    }

    #[test]
    fn first_rows_other_authors_and_headers_never_group() {
        let utc = FixedOffset::east_opt(0).unwrap();
        let first = row("maya", "2026-09-23T09:41:00Z");
        assert!(!grouped_in(None, &first, &utc), "a list's first row");
        assert!(!grouped_in(
            Some(&first),
            &row("alex", "2026-09-23T09:42:00Z"),
            &utc
        ));
        assert!(
            !grouped_in(
                Some(&first),
                &Row::new("maya", "2026-09-23T09:42:00Z", true),
                &utc
            ),
            "a broadcast keeps its header"
        );
        // ...but the row after a broadcast may group under it.
        assert!(grouped_in(
            Some(&Row::new("maya", "2026-09-23T09:41:00Z", true)),
            &row("maya", "2026-09-23T09:42:00Z"),
            &utc
        ));
        assert!(!grouped_in(
            Some(&first),
            &row("maya", "not a timestamp"),
            &utc
        ));
    }

    #[test]
    fn a_local_day_change_breaks_the_group() {
        let late = row("maya", "2026-09-23T23:58:00Z");
        let early = row("maya", "2026-09-24T00:01:00Z");
        let utc = FixedOffset::east_opt(0).unwrap();
        assert!(!grouped_in(Some(&late), &early, &utc), "midnight in UTC");
        // Three hours behind UTC, both are on the evening of the 23rd.
        let west = FixedOffset::west_opt(3 * 3_600).unwrap();
        assert!(grouped_in(Some(&late), &early, &west));
    }
}
