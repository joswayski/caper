package chat.caper.android

import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.font.FontWeight
import chat.caper.android.data.linkRanges
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class MessageFormattingTest {
    @Test fun `formatted emoji and mention completion keep source replacement ranges`() {
        val emoji = androidx.compose.ui.text.input.TextFieldValue("**:roc**", androidx.compose.ui.text.TextRange(6))
        assertEquals(EmojiToken(2, 6, "roc"), emojiToken(emoji))
        assertEquals("**🚀**", insertEmoji(emoji, emojiToken(emoji)!!, "🚀")!!.text)
        val mention = androidx.compose.ui.text.input.TextFieldValue("**@al**", androidx.compose.ui.text.TextRange(5))
        assertEquals(MentionToken(2, 5, "al"), mentionToken(mention))
        assertEquals("**@alice** ", insertMention(mention, mentionToken(mention)!!, "alice")!!.text)
        val crossing = androidx.compose.ui.text.input.TextFieldValue("**hello** tail")
        assertEquals("🚀 tail", insertEmoji(crossing, EmojiToken(0, 7, ""), "🚀")!!.text)
    }

    @Test fun `composer hides delimiters styles nested text and maps UTF16 selections`() {
        val source = "**hi _🙂_** @alex https://example.com/a_b"
        val result = MentionVisualTransformation(MentionSource(listOf(MentionCandidate("alex", "Alex")), false))
            .filter(androidx.compose.ui.text.AnnotatedString(source))
        assertEquals("hi 🙂 @alex https://example.com/a_b", result.text.text)
        assertTrue(result.text.spanStyles.any { it.start == 0 && it.end == 3 && it.item.fontWeight == FontWeight.Bold })
        assertTrue(result.text.spanStyles.any { it.start == 3 && it.end == 5 && it.item.fontStyle == androidx.compose.ui.text.font.FontStyle.Italic && it.item.fontWeight == FontWeight.Bold })
        val mapping = result.offsetMapping
        for (offset in 0..result.text.length) {
            assertEquals(offset, mapping.originalToTransformed(mapping.transformedToOriginal(offset)))
        }
        assertEquals(6, mapping.transformedToOriginal(3))
        assertEquals(3, mapping.originalToTransformed(6))
        val trailing = MentionVisualTransformation(MentionSource(emptyList(), false))
            .filter(androidx.compose.ui.text.AnnotatedString("**hello**"))
        assertEquals(2, trailing.offsetMapping.transformedToOriginal(0))
        assertEquals(7, trailing.offsetMapping.transformedToOriginal(5))
    }

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
