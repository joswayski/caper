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
        assertEquals("hi 🙂 @alex ${DraftLinkPrefix}https://example.com/a_b", result.text.text)
        assertTrue(result.text.spanStyles.any { it.start == 0 && it.end == 3 && it.item.fontWeight == FontWeight.Bold })
        assertTrue(result.text.spanStyles.any { it.start == 3 && it.end == 5 && it.item.fontStyle == androidx.compose.ui.text.font.FontStyle.Italic && it.item.fontWeight == FontWeight.Bold })
        val mapping = result.offsetMapping
        val icon = result.text.getStringAnnotations(DraftLinkPill, 0, result.text.length).single()
        for (offset in 0..result.text.length) {
            val caret = if (offset in icon.start until icon.start + DraftLinkPrefix.length) icon.start + DraftLinkPrefix.length else offset
            assertEquals(caret, mapping.originalToTransformed(mapping.transformedToOriginal(offset)))
        }
        assertEquals(6, mapping.transformedToOriginal(3))
        assertEquals(3, mapping.originalToTransformed(6))
        val trailing = MentionVisualTransformation(MentionSource(emptyList(), false))
            .filter(androidx.compose.ui.text.AnnotatedString("**hello**"))
        assertEquals(2, trailing.offsetMapping.transformedToOriginal(0))
        assertEquals(7, trailing.offsetMapping.transformedToOriginal(5))
        val trailingUrl = MentionVisualTransformation(MentionSource(emptyList(), false))
            .filter(androidx.compose.ui.text.AnnotatedString("**https://a.co**"))
        assertEquals(DraftLinkPrefix + "https://a.co", trailingUrl.text.text)
        assertEquals(2, trailingUrl.offsetMapping.transformedToOriginal(0))
        assertEquals(14, trailingUrl.offsetMapping.transformedToOriginal(trailingUrl.text.length))
    }

    @Test fun `nested formatted URLs keep pills styles UTF16 carets and trailing completion in source coordinates`() {
        val source = "**🙂 _https://Example.org/a_b_ @maya**\n_www.b.org/Case_ **@al**"
        val transformation = MentionVisualTransformation(MentionSource(listOf(MentionCandidate("maya", "Maya")), false))
        val result = transformation.filter(androidx.compose.ui.text.AnnotatedString(source))
        val visible = "🙂 ${DraftLinkPrefix}https://Example.org/a_b @maya\n${DraftLinkPrefix}www.b.org/Case @al"
        assertEquals(visible, result.text.text)
        val links = result.text.getStringAnnotations(DraftLinkPill, 0, result.text.length)
        assertEquals(listOf("https://Example.org/a_b", "https://www.b.org/Case"), links.map { it.item })
        for (link in links) {
            val firstGlyph = link.start + DraftLinkPrefix.length
            val sourceStart = source.indexOf(result.text.text.substring(firstGlyph, link.end))
            for (offset in link.start..firstGlyph) {
                assertEquals(sourceStart, result.offsetMapping.transformedToOriginal(offset))
            }
            for (offset in firstGlyph until link.end) {
                assertEquals(sourceStart + offset - firstGlyph, result.offsetMapping.transformedToOriginal(offset))
                assertEquals(offset, result.offsetMapping.originalToTransformed(sourceStart + offset - firstGlyph))
            }
            // Internal boundaries resolve to the next visible character, skipping the
            // closing underscore; this is the upstream formatter's existing affinity.
            assertEquals(sourceStart + link.end - firstGlyph + 1, result.offsetMapping.transformedToOriginal(link.end))
            assertTrue(result.text.spanStyles.any { it.start <= firstGlyph && it.end >= link.end &&
                it.item.fontStyle == androidx.compose.ui.text.font.FontStyle.Italic })
        }
        val firstGlyph = links.first().start + DraftLinkPrefix.length
        assertTrue(result.text.spanStyles.any { it.start <= firstGlyph && it.end >= links.first().end && it.item.fontWeight == FontWeight.Bold })
        assertEquals("@maya", result.text.spanStyles.last().let { result.text.text.substring(it.start, it.end) })
        // Both UTF-16 code units of the leading emoji still refer to source positions.
        assertEquals(2, result.offsetMapping.transformedToOriginal(0))
        assertEquals(3, result.offsetMapping.transformedToOriginal(1))
        val caret = result.offsetMapping.transformedToOriginal(result.text.length)
        assertEquals(source.length - 2, caret)
        val draft = androidx.compose.ui.text.input.TextFieldValue(source, androidx.compose.ui.text.TextRange(caret))
        val token = mentionToken(draft)!!
        assertEquals(MentionToken(source.lastIndexOf("@al"), source.length - 2, "al"), token)
        assertEquals(source.replace("**@al**", "**@alice** "), insertMention(draft, token, "alice")!!.text)
        assertEquals("🙂 https://Example.org/a_b @maya\nwww.b.org/Case @al",
            chat.caper.android.data.copiedFormatText(source, 0, source.length))
        val emojiSource = source.replace("**@al**", "**:roc**")
        val emojiResult = transformation.filter(androidx.compose.ui.text.AnnotatedString(emojiSource))
        val emojiDraft = androidx.compose.ui.text.input.TextFieldValue(emojiSource,
            androidx.compose.ui.text.TextRange(emojiResult.offsetMapping.transformedToOriginal(emojiResult.text.length)))
        assertEquals(emojiSource.replace("**:roc**", "**🚀**"), insertEmoji(emojiDraft, emojiToken(emojiDraft)!!, "🚀")!!.text)
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
        assertEquals("bold \uFFFDhttps://example.com/a_b", rendered.text)
        assertTrue(rendered.spanStyles.any { it.start == 0 && it.end == 4 && it.item.fontWeight == FontWeight.Bold })
        val link = rendered.getLinkAnnotations(0, rendered.length).single()
        assertEquals("https://example.com/a_b", (link.item as LinkAnnotation.Url).url)
        assertEquals("\uFFFDhttps://example.com/a_b", rendered.text.substring(link.start, link.end))
        assertTrue(rendered.spanStyles.any { it.start == link.start && it.end == link.end &&
            it.item.fontStyle == androidx.compose.ui.text.font.FontStyle.Italic })
    }
}
