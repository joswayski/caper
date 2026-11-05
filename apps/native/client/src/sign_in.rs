//! Sign-in code entry: the code alphabet and the "Resend code" cooldown,
//! shared by every desktop client's sign-in screen.

use std::time::Duration;

/// "Resend code" waits this long after each code is sent.
pub const RESEND_COOLDOWN: Duration = Duration::from_secs(60);
/// The API stops sending after three codes per 15 minutes, so a fourth would
/// never verify: one code plus at most two resends per email entry.
pub const MAX_RESENDS: u8 = 2;

/// Web's sign-in code cleanup: uppercase, drop anything outside the code
/// alphabet (spaces, dashes), then keep six characters.
pub fn clean_sign_in_code(code: &mut String) {
    code.make_ascii_uppercase();
    code.retain(|character| "ABCDEFGHJKMNPQRSTWXYZ23456789".contains(character));
    code.truncate(6);
}

/// How long "Resend code" still waits, `elapsed` after the latest code.
pub fn resend_wait(elapsed: Duration) -> Option<Duration> {
    RESEND_COOLDOWN
        .checked_sub(elapsed)
        .filter(|wait| !wait.is_zero())
}

/// "Resend code in 0:42" (whole seconds rounded up), and how long until the
/// shown second changes.
pub fn resend_countdown(wait: Duration) -> (String, Duration) {
    let seconds = wait.as_secs() + u64::from(wait.subsec_nanos() > 0);
    (
        format!("Resend code in {}:{:02}", seconds / 60, seconds % 60),
        wait.saturating_sub(Duration::from_secs(seconds.saturating_sub(1))),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resend_countdown_shows_minutes_and_seconds_rounded_up() {
        assert_eq!(resend_wait(Duration::ZERO), Some(RESEND_COOLDOWN));
        assert_eq!(resend_wait(RESEND_COOLDOWN), None);
        assert_eq!(resend_wait(Duration::from_secs(90)), None);
        let (label, changes) = resend_countdown(RESEND_COOLDOWN);
        assert_eq!(
            (label.as_str(), changes),
            ("Resend code in 1:00", Duration::from_secs(1))
        );
        let (label, changes) = resend_countdown(Duration::from_millis(41_250));
        assert_eq!(
            (label.as_str(), changes),
            ("Resend code in 0:42", Duration::from_millis(250))
        );
        assert_eq!(
            resend_countdown(Duration::from_millis(1)).0,
            "Resend code in 0:01"
        );
    }
}
