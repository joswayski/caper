package chat.caper.android.data

/**
 * Resends per email entry. With the first code that is three, the server's limit per
 * 15 minutes: it silently stops sending after that, so a fourth code could never verify.
 */
internal const val MAX_CODE_RESENDS = 2

/** "Resend code" waits this long after each code is sent, the first included. */
internal const val RESEND_COOLDOWN_MILLIS = 60_000L

/** Whole seconds until "Resend code" enables, rounded up; 0 once it can be pressed. */
internal fun resendSecondsLeft(sentAt: Long, now: Long): Int {
    val left = (sentAt + RESEND_COOLDOWN_MILLIS - now).coerceIn(0, RESEND_COOLDOWN_MILLIS)
    return ((left + 999) / 1_000).toInt()
}

/** `Resend code in 0:42` (m:ss) while waiting, then `Resend code`. */
internal fun resendLabel(secondsLeft: Int): String =
    if (secondsLeft <= 0) "Resend code" else "Resend code in ${secondsLeft / 60}:${(secondsLeft % 60).toString().padStart(2, '0')}"

internal const val RESENT_CODE_STATUS = "We sent a new code. Earlier codes no longer work."
internal const val RESEND_LIMIT_HINT = "Still nothing? Check your spam folder, or try again in 15 minutes."
