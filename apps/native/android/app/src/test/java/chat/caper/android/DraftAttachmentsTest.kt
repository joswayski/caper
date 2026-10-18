package chat.caper.android

import chat.caper.android.data.AttachmentEdit
import chat.caper.android.data.EditRect
import chat.caper.android.model.DraftAttachmentUi
import org.junit.Assert.*
import org.junit.Test

class DraftAttachmentsTest {
    private val cropped = AttachmentEdit(crop = EditRect(0.0, 0.0, 0.5, 0.5))

    @Test fun `a tile's bar holds Spoiler, Edit then Remove, what applies, or nothing while locked`() {
        assertEquals(listOf(DraftAction.SPOILER, DraftAction.EDIT, DraftAction.REMOVE), draftActions(spoiler = true, editable = true, enabled = true))
        assertEquals(listOf(DraftAction.SPOILER, DraftAction.REMOVE), draftActions(spoiler = true, editable = false, enabled = true))
        assertEquals(listOf(DraftAction.EDIT, DraftAction.REMOVE), draftActions(spoiler = false, editable = true, enabled = true))
        assertEquals(listOf(DraftAction.REMOVE), draftActions(spoiler = false, editable = false, enabled = true))
        for (spoiler in listOf(true, false)) for (editable in listOf(true, false)) {
            assertEquals(emptyList<DraftAction>(), draftActions(spoiler, editable, enabled = false))
        }
    }

    @Test fun `tile actions keep their names, and the spoiler toggle says what it will do`() {
        assertEquals("Mark as spoiler: a.jpg", draftActionLabel(DraftAction.SPOILER, "a.jpg", spoiler = false))
        assertEquals("Remove spoiler from spoiler attachment", draftActionLabel(DraftAction.SPOILER, "spoiler attachment", spoiler = true))
        assertEquals("Edit a.jpg", draftActionLabel(DraftAction.EDIT, "a.jpg", spoiler = false))
        assertEquals("Remove a.jpg", draftActionLabel(DraftAction.REMOVE, "a.jpg", spoiler = false))
    }

    @Test fun `a spoilered tile blurs the edited render, never the unedited original`() {
        val image = DraftAttachmentUi("k", "a.jpg", "image", pickedImage = "content://picked/a", pickedUri = "content://picked/a", spoiler = true)
        assertEquals("content://picked/a", draftSpoilerSource(image))
        assertEquals("file:///a/thumb.webp", draftSpoilerSource(image.copy(thumbnail = "file:///a/thumb.webp")))
        val edited = image.copy(edit = cropped)
        assertEquals("file:///a/edited.webp", draftSpoilerSource(edited.copy(thumbnail = "file:///a/edited.webp")))
        assertNull("nothing while the render is written", draftSpoilerSource(edited))
        assertEquals("file:///a/out.avif", draftSpoilerSource(edited.copy(localUrl = "file:///a/out.avif")))
        val video = DraftAttachmentUi("v", "b.mp4", "video", pickedUri = "content://picked/b", spoiler = true)
        assertNull("a video only blurs a poster", draftSpoilerSource(video))
        assertEquals("file:///b/poster.webp", draftSpoilerSource(video.copy(thumbnail = "file:///b/poster.webp")))
        assertNull(draftSpoilerSource(image.copy(error = "Failed to upload.")))
        assertNull(draftSpoilerSource(DraftAttachmentUi("f", "c.pdf", "file", pickedUri = "content://picked/c", spoiler = true)))
    }

    @Test fun `the pencil opens Draw and the preview offers Crop, Draw and Text until the files lock`() {
        assertEquals(EditTool.DRAW, OpenEditor("a").tool)
        assertEquals(EditTool.TEXT, OpenEditor("a", EditTool.TEXT).tool)
        assertEquals(listOf(EditTool.CROP, EditTool.DRAW, EditTool.TEXT), draftPreviewTools(enabled = true))
        assertEquals(listOf("Crop", "Draw", "Text"), draftPreviewTools(enabled = true).map { it.label })
        assertTrue(draftPreviewTools(enabled = false).isEmpty())
    }

    @Test fun `a draft preview shows the original until edited, then what will be sent, never the original`() {
        val image = DraftAttachmentUi("k", "a.jpg", "image", original = "file:///a/source", pickedUri = "content://picked/a", editKind = "image")
        assertEquals(DraftPreviewMedia("file:///a/source", video = false), draftPreviewMedia(image))
        assertEquals(DraftPreviewMedia("content://picked/a", video = false), draftPreviewMedia(image.copy(original = null)))
        val edited = image.copy(edit = cropped, thumbnail = "file:///a/edited.webp")
        assertEquals("the editor's render until the edit is prepared", DraftPreviewMedia("file:///a/edited.webp", video = false), draftPreviewMedia(edited))
        assertEquals(DraftPreviewMedia("file:///a/out.avif", video = false), draftPreviewMedia(edited.copy(localUrl = "file:///a/out.avif")))
        assertNull("nothing to show while the render is written", draftPreviewMedia(edited.copy(thumbnail = null)))

        val video = DraftAttachmentUi("v", "b.mp4", "video", original = "file:///b/source", editKind = "video")
        assertEquals(DraftPreviewMedia("file:///b/source", video = true), draftPreviewMedia(video))
        val editedVideo = video.copy(edit = cropped, thumbnail = "file:///b/edited.webp")
        assertEquals("an edited video shows its render as a still until prepared", DraftPreviewMedia("file:///b/edited.webp", video = false), draftPreviewMedia(editedVideo))
        assertEquals(DraftPreviewMedia("file:///b/out.mp4", video = true), draftPreviewMedia(editedVideo.copy(localUrl = "file:///b/out.mp4")))
    }

    @Test fun `the brush ring is as wide as a stroke looks on screen, at least 6dp`() {
        assertEquals(12f, brushRingDiameter(6f, 2f), 0f)
        assertEquals(105f, brushRingDiameter(40f, 2.625f), 0.0001f)
        assertEquals("thin brushes keep a visible ring", 18f, brushRingDiameter(2f, 3f), 0f)
    }
}
