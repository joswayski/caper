package chat.caper.android

import chat.caper.android.model.ChatAttachment
import org.junit.Assert.*
import org.junit.Test

class MediaViewerTest {
    private fun file(
        id: String, kind: String = "image", status: String? = null, url: Boolean = true, unavailable: Boolean = false, animated: Boolean = false,
    ) = ChatAttachment(
        id, kind, "$kind/x", "$id.bin", 10, status = status, animated = animated, unavailable = unavailable,
        url = if (url) "https://cdn.example/original/$id?exp=2000&sig=s" else null,
    )

    @Test fun `only ready images and videos with a url are viewable`() {
        assertTrue(file("a").viewable())
        assertTrue(file("b", status = "ready").viewable())
        assertTrue(file("c", kind = "video").viewable())
        assertTrue(file("d", kind = "video", animated = true).viewable())
        assertFalse(file("e", kind = "audio").viewable())
        assertFalse(file("f", kind = "file").viewable())
        assertFalse(file("g", status = "processing").viewable())
        assertFalse(file("h", status = "failed").viewable())
        assertFalse(file("i", url = false).viewable())
        assertFalse(file("j", unavailable = true).viewable())
    }

    @Test fun `the viewer pages through the message's images and videos in order from the tapped one`() {
        val message = listOf(
            file("one"), file("song", kind = "audio"), file("two", kind = "video"), file("pdf", kind = "file"),
            file("busy", status = "processing"), file("gone", unavailable = true), file("three"),
        )
        assertEquals(ViewerPages(listOf("one", "two", "three"), 0), viewerPages(message, "one"))
        assertEquals(ViewerPages(listOf("one", "two", "three"), 1), viewerPages(message, "two"))
        assertEquals(ViewerPages(listOf("one", "two", "three"), 2), viewerPages(message, "three"))
    }

    @Test fun `files that cannot be viewed do not open the viewer`() {
        val message = listOf(file("one"), file("song", kind = "audio"), file("busy", status = "processing"))
        assertNull(viewerPages(message, "song"))
        assertNull(viewerPages(message, "busy"))
        assertNull(viewerPages(message, "missing"))
        assertNull(viewerPages(emptyList(), "one"))
    }

    @Test fun `a repeated id keeps its first place so pager keys stay unique`() {
        assertEquals(ViewerPages(listOf("one", "two"), 1), viewerPages(listOf(file("one"), file("two"), file("one")), "two"))
    }

    @Test fun `the counter shows only with more than one file`() {
        assertNull(viewerCounter(0, 1))
        assertEquals("1 / 2", viewerCounter(0, 2))
        assertEquals("5 / 5", viewerCounter(4, 5))
    }

    @Test fun `zoomed images pan only until their edges meet the screen's`() {
        assertEquals(1000f to 500f, fittedSize(4000f, 2000f, 1000f, 2000f))
        assertEquals(500f to 1000f, fittedSize(100f, 200f, 1000f, 1000f))
        assertEquals("unknown size fills the viewport", 1000f to 2000f, fittedSize(0f, 0f, 1000f, 2000f))
        assertEquals(0f, panLimit(1000f, 1000f, 1f), 0f)
        assertEquals(500f, panLimit(1000f, 1000f, 2f), 0f)
        // A letterboxed image moves vertically only once it is taller than the screen.
        assertEquals(0f, panLimit(500f, 2000f, 2f), 0f)
        assertEquals(500f, panLimit(500f, 2000f, 6f), 0f)
    }

    @Test fun `download names cannot leave the downloads folder`() {
        assertEquals("photo.jpg", downloadName("photo.jpg"))
        assertEquals("_.._etc_passwd", downloadName("/../etc/passwd"))
        assertEquals("a_b_c.png", downloadName("a\\b:c.png"))
        assertEquals("hidden", downloadName(".hidden"))
        assertEquals("file", downloadName(".."))
        assertEquals("file", downloadName("  "))
        assertEquals("line_break.txt", downloadName("line\nbreak.txt"))
    }
}
