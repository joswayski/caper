package chat.caper.android

import chat.caper.android.model.ChatAttachment
import org.junit.Assert.*
import org.junit.Test

class ReplyAttachmentsTest {
    private val image = ChatAttachment("attachment", "image", "image/png", "photo.png", 1, url = "https://cdn.example/original", previewUrl = "https://cdn.example/preview")

    @Test fun `quotes prefer previews and only fall back to still image originals`() {
        assertEquals(image.previewUrl, replyAttachmentPreviewUrl(image))
        assertEquals(image.url, replyAttachmentPreviewUrl(image.copy(previewUrl = null)))
        assertEquals(image.previewUrl, replyAttachmentPreviewUrl(image.copy(kind = "video")))
        assertNull(replyAttachmentPreviewUrl(image.copy(kind = "video", previewUrl = null)))
        assertNull(replyAttachmentPreviewUrl(image.copy(contentType = "image/gif", previewUrl = null)))
        assertNull(replyAttachmentPreviewUrl(image.copy(animated = true, previewUrl = null)))
        assertNull(replyAttachmentPreviewUrl(image.copy(kind = "audio")))
    }

    @Test fun `concealed or unready quote media has no fetch source`() {
        assertNull(replyAttachmentPreviewUrl(image.copy(spoiler = true)))
        assertNull(replyAttachmentPreviewUrl(image.copy(unavailable = true)))
        assertNull(replyAttachmentPreviewUrl(image.copy(status = "processing")))
        assertNull(replyAttachmentPreviewUrl(image.copy(status = "failed")))
        assertNull(replyAttachmentPreviewUrl(image.copy(url = null, previewUrl = null)))
    }
}
