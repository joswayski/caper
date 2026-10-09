package chat.caper.android

import org.junit.Assert.*
import org.junit.Test

class ConversationDraftsTest {
    @Test fun `each conversation keeps its own draft until cleared or sign-out`() {
        val drafts = ConversationDrafts()
        drafts["general"] = "first draft"
        assertEquals("", drafts["design"])
        drafts["design"] = "design draft"
        assertEquals("first draft", drafts["general"])
        drafts["general"] = ""
        assertEquals("", drafts["general"])
        assertEquals("design draft", drafts["design"])
        drafts.clear()
        assertEquals("", drafts["design"])
    }
}
