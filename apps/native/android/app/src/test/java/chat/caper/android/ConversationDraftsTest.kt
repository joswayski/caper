package chat.caper.android

import chat.caper.android.model.ServerDraft
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

    private fun server(channel: String, text: String, at: String, root: String? = null) =
        ServerDraft(channel, root, text, "2026-11-27T18:04:${at}Z")

    @Test fun `a server draft applies when clean and newer, and an older one is ignored`() {
        val drafts = ConversationDrafts()
        assertTrue(drafts.apply(server("channel00001", "from the web", "05.000001")))
        assertEquals("from the web", drafts["channel00001"])
        assertFalse("older", drafts.apply(server("channel00001", "stale", "04.999999")))
        assertFalse("the same one again", drafts.apply(server("channel00001", "stale", "05.000001")))
        assertEquals("from the web", drafts["channel00001"])
        assertTrue(drafts.apply(server("channel00001", "", "06.000000")))
        assertEquals("cleared elsewhere", "", drafts["channel00001"])
        assertTrue(drafts.apply(server("channel00001", "reply text", "07.000000", root = "message00000001")))
        assertEquals("reply text", drafts[ConversationDrafts.thread("message00000001")])
        assertEquals("", drafts["channel00001"])
        // A cleared draft remembers when, so an older copy still on its way can't bring sent text back.
        assertFalse(drafts.apply(server("channel00001", "sent long ago", "05.500000")))
        assertEquals("", drafts["channel00001"])
    }

    @Test fun `a dirty draft is never overwritten until its save lands, and edits since stay dirty`() {
        val edits = mutableListOf<Pair<String, Boolean>>()
        val drafts = ConversationDrafts().apply { edited = { key, now -> edits += key to now } }
        drafts["channel00001"] = "typing here"
        assertEquals(listOf("channel00001" to false), edits)
        assertFalse(drafts.apply(server("channel00001", "from the phone", "09.000000")))
        assertEquals("typing here", drafts["channel00001"])
        val save = requireNotNull(drafts.pendingSave("channel00001"))
        assertEquals(DraftSave("channel00001", "channel00001", null, "typing here"), save)
        drafts["channel00001"] = "typing here, more"
        drafts.saved(save, server("channel00001", "typing here", "10.000000"))
        assertEquals("edited while saving", listOf("channel00001"), drafts.dirtyKeys())
        assertEquals("typing here, more", drafts.pendingSave("channel00001")?.text)
        val next = requireNotNull(drafts.pendingSave("channel00001"))
        drafts.saved(next, server("channel00001", "typing here, more", "11.000000"))
        assertNull(drafts.pendingSave("channel00001"))
        assertTrue(drafts.dirtyKeys().isEmpty())
        // Clean again: a newer copy from another device applies.
        assertTrue(drafts.apply(server("channel00001", "from the phone", "12.000000")))
        // A send clears the text: it saves at once, as empty text.
        drafts["channel00001"] = ""
        assertEquals("channel00001" to true, edits.last())
        assertEquals("", drafts.pendingSave("channel00001")?.text)
    }

    @Test fun `own echo changes nothing`() {
        val drafts = ConversationDrafts()
        drafts["channel00001", "message00000001"] = "my reply"
        val save = requireNotNull(drafts.pendingSave(ConversationDrafts.thread("message00000001")))
        assertEquals(DraftSave(ConversationDrafts.thread("message00000001"), "channel00001", "message00000001", "my reply"), save)
        val echo = server("channel00001", "my reply", "20.000000", root = "message00000001")
        // The echo can arrive before the save's answer (still dirty) or after it (already applied).
        assertFalse(drafts.apply(echo))
        drafts.saved(save, echo)
        assertFalse(drafts.apply(echo))
        assertEquals("my reply", drafts[ConversationDrafts.thread("message00000001")])
        assertTrue(drafts.dirtyKeys().isEmpty())
    }

    @Test fun `after GET, device-only drafts are uploaded and listed ones apply`() {
        val dir = File(folder.root, "drafts")
        // Typed before sync existed: the file has text but no sync state.
        dir.mkdirs()
        File(dir, "alice123.json").writeText("""{"channel00001":"typed offline","channel00002":"old text","thread:message00000001":"unknown channel"}""")
        val drafts = ConversationDrafts().apply { open(dir, "alice123") }
        drafts["channel00003", "message00000002"] = "a thread reply"
        val merge = drafts.merge(listOf(server("channel00002", "newer elsewhere", "30.000000"), server("channel00004", "", "29.000000")))
        assertEquals(setOf("channel00002"), merge.applied)
        assertEquals("newer elsewhere", drafts["channel00002"])
        assertEquals(setOf("channel00001", ConversationDrafts.thread("message00000002")), merge.saves.toSet())
        assertEquals("typed offline", drafts.pendingSave("channel00001")?.text)
        // A thread reply whose channel this device never learned stays here.
        assertNull(drafts.pendingSave(ConversationDrafts.thread("message00000001")))
        assertEquals("unknown channel", drafts[ConversationDrafts.thread("message00000001")])
        // Unsaved edits survive a restart, so they still upload and still win over the server.
        val after = ConversationDrafts().apply { open(dir, "alice123") }
        assertEquals("typed offline", after.pendingSave("channel00001")?.text)
        assertFalse(after.apply(server("channel00001", "older server copy", "31.000000")))
        assertFalse(after.apply(server("channel00002", "older", "30.000000")))
    }
}
