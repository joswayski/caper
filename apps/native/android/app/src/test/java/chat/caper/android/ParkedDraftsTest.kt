package chat.caper.android

import chat.caper.android.data.AttachmentEdit
import chat.caper.android.data.EditRect
import chat.caper.android.model.DraftAttachmentUi
import org.junit.Assert.*
import org.junit.Test

class ParkedDraftsTest {
    private val photo = DraftAttachmentUi("k", "a.jpg", "image", pickedUri = "content://media/1", spoiler = true)

    @Test fun `only unedited, non-voice files with a picked source are parked`() {
        val parked = ParkedDrafts<String>()
        assertFalse(parked.park("general", photo.copy(key = "edited", edit = AttachmentEdit(crop = EditRect(0.0, 0.0, 0.5, 0.5)))))
        assertFalse(parked.park("general", photo.copy(key = "voice", voice = true)))
        assertFalse(parked.park("general", photo.copy(key = "camera", pickedUri = null)))
        assertTrue(parked.park("general", photo))
        assertEquals(listOf("k"), parked.drafts().map { it.key })
    }

    @Test fun `the same source at the same place restores the draft once, its upload kept and spoiler cleared`() {
        val parked = ParkedDrafts<String>()
        parked.park("general", photo)
        // Progress and completion keep arriving while it is parked.
        parked.update("k") { it.copy(progress = 1f, compressing = false) }
        assertNull("another composer", parked.restore("random", "content://media/1"))
        assertNull("another file", parked.restore("general", "content://media/2"))
        val restored = parked.restore("general", "content://media/1")
        assertEquals(photo.copy(progress = 1f, compressing = false, spoiler = false), restored)
        assertNull(parked.restore("general", "content://media/1"))
        assertNull("the timer finds nothing left to release", parked.release("k"))
    }

    @Test fun `releasing takes files out by key or by place`() {
        val parked = ParkedDrafts<String>()
        parked.park("general", photo)
        parked.park("general", photo.copy(key = "k2", pickedUri = "content://media/2"))
        parked.park("random", photo.copy(key = "k3"))
        assertEquals("k2", parked.release("k2")?.key)
        assertEquals(listOf("k"), parked.releaseAll { it == "general" }.map { it.key })
        assertEquals(listOf("k3"), parked.drafts().map { it.key })
        assertEquals(listOf("k3"), parked.releaseAll().map { it.key })
        assertTrue(parked.drafts().isEmpty())
    }
}
