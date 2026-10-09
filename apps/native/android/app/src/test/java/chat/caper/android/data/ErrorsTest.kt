package chat.caper.android.data

import java.io.IOException
import java.net.ConnectException
import java.net.SocketTimeoutException
import java.net.UnknownHostException
import org.junit.Assert.assertEquals
import org.junit.Test

class ErrorsTest {
    @Test fun `known server errors read as web's sentences`() {
        assertEquals("A channel with that name already exists.", friendlyError(ApiException(409, "channel name already exists")))
        assertEquals(
            "This person needs to join the space before you can add them to a channel.",
            friendlyError(ApiException(409, "user must join the space first")),
        )
        assertEquals("This person is already in the space.", friendlyError(ApiException(409, "user already in space")))
        assertEquals("That’s no longer available.", friendlyError(ApiException(404, "resource not found")))
    }

    @Test fun `network failures and timeouts don't show the platform's text`() {
        assertEquals("Couldn’t reach Caper. Check your connection.", friendlyError(UnknownHostException("Unable to resolve host \"caper.chat\"")))
        assertEquals("Couldn’t reach Caper. Check your connection.", friendlyError(ConnectException("Failed to connect")))
        assertEquals("Couldn’t reach Caper. Check your connection.", friendlyError(IOException("unexpected end of stream")))
        assertEquals("That took too long. Try again.", friendlyError(SocketTimeoutException("timeout")))
        assertEquals("That took too long. Try again.", friendlyError(java.io.InterruptedIOException("timeout")))
    }

    @Test fun `unknown text is capitalized and punctuated, readable text is kept`() {
        assertEquals("Too many things. Try again later.", friendlyError(ApiException(429, "too many things; try again later")))
        assertEquals("This channel is no longer accessible.", friendlyError(IllegalStateException("This channel is no longer accessible.")))
        assertEquals("Request failed (503).", friendlyError(ApiException(503, "Request failed (503).")))
        assertEquals("That didn’t work. Try again.", friendlyError(IllegalStateException()))
        assertEquals("That didn’t work. Try again.", friendlyError(null))
        assertEquals("Couldn’t load this setting.", friendlyError(IllegalStateException(" "), "Couldn’t load this setting."))
    }

    @Test fun `display mapping leaves the exception for logic`() {
        val error = ApiException(404, "resource not found", "dm_not_accepted")
        friendlyError(error)
        assertEquals("resource not found", error.message)
        assertEquals("dm_not_accepted", error.code)
    }
}
