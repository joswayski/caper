package chat.caper.android

import chat.caper.android.model.AttachmentDraftTarget
import chat.caper.android.model.ChatAttachment
import chat.caper.android.model.DraftAttachmentUi
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

    private fun server(channel: String, text: String, at: String, root: String? = null, files: List<ChatAttachment> = emptyList()) =
        ServerDraft(channel, root, text, files, "2026-11-27T18:04:${at}Z")

    /** Applied, and the text here changed. */
    private fun ConversationDrafts.applies(draft: ServerDraft) = apply(draft)?.textChanged == true

    @Test fun `a server draft applies when clean and newer, and an older one is ignored`() {
        val drafts = ConversationDrafts()
        assertTrue(drafts.applies(server("channel00001", "from the web", "05.000001")))
        assertEquals("from the web", drafts["channel00001"])
        assertNull("older", drafts.apply(server("channel00001", "stale", "04.999999")))
        assertNull("the same one again", drafts.apply(server("channel00001", "stale", "05.000001")))
        assertEquals("from the web", drafts["channel00001"])
        assertTrue(drafts.applies(server("channel00001", "", "06.000000")))
        assertEquals("cleared elsewhere", "", drafts["channel00001"])
        assertTrue(drafts.applies(server("channel00001", "reply text", "07.000000", root = "message00000001")))
        assertEquals("reply text", drafts[ConversationDrafts.thread("message00000001")])
        assertEquals("", drafts["channel00001"])
        // A cleared draft remembers when, so an older copy still on its way can't bring sent text back.
        assertNull(drafts.apply(server("channel00001", "sent long ago", "05.500000")))
        assertEquals("", drafts["channel00001"])
    }

    @Test fun `a dirty draft is never overwritten until its save lands, and edits since stay dirty`() {
        val edits = mutableListOf<Pair<String, Boolean>>()
        val drafts = ConversationDrafts().apply { edited = { key, now -> edits += key to now } }
        drafts["channel00001"] = "typing here"
        assertEquals(listOf("channel00001" to false), edits)
        assertNull(drafts.apply(server("channel00001", "from the phone", "09.000000")))
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
        assertTrue(drafts.applies(server("channel00001", "from the phone", "12.000000")))
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
        assertNull(drafts.apply(echo))
        drafts.saved(save, echo)
        assertNull(drafts.apply(echo))
        assertEquals("my reply", drafts[ConversationDrafts.thread("message00000001")])
        assertTrue(drafts.dirtyKeys().isEmpty())
    }

    private fun file(id: String, kind: String = "image", type: String = "image/png", spoiler: Boolean = false, url: String = "https://cdn.caper.chat/assets/$id?sig=1") =
        ChatAttachment(id = id, kind = kind, contentType = type, name = "$id.png", size = 1_234, url = url, previewUrl = "$url&preview", spoiler = spoiler)
    private fun uploading(key: String) = DraftAttachmentUi(key, "picked.jpg", "image", pickedUri = "content://picked/$key", progress = 0.4f)
    private fun ready(key: String, id: String, spoiler: Boolean = false) =
        DraftAttachmentUi(key, "picked.jpg", "image", pickedUri = "content://picked/$key", localUrl = "file:///cache/$key.avif", compressing = false,
            attachment = file(id), uploadedAt = 1L, spoiler = spoiler)

    @Test fun `a completed upload saves its ID and spoiler mark, and an uploading tile isn't sent`() {
        val edits = mutableListOf<Pair<String, Boolean>>()
        val drafts = ConversationDrafts().apply { edited = { key, now -> edits += key to now } }
        val channel = AttachmentDraftTarget("channel00001")
        // Files not known here yet, and none uploaded: nothing to save, so the server's files stay.
        drafts.setFiles(channel, DraftFiles.of(listOf(uploading("a"))))
        assertTrue(edits.isEmpty())
        assertNull(drafts.pendingSave("channel00001"))
        drafts["channel00001"] = "look"
        assertNull("text alone leaves the server's files as they are", drafts.pendingSave("channel00001")?.files)
        // The upload completes: an edit like typing, saved after the usual pause.
        drafts.setFiles(channel, DraftFiles.of(listOf(ready("a", "file000000000001", spoiler = true), uploading("b"))))
        assertEquals("channel00001" to false, edits.last())
        val save = requireNotNull(drafts.pendingSave("channel00001"))
        assertEquals(DraftSave("channel00001", "channel00001", null, "look", DraftFiles(listOf("file000000000001"), listOf("file000000000001"))), save)
        // Progress on the uploading tile changes nothing.
        val count = edits.size
        drafts.setFiles(channel, DraftFiles.of(listOf(ready("a", "file000000000001", spoiler = true), uploading("b").copy(progress = 0.9f))))
        assertEquals(count, edits.size)
        drafts.saved(save, server("channel00001", "look", "40.000000", files = listOf(file("file000000000001", spoiler = true))))
        assertTrue(drafts.dirtyKeys().isEmpty())
        // Toggling the spoiler mark, then removing the file, are edits too; removing the last one saves no files.
        drafts.setFiles(channel, DraftFiles.of(listOf(ready("a", "file000000000001"))))
        assertEquals(DraftFiles(listOf("file000000000001"), emptyList()), drafts.pendingSave("channel00001")?.files)
        drafts.setFiles(channel, DraftFiles.of(emptyList()))
        assertEquals(DraftFiles(emptyList(), emptyList()), drafts.pendingSave("channel00001")?.files)
        // A thread composer's files save with its thread.
        drafts.setFiles(AttachmentDraftTarget("channel00001", "message00000001"), DraftFiles.of(listOf(ready("c", "file000000000002"))))
        assertEquals(DraftSave(ConversationDrafts.thread("message00000001"), "channel00001", "message00000001", "", DraftFiles(listOf("file000000000002"), emptyList())),
            drafts.pendingSave(ConversationDrafts.thread("message00000001")))
    }

    @Test fun `server files appear as ready tiles, uploading tiles survive, and dropped files disappear`() {
        val local = listOf(
            ready("own", "file000000000001"),
            syncedDraftTile("synced", file("file000000000002", url = "https://cdn.caper.chat/assets/file000000000002?sig=old")),
            ready("gone", "file000000000003"),
            uploading("busy"),
        )
        val listed = listOf(
            file("file000000000002", spoiler = true, url = "https://cdn.caper.chat/assets/file000000000002?sig=new"),
            file("file000000000004", kind = "file", type = "application/pdf"),
            file("file000000000005"),
            file("file000000000001"),
        )
        var next = 0
        val merge = reconcileDraftTiles(local, listed) { "new${++next}" }
        assertEquals(listOf("synced", "new1", "new2", "own", "busy"), merge.tiles.map { it.key })
        assertEquals(listOf("gone"), merge.dropped.map { it.key })
        val synced = merge.tiles[0]
        assertTrue(synced.spoiler)
        assertEquals("https://cdn.caper.chat/assets/file000000000002?sig=new", synced.attachment?.url)
        assertEquals("https://cdn.caper.chat/assets/file000000000002?sig=new&preview", synced.thumbnail)
        // Another device's files are ready tiles from their signed URLs: no upload, no local original.
        val pdf = merge.tiles[1]
        assertTrue(pdf.synced)
        assertFalse(pdf.compressing)
        assertEquals("file000000000004", pdf.attachment?.id)
        assertNull(pdf.pickedUri)
        assertNull(pdf.original)
        assertNull(pdf.localUrl)
        assertNull("a PDF isn't editable", pdf.editKind)
        assertEquals("image", merge.tiles[2].editKind)
        assertEquals("own", merge.tiles[3].key)
        assertFalse("this device's upload keeps its own tile", merge.tiles[3].synced)
        assertEquals("file:///cache/own.avif", merge.tiles[3].localUrl)
        // Animated files (GIFs stored as video) aren't editable.
        assertNull(syncedDraftTile("gif", file("file000000000006", kind = "video", type = "video/mp4").copy(animated = true)).editKind)

        // Applying it marks nothing dirty: the tiles shown are the files the server has.
        val edits = mutableListOf<Pair<String, Boolean>>()
        val drafts = ConversationDrafts().apply { edited = { key, now -> edits += key to now } }
        val applied = requireNotNull(drafts.apply(server("channel00001", "", "50.000000", files = listed)))
        assertEquals(AttachmentDraftTarget("channel00001"), applied.target)
        assertEquals(listed, applied.attachments)
        drafts.setFiles(applied.target, DraftFiles.of(merge.tiles))
        assertTrue(edits.isEmpty())
        assertTrue(drafts.dirtyKeys().isEmpty())
        // The uploading tile finishing is this device's edit, and it doesn't drop the others.
        drafts.setFiles(applied.target, DraftFiles.of(merge.tiles.map { if (it.key == "busy") ready("busy", "file000000000007") else it }))
        assertEquals(listOf("file000000000002", "file000000000004", "file000000000005", "file000000000001", "file000000000007"),
            drafts.pendingSave("channel00001")?.files?.ids)
        assertEquals(listOf("file000000000002"), drafts.pendingSave("channel00001")?.files?.spoilers)
    }

    @Test fun `an unapplied server draft only renews synced tiles' signatures`() {
        val own = ready("own", "file000000000001")
        val synced = syncedDraftTile("synced", file("file000000000002", url = "https://cdn.caper.chat/assets/file000000000002?sig=old"))
        val fresh = refreshSyncedTiles(listOf(own, synced), listOf(
            file("file000000000001", url = "https://cdn.caper.chat/assets/file000000000001?sig=new"),
            file("file000000000002", spoiler = true, url = "https://cdn.caper.chat/assets/file000000000002?sig=new"),
        ))
        assertEquals(own, fresh[0])
        assertEquals("https://cdn.caper.chat/assets/file000000000002?sig=new", fresh[1].attachment?.url)
        assertFalse("not applied: the mark here stays", fresh[1].spoiler)
    }

    @Test fun `own echo with files changes nothing, and a dirty draft keeps its files`() {
        val drafts = ConversationDrafts()
        val channel = AttachmentDraftTarget("direct000001")
        drafts.setFiles(channel, DraftFiles.of(listOf(ready("a", "file000000000001"))))
        val save = requireNotNull(drafts.pendingSave("direct000001"))
        val echo = server("direct000001", "", "60.000000", files = listOf(file("file000000000001")))
        // Another device's change while this one is dirty doesn't apply (its files neither).
        assertNull(drafts.apply(server("direct000001", "elsewhere", "59.000000", files = emptyList())))
        assertNull(drafts.apply(echo))
        drafts.saved(save, echo)
        assertNull(drafts.apply(echo))
        assertTrue(drafts.dirtyKeys().isEmpty())
        // A newer draft from another device that removed the file applies: its composer drops the tile.
        val removed = requireNotNull(drafts.apply(server("direct000001", "", "61.000000")))
        assertEquals(listOf("a"), reconcileDraftTiles(listOf(ready("a", "file000000000001"), uploading("b")), removed.attachments) { "x" }.dropped.map { it.key })

        // A text-only save (files not known here, as after a restart) leaves the server's files alone and learns them
        // from its answer, so the composer shows them and the echo still changes nothing.
        val restarted = ConversationDrafts()
        restarted["channel00002"] = "text only"
        val textSave = requireNotNull(restarted.pendingSave("channel00002"))
        assertNull(textSave.files)
        val answer = server("channel00002", "text only", "70.000000", files = listOf(file("file000000000009")))
        assertEquals(listOf("file000000000009"), restarted.saved(textSave, answer)?.attachments?.map { it.id })
        assertNull(restarted.apply(answer))
        assertNull("learned once", restarted.saved(textSave, answer))
    }

    @Test fun `after GET, device-only drafts are uploaded and listed ones apply`() {
        val dir = File(folder.root, "drafts")
        // Typed before sync existed: the file has text but no sync state.
        dir.mkdirs()
        File(dir, "alice123.json").writeText("""{"channel00001":"typed offline","channel00002":"old text","thread:message00000001":"unknown channel"}""")
        val drafts = ConversationDrafts().apply { open(dir, "alice123") }
        drafts["channel00003", "message00000002"] = "a thread reply"
        val merge = drafts.merge(listOf(server("channel00002", "newer elsewhere", "30.000000"), server("channel00004", "", "29.000000")))
        assertEquals(listOf("channel00002" to true, "channel00004" to false), merge.applied.map { it.key to it.textChanged })
        assertEquals("newer elsewhere", drafts["channel00002"])
        assertEquals(setOf("channel00001", ConversationDrafts.thread("message00000002")), merge.saves.toSet())
        assertEquals("typed offline", drafts.pendingSave("channel00001")?.text)
        // A thread reply whose channel this device never learned stays here.
        assertNull(drafts.pendingSave(ConversationDrafts.thread("message00000001")))
        assertEquals("unknown channel", drafts[ConversationDrafts.thread("message00000001")])
        // Unsaved edits survive a restart, so they still upload and still win over the server.
        val after = ConversationDrafts().apply { open(dir, "alice123") }
        assertEquals("typed offline", after.pendingSave("channel00001")?.text)
        assertNull(after.apply(server("channel00001", "older server copy", "31.000000")))
        assertNull(after.apply(server("channel00002", "older", "29.000000")))
        // Staged files stay in memory: after a restart the same server draft applies again to bring them back.
        assertEquals(false, after.apply(server("channel00002", "newer elsewhere", "30.000000"))?.textChanged)
        assertNull("once known, the same one is ignored", after.apply(server("channel00002", "newer elsewhere", "30.000000")))
    }
}
