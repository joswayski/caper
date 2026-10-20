package chat.caper.android.data

import java.io.File
import kotlinx.serialization.json.*
import org.junit.Assert.assertEquals
import org.junit.Test

class FormattingTest {
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

    @Test fun `toggle removes selected markers or inserts at caret`() {
        assertEquals(FormatSelection("bold", 0, 4), toggleFormat("**bold**", 0, 8, "**"))
        assertEquals(FormatSelection("italic", 0, 6), toggleFormat("_italic_", 0, 8, "_"))
        assertEquals(FormatSelection("a****b", 3, 3), toggleFormat("ab", 1, 1, "**"))
        assertEquals(FormatSelection(" _word_ \n", 2, 6), toggleFormat(" word \n", 0, 7, "_"))
        assertEquals(" \t", toggleFormat(" \t", 0, 2, "**").text)
    }
}
