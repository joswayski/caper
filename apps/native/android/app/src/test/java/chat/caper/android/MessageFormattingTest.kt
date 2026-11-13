package chat.caper.android

import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import chat.caper.android.data.linkRanges
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
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

    @Test fun `underline and strikethrough render as text decorations`() {
        val rendered = messageText("__under__ ~~gone~~ ___both___", emptyList())
        assertEquals("under gone both", rendered.text)
        fun decoration(word: String) = rendered.spanStyles.single { rendered.text.substring(it.start, it.end) == word }.item
        assertEquals(TextDecoration.Underline, decoration("under").textDecoration)
        assertEquals(TextDecoration.LineThrough, decoration("gone").textDecoration)
        assertEquals(TextDecoration.Underline, decoration("both").textDecoration)
        assertEquals(androidx.compose.ui.text.font.FontStyle.Italic, decoration("both").fontStyle)
    }

    @Test fun `hidden spoilers hide text and keep their links and mentions inert until revealed together`() {
        val text = "a ||https://example.com see **@alex**|| b ||two||"
        val mentions = listOf(MentionSpan(text.indexOf("@alex"), text.indexOf("@alex") + 5, "alex"))
        val alex = chat.caper.android.model.MessageMention("user", "1", "alex")
        val links = chat.caper.android.data.messageLinkRanges(text, mentions.map { it.start until it.end })
        assertEquals(listOf("https://example.com"), links.map { it.href })
        val revealed = mutableListOf<Int>()
        val hidden = messageText(text, mentions, listOf(alex), links, reveal = { revealed += it })
        assertEquals("a https://example.com see @alex b two", hidden.text)
        val spoilers = hidden.getStringAnnotations(SpoilerText, 0, hidden.length)
        assertEquals(listOf("https://example.com see @alex", "two"), spoilers.map { hidden.text.substring(it.start, it.end) })
        assertTrue(spoilers.all { it.item == SPOILER_HIDDEN })
        assertTrue(spoilers.all { spoiler -> hidden.spanStyles.any { it.start == spoiler.start && it.end == spoiler.end && it.item.color == Color.Transparent } })
        // Only the reveal links remain: the mention and URL cannot open while hidden.
        val reveals = hidden.getLinkAnnotations(0, hidden.length)
        assertEquals(listOf(SpoilerReveal, SpoilerReveal), reveals.map { (it.item as LinkAnnotation.Clickable).tag })
        (reveals.first().item as LinkAnnotation.Clickable).let { it.linkInteractionListener!!.onClick(it) }
        assertEquals(listOf(0), revealed)
        assertTrue(inHiddenSpoiler(text, emptySet(), mentions[0].start, mentions[0].end))
        assertFalse(inHiddenSpoiler(text, setOf(0), mentions[0].start, mentions[0].end))

        val shown = messageText(text, mentions, listOf(alex), links, revealed = setOf(0))
        assertEquals("a \uFFFDhttps://example.com see @alex b two", shown.text)
        // The revealed spoiler owns the link's icon.
        assertEquals("\uFFFDhttps://example.com see @alex", shown.getStringAnnotations(SpoilerText, 0, shown.length).first().let { shown.text.substring(it.start, it.end) })
        assertEquals(listOf(SPOILER_SHOWN, SPOILER_HIDDEN), shown.getStringAnnotations(SpoilerText, 0, shown.length).map { it.item })
        val tags = shown.getLinkAnnotations(0, shown.length).map { (it.item as? LinkAnnotation.Clickable)?.tag ?: (it.item as LinkAnnotation.Url).url }
        assertEquals(setOf("mention:alex", "https://example.com", SpoilerReveal), tags.toSet())
        assertFalse(shown.spanStyles.any { it.item.color == Color.Transparent && it.start < shown.text.indexOf(" b ") })
    }

    @Test fun `quote blocks are indented paragraphs with a bar and keep every offset`() {
        val text = "before\n> one\n> **two**\nafter"
        val rendered = messageText(text, emptyList())
        // Each edge newline is a paragraph break of the same length; the newline inside the block stays.
        assertEquals("before\u200Bone\ntwo\u200Bafter", rendered.text)
        val block = rendered.getStringAnnotations(QuoteBlock, 0, rendered.length).single()
        assertEquals("one\ntwo", rendered.text.substring(block.start, block.end))
        val paragraph = rendered.paragraphStyles.single()
        assertEquals(block.start, paragraph.start)
        assertEquals(block.end + 1, paragraph.end)
        assertTrue(paragraph.item.textIndent!!.firstLine.value > 0f)
    }

    @Test fun `composer shows spoilers marked and quotes with bars in source coordinates`() {
        val source = "> **q** https://a.co\n||x||"
        val result = MentionVisualTransformation(MentionSource(emptyList(), false)).filter(androidx.compose.ui.text.AnnotatedString(source))
        assertEquals("q ${DraftLinkPrefix}https://a.co\u200Bx", result.text.text)
        val spoiler = result.text.getStringAnnotations(SpoilerText, 0, result.text.length).single()
        assertEquals(SPOILER_DRAFT, spoiler.item)
        assertEquals("x", result.text.text.substring(spoiler.start, spoiler.end))
        val quote = result.text.getStringAnnotations(QuoteBlock, 0, result.text.length).single()
        assertEquals("q ${DraftLinkPrefix}https://a.co", result.text.text.substring(quote.start, quote.end))
        assertEquals(source.indexOf("x"), result.offsetMapping.transformedToOriginal(spoiler.start))
        assertEquals(spoiler.start, result.offsetMapping.originalToTransformed(source.indexOf("x")))
        assertTrue(result.text.spanStyles.any { it.item.fontWeight == FontWeight.Bold && result.text.text.substring(it.start, it.end) == "q" })
    }

    @Test fun `links are found in each formatted run as web finds them`() {
        // A spoiler's delimiters neither hide its link nor join the address.
        val text = "||https://a.co|| and **https://c.co/x** y"
        val links = chat.caper.android.data.messageLinkRanges(text)
        assertEquals(listOf("https://a.co", "https://c.co/x"), links.map { it.href })
        assertEquals(listOf("https://a.co", "https://c.co/x"), links.map { text.substring(it.start, it.end) })
    }
}
