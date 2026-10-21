package chat.caper.android

import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.font.FontWeight
import chat.caper.android.data.linkRanges
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MessageFormattingTest {
    @Test fun `mention pill follows removed markers`() {
        val text = "**hi** @alex _there_"
        val rendered = messageText(text, listOf(MentionSpan(7, 12, "alex")))
        assertEquals("hi @alex there", rendered.text)
        assertTrue(rendered.spanStyles.any { it.start == 3 && it.end == 8 && it.item.fontWeight == FontWeight.Medium })
    }

    @Test fun `rendered formatting preserves URL target and offsets`() {
        val text = "**bold** _https://example.com/a_b_"
        val rendered = messageText(text, emptyList(), links = linkRanges(text))
        assertTrue(rendered.text.startsWith("bold https://example.com/a_b"))
        assertTrue(rendered.spanStyles.any { it.start == 0 && it.end == 4 && it.item.fontWeight == FontWeight.Bold })
        val link = rendered.getLinkAnnotations(0, rendered.length).single()
        assertEquals("https://example.com/a_b", (link.item as LinkAnnotation.Url).url)
        assertTrue(rendered.text.substring(link.start, link.end).startsWith("https://example.com/a_b"))
    }
}
