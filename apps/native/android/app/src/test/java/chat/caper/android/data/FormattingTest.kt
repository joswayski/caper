package chat.caper.android.data

import java.io.File
import kotlinx.serialization.json.*
import org.junit.Assert.assertEquals
import org.junit.Test

class FormattingTest {
    @Test fun `undo restores source snapshots without deleting literal delimiters`() {
        val history = FormatEditHistory()
        history.remember("**")
        history.remember("**x**")
        assertEquals(FormatSelection("**", 2, 2), history.edit("**x**", FormatSelection("**", 2, 2)))
        assertEquals(FormatSelection("**x**", 3, 3), history.edit("**", FormatSelection("**x**", 3, 3), true))
        assertEquals(FormatSelection("", 0, 0), history.edit("**x**", FormatSelection("****", 2, 2)))
    }

    @Test fun `visible clipboard and replacements preserve emoji and link targets`() {
        assertEquals("hello 🙂", copiedFormatText("**hello _🙂_**", 0, 14))
        assertEquals("ell", copiedFormatText("**hello**", 3, 6))
        assertEquals(FormatSelection("**new**", 5, 5), repairFormatEdit("**hello**", FormatSelection("**new**", 5, 5)))
        val url = "https://example.com/a_b"
        for (text in listOf("** $url**", "**$url **")) {
            val repaired = repairFormatEdit("**$url**", FormatSelection(text, 3, 3))
            assertEquals(listOf(url), linkSegments(repaired.text).mapNotNull { it.href })
            assertEquals(text.replace("**", ""), formattedMessage(repaired.text).text)
        }
    }

    @Test fun `toolbar state follows styles anywhere in the source selection`() {
        val text = "a **bold** and _it_ c"
        // Source offsets; the field hides the delimiters.
        assertEquals(true, selectionHasStyle(text, 4, 8, bold = true))
        assertEquals(true, selectionHasStyle(text, 0, 5, bold = true))
        assertEquals(false, selectionHasStyle(text, 0, 1, bold = true))
        assertEquals(false, selectionHasStyle(text, 4, 8, bold = false))
        assertEquals(true, selectionHasStyle(text, 16, 18, bold = false))
        assertEquals(false, selectionHasStyle(text, 11, 14, bold = false))
    }

    @Test fun `edits discard orphan delimiters and retain literal punctuation`() {
        assertEquals(FormatSelection("", 0, 0), repairFormatEdit("**hello**", FormatSelection("****", 2, 2)))
        assertEquals(FormatSelection("", 0, 0), repairFormatEdit("**_🙂_**", FormatSelection("**__**", 3, 3)))
        assertEquals(FormatSelection("a  z", 2, 2), repairFormatEdit("a **bold** z", FormatSelection("a ** z", 4, 4)))
        assertEquals(FormatSelection("**llo** tail", 2, 2), repairFormatEdit("**hello** tail", FormatSelection("**llo** tail", 2, 2)))
        assertEquals(FormatSelection("a *", 3, 3), repairFormatEdit("a **", FormatSelection("a *", 3, 3)))
        assertEquals(FormatSelection("hello rest", 0, 5), toggleFormat("**hello** rest", 2, 9, "**"))
        assertEquals("identifier", toggleFormat("identifier", 2, 5, "_").text)
    }

    @Test fun `formatting cannot split links but can wrap complete links`() {
        val text = "🙂 https://example.com/a_b?q=one&x=two tail"
        val start = text.indexOf("https:")
        val end = text.indexOf(" tail")
        for (marker in listOf("**", "_")) {
            for ((a, b) in listOf(start + 2 to start + 2, start + 2 to end, 0 to end - 2, start to end - 1)) {
                assertEquals(FormatSelection(text, a, b), toggleFormat(text, a, b, marker))
            }
            val wrapped = toggleFormat(text, start, end, marker)
            assertEquals(text, formattedMessage(wrapped.text).text)
            assertEquals(linkSegments(text).filter { it.href != null }.map { it.href },
                linkSegments(wrapped.text).filter { it.href != null }.map { it.href })
        }
    }

    private val cases get() = Json.parseToJsonElement(
        generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/messages/formatting-cases.json") }.first { it.isFile }.readText()
    ).jsonObject.getValue("cases").jsonArray

    @Test fun `shared parser fixtures match observable runs and text`() {
        cases.forEach { fixture ->
            val fields = fixture.jsonObject
            val text = fields.getValue("text").jsonPrimitive.content
            val expected = fields.getValue("runs").jsonArray.map {
                val run = it.jsonObject
                FormattedRun(run.getValue("text").jsonPrimitive.content, run.getValue("bold").jsonPrimitive.boolean, run.getValue("italic").jsonPrimitive.boolean)
            }
            val actual = formattedMessage(text)
            assertEquals(text, expected, actual.runs)
            assertEquals(expected.joinToString("") { it.text }, actual.text)
            assertEquals(actual.text, actual.sourceIndices.map { text[it] }.joinToString(""))
        }
    }

    @Test fun `mixed formatting preserves content and the other style`() {
        assertEquals(FormatSelection("**bold plus normal**", 2, 18), toggleFormat("**bold** plus normal", 0, 20, "**"))
        assertEquals("**one and two**", toggleFormat("**one** and **two**", 0, 19, "**").text)
        assertEquals("_**bold both**_", toggleFormat("**bold _both_**", 0, 15, "_").text)
        assertEquals(FormatSelection("a **🙂word** z", 4, 10), toggleFormat("a 🙂word z", 2, 8, "**"))
        assertEquals(FormatSelection("a 🙂word z", 2, 8), toggleFormat("a **🙂word** z", 4, 10, "**"))
    }

    @Test fun `toggle removes selected markers and ignores a caret`() {
        assertEquals(FormatSelection("bold", 0, 4), toggleFormat("**bold**", 0, 8, "**"))
        assertEquals(FormatSelection("italic", 0, 6), toggleFormat("_italic_", 0, 8, "_"))
        assertEquals(FormatSelection("ab", 1, 1), toggleFormat("ab", 1, 1, "**"))
        assertEquals(FormatSelection(" _word_ \n", 2, 6), toggleFormat(" word \n", 0, 7, "_"))
        assertEquals(" \t", toggleFormat(" \t", 0, 2, "**").text)
    }
}
