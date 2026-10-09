package chat.caper.android.data

import org.junit.Assert.assertEquals
import org.junit.Test

class CodeResendTest {
    @Test fun `resend waits a minute after each code, rounding up to whole seconds`() {
        val sentAt = 1_000_000L
        assertEquals(60, resendSecondsLeft(sentAt, sentAt))
        assertEquals(60, resendSecondsLeft(sentAt, sentAt + 1))
        assertEquals(59, resendSecondsLeft(sentAt, sentAt + 1_000))
        assertEquals(1, resendSecondsLeft(sentAt, sentAt + 59_999))
        assertEquals(0, resendSecondsLeft(sentAt, sentAt + 60_000))
        assertEquals(0, resendSecondsLeft(sentAt, sentAt + 3_600_000))
        assertEquals("a clock that went backwards never waits longer", 60, resendSecondsLeft(sentAt, sentAt - 5_000))
    }

    @Test fun `the label counts down in m ss`() {
        assertEquals("Resend code in 1:00", resendLabel(60))
        assertEquals("Resend code in 0:42", resendLabel(42))
        assertEquals("Resend code in 0:05", resendLabel(5))
        assertEquals("Resend code", resendLabel(0))
    }

    @Test fun `two resends make three codes, the server's limit`() {
        assertEquals(2, MAX_CODE_RESENDS)
        assertEquals("We sent a new code. Earlier codes no longer work.", RESENT_CODE_STATUS)
        assertEquals("Still nothing? Check your spam folder, or try again in 15 minutes.", RESEND_LIMIT_HINT)
    }
}
