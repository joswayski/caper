package chat.caper.android

import androidx.compose.ui.graphics.Color
import chat.caper.android.model.AppUiState
import chat.caper.android.model.AttachmentDraftTarget
import chat.caper.android.model.Channel
import chat.caper.android.model.DraftAttachmentUi
import chat.caper.android.model.ThreadUi
import chat.caper.android.ui.TextMuted
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class ComposerCounterTest {
    @Test fun `gallery requests only version appropriate media permissions`() {
        assertEquals(listOf("android.permission.READ_EXTERNAL_STORAGE"), galleryPermissions(32).toList())
        assertEquals(listOf("android.permission.READ_MEDIA_IMAGES", "android.permission.READ_MEDIA_VIDEO"), galleryPermissions(33).toList())
        assertEquals(listOf("android.permission.READ_MEDIA_IMAGES", "android.permission.READ_MEDIA_VIDEO", "android.permission.READ_MEDIA_VISUAL_USER_SELECTED"), galleryPermissions(34).toList())
    }

    @Test fun `gallery distinguishes denied selected-only one-media-type and full grants`() {
        for (sdk in listOf(26, 32, 33, 34, 36)) {
            assertEquals(GalleryAccess.Denied, galleryAccess(sdk) { false })
            assertEquals(GalleryAccess.Full, galleryAccess(sdk) { true })
        }
        assertEquals(GalleryAccess.Limited, galleryAccess(34) { it == "android.permission.READ_MEDIA_VISUAL_USER_SELECTED" })
        assertEquals(GalleryAccess.Limited, galleryAccess(33) { it == "android.permission.READ_MEDIA_IMAGES" })
        assertEquals(GalleryAccess.Limited, galleryAccess(34) { it == "android.permission.READ_MEDIA_VIDEO" })
        assertEquals(GalleryAccess.Denied, galleryAccess(33) { it == "android.permission.READ_MEDIA_VISUAL_USER_SELECTED" })
        assertEquals(GalleryAccess.Full, galleryAccess(32) { it == "android.permission.READ_EXTERNAL_STORAGE" })
    }

    @Test fun `gallery reserves existing draft slots and rejects a third selection`() {
        val remaining = chat.caper.android.data.AttachmentPolicy.MAX_ATTACHMENTS - 8
        val first = toggleGallerySelection(emptyList(), "photo", remaining)
        val second = toggleGallerySelection(first, "video", remaining)
        assertEquals(listOf("photo", "video"), second)
        assertEquals(second, toggleGallerySelection(second, "third", remaining))
        val removed = toggleGallerySelection(second, "photo", remaining)
        assertEquals(listOf("video"), removed)
        assertEquals(listOf("video", "third"), toggleGallerySelection(removed, "third", remaining))
    }

    @Test fun `gallery keeps tap order and reselect appends rather than library order`() {
        val selected = listOf("newer", "older")
        assertEquals(listOf("older", "newer"), toggleGallerySelection(toggleGallerySelection(selected, "newer", 2), "newer", 2))
    }

    @Test fun `gallery zero capacity rejects additions but permits deselection`() {
        assertEquals(emptyList<String>(), toggleGallerySelection(emptyList(), "photo", 0))
        assertEquals(emptyList<String>(), toggleGallerySelection(emptyList(), "photo", -1))
        assertEquals(emptyList<String>(), toggleGallerySelection(listOf("photo"), "photo", 0))
    }

    @Test fun `gallery prunes revoked media and reduced capacity without reordering`() {
        val selected = listOf("revoked", "video", "photo")
        assertEquals(listOf("video", "photo"), pruneGallerySelection(selected, 10) { it != "revoked" })
        assertEquals(listOf("video"), pruneGallerySelection(selected, 1) { it != "revoked" })
        assertEquals(emptyList<String>(), pruneGallerySelection(selected, 10) { false })
        assertEquals(emptyList<String>(), pruneGallerySelection(selected, 0) { true })
        assertEquals(selected, pruneGallerySelection(selected, 10) { true })
    }

    @Test fun `counter tones match the web thresholds`() {
        assertEquals(TextMuted, counterTone(3499))
        assertEquals(Color(0xFFE4C76A), counterTone(3500))
        assertEquals(Color(0xFFEDA361), counterTone(3750))
        assertEquals(Color(0xFFFF827C), counterTone(3900))
    }

    @Test fun `attach control explains checking, unavailable and full like every client`() {
        val channel = Channel("general", "space", "general", private = false)
        val target = AttachmentDraftTarget(channel.id)
        val ready = AppUiState(uploadsEnabled = true, uploadsChecked = true, chatAuthorId = "me", selectedChannel = channel)
        assertEquals("Checking file uploads…", attachDisabledReason(AppUiState(chatAuthorId = "me")))
        assertEquals("Checking file uploads…", attachDisabledReason(ready.copy(messagesLoading = true)))
        assertEquals("File uploads are unavailable", attachDisabledReason(ready.copy(uploadsEnabled = false)))
        assertEquals("File uploads are unavailable", attachDisabledReason(ready.copy(messagesError = "Failed")))
        val full = List(10) { DraftAttachmentUi("$it", "a.txt", "file") }
        assertEquals("You can attach up to 10 files.", attachDisabledReason(ready.copy(attachmentDrafts = mapOf(target to full))))
        assertNull(attachDisabledReason(ready))
    }

    @Test fun `each thread has its own file limit and channel files survive closing and switching`() {
        val general = Channel("general", "space", "general", private = false)
        val design = general.copy(id = "design", name = "design")
        val channel = AttachmentDraftTarget(general.id)
        val thread = AttachmentDraftTarget(general.id, "root")
        val otherThread = AttachmentDraftTarget(general.id, "other-root")
        val designThread = AttachmentDraftTarget(design.id, "root")
        val channelFiles = listOf(DraftAttachmentUi("channel-file", "channel.txt", "file"))
        val full = List(10) { DraftAttachmentUi("thread-$it", "thread.txt", "file") }
        val state = AppUiState(
            uploadsEnabled = true, uploadsChecked = true, chatAuthorId = "me", selectedChannel = general,
            thread = ThreadUi("root", loading = false),
            attachmentDrafts = mapOf(channel to channelFiles, thread to full),
            attachmentErrors = mapOf(thread to "Thread upload error"),
        )
        assertEquals(channelFiles, state.drafts)
        assertNull(state.attachmentError)
        assertNull(attachDisabledReason(state))
        assertEquals("You can attach up to 10 files.", attachDisabledReason(state, thread))
        assertNull(attachDisabledReason(state, otherThread))
        assertNull(attachDisabledReason(state, designThread))
        assertEquals("Checking file uploads…", attachDisabledReason(state.copy(thread = ThreadUi("root")), thread))
        assertEquals("File uploads are unavailable", attachDisabledReason(state.copy(thread = ThreadUi("root", error = "Failed")), thread))

        val closed = state.copy(thread = null)
        val switched = closed.copy(selectedChannel = design)
        assertEquals(emptyList<DraftAttachmentUi>(), switched.drafts)
        val reopened = switched.copy(selectedChannel = general, thread = state.thread)
        assertEquals(channelFiles, reopened.drafts)
        assertEquals(full, reopened.attachmentDrafts[thread])
        assertEquals("Thread upload error", reopened.attachmentErrors[thread])
    }
}
