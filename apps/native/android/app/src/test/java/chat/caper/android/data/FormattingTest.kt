package chat.caper.android.data

import java.io.File
import kotlinx.serialization.json.*
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
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

    /** Every client types these same steps, generated from web; see shared/messages/composer-cases.json. */
    @Test fun `composer typing follows the shared cases`() {
        val file = generateSequence(java.io.File("").absoluteFile) { it.parentFile }
            .map { java.io.File(it, "shared/messages/composer-cases.json") }.first { it.isFile }
        val cases = Json.parseToJsonElement(file.readText()).jsonObject.getValue("cases").jsonArray
        assertTrue(cases.size > 20)
        for (case in cases.map { it.jsonObject }) {
            var text = case.getValue("text").jsonPrimitive.content
            var caret = formattedMessage(text).original(case.getValue("caret").jsonPrimitive.int)
            var history = FormatEditHistory()
            var pending = -1
            for (step in case.getValue("steps").jsonArray.map { it.jsonObject }) {
                if (step.getValue("placed").jsonPrimitive.boolean) {
                    // A tap: a new caret, so no style carries over from the last key.
                    caret = formattedMessage(text).let { it.original(it.displayed(caret)) }
                    history = FormatEditHistory()
                    pending = -1
                }
                val key = step.getValue("key").jsonPrimitive.content
                val next = FormatSelection(text.substring(0, caret) + key + text.substring(caret), caret + key.length, caret + key.length)
                val note = case.getValue("note").jsonPrimitive.content + ": " + key
                // The carried style is any style bits, including underline, strikethrough, spoiler and quote.
                val edit = composerEdit(text, next, pending)
                pending = edit.pending
                assertEquals(note, step.getValue("pending").jsonPrimitive.int, pending)
                val typed = history.edit(text, next)
                assertEquals(note, edit.selection, typed)
                text = typed.text
                caret = typed.start
                assertEquals(note, step.getValue("text").jsonPrimitive.content, text)
                assertEquals(note, step.getValue("caret").jsonPrimitive.int, formattedMessage(text).displayed(caret))
            }
        }
    }

    /** The style continues only while the field shows what typing left, as in every client's composer. */
    @Test fun `composer typing ends when the field changes under it`() {
        fun FormatEditHistory.type(text: String, key: String) =
            edit(text, FormatSelection(text + key, text.length + key.length, text.length + key.length)).text
        var history = FormatEditHistory()
        var text = history.type("**bold**", " ")
        assertEquals("continuous typing", "**bold** **a**", history.type(text, "a"))
        history = FormatEditHistory(); text = history.type("**bold**", " ")
        history.edit(text, FormatSelection(text, 0, 0)); history.edit(text, FormatSelection(text, 9, 9))
        assertEquals("caret moved away and back", "**bold** a", history.type(text, "a"))
        history = FormatEditHistory(); text = history.type("**bold**", " ")
        history.edit(text, FormatSelection("**bold**", 8, 8), restoringHistory = true)
        history.edit("**bold**", FormatSelection(text, 9, 9), restoringHistory = true)
        assertEquals("undo then redo", "**bold** a", history.type(text, "a"))
        history = FormatEditHistory(); text = history.type("**bold**", " ")
        history.endTyping()
        assertEquals("identical draft in another conversation", "**bold** a", history.type(text, "a"))
    }

    @Test fun `typing on at the end of a styled draft keeps the style`() {
        // Types one character at a time with the caret at the visible end, as the composer maps it.
        fun type(start: String, typed: String) = typed.fold(start) { text, c ->
            val caret = formattedMessage(text).let { it.original(it.text.length) }
            FormatEditHistory().edit(text, FormatSelection(text.substring(0, caret) + c + text.substring(caret), caret + 1, caret + 1)).text
        }
        assertEquals("**bold** and", type("**bold**", " and"))
        assertEquals("**bolder**", type("**bold**", "er"))
        assertEquals("_it_ x", type("_it_", " x"))
        assertEquals("**a** _b_ c", type("**a** _b_", " c"))
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
        val text = "a **bold** and _it_ c __u__ ~~s~~ ||x||\n> q"
        // Source offsets; the field hides the delimiters.
        assertEquals(FormatStyle.BOLD, selectionStyles(text, 4, 8))
        assertEquals(FormatStyle.BOLD, selectionStyles(text, 0, 5))
        assertEquals(0, selectionStyles(text, 0, 1))
        assertEquals(FormatStyle.ITALIC, selectionStyles(text, 16, 18))
        assertEquals(0, selectionStyles(text, 11, 14))
        val underline = text.indexOf("u__")
        assertEquals(FormatStyle.UNDERLINE, selectionStyles(text, underline, underline + 1))
        assertEquals(FormatStyle.STRIKETHROUGH, selectionStyles(text, text.indexOf("s~~"), text.indexOf("s~~") + 1))
        assertEquals(FormatStyle.SPOILER, selectionStyles(text, text.indexOf("x||"), text.indexOf("x||") + 1))
        assertEquals(FormatStyle.QUOTE, selectionStyles(text, text.length - 1, text.length))
        assertEquals(FormatStyle.UNDERLINE or FormatStyle.STRIKETHROUGH or FormatStyle.SPOILER, selectionStyles(text, underline, text.indexOf("x||") + 1))
    }

    /** Every toolbar toggle web makes, through the shared toggle (shared/messages/toggle-cases.json). */
    @Test fun `toolbar toggles follow the shared cases`() {
        val file = generateSequence(File("").absoluteFile) { it.parentFile }
            .map { File(it, "shared/messages/toggle-cases.json") }.first { it.isFile }
        val cases = Json.parseToJsonElement(file.readText()).jsonObject.getValue("cases").jsonArray
        assertTrue(cases.size > 300)
        for (case in cases.map { it.jsonObject }) {
            val text = case.getValue("text").jsonPrimitive.content
            val style = when (val name = case.getValue("style").jsonPrimitive.content) {
                "bold" -> FormatStyle.BOLD
                "italic" -> FormatStyle.ITALIC
                "underline" -> FormatStyle.UNDERLINE
                "strikethrough" -> FormatStyle.STRIKETHROUGH
                "spoiler" -> FormatStyle.SPOILER
                "quote" -> FormatStyle.QUOTE
                else -> error(name)
            }
            // Visible UTF-16 offsets to the source, as the field maps a selection.
            val from = case.getValue("from").jsonPrimitive.int
            val to = case.getValue("to").jsonPrimitive.int
            val shown = formattedMessage(text).sourceIndices
            val start = shown.getOrNull(from) ?: text.length
            val end = if (to > from) shown[to - 1] + 1 else start
            val result = toggleFormat(text, start, end, style)
            val label = case.toString()
            val expected = case.getValue("result").jsonPrimitive.contentOrNull
            if (expected == null) assertEquals(label, null, result)
            else {
                requireNotNull(result) { label }
                assertEquals(label, expected, result.text)
                val after = formattedMessage(result.text)
                assertEquals(label, from to to, after.displayed(result.start) to after.displayed(result.end))
            }
        }
    }

    @Test fun `edits discard orphan delimiters and retain literal punctuation`() {
        assertEquals(FormatSelection("", 0, 0), repairFormatEdit("**hello**", FormatSelection("****", 2, 2)))
        assertEquals(FormatSelection("", 0, 0), repairFormatEdit("**_🙂_**", FormatSelection("**__**", 3, 3)))
        assertEquals(FormatSelection("a  z", 2, 2), repairFormatEdit("a **bold** z", FormatSelection("a ** z", 4, 4)))
        assertEquals(FormatSelection("**llo** tail", 2, 2), repairFormatEdit("**hello** tail", FormatSelection("**llo** tail", 2, 2)))
        assertEquals(FormatSelection("a *", 3, 3), repairFormatEdit("a **", FormatSelection("a *", 3, 3)))
        assertEquals(null, toggleFormat("identifier", 2, 5, FormatStyle.ITALIC))
        assertEquals(null, toggleFormat("identifier", 2, 5, FormatStyle.UNDERLINE))
    }

    @Test fun `formatting cannot split links but can wrap complete links`() {
        val text = "🙂 https://example.com/a_b?q=one&x=two tail"
        val start = text.indexOf("https:")
        val end = text.indexOf(" tail")
        for (style in listOf(FormatStyle.BOLD, FormatStyle.ITALIC, FormatStyle.UNDERLINE, FormatStyle.STRIKETHROUGH, FormatStyle.SPOILER)) {
            for ((a, b) in listOf(start + 2 to start + 2, start + 2 to end, 0 to end - 2, start to end - 1)) {
                assertEquals(null, toggleFormat(text, a, b, style))
            }
            val wrapped = requireNotNull(toggleFormat(text, start, end, style))
            assertEquals(text, formattedMessage(wrapped.text).text)
            // Links are found in the displayed runs, as rendering finds them.
            assertEquals(linkSegments(text).filter { it.href != null }.map { it.href }, messageLinkRanges(wrapped.text).map { it.href })
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
                // Styles other than bold and italic are listed only when present.
                fun bit(name: String, style: Int) = if (run[name]?.jsonPrimitive?.boolean == true) style else 0
                FormattedRun(run.getValue("text").jsonPrimitive.content,
                    bit("bold", FormatStyle.BOLD) or bit("italic", FormatStyle.ITALIC) or bit("underline", FormatStyle.UNDERLINE) or
                        bit("strikethrough", FormatStyle.STRIKETHROUGH) or bit("spoiler", FormatStyle.SPOILER) or bit("quote", FormatStyle.QUOTE))
            }
            val actual = formattedMessage(text)
            assertEquals(text, expected, actual.runs)
            assertEquals(expected.joinToString("") { it.text }, actual.text)
            assertEquals(actual.text, actual.sourceIndices.map { text[it] }.joinToString(""))
        }
    }

    @Test fun `toolbar toggles keep content and refuse a caret`() {
        assertEquals(FormatSelection("bold", 0, 4), toggleFormat("**bold**", 2, 6, FormatStyle.BOLD))
        assertEquals(FormatSelection("a **🙂word** z", 4, 10), toggleFormat("a 🙂word z", 2, 8, FormatStyle.BOLD))
        assertEquals(FormatSelection("a ||🙂word|| z", 4, 10), toggleFormat("a 🙂word z", 2, 8, FormatStyle.SPOILER))
        assertEquals(FormatSelection("> quoted", 2, 8), toggleFormat("quoted", 0, 6, FormatStyle.QUOTE))
        assertEquals(null, toggleFormat("ab", 1, 1, FormatStyle.BOLD))
    }
}
