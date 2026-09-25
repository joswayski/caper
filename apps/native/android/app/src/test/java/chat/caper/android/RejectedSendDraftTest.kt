package chat.caper.android

import org.junit.Assert.*
import org.junit.Test

class RejectedSendDraftTest {
    @Test fun `edit leaves unrelated next draft untouched`() {
        assertFalse(canEditRejectedMessage("next draft", "rejected message"))
        assertTrue(canEditRejectedMessage("", "rejected message"))
        assertTrue(canEditRejectedMessage("rejected message", "rejected message"))
    }
}
