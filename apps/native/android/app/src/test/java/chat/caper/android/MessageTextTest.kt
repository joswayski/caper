package chat.caper.android

import androidx.compose.ui.text.LinkAnnotation
import chat.caper.android.data.linkRanges
import chat.caper.android.model.MessageMention
import org.junit.Assert.assertEquals
import org.junit.Test

class MessageTextTest {
    private val maya = MessageMention("user", "maya0000000a", "maya")

    @Test fun `links open their href and mentions stay pills`() {
        val text = "@maya see https://a.com/(@maya) or www.b.org."
        val spans = highlightedMentions(text, listOf(maya))
        val opened = mutableListOf<String>()
        val annotated = messageText(text, spans, listOf(maya), linkRanges(text, spans.map { it.start until it.end }), openLink = { opened += it })
        assertEquals(text, annotated.text.replace("\uFFFD", ""))
        assertEquals(2, annotated.getStringAnnotations("androidx.compose.foundation.text.inlineContent", 0, annotated.length).size)
        val links = annotated.getLinkAnnotations(0, annotated.length).sortedBy { it.start }
        assertEquals(listOf("mention:maya", "https://a.com/(", "mention:maya", "https://www.b.org"), links.map {
            when (val item = it.item) {
                is LinkAnnotation.Url -> item.url
                is LinkAnnotation.Clickable -> item.tag
                else -> error("unexpected $item")
            }
        })
        assertEquals(text.indexOf("www.b.org") + 1 until text.indexOf("www.b.org") + 11, links.last().let { it.start until it.end })
        assertEquals("@maya", annotated.text.substring(links[2].start, links[2].end))
        links.mapNotNull { it.item as? LinkAnnotation.Url }.forEach { it.linkInteractionListener?.onClick(it) }
        assertEquals(listOf("https://a.com/(", "https://www.b.org"), opened)
    }

    @Test fun `a grouped row's edited marker follows the text`() {
        var history = 0
        val annotated = messageText("hello", emptyList(), openHistory = { history++ })
        assertEquals("hello (edited)", annotated.text)
        val edited = annotated.getLinkAnnotations(0, annotated.length).single()
        assertEquals(6 until 14, edited.start until edited.end)
        (edited.item as LinkAnnotation.Clickable).linkInteractionListener?.onClick(edited.item)
        assertEquals(1, history)
    }

    @Test fun `icons preserve mixed case destinations and edited marker offsets`() {
        val text = "HTTPS://Example.org/AbCd?q=Case @maya"
        val opened = mutableListOf<String>()
        val annotated = messageText(text, highlightedMentions(text, listOf(maya)), listOf(maya), linkRanges(text),
            openLink = { opened += it }, openHistory = {})
        val links = annotated.getLinkAnnotations(0, annotated.length).sortedBy { it.start }
        val url = links.first().item as LinkAnnotation.Url
        url.linkInteractionListener?.onClick(url)
        assertEquals(listOf("https://Example.org/AbCd?q=Case"), opened)
        assertEquals("@maya", annotated.text.substring(links[1].start, links[1].end))
        assertEquals("(edited)", annotated.text.substring(links.last().start, links.last().end))
        assertEquals(text + " (edited)", annotated.text.replace("\uFFFD", ""))
        assertEquals(androidx.compose.ui.graphics.Color.Unspecified, MessageLink.style?.background)
        assertEquals(chat.caper.android.ui.LinkBlue, MessageLink.style?.color)
        assertEquals(chat.caper.android.ui.LinkBlueActive, MessageLink.hoveredStyle?.color)
    }
}
