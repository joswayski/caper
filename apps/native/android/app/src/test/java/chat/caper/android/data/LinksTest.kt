package chat.caper.android.data

import java.io.File
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class LinksTest {
    private data class Case(val note: String, val text: String, val segments: List<LinkSegment>)

    /** Every client runs these same cases; see shared/messages/link-cases.json. */
    private val cases: List<Case> by lazy {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/messages/link-cases.json") }.first { it.isFile }
        Json.parseToJsonElement(file.readText()).jsonObject.getValue("cases").jsonArray.map { case ->
            val fields = case.jsonObject
            Case(
                fields.getValue("note").jsonPrimitive.content,
                fields.getValue("text").jsonPrimitive.content,
                fields.getValue("segments").jsonArray.map { segment ->
                    val parts = segment.jsonObject
                    LinkSegment(parts.getValue("text").jsonPrimitive.content, parts["href"]?.jsonPrimitive?.content)
                },
            )
        }
    }

    @Test fun `shared link cases match`() {
        assertTrue("expected the full shared case list", cases.size > 30)
        cases.forEach { case -> assertEquals(case.note, case.segments, linkSegments(case.text)) }
    }

    @Test fun `segments always rebuild the original text and only link http(s)`() {
        val texts = listOf("a https://x.io b", "javascript:alert(1) www.ok.com", "(https://a.b/c))", "", "www.a.b.") + cases.map { it.text }
        texts.forEach { text ->
            val segments = linkSegments(text)
            assertEquals(text, segments.joinToString("") { it.text })
            segments.mapNotNull { it.href }.forEach { href -> assertTrue(href, href.startsWith("https://") || href.startsWith("http://")) }
        }
    }

    @Test fun `scheme matching is ASCII-only`() {
        // Kotlin's ignoreCase would treat the long s (ſ) as `s`; web's toLowerCase does not.
        assertEquals(listOf(LinkSegment("httpſ://example.com")), linkSegments("httpſ://example.com"))
        assertEquals(listOf(LinkSegment("Https://Example.com", "https://Example.com")), linkSegments("Https://Example.com"))
    }

    @Test fun `links are found only outside mentions, each run on its own`() {
        val text = "@maya https://a.com and https://b.com/(@sam) now"
        val maya = 0 until 5
        val sam = text.indexOf("@sam").let { it until it + 4 }
        // The run before "@sam" ends at the mention, so the link cannot swallow the pill.
        assertEquals(
            listOf(LinkRange(6, 19, "https://a.com"), LinkRange(24, 39, "https://b.com/(")),
            linkRanges(text, listOf(maya, sam)),
        )
        assertEquals(listOf(LinkRange(6, 19, "https://a.com"), LinkRange(24, 44, "https://b.com/(@sam)")), linkRanges(text))
        assertEquals(emptyList<LinkRange>(), linkRanges("", listOf(0 until 0)))
    }
}
