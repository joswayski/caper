package chat.caper.android

import java.io.File
import org.junit.Assert.*
import org.junit.Rule
import org.junit.Test
import org.junit.rules.TemporaryFolder

class ConversationDraftsTest {
    @get:Rule val folder = TemporaryFolder()

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

    @Test fun `drafts and thread replies survive a restart per account until sign-out`() {
        val dir = File(folder.root, "drafts")
        val before = ConversationDrafts().apply { open(dir, "alice123") }
        before["general"] = "half a thought"
        before[ConversationDrafts.thread("root1")] = "a reply"
        before["design"] = "sent"
        before["design"] = ""

        val after = ConversationDrafts().apply { open(dir, "alice123") }
        assertEquals("half a thought", after["general"])
        assertEquals("a reply", after[ConversationDrafts.thread("root1")])
        assertEquals("", after["design"])
        assertEquals("", after["root1"])
        assertEquals("another account never sees them", "", ConversationDrafts().apply { open(dir, "bob4567") }["general"])

        after.clear()
        after["general"] = "after sign-out"
        assertEquals("", ConversationDrafts().apply { open(dir, "alice123") }["general"])
        // Signing back in opens the account's (now empty) file again.
        after.open(dir, "alice123")
        after["general"] = "signed in again"
        assertEquals("signed in again", ConversationDrafts().apply { open(dir, "alice123") }["general"])
    }

    @Test fun `unsafe account IDs and corrupt files keep drafts in memory only`() {
        val dir = File(folder.root, "drafts")
        val escaping = ConversationDrafts().apply { open(dir, "../outside") }
        escaping["general"] = "kept in memory"
        assertEquals("kept in memory", escaping["general"])
        assertFalse(dir.exists())
        dir.mkdirs()
        File(dir, "alice123.json").writeText("not json")
        assertEquals("", ConversationDrafts().apply { open(dir, "alice123") }["general"])
    }
}
