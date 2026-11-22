package chat.caper.android.data

import java.io.IOException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SendRetryTest {
    @Test fun `an unconfirmed send retries after 3, 10 and 30 seconds, then waits for Retry send`() {
        assertEquals(listOf(3_000L, 10_000L, 30_000L), (0..2).map { sendRetryDelayMs(it) })
        assertNull(sendRetryDelayMs(3))
    }

    @Test fun `only network errors, timeouts and server failures retry by themselves`() {
        assertTrue(transientSendFailure(IOException("reset")))
        assertTrue(transientSendFailure(SocketTimeoutException("timeout")))
        assertTrue(transientSendFailure(UnknownHostException("caper.chat")))
        for (status in listOf(500, 502, 503, 429)) assertTrue("$status", transientSendFailure(ApiException(status, "busy")))
        // A refusal gives the same answer every time.
        for (status in listOf(400, 401, 403, 404, 409, 413, 422)) assertFalse("$status", transientSendFailure(ApiException(status, "no")))
        // An upload interrupted or failed by storage retries; a file problem does not.
        assertTrue(transientSendFailure(UploadException("The upload was interrupted.", transient = true)))
        assertFalse(transientSendFailure(UploadException("This file is empty.")))
        assertFalse(transientSendFailure(IllegalStateException("Chat session is unavailable.")))
    }

    @Test fun `queued messages return to the composer after its text when the conversation closes`() {
        assertEquals("draft\n\nfirst\n\nsecond", restoredDraft("draft", listOf("first", "second")))
        assertEquals("first", restoredDraft("", listOf("first")))
        // A file-only queued message adds no blank paragraph.
        assertEquals("draft", restoredDraft("draft", listOf("")))
    }
}
